// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Session captures for a Tranquillity region.
//!
//! The file layout is shared with `tools/capture/tranquillity-capture.patch`,
//! which writes the UDP records from `LLUDPServer` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`. This crate writes the HTTP
//! records, redacts both, and checks UDP datagrams. A packet the server sent
//! must encode back to the same bytes. A packet the client sent may differ
//! only in packed quaternion components, within the packing tolerance.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod format;
mod proxy;
mod redact;

pub use format::{
    Kind, Record, read_capture, read_capture_limited, read_complete_prefix, write_capture,
};
pub use proxy::{
    DEFAULT_REPLY_TIMEOUT, ProxyStatus, serve, serve_on, serve_on_with_status,
    serve_on_with_timeout, serve_with_timeout,
};
pub use redact::{UdpMismatch, check_udp, redact_records, udp_mismatch, write_udp_mismatch};
