// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read one datagram from stdin and parse it.
//!
//! `cargo xtask check` compiles this. A fuzzer can feed it the same way:
//! malformed packets return errors and do not panic.

use std::io::Read;

fn main() {
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    if buf.len() > ferroan_protocol::packet::MAX_DATAGRAM {
        buf.truncate(ferroan_protocol::packet::MAX_DATAGRAM);
    }
    let _ = ferroan_protocol::reencode_datagram(&buf);
}
