// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenarios for `cargo xtask live run`. Each check uses a reply from the
//! region. The caller starts and stops the region and supplies the accounts.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use ferroan_llsd::{Llsd, from_xml, to_xml};
use ferroan_protocol::Msg;
use glam::Vec3;
use uuid::Uuid;

use crate::login::{self, LoginRequest, LoginResponse};
use crate::session::{Agent, Session};
use crate::world::{self, SeenObject};

const HANDSHAKE: Duration = Duration::from_secs(30);
const STEP: Duration = Duration::from_millis(100);
const BODY_LIMIT: usize = 8 * 1024 * 1024;
const UPLOAD_NAME: &str = "Ferroan Harness Texture";

/// One account the region already has. The password is not printed.
pub struct Account {
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

/// A region next to the one [`Harness::region_id`] names.
///
/// `x` and `y` are map tiles. The handle is [`region_handle`].
pub struct Neighbour {
    pub region_id: Uuid,
    pub x: u32,
    pub y: u32,
    pub home_x: u32,
    pub home_y: u32,
}

/// What `live run` knows about the region. Passwords stay out of `Debug`.
pub struct Harness {
    pub login_uri: String,
    pub region_id: Uuid,
    pub owner: Account,
    pub chat: Account,
    pub texture: Vec<u8>,
    pub library_texture: Uuid,
    /// HTTP exchanges recorded while [`run`] is in progress.
    pub http_trace: std::sync::Arc<crate::trace::HttpTrace>,
    /// Region stdout. Logout is confirmed here when `LogoutReply` is dropped.
    pub region_log: std::path::PathBuf,
    /// Require the viewer's wire acknowledgement; do not accept a server log.
    pub require_logout_reply: bool,
    /// Set by grid mode. Standalone leaves this empty, and `all` then stays
    /// the seven names in [`NAMES`].
    pub neighbour: Option<Neighbour>,
}

/// Region handle from map tiles, as `Util.RegionGridLocToHandle` packs it.
pub fn region_handle(x: u32, y: u32) -> u64 {
    (u64::from(x) << 40) | (u64::from(y) << 8)
}

/// The scenarios `all` runs, in order.
pub const NAMES: &[&str] = &[
    "login",
    "walk",
    "chat",
    "build",
    "teleport",
    "texture",
    "inventory",
];

pub fn selection(name: &str) -> Option<&'static [&'static str]> {
    if name == "all" {
        Some(NAMES)
    } else if name == "assets" {
        Some(&NAMES[5..7])
    } else if NAMES.contains(&name) {
        Some(match name {
            "login" => &NAMES[0..1],
            "walk" => &NAMES[1..2],
            "chat" => &NAMES[2..3],
            "build" => &NAMES[3..4],
            "teleport" => &NAMES[4..5],
            "texture" => &NAMES[5..6],
            "inventory" => &NAMES[6..7],
            _ => return None,
        })
    } else {
        None
    }
}

/// Run the named scenario, or every scenario when `name` is `all`.
/// The returned text is the report. An error is a report that failed.
pub async fn run(name: &str, harness: &Harness) -> Result<String, String> {
    harness.http_trace.scope(run_traced(name, harness)).await
}

async fn run_traced(name: &str, harness: &Harness) -> Result<String, String> {
    if !world::is_j2c(&harness.texture) {
        return Err("the texture does not start with a JPEG 2000 codestream".to_string());
    }
    let http = login::http_client().map_err(|error| error.to_string())?;
    if name == "neighbour" {
        let line = teleport_neighbour(&http, harness).await?;
        return Ok(format!("{line}\nlive: run passed"));
    }
    let Some(names) = selection(name) else {
        return Err(format!(
            "unknown scenario {name}; expected all or {}",
            NAMES.join(", ")
        ));
    };
    let mut lines = Vec::new();
    for scenario in names {
        match *scenario {
            "login" => login_scenario(&http, harness).await,
            "walk" => walk_scenario(&http, harness).await,
            "chat" => chat_scenario(&http, harness).await,
            "build" => build_scenario(&http, harness).await,
            "teleport" => teleport_scenario(&http, harness).await,
            "texture" => texture_scenario(&http, harness).await,
            "inventory" => inventory_scenario(&http, harness).await,
            _ => Err(format!("unknown scenario {scenario}")),
        }
        .map(|line| lines.push(line))
        .map_err(|error| {
            lines.push(format!("{scenario}: fail ({error})"));
            lines.join("\n")
        })?;
    }
    if name == "all" && harness.neighbour.is_some() {
        teleport_neighbour(&http, harness)
            .await
            .map(|line| lines.push(line))
            .map_err(|error| {
                lines.push(format!("teleport neighbour: fail ({error})"));
                lines.join("\n")
            })?;
    }
    lines.push("live: run passed".to_string());
    Ok(lines.join("\n"))
}

async fn login_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, _) = enter(http, harness, &harness.owner).await?;
    let region = session.region_id().ok_or("the handshake set no region")?;
    if region != harness.region_id {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err(format!("region {region}"));
    }
    leave(harness, &mut session, &harness.owner).await?;
    Ok("login: pass".to_string())
}

async fn walk_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, _) = enter(http, harness, &harness.owner).await?;
    let agent = session.agent();
    let mut start = None;
    let mut look = Vec3::X;
    let mut handle = 0u64;
    let mut local_id = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while start.is_none() && std::time::Instant::now() < deadline {
        session
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            // The full avatar update arrives in this same burst. Recording it
            // here is what lets later terse updates match our local id.
            let _ = avatar_position(&packet, agent.agent_id, &mut local_id);
            if packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID
                && let Some(position) = world::movement_position(&packet)
            {
                start = Some(position);
                if let Some((seen_look, seen_handle)) = world::movement_look(&packet) {
                    look = seen_look;
                    handle = seen_handle;
                }
            }
        }
    }
    let Some(start) = start else {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err("no AgentMovementComplete".to_string());
    };
    let _ = handle;
    let facing = world::facing_look(look);
    let forward = world::horizontal_forward(facing);
    let mut latest = start;
    let mut moved = false;
    let mut samples = 0u32;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        let update = world::agent_update(agent, latest, facing, world::CONTROL_AT_POS);
        session
            .send(&update, false)
            .await
            .map_err(|error| error.to_string())?;
        session
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if let Some(position) = avatar_position(&packet, agent.agent_id, &mut local_id) {
                latest = position;
                samples += 1;
            }
        }
        let delta = Vec3::new(latest.x - start.x, latest.y - start.y, 0.0);
        if delta.dot(forward) >= 1.0 {
            moved = true;
            break;
        }
    }
    // Release forward control so the region stops the movement burst
    // before logout. LogoutReply sits on that same throttle.
    let stop = world::agent_update(agent, latest, facing, 0);
    let _ = session.send(&stop, false).await;
    let logged_out = leave(harness, &mut session, &harness.owner).await;
    if !moved {
        return Err(format!(
            "position stayed at {:.1},{:.1},{:.1} under forward control ({samples} position updates, local id {local_id:?})",
            latest.x, latest.y, latest.z
        ));
    }
    logged_out?;
    Ok(format!(
        "walk: pass ({:.1},{:.1},{:.1} to {:.1},{:.1},{:.1})",
        start.x, start.y, start.z, latest.x, latest.y, latest.z
    ))
}

async fn chat_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let text = "ferroan harness chat";
    let (mut listener, _) = enter(http, harness, &harness.chat).await?;
    let (mut speaker, _) = enter(http, harness, &harness.owner).await?;
    let speaker_id = speaker.agent();
    let chat = world::say(speaker_id, text);
    speaker
        .send(&chat, true)
        .await
        .map_err(|error| error.to_string())?;
    let mut heard = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        speaker
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        listener
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = listener.poll_inbox() {
            if let Some((name, source, chat_type, message)) = world::chat_heard(&packet)
                && source == speaker_id.agent_id
                && chat_type == world::CHAT_SAY
                && message == text
                && name.contains("Test")
                && name.contains("User")
            {
                heard = true;
            }
        }
        if heard {
            break;
        }
    }
    let speaker_out = leave(harness, &mut speaker, &harness.owner).await;
    let listener_out = leave(harness, &mut listener, &harness.chat).await;
    if !heard {
        return Err("the second bot did not hear the say on channel 0".to_string());
    }
    speaker_out?;
    listener_out?;
    Ok("chat: pass".to_string())
}

async fn build_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut owner, _) = enter(http, harness, &harness.owner).await?;
    let (mut other, _) = enter(http, harness, &harness.chat).await?;
    let owner_agent = owner.agent();
    let stand = world::agent_update(
        owner_agent,
        Vec3::new(128.0, 128.0, 24.0),
        glam::Quat::IDENTITY,
        0,
    );
    owner
        .send(&stand, true)
        .await
        .map_err(|error| error.to_string())?;
    let other_agent = other.agent();
    let other_stand = world::agent_update(
        other_agent,
        Vec3::new(128.0, 128.0, 24.0),
        glam::Quat::IDENTITY,
        0,
    );
    other
        .send(&other_stand, true)
        .await
        .map_err(|error| error.to_string())?;
    let scale = Vec3::new(0.5, 0.5, 0.5);
    let place = Vec3::new(128.0, 130.0, 22.0);
    let prim = world::box_at(owner_agent, place, scale);
    owner
        .send(&prim, true)
        .await
        .map_err(|error| error.to_string())?;
    let seen = wait_object(&mut owner, &mut other, |object| {
        object.pcode == world::PCODE_PRIM
            && near_vec(object.scale, [scale.x, scale.y, scale.z], 0.15)
    })
    .await?;
    let renamed = "Ferroan Harness Prim";
    let name = world::rename(owner_agent, seen.local_id, renamed);
    owner
        .send(&name, true)
        .await
        .map_err(|error| error.to_string())?;
    wait_name(&mut owner, &mut other, seen.full_id, renamed).await?;
    let moved = Vec3::new(140.0, 130.0, 22.0);
    let scaled = Vec3::new(1.5, 0.5, 2.0);
    let position = world::transform(owner_agent, seen.local_id, world::UPDATE_POSITION, moved);
    let sized = world::transform(owner_agent, seen.local_id, world::UPDATE_SCALE, scaled);
    owner
        .send(&position, true)
        .await
        .map_err(|error| error.to_string())?;
    owner
        .send(&sized, true)
        .await
        .map_err(|error| error.to_string())?;
    let updated = wait_object(&mut owner, &mut other, |object| {
        object.full_id == seen.full_id
            && near_vec(object.position, [moved.x, moved.y, moved.z], 1.0)
            && near_vec(object.scale, [scaled.x, scaled.y, scaled.z], 0.15)
    })
    .await?;
    let delete = world::delete_object(owner_agent, updated.local_id);
    owner
        .send(&delete, true)
        .await
        .map_err(|error| error.to_string())?;
    wait_kill(&mut owner, &mut other, updated.local_id).await?;
    leave(harness, &mut owner, &harness.owner).await?;
    leave(harness, &mut other, &harness.chat).await?;
    Ok("build: pass".to_string())
}

const CROSS: Duration = Duration::from_secs(45);
const ARRIVE_AT: Vec3 = Vec3::new(128.0, 128.0, 30.0);

async fn teleport_neighbour(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let neighbour = harness
        .neighbour
        .as_ref()
        .ok_or("neighbour teleport needs a grid-mode harness")?;
    let (session, _) = enter(http, harness, &harness.owner).await?;
    // Circuits the source region kept as children. The return hop reuses
    // one instead of sending a second UseCircuitCode.
    let mut parked: Vec<Session> = Vec::new();
    let away = match cross_to(
        http,
        session,
        region_handle(neighbour.x, neighbour.y),
        &mut parked,
    )
    .await
    {
        Ok(session) => session,
        Err(error) => return Err(error),
    };
    if away.region_id() != Some(neighbour.region_id) {
        return Err(format!(
            "landed in {:?}, expected {}",
            away.region_id(),
            neighbour.region_id
        ));
    }
    let home = cross_to(
        http,
        away,
        region_handle(neighbour.home_x, neighbour.home_y),
        &mut parked,
    )
    .await?;
    if home.region_id() != Some(harness.region_id) {
        return Err(format!(
            "returned to {:?}, expected {}",
            home.region_id(),
            harness.region_id
        ));
    }
    let mut home = home;
    leave(harness, &mut home, &harness.owner).await?;
    // Child circuits stay open until the regions are stopped. LogoutRequest
    // on them would close an agent that is no longer root there.
    drop(parked);
    Ok(format!(
        "teleport neighbour: pass ({},{} and back)",
        neighbour.x, neighbour.y
    ))
}

/// `TeleportLocationRequest`, then the destination circuit.
///
/// The bot polls the source event queue. A destination we have not seen
/// is opened with `UseCircuitCode` when `EnableSimulator` or
/// `TeleportFinish` names `handle`. `CompleteAgentMovement` waits until
/// `TeleportFinish`. The hop is done when the destination sends
/// `AgentMovementComplete`. The source circuit is parked: Tranquillity
/// keeps that child, and a later hop to the same address sends
/// `CompleteAgentMovement` on it instead of a second `UseCircuitCode`.
/// A UDP `TeleportFinish` is accepted as well. Nothing is logged out here.
async fn cross_to(
    http: &reqwest::Client,
    mut from: Session,
    handle: u64,
    parked: &mut Vec<Session>,
) -> Result<Session, String> {
    let eq_url = from.caps().get("EventQueueGet").cloned();
    let request = world::teleport_to(from.agent(), handle, ARRIVE_AT);
    from.send(&request, true)
        .await
        .map_err(|error| error.to_string())?;

    let deadline = std::time::Instant::now() + CROSS;
    let mut ack = None;
    let mut dest: Option<Dest> = None;
    let mut pending_seed: Option<(SocketAddr, String)> = None;
    let mut next: Option<Session> = None;
    let mut failed = None;
    let mut polling = eq_url.is_some();
    let mut poll = AbortPoll::default();
    if let Some(url) = &eq_url {
        poll.spawn(http, url, ack);
    }

    while failed.is_none() && !crossing_ready(&dest) && std::time::Instant::now() < deadline {
        from.drive(STEP).await.map_err(|error| error.to_string())?;
        drive_parked(parked).await;
        if let Some(session) = next.as_mut() {
            session
                .drive(STEP)
                .await
                .map_err(|error| error.to_string())?;
        }
        let opened = next.as_ref().map(Session::sim_addr);
        note_udp(
            &mut from,
            handle,
            &mut dest,
            &mut pending_seed,
            &mut failed,
            opened,
        )?;
        if poll.is_finished() {
            let task = poll.take().expect("is_finished means a task is present");
            match task.await {
                Ok(Ok(batch)) => {
                    ack = Some(batch.id);
                    for event in batch.events {
                        if let Some(hint) = crate::caps::crossing_hint(&event.message, &event.body)
                        {
                            apply_hint(&mut dest, &mut pending_seed, &hint, handle, opened)?;
                        }
                    }
                }
                Ok(Err(crate::caps::EqPollError::Retry)) => {}
                Ok(Err(crate::caps::EqPollError::Stop)) => polling = false,
                Ok(Err(crate::caps::EqPollError::Fatal(text))) => return Err(text),
                Err(_) => return Err("event queue poll stopped".to_string()),
            }
            if polling && let Some(url) = &eq_url {
                poll.spawn(http, url, ack);
            }
        }
        open_destination(&mut next, &dest, from.agent(), parked).await?;
    }
    drop(poll);

    if let Some(reason) = failed {
        return Err(format!("teleport failed: {reason}"));
    }
    let Some(dest) = dest.filter(|dest| dest.finish) else {
        return Err(format!("no TeleportFinish for handle {handle}"));
    };
    if dest.seed.is_empty() {
        return Err("TeleportFinish has no seed capability".to_string());
    }
    let Some(mut next) = next else {
        return Err("no circuit for the destination".to_string());
    };
    if next.movement_sent() {
        next.resume_as_root(http, &dest.seed, HANDSHAKE)
            .await
            .map_err(|error| error.to_string())?;
    } else {
        next.run_until_ready(http, &dest.seed, HANDSHAKE)
            .await
            .map_err(|error| error.to_string())?;
    }
    // `CompleteAgentMovement` is sent before this returns.
    // Tranquillity V2 is still inside `UpdateAgent` until the destination
    // has made this avatar root.
    // Starting the return hop before `AgentMovementComplete` aborts it.
    wait_for_arrival(&mut from, &mut next, deadline).await?;
    parked.push(from);
    Ok(next)
}

/// Keep child circuits answering pings. A dead socket is dropped so the
/// next hop can open a new one.
async fn drive_parked(parked: &mut Vec<Session>) {
    let mut index = 0;
    while index < parked.len() {
        if parked[index].drive(STEP).await.is_err() {
            parked.swap_remove(index);
        } else {
            index += 1;
        }
    }
}

/// The destination is in the scene when `AgentMovementComplete` arrives.
/// A `TeleportFailed` on the source circuit is the hop failing.
async fn wait_for_arrival(
    from: &mut Session,
    next: &mut Session,
    deadline: std::time::Instant,
) -> Result<(), String> {
    let mut source_open = true;
    loop {
        let failed = take_failure(from, source_open);
        let mut arrived = false;
        while let Some(packet) = next.poll_inbox() {
            if packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID {
                arrived = true;
            }
        }
        if let Some(reason) = failed {
            return Err(format!("teleport failed: {reason}"));
        }
        if arrived {
            // The destination sets the root flag at the start of MakeRoot and
            // only then sends this packet. Scene.IncomingUpdateChildAgent polls
            // that flag every 250ms before replying to the source's UpdateAgent.
            // An immediate return can make the presence a child again before
            // that poll observes root. Retain this bounded grace for that C#
            // race even with a proxy timeout long enough for event queues.
            // Source: Scene.cs / ScenePresence.cs, Tranquillity 8583218.
            let settle = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < settle {
                let was_open = source_open;
                if source_open && from.drive(STEP).await.is_err() {
                    source_open = false;
                }
                next.drive(STEP).await.map_err(|error| error.to_string())?;
                if let Some(reason) = take_failure(from, was_open) {
                    return Err(format!("teleport failed: {reason}"));
                }
            }
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("no AgentMovementComplete on the destination".to_string());
        }
        if source_open && from.drive(STEP).await.is_err() {
            source_open = false;
        }
        next.drive(STEP).await.map_err(|error| error.to_string())?;
    }
}

struct Dest {
    addr: SocketAddr,
    seed: String,
    finish: bool,
}

fn take_failure(session: &mut Session, open: bool) -> Option<String> {
    if !open {
        return None;
    }
    let mut failed = None;
    while let Some(packet) = session.poll_inbox() {
        if let Some(reason) = world::teleport_failure(&packet) {
            failed = Some(reason);
        }
    }
    failed
}

fn crossing_ready(dest: &Option<Dest>) -> bool {
    dest.as_ref()
        .is_some_and(|dest| dest.finish && !dest.seed.is_empty())
}

fn note_udp(
    session: &mut Session,
    handle: u64,
    dest: &mut Option<Dest>,
    pending_seed: &mut Option<(SocketAddr, String)>,
    failed: &mut Option<String>,
    opened: Option<SocketAddr>,
) -> Result<(), String> {
    while let Some(packet) = session.poll_inbox() {
        if let Some(reason) = world::teleport_failure(&packet) {
            *failed = Some(reason);
        }
        if let Some(arrival) = world::teleport_arrival(&packet) {
            let hint = crate::caps::CrossingHint {
                ip: arrival.sim_ip,
                port: arrival.sim_port,
                handle: Some(arrival.region_handle),
                seed: if arrival.seed_capability.is_empty() {
                    None
                } else {
                    Some(arrival.seed_capability)
                },
                finish: true,
            };
            apply_hint(dest, pending_seed, &hint, handle, opened)?;
        }
    }
    Ok(())
}

/// Remember a destination only when its handle is the one we requested.
/// A seed that arrives first is kept until that handle is known.
fn apply_hint(
    dest: &mut Option<Dest>,
    pending_seed: &mut Option<(SocketAddr, String)>,
    hint: &crate::caps::CrossingHint,
    wanted: u64,
    opened: Option<SocketAddr>,
) -> Result<(), String> {
    let addr = SocketAddr::from((std::net::Ipv4Addr::from(hint.ip), hint.port));
    if let Some(handle) = hint.handle {
        if handle != wanted {
            return Ok(());
        }
        if opened.is_some_and(|opened| opened != addr) {
            return Err("destination hint changed after its circuit was opened".to_string());
        }
        let remembered = pending_seed
            .as_ref()
            .filter(|(pending, _)| *pending == addr)
            .map(|(_, seed)| seed.clone());
        if remembered.is_some() {
            *pending_seed = None;
        }
        let seed = hint
            .seed
            .clone()
            .filter(|seed| !seed.is_empty())
            .or(remembered)
            .unwrap_or_default();
        if let Some(existing) = dest.as_mut().filter(|existing| existing.addr == addr) {
            if !seed.is_empty() {
                existing.seed = seed;
            }
            existing.finish |= hint.finish;
            return Ok(());
        }
        if dest.as_ref().is_some_and(|existing| existing.finish) {
            return Ok(());
        }
        *dest = Some(Dest {
            addr,
            seed,
            finish: hint.finish,
        });
        return Ok(());
    }
    let Some(seed) = hint.seed.clone().filter(|seed| !seed.is_empty()) else {
        return Ok(());
    };
    if let Some(existing) = dest.as_mut().filter(|dest| dest.addr == addr) {
        existing.seed = seed;
    } else {
        *pending_seed = Some((addr, seed));
    }
    Ok(())
}

async fn open_destination(
    next: &mut Option<Session>,
    dest: &Option<Dest>,
    agent: Agent,
    parked: &mut Vec<Session>,
) -> Result<(), String> {
    let Some(dest) = dest else {
        return Ok(());
    };
    if let Some(next) = next {
        if next.sim_addr() != dest.addr {
            return Err("destination differs from its open circuit".to_string());
        }
        return Ok(());
    }
    if let Some(index) = parked
        .iter()
        .position(|session| session.sim_addr() == dest.addr)
    {
        *next = Some(parked.swap_remove(index));
        return Ok(());
    }
    *next = Some(
        Session::connect(dest.addr, agent)
            .await
            .map_err(|error| error.to_string())?,
    );
    Ok(())
}

/// One event-queue poll. Drop aborts it, including on an early return.
#[derive(Default)]
struct AbortPoll {
    task: Option<tokio::task::JoinHandle<Result<crate::caps::EqBatch, crate::caps::EqPollError>>>,
}

impl Drop for AbortPoll {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl AbortPoll {
    fn spawn(&mut self, http: &reqwest::Client, url: &str, ack: Option<i32>) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let http = http.clone();
        let url = url.to_owned();
        self.task = Some(tokio::spawn(async move {
            crate::caps::poll_event_queue(&http, &url, ack).await
        }));
    }

    fn is_finished(&self) -> bool {
        self.task.as_ref().is_some_and(|task| task.is_finished())
    }

    fn take(
        &mut self,
    ) -> Option<tokio::task::JoinHandle<Result<crate::caps::EqBatch, crate::caps::EqPollError>>>
    {
        self.task.take()
    }
}

async fn teleport_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, _) = enter(http, harness, &harness.owner).await?;
    let agent = session.agent();
    let mut start = None;
    let mut handle = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline && (start.is_none() || handle.is_none()) {
        session
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID {
                start = world::movement_position(&packet).or(start);
                if let Some((_, seen)) = world::movement_look(&packet) {
                    handle = Some(seen);
                }
            }
        }
    }
    let (Some(start), Some(handle)) = (start, handle) else {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err("no AgentMovementComplete".to_string());
    };
    let target = Vec3::new(40.0, 200.0, 30.0);
    let request = world::teleport_to(agent, handle, target);
    session
        .send(&request, true)
        .await
        .map_err(|error| error.to_string())?;
    let mut landed = None;
    let mut failed = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while landed.is_none() && failed.is_none() && std::time::Instant::now() < deadline {
        session
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if let Some(position) = world::teleport_landed(&packet) {
                landed = Some(position);
            }
            if let Some(reason) = world::teleport_failure(&packet) {
                failed = Some(reason);
            }
        }
    }
    let logged_out = leave(harness, &mut session, &harness.owner).await;
    if let Some(reason) = failed {
        return Err(format!("teleport failed: {reason}"));
    }
    let Some(landed) = landed else {
        return Err(format!("no TeleportLocal (region handle {handle})"));
    };
    let horizontal = (landed.x - target.x).hypot(landed.y - target.y);
    let travelled = (landed.x - start.x).hypot(landed.y - start.y);
    if horizontal > 5.0 || travelled < 20.0 {
        return Err(format!(
            "landed at {:.1},{:.1},{:.1}, target {:.1},{:.1},{:.1}",
            landed.x, landed.y, landed.z, target.x, target.y, target.z
        ));
    }
    logged_out?;
    Ok(format!(
        "teleport: pass ({:.1},{:.1},{:.1})",
        landed.x, landed.y, landed.z
    ))
}

async fn texture_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, login) = enter(http, harness, &harness.owner).await?;
    let folder = textures_folder(&login).ok_or("the login reply has no Textures folder")?;
    let uploaded = upload_texture(http, &session, folder, &harness.texture).await?;
    let fetched = fetch_texture(http, &session, uploaded).await?;
    if fetched != harness.texture {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err(format!(
            "uploaded texture came back as {} bytes, sent {}",
            fetched.len(),
            harness.texture.len()
        ));
    }
    let library = fetch_texture(http, &session, harness.library_texture).await?;
    leave(harness, &mut session, &harness.owner).await?;
    if !world::is_j2c(&fetched) || !world::is_j2c(&library) {
        return Err("a fetched texture is not a JPEG 2000 codestream".to_string());
    }
    Ok(format!(
        "texture: pass (upload {} bytes, library {} bytes)",
        fetched.len(),
        library.len()
    ))
}

async fn inventory_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, login) = enter(http, harness, &harness.owner).await?;
    let root = login.inventory_root;
    if root.is_nil() {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err("the login reply has no inventory root".to_string());
    }
    let folder = textures_folder(&login).ok_or("the login reply has no Textures folder")?;
    let item = upload_texture(http, &session, folder, &harness.texture).await?;
    let reply = fetch_folder(http, &session, folder, session.agent().agent_id).await?;
    leave(harness, &mut session, &harness.owner).await?;
    if !llsd_has_uuid(&reply, folder) || !llsd_has_text(&reply, UPLOAD_NAME) {
        return Err("the folder fetch did not return the uploaded item".to_string());
    }
    if !llsd_has_uuid(&reply, item) && !llsd_has_text(&reply, &item.to_string()) {
        return Err("the folder fetch did not return the new asset".to_string());
    }
    Ok("inventory: pass".to_string())
}

async fn enter(
    http: &reqwest::Client,
    harness: &Harness,
    account: &Account,
) -> Result<(Session, LoginResponse), String> {
    let response = login::login(
        http,
        &LoginRequest {
            login_uri: harness.login_uri.clone(),
            username: format!("{} {}", account.first_name, account.last_name),
            password: account.password.clone(),
            start: "last".to_string(),
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    if response.seed_capability.is_empty() {
        return Err("the login reply has no seed capability".to_string());
    }
    let sim = SocketAddr::from((response.sim_ip, response.sim_port));
    let mut session = Session::connect(
        sim,
        Agent {
            agent_id: response.agent_id,
            session_id: response.session_id,
            circuit_code: response.circuit_code,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    session
        .run_until_ready(http, &response.seed_capability, HANDSHAKE)
        .await
        .map_err(|error| error.to_string())?;
    Ok((session, response))
}

async fn leave(harness: &Harness, session: &mut Session, account: &Account) -> Result<(), String> {
    // LogoutReply is queued on the task throttle. The outgoing loop sleeps
    // while that queue is idle, and closing the agent then clears the queue,
    // so the reply is often never sent. HandleLogout logs the request before
    // that. An idle gap still gives the reply a chance to win the race.
    let name = format!("{} {}", account.first_name, account.last_name);
    let logged_before = logout_requests(&harness.region_log, &name)?;
    let started = std::time::Instant::now();
    let before = session.received();
    let deadline = started + Duration::from_secs(8);
    let mut quiet_since = None;
    while std::time::Instant::now() < deadline {
        let seen = session.received();
        session
            .drive(Duration::from_millis(200))
            .await
            .map_err(|error| error.to_string())?;
        while session.poll_inbox().is_some() {}
        if session.received() == seen {
            let since = quiet_since.unwrap_or_else(std::time::Instant::now);
            quiet_since = Some(since);
            if since.elapsed() >= Duration::from_millis(400) {
                break;
            }
        } else {
            quiet_since = None;
        }
    }
    if session.logout(Duration::from_secs(2)).await.is_ok() {
        return Ok(());
    }
    if harness.require_logout_reply {
        return Err("logout: timed out waiting for LogoutReply".to_string());
    }
    let log_deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if logout_requests(&harness.region_log, &name)? > logged_before {
            return Ok(());
        }
        if std::time::Instant::now() >= log_deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let quiet_ms = quiet_since.map_or(0, |since| since.elapsed().as_millis());
    Err(format!(
        "logout: timed out waiting for logout (settled {}ms, quiet {quiet_ms}ms, {} datagrams, no log line for {name})",
        started.elapsed().as_millis(),
        session.received() - before
    ))
}

/// `HandleLogout` writes this before it queues `LogoutReply`.
fn logout_requests(path: &Path, name: &str) -> Result<usize, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let needle = format!("Got a logout request for {name} ");
    Ok(text.matches(&needle).count())
}

fn avatar_position(
    packet: &ferroan_protocol::IncomingPacket,
    agent: Uuid,
    local_id: &mut Option<u32>,
) -> Option<Vec3> {
    for object in world::objects_in(packet) {
        if object.full_id == agent {
            // A non-avatar update can share the agent id. Keep an avatar local
            // id once one arrives, and still report the position either way.
            if local_id.is_none() || object.pcode == world::PCODE_AVATAR {
                *local_id = Some(object.local_id);
            }
            return Some(Vec3::from(object.position));
        }
    }
    for (seen, avatar, position) in world::terse_in(packet) {
        if local_id.is_none() && avatar {
            *local_id = Some(seen);
        }
        if *local_id == Some(seen) {
            return Some(position);
        }
    }
    None
}

async fn wait_object<F>(
    owner: &mut Session,
    other: &mut Session,
    mut pred: F,
) -> Result<SeenObject, String>
where
    F: FnMut(SeenObject) -> bool,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut from_owner = None;
    let mut from_other = None;
    while std::time::Instant::now() < deadline {
        owner.drive(STEP).await.map_err(|error| error.to_string())?;
        other.drive(STEP).await.map_err(|error| error.to_string())?;
        while let Some(packet) = owner.poll_inbox() {
            for object in world::objects_in(&packet) {
                if pred(object) {
                    from_owner = Some(object);
                }
            }
        }
        while let Some(packet) = other.poll_inbox() {
            for object in world::objects_in(&packet) {
                if pred(object) {
                    from_other = Some(object);
                }
            }
        }
        if let (Some(left), Some(right)) = (from_owner, from_other)
            && left.full_id == right.full_id
            && near_vec(left.position, right.position, 0.5)
            && near_vec(left.scale, right.scale, 0.15)
        {
            return Ok(left);
        }
    }
    Err("the two bots did not see the same object update".to_string())
}

async fn wait_name(
    owner: &mut Session,
    other: &mut Session,
    object_id: Uuid,
    expected: &str,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut owner_named = false;
    let mut other_named = false;
    let mut last_request = std::time::Instant::now() - Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        if last_request.elapsed() >= Duration::from_secs(1) {
            let ask_owner = world::request_properties(owner.agent(), object_id);
            owner
                .send(&ask_owner, true)
                .await
                .map_err(|error| error.to_string())?;
            let ask_other = world::request_properties(other.agent(), object_id);
            other
                .send(&ask_other, true)
                .await
                .map_err(|error| error.to_string())?;
            last_request = std::time::Instant::now();
        }
        owner.drive(STEP).await.map_err(|error| error.to_string())?;
        other.drive(STEP).await.map_err(|error| error.to_string())?;
        while let Some(packet) = owner.poll_inbox() {
            if world::property_name(&packet, object_id).as_deref() == Some(expected) {
                owner_named = true;
            }
        }
        while let Some(packet) = other.poll_inbox() {
            if world::property_name(&packet, object_id).as_deref() == Some(expected) {
                other_named = true;
            }
        }
        if owner_named && other_named {
            return Ok(());
        }
    }
    Err(format!(
        "object name {expected} was not on both bots (owner {owner_named}, other {other_named})"
    ))
}

async fn wait_kill(owner: &mut Session, other: &mut Session, local_id: u32) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut owner_saw = false;
    let mut other_saw = false;
    while std::time::Instant::now() < deadline {
        owner.drive(STEP).await.map_err(|error| error.to_string())?;
        other.drive(STEP).await.map_err(|error| error.to_string())?;
        while let Some(packet) = owner.poll_inbox() {
            if world::killed(&packet).contains(&local_id) {
                owner_saw = true;
            }
        }
        while let Some(packet) = other.poll_inbox() {
            if world::killed(&packet).contains(&local_id) {
                other_saw = true;
            }
        }
        if owner_saw && other_saw {
            return Ok(());
        }
    }
    Err("KillObject did not reach both bots".to_string())
}

fn textures_folder(login: &LoginResponse) -> Option<Uuid> {
    for folder in login.raw.get("inventory-skeleton").as_array() {
        let name = folder.get("name").as_str().trim_matches('"');
        if name == "Textures" {
            let id = folder.get("folder_id").as_uuid();
            if !id.is_nil() {
                return Some(id);
            }
        }
    }
    None
}

async fn upload_texture(
    http: &reqwest::Client,
    session: &Session,
    folder: Uuid,
    texture: &[u8],
) -> Result<Uuid, String> {
    let Some(url) = session.caps().get("NewFileAgentInventory") else {
        return Err("the seed has no NewFileAgentInventory".to_string());
    };
    let mut request = Llsd::new_map();
    request.insert("asset_type", "texture");
    request.insert("inventory_type", "texture");
    request.insert("name", UPLOAD_NAME);
    request.insert("description", "synthetic harness upload");
    request.insert("folder_id", folder);
    request.insert("texture_folder_id", folder);
    request.insert("next_owner_mask", Llsd::Integer(0x0008_e000));
    request.insert("group_mask", Llsd::Integer(0));
    request.insert("everyone_mask", Llsd::Integer(0));
    let offered = post_llsd(http, url, &request).await?;
    if offered.get("state").as_str() == "error" {
        return Err(format!(
            "upload refused: {}",
            offered.get("error").get("message").as_str()
        ));
    }
    let uploader = offered.get("uploader").as_str();
    if uploader.is_empty() {
        return Err(format!("upload state {}", offered.get("state").as_str()));
    }
    let completed = post_bytes_llsd(http, uploader, texture).await?;
    if completed.get("state").as_str() != "complete" {
        return Err(format!(
            "upload finished as {}: {}",
            completed.get("state").as_str(),
            completed.get("error").get("message").as_str()
        ));
    }
    let asset = completed.get("new_asset").as_uuid();
    if asset.is_nil() {
        return Err("the upload reply has no new asset".to_string());
    }
    Ok(asset)
}

async fn fetch_texture(
    http: &reqwest::Client,
    session: &Session,
    asset: Uuid,
) -> Result<Vec<u8>, String> {
    let Some(url) = session.caps().get("GetTexture") else {
        return Err("the seed has no GetTexture".to_string());
    };
    let target = if url.contains('?') {
        format!("{url}&texture_id={asset}")
    } else {
        format!("{url}?texture_id={asset}")
    };
    let mut last = "no reply".to_string();
    for _ in 0..5 {
        match get_bytes(http, &target).await {
            Ok(bytes) if world::is_j2c(&bytes) => return Ok(bytes),
            Ok(bytes) => last = format!("reply is {} bytes and is not a codestream", bytes.len()),
            Err(error) => last = error,
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(last)
}

async fn fetch_folder(
    http: &reqwest::Client,
    session: &Session,
    folder: Uuid,
    owner: Uuid,
) -> Result<Llsd, String> {
    let Some(url) = session.caps().get("FetchInventoryDescendents2") else {
        return Err("the seed has no FetchInventoryDescendents2".to_string());
    };
    let mut one = Llsd::new_map();
    one.insert("folder_id", folder);
    one.insert("owner_id", owner);
    one.insert("fetch_folders", Llsd::Integer(1));
    one.insert("fetch_items", Llsd::Integer(1));
    one.insert("sort_order", Llsd::Integer(0));
    let mut folders = Llsd::new_array();
    folders.push(one);
    let mut request = Llsd::new_map();
    request.insert("folders", folders);
    post_llsd(http, url, &request).await
}

async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, String> {
    let xml = to_xml(body);
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .header(reqwest::header::ACCEPT, "application/llsd+xml")
        .body(xml.clone())
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let response_type = content_type(&response);
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if bytes.len() > BODY_LIMIT {
        return Err(format!("cap reply is {} bytes", bytes.len()));
    }
    crate::trace::note_exchange(crate::trace::Exchange {
        method: "POST",
        url,
        request_type: Some("application/llsd+xml"),
        accept: Some("application/llsd+xml"),
        request_body: &xml,
        status: status.as_u16(),
        response_type: response_type.as_deref(),
        response_body: &bytes,
    });
    if !status.is_success() {
        return Err(format!("cap status {}", status.as_u16()));
    }
    from_xml(&bytes).map_err(|error| error.to_string())
}

async fn post_bytes_llsd(http: &reqwest::Client, url: &str, body: &[u8]) -> Result<Llsd, String> {
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(body.to_vec())
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let response_type = content_type(&response);
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if bytes.len() > BODY_LIMIT {
        return Err(format!("upload reply is {} bytes", bytes.len()));
    }
    crate::trace::note_exchange(crate::trace::Exchange {
        method: "POST",
        url,
        request_type: Some("application/octet-stream"),
        accept: None,
        request_body: body,
        status: status.as_u16(),
        response_type: response_type.as_deref(),
        response_body: &bytes,
    });
    if !status.is_success() {
        return Err(format!("upload status {}", status.as_u16()));
    }
    from_xml(&bytes).map_err(|error| error.to_string())
}

async fn get_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let response_type = content_type(&response);
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if bytes.len() > BODY_LIMIT {
        return Err(format!("texture reply is {} bytes", bytes.len()));
    }
    crate::trace::note_exchange(crate::trace::Exchange {
        method: "GET",
        url,
        request_type: None,
        accept: None,
        request_body: b"",
        status: status.as_u16(),
        response_type: response_type.as_deref(),
        response_body: &bytes,
    });
    if !status.is_success() {
        return Err(format!("texture status {}", status.as_u16()));
    }
    Ok(bytes.to_vec())
}

fn content_type(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn llsd_has_text(value: &Llsd, needle: &str) -> bool {
    match value {
        Llsd::String(text) | Llsd::Uri(text) => text.contains(needle),
        Llsd::Array(items) => items.iter().any(|item| llsd_has_text(item, needle)),
        Llsd::Map(map) => map.values().any(|item| llsd_has_text(item, needle)),
        _ => false,
    }
}

fn llsd_has_uuid(value: &Llsd, id: Uuid) -> bool {
    match value {
        Llsd::Uuid(found) => *found == id,
        Llsd::String(text) => text.eq_ignore_ascii_case(&id.to_string()),
        Llsd::Array(items) => items.iter().any(|item| llsd_has_uuid(item, id)),
        Llsd::Map(map) => map.values().any(|item| llsd_has_uuid(item, id)),
        _ => false,
    }
}

fn near_vec(left: [f32; 3], right: [f32; 3], epsilon: f32) -> bool {
    (left[0] - right[0]).abs() <= epsilon
        && (left[1] - right[1]).abs() <= epsilon
        && (left[2] - right[2]).abs() <= epsilon
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn a_logout_log_line_counts_only_that_avatar() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-logout-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "[CLIENT]: Got a logout request for Test User in Ferroan Test\n\
             [CLIENT]: Got a logout request for Chat Bot in Ferroan Test\n\
             [CLIENT]: Got a logout request for Test User in Ferroan Test\n",
        )
        .unwrap();
        assert_eq!(logout_requests(&path, "Test User").unwrap(), 2);
        assert_eq!(logout_requests(&path, "Chat Bot").unwrap(), 1);
        assert_eq!(logout_requests(&path, "Other User").unwrap(), 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn all_expands_and_a_single_name_does_not() {
        assert_eq!(selection("all").unwrap().len(), 7);
        assert_eq!(selection("login").unwrap(), &["login"]);
        assert_eq!(selection("assets").unwrap(), &["texture", "inventory"]);
        assert!(selection("capture").is_none());
        assert!(selection("neighbour").is_none());
    }

    #[test]
    fn event_queue_names_the_neighbour_before_teleport_finish() {
        let handle = region_handle(1001, 1000);
        let handle_b64 = base64::engine::general_purpose::STANDARD.encode(handle.to_be_bytes());
        let ip_b64 = base64::engine::general_purpose::STANDARD.encode([127, 0, 0, 1]);
        let doc = format!(
            "<llsd><map>\
             <key>events</key><array>\
             <map><key>message</key><string>EnableSimulator</string><key>body</key><map>\
             <key>SimulatorInfo</key><array><map>\
             <key>Handle</key><binary>{handle_b64}</binary>\
             <key>IP</key><binary>{ip_b64}</binary>\
             <key>Port</key><integer>39405</integer>\
             </map></array></map></map>\
             <map><key>message</key><string>EstablishAgentCommunication</string><key>body</key><map>\
             <key>agent-id</key><uuid>4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55</uuid>\
             <key>sim-ip-and-port</key><string>127.0.0.1:39405</string>\
             <key>seed-capability</key><string>http://127.0.0.1:39405/CAPS/seed</string>\
             </map></map>\
             <map><key>message</key><string>TeleportFinish</string><key>body</key><map>\
             <key>Info</key><array><map>\
             <key>SimIP</key><binary>{ip_b64}</binary>\
             <key>SimPort</key><integer>39405</integer>\
             <key>RegionHandle</key><binary>{handle_b64}</binary>\
             <key>SeedCapability</key><string>http://127.0.0.1:39405/CAPS/seed</string>\
             </map></array></map></map>\
             </array>\
             <key>id</key><integer>7</integer>\
             </map></llsd>"
        );
        let batch = crate::caps::parse_event_queue(doc.as_bytes()).unwrap();
        assert_eq!(batch.id, 7);
        assert_eq!(batch.events.len(), 3);
        let enable =
            crate::caps::crossing_hint(&batch.events[0].message, &batch.events[0].body).unwrap();
        assert_eq!(enable.ip, [127, 0, 0, 1]);
        assert_eq!(enable.port, 39405);
        assert_eq!(enable.handle, Some(handle));
        assert!(!enable.finish);
        let establish =
            crate::caps::crossing_hint(&batch.events[1].message, &batch.events[1].body).unwrap();
        assert_eq!(establish.handle, None);
        assert_eq!(
            establish.seed.as_deref(),
            Some("http://127.0.0.1:39405/CAPS/seed")
        );
        let finish =
            crate::caps::crossing_hint(&batch.events[2].message, &batch.events[2].body).unwrap();
        assert!(finish.finish);
        assert_eq!(finish.handle, Some(handle));

        let mut dest = None;
        let mut pending = None;
        apply_hint(&mut dest, &mut pending, &establish, handle, None).unwrap();
        assert!(dest.is_none());
        assert!(pending.is_some());
        apply_hint(&mut dest, &mut pending, &enable, handle, None).unwrap();
        let opened = dest.as_ref().unwrap();
        assert_eq!(opened.addr, "127.0.0.1:39405".parse().unwrap());
        assert_eq!(opened.seed, "http://127.0.0.1:39405/CAPS/seed");
        assert!(!opened.finish);
        assert!(pending.is_none());
        apply_hint(&mut dest, &mut pending, &finish, handle, None).unwrap();
        assert!(crossing_ready(&dest));

        let other = region_handle(1002, 1000);
        let mut ignored = crate::caps::CrossingHint {
            handle: Some(other),
            finish: false,
            seed: None,
            ..enable
        };
        ignored.ip = [127, 0, 0, 2];
        ignored.port = 9;
        apply_hint(&mut dest, &mut pending, &ignored, handle, None).unwrap();
        assert_eq!(
            dest.as_ref().unwrap().addr,
            "127.0.0.1:39405".parse().unwrap()
        );
        assert!(crate::caps::crossing_hint("ParcelProperties", &batch.events[0].body).is_none());
    }

    #[tokio::test]
    async fn a_destination_hint_cannot_retarget_an_open_circuit() {
        let first = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let second = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let first_addr = first.local_addr().unwrap();
        let second_addr = second.local_addr().unwrap();
        let handle = region_handle(1001, 1000);
        let mut hint = crate::caps::CrossingHint {
            ip: [127, 0, 0, 1],
            port: first_addr.port(),
            handle: Some(handle),
            seed: None,
            finish: false,
        };
        let mut dest = None;
        let mut pending = None;
        apply_hint(&mut dest, &mut pending, &hint, handle, None).unwrap();
        let agent = Agent {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
        };
        let mut next = None;
        open_destination(&mut next, &dest, agent, &mut Vec::new())
            .await
            .unwrap();
        hint.port = second_addr.port();
        hint.seed = Some("http://127.0.0.1/new-seed".to_string());
        hint.finish = true;
        assert!(
            apply_hint(
                &mut dest,
                &mut pending,
                &hint,
                handle,
                next.as_ref().map(Session::sim_addr)
            )
            .is_err()
        );
        assert_eq!(dest.as_ref().unwrap().addr, first_addr);
        assert_eq!(next.as_ref().unwrap().sim_addr(), first_addr);
        assert!(!crossing_ready(&dest));

        // The circuit-opening boundary also rejects inconsistent state.
        dest.as_mut().unwrap().addr = second_addr;
        assert!(
            open_destination(&mut next, &dest, agent, &mut Vec::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_return_hop_reuses_a_parked_circuit() {
        let home = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let home_addr = home.local_addr().unwrap();
        let agent = Agent {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
        };
        let parked_session = Session::connect(home_addr, agent).await.unwrap();
        let local = parked_session.local_addr().unwrap();
        let mut parked = vec![parked_session];
        let mut next = None;
        let dest = Some(Dest {
            addr: home_addr,
            seed: "http://127.0.0.1/seed".to_string(),
            finish: true,
        });
        open_destination(&mut next, &dest, agent, &mut parked)
            .await
            .unwrap();
        assert!(parked.is_empty());
        let next = next.expect("parked session");
        assert_eq!(next.local_addr().unwrap(), local);
        assert_eq!(next.sim_addr(), home_addr);

        let other = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let other_addr = other.local_addr().unwrap();
        let mut fresh = None;
        let dest = Some(Dest {
            addr: other_addr,
            seed: String::new(),
            finish: false,
        });
        let mut parked = Vec::new();
        open_destination(&mut fresh, &dest, agent, &mut parked)
            .await
            .unwrap();
        let fresh = fresh.expect("new circuit");
        assert_ne!(fresh.local_addr().unwrap(), local);
        assert!(!fresh.movement_sent());
    }

    #[test]
    fn region_handle_packs_map_tiles() {
        // Util.RegionGridLocToHandle: X << 40 | Y << 8.
        assert_eq!(region_handle(1000, 1000), (1000_u64 << 40) | (1000 << 8));
        assert_eq!(region_handle(1001, 1000), (1001_u64 << 40) | (1000 << 8));
    }

    #[test]
    fn the_report_path_does_not_need_a_password_in_the_name_list() {
        let names = selection("all").unwrap();
        assert!(names.contains(&"chat"));
        assert!(names.contains(&"inventory"));
    }
}
