// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replace secrets before any line is written. Short values are ignored so a
//! circuit code cannot wipe a timestamp, and a general secret must be at least
//! 8 characters.
//!
//! A login holds its session id, secure session id, circuit code and seed path
//! in [`SessionSecrets`]. Dropping that value forgets them. The same text
//! shared by two live sessions stays until the last one drops. The list stops
//! at [`MAX_SECRETS`]; a distinct secret past that is not stored and the miss
//! is `log.redact-secrets`. `ferroan-region` checks that this bound still
//! covers every ordinary root and every bypass root, including one
//! overlapping replacement.

use std::sync::Mutex;

use ferroan_diag::{Kind, REDACT_SECRETS};

const REPLACEMENT: &str = "[redacted]";
const CAPABILITY: &str = "[redacted-capability]";
/// Ordinary roots plus staff and god bypass roots, each overlapping one
/// replacement, times the four login secrets (session id, secure session id,
/// circuit code, seed path). `ferroan-region` asserts this equals
/// `(ROOT_AGENT_LIMIT + ROOT_BYPASS_LIMIT) * 2 * 4`.
pub const MAX_SECRETS: usize = 488;

struct Entry {
    text: String,
    holders: u32,
}

static SECRETS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

pub fn register_secret(secret: &str) {
    let _stored = remember(secret, 8);
}

pub fn register_circuit_code(code: u32) {
    let text = code.to_string();
    let _stored = remember(&text, 6);
}

/// Secrets remembered for one region session. Dropping it releases that
/// session's hold. Do not log this value.
#[derive(Default)]
pub struct SessionSecrets {
    held: Vec<String>,
}

impl SessionSecrets {
    /// `seed_path` is the seed capability path registered for this login,
    /// including the `/CAPS/` prefix.
    pub fn login(
        session_id: &str,
        secure_session_id: &str,
        circuit_code: u32,
        seed_path: &str,
    ) -> Self {
        let mut secrets = Self { held: Vec::new() };
        secrets.hold(session_id, 8);
        secrets.hold(secure_session_id, 8);
        secrets.hold(&circuit_code.to_string(), 6);
        secrets.hold(seed_path, 8);
        secrets
    }

    fn hold(&mut self, secret: &str, min: usize) {
        let secret = secret.trim();
        if remember(secret, min) {
            self.held.push(secret.to_owned());
        }
    }
}

impl Drop for SessionSecrets {
    fn drop(&mut self) {
        for secret in self.held.drain(..) {
            release(&secret);
        }
    }
}

/// Returns whether `secret` is now held. A value already in the list gains
/// another holder and does not take a new slot.
fn remember(secret: &str, min: usize) -> bool {
    let secret = secret.trim();
    if secret.len() < min || secret.len() > 2048 {
        return false;
    }
    let mut guard = SECRETS.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(entry) = guard.iter_mut().find(|entry| entry.text == secret) {
        entry.holders = entry.holders.saturating_add(1);
        return true;
    }
    if guard.len() >= MAX_SECRETS {
        let observed = (guard.len() as u64).saturating_add(1);
        drop(guard);
        ferroan_diag::refuse(
            REDACT_SECRETS,
            Kind::Hard,
            MAX_SECRETS as u64,
            observed,
            "log",
        );
        return false;
    }
    guard.push(Entry {
        text: secret.to_owned(),
        holders: 1,
    });
    guard.sort_by_key(|entry| std::cmp::Reverse(entry.text.len()));
    true
}

fn release(secret: &str) {
    let mut guard = SECRETS.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(pos) = guard.iter().position(|entry| entry.text == secret) else {
        return;
    };
    if guard[pos].holders <= 1 {
        guard.remove(pos);
    } else {
        guard[pos].holders -= 1;
    }
}

pub fn redact(text: &str) -> String {
    let guard = SECRETS.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut out = text.to_owned();
    for secret in guard.iter() {
        if out.contains(&secret.text) {
            out = out.replace(&secret.text, REPLACEMENT);
        }
    }
    drop(guard);
    scrub_capabilities(&scrub_queries(&out))
}

fn scrub_queries(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(skip) = query_key(bytes, index) {
            out.extend_from_slice(&bytes[index..skip]);
            index = skip;
            let start = index;
            while index < bytes.len() && bytes[index] != b'&' && !bytes[index].is_ascii_whitespace()
            {
                index += 1;
            }
            if start != index {
                out.extend_from_slice(REPLACEMENT.as_bytes());
            }
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_key(bytes: &[u8], index: usize) -> Option<usize> {
    const KEYS: &[&[u8]] = &[b"password=", b"pass=", b"consolepass="];
    for key in KEYS {
        if bytes[index..].len() >= key.len()
            && bytes[index..index + key.len()].eq_ignore_ascii_case(key)
        {
            return Some(index + key.len());
        }
    }
    None
}

fn scrub_capabilities(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = capability_at(bytes, index) {
            out.extend_from_slice(CAPABILITY.as_bytes());
            index = end;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn capability_at(bytes: &[u8], index: usize) -> Option<usize> {
    let rest = &bytes[index..];
    let scheme = if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case(b"https://") {
        8
    } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case(b"http://") {
        7
    } else {
        return None;
    };
    let mut end = scheme;
    while end < rest.len()
        && !rest[end].is_ascii_whitespace()
        && rest[end] != b'"'
        && rest[end] != b'\''
    {
        end += 1;
    }
    let url = &rest[..end];
    if url
        .windows(6)
        .any(|window| window.eq_ignore_ascii_case(b"/CAPS/"))
    {
        Some(index + end)
    } else {
        None
    }
}

#[cfg(test)]
fn secret_count() -> usize {
    SECRETS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .len()
}

#[cfg(test)]
pub(crate) fn test_secret_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_queries_and_capability_urls_are_removed() {
        let _hold = test_secret_lock();
        register_secret("session-secret-value");
        register_secret("short");
        register_circuit_code(12);
        register_circuit_code(987654);
        let line = redact(
            "sees short and code 12 and session-secret-value and 987654 password=keep-me-out pass=also-hidden http://127.0.0.1/CAPS/abc/seed",
        );
        assert!(line.contains("short"));
        assert!(line.contains("code 12"));
        assert!(!line.contains("session-secret-value"));
        assert!(!line.contains("987654"));
        assert!(line.contains("[redacted]"));
        assert!(!line.contains("keep-me-out"));
        assert!(!line.contains("also-hidden"));
        assert!(line.contains("[redacted-capability]"));
        assert!(!line.contains("/CAPS/"));
    }

    #[test]
    fn a_closed_session_forgets_its_secrets_and_the_list_is_bounded() {
        let _hold = test_secret_lock();
        let shared = "shared-session-eeeeffff";
        let secure = "secure-close-ccccdddd";
        let path = "/CAPS/close-path-eeee/";
        let first = SessionSecrets::login(shared, secure, 424242, path);
        let second = SessionSecrets::login(shared, "other-secure-gggghhhh", 100000, path);
        assert!(!redact(shared).contains(shared));
        assert!(!redact(secure).contains(secure));
        assert!(!redact("424242").contains("424242"));
        assert!(!redact("100000").contains("100000"));
        assert!(redact("sees 99999 intact").contains("99999"));
        drop(first);
        assert!(
            !redact(shared).contains(shared),
            "the other session still holds it"
        );
        assert!(redact(secure).contains(secure));
        assert!(redact("code 424242").contains("424242"));
        assert!(!redact("100000").contains("100000"));
        drop(second);
        assert!(redact(shared).contains(shared));
        assert!(redact("100000").contains("100000"));

        let start = secret_count();
        let hits_before = refuse_hits(REDACT_SECRETS);
        let mut held = Vec::new();
        let mut refused = None;
        for n in 0..(MAX_SECRETS + 8) {
            let secret = format!("redact-bound-{n:04}-xxxxxxxx");
            let one = hold_one(&secret);
            if redact(&secret).contains(&secret) {
                refused = Some(secret);
                drop(one);
                break;
            }
            held.push(one);
        }
        let secret = refused.expect("a secret past the bound");
        assert_eq!(secret_count(), MAX_SECRETS);
        assert!(refuse_hits(REDACT_SECRETS) > hits_before);
        assert!(ferroan_diag::render().contains("log.redact-secrets"));
        drop(held);
        assert_eq!(secret_count(), start);
        assert!(redact(&secret).contains(&secret));
    }

    fn hold_one(secret: &str) -> SessionSecrets {
        let mut secrets = SessionSecrets { held: Vec::new() };
        secrets.hold(secret, 8);
        secrets
    }

    fn refuse_hits(id: &str) -> u64 {
        let needle = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"refuse\"}} ");
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&needle))
            .and_then(|rest| rest.trim().parse().ok())
            .unwrap_or(0)
    }
}
