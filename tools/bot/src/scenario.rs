// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenarios for `cargo xtask live run`. Each check uses a reply from the
//! region. The caller starts and stops the region and supplies the accounts.

mod animations;
pub use animations::steps as animation_steps;
pub mod child_view;
mod material_override;
#[path = "object_cross.rs"]
mod object_cross;
mod observer;
mod parcel_properties;
mod prim_arrival;
use observer::{ObservedAvatar, ObserverSession};
pub use parcel_properties::parcel_properties;

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ferroan_llsd::{Llsd, from_xml, to_xml};
use ferroan_protocol::Msg;
use ferroan_protocol::msgs::{AvatarAppearance, ObjectUpdate, UUIDNameRequest, uuid_name_request};
use glam::Vec3;
use uuid::Uuid;

use crate::login::{self, LoginRequest, LoginResponse};
use crate::register::Bot;
use crate::session::{Agent, PrimTrace, Session};
use crate::world::{self, SeenObject};

const HANDSHAKE: Duration = Duration::from_secs(30);
const STEP: Duration = Duration::from_millis(100);
const BODY_LIMIT: usize = 8 * 1024 * 1024;
const UPLOAD_NAME: &str = "Ferroan Harness Texture";

/// Number of parts in the persisted Pin8 prim-update capture scene.
pub const PRIM_CAPTURE_COUNT: usize = 8;
pub const PRIM_LOAD_COUNT: usize = 500;
pub const HANDOFF_TASK_BPS: f32 = 512_000.0;

/// Called with the account's principal before [`login::login`].
///
/// The harness installs this. A nil id is refused by the caller, and the hook
/// itself must not log in.
pub type LoginGate =
    Arc<dyn Fn(Uuid) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// Called with the account's principal at a [`Moment`] of that login. The
/// harness takes its `online` memory points here. The hook must return
/// quickly and must not log in or out.
pub type AfterLogin = Arc<dyn Fn(Uuid, Moment) + Send + Sync>;

/// When [`Harness::after_login`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    /// The login reached the region: the circuit is ready and the arrival
    /// handshake is done.
    Login,
    /// A stored-prim run's viewer has received every stored part, before it
    /// logs out.
    Scene,
    /// Both `roots` bots are in and have been driven for [`SETTLE`], before
    /// either logs out.
    Settled,
}

/// How long `roots` drives both bots before its [`Moment::Settled`] hook.
pub const SETTLE: Duration = Duration::from_secs(2);

/// One account the region already has. The password is not printed.
pub struct Account {
    pub first_name: String,
    pub last_name: String,
    pub password: String,
    /// GridUser and Presence key. Nil is refused before login.
    pub principal_id: Uuid,
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
    /// the seven names from the registration table.
    pub neighbour: Option<Neighbour>,
    /// Presence and GridUser listener. Robust in grid mode, the region in
    /// standalone. The hook reads it; the login itself uses [`Harness::login_uri`].
    pub service_probe: String,
    /// Runs before every [`enter`]. The harness waits for the account to settle.
    pub before_login: LoginGate,
    /// Runs at the end of every [`enter`] that succeeded, and once more when a
    /// stored-prim run has the whole scene. `None` does nothing.
    pub after_login: Option<AfterLogin>,
    /// Region metrics listener. Set for the Ferroan `roots` scenario.
    /// The C# region leaves this empty.
    pub metrics: Option<String>,
}

/// Region handle from map tiles, as `Util.RegionGridLocToHandle` packs it.
pub fn region_handle(x: u32, y: u32) -> u64 {
    (u64::from(x) << 40) | (u64::from(y) << 8)
}

/// The scenarios `all` runs, in order.
pub fn names() -> &'static [&'static str] {
    crate::register::names()
}

pub fn selection(name: &str) -> Option<&'static [&'static str]> {
    crate::register::selection(name)
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
    match crate::register::get(name).map(|row| row.bot) {
        Some(Bot::Caps) => {
            let (mut session, login) = enter(&http, harness, &harness.owner, "last").await?;
            let line = crate::caps_probe::run(&http, &mut session, &login.seed_capability).await?;
            leave(harness, &mut session, &harness.owner).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::Neighbour) => {
            let line = teleport_neighbour(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::Hop) => {
            let line = teleport_hop(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::Roots) => {
            let line = roots_scenario(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::Uploads) => {
            let line = uploads_scenario(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::ObjectCross) => object_cross::run(&http, harness).await,
        Some(Bot::PrimArrival) => prim_arrival::run(&http, harness).await,
        Some(Bot::MaterialOverride) => material_override::run(&http, harness).await,
        Some(Bot::Stored(kind)) => {
            let (count, throttle, handoff) = match kind {
                crate::register::Stored::Load => (
                    PRIM_LOAD_COUNT,
                    ArrivalThrottle::Task(PrimTrace::Load),
                    true,
                ),
                crate::register::Stored::Handoff => (
                    PRIM_CAPTURE_COUNT,
                    ArrivalThrottle::Task(PrimTrace::Arrival(HANDOFF_TASK_BPS)),
                    true,
                ),
                crate::register::Stored::Capture => {
                    (PRIM_CAPTURE_COUNT, ArrivalThrottle::Default, false)
                }
            };
            let (mut session, _) =
                enter_with_throttle(&http, harness, &harness.owner, "last", throttle).await?;
            let result = wait_for_stored_prims(&mut session, count).await;
            if result.is_ok()
                && let Some(hook) = &harness.after_login
            {
                hook(harness.owner.principal_id, Moment::Scene);
            }
            let logout = leave(harness, &mut session, &harness.owner).await;
            result?;
            logout?;
            if handoff {
                let records = session
                    .take_arrival_records()
                    .ok_or("missing handoff viewer capture")?;
                let bytes = ferroan_capture::write_capture(&records).map_err(|e| e.to_string())?;
                std::fs::write(harness.region_log.with_extension("viewer.cap"), bytes)
                    .map_err(|e| e.to_string())?;
            }
            Ok(format!(
                "{name}: viewer received {count} distinct full prim updates\nlive: run passed"
            ))
        }
        Some(Bot::Avatars) => {
            let line = avatars_scenario(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(Bot::Animations) => {
            let line = animations::run(&http, harness).await?;
            Ok(format!("{line}\nlive: run passed"))
        }
        Some(
            Bot::Login
            | Bot::Walk
            | Bot::Chat
            | Bot::Build
            | Bot::Teleport
            | Bot::Texture
            | Bot::Inventory
            | Bot::All
            | Bot::Assets,
        ) => run_selected(name, harness, &http).await,
        Some(
            Bot::ChildView
            | Bot::ParcelTiming
            | Bot::ParcelProperties
            | Bot::ParcelOverlay
            | Bot::Limits
            | Bot::Report
            | Bot::Ferrobot,
        )
        | None => Err(crate::register::bot_unknown(name)),
    }
}

async fn run_selected(
    name: &str,
    harness: &Harness,
    http: &reqwest::Client,
) -> Result<String, String> {
    let Some(names) = selection(name) else {
        return Err(crate::register::bot_unknown(name));
    };
    let mut lines = Vec::new();
    for scenario in names {
        let step = match crate::register::get(scenario).map(|row| row.bot) {
            Some(Bot::Login) => login_scenario(http, harness).await,
            Some(Bot::Walk) => walk_scenario(http, harness).await,
            Some(Bot::Chat) => chat_scenario(http, harness).await,
            Some(Bot::Build) => build_scenario(http, harness).await,
            Some(Bot::Teleport) => teleport_scenario(http, harness).await,
            Some(Bot::Texture) => texture_scenario(http, harness).await,
            Some(Bot::Inventory) => inventory_scenario(http, harness).await,
            _ => Err(format!("unknown scenario {scenario}")),
        };
        step.map(|line| lines.push(line)).map_err(|error| {
            lines.push(format!("{scenario}: fail ({error})"));
            lines.join("\n")
        })?;
    }
    if crate::register::get(name).is_some_and(|row| row.bot == Bot::All)
        && harness.neighbour.is_some()
    {
        teleport_neighbour(http, harness)
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

/// Observe control and limited Land on two fresh circuits, including the late initial-data gate.
pub async fn parcel_timing(harness: &Harness) -> Result<String, String> {
    let http = login::http_client().map_err(|e| e.to_string())?;
    for (account, rate) in [(&harness.owner, 512_000.0), (&harness.chat, 8_000.0)] {
        let (mut session, _) =
            enter_with_throttle(&http, harness, account, "last", ArrivalThrottle::Land(rate))
                .await?;
        let result = async {
            if session.region_id() != Some(harness.region_id) {
                return Err("parcel timing viewer arrived on another region".into());
            }
            let observation = Duration::from_secs(2);
            session
                .drive(observation)
                .await
                .map_err(|e| e.to_string())?;
            if rate == 8_000.0 {
                session.set_land_rate(Some(512_000.0));
                session
                    .send_throttles(None)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            receive_overlay(&mut session, 4).await?;
            // Continue acknowledging through the delayed initial-data gate and its Land drain.
            session.drive(observation).await.map_err(|e| e.to_string())
        }
        .await;
        let logout = leave(harness, &mut session, account).await;
        result.and(logout)?;
    }
    Ok("parcel-timing: captured two fresh circuits through logout".into())
}

/// Receive each test account's unredacted ParcelData bodies on a separate circuit.
pub async fn parcel_overlays(harness: &Harness, count: usize) -> Result<[Vec<u8>; 2], String> {
    if !matches!(count, 4 | 16) {
        return Err("parcel capture expects four or sixteen packets".into());
    }
    let http = login::http_client().map_err(|e| e.to_string())?;
    let mut captures = [Vec::new(), Vec::new()];
    for (out, account) in captures.iter_mut().zip([&harness.owner, &harness.chat]) {
        let (mut session, login) = enter(&http, harness, account, "last").await?;
        let result = if session.region_id() == Some(harness.region_id)
            && login.agent_id == account.principal_id
        {
            receive_overlay(&mut session, count).await
        } else {
            Err("parcel capture arrived on the wrong region or account".into())
        };
        let logout = leave(harness, &mut session, account).await;
        *out = result?;
        logout?;
    }
    Ok(captures)
}

async fn receive_overlay(session: &mut Session, count: usize) -> Result<Vec<u8>, String> {
    use ferroan_protocol::{Writer, msgs::ParcelOverlay};
    let mut bodies = vec![None; count];
    let mut received = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        session.drive(STEP).await.map_err(|e| e.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id != ParcelOverlay::ID {
                continue;
            }
            let msg = packet
                .decode::<ParcelOverlay>()
                .map_err(|e| e.to_string())?;
            let seq = msg.parcel_data.sequence_id;
            let mut check = Writer::new();
            msg.encode_body(&mut check);
            if check.buf != packet.body || msg.parcel_data.data.len() != 1024 {
                return Err(format!("ParcelOverlay {seq}: invalid captured body"));
            }
            let slot = usize::try_from(seq)
                .ok()
                .and_then(|i| bodies.get_mut(i))
                .ok_or_else(|| format!("out-of-range ParcelOverlay sequence {seq}"))?;
            if let Some(original) = slot {
                if original != &packet.body {
                    return Err(format!("conflicting ParcelOverlay {seq}"));
                }
            } else {
                *slot = Some(packet.body);
                received += 1;
            }
        }
        if received == count {
            return Ok(bodies.into_iter().flatten().flatten().collect());
        }
    }
    Err(format!(
        "viewer received {received} of {count} ParcelOverlay packets"
    ))
}

/// Wait for the full set of fixture parts, including the linked child.
async fn wait_for_stored_prims(session: &mut Session, count: usize) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let deadline = std::time::Instant::now()
        + Duration::from_secs(if count == PRIM_LOAD_COUNT { 60 } else { 15 });
    while std::time::Instant::now() < deadline {
        session
            .drive(STEP)
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id == ObjectUpdate::ID {
                let update = packet
                    .decode::<ObjectUpdate>()
                    .map_err(|error| error.to_string())?;
                for block in update.object_data {
                    if block.p_code != 47 {
                        ids.insert(block.full_id);
                        if ids.len() > count {
                            return Err("unexpected extra stored prim".into());
                        }
                    }
                }
            }
        }
        if ids.len() == count {
            return Ok(());
        }
    }
    Err(format!(
        "expected {count} stored prims, received {}",
        ids.len()
    ))
}

async fn login_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut session, _) = enter(http, harness, &harness.owner, "last").await?;
    let region = session.region_id().ok_or("the handshake set no region")?;
    if region != harness.region_id {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err(format!("region {region}"));
    }
    if let Err(error) = require_avatar(&mut session, &harness.owner).await {
        let _ = leave(harness, &mut session, &harness.owner).await;
        return Err(error);
    }
    leave(harness, &mut session, &harness.owner).await?;
    Ok("login: pass".to_string())
}

/// The avatar is not a cloud: one avatar `ObjectUpdate` and one unbaked
/// `AvatarAppearance` for this agent. Visual-param bytes come from the avatar
/// service, so this check does not require the constructor defaults.
async fn require_avatar(session: &mut Session, account: &Account) -> Result<(), String> {
    let agent = session.agent().agent_id;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut saw_object = false;
    let mut saw_appearance = false;
    while std::time::Instant::now() < deadline && !(saw_object && saw_appearance) {
        session
            .drive(Duration::from_millis(200))
            .await
            .map_err(|error| error.to_string())?;
        while let Some(packet) = session.poll_inbox() {
            if packet.id == ObjectUpdate::ID
                && let Ok(message) = packet.decode::<ObjectUpdate>()
            {
                saw_object |= message.object_data.iter().any(|object| {
                    object.p_code == 47
                        && object.material == 4
                        && object.full_id == agent
                        && object.object_data.len() == 76
                        && name_has(&object.name_value, &account.first_name)
                        && name_has(&object.name_value, &account.last_name)
                });
            }
            if packet.id == AvatarAppearance::ID
                && let Ok(message) = packet.decode::<AvatarAppearance>()
            {
                saw_appearance |= message.sender.id == agent
                    && !message.visual_param.is_empty()
                    && !message.object_data.texture_entry.is_empty()
                    && message.appearance_data.is_empty()
                    && message.appearance_hover.len() == 1;
            }
        }
    }
    if !saw_object || !saw_appearance {
        return Err(format!(
            "avatar appearance missing (object {saw_object}, appearance {saw_appearance})"
        ));
    }
    Ok(())
}

fn name_has(bytes: &[u8], name: &str) -> bool {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(|text| text.contains(name))
        .unwrap_or(false)
}

/// The current circuit's root-arrival data. Session identity stays private to
/// the test harness and must not be printed or included in public reports.
pub struct RootArrival {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub region_id: Uuid,
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub region_handle: u64,
}

/// Inspect service state after the actual arrival and before logout. Continue
/// driving the circuit during the bounded observer, then attempt logout even
/// when the observer fails. Ordinary scenarios do not need a service observer.
pub async fn run_login_observed<F, Fut>(harness: &Harness, observe: F) -> Result<String, String>
where
    F: FnOnce(RootArrival) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    harness.http_trace.scope(async {
        let http = login::http_client().map_err(|error| error.to_string())?;
        let (mut session, _) = enter(&http, harness, &harness.owner, "last").await?;
        let observed = async {
            let region = session.region_id().ok_or("the handshake set no region")?;
            if region != harness.region_id { return Err("arrival region differs from the test region".to_owned()); }
            let mut arrival = None;
            while let Some(packet) = session.poll_inbox() {
                if packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID {
                    let message = packet.decode::<ferroan_protocol::msgs::AgentMovementComplete>().map_err(|_| "invalid arrival message")?;
                    if message.agent_data.agent_id == session.agent().agent_id && message.agent_data.session_id == session.agent().session_id {
                        arrival = Some(message);
                    }
                }
            }
            let arrival = arrival.ok_or("the ready circuit has no current arrival message")?;
            let future = observe(RootArrival {
                agent_id: arrival.agent_data.agent_id,
                session_id: arrival.agent_data.session_id,
                region_id: region,
                position: arrival.data.position.to_array(),
                look_at: arrival.data.look_at.to_array(),
                region_handle: arrival.data.region_handle,
            });
            tokio::pin!(future);
            let deadline = tokio::time::sleep(Duration::from_secs(15));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    result = &mut future => break result,
                    _ = &mut deadline => break Err("online service observer exceeded fifteen seconds".to_owned()),
                    result = session.drive(STEP) => { result.map_err(|error| error.to_string())?; while session.poll_inbox().is_some() {} }
                }
            }
        }.await;
        let logout = leave(harness, &mut session, &harness.owner).await;
        match (observed, logout) {
            (Ok(line), Ok(())) => Ok(format!("login: pass\n{line}\nlive: run passed")),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(logout)) => Err(format!("{error}\nlogout: {logout}")),
        }
    }).await
}

async fn enter_observed_pair(
    http: &reqwest::Client,
    harness: &Harness,
    first: &Account,
    second: &Account,
) -> Result<(ObserverSession, ObserverSession), String> {
    let (session, _) = enter(http, harness, first, "last").await?;
    let mut observer = ObserverSession::new(session, second.principal_id);
    let (login, observation) = observer.during(enter(http, harness, second, "last")).await;
    let error = match (login, observation) {
        (Ok((session, _)), Ok(())) => {
            return Ok((observer, ObserverSession::new(session, first.principal_id)));
        }
        (Ok((mut session, _)), Err(error)) => {
            let _ = leave(harness, &mut session, second).await;
            error
        }
        (Err(error), observed) => observed.err().unwrap_or(error),
    };
    let _ = leave(harness, &mut observer.session, first).await;
    Err(error)
}

/// The staying viewer must witness the walker leaving on the same circuit.
/// Drive and log out both accounts even if a movement assertion failed.
async fn finish_observed_walk(
    harness: &Harness,
    mut watcher: ObserverSession,
    watcher_account: &Account,
    mut walker: ObserverSession,
    walker_account: &Account,
    check: Result<(), String>,
    check_minimap: bool,
) -> Result<(ObservedAvatar, usize), String> {
    let early = watcher.seen.peer_kills();
    let walker_id = watcher.seen.local_id;
    let mut leaving_kills = walker
        .seen
        .killed
        .iter()
        .filter(|&&id| Some(id) == walker_id)
        .count();
    let (left, observation) = watcher
        .during(leave_observing_before_request(
            harness,
            &mut walker.session,
            walker_account,
            |packet| {
                leaving_kills += world::killed(packet)
                    .iter()
                    .filter(|&&id| Some(id) == walker_id)
                    .count();
            },
        ))
        .await;
    drop(walker); // The leaving circuit ends at logout (D70).
    let departure = async {
        check?;
        observation?;
        left?;
        if early != 0 {
            return Err("observer saw the walker killed before logout".into());
        }
        watcher.seen.coarse = None;
        let alone = |view: &ObservedAvatar| {
            !check_minimap
                || view
                    .dots_match(
                        watcher_account.principal_id,
                        walker_account.principal_id,
                        &[watcher_account.principal_id],
                    )
                    .is_ok()
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !(watcher.seen.peer_kills() != 0 && alone(&watcher.seen))
            && std::time::Instant::now() < deadline
        {
            watcher.step().await?;
        }
        if watcher.seen.peer_kills() == 0 {
            return Err(format!(
                "no KillObject for the walker's local id {:?}",
                watcher.seen.local_id
            ));
        }
        // Observe late duplicate kills on the circuit that stays in the region.
        let settle = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < settle {
            watcher.step().await?;
        }
        watcher.seen.check_departure(leaving_kills)?;
        if check_minimap {
            watcher.seen.dots_match(
                watcher_account.principal_id,
                walker_account.principal_id,
                &[watcher_account.principal_id],
            )?;
        }
        Ok(())
    }
    .await;
    let logout = leave(harness, &mut watcher.session, watcher_account).await;
    departure?;
    logout?;
    Ok((watcher.seen, leaving_kills))
}

async fn walk_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut walker, mut watcher) =
        enter_observed_pair(http, harness, &harness.owner, &harness.chat).await?;
    let moved = observed_forward(&mut walker, &mut watcher).await;
    let (seen, leaving_kills) = finish_observed_walk(
        harness,
        watcher,
        &harness.chat,
        walker,
        &harness.owner,
        moved,
        false,
    )
    .await?;
    Ok(format!(
        "walk: pass (forward movement, observer kills={}, leaving viewer kills={leaving_kills})",
        seen.peer_kills()
    ))
}

async fn observed_forward(
    walker: &mut ObserverSession,
    watcher: &mut ObserverSession,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let (arrival, look, peer_start) = loop {
        walker.step().await?;
        watcher.step().await?;
        if let (Some((pos, look)), Some(_), Some(peer)) = (
            walker.seen.arrival,
            walker.seen.own_position,
            watcher.seen.position,
        ) {
            break (pos, look, peer);
        }
        if std::time::Instant::now() >= deadline {
            return Err("no walker arrival or matching full avatar update".into());
        }
    };
    let facing = world::facing_look(look);
    let forward = world::horizontal_forward(facing);
    let advanced = |pos: Vec3, start: Vec3| {
        Vec3::new(pos.x - start.x, pos.y - start.y, 0.0).dot(forward) >= 1.0
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut latest = walker.seen.own_position.unwrap_or(arrival);
    watcher.seen.peer_terse = None;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "forward walk timed out: own {arrival:?} to {latest:?}, observer {peer_start:?} to {:?}",
                watcher.seen.peer_terse
            ));
        }
        let update = world::agent_update(
            walker.session.agent(),
            latest,
            facing,
            world::CONTROL_AT_POS,
        );
        walker
            .session
            .send(&update, false)
            .await
            .map_err(|e| e.to_string())?;
        walker.step().await?;
        watcher.step().await?;
        latest = walker.seen.own_position.unwrap_or(latest);
        if advanced(latest, arrival)
            && watcher
                .seen
                .peer_terse
                .is_some_and(|p| advanced(p, peer_start))
        {
            break;
        }
    }
    let stop = world::agent_update(walker.session.agent(), latest, facing, 0);
    walker
        .session
        .send(&stop, false)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn chat_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    const SAY: &str = "ferroan harness chat";
    const DEBUG: &str = "ferroan harness debug";
    let (mut listener, _) = enter(http, harness, &harness.chat, "last").await?;
    let (mut speaker, _) = enter(http, harness, &harness.owner, "last").await?;
    let speaker_id = speaker.agent();
    for (channel, text) in [(0, SAY), (i32::MAX, DEBUG)] {
        let mut chat = world::say(speaker_id, text);
        chat.chat_data.channel = channel;
        speaker
            .send(&chat, true)
            .await
            .map_err(|error| error.to_string())?;
    }
    let mut heard_say = false;
    let mut heard_debug = false;
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
                && name.contains("Test")
                && name.contains("User")
            {
                heard_say |= chat_type == world::CHAT_SAY && message == SAY;
                heard_debug |= chat_type == 6 && message == DEBUG;
            }
        }
        if heard_say && heard_debug {
            break;
        }
    }
    let speaker_out = leave(harness, &mut speaker, &harness.owner).await;
    let listener_out = leave(harness, &mut listener, &harness.chat).await;
    if !heard_say {
        return Err("the second bot did not hear the say on channel 0".to_string());
    }
    if !heard_debug {
        return Err("the second bot did not hear type 6 on DEBUG_CHANNEL".to_string());
    }
    speaker_out?;
    listener_out?;
    Ok("chat: pass".to_string())
}

/// Both harness accounts in the region at once. No chat is sent or required.
async fn roots_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut chat, _) = enter(http, harness, &harness.chat, "last").await?;
    refuse_chat(&mut chat)?;
    let owner_login = enter(http, harness, &harness.owner, "last");
    tokio::pin!(owner_login);
    let owner_result = loop {
        tokio::select! {
            result = &mut owner_login => break result,
            step = chat.drive(STEP) => {
                step.map_err(|error| error.to_string())?;
                refuse_chat(&mut chat)?;
            }
        }
    };
    let (mut owner, _) = match owner_result {
        Ok(pair) => pair,
        Err(error) => {
            let _ = leave(harness, &mut chat, &harness.chat).await;
            return Err(error);
        }
    };
    refuse_chat(&mut owner)?;
    refuse_chat(&mut chat)?;
    if let Some(hook) = &harness.after_login {
        if let Err(error) = drive_both(&mut chat, &mut owner, SETTLE).await {
            let _ = leave_while_driving(harness, &mut chat, &harness.chat, &mut owner).await;
            let _ = leave(harness, &mut owner, &harness.owner).await;
            return Err(error);
        }
        hook(harness.owner.principal_id, Moment::Settled);
    }
    if let Some(metrics) = &harness.metrics {
        let mut sessions: [&mut Session; 2] = [&mut chat, &mut owner];
        if let Err(error) = await_root_agents(http, metrics, 2, &mut sessions).await {
            let _ = leave_while_driving(harness, &mut chat, &harness.chat, &mut owner).await;
            let _ = leave(harness, &mut owner, &harness.owner).await;
            return Err(error);
        }
    }
    leave_while_driving(harness, &mut chat, &harness.chat, &mut owner).await?;
    if let Some(metrics) = &harness.metrics {
        let mut sessions: [&mut Session; 1] = [&mut owner];
        await_root_agents(http, metrics, 1, &mut sessions).await?;
    }
    leave(harness, &mut owner, &harness.owner).await?;
    if let Some(metrics) = &harness.metrics {
        await_root_agents(http, metrics, 0, &mut []).await?;
    }
    Ok("roots: pass".to_string())
}

/// Both harness accounts upload one texture while they are in the region together.
async fn uploads_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut owner, owner_login) = enter(http, harness, &harness.owner, "last").await?;
    let Some(owner_folder) = textures_folder(&owner_login) else {
        let _ = leave(harness, &mut owner, &harness.owner).await;
        return Err("the owner login reply has no Textures folder".to_string());
    };
    let chat_login = enter(http, harness, &harness.chat, "last");
    tokio::pin!(chat_login);
    let chat_result = loop {
        tokio::select! {
            result = &mut chat_login => break result,
            step = owner.drive(STEP) => {
                step.map_err(|error| error.to_string())?;
            }
        }
    };
    let (mut chat, chat_login) = match chat_result {
        Ok(pair) => pair,
        Err(error) => {
            let _ = leave(harness, &mut owner, &harness.owner).await;
            return Err(error);
        }
    };
    let Some(chat_folder) = textures_folder(&chat_login) else {
        let _ = leave(harness, &mut chat, &harness.chat).await;
        let _ = leave(harness, &mut owner, &harness.owner).await;
        return Err("the second login reply has no Textures folder".to_string());
    };
    let owner_upload = upload_texture(http, &owner, owner_folder, &harness.texture).await;
    if let Err(error) = owner.drive(STEP).await {
        let _ = leave(harness, &mut chat, &harness.chat).await;
        let _ = leave(harness, &mut owner, &harness.owner).await;
        return Err(error.to_string());
    }
    let chat_upload = upload_texture(http, &chat, chat_folder, &harness.texture).await;
    let owner_asset = match owner_upload {
        Ok(asset) => asset,
        Err(error) => {
            let _ = leave(harness, &mut chat, &harness.chat).await;
            let _ = leave(harness, &mut owner, &harness.owner).await;
            return Err(format!("owner upload: {error}"));
        }
    };
    let chat_asset = match chat_upload {
        Ok(asset) => asset,
        Err(error) => {
            let _ = leave(harness, &mut chat, &harness.chat).await;
            let _ = leave(harness, &mut owner, &harness.owner).await;
            return Err(format!("second upload: {error}"));
        }
    };
    if owner_asset == chat_asset {
        let _ = leave(harness, &mut chat, &harness.chat).await;
        let _ = leave(harness, &mut owner, &harness.owner).await;
        return Err("both uploads returned the same asset".to_string());
    }
    let owner_bytes = fetch_texture(http, &owner, owner_asset).await;
    let chat_bytes = fetch_texture(http, &chat, chat_asset).await;
    let chat_out = leave(harness, &mut chat, &harness.chat).await;
    let owner_out = leave(harness, &mut owner, &harness.owner).await;
    let owner_bytes = owner_bytes?;
    let chat_bytes = chat_bytes?;
    chat_out?;
    owner_out?;
    if owner_bytes != harness.texture || chat_bytes != harness.texture {
        return Err(format!(
            "uploaded bytes came back as {} and {}, sent {}",
            owner_bytes.len(),
            chat_bytes.len(),
            harness.texture.len()
        ));
    }
    Ok(format!(
        "uploads: pass ({} bytes, two avatars)",
        harness.texture.len()
    ))
}

/// Both harness accounts in the region together. Each bot gets the other's
/// full avatar `ObjectUpdate` and `AvatarAppearance`: the one already there
/// when the other arrives, the newcomer once its initial data is sent. Then
/// the second bot walks east and stops: on both minimaps
/// (`CoarseLocationUpdate`) its dot follows, and each dot ends at the cell of
/// the position that bot last saw. When the second bot leaves, the first
/// gets `KillObject` for the local id it saw, and its minimap lists only
/// itself.
async fn avatars_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut owner, mut chat) =
        enter_observed_pair(http, harness, &harness.owner, &harness.chat).await?;
    let checked = check_avatars(harness, &mut owner, &mut chat).await;
    let (owner_saw, _) = finish_observed_walk(
        harness,
        owner,
        &harness.owner,
        chat,
        &harness.chat,
        checked,
        true,
    )
    .await?;
    let cells = |cells: &[(u8, u8)]| {
        cells
            .iter()
            .map(|(x, y)| format!("{x},{y}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Ok(format!(
        "avatars: pass (each bot saw the other's local id, appearance and minimap dot and got its name; the unknown id was answered only from the cached miss; the dot followed a walk; KillObject and no dot on leave)\nminimap: the owner saw the second bot's dot at {}",
        cells(&owner_saw.other_cells)
    ))
}

async fn check_avatars(
    harness: &Harness,
    owner: &mut ObserverSession,
    chat: &mut ObserverSession,
) -> Result<(), String> {
    let owner_id = harness.owner.principal_id;
    let chat_id = harness.chat.principal_id;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !(owner.seen.seen() && chat.seen.seen()) && std::time::Instant::now() < deadline {
        owner.step().await?;
        chat.step().await?;
    }
    let early_kill = [owner.seen.local_id, chat.seen.local_id]
        .into_iter()
        .flatten()
        .any(|id| owner.seen.killed.contains(&id) || chat.seen.killed.contains(&id));
    if !(owner.seen.seen() && chat.seen.seen()) || early_kill {
        if early_kill {
            return Err("an avatar was killed while both were in the region".to_string());
        }
        return Err(format!(
            "the bots did not see each other (owner saw full {:?}, appearance {}; second bot saw full {:?}, appearance {})",
            owner.seen.local_id,
            owner.seen.seen(),
            chat.seen.local_id,
            chat.seen.seen()
        ));
    }
    let walked = walk_east(chat, owner).await;
    let dots = walked.and_then(|start| {
        owner.seen.dots_match(owner_id, chat_id, &[owner_id, chat_id])?;
        chat.seen.dots_match(chat_id, owner_id, &[owner_id, chat_id])?;
        let from = (start.x as i32) as u8;
        let walked_to = |cells: &[(u8, u8)]| cells.last().map(|(x, _)| *x);
        let far = |cell: Option<u8>| cell.is_some_and(|x| x >= from.saturating_add(2));
        if !far(walked_to(&owner.seen.other_cells)) || !far(walked_to(&chat.seen.own_cells)) {
            return Err(format!(
                "the walking bot's dot did not follow it east of cell {from} (owner saw cells {:?}, second bot saw its own at {:?})",
                owner.seen.other_cells, chat.seen.own_cells
            ));
        }
        Ok(())
    });
    dots.map_err(|e| format!("minimap: {e}"))?;
    exchange_names(harness, owner, chat).await
}

/// The walking bot goes east under forward control until it is 2.5 m from
/// where it stood, or eight seconds pass, then stops. After a second at rest
/// both bots wait for their next minimap update, which the region sends
/// every 50 frames (about 4.5 s). Both circuits are driven throughout.
/// Returns where the walk started.
async fn walk_east(
    walker: &mut ObserverSession,
    watcher: &mut ObserverSession,
) -> Result<Vec3, String> {
    let start = walker
        .seen
        .own_position
        .ok_or("the walking bot has not seen its own avatar")?;
    let facing = glam::Quat::IDENTITY;
    let mut latest = start;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline && latest.x - start.x < 2.5 {
        let update = world::agent_update(
            walker.session.agent(),
            latest,
            facing,
            world::CONTROL_AT_POS,
        );
        walker
            .session
            .send(&update, false)
            .await
            .map_err(|error| error.to_string())?;
        walker.step().await?;
        watcher.step().await?;
        latest = walker.seen.own_position.unwrap_or(latest);
    }
    if latest.x - start.x < 2.5 {
        return Err(format!(
            "the walking bot stayed near {:.1},{:.1} under forward control",
            latest.x, latest.y
        ));
    }
    let stopped = std::time::Instant::now();
    let settle = stopped + Duration::from_secs(1);
    let deadline = stopped + Duration::from_secs(12);
    let mut fresh = false;
    while std::time::Instant::now() < deadline {
        if !fresh && std::time::Instant::now() >= settle {
            // Only a list made after the walker stopped counts.
            walker.seen.coarse = None;
            watcher.seen.coarse = None;
            fresh = true;
        }
        if fresh && walker.seen.coarse.is_some() && watcher.seen.coarse.is_some() {
            break;
        }
        // `AgentUpdate` is unreliable: repeat the stop as a viewer does.
        let stop = world::agent_update(walker.session.agent(), latest, facing, 0);
        walker
            .session
            .send(&stop, false)
            .await
            .map_err(|error| error.to_string())?;
        walker.step().await?;
        watcher.step().await?;
        latest = walker.seen.own_position.unwrap_or(latest);
    }
    Ok(start)
}

/// An id no harness grid has: `UUIDNameRequest` for it gets no reply.
pub const UNKNOWN_NAME_ID: Uuid = Uuid::from_u128(0x0f0e_0d0c_0b0a_4984_8984_0102_0304_0506);

fn name_request(ids: &[Uuid]) -> UUIDNameRequest {
    let mut request = UUIDNameRequest::default();
    for id in ids {
        request
            .uuid_name_block
            .push(uuid_name_request::UUIDNameBlock { id: *id });
    }
    request
}

/// Each bot asks for the other's name and for [`UNKNOWN_NAME_ID`], in one
/// `UUIDNameRequest`. Each must get one `UUIDNameReply` with the other's
/// account names. The first request for the unknown id to be looked up gets
/// no reply; the other bot's may come after that miss is cached and then
/// gets `Unknown` and the id (TB89), so at most one of the two is answered,
/// only that way. Then the owner asks for the unknown id again and must get
/// `Unknown` and the id from the cached miss.
async fn exchange_names(
    harness: &Harness,
    owner: &mut ObserverSession,
    chat: &mut ObserverSession,
) -> Result<(), String> {
    let owner_id = harness.owner.principal_id;
    let chat_id = harness.chat.principal_id;
    owner
        .session
        .send(&name_request(&[chat_id, UNKNOWN_NAME_ID]), true)
        .await
        .map_err(|error| error.to_string())?;
    chat.session
        .send(&name_request(&[owner_id, UNKNOWN_NAME_ID]), true)
        .await
        .map_err(|error| error.to_string())?;
    let mut settle_until = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        owner.step().await?;
        chat.step().await?;
        unknown_answers(&owner.seen, &chat.seen, 1)?;
        let both = !owner.seen.named(chat_id).is_empty() && !chat.seen.named(owner_id).is_empty();
        match settle_until {
            None if both => {
                settle_until = Some(std::time::Instant::now() + Duration::from_secs(1));
            }
            Some(until) if std::time::Instant::now() >= until => break,
            _ => {}
        }
        if settle_until.is_none() && std::time::Instant::now() >= deadline {
            return Err(format!(
                "UUIDNameReply did not name the other bot (owner got {:?}, second bot got {:?})",
                owner.seen.names, chat.seen.names
            ));
        }
    }
    for (saw, other, account) in [
        (&owner.seen, chat_id, &harness.chat),
        (&chat.seen, owner_id, &harness.owner),
    ] {
        let named = saw.named(other);
        let want = (account.first_name.clone(), account.last_name.clone());
        if named != [want.clone()] {
            return Err(format!(
                "UUIDNameReply named {other} as {named:?}, not once as {want:?}"
            ));
        }
    }
    let before = owner.seen.named(UNKNOWN_NAME_ID).len();
    owner
        .session
        .send(&name_request(&[UNKNOWN_NAME_ID]), true)
        .await
        .map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while owner.seen.named(UNKNOWN_NAME_ID).len() == before && std::time::Instant::now() < deadline
    {
        owner.step().await?;
        chat.step().await?;
    }
    if owner.seen.named(UNKNOWN_NAME_ID).len() != before + 1 {
        return Err(format!(
            "the second request for the unknown id got {:?}, not Unknown and the id from the cached miss",
            owner.seen.named(UNKNOWN_NAME_ID)
        ));
    }
    unknown_answers(&owner.seen, &chat.seen, 2)
}

/// Every reply for [`UNKNOWN_NAME_ID`] is `Unknown` and the id, and the two
/// bots got at most `most` of them.
fn unknown_answers(
    owner: &ObservedAvatar,
    chat: &ObservedAvatar,
    most: usize,
) -> Result<(), String> {
    let want = ("Unknown".to_string(), UNKNOWN_NAME_ID.to_string());
    let answers: Vec<_> = owner
        .named(UNKNOWN_NAME_ID)
        .into_iter()
        .chain(chat.named(UNKNOWN_NAME_ID))
        .collect();
    if answers.len() > most || answers.iter().any(|answer| *answer != want) {
        return Err(format!(
            "the region answered the unknown id {answers:?}; at most {most} reply, each Unknown and the id"
        ));
    }
    Ok(())
}

/// Drive both circuits for `span`, refusing any chat either one hears.
async fn drive_both(
    first: &mut Session,
    second: &mut Session,
    span: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + span;
    while tokio::time::Instant::now() < deadline {
        // In turn, so neither drive is cancelled part way.
        first
            .drive(STEP / 2)
            .await
            .map_err(|error| error.to_string())?;
        second
            .drive(STEP / 2)
            .await
            .map_err(|error| error.to_string())?;
        refuse_chat(first)?;
        refuse_chat(second)?;
    }
    Ok(())
}

fn refuse_chat(session: &mut Session) -> Result<(), String> {
    while let Some(packet) = session.poll_inbox() {
        if world::chat_heard(&packet).is_some() {
            return Err("the region sent ChatFromSimulator".to_string());
        }
    }
    Ok(())
}

async fn await_root_agents(
    http: &reqwest::Client,
    metrics: &str,
    want: u64,
    sessions: &mut [&mut Session],
) -> Result<(), String> {
    let root = format!("ferroan_region_root_agents {want}\n");
    let circuits = format!("ferroan_region_circuits {want}\n");
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let url = format!("{}/metrics", metrics.trim_end_matches('/'));
    loop {
        let response = http
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|error| format!("metrics: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("metrics status {}", response.status()));
        }
        let body = response
            .text()
            .await
            .map_err(|error| format!("metrics: {error}"))?;
        if body.contains(&root)
            && body.contains(&circuits)
            && body.contains("ferroan_region_child_agents 0\n")
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "ferroan_region_root_agents was not {want} with child agents 0"
            ));
        }
        if sessions.is_empty() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for session in sessions.iter_mut() {
            session
                .drive(STEP)
                .await
                .map_err(|error| error.to_string())?;
            refuse_chat(session)?;
        }
    }
}

async fn leave_while_driving(
    harness: &Harness,
    leaving: &mut Session,
    account: &Account,
    staying: &mut Session,
) -> Result<(), String> {
    let logout = leave(harness, leaving, account);
    tokio::pin!(logout);
    loop {
        tokio::select! {
            result = &mut logout => return result,
            step = staying.drive(STEP) => {
                step.map_err(|error| error.to_string())?;
                refuse_chat(staying)?;
            }
        }
    }
}

async fn build_scenario(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut owner, _) = enter(http, harness, &harness.owner, "last").await?;
    let (mut other, _) = enter(http, harness, &harness.chat, "last").await?;
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
    let (session, _) = enter(http, harness, &harness.owner, "last").await?;
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

/// One hop onto the neighbour. The source keeps the child circuit.
/// The return hop is a separate check.
async fn teleport_hop(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let neighbour = harness
        .neighbour
        .as_ref()
        .ok_or("neighbour hop needs a grid-mode harness")?;
    let (session, _) = enter(http, harness, &harness.owner, "last").await?;
    let mut parked: Vec<Session> = Vec::new();
    let away = cross_to(
        http,
        session,
        region_handle(neighbour.x, neighbour.y),
        &mut parked,
    )
    .await?;
    if away.region_id() != Some(neighbour.region_id) {
        return Err(format!(
            "landed in {:?}, expected {}",
            away.region_id(),
            neighbour.region_id
        ));
    }
    let mut away = away;
    leave(harness, &mut away, &harness.owner).await?;
    drop(parked);
    Ok(format!(
        "teleport hop: pass ({},{})",
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
            let Some(task) = poll.take() else {
                return Err("event-queue poll finished without a task".to_string());
            };
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
    let (mut session, _) = enter(http, harness, &harness.owner, "last").await?;
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
    let (mut session, login) = enter(http, harness, &harness.owner, "last").await?;
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
    let (mut session, login) = enter(http, harness, &harness.owner, "last").await?;
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
    start: &str,
) -> Result<(Session, LoginResponse), String> {
    enter_with_throttle(http, harness, account, start, ArrivalThrottle::Default).await
}

enum ArrivalThrottle {
    Default,
    Task(PrimTrace),
    Land(f32),
}

async fn enter_with_throttle(
    http: &reqwest::Client,
    harness: &Harness,
    account: &Account,
    start: &str,
    throttle: ArrivalThrottle,
) -> Result<(Session, LoginResponse), String> {
    enter_prepared(http, harness, account, start, throttle, |_| {}).await
}

async fn enter_prepared(
    http: &reqwest::Client,
    harness: &Harness,
    account: &Account,
    start: &str,
    throttle: ArrivalThrottle,
    prepare: impl FnOnce(&mut Session),
) -> Result<(Session, LoginResponse), String> {
    if account.principal_id.is_nil() {
        return Err("the account has no principal id".to_string());
    }
    (harness.before_login)(account.principal_id).await?;
    let response = login::login(
        http,
        &LoginRequest {
            login_uri: harness.login_uri.clone(),
            username: format!("{} {}", account.first_name, account.last_name),
            password: account.password.clone(),
            start: start.to_owned(),
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
    match throttle {
        ArrivalThrottle::Default => {}
        ArrivalThrottle::Task(trace) => session.capture_prim_arrival(trace),
        ArrivalThrottle::Land(rate) => session.set_land_rate(Some(rate)),
    }
    prepare(&mut session);
    session
        .run_until_ready(http, &response.seed_capability, HANDSHAKE)
        .await
        .map_err(|error| error.to_string())?;
    crate::redact_scan::scan_login(
        &account.password,
        &response.session_id.to_string(),
        &response.secure_session_id.to_string(),
        &response.seed_capability,
        &harness.region_log,
    )?;
    if let Some(hook) = &harness.after_login {
        hook(account.principal_id, Moment::Login);
    }
    Ok((session, response))
}

async fn leave(harness: &Harness, session: &mut Session, account: &Account) -> Result<(), String> {
    leave_observing_before_request(harness, session, account, |_| {}).await
}

async fn leave_observing_before_request(
    harness: &Harness,
    session: &mut Session,
    account: &Account,
    mut record: impl FnMut(&ferroan_protocol::IncomingPacket),
) -> Result<(), String> {
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
        while let Some(packet) = session.poll_inbox() {
            record(&packet);
        }
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
    // Stop recording when we send LogoutRequest. Pin8 can send its departing
    // viewer a kill for its own avatar during teardown (TB87).
    let logout = session.logout(Duration::from_secs(2)).await;
    if logout.is_ok() {
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
        assert!(selection("hop").is_none());
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
