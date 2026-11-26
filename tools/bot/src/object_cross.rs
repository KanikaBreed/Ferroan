// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Two viewer-driven group-position edits, following LLClientView.HandleMultipleObjUpdate
//! at Pin8 c401bceecd. Both viewers remain in their original regions.

use super::{Account, HANDSHAKE, Harness, STEP, Session, enter, leave, world};
use ferroan_protocol::{
    Msg,
    msgs::{ObjectPermissions, ObjectProperties, object_permissions},
};
use glam::Vec3;
use std::time::Instant;
use uuid::Uuid;

const ROOT: Uuid = Uuid::from_u128(0x65000000_0000_4000_8000_000000000001);
const MOVE: u32 = 1 << 19;

async fn arrive(
    http: &reqwest::Client,
    h: &Harness,
    account: &Account,
    start: &str,
    expected: Uuid,
    camera: Vec3,
) -> Result<Session, String> {
    let (mut session, _) = enter(http, h, account, start).await?;
    if session.region_id() != Some(expected) {
        return Err("object crossing viewer arrived on the wrong region".into());
    }
    session
        .send(
            &world::agent_update(session.agent(), camera, glam::Quat::IDENTITY, 0),
            true,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(session)
}

async fn root_at(s: &mut Session, x: f32) -> Result<u32, String> {
    let deadline = Instant::now() + HANDSHAKE;
    while Instant::now() < deadline {
        s.drive(STEP).await.map_err(|e| e.to_string())?;
        while let Some(packet) = s.poll_inbox() {
            for part in world::objects_in(&packet) {
                if part.full_id == ROOT && (part.position[0] - x).abs() < 0.01 {
                    return Ok(part.local_id);
                }
            }
        }
    }
    Err(format!("viewer did not receive the crossing root at x={x}"))
}

async fn share_move(owner: &mut Session, root: u32) -> Result<(), String> {
    let mut grant = ObjectPermissions::default();
    grant.agent_data.agent_id = owner.agent().agent_id;
    grant.agent_data.session_id = owner.agent().session_id;
    grant.object_data.push(object_permissions::ObjectData {
        object_local_id: root,
        field: 8,
        set: 1,
        mask: MOVE,
    });
    owner.send(&grant, true).await.map_err(|e| e.to_string())?;
    let deadline = Instant::now() + HANDSHAKE;
    while Instant::now() < deadline {
        owner.drive(STEP).await.map_err(|e| e.to_string())?;
        while let Some(packet) = owner.poll_inbox() {
            if packet.id == ObjectProperties::ID
                && packet
                    .decode::<ObjectProperties>()
                    .map_err(|e| e.to_string())?
                    .object_data
                    .iter()
                    .any(|p| p.object_id == ROOT && p.everyone_mask & MOVE != 0)
            {
                return Ok(());
            }
        }
    }
    Err("the region did not confirm everyone Move on the linkset".into())
}

async fn edit(s: &mut Session, root: u32, x: f32) -> Result<(), String> {
    // MultipleObjectUpdate type 9 changes the position of the whole linkset.
    s.send(
        &world::transform(
            s.agent(),
            root,
            8 | world::UPDATE_POSITION,
            Vec3::new(x, 128.0, 40.0),
        ),
        true,
    )
    .await
    .map_err(|e| e.to_string())
}

pub(super) async fn run(http: &reqwest::Client, h: &Harness) -> Result<String, String> {
    let n = h
        .neighbour
        .as_ref()
        .ok_or("object-cross needs two grid regions")?;
    let mut home = arrive(
        http,
        h,
        &h.owner,
        "uri:Ferroan Test&172&128&35",
        h.region_id,
        Vec3::new(172., 128., 35.),
    )
    .await?;
    let mut east = arrive(
        http,
        h,
        &h.chat,
        "uri:Ferroan Neighbour&24&128&35",
        n.region_id,
        Vec3::new(24., 128., 35.),
    )
    .await?;
    let result = async {
        let root = root_at(&mut home, 130.).await?;
        share_move(&mut home, root).await?;
        edit(&mut home, root, 264.).await?;
        let arrived = root_at(&mut east, 8.).await?;
        edit(&mut east, arrived, -8.).await?;
        root_at(&mut home, 248.).await?;
        Ok::<_, String>(())
    }
    .await;
    let (left, right) = tokio::join!(leave(h, &mut home, &h.owner), leave(h, &mut east, &h.chat));
    result?;
    left?;
    right?;
    Ok("object cross: linkset observed on both arrivals".into())
}
