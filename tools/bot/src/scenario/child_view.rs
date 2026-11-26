// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Observe a second simulator while keeping the viewer rooted at home.
//! Reference: EntityTransferModule.EnableChildAgents and ScenePresence.SendInitialData,
//! OpenSim-Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b.

use super::{Account, Harness, ObservedAvatar, STEP, Session, enter, region_handle};
use crate::{caps, login, world};
use ferroan_protocol::{IncomingPacket, Msg, msgs::ObjectUpdate};
use glam::{Quat, Vec3};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Identity of the persisted box in the Q65a store.
pub const PRIM: Uuid = Uuid::from_u128(0x65000000_0000_4000_8000_000000000001);

/// The successful home poll. Only the extractor sees the private path and body.
pub struct EventPoll {
    pub path: String,
    pub ack: Option<i32>,
    pub body: Vec<u8>,
}

pub struct Observation {
    pub events: EventPoll,
    pub avatar: Option<IncomingPacket>,
    pub prim: Option<IncomingPacket>,
}

pub async fn run(
    harness: &Harness,
    home_name: &str,
    neighbour_name: &str,
) -> Result<Observation, String> {
    let http = login::http_client().map_err(|e| e.to_string())?;
    let neighbour = harness
        .neighbour
        .as_ref()
        .ok_or("child view needs two regions")?;
    let mut resident = root(
        &http,
        harness,
        &harness.chat,
        neighbour_name,
        16,
        neighbour.region_id,
    )
    .await?;
    let result = async {
        let mut home = root(
            &http,
            harness,
            &harness.owner,
            home_name,
            200,
            harness.region_id,
        )
        .await?;
        let seen = observe(
            &http,
            &mut home,
            &mut resident,
            region_handle(neighbour.x, neighbour.y),
        )
        .await;
        // A reply on the original circuit after observing the neighbour is part of the proof.
        let logout = home.logout(Duration::from_secs(10)).await;
        let (seen, _child) = seen?;
        logout.map_err(|e| e.to_string())?;
        Ok::<_, String>(seen)
    }
    .await;
    let logout = resident.logout(Duration::from_secs(10)).await;
    let result = result?;
    logout.map_err(|e| e.to_string())?;
    Ok(result)
}

async fn root(
    http: &reqwest::Client,
    harness: &Harness,
    account: &Account,
    name: &str,
    x: u32,
    region: Uuid,
) -> Result<Session, String> {
    let (s, _) = enter(http, harness, account, &format!("uri:{name}&{x}&128&30")).await?;
    if s.region_id() != Some(region) || s.agent().agent_id != account.principal_id {
        return Err("root arrived on the wrong account or region".into());
    }
    Ok(s)
}

async fn events(
    http: &reqwest::Client,
    url: &str,
    agent: Uuid,
    wanted: u64,
    addr: SocketAddr,
) -> Result<(EventPoll, String), String> {
    let mut ack = None;
    let path = reqwest::Url::parse(url)
        .map_err(|_| "invalid home event queue URL")?
        .path()
        .to_owned();
    loop {
        let body = match caps::poll_event_queue_body(http, url, ack).await {
            Ok(body) => body,
            Err(caps::EqPollError::Retry) => continue,
            Err(_) => return Err("home event queue closed before child admission".into()),
        };
        let batch = caps::parse_event_queue(&body)?;
        let mut enabled = false;
        for event in batch.events {
            let Some(hint) = caps::crossing_hint(&event.message, &event.body) else {
                continue;
            };
            if hint.finish {
                return Err("home received a teleport finish".into());
            }
            if SocketAddr::from((hint.ip, hint.port)) != addr {
                continue;
            }
            if event.message == "EnableSimulator" && hint.handle == Some(wanted) {
                enabled = true;
            } else if let Some(seed) = hint.seed {
                if !enabled
                    || event.body.get("agent-id").as_uuid() != agent
                    || !seed.starts_with(&format!("http://{addr}/CAPS/"))
                {
                    return Err("unmatched EstablishAgentCommunication in home poll".into());
                }
                return Ok((EventPoll { path, ack, body }, seed));
            }
        }
        if enabled {
            return Err("neighbour events were split across home polls".into());
        }
        ack = Some(batch.id);
    }
}

async fn observe(
    http: &reqwest::Client,
    home: &mut Session,
    resident: &mut Session,
    handle: u64,
) -> Result<(Observation, Session), String> {
    let url = home
        .caps()
        .get("EventQueueGet")
        .ok_or("home has no event queue")?
        .clone();
    let addr = resident.sim_addr();
    let agent = home.agent();
    let wait = tokio::time::timeout(
        Duration::from_secs(35),
        events(http, &url, agent.agent_id, handle, addr),
    );
    tokio::pin!(wait);
    let (events, seed) = loop {
        tokio::select! {
            result = &mut wait => break result.map_err(|_| "home never announced the neighbour")??,
            (root, peer) = async { tokio::join!(home.drive(STEP), resident.drive(STEP)) } => {
                root.map_err(|e| e.to_string())?;
                peer.map_err(|e| e.to_string())?;
            }
        }
    };
    let mut child = Session::connect(addr, agent)
        .await
        .map_err(|e| e.to_string())?;
    caps::request_seed(http, &seed)
        .await
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(12);
    while child.region_id().is_none() && Instant::now() < deadline {
        child.drive(STEP).await.map_err(|e| e.to_string())?;
    }
    if child.region_id() != resident.region_id() {
        return Err("the child's RegionHandshake did not name the neighbour".into());
    }
    child
        .send_viewer_settings()
        .await
        .map_err(|e| e.to_string())?;
    let mut result = Observation {
        events,
        avatar: None,
        prim: None,
    };
    let start_x = 200.0;
    let mut pos = Vec3::new(start_x, 128.0, 30.0);
    let mut home_view = ObservedAvatar::default();
    let mut stopped = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let controls = if stopped.is_none() {
            world::CONTROL_AT_POS
        } else {
            0
        };
        let mut update = world::agent_update(agent, pos, Quat::IDENTITY, controls);
        // Pin8 admits this child at the login draw distance. Keep it fixed so
        // a later position PUT is attributable to motion, not a view change.
        update.agent_data.far = 255.0;
        home.send(&update, false).await.map_err(|e| e.to_string())?;
        let (a, b, c) = tokio::join!(home.drive(STEP), child.drive(STEP), resident.drive(STEP));
        for status in [a, b, c] {
            status.map_err(|e| e.to_string())?;
        }
        while resident.poll_inbox().is_some() {}
        home_view.note(home, resident.agent().agent_id)?;
        if let Some(seen) = home_view.own_position {
            pos = seen;
            if pos.x >= start_x + 20.0 {
                stopped.get_or_insert_with(Instant::now);
            }
        }
        while let Some(packet) = child.poll_inbox() {
            if packet.id != ObjectUpdate::ID {
                continue;
            }
            let update = packet.decode::<ObjectUpdate>().map_err(|e| e.to_string())?;
            if update.region_data.region_handle != handle {
                return Err("wrong region handle on child update".into());
            }
            for block in update.object_data {
                if block.p_code == 47
                    && block.full_id == resident.agent().agent_id
                    && result.avatar.is_none()
                {
                    result.avatar = Some(packet.clone());
                }
                if block.p_code == 9 && block.full_id == PRIM && result.prim.is_none() {
                    result.prim = Some(packet.clone());
                }
            }
        }
        // Pin8 checks child movement at most once every two seconds.
        if stopped.is_some_and(|time| time.elapsed() > Duration::from_secs(3))
            && result.avatar.is_some()
            && result.prim.is_some()
        {
            break;
        }
    }
    if stopped.is_none() {
        return Err(format!("home viewer did not walk 20 m (final X {})", pos.x));
    }
    Ok((result, child))
}
