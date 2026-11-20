// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Level names from `CategoryLogLevels.FromSettings` and `set log level`.
//! `FATAL` is `ERROR`: `tracing` has no fatal level. `ALL` is trace.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

pub(crate) const OFF: u8 = 0;
pub(crate) const ERROR: u8 = 1;
pub(crate) const WARN: u8 = 2;
pub(crate) const INFO: u8 = 3;
pub(crate) const DEBUG: u8 = 4;
pub(crate) const TRACE: u8 = 5;

#[derive(Clone)]
pub(crate) struct RuntimeLevel(Arc<AtomicU8>);

impl RuntimeLevel {
    pub(crate) fn new(rank: u8) -> Self {
        Self(Arc::new(AtomicU8::new(rank)))
    }

    pub(crate) fn get(&self) -> u8 {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn set(&self, rank: u8) {
        self.0.store(rank, Ordering::Relaxed);
    }
}

pub(crate) fn parse(name: &str) -> Option<u8> {
    match name.trim().to_ascii_lowercase().as_str() {
        "off" | "none" => Some(OFF),
        "error" | "fatal" | "critical" => Some(ERROR),
        "warn" | "warning" => Some(WARN),
        "info" | "information" => Some(INFO),
        "debug" => Some(DEBUG),
        "all" | "trace" => Some(TRACE),
        _ => None,
    }
}

pub(crate) fn name(rank: u8) -> &'static str {
    match rank {
        OFF => "OFF",
        ERROR => "ERROR",
        WARN => "WARN",
        DEBUG => "DEBUG",
        TRACE => "ALL",
        _ => "INFO",
    }
}

pub(crate) fn label(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::ERROR => "ERROR",
        tracing::Level::WARN => "WARN",
        tracing::Level::INFO => "INFO",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::TRACE => "TRACE",
    }
}

pub(crate) fn severity(level: &tracing::Level) -> i32 {
    match *level {
        tracing::Level::TRACE => 1,
        tracing::Level::DEBUG => 5,
        tracing::Level::INFO => 9,
        tracing::Level::WARN => 13,
        tracing::Level::ERROR => 17,
    }
}

pub(crate) fn ceiling(rank: u8) -> Option<tracing::Level> {
    match rank {
        OFF => None,
        ERROR => Some(tracing::Level::ERROR),
        WARN => Some(tracing::Level::WARN),
        DEBUG => Some(tracing::Level::DEBUG),
        TRACE => Some(tracing::Level::TRACE),
        _ => Some(tracing::Level::INFO),
    }
}
