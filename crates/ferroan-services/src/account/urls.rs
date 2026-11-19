// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The two `ServiceURLs` encodings.
//!
//! The database column is `HttpUtility.UrlEncode(key)=UrlEncode(value)` pairs
//! separated by spaces. The HTTP dictionary value is unencoded `key*value;`
//! pairs. `url_encode` matches the bytes `ferroan-http` documents for
//! `HttpUtility`: UTF-8, space as `+`, lowercase hex, and the safe set
//! `! ( ) * - . 0-9 A-Z _ a-z`.

pub(crate) fn service_urls_http(urls: &[(String, String)]) -> String {
    let mut out = String::new();
    for (key, value) in urls {
        out.push_str(key);
        out.push('*');
        out.push_str(value);
        out.push(';');
    }
    out
}

pub(crate) fn service_urls_db(urls: &[(String, String)]) -> String {
    let mut out = String::new();
    for (index, (key, value)) in urls.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&url_encode(key));
        out.push('=');
        out.push_str(&url_encode(value));
    }
    out
}

/// Split on spaces, then on `=`. A piece that is not exactly two parts is
/// skipped, which is what `MakeUserAccount` does.
pub(crate) fn service_urls_from_db(text: &str) -> Vec<(String, String)> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for part in text.split(' ') {
        let pieces: Vec<&str> = part.split('=').collect();
        if pieces.len() != 2 {
            continue;
        }
        out.push((url_decode(pieces[0]), url_decode(pieces[1])));
    }
    out
}

fn url_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if is_url_safe(byte) {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn is_url_safe(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'(' | b')' | b'*' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'_' | b'a'..=b'z'
    )
}

fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    out.push((high << 4) | low);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_encoding_round_trips_a_space() {
        let urls = vec![
            ("HomeURI".to_string(), String::new()),
            (
                "AssetServerURI".to_string(),
                "http://127.0.0.1/a b".to_string(),
            ),
        ];
        let stored = service_urls_db(&urls);
        assert_eq!(
            stored,
            "HomeURI= AssetServerURI=http%3a%2f%2f127.0.0.1%2fa+b"
        );
        assert_eq!(service_urls_from_db(&stored), urls);
        assert_eq!(
            service_urls_http(&urls),
            "HomeURI*;AssetServerURI*http://127.0.0.1/a b;"
        );
    }
}
