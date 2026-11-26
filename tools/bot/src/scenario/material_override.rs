// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Two viewers on the stored scene. Only the control viewer asks for PBR.
//! Pin8 c401bceecd1684832ae8ba86a31b5ee01a4ded7b,
//! Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs, needMaterials.

use super::{
    ArrivalThrottle, Harness, ObserverSession, PRIM_CAPTURE_COUNT, PrimTrace, enter_prepared,
    leave, world,
};
use std::time::{Duration, Instant};

const NORMAL_TASK_BPS: f32 = 512_000.0;
const PBR_CAP: &str = "ModifyMaterialParams";

fn part_count(observer: &ObserverSession) -> Result<usize, String> {
    observer
        .seen
        .prim_arrival
        .as_ref()
        .map(|seen| seen.len())
        .ok_or_else(|| "missing prim observer".into())
}

async fn hold(observer: &mut ObserverSession) -> Result<(), String> {
    if let Some((pos, look)) = observer.seen.arrival {
        let update = world::agent_update(
            observer.session.agent(),
            observer.seen.own_position.unwrap_or(pos),
            world::facing_look(look),
            0,
        );
        observer
            .session
            .send(&update, false)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn collect(
    control: &mut ObserverSession,
    limited: &mut ObserverSession,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut ready = None;
    while Instant::now() < deadline {
        let (a, b) = tokio::join!(control.step(), limited.step());
        a?;
        b?;
        hold(control).await?;
        hold(limited).await?;
        let now = Instant::now();
        if part_count(control)? == PRIM_CAPTURE_COUNT
            && part_count(limited)? == PRIM_CAPTURE_COUNT
            && control.session.material_messages() >= 1
        {
            let since = *ready.get_or_insert(now);
            if now.duration_since(since) >= Duration::from_secs(1) {
                if limited.session.material_messages() != 0 {
                    return Err(
                        "the viewer that did not request PBR received a material override".into(),
                    );
                }
                return Ok(());
            }
        }
    }
    Err(format!(
        "material override timed out: control parts {}, limited parts {}, control materials {}, limited materials {}",
        part_count(control)?,
        part_count(limited)?,
        control.session.material_messages(),
        limited.session.material_messages()
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
                .with_file_name(format!("material-override-{alias}.cap")),
            bytes,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) async fn run(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (session, _) = enter_prepared(
        http,
        harness,
        &harness.owner,
        "last",
        ArrivalThrottle::Task(PrimTrace::Arrival(NORMAL_TASK_BPS)),
        |session| {
            session.record_material_messages();
            session.request_seed_cap(PBR_CAP);
        },
    )
    .await?;
    let mut control = ObserverSession::new(session, harness.chat.principal_id);
    control.seen.prim_arrival = Some(super::prim_arrival::ObservedPrims::default());
    let (login, observations) = control
        .during(enter_prepared(
            http,
            harness,
            &harness.chat,
            "last",
            ArrivalThrottle::Task(PrimTrace::Arrival(NORMAL_TASK_BPS)),
            |session| session.record_material_messages(),
        ))
        .await;
    let result = match login {
        Ok((session, _)) => {
            let mut limited = ObserverSession::new(session, harness.owner.principal_id);
            limited.seen.prim_arrival = Some(super::prim_arrival::ObservedPrims::default());
            let observation = async {
                observations?;
                collect(&mut control, &mut limited).await
            }
            .await;
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
    Ok(
        "material-override: the PBR viewer received method 16757 and the other viewer did not\nlive: run passed"
            .into(),
    )
}
