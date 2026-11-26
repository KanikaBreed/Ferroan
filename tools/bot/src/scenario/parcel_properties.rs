// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One estate-owner circuit on the Q60b multi-parcel store.
//!
//! Pin8 `c401bceecd` sends `ParcelProperties` from `CompleteMovement`
//! (`ScenePresence.cs` 2475), from `ClientOnParcelPropertiesRequest`
//! (`LandManagementModule.cs` 1594) and from `EventManagerOnClientMovement`
//! (`LandManagementModule.cs` 536) when a walk changes parcel. A same-region
//! teleport only moves the avatar (`EntityTransferModule.cs` 703); the next
//! `AgentUpdate` is what sends the landing parcel.

use std::time::{Duration, Instant};

use ferroan_llsd::Llsd;
use ferroan_protocol::msgs::ParcelPropertiesRequest;
use glam::Vec3;

use super::{AbortPoll, Harness, STEP, leave};
use crate::caps::EqPollError;
use crate::session::{Agent, Session};
use crate::world::{self, PCODE_AVATAR};

const CELL_SEQUENCE: i32 = 7;
const SPAN_SEQUENCE: i32 = 8;
/// One land unit inside West, cell (1, 0).
const CELL: [f32; 4] = [5.0, 1.0, 6.0, 2.0];
/// Land units (1, 0) and (2, 0): West then East.
const SPAN: [f32; 4] = [4.0, 0.0, 12.0, 4.0];
const LAND_AT: Vec3 = Vec3::new(6.0, 2.0, 30.0);
const CAPTURE_BUDGET: Duration = Duration::from_secs(90);
/// `CreateDefaultParcel` flags on the remainder parcel. The arrival element
/// sends this word. `KAAgSw==` is the same four bytes.
const ARRIVAL_FLAGS: u32 = 1 | 2 | 8 | 64 | (1 << 13) | (1 << 27) | (1 << 29);

struct ParcelNote {
    name: String,
    sequence: i32,
    result: i32,
    snap: bool,
    flags: u32,
}

struct Seen {
    arrival: Option<Vec3>,
    handle: Option<u64>,
    landed: Option<Vec3>,
    failed: Option<String>,
    own_local: Option<u32>,
    own_pos: Option<Vec3>,
    notes: Vec<ParcelNote>,
}

struct Sent {
    cell: bool,
    span: bool,
    teleport: bool,
}

enum Step {
    Wait,
    SendCell,
    SendSpan,
    SendTeleport,
    Hold,
    Walk,
    Done,
}

pub async fn parcel_properties(harness: &Harness, requests: bool) -> Result<String, String> {
    let http = crate::login::http_client().map_err(|error| error.to_string())?;
    let (mut session, _) = super::enter(&http, harness, &harness.owner, "last").await?;
    let eq = session
        .caps()
        .get("EventQueueGet")
        .cloned()
        .ok_or("login supplied no EventQueueGet")?;
    let result = capture(&http, &mut session, &eq, requests).await;
    let logout = leave(harness, &mut session, &harness.owner).await;
    result.and(logout)?;
    if requests {
        Ok(
            "parcel-properties: captured arrival, one cell, a two-parcel span and a border walk"
                .into(),
        )
    } else {
        Ok("parcel-properties: arrival, a West landing and a border walk".into())
    }
}

async fn capture(
    http: &reqwest::Client,
    session: &mut Session,
    eq: &str,
    requests: bool,
) -> Result<(), String> {
    let me = session.agent().agent_id;
    let mut seen = Seen {
        arrival: None,
        handle: None,
        landed: None,
        failed: None,
        own_local: None,
        own_pos: None,
        notes: Vec::new(),
    };
    let mut sent = Sent {
        cell: false,
        span: false,
        teleport: false,
    };
    let mut polling = Polling::default();
    let mut max_x = 0.0_f32;
    let deadline = Instant::now() + CAPTURE_BUDGET;
    loop {
        if let Some(reason) = seen.failed.clone() {
            return Err(format!("teleport failed: {reason}"));
        }
        if let Some(landed) = seen.landed {
            let horizontal = (landed.x - LAND_AT.x).hypot(landed.y - LAND_AT.y);
            if horizontal > 5.0 {
                return Err(format!(
                    "landed at {:.1},{:.1}, West target {:.1},{:.1}",
                    landed.x, landed.y, LAND_AT.x, LAND_AT.y
                ));
            }
        }
        track(&seen, &mut max_x);
        if max_x >= 12.0 {
            return Err(format!(
                "the walk left East before its ParcelProperties ({})",
                describe(&seen.notes)
            ));
        }
        match next_step(&seen, &sent, max_x, requests)? {
            Step::Done => {
                send_move(session, camera(&seen), 0).await?;
                finish_poll(http, session, eq, &mut polling, &mut seen, me).await?;
                break;
            }
            Step::SendCell => {
                send_request(session, CELL_SEQUENCE, CELL).await?;
                sent.cell = true;
            }
            Step::SendSpan => {
                send_request(session, SPAN_SEQUENCE, SPAN).await?;
                sent.span = true;
            }
            Step::SendTeleport => {
                let handle = seen.handle.ok_or("no region handle on arrival")?;
                let request = world::teleport_to(session.agent(), handle, LAND_AT);
                session
                    .send(&request, true)
                    .await
                    .map_err(|error| error.to_string())?;
                sent.teleport = true;
            }
            Step::Hold => send_move(session, camera(&seen), 0).await?,
            Step::Walk => send_move(session, camera(&seen), world::CONTROL_AT_POS).await?,
            Step::Wait => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "ParcelProperties capture timed out at {:.1},{:.1} ({})",
                seen.own_pos.or(seen.landed).unwrap_or(Vec3::ZERO).x,
                seen.own_pos.or(seen.landed).unwrap_or(Vec3::ZERO).y,
                describe(&seen.notes)
            ));
        }
        pump(http, session, eq, &mut polling, &mut seen, me).await?;
    }
    Ok(())
}

#[derive(Default)]
struct Polling {
    poll: AbortPoll,
    active: bool,
    ack: Option<i32>,
}

fn next_step(seen: &Seen, sent: &Sent, max_x: f32, requests: bool) -> Result<Step, String> {
    if requests {
        check_notes(&seen.notes)?;
    } else {
        check_walk_notes(&seen.notes)?;
    }
    if let Some(position) = seen.arrival
        && !near_default(position)
    {
        return Err(format!(
            "arrival at {:.1},{:.1} is outside the default parcel",
            position.x, position.y
        ));
    }
    if !requests {
        return Ok(match seen.notes.len() {
            0 => Step::Wait,
            1 if !sent.teleport => Step::SendTeleport,
            1 => Step::Hold,
            2 if seen.landed.is_none() || max_x >= 8.5 => Step::Hold,
            2 => Step::Walk,
            3 if max_x < 8.0 => Step::Hold,
            3 => Step::Done,
            _ => {
                return Err(format!(
                    "unexpected ParcelProperties ({})",
                    describe(&seen.notes)
                ));
            }
        });
    }
    Ok(match seen.notes.len() {
        0 | 3 => Step::Wait,
        1 if seen.arrival.is_none() || sent.cell => Step::Wait,
        1 => Step::SendCell,
        2 if !sent.span => Step::SendSpan,
        2 => Step::Wait,
        4 if !sent.teleport => Step::SendTeleport,
        4 => {
            if seen.landed.is_some() {
                Step::Hold
            } else {
                Step::Wait
            }
        }
        5 if seen.landed.is_none() || max_x >= 8.5 => Step::Hold,
        5 => Step::Walk,
        6 if max_x < 8.0 => Step::Hold,
        6 => Step::Done,
        _ => {
            return Err(format!(
                "unexpected ParcelProperties ({})",
                describe(&seen.notes)
            ));
        }
    })
}

fn check_notes(notes: &[ParcelNote]) -> Result<(), String> {
    const WANT: &[(&str, i32, i32)] = &[
        ("Your Parcel", 0, 0),
        ("West", CELL_SEQUENCE, 0),
        ("West", SPAN_SEQUENCE, 1),
        ("East", SPAN_SEQUENCE, 1),
        ("West", 0, 0),
        ("East", 0, 0),
    ];
    if notes.len() > WANT.len() {
        return Err(format!("unexpected ParcelProperties ({})", describe(notes)));
    }
    if let Some(first) = notes.first()
        && first.flags != ARRIVAL_FLAGS
    {
        return Err(format!(
            "arrival ParcelFlags {:#x} ({})",
            first.flags,
            describe(notes)
        ));
    }
    for (note, want) in notes.iter().zip(WANT) {
        if note.name != want.0 || note.sequence != want.1 || note.result != want.2 || note.snap {
            return Err(format!("unexpected ParcelProperties ({})", describe(notes)));
        }
    }
    Ok(())
}

fn check_walk_notes(notes: &[ParcelNote]) -> Result<(), String> {
    const WANT: &[(&str, i32, i32)] = &[("Your Parcel", 0, 0), ("West", 0, 0), ("East", 0, 0)];
    if notes.len() > WANT.len() {
        return Err(format!("unexpected ParcelProperties ({})", describe(notes)));
    }
    if let Some(first) = notes.first()
        && first.flags != ARRIVAL_FLAGS
    {
        return Err(format!(
            "arrival ParcelFlags {:#x} ({})",
            first.flags,
            describe(notes)
        ));
    }
    for (note, want) in notes.iter().zip(WANT) {
        if note.name != want.0 || note.sequence != want.1 || note.result != want.2 || note.snap {
            return Err(format!("unexpected ParcelProperties ({})", describe(notes)));
        }
    }
    Ok(())
}

fn describe(notes: &[ParcelNote]) -> String {
    if notes.is_empty() {
        return "no ParcelProperties yet".to_string();
    }
    notes
        .iter()
        .map(|note| {
            format!(
                "{} sequence {} result {} snap {} flags {:#x}",
                note.name, note.sequence, note.result, note.snap, note.flags
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn near_default(position: Vec3) -> bool {
    (position.x - 128.0).abs() <= 2.0 && (position.y - 128.0).abs() <= 2.0
}

fn track(seen: &Seen, max_x: &mut f32) {
    if let Some(landed) = seen.landed {
        *max_x = max_x.max(landed.x);
    }
    if let Some(own) = seen.own_pos
        && own.y < 20.0
    {
        *max_x = max_x.max(own.x);
    }
}

fn camera(seen: &Seen) -> Vec3 {
    match (seen.own_pos, seen.landed) {
        (Some(own), Some(landed))
            if (own.x - landed.x).abs() < 30.0 && (own.y - landed.y).abs() < 30.0 =>
        {
            own
        }
        (_, Some(landed)) => landed,
        (Some(own), None) => own,
        (None, None) => Vec3::new(128.0, 128.0, 25.0),
    }
}

async fn send_request(
    session: &mut Session,
    sequence: i32,
    bounds: [f32; 4],
) -> Result<(), String> {
    let agent = session.agent();
    let message = parcel_request(agent, sequence, bounds);
    session
        .send(&message, true)
        .await
        .map_err(|error| error.to_string())
}

fn parcel_request(agent: Agent, sequence: i32, bounds: [f32; 4]) -> ParcelPropertiesRequest {
    let mut message = ParcelPropertiesRequest::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.parcel_data.sequence_id = sequence;
    message.parcel_data.west = bounds[0];
    message.parcel_data.south = bounds[1];
    message.parcel_data.east = bounds[2];
    message.parcel_data.north = bounds[3];
    message.parcel_data.snap_selection = false;
    message
}

async fn send_move(session: &mut Session, position: Vec3, controls: u32) -> Result<(), String> {
    let update = world::agent_update(session.agent(), position, glam::Quat::IDENTITY, controls);
    session
        .send(&update, false)
        .await
        .map_err(|error| error.to_string())
}

/// Read the poll that was already in flight. Another parcel element means
/// the avatar left East after the walk element was queued.
async fn finish_poll(
    http: &reqwest::Client,
    session: &mut Session,
    eq: &str,
    polling: &mut Polling,
    seen: &mut Seen,
    me: uuid::Uuid,
) -> Result<(), String> {
    if !polling.active {
        return Ok(());
    }
    let before = seen.notes.len();
    let settle = Instant::now() + Duration::from_secs(70);
    while polling.active && Instant::now() < settle {
        send_move(session, camera(seen), 0).await?;
        pump(http, session, eq, polling, seen, me).await?;
        if seen.notes.len() != before {
            return Err(format!(
                "another ParcelProperties arrived after the walk ({})",
                describe(&seen.notes)
            ));
        }
    }
    if polling.active {
        return Err("event poll did not finish after the walk".into());
    }
    Ok(())
}

async fn pump(
    http: &reqwest::Client,
    session: &mut Session,
    eq: &str,
    polling: &mut Polling,
    seen: &mut Seen,
    me: uuid::Uuid,
) -> Result<(), String> {
    if !polling.active {
        polling.poll.spawn(http, eq, polling.ack);
        polling.active = true;
    }
    session
        .drive(STEP)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(packet) = session.poll_inbox() {
        note_packet(&packet, me, seen);
    }
    if !polling.poll.is_finished() {
        return Ok(());
    }
    polling.active = false;
    let Some(task) = polling.poll.take() else {
        return Err("event poll finished without a task".into());
    };
    match task.await {
        Ok(Ok(batch)) => {
            polling.ack = Some(batch.id);
            for event in batch.events {
                if event.message == "ParcelProperties" {
                    seen.notes.push(parcel_note(&event.body)?);
                }
            }
            Ok(())
        }
        Ok(Err(EqPollError::Retry)) => Ok(()),
        Ok(Err(EqPollError::Stop)) => Err("event queue closed".into()),
        Ok(Err(EqPollError::Fatal(error))) => Err(error),
        Err(error) => Err(format!("event poll task ended: {error}")),
    }
}

fn note_packet(packet: &ferroan_protocol::IncomingPacket, me: uuid::Uuid, seen: &mut Seen) {
    if seen.arrival.is_none() {
        seen.arrival = world::movement_position(packet);
    }
    if seen.handle.is_none()
        && let Some((_, handle)) = world::movement_look(packet)
    {
        seen.handle = Some(handle);
    }
    if let Some(position) = world::teleport_landed(packet) {
        seen.landed = Some(position);
    }
    if let Some(reason) = world::teleport_failure(packet) {
        seen.failed = Some(reason);
    }
    for object in world::objects_in(packet) {
        if object.full_id == me && object.pcode == PCODE_AVATAR {
            seen.own_local = Some(object.local_id);
            seen.own_pos = Some(Vec3::from(object.position));
        }
    }
    for (local_id, avatar, position) in world::terse_in(packet) {
        if avatar && Some(local_id) == seen.own_local {
            seen.own_pos = Some(position);
        }
    }
}

fn parcel_note(body: &Llsd) -> Result<ParcelNote, String> {
    let data = body.get("ParcelData").at(0);
    if !data.is_map()
        || !data.has("Name")
        || !data.has("SequenceID")
        || !data.has("RequestResult")
        || !data.has("SnapSelection")
    {
        return Err("ParcelProperties ParcelData is missing a field".into());
    }
    let name = data.get("Name").as_str();
    if name.is_empty() {
        return Err("ParcelProperties has an empty name".into());
    }
    let raw = data.get("ParcelFlags").as_binary();
    if raw.len() != 4 {
        return Err("ParcelProperties ParcelFlags is not 4 bytes".into());
    }
    let flags = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
    Ok(ParcelNote {
        name: name.to_string(),
        sequence: data.get("SequenceID").as_i32(),
        result: data.get("RequestResult").as_i32(),
        snap: data.get("SnapSelection").as_bool(),
        flags,
    })
}
