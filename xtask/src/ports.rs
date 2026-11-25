// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One port base for every listener a live run opens.
//!
//! `FERROAN_LIVE_PORT_BASE` or `--port-base` selects the base. Every harness
//! listener, C# Robust and region port, Ferroan service, capture proxy and
//! Docker smoke port is `base + offset`, and the run's files live under
//! `.scratch/live/runs/<base>-<pid>/`. With no setting, ephemeral listeners
//! stay ephemeral and Docker smoke stays on 3100, 9095, 4317 and 4318.
//!
//! A base is a multiple of [`SPAN`], so two runs on different bases never
//! share a port, and its block lies outside the kernel's ephemeral range
//! (`/proc/sys/net/ipv4/ip_local_port_range`), so a socket bound to port 0,
//! by this run or anything else on the machine, cannot take one of its ports
//! (limit `live.port-base`). A listener the harness binds to port 0 itself
//! (the TCP_NODELAY sample, the console-forms grid stub, the Ferrobot UDP
//! relay's upstream socket) therefore stays ephemeral and cannot collide.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::net::{Ipv4Addr, TcpListener, UdpSocket};
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
    /// Region ports carry UDP as well as HTTP (`InternalPort`).
    fn carries_udp(self) -> bool {
        matches!(
            self,
            Self::Home | Self::Neighbour | Self::StandalonePublic | Self::ConsolePublic
        )
    }

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
        Self::parse(value, ephemeral_range())
    }

    /// [`Plan::from_raw`] against a given ephemeral range (`None`: unknown).
    fn parse(value: Option<&str>, ephemeral: Option<(u16, u16)>) -> Result<Self, String> {
        match value {
            None | Some("") => Ok(Self::unset()),
            Some(text) => {
                let base: u16 = text.parse().map_err(|_| format!("{ENV} needs a port"))?;
                validate_base(base, ephemeral)?;
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

const EPHEMERAL_RANGE: &str = "/proc/sys/net/ipv4/ip_local_port_range";

/// The kernel's ephemeral port range, or `None` where it cannot be read
/// (not Linux, or no `/proc`).
fn ephemeral_range() -> Option<(u16, u16)> {
    parse_ephemeral(&std::fs::read_to_string(EPHEMERAL_RANGE).ok()?)
}

fn parse_ephemeral(text: &str) -> Option<(u16, u16)> {
    let mut fields = text.split_whitespace();
    let low = fields.next()?.parse().ok()?;
    let high = fields.next()?.parse().ok()?;
    (fields.next().is_none() && low <= high).then_some((low, high))
}

fn validate_base(base: u16, ephemeral: Option<(u16, u16)>) -> Result<(), String> {
    if base < 1024 {
        return Err(format!("{ENV} {base} is below 1024"));
    }
    let last = u32::from(base) + u32::from(SPAN) - 1;
    if last > u32::from(u16::MAX) {
        return Err(format!("{ENV} {base} does not leave room for {SPAN} ports"));
    }
    if !base.is_multiple_of(SPAN) {
        let below = base - base % SPAN;
        return Err(format!(
            "{ENV} {base} is not a multiple of {SPAN}, so its block can overlap another \
             run's; use {below} or {}",
            u32::from(below) + u32::from(SPAN)
        ));
    }
    if let Some((low, high)) = ephemeral
        && u32::from(base) <= u32::from(high)
        && last >= u32::from(low)
    {
        ferroan_diag::refuse(
            ferroan_diag::LIVE_PORT_BASE,
            ferroan_diag::Kind::Hard,
            u64::from(low),
            u64::from(base),
            "live port base",
        );
        return Err(format!(
            "{ENV} {base} ({base}-{last}) overlaps the ephemeral range {low}-{high} in \
             {EPHEMERAL_RANGE}, where any socket bound to port 0 can take one of its ports \
             (limit {}); pick a multiple of {SPAN} outside it",
            ferroan_diag::LIVE_PORT_BASE
        ));
    }
    Ok(())
}

static PLAN: OnceLock<Plan> = OnceLock::new();
static ASSIGNED: Mutex<Vec<(u8, u16)>> = Mutex::new(Vec::new());
static DYNAMIC_NEXT: Mutex<u8> = Mutex::new(DYNAMIC);
static RUN_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn install(flag: Option<u16>) -> Result<(), String> {
    let plan = Plan::resolve(flag)?;
    PLAN.set(plan)
        .map_err(|_| "the live port base is already set".to_string())
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
    alloc_key(
        slot as u8,
        current().today_or_base(slot),
        slot.carries_udp(),
    )
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
    alloc_key(key, None, false)
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

fn alloc_key(key: u8, today: Option<u16>, udp: bool) -> Result<u16, String> {
    let mut assigned = ASSIGNED.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some((_, port)) = assigned.iter().find(|(have, _)| *have == key) {
        return Ok(*port);
    }
    let port = if let Some(base) = current().base {
        let port = base + u16::from(key);
        ensure_free(port, udp)?;
        port
    } else if let Some(port) = today {
        ensure_free(port, udp)?;
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

/// TCP is always checked. A region slot also needs the UDP port.
fn ensure_free(port: u16, udp: bool) -> Result<(), String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|error| format!("127.0.0.1:{port} is already in use ({error})"))?;
    if udp {
        ensure_udp_free(port)?;
    }
    drop(listener);
    Ok(())
}

fn ensure_udp_free(port: u16) -> Result<(), String> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|error| format!("UDP 127.0.0.1:{port} is already in use ({error})"))?;
    drop(socket);
    Ok(())
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
        let based = Plan::parse(Some("25024"), None).unwrap();
        assert_eq!(based.port_of(Slot::Home), Some(25028));
        assert_eq!(based.port_of(Slot::Neighbour), Some(25029));
        assert_eq!(based.port_of(Slot::PublicProxy), Some(25026));
        assert_eq!(based.port_of(Slot::PrivateProxy), Some(25027));
        assert_eq!(based.port_of(Slot::Loki), Some(25038));
        assert_ne!(based.port_of(Slot::Loki), Some(CANONICAL_LOKI));
        assert_ne!(based.port_of(Slot::OtelHttp), Some(CANONICAL_OTEL_HTTP));
        assert!(Plan::from_raw(Some("80")).is_err());
        assert!(Plan::from_raw(Some("65505")).is_err());
        assert!(Plan::from_raw(Some("nope")).is_err());
        assert!(
            Plan::parse(Some("25000"), None).is_err(),
            "not a multiple of 32"
        );
        assert!(Plan::parse(Some("25024"), None).is_ok());
        assert_eq!(Plan::from_raw(None).unwrap(), Plan::unset());
    }

    #[test]
    fn a_base_in_the_ephemeral_range_is_refused() {
        let linux = Some((32_768, 60_999));
        assert_eq!(parse_ephemeral("32768\t60999\n"), Some((32_768, 60_999)));
        assert_eq!(parse_ephemeral("1024 65535"), Some((1_024, 65_535)));
        assert_eq!(parse_ephemeral("60999 32768"), None);
        assert_eq!(parse_ephemeral("32768"), None);
        // The last block below the range and the first above it.
        assert!(validate_base(32_736, linux).is_ok());
        assert!(validate_base(61_024, linux).is_ok());
        for inside in [32_768, 40_000 - 40_000 % SPAN, 60_992] {
            let error = validate_base(inside, linux).unwrap_err();
            assert!(error.contains("live.port-base"), "{error}");
            assert!(error.contains("32768-60999"), "{error}");
        }
        // A block that only reaches into the range at its top.
        let high_start = Some((32_770, 60_999));
        assert!(validate_base(32_736, high_start).is_ok());
        assert!(validate_base(32_768, high_start).is_err());
        let low_end = Some((10_000, 32_740));
        assert!(validate_base(32_736, low_end).is_err());
        assert!(validate_base(32_736, None).is_ok());
        // The live harness and region compare bases stay valid on Linux.
        for base in [22_016, 22_048, 25_024] {
            assert!(validate_base(base, linux).is_ok(), "{base}");
        }
    }

    #[test]
    fn a_base_off_the_span_is_refused_with_the_neighbours() {
        let error = validate_base(25_000, None).unwrap_err();
        assert!(
            error.contains("24992") && error.contains("25024"),
            "{error}"
        );
        assert!(validate_base(1_024, None).is_ok());
    }

    #[test]
    fn a_region_slot_checks_its_udp_port() {
        assert!(Slot::Home.carries_udp());
        assert!(Slot::Neighbour.carries_udp());
        assert!(Slot::StandalonePublic.carries_udp());
        assert!(Slot::ConsolePublic.carries_udp());
        assert!(!Slot::RobustPublic.carries_udp());
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        // The TCP half of `ensure_free` is the same bind with TCP; other
        // tests may take that number meanwhile, so only the UDP half is held.
        let error = ensure_udp_free(port).unwrap_err();
        assert!(error.contains("UDP"), "{error}");
        drop(socket);
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
        let based = Plan::parse(Some("25024"), None).unwrap();
        let rewritten = rewrite_collector(source, based);
        assert!(!rewritten.contains(":3100"));
        assert!(!rewritten.contains(":4317"));
        assert!(!rewritten.contains(":4318"));
        assert!(rewritten.contains("127.0.0.1:25038"));
        assert!(rewritten.contains("127.0.0.1:25040"));
        assert!(rewritten.contains("127.0.0.1:25041"));
        let args = loki_server_args(based);
        assert_eq!(
            args,
            vec![
                "-config.file=/etc/loki/local-config.yaml".to_string(),
                "-server.http-listen-port=25038".to_string(),
                "-server.grpc-listen-port=25039".to_string(),
            ]
        );
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
