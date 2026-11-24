// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One port base for every listener a live run opens.
//!
//! `FERROAN_LIVE_PORT_BASE` or `--port-base` selects the base. Every harness
//! listener, C# Robust and region port, Ferroan service, capture proxy and
//! Docker smoke port is `base + offset`, and the run's files live under
//! `.scratch/live/runs/<base>-<pid>/`. A later command with the same base
//! finds that directory. With no setting, ephemeral listeners
//! stay ephemeral and Docker smoke stays on 3100, 9095, 4317 and 4318.
//!
//! A set base reserves the free ports in its block for this process. Those
//! ports sit inside the kernel's ephemeral range, so a client connection (the
//! asset service's database session, a capture proxy) can otherwise occupy a
//! slot before the server binds it. The reservation for a slot is released
//! when that slot is allocated. A port something else already holds stays
//! unreserved: the kernel will not hand it out while it is taken, and a later
//! allocation of that slot still fails if it is busy. A follow-up command
//! such as `live down` starts while those servers are listening. Before this
//! process starts another xtask on the same base, it drops the whole
//! reservation so the child can reserve the block itself.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

pub(crate) const ENV: &str = "FERROAN_LIVE_PORT_BASE";

/// Width of one run's block. Adjacent bases this far apart never share a port.
pub(crate) const SPAN: u16 = 32;

const DYNAMIC: u8 = 18;

const CANONICAL_HOME: u16 = 9000;
const CANONICAL_NEIGHBOUR: u16 = 9001;
const CANONICAL_PUBLIC_PROXY: u16 = 8002;
const CANONICAL_PRIVATE_PROXY: u16 = 8003;
const CANONICAL_LOKI: u16 = 3100;
const CANONICAL_LOKI_GRPC: u16 = 9095;
const CANONICAL_OTEL_GRPC: u16 = 4317;
const CANONICAL_OTEL_HTTP: u16 = 4318;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum Slot {
    RobustPublic = 0,
    RobustPrivate = 1,
    PublicProxy = 2,
    PrivateProxy = 3,
    Home = 4,
    Neighbour = 5,
    RustService = 6,
    RegionMetrics = 7,
    RobustMetrics = 8,
    StandalonePublic = 9,
    StandalonePrivate = 10,
    ConsolePublic = 11,
    ConsolePrivate = 12,
    FerrobotHttp = 13,
    Loki = 14,
    LokiGrpc = 15,
    OtelGrpc = 16,
    OtelHttp = 17,
}

impl Slot {
    fn from_offset(offset: u8) -> Option<Self> {
        Some(match offset {
            0 => Self::RobustPublic,
            1 => Self::RobustPrivate,
            2 => Self::PublicProxy,
            3 => Self::PrivateProxy,
            4 => Self::Home,
            5 => Self::Neighbour,
            6 => Self::RustService,
            7 => Self::RegionMetrics,
            8 => Self::RobustMetrics,
            9 => Self::StandalonePublic,
            10 => Self::StandalonePrivate,
            11 => Self::ConsolePublic,
            12 => Self::ConsolePrivate,
            13 => Self::FerrobotHttp,
            14 => Self::Loki,
            15 => Self::LokiGrpc,
            16 => Self::OtelGrpc,
            17 => Self::OtelHttp,
            _ => return None,
        })
    }

    /// Fixture port this listener is rewritten to. Absent means the compare
    /// never carries the number, so it is not rewritten.
    pub(crate) fn canonical(self) -> Option<u16> {
        Some(match self {
            Self::Home | Self::StandalonePublic | Self::ConsolePublic => CANONICAL_HOME,
            Self::Neighbour => CANONICAL_NEIGHBOUR,
            Self::PublicProxy => CANONICAL_PUBLIC_PROXY,
            Self::PrivateProxy => CANONICAL_PRIVATE_PROXY,
            Self::Loki => CANONICAL_LOKI,
            Self::LokiGrpc => CANONICAL_LOKI_GRPC,
            Self::OtelGrpc => CANONICAL_OTEL_GRPC,
            Self::OtelHttp => CANONICAL_OTEL_HTTP,
            Self::RobustPublic
            | Self::RobustPrivate
            | Self::RustService
            | Self::RegionMetrics
            | Self::RobustMetrics
            | Self::StandalonePrivate
            | Self::ConsolePrivate
            | Self::FerrobotHttp => return None,
        })
    }

    /// Ports that are fixed today. Everything else asks the OS for a free port.
    fn today(self) -> Option<u16> {
        match self {
            Self::Loki => Some(CANONICAL_LOKI),
            Self::LokiGrpc => Some(CANONICAL_LOKI_GRPC),
            Self::OtelGrpc => Some(CANONICAL_OTEL_GRPC),
            Self::OtelHttp => Some(CANONICAL_OTEL_HTTP),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) base: Option<u16>,
}

impl Plan {
    pub(crate) fn unset() -> Self {
        Self { base: None }
    }

    pub(crate) fn from_raw(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("") => Ok(Self::unset()),
            Some(text) => {
                let base: u16 = text.parse().map_err(|_| format!("{ENV} needs a port"))?;
                validate_base(base)?;
                Ok(Self { base: Some(base) })
            }
        }
    }

    pub(crate) fn resolve(flag: Option<u16>) -> Result<Self, String> {
        if let Some(base) = flag {
            return Self::from_raw(Some(&base.to_string()));
        }
        match std::env::var(ENV) {
            Ok(value) => Self::from_raw(Some(&value)),
            Err(_) => Self::from_raw(None),
        }
    }

    pub(crate) fn port_of(self, slot: Slot) -> Option<u16> {
        self.base.map(|base| base + u16::from(slot as u8))
    }

    /// The port a slot uses for this plan. `None` means "ask for a free port".
    pub(crate) fn today_or_base(self, slot: Slot) -> Option<u16> {
        self.port_of(slot).or_else(|| slot.today())
    }
}

fn validate_base(base: u16) -> Result<(), String> {
    if base < 1024 {
        return Err(format!("{ENV} {base} is below 1024"));
    }
    if u32::from(base) + u32::from(SPAN) - 1 > u32::from(u16::MAX) {
        return Err(format!("{ENV} {base} does not leave room for {SPAN} ports"));
    }
    Ok(())
}

static PLAN: OnceLock<Plan> = OnceLock::new();
static ASSIGNED: Mutex<Vec<(u8, u16)>> = Mutex::new(Vec::new());
static DYNAMIC_NEXT: Mutex<u8> = Mutex::new(DYNAMIC);
static RUN_ROOT: OnceLock<PathBuf> = OnceLock::new();

struct ReservedBlock {
    base: u16,
    listeners: Vec<Option<TcpListener>>,
}

static RESERVED: Mutex<Option<ReservedBlock>> = Mutex::new(None);

pub(crate) fn install(flag: Option<u16>) -> Result<(), String> {
    if PLAN.get().is_some() {
        return Err("the live port base is already set".to_string());
    }
    let plan = Plan::resolve(flag)?;
    if let Some(base) = plan.base {
        reserve_block(base)?;
    }
    PLAN.set(plan)
        .map_err(|_| "the live port base is already set".to_string())?;
    Ok(())
}

fn slot_port(base: u16, offset: usize) -> Result<u16, String> {
    let offset = u16::try_from(offset).map_err(|_| format!("port offset {offset} does not fit"))?;
    base.checked_add(offset)
        .ok_or_else(|| format!("port base {base} has no room at offset {offset}"))
}

/// Bind every free port in the block so the kernel will not hand one out as
/// an ephemeral client port. A busy port is retried briefly, then left
/// unreserved. Replacing the stored block drops any listeners this process
/// still held.
fn reserve_block(base: u16) -> Result<(), String> {
    reserve_block_for(base, std::time::Duration::from_secs(5))
}

fn reserve_block_for(base: u16, wait: std::time::Duration) -> Result<(), String> {
    let mut listeners = std::iter::repeat_with(|| None)
        .take(usize::from(SPAN))
        .collect::<Vec<Option<TcpListener>>>();
    let mut pending: Vec<usize> = (0..usize::from(SPAN)).collect();
    let deadline = std::time::Instant::now() + wait;
    while !pending.is_empty() {
        let mut still = Vec::new();
        for offset in pending {
            let port = slot_port(base, offset)?;
            match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                Ok(listener) => listeners[offset] = Some(listener),
                Err(_) if std::time::Instant::now() >= deadline => {}
                Err(_) => still.push(offset),
            }
        }
        if still.is_empty() || std::time::Instant::now() >= deadline {
            break;
        }
        pending = still;
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    *RESERVED.lock().unwrap_or_else(|poison| poison.into_inner()) =
        Some(ReservedBlock { base, listeners });
    Ok(())
}

/// Drop every listener this process reserved. A nested xtask with the same
/// base reserves the block itself.
pub(crate) fn release_block() {
    *RESERVED.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
}

/// Drop the reservation for `port` when this process holds it.
fn release_reservation(port: u16) -> bool {
    let mut reserved = RESERVED.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(block) = reserved.as_mut() else {
        return false;
    };
    let Some(offset) = port.checked_sub(block.base) else {
        return false;
    };
    if offset >= SPAN {
        return false;
    }
    block
        .listeners
        .get_mut(usize::from(offset))
        .and_then(Option::take)
        .is_some()
}

pub(crate) fn current() -> Plan {
    PLAN.get().copied().unwrap_or_else(Plan::unset)
}

pub(crate) fn isolated() -> bool {
    current().base.is_some()
}

pub(crate) fn directory_for(base: u16, pid: u32) -> PathBuf {
    crate::live::live_root()
        .join("runs")
        .join(format!("{base}-{pid}"))
}

/// Directory that holds this process's grid, standalone and state files.
/// Unset, that is `.scratch/live` and the historical `run/`, `grid/` and
/// `grid-state` names stay where they are.
pub(crate) fn run_root() -> PathBuf {
    let Some(base) = current().base else {
        return crate::live::live_root();
    };
    RUN_ROOT
        .get_or_init(|| {
            let dir = directory_for(base, std::process::id());
            let _ = std::fs::create_dir_all(&dir);
            dir
        })
        .clone()
}

pub(crate) fn grid_state_file() -> PathBuf {
    if isolated() {
        run_root().join("grid-state")
    } else {
        crate::live::live_root().join("grid-state")
    }
}

pub(crate) fn alloc(slot: Slot) -> Result<u16, String> {
    alloc_key(slot as u8, current().today_or_base(slot))
}

pub(crate) fn alloc_dynamic() -> Result<u16, String> {
    let key = {
        let mut next = DYNAMIC_NEXT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let key = *next;
        if u16::from(key) >= SPAN {
            return Err(format!(
                "the port base has no free offset left (span {SPAN})"
            ));
        }
        *next = key + 1;
        key
    };
    alloc_key(key, None)
}

pub(crate) fn alloc_grid(count: usize) -> Result<Vec<u16>, String> {
    const ORDER: [Slot; 9] = [
        Slot::RobustPublic,
        Slot::RobustPrivate,
        Slot::PublicProxy,
        Slot::PrivateProxy,
        Slot::Home,
        Slot::Neighbour,
        Slot::RustService,
        Slot::RegionMetrics,
        Slot::RobustMetrics,
    ];
    if count > ORDER.len() {
        return Err(format!(
            "a grid run needs {count} ports and the base maps {}",
            ORDER.len()
        ));
    }
    let mut ports = Vec::with_capacity(count);
    for slot in ORDER.into_iter().take(count) {
        ports.push(alloc(slot)?);
    }
    Ok(ports)
}

pub(crate) fn alloc_slots(slots: &[Slot]) -> Result<Vec<u16>, String> {
    let mut ports = Vec::with_capacity(slots.len());
    for slot in slots {
        ports.push(alloc(*slot)?);
    }
    Ok(ports)
}

pub(crate) fn assigned(slot: Slot) -> Option<u16> {
    let assigned = ASSIGNED.lock().unwrap_or_else(|poison| poison.into_inner());
    assigned
        .iter()
        .find(|(key, _)| *key == slot as u8)
        .map(|(_, port)| *port)
}

fn alloc_key(key: u8, today: Option<u16>) -> Result<u16, String> {
    let mut assigned = ASSIGNED.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some((_, port)) = assigned.iter().find(|(have, _)| *have == key) {
        return Ok(*port);
    }
    let port = if let Some(base) = current().base {
        let port = base + u16::from(key);
        ensure_free(port)?;
        port
    } else if let Some(port) = today {
        ensure_free(port)?;
        port
    } else {
        ephemeral(&assigned)?
    };
    if assigned.iter().any(|(_, have)| *have == port) {
        return Err(format!("port {port} was already allocated for this run"));
    }
    assigned.push((key, port));
    Ok(port)
}

fn ephemeral(assigned: &[(u8, u16)]) -> Result<u16, String> {
    for _ in 0..16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("could not bind a local port: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("could not read a local port: {error}"))?
            .port();
        drop(listener);
        if !assigned.iter().any(|(_, have)| *have == port) {
            return Ok(port);
        }
    }
    Err("could not allocate a local port".to_string())
}

fn ensure_free(port: u16) -> Result<(), String> {
    // Releasing the reservation is the check: nobody else could bind it while
    // this process held the listener. A port outside the block still probes.
    if release_reservation(port) {
        return Ok(());
    }
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(error) => Err(format!("127.0.0.1:{port} is already in use ({error})")),
    }
}

/// Actual listener to the canonical fixture port. Compares stay byte-strict
/// after this rewrite; a number that is not one of the run's ports is left as it is.
pub(crate) fn write_fixture_ports(dir: &Path) -> Result<(), String> {
    let assigned = ASSIGNED.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut rows = Vec::new();
    for (offset, port) in assigned.iter() {
        let Some(slot) = Slot::from_offset(*offset) else {
            continue;
        };
        let Some(canonical) = slot.canonical() else {
            continue;
        };
        rows.push(json!({"actual": port, "canonical": canonical}));
    }
    drop(assigned);
    if rows.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let path = dir.join("ports.json");
    let bytes = serde_json::to_vec_pretty(&rows).map_err(|_| "could not encode the port map")?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

pub(crate) fn load_rewrites(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let path = directory.join("ports.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let rows: Vec<Value> = serde_json::from_slice(&bytes)
        .map_err(|_| format!("{} is not a port map", path.display()))?;
    let mut pairs = Vec::with_capacity(rows.len());
    for row in rows {
        let actual = row
            .get("actual")
            .and_then(Value::as_u64)
            .filter(|port| (1..65536).contains(port))
            .ok_or_else(|| format!("{} has a port outside 1..=65535", path.display()))?;
        let canonical = row
            .get("canonical")
            .and_then(Value::as_u64)
            .filter(|port| (1..65536).contains(port))
            .ok_or_else(|| format!("{} has a canonical port outside 1..=65535", path.display()))?;
        pairs.push((actual.to_string(), canonical.to_string()));
    }
    Ok(pairs)
}

/// Replace recorded listeners with their canonical fixture ports. One pass, so
/// a canonical number that happens to equal another listener is not rewritten again.
pub(crate) fn rewrite_ports(text: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(canonical) = port_at(text, index, pairs) {
            let (prefix, digits) = canonical;
            out.push_str(prefix);
            out.push_str(digits);
            index += prefix.len() + digits_after_prefix(text, index, prefix.len());
            continue;
        }
        let ch = text[index..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn port_at<'t, 'p>(
    text: &'t str,
    index: usize,
    pairs: &'p [(String, String)],
) -> Option<(&'t str, &'p str)> {
    let rest = &text[index..];
    let prefix = if rest.starts_with("%3A") || rest.starts_with("%3a") {
        &rest[..3]
    } else if rest.starts_with(':') {
        ":"
    } else {
        return None;
    };
    let digits = &rest[prefix.len()..];
    let length = digits.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if length == 0 {
        return None;
    }
    let number = &digits[..length];
    pairs
        .iter()
        .find(|(actual, _)| actual == number)
        .map(|(_, canonical)| (prefix, canonical.as_str()))
}

fn digits_after_prefix(text: &str, index: usize, prefix: usize) -> usize {
    text[index + prefix..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count()
}

pub(crate) fn rewrite_collector(text: &str, plan: Plan) -> String {
    let Some(_) = plan.base else {
        return text.to_string();
    };
    let mut pairs = Vec::new();
    for slot in [Slot::OtelGrpc, Slot::OtelHttp, Slot::Loki] {
        if let (Some(canonical), Some(planned)) = (slot.canonical(), plan.port_of(slot)) {
            pairs.push((canonical.to_string(), planned.to_string()));
        }
    }
    rewrite_ports(text, &pairs)
}

pub(crate) fn loki_server_args(plan: Plan) -> Vec<String> {
    let Some(_) = plan.base else {
        return Vec::new();
    };
    // Extra `docker run` arguments replace the image command, so the config
    // flag travels with the listen ports. Unset smoke passes no arguments and
    // keeps the image command.
    vec![
        "-config.file=/etc/loki/local-config.yaml".to_string(),
        format!(
            "-server.http-listen-port={}",
            plan.port_of(Slot::Loki).unwrap_or(CANONICAL_LOKI)
        ),
        format!(
            "-server.grpc-listen-port={}",
            plan.port_of(Slot::LokiGrpc).unwrap_or(CANONICAL_LOKI_GRPC)
        ),
    ]
}

pub(crate) fn scoped_name(canonical: &str, plan: Plan) -> String {
    match plan.base {
        Some(base) => format!("{canonical}-{base}"),
        None => canonical.to_string(),
    }
}

pub(crate) fn compose_project(plan: Plan) -> String {
    match plan.base {
        Some(base) => format!("ferroan-grid-{base}"),
        None => "ferroan-grid".to_string(),
    }
}

/// Ports written as listeners that are not in `allowed`. A hard-coded
/// canonical port fails this when the base has moved.
#[cfg(test)]
pub(crate) fn ports_outside(text: &str, allowed: &[u16]) -> Vec<u16> {
    let mut outside = Vec::new();
    for line in text.lines() {
        if !(line.contains("Port")
            || line.contains("port")
            || line.contains("http://")
            || line.contains("https://")
            || line.contains("endpoint")
            || line.contains("/OpenSim")
            || line.contains("/otlp"))
        {
            continue;
        }
        for port in line_ports(line) {
            if !allowed.contains(&port) && !outside.contains(&port) {
                outside.push(port);
            }
        }
    }
    outside
}

#[cfg(test)]
fn line_ports(line: &str) -> Vec<u16> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let take = if bytes[index] == b':' {
            Some(index + 1)
        } else if line[index..].starts_with("%3A") || line[index..].starts_with("%3a") {
            Some(index + 3)
        } else {
            None
        };
        if let Some(start) = take
            && let Some(port) = bounded_port(line, start)
        {
            found.push(port);
        }
        index += 1;
    }
    let lower = line.to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut base = 0usize;
    while let Some(at) = rest.find("port") {
        let after = at + "port".len();
        let tail = &line[base + after..];
        let digits_at = tail.find(|ch: char| ch.is_ascii_digit()).filter(|offset| {
            tail[..*offset]
                .chars()
                .all(|ch| !ch.is_ascii_alphanumeric())
        });
        if let Some(offset) = digits_at
            && let Some(port) = bounded_port(line, base + after + offset)
        {
            found.push(port);
        }
        rest = &rest[after..];
        base += after;
    }
    let marker = "/OpenSim";
    if let Some(at) = line.find(marker)
        && let Some(port) = port_before(line, at)
    {
        found.push(port);
    }
    found
}

#[cfg(test)]
fn bounded_port(text: &str, start: usize) -> Option<u16> {
    let digits = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if !(2..=5).contains(&digits) {
        return None;
    }
    let number = &text[start..start + digits];
    let port: u16 = number.parse().ok()?;
    if port == 0 {
        return None;
    }
    Some(port)
}

#[cfg(test)]
fn port_before(text: &str, end: usize) -> Option<u16> {
    let head = &text[..end];
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(index, _)| index)
        .last()?;
    bounded_port(text, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_keeps_todays_ports_and_a_base_moves_every_slot() {
        let unset = Plan::unset();
        assert_eq!(unset.today_or_base(Slot::Loki), Some(CANONICAL_LOKI));
        assert_eq!(
            unset.today_or_base(Slot::LokiGrpc),
            Some(CANONICAL_LOKI_GRPC)
        );
        assert_eq!(
            unset.today_or_base(Slot::OtelGrpc),
            Some(CANONICAL_OTEL_GRPC)
        );
        assert_eq!(
            unset.today_or_base(Slot::OtelHttp),
            Some(CANONICAL_OTEL_HTTP)
        );
        assert_eq!(unset.port_of(Slot::Home), None);
        assert_eq!(unset.today_or_base(Slot::Home), None);
        assert_eq!(unset.today_or_base(Slot::PublicProxy), None);
        let based = Plan::from_raw(Some("25000")).unwrap();
        assert_eq!(based.port_of(Slot::Home), Some(25004));
        assert_eq!(based.port_of(Slot::Neighbour), Some(25005));
        assert_eq!(based.port_of(Slot::PublicProxy), Some(25002));
        assert_eq!(based.port_of(Slot::PrivateProxy), Some(25003));
        assert_eq!(based.port_of(Slot::Loki), Some(25014));
        assert_ne!(based.port_of(Slot::Loki), Some(CANONICAL_LOKI));
        assert_ne!(based.port_of(Slot::OtelHttp), Some(CANONICAL_OTEL_HTTP));
        assert!(Plan::from_raw(Some("80")).is_err());
        assert!(Plan::from_raw(Some("65505")).is_err());
        assert!(Plan::from_raw(Some("nope")).is_err());
        assert_eq!(Plan::from_raw(None).unwrap(), Plan::unset());
    }

    #[test]
    fn adjacent_bases_do_not_share_a_port() {
        let origin = 20_000u16;
        let left: Vec<_> = (0..SPAN).map(|offset| origin + offset).collect();
        let right: Vec<_> = (0..SPAN).map(|offset| origin + SPAN + offset).collect();
        assert!(left.iter().all(|port| !right.contains(port)));
    }

    #[test]
    fn run_directories_follow_the_base_and_the_process() {
        let first = directory_for(20_000, 11);
        let second = directory_for(21_000, 22);
        assert_ne!(first, second);
        assert!(first.ends_with("runs/20000-11"));
        assert!(second.ends_with("runs/21000-22"));
        assert_eq!(
            scoped_name("ferroan-p011-loki", Plan::unset()),
            "ferroan-p011-loki"
        );
        assert_eq!(
            scoped_name("ferroan-p011-loki", Plan { base: Some(20_000) }),
            "ferroan-p011-loki-20000"
        );
        assert_eq!(compose_project(Plan::unset()), "ferroan-grid");
        assert_eq!(
            compose_project(Plan { base: Some(20_000) }),
            "ferroan-grid-20000"
        );
    }

    #[test]
    fn a_hard_coded_port_outside_the_base_fails() {
        let home = 25_004u16;
        let text = format!(
            "http_listener_port = {home}\nPublicPort = \"{home}\"\nendpoint: 127.0.0.1:{home}\n"
        );
        assert!(ports_outside(&text, &[home]).is_empty(), "{text}");
        let bad = format!("{text}http_listener_port = 9000\n");
        assert_eq!(ports_outside(&bad, &[home]), vec![9000]);
        let connector = format!(
            "LLLoginServiceInConnector = \"{home}/OpenSim.Server.Handlers.dll:LLLoginServiceInConnector\"\n"
        );
        assert!(ports_outside(&connector, &[home]).is_empty(), "{connector}");
        let leaked = format!(
            "{connector}GridServiceConnector = \"8003/OpenSim.Server.Handlers.dll:GridServiceConnector\"\n"
        );
        assert_eq!(ports_outside(&leaked, &[home]), vec![8003]);
    }

    #[test]
    fn collector_and_loki_move_with_the_base_and_stay_put_without_one() {
        let source = include_str!("../../deploy/monitoring/collector/loki.yaml");
        assert_eq!(rewrite_collector(source, Plan::unset()), source);
        assert!(loki_server_args(Plan::unset()).is_empty());
        let based = Plan::from_raw(Some("25000")).unwrap();
        let rewritten = rewrite_collector(source, based);
        assert!(!rewritten.contains(":3100"));
        assert!(!rewritten.contains(":4317"));
        assert!(!rewritten.contains(":4318"));
        assert!(rewritten.contains("127.0.0.1:25014"));
        assert!(rewritten.contains("127.0.0.1:25016"));
        assert!(rewritten.contains("127.0.0.1:25017"));
        let args = loki_server_args(based);
        assert_eq!(
            args,
            vec![
                "-config.file=/etc/loki/local-config.yaml".to_string(),
                "-server.http-listen-port=25014".to_string(),
                "-server.grpc-listen-port=25015".to_string(),
            ]
        );
    }

    fn reservation_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    struct ClearReservation;
    impl Drop for ClearReservation {
        fn drop(&mut self) {
            release_block();
        }
    }

    fn free_block() -> u16 {
        (0..40u16)
            .map(|step| 20_000 + step * SPAN)
            .find(|candidate| {
                (0..SPAN).all(|offset| {
                    let port = slot_port(*candidate, usize::from(offset)).expect("offset fits");
                    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                        .map(drop)
                        .is_ok()
                })
            })
            .expect("a free 32-port block")
    }

    #[test]
    fn a_reserved_slot_stays_unavailable_until_it_is_allocated() {
        let _lock = reservation_lock();
        let _clear = ClearReservation;
        let base = free_block();
        reserve_block(base).expect("the free block reserves");
        let port = base + u16::from(Slot::StandalonePrivate as u8);
        assert!(
            TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err(),
            "a client must not bind a reserved slot"
        );
        assert!(ensure_free(port).is_ok());
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("the allocated slot is free");
        drop(listener);
    }

    #[test]
    fn releasing_the_block_lets_the_next_reserve_hold_it() {
        let _lock = reservation_lock();
        let _clear = ClearReservation;
        let base = free_block();
        let port = base + u16::from(Slot::StandalonePrivate as u8);
        reserve_block(base).expect("the free block reserves");
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err());
        release_block();
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("a released slot binds");
        drop(taken);
        reserve_block(base).expect("the block reserves again");
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err());
    }

    #[test]
    fn a_port_already_taken_stays_unreserved() {
        let _lock = reservation_lock();
        let _clear = ClearReservation;
        let base = free_block();
        let busy = base + u16::from(Slot::StandalonePrivate as u8);
        let held = base + u16::from(Slot::StandalonePublic as u8);
        let outside =
            TcpListener::bind((Ipv4Addr::LOCALHOST, busy)).expect("the outside hold binds");
        reserve_block_for(base, std::time::Duration::ZERO)
            .expect("a busy slot does not fail the reserve");
        assert!(
            TcpListener::bind((Ipv4Addr::LOCALHOST, busy)).is_err(),
            "the outside listener keeps the busy slot"
        );
        assert!(ensure_free(busy).is_err());
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, held)).is_err());
        assert!(ensure_free(held).is_ok());
        drop(outside);
        // macOS can keep a closed listener's port busy. The checks above
        // already show the reservation left that slot unheld.
        #[cfg(not(target_os = "macos"))]
        assert!(ensure_free(busy).is_ok());
    }

    #[test]
    fn fixture_rewrites_replace_only_recorded_ports() {
        let pairs = vec![("25004".to_string(), "9000".to_string())];
        let text = "http://192.0.2.1:25004/CAPS and http://192.0.2.1:12345/left and %3a25004";
        let rewritten = rewrite_ports(text, &pairs);
        assert_eq!(
            rewritten,
            "http://192.0.2.1:9000/CAPS and http://192.0.2.1:12345/left and %3a9000"
        );
        assert!(rewritten.contains(":12345"));
    }
}
