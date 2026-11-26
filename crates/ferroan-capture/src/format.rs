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

fn header_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], FormatError> {
    bytes.try_into().map_err(|_| FormatError::Truncated)
}

pub fn read_capture(bytes: &[u8]) -> Result<Vec<Record>, FormatError> {
    read_capture_limited(bytes, MAX_RECORDS)
}

/// Refuse excess records before allocating them for a bounded viewer trace.
pub fn read_capture_limited(bytes: &[u8], limit: usize) -> Result<Vec<Record>, FormatError> {
    let limit = limit.min(MAX_RECORDS);
    if bytes.len() < MAGIC.len() {
        return Err(FormatError::Short);
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(FormatError::Magic);
    }
    let mut i = MAGIC.len();
    let mut records = Vec::new();
    while i < bytes.len() {
        if records.len() == limit {
            return Err(FormatError::Count);
        }
        if bytes.len() - i < 13 {
            return Err(FormatError::Truncated);
        }
        let kind_byte = bytes[i];
        let kind = Kind::from_u8(kind_byte).ok_or(FormatError::Kind(kind_byte))?;
        let time_ms = u64::from_le_bytes(header_array(&bytes[i + 1..i + 9])?);
        let len = u32::from_le_bytes(header_array(&bytes[i + 9..i + 13])?) as usize;
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

/// Read every complete record and ignore a torn tail.
///
/// The region is stopped with `SIGTERM`. A write that was cut off is not a
/// record, and it must not hide the records that were flushed before it.
pub fn read_complete_prefix(bytes: &[u8]) -> Result<Vec<Record>, FormatError> {
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
            break;
        }
        let kind = match Kind::from_u8(bytes[i]) {
            Some(kind) => kind,
            None => return Err(FormatError::Kind(bytes[i])),
        };
        let time_ms = u64::from_le_bytes(header_array(&bytes[i + 1..i + 9])?);
        let len = u32::from_le_bytes(header_array(&bytes[i + 9..i + 13])?) as usize;
        if len > kind.max_len() {
            return Err(FormatError::Length);
        }
        let start = i + 13;
        let Some(end) = start.checked_add(len) else {
            break;
        };
        if end > bytes.len() {
            break;
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

    #[test]
    fn a_torn_tail_is_dropped_and_the_complete_record_stays() {
        let records = vec![Record {
            kind: Kind::UdpIn,
            time_ms: 9,
            bytes: vec![9, 8, 7],
        }];
        let mut bytes = write_capture(&records).unwrap();
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(read_capture(&bytes).unwrap_err(), FormatError::Truncated);
        assert_eq!(read_complete_prefix(&bytes).unwrap(), records);
    }
}
