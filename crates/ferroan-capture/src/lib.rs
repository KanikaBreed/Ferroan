// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Session captures for a Tranquillity region.
//!
//! The file layout is shared with `tools/capture/tranquillity-capture.patch`,
//! which writes the UDP records from `LLUDPServer` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`. This crate writes the HTTP
//! records, redacts both, and checks that every UDP datagram decodes and
//! encodes back to the same bytes.

mod format;
mod proxy;
mod redact;

pub use format::{Kind, Record, read_capture, write_capture};
pub use proxy::serve;
pub use redact::{check_udp, redact_records};
