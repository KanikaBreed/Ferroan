// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Capture file.
//!
//! ```text
//! magic:  FERROCAP1
//! record: kind u8 | time_ms u64 LE | len u32 LE | bytes
//! ```
//!
//! Kind 1 is a UDP datagram the region received. Kind 2 is one it sent.
//! Kind 3 is an HTTP request. Kind 4 is the response. The UDP bytes are the
//! datagram only. The socket address is not stored.

use thiserror::Error;

pub const MAGIC: &[u8] = b"FERROCAP1";
const MAX_UDP: usize = 4096;
const MAX_HTTP: usize = 1_048_576;
const MAX_RECORDS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    UdpIn = 1,
    UdpOut = 2,
    HttpRequest = 3,
    HttpResponse = 4,
}

impl Kind {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::UdpIn),
            2 => Some(Self::UdpOut),
            3 => Some(Self::HttpRequest),
            4 => Some(Self::HttpResponse),
            _ => None,
        }
    }

    fn max_len(self) -> usize {
        match self {
            Self::UdpIn | Self::UdpOut => MAX_UDP,
            Self::HttpRequest | Self::HttpResponse => MAX_HTTP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: Kind,
    pub time_ms: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("capture file is too short")]
    Short,
    #[error("capture file does not start with FERROCAP1")]
    Magic,
    #[error("unknown capture record kind {0}")]
    Kind(u8),
    #[error("capture record is larger than its kind allows")]
    Length,
    #[error("capture record is truncated")]
    Truncated,
    #[error("capture has too many records")]
    Count,
}

pub fn write_capture(records: &[Record]) -> Result<Vec<u8>, FormatError> {
    if records.len() > MAX_RECORDS {
        return Err(FormatError::Count);
    }
    let mut out = Vec::from(MAGIC);
    for record in records {
        if record.bytes.len() > record.kind.max_len() {
            return Err(FormatError::Length);
        }
        out.push(record.kind as u8);
        out.extend_from_slice(&record.time_ms.to_le_bytes());
        out.extend_from_slice(&(record.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&record.bytes);
    }
    Ok(out)
}

pub fn read_capture(bytes: &[u8]) -> Result<Vec<Record>, FormatError> {
    if bytes.len() < MAGIC.len() {
        return Err(FormatError::Short);
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(FormatError::Magic);
    }
    let mut i = MAGIC.len();
    let mut records = Vec::new();
    while i < bytes.len() {
        if records.len() == MAX_RECORDS {
            return Err(FormatError::Count);
        }
        if bytes.len() - i < 13 {
            return Err(FormatError::Truncated);
        }
        let kind_byte = bytes[i];
        let kind = Kind::from_u8(kind_byte).ok_or(FormatError::Kind(kind_byte))?;
        let time_ms = u64::from_le_bytes(bytes[i + 1..i + 9].try_into().expect("8 bytes"));
        let len = u32::from_le_bytes(bytes[i + 9..i + 13].try_into().expect("4 bytes")) as usize;
        if len > kind.max_len() {
            return Err(FormatError::Length);
        }
        let start = i + 13;
        let end = start.checked_add(len).ok_or(FormatError::Truncated)?;
        if end > bytes.len() {
            return Err(FormatError::Truncated);
        }
        records.push(Record {
            kind,
            time_ms,
            bytes: bytes[start..end].to_vec(),
        });
        i = end;
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_round_trips() {
        let records = vec![Record {
            kind: Kind::UdpOut,
            time_ms: 5,
            bytes: vec![1, 2, 3, 4],
        }];
        let bytes = write_capture(&records).unwrap();
        assert_eq!(read_capture(&bytes).unwrap(), records);
    }

    #[test]
    fn a_short_file_and_a_bad_magic_are_errors() {
        assert_eq!(read_capture(b"FERRO").unwrap_err(), FormatError::Short);
        assert_eq!(read_capture(b"FERROCAP0").unwrap_err(), FormatError::Magic);
    }
}
