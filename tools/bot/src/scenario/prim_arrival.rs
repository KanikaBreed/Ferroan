// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Observe each viewer receiving the stored parts on Pin8 c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
//! Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPClient.cs, SetThrottles.

use super::{Harness, ObserverSession, PRIM_CAPTURE_COUNT, enter_with_task, leave, world};
use ferroan_protocol::msgs::ObjectUpdate;
use ferroan_protocol::{IncomingPacket, Msg};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

// With the other six viewer rates unchanged, both groups fit Pin8's client ceiling.
const NORMAL_TASK_BPS: f32 = 512_000.0;

#[derive(Default, Debug)]
pub(super) struct ObservedPrims {
    parts: BTreeSet<Uuid>,
}

impl ObservedPrims {
    pub(super) fn note(&mut self, packet: &IncomingPacket) -> Result<(), String> {
        if packet.id == ObjectUpdate::ID {
            for block in packet
                .decode::<ObjectUpdate>()
                .map_err(|e| e.to_string())?
                .object_data
            {
                if block.p_code != world::PCODE_AVATAR {
                    self.parts.insert(block.full_id);
                    if self.parts.len() > PRIM_CAPTURE_COUNT {
                        return Err("unexpected prim in the capture store".into());
                    }
                }
            }
        }
        Ok(())
    }
}

fn part_count(observer: &ObserverSession) -> Result<usize, String> {
    observer
        .seen
        .prim_arrival
        .as_ref()
        .map(|seen| seen.parts.len())
        .ok_or_else(|| "missing prim observer".into())
}

async fn collect(
    control: &mut ObserverSession,
    limited: &mut ObserverSession,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut first = None;
    let mut restored = None;
    while Instant::now() < deadline {
        let (a, b) = tokio::join!(control.step(), limited.step());
        a?;
        b?;
        let now = Instant::now();
        if let Some((pos, look)) = control.seen.arrival {
            let moving = restored.is_none();
            let update = world::agent_update(
                control.session.agent(),
                control.seen.own_position.unwrap_or(pos),
                world::facing_look(look),
                if moving { world::CONTROL_AT_POS } else { 0 },
            );
            control
                .session
                .send(&update, false)
                .await
                .map_err(|e| e.to_string())?;
        }
        let slow = part_count(limited)?;
        if slow > 0 {
            let since = *first.get_or_insert(now);
            if restored.is_none() && now.duration_since(since) >= Duration::from_millis(300) {
                if slow >= PRIM_CAPTURE_COUNT {
                    return Err("limited viewer received all prims before the Task increase".into());
                }
                limited
                    .session
                    .send_throttles(Some(NORMAL_TASK_BPS))
                    .await
                    .map_err(|e| e.to_string())?;
                restored = Some(now);
            }
        }
        if slow == PRIM_CAPTURE_COUNT
            && part_count(control)? == PRIM_CAPTURE_COUNT
            && limited.seen.peer_terse.is_some()
            && restored.is_some_and(|at| now.duration_since(at) >= Duration::from_secs(2))
        {
            return Ok(());
        }
    }
    Err(format!(
        "prim arrival timed out: control {}, limited {}, Task restored {}, peer terse {}",
        part_count(control)?,
        part_count(limited)?,
        restored.is_some(),
        limited.seen.peer_terse.is_some()
    ))
}

fn save_captures(
    harness: &Harness,
    control: &mut ObserverSession,
    limited: &mut ObserverSession,
) -> Result<(), String> {
    for (alias, viewer) in [("control", control), ("limited", limited)] {
        let records = viewer
            .session
            .take_arrival_records()
            .ok_or("missing circuit capture")?;
        let bytes = ferroan_capture::write_capture(&records).map_err(|e| e.to_string())?;
        std::fs::write(
            harness
                .region_log
                .with_file_name(format!("prim-arrival-{alias}.cap")),
            bytes,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) async fn run(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (session, _) =
        enter_with_task(http, harness, &harness.owner, "last", Some(NORMAL_TASK_BPS)).await?;
    let mut control = ObserverSession::new(session, harness.chat.principal_id);
    control.seen.prim_arrival = Some(ObservedPrims::default());
    let (login, observations) = control
        .during(enter_with_task(
            http,
            harness,
            &harness.chat,
            "last",
            Some(8000.0),
        ))
        .await;
    let result = match login {
        Ok((session, _)) => {
            let mut limited = ObserverSession::new(session, harness.owner.principal_id);
            limited.seen.prim_arrival = Some(ObservedPrims::default());
            let observation = async {
                observations?;
                collect(&mut control, &mut limited).await
            }
            .await;
            // Keep draining both sockets through the limited viewer's LogoutRequest.
            let (logout, watching) = control
                .during(leave(harness, &mut limited.session, &harness.chat))
                .await;
            observation
                .and(logout)
                .and(watching)
                .and_then(|()| save_captures(harness, &mut control, &mut limited))
        }
        Err(e) => Err(e),
    };
    let logout = leave(harness, &mut control.session, &harness.owner).await;
    result.and(logout)?;
    Ok("prim-arrival: both viewers received the eight stored parts\nlive: run passed".into())
}
