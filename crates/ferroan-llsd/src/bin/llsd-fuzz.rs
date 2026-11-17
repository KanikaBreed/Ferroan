// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read one buffer from stdin and parse it as XML, binary and notation LLSD.
//!
//! `cargo xtask check` compiles this. A fuzzer can feed it the same way:
//! the parsers return errors for bad input and do not panic on the limits.

use std::io::Read;

fn main() {
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    let _ = ferroan_llsd::from_xml(&buf);
    let _ = ferroan_llsd::from_binary(&buf);
    let _ = ferroan_llsd::from_notation(&buf);
}
