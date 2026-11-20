// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replace secrets before any line is written. Short values are ignored so a
//! circuit code cannot wipe a timestamp, and a general secret must be at least
//! 8 characters.

use std::sync::Mutex;

const REPLACEMENT: &str = "[redacted]";
const CAPABILITY: &str = "[redacted-capability]";

static SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn register_secret(secret: &str) {
    remember(secret, 8);
}

pub fn register_circuit_code(code: u32) {
    remember(&code.to_string(), 6);
}

fn remember(secret: &str, min: usize) {
    let secret = secret.trim();
    if secret.len() < min || secret.len() > 2048 {
        return;
    }
    let mut guard = SECRETS.lock().unwrap_or_else(|poison| poison.into_inner());
    if guard.iter().any(|item| item == secret) {
        return;
    }
    guard.push(secret.to_owned());
    guard.sort_by_key(|item| std::cmp::Reverse(item.len()));
}

pub fn redact(text: &str) -> String {
    let guard = SECRETS.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut out = text.to_owned();
    for secret in guard.iter() {
        if out.contains(secret) {
            out = out.replace(secret, REPLACEMENT);
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
mod tests {
    use super::*;

    #[test]
    fn secrets_queries_and_capability_urls_are_removed() {
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
}
