// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Limit { id: &'static str },
    Invalid { messages: Vec<String> },
    Object(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Limit { id } => write!(f, "limit {id}"),
            Failure::Invalid { messages } => write!(f, "{}", messages.join(" | ")),
            Failure::Object(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Failure {}
