// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One helper for bounds that refuse, truncate, drop or delay outside input.
//!
//! A hit writes a `tracing` event on target `ferroan::limit`, increments
//! `ferroan_limit_hits_total`, and keeps a bounded ring of warning, error and
//! limit lines. The first hit for an id is logged in full. Later hits in the
//! same minute increment the counter and are summarised once the minute ends.
//! Soft limits also warn once when occupancy crosses 80 percent, and again
//! after it falls below that line.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod catalog;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use catalog::*;

pub const RING_CAP: usize = 1000;
pub const RATE_INTERVAL: Duration = Duration::from_secs(60);
pub const HIGH_WATER_PERCENT: u64 = 80;
pub const EXCERPT_BYTES: usize = 256;
pub const TRANQUILLITY_PIN: &str = "c70fee205f64ee8672a8d607cef6a733931da951";

const MESSAGE_CAP: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Soft,
    Hard,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Soft => "soft",
            Kind::Hard => "hard",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Refuse,
    Truncate,
    Drop,
    Delay,
    HighWater,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Refuse => "refuse",
            Action::Truncate => "truncate",
            Action::Drop => "drop",
            Action::Delay => "delay",
            Action::HighWater => "high-water",
        }
    }
}

pub struct Hit<'a> {
    pub id: &'static str,
    pub kind: Kind,
    pub configured: u64,
    pub observed: u64,
    pub action: Action,
    pub route: &'a str,
    pub agent_id: Option<&'a str>,
}

struct Slot {
    suppressed: u64,
    last_log: Option<Instant>,
    high_water: u64,
    high_open: bool,
}

struct Identity {
    server: String,
    region: String,
    version: String,
    pin: String,
    config: String,
}

struct State {
    slots: BTreeMap<String, Slot>,
    counters: BTreeMap<(&'static str, &'static str), u64>,
    ring: std::collections::VecDeque<String>,
    identity: Identity,
    snapshot: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    redactor: fn(&str) -> String,
}

fn default_redact(text: &str) -> String {
    text.to_owned()
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            slots: BTreeMap::new(),
            counters: BTreeMap::new(),
            ring: std::collections::VecDeque::new(),
            identity: Identity {
                server: String::new(),
                region: String::new(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                pin: TRANQUILLITY_PIN.to_owned(),
                config: String::new(),
            },
            snapshot: None,
            redactor: default_redact,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|poison| poison.into_inner())
}

pub fn set_redactor(redactor: fn(&str) -> String) {
    lock().redactor = redactor;
}

pub fn set_context(server: &str, region: &str) {
    let mut guard = lock();
    guard.identity.server = server.to_owned();
    guard.identity.region = region.to_owned();
}

pub fn set_version(version: &str) {
    lock().identity.version = version.to_owned();
}

pub fn set_configuration(text: &str) {
    let redacted = {
        let guard = lock();
        (guard.redactor)(text)
    };
    lock().identity.config = one_line(&redacted);
}

pub fn set_snapshot(snapshot: Arc<dyn Fn() -> String + Send + Sync>) {
    lock().snapshot = Some(snapshot);
}

pub fn hit(hit: &Hit<'_>) {
    let mut agent = String::new();
    if let Some(id) = hit.agent_id.filter(|id| !id.is_empty()) {
        agent = format!(" agent_id={id}");
    }
    let (server, region) = {
        let guard = lock();
        (guard.identity.server.clone(), guard.identity.region.clone())
    };
    let message = format!(
        "limit {id} {kind} {action} configured={configured} observed={observed} route={route} server={server} region={region}{agent}",
        id = hit.id,
        kind = hit.kind.label(),
        action = hit.action.label(),
        configured = hit.configured,
        observed = hit.observed,
        route = field(hit.route),
    );
    if hit.action == Action::HighWater {
        // The once-per-climb flag in `observe` is this warning's throttle.
        // A shared minute slot would hide the next refuse on the same id.
        record_open(hit.id, &message);
    } else {
        record(hit.id, hit.action.label(), &message, Some(hit.id));
    }
}

/// Count and log a hard or soft refusal. `configured` and `observed` are the
/// bound and the value that crossed it.
pub fn refuse(id: &'static str, kind: Kind, configured: u64, observed: u64, route: &str) {
    hit(&Hit {
        id,
        kind,
        configured,
        observed,
        action: Action::Refuse,
        route,
        agent_id: None,
    });
}

/// Count and log a drop. The counter still increments when the log line is suppressed.
pub fn drop_bound(id: &'static str, kind: Kind, configured: u64, observed: u64, route: &str) {
    hit(&Hit {
        id,
        kind,
        configured,
        observed,
        action: Action::Drop,
        route,
        agent_id: None,
    });
}

/// Soft-limit high-water mark. Warns once when `observed` reaches 80 percent
/// of `configured`, then again only after it drops below that fraction.
pub fn observe(id: &'static str, configured: u64, observed: u64, route: &str) {
    if configured == 0 {
        return;
    }
    let crossed = observed.saturating_mul(100) >= configured.saturating_mul(HIGH_WATER_PERCENT);
    let mut guard = lock();
    let slot = guard.slots.entry(id.to_owned()).or_insert(Slot {
        suppressed: 0,
        last_log: None,
        high_water: 0,
        high_open: false,
    });
    if observed > slot.high_water {
        slot.high_water = observed;
    }
    let open = slot.high_open;
    if !crossed {
        slot.high_open = false;
        return;
    }
    if open {
        return;
    }
    slot.high_open = true;
    drop(guard);
    hit(&Hit {
        id,
        kind: Kind::Soft,
        configured,
        observed,
        action: Action::HighWater,
        route,
        agent_id: None,
    });
}

pub fn log_error(route: &str, error: &dyn std::error::Error) {
    let mut chain = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        chain.push_str(": ");
        chain.push_str(&next.to_string());
        source = next.source();
    }
    log_boundary(route, &chain);
}

pub fn log_boundary(route: &str, message: &str) {
    let text = format!("error route={} {}", field(route), message);
    record(&format!("error:{route}"), "error", &text, None);
}

/// Record a malformed body.
///
/// The ring, `diag dump` files and panic files keep the route, the length and
/// `error`. They do not keep the body. The hex excerpt is only the debug
/// line, after the known secret fields of that message type are masked:
/// CreateAgent's `session_id`, `secure_session_id`, `circuit_code` and
/// `caps_path` on route `/agent`, and UseCircuitCode's `Code` and `SessionID`
/// on route `udp`.
pub fn log_malformed(route: &str, bytes: &[u8], error: &str) {
    let (ring_text, debug_text) = malformed_record(route, bytes, error);
    let key = format!("malformed:{route}");
    let emit = {
        let mut guard = lock();
        let redacted = (guard.redactor)(&ring_text);
        let hex_line = (guard.redactor)(&debug_text);
        take_log(&mut guard, &key, &redacted).map(|line| (line, hex_line))
    };
    if let Some((message, hex_line)) = emit {
        tracing::debug!(target: "ferroan::error", "{message}");
        tracing::debug!(target: "ferroan::error", "{hex_line}");
    }
}

pub fn metrics() -> String {
    let guard = lock();
    let mut text = String::from(
        "# HELP ferroan_limit_hits_total Bounds that refused, truncated, dropped, delayed or crossed a high-water mark.\n# TYPE ferroan_limit_hits_total counter\n",
    );
    let mut rows: Vec<_> = guard.counters.iter().collect();
    rows.sort_by_key(|(key, _)| *key);
    for ((limit, action), count) in rows {
        text.push_str(&format!(
            "ferroan_limit_hits_total{{limit=\"{limit}\",action=\"{action}\"}} {count}\n"
        ));
    }
    text
}

pub fn render() -> String {
    let guard = lock();
    let snapshot = guard.snapshot.clone();
    let mut text = String::new();
    text.push_str("ferroan diag\n");
    text.push_str(&format!("version: {}\n", guard.identity.version));
    text.push_str(&format!("pin: {}\n", guard.identity.pin));
    text.push_str(&format!("server: {}\n", guard.identity.server));
    text.push_str(&format!("region: {}\n", guard.identity.region));
    text.push_str("\n# configuration\n");
    if guard.identity.config.is_empty() {
        text.push_str("(none recorded)\n");
    } else {
        text.push_str(&guard.identity.config);
        text.push('\n');
    }
    text.push_str("\n# runtime\n");
    drop(guard);
    let runtime = snapshot.map(|snapshot| snapshot()).unwrap_or_default();
    if runtime.is_empty() {
        text.push_str("(none recorded)\n");
    } else {
        text.push_str(&runtime);
        if !runtime.ends_with('\n') {
            text.push('\n');
        }
    }
    let guard = lock();
    text.push_str("\n# limits\n");
    if guard.counters.is_empty() {
        text.push_str("(no hits)\n");
    }
    for ((limit, action), count) in &guard.counters {
        let high = guard
            .slots
            .get(*limit)
            .map(|slot| slot.high_water)
            .unwrap_or(0);
        text.push_str(&format!(
            "{limit} action={action} hits={count} high_water={high}\n"
        ));
    }
    text.push_str("\n# recent events\n");
    if guard.ring.is_empty() {
        text.push_str("(empty)\n");
    }
    for line in &guard.ring {
        text.push_str(line);
        text.push('\n');
    }
    text
}

pub fn write_dump() -> std::io::Result<PathBuf> {
    write_report("diag", &render())
}

pub fn install_panic_hook() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = write_panic(info);
        previous(info);
    }));
}

pub fn push_log(level: &str, target: &str, message: &str) {
    let redactor = lock().redactor;
    let line = redactor(&format!("{level} {target} {}", one_line(message)));
    push_ring(&mut lock(), &line);
}

fn action_label(action: &str) -> &'static str {
    match action {
        "refuse" => "refuse",
        "truncate" => "truncate",
        "drop" => "drop",
        "delay" => "delay",
        "high-water" => "high-water",
        "error" => "error",
        _ => "other",
    }
}

fn record(key: &str, action: &str, message: &str, count_id: Option<&'static str>) {
    let emit = {
        let mut guard = lock();
        let redacted = (guard.redactor)(message);
        if let Some(id) = count_id {
            count_hit(&mut guard, id, action);
        }
        take_log(&mut guard, key, &redacted)
    };
    if let Some(message) = emit {
        if count_id.is_some() {
            tracing::warn!(target: "ferroan::limit", "{message}");
        } else {
            tracing::error!(target: "ferroan::error", "{message}");
        }
    }
}

fn record_open(catalog_id: &'static str, message: &str) {
    let emit = {
        let mut guard = lock();
        let redacted = (guard.redactor)(message);
        count_hit(&mut guard, catalog_id, "high-water");
        let line = clip(&redacted);
        push_ring(&mut guard, &line);
        line
    };
    tracing::warn!(target: "ferroan::limit", "{emit}");
}

fn count_hit(guard: &mut State, key: &'static str, action: &str) {
    let action = action_label(action);
    *guard.counters.entry((key, action)).or_insert(0) += 1;
}

fn take_log(guard: &mut State, key: &str, message: &str) -> Option<String> {
    let now = Instant::now();
    let slot = guard.slots.entry(key.to_owned()).or_insert(Slot {
        suppressed: 0,
        last_log: None,
        high_water: 0,
        high_open: false,
    });
    let due = match slot.last_log {
        None => true,
        Some(then) => now.saturating_duration_since(then) >= RATE_INTERVAL,
    };
    if !due {
        slot.suppressed = slot.suppressed.saturating_add(1);
        return None;
    }
    let suppressed = slot.suppressed;
    slot.suppressed = 0;
    slot.last_log = Some(now);
    let mut line = message.to_owned();
    if suppressed > 0 {
        line.push_str(&format!(" suppressed={suppressed}"));
    }
    let line = clip(&line);
    push_ring(guard, &line);
    Some(line)
}

fn push_ring(guard: &mut State, line: &str) {
    if guard.ring.len() == RING_CAP {
        guard.ring.pop_front();
    }
    guard.ring.push_back(line.to_owned());
}

fn write_panic(info: &std::panic::PanicHookInfo<'_>) -> std::io::Result<()> {
    let backtrace = std::backtrace::Backtrace::force_capture();
    write_panic_report(&format!("{info}\n{backtrace}")).map(|_| ())
}

fn write_panic_report(summary: &str) -> std::io::Result<PathBuf> {
    write_report("panic", &format!("panic: {summary}\n\n{}", render()))
}

fn write_report(kind: &str, text: &str) -> std::io::Result<PathBuf> {
    let server = {
        let name = sanitize(&lock().identity.server);
        if name.is_empty() {
            "ferroan".to_owned()
        } else {
            name
        }
    };
    let dir = dump_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{kind}-{server}-{}.txt", unix_millis()));
    std::fs::write(&path, text)?;
    Ok(path)
}

fn dump_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LOGDIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    PathBuf::from(".scratch")
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .take(64)
        .collect()
}

fn field(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        one_line(text)
    }
}

fn one_line(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= MESSAGE_CAP {
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn clip(text: &str) -> String {
    let mut out = text.to_owned();
    if out.len() > MESSAGE_CAP {
        out.truncate(MESSAGE_CAP);
    }
    out
}

fn malformed_record(route: &str, bytes: &[u8], error: &str) -> (String, String) {
    let ring = format!(
        "malformed route={} bytes={} error={}",
        field(route),
        bytes.len(),
        field(error)
    );
    let masked = mask_known_fields(route, bytes);
    let excerpt = &masked[..masked.len().min(EXCERPT_BYTES)];
    let debug = format!("{ring} hex={}", hex_excerpt(excerpt));
    (ring, debug)
}

/// Mask secret fields before the debug hex excerpt.
///
/// `/agent` is a CreateAgent JSON body, which may already be malformed.
/// `udp` is a datagram. UseCircuitCode (`message_template.msg`, Low 3,
/// unencoded) is message number `ff ff 00 03` after the 6-byte header and
/// its extra bytes (`packet.rs`), then `Code` U32 and `SessionID` LLUUID.
/// The agent id that follows is not one of the secret fields.
fn mask_known_fields(route: &str, bytes: &[u8]) -> Vec<u8> {
    if route == "/agent" {
        mask_json_fields(
            bytes,
            &[
                b"secure_session_id",
                b"session_id",
                b"circuit_code",
                b"caps_path",
            ],
        )
    } else if route == "udp" {
        mask_use_circuit_code(bytes)
    } else {
        bytes.to_vec()
    }
}

fn mask_json_fields(bytes: &[u8], keys: &[&[u8]]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for key in keys {
        let mut pattern = Vec::with_capacity(key.len() + 2);
        pattern.push(b'"');
        pattern.extend_from_slice(key);
        pattern.push(b'"');
        let mut search = 0;
        while let Some(found) = find_slice(&out, &pattern, search) {
            let mut index = found + pattern.len();
            while index < out.len() && out[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= out.len() || out[index] != b':' {
                search = found + 1;
                continue;
            }
            index += 1;
            while index < out.len() && out[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= out.len() {
                break;
            }
            if out[index] == b'"' {
                index += 1;
                while index < out.len() && out[index] != b'"' {
                    if out[index] == b'\\' && index + 1 < out.len() {
                        out[index] = b'*';
                        out[index + 1] = b'*';
                        index += 2;
                        continue;
                    }
                    out[index] = b'*';
                    index += 1;
                }
                search = index.saturating_add(1);
            } else if out[index] == b'-' || out[index].is_ascii_digit() {
                while index < out.len()
                    && (out[index].is_ascii_digit()
                        || matches!(out[index], b'-' | b'+' | b'.' | b'e' | b'E'))
                {
                    out[index] = b'*';
                    index += 1;
                }
                search = index;
            } else {
                search = found + 1;
            }
        }
    }
    out
}

fn mask_use_circuit_code(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    // Zerocoded packets do not keep these offsets. UseCircuitCode is unencoded.
    if out.len() < 6 || out[0] & 0x80 != 0 {
        return out;
    }
    let message = 6usize.saturating_add(out[5] as usize);
    let code_at = message.saturating_add(4);
    let session_at = code_at.saturating_add(4);
    let session_end = session_at.saturating_add(16);
    if session_end > out.len() || out.get(message..message + 4) != Some(&[0xff, 0xff, 0x00, 0x03]) {
        return out;
    }
    for byte in &mut out[code_at..session_end] {
        *byte = b'*';
    }
    out
}

fn find_slice(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| from + index)
}

fn hex_excerpt(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold() -> std::sync::MutexGuard<'static, ()> {
        static TEST: Mutex<()> = Mutex::new(());
        TEST.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn reset() {
        let mut guard = lock();
        guard.slots.clear();
        guard.counters.clear();
        guard.ring.clear();
        guard.identity.config.clear();
        guard.snapshot = None;
        guard.redactor = default_redact;
    }

    fn sample(observed: u64) -> Hit<'static> {
        Hit {
            id: HTTP_POST_BODY,
            kind: Kind::Hard,
            configured: 1024,
            observed,
            action: Action::Refuse,
            route: "/agent",
            agent_id: None,
        }
    }

    #[test]
    fn first_hit_is_logged_and_counted_and_the_next_is_counted_only() {
        let _hold = hold();
        reset();
        hit(&sample(1025));
        hit(&sample(1026));
        let (line, hits) = {
            let guard = lock();
            (
                guard.ring[0].clone(),
                guard.counters.get(&(HTTP_POST_BODY, "refuse")).copied(),
            )
        };
        assert_eq!(lock().ring.len(), 1);
        assert!(line.contains("http.post-body"));
        assert!(line.contains("observed=1025"));
        assert_eq!(hits, Some(2));
        let text = metrics();
        assert!(
            text.contains(
                "ferroan_limit_hits_total{limit=\"http.post-body\",action=\"refuse\"} 2\n"
            )
        );
    }

    #[test]
    fn a_later_interval_reports_the_suppressed_count() {
        let _hold = hold();
        reset();
        hit(&sample(1));
        hit(&sample(2));
        {
            let mut guard = lock();
            guard.slots.get_mut(HTTP_POST_BODY).unwrap().last_log =
                Some(Instant::now() - RATE_INTERVAL);
        }
        hit(&sample(3));
        let guard = lock();
        assert_eq!(guard.ring.len(), 2);
        assert!(guard.ring[1].contains("suppressed=1"));
        assert!(guard.ring[1].contains("observed=3"));
    }

    #[test]
    fn soft_high_water_warns_once_until_it_falls() {
        let _hold = hold();
        reset();
        observe(UDP_UNACKED, 100, 79, "udp");
        observe(UDP_UNACKED, 100, 80, "udp");
        observe(UDP_UNACKED, 100, 90, "udp");
        assert_eq!(lock().ring.len(), 1);
        assert!(lock().ring[0].contains("high-water"));
        observe(UDP_UNACKED, 100, 10, "udp");
        observe(UDP_UNACKED, 100, 80, "udp");
        assert_eq!(lock().ring.len(), 2);
        hit(&Hit {
            id: UDP_UNACKED,
            kind: Kind::Soft,
            configured: 100,
            observed: 100,
            action: Action::Refuse,
            route: "udp",
            agent_id: None,
        });
        let guard = lock();
        assert_eq!(guard.ring.len(), 3);
        assert!(guard.ring[2].contains("refuse"));
        assert_eq!(
            guard.counters.get(&(UDP_UNACKED, "refuse")).copied(),
            Some(1)
        );
    }

    #[test]
    fn the_ring_drops_the_oldest_line() {
        let _hold = hold();
        reset();
        for index in 0..RING_CAP + 1 {
            push_log("WARN", "test", &format!("line-{index}"));
        }
        let guard = lock();
        assert_eq!(guard.ring.len(), RING_CAP);
        assert!(guard.ring.front().unwrap().contains("line-1"));
        assert!(
            guard
                .ring
                .back()
                .unwrap()
                .contains(&format!("line-{RING_CAP}"))
        );
    }

    #[test]
    fn malformed_excerpt_is_hex_and_bounded() {
        let _hold = hold();
        reset();
        let bytes = vec![0x0a, 0xff, b'A'];
        log_malformed("udp", &bytes, "short");
        let guard = lock();
        assert_eq!(guard.ring.len(), 1);
        assert!(!guard.ring[0].contains("hex="));
        assert!(guard.ring[0].contains("bytes=3"));
        assert!(guard.ring[0].contains("error=short"));
        let (_ring, debug) = malformed_record("udp", &bytes, "short");
        assert!(debug.contains("hex=0aff41"));
    }

    #[test]
    fn malformed_secrets_stay_out_of_the_ring_dump_and_panic_file() {
        let _hold = hold();
        reset();
        set_context("diag-secret", "Test");
        let session = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let secure = "11111111-2222-3333-4444-555555555555";
        let code = "987654321";
        let seed = "SEEDPATHSECRET";
        let body = format!(
            "{{\"session_id\":\"{session}\",\"secure_session_id\":\"{secure}\",\"circuit_code\":\"{code}\",\"caps_path\":\"{seed}\","
        );
        log_malformed("/agent", body.as_bytes(), "expected value");
        let mut circuit = vec![0u8; 6];
        circuit.extend_from_slice(&[0xff, 0xff, 0x00, 0x03]);
        circuit.extend_from_slice(&0x89ab_cdef_u32.to_le_bytes());
        circuit.extend_from_slice(&[0xa1; 16]);
        circuit.extend_from_slice(&[0x11; 16]);
        circuit.resize(4097, 0x5a);
        log_malformed("udp", &circuit, "datagram exceeds udp.datagram");

        let forbidden = [
            session,
            secure,
            code,
            seed,
            &hex_excerpt(session.as_bytes()),
            &hex_excerpt(secure.as_bytes()),
            &hex_excerpt(code.as_bytes()),
            &hex_excerpt(seed.as_bytes()),
            "efcdab89",
            &"a1".repeat(16),
        ];
        let ring = {
            let mut guard = lock();
            assert_eq!(guard.ring.len(), 2);
            assert!(guard.ring[0].contains("route=/agent"));
            assert!(guard.ring[0].contains(&format!("bytes={}", body.len())));
            assert!(guard.ring[0].contains("error=expected value"));
            assert!(guard.ring[1].contains("route=udp"));
            assert!(guard.ring[1].contains("bytes=4097"));
            assert!(guard.ring[1].contains("error=datagram exceeds udp.datagram"));
            guard.ring.make_contiguous().join("\n")
        };
        let rendered = render();
        let dump = write_dump().expect("diag dump");
        let panic_path = write_panic_report("unit").expect("panic file");
        let dump_text = std::fs::read_to_string(&dump).expect("read dump");
        let panic_text = std::fs::read_to_string(&panic_path).expect("read panic");
        let (_agent_ring, agent_debug) =
            malformed_record("/agent", body.as_bytes(), "expected value");
        let (_udp_ring, udp_debug) =
            malformed_record("udp", &circuit, "datagram exceeds udp.datagram");
        for secret in forbidden {
            assert!(!ring.contains(secret), "ring contains {secret}");
            assert!(!rendered.contains(secret), "render contains {secret}");
            assert!(!dump_text.contains(secret), "dump contains {secret}");
            assert!(!panic_text.contains(secret), "panic file contains {secret}");
            assert!(
                !agent_debug.contains(secret),
                "create-agent debug contains {secret}"
            );
            assert!(
                !udp_debug.contains(secret),
                "use-circuit debug contains {secret}"
            );
        }
        assert!(agent_debug.contains("hex="));
        assert!(udp_debug.contains("hex="));
        assert!(agent_debug.contains(&"2a".repeat(seed.len())));
        assert!(udp_debug.contains(&"2a".repeat(20)));
        assert!(panic_text.contains("panic: unit"));
        assert!(dump_text.contains("error=expected value"));
        let _ = std::fs::remove_file(dump);
        let _ = std::fs::remove_file(panic_path);
    }

    #[test]
    fn an_error_chain_is_one_line() {
        let _hold = hold();
        reset();
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("inner")
            }
        }
        impl std::error::Error for Inner {}
        #[derive(Debug)]
        struct Outer;
        impl std::fmt::Display for Outer {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("outer")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&Inner)
            }
        }
        log_error("udp", &Outer);
        let guard = lock();
        assert!(guard.ring[0].contains("outer: inner"));
    }

    #[test]
    fn the_dump_names_the_pin_and_the_hit() {
        let _hold = hold();
        reset();
        set_redactor(|text| text.replace("secret-value", "[redacted]"));
        set_context("ferroan-region", "Test");
        set_configuration("token=secret-value");
        hit(&sample(9));
        let text = render();
        assert!(text.contains("pin: c70fee205f64ee8672a8d607cef6a733931da951"));
        assert!(text.contains("http.post-body"));
        assert!(text.contains("server: ferroan-region"));
        assert!(text.contains("[redacted]"));
        assert!(!text.contains("secret-value"));
    }

    #[test]
    fn every_catalog_id_is_listed_in_limits_md() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/LIMITS.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for id in catalog_const_ids() {
            assert!(text.contains(&format!("`{id}`")), "missing {id}");
        }
    }

    #[test]
    fn a_hit_is_counted_from_its_id() {
        let _hold = hold();
        reset();
        refuse("not-a-catalog-id", Kind::Hard, 3, 4, "test");
        let metrics = metrics();
        assert!(
            metrics.contains(
                "ferroan_limit_hits_total{limit=\"not-a-catalog-id\",action=\"refuse\"} 1"
            )
        );
        assert_eq!(
            lock()
                .counters
                .get(&("not-a-catalog-id", "refuse"))
                .copied(),
            Some(1)
        );
    }

    fn catalog_const_ids() -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/catalog.rs");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let mut ids = Vec::new();
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("pub const ") else {
                continue;
            };
            let Some((_, rest)) = rest.split_once(':') else {
                continue;
            };
            let Some(id) = rest.split('"').nth(1) else {
                continue;
            };
            if !id.is_empty() {
                ids.push(id.to_owned());
            }
        }
        assert!(!ids.is_empty());
        ids
    }
}
