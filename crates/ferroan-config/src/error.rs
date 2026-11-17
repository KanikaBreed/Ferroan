// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Failures while reading or resolving a configuration.
//!
//! The codes are the ones `tools/config-oracle` writes. They name the Nini or
//! Tranquillity outcome, not a second parser's wording.

use std::fmt;

/// A configuration could not be parsed or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The text is not INI that Nini's standard reader accepts.
    Parse,
    /// `${key}` names a key that is not in the current section.
    ExpandKey { key: String },
    /// `${Section|key}` names a section that is not loaded.
    ExpandSection { section: String },
    /// A key's value refers to the key itself (`${SameKey}`).
    ExpandSelf { key: String },
    /// The region master file was requested and is not on disk.
    MissingMaster { name: String },
    /// The region loader read no file.
    NoConfig,
    /// An include is an `http` or `https` URI. Ferroan does not fetch those.
    RemoteInclude { value: String },
    /// The region loader would fall back to `OpenSim.xml`.
    XmlConfig,
    /// A size, count or expansion bound was hit.
    Limit { what: &'static str },
    /// A file or directory could not be read.
    Io { path: String, message: String },
}

impl ConfigError {
    /// Stable code written in oracle fixtures.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::ExpandKey { .. } => "expand-key",
            Self::ExpandSection { .. } => "expand-section",
            Self::ExpandSelf { .. } => "expand-self",
            Self::MissingMaster { .. } => "missing-master",
            Self::NoConfig => "no-config",
            Self::RemoteInclude { .. } => "remote-include",
            Self::XmlConfig => "xml-config",
            Self::Limit { .. } => "limit",
            Self::Io { .. } => "io",
        }
    }

    /// The detail the oracle writes after the code. Empty when the code is enough.
    pub fn detail(&self) -> &str {
        match self {
            Self::ExpandKey { key }
            | Self::ExpandSelf { key }
            | Self::MissingMaster { name: key } => key,
            Self::ExpandSection { section } => section,
            Self::RemoteInclude { value } => value,
            Self::Io { message, .. } => message,
            _ => "",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => formatter.write_str("ini parse error"),
            Self::ExpandKey { key } => write!(formatter, "substitution key not found: {key}"),
            Self::ExpandSection { section } => {
                write!(formatter, "substitution section not found: {section}")
            }
            Self::ExpandSelf { key } => {
                write!(formatter, "key {key} substitutes itself")
            }
            Self::MissingMaster { name } => write!(formatter, "master ini file {name} not found"),
            Self::NoConfig => formatter.write_str("could not load any configuration"),
            Self::RemoteInclude { value } => {
                write!(formatter, "remote include is not fetched: {value}")
            }
            Self::XmlConfig => formatter.write_str("XML configuration is not read"),
            Self::Limit { what } => write!(formatter, "configuration limit exceeded: {what}"),
            Self::Io { path, message } => write!(formatter, "could not read {path}: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}
