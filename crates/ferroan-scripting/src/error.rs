// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Limit {
        id: &'static str,
    },
    Invalid {
        messages: Vec<String>,
    },
    Object(String),
    /// `MMRScriptObjCode` rejects this before it reads the source hash.
    Version {
        found: i32,
        accepted: i32,
    },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Limit { id } => write!(f, "limit {id}"),
            Failure::Invalid { messages } => write!(f, "{}", messages.join(" | ")),
            Failure::Object(message) => write!(f, "{message}"),
            Failure::Version { found, accepted } => {
                write!(f, "Object version is {found} but accept only {accepted}")
            }
        }
    }
}

impl std::error::Error for Failure {}
