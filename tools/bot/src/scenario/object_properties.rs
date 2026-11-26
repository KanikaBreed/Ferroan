// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One estate-owner circuit on the stored prim scene.
//!
//! Pin8 `c401bceecd` answers `RequestObjectPropertiesFamily` with the root
//! (`LLClientView.SendObjectPropertiesFamilyData`), `ObjectSelect` with
//! `SendPropertiesToClient`, and the same pass's `ObjectPhysicsProperties`.
//! Deleting the linkset root sends `KillObject` immediately and again from
//! `PrimUpdateFlags.Kill` (`SceneObjectGroup.DeleteGroupFromScene`).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use ferroan_protocol::Msg;
use ferroan_protocol::msgs::{ObjectProperties, ObjectPropertiesFamily, ObjectUpdate};
use uuid::Uuid;

use super::{Harness, PRIM_CAPTURE_COUNT, leave};
use crate::caps::{EqPollError, poll_event_queue};
use crate::session::Session;
use crate::world::{self, PCODE_AVATAR};

const STEP: Duration = Duration::from_millis(100);
const LINKSET: &str = "65000000-0000-4000-8000-000000000001";
/// Roots after the child request merges onto its root.
const FAMILY_ROOTS: usize = PRIM_CAPTURE_COUNT - 1;

struct Part {
    local_id: u32,
    full_id: Uuid,
    parent_id: u32,
}

pub async fn run(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, _) = super::enter(http, harness, &harness.owner, "last").await?;
    let result = capture(http, &mut session).await;
    let logout = leave(harness, &mut session, &harness.owner).await;
    result.and(logout)?;
    Ok(
        "object-properties: family replies, object properties, the physics event and two root kills\nlive: run passed"
            .into(),
    )
}

async fn capture(http: &reqwest::Client, session: &mut Session) -> Result<(), String> {
    let parts = wait_parts(session).await?;
    let by_local: BTreeMap<u32, Uuid> = parts
        .values()
        .map(|part| (part.local_id, part.full_id))
        .collect();
    let mut roots = BTreeSet::new();
    for part in parts.values() {
        let root = if part.parent_id == 0 {
            part.full_id
        } else {
            *by_local
                .get(&part.parent_id)
                .ok_or_else(|| format!("parent {} is not a stored part", part.parent_id))?
        };
        roots.insert(root);
    }
    if roots.len() != FAMILY_ROOTS {
        return Err(format!(
            "expected {FAMILY_ROOTS} linkset roots, found {}",
            roots.len()
        ));
    }
    let linkset: Uuid = LINKSET.parse().map_err(|_| "linkset uuid".to_string())?;
    let root_local = parts
        .get(&linkset)
        .ok_or("the linkset root is not in the scene")?
        .local_id;

    let agent = session.agent();
    for part in parts.values() {
        let request = world::request_properties(agent, part.full_id);
        session
            .send(&request, true)
            .await
            .map_err(|error| error.to_string())?;
    }
    wait_families(session, &roots).await?;

    let local_ids: Vec<u32> = parts.values().map(|part| part.local_id).collect();
    let select = world::select_objects(session.agent(), &local_ids);
    session
        .send(&select, true)
        .await
        .map_err(|error| error.to_string())?;
    wait_objects(session, &parts).await?;
    wait_physics(http, session).await?;

    let delete = world::delete_object(session.agent(), root_local);
    session
        .send(&delete, true)
        .await
        .map_err(|error| error.to_string())?;
    wait_kills(session, root_local).await
}

async fn wait_parts(session: &mut Session) -> Result<BTreeMap<Uuid, Part>, String> {
    let mut parts: BTreeMap<Uuid, Part> = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        drive(session).await?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id != ObjectUpdate::ID {
                continue;
            }
            let update = packet
                .decode::<ObjectUpdate>()
                .map_err(|error| error.to_string())?;
            for block in update.object_data {
                if block.p_code == PCODE_AVATAR {
                    continue;
                }
                let part = Part {
                    local_id: block.id,
                    full_id: block.full_id,
                    parent_id: block.parent_id,
                };
                match parts.get(&part.full_id) {
                    Some(seen)
                        if seen.local_id != part.local_id || seen.parent_id != part.parent_id =>
                    {
                        return Err(format!("stored part {} changed local id", part.full_id));
                    }
                    Some(_) => {}
                    None => {
                        parts.insert(part.full_id, part);
                    }
                }
                if parts.len() > PRIM_CAPTURE_COUNT {
                    return Err("unexpected extra stored prim".into());
                }
            }
        }
        if parts.len() == PRIM_CAPTURE_COUNT {
            return Ok(parts);
        }
    }
    Err(format!(
        "expected {PRIM_CAPTURE_COUNT} stored prims, received {}",
        parts.len()
    ))
}

async fn wait_families(session: &mut Session, roots: &BTreeSet<Uuid>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        drive(session).await?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id != ObjectPropertiesFamily::ID {
                continue;
            }
            let reply = packet
                .decode::<ObjectPropertiesFamily>()
                .map_err(|error| error.to_string())?;
            let object_id = reply.object_data.object_id;
            if !roots.contains(&object_id) {
                return Err(format!("family reply {object_id} is not a linkset root"));
            }
            seen.insert(object_id);
        }
        if seen.len() == roots.len() {
            return Ok(());
        }
    }
    Err(format!(
        "expected {} family roots, received {}",
        roots.len(),
        seen.len()
    ))
}

async fn wait_objects(session: &mut Session, parts: &BTreeMap<Uuid, Part>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        drive(session).await?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id != ObjectProperties::ID {
                continue;
            }
            let reply = packet
                .decode::<ObjectProperties>()
                .map_err(|error| error.to_string())?;
            for block in reply.object_data {
                if !parts.contains_key(&block.object_id) {
                    return Err(format!("object properties for unknown {}", block.object_id));
                }
                seen.insert(block.object_id);
            }
        }
        if seen.len() == parts.len() {
            return Ok(());
        }
    }
    Err(format!(
        "expected {} object properties, received {}",
        parts.len(),
        seen.len()
    ))
}

async fn wait_physics(http: &reqwest::Client, session: &mut Session) -> Result<(), String> {
    let eq = session
        .caps()
        .get("EventQueueGet")
        .cloned()
        .ok_or("login supplied no EventQueueGet")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ack = None;
    while Instant::now() < deadline {
        let http = http.clone();
        let url = eq.clone();
        let pending = ack;
        let poll = tokio::spawn(async move { poll_event_queue(&http, &url, pending).await });
        while Instant::now() < deadline && !poll.is_finished() {
            drive(session).await?;
        }
        if !poll.is_finished() {
            poll.abort();
            break;
        }
        match poll.await.map_err(|_| "event poll task".to_string())? {
            Ok(batch) => {
                ack = Some(batch.id);
                if batch
                    .events
                    .iter()
                    .any(|event| event.message == "ObjectPhysicsProperties")
                {
                    return Ok(());
                }
            }
            Err(EqPollError::Retry) => {}
            Err(EqPollError::Stop) => {
                return Err("event queue closed before ObjectPhysicsProperties".into());
            }
            Err(EqPollError::Fatal(text)) => return Err(text),
        }
    }
    Err("ObjectPhysicsProperties did not arrive".into())
}

async fn wait_kills(session: &mut Session, root_local: u32) -> Result<(), String> {
    let mut packets = 0u32;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        drive(session).await?;
        while let Some(packet) = session.poll_inbox() {
            let ids = world::killed(&packet);
            if ids.contains(&root_local) {
                packets += 1;
            }
        }
        if packets >= 2 {
            return Ok(());
        }
    }
    Err(format!(
        "expected two KillObject packets for {root_local}, received {packets}"
    ))
}

async fn drive(session: &mut Session) -> Result<(), String> {
    session.drive(STEP).await.map_err(|error| error.to_string())
}
