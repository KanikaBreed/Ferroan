// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Append raw UDP datagrams to a FERROCAP1 file.
//!
//! The record layout matches `crates/ferroan-capture` and the Tranquillity
//! capture patch: kind, Unix time in milliseconds, length, then the datagram.
//! Kind 1 is inbound and kind 2 is outbound. The socket address is not stored.
//! A datagram longer than 4096 bytes is skipped. A write error closes the file
//! and later datagrams are not recorded.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_DATAGRAM: usize = 4096;
const MAGIC: &[u8] = b"FERROCAP1";

pub(crate) struct DatagramLog {
    file: Mutex<Option<File>>,
}

impl DatagramLog {
    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        let mut file = File::create(path)?;
        file.write_all(MAGIC)?;
        file.flush()?;
        Ok(Self {
            file: Mutex::new(Some(file)),
        })
    }

    pub(crate) fn write(&self, kind: u8, bytes: &[u8]) {
        if bytes.is_empty() || bytes.len() > MAX_DATAGRAM {
            return;
        }
        let mut guard = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(file) = guard.as_mut() else {
            return;
        };
        let mut head = [0u8; 13];
        head[0] = kind;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        head[1..9].copy_from_slice(&millis.to_le_bytes());
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        head[9..13].copy_from_slice(&len.to_le_bytes());
        if file
            .write_all(&head)
            .and_then(|_| file.write_all(bytes))
            .and_then(|_| file.flush())
            .is_err()
        {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_capture::{Kind, read_capture};

    #[test]
    fn a_log_round_trips_one_inbound_datagram() {
        let path =
            std::env::temp_dir().join(format!("ferroan-datagram-log-{}", uuid::Uuid::new_v4()));
        let log = DatagramLog::create(&path).unwrap();
        log.write(1, &[1, 2, 3, 4]);
        log.write(1, &[]);
        log.write(1, &[0u8; 4097]);
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, Kind::UdpIn);
        assert_eq!(records[0].bytes, vec![1, 2, 3, 4]);
    }
}
