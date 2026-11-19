// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! INI text as Nini 0.9.2.16 reads it in `IniFileType.Standard`.
//!
//! The reader follows that package: `;` comments, `=` assignment, the first
//! key in a section winning, a repeated section replacing the earlier one,
//! and a quote ending a value. `#` is not a comment.

use std::io::Read;

use crate::error::ConfigError;

const MAX_TEXT: usize = 16 * 1024 * 1024;
const MAX_SECTIONS: usize = 8_192;
const MAX_KEYS_PER_SECTION: usize = 8_192;

/// One loaded configuration. Sections and keys keep the order Nini keeps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    name: String,
    keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    name: String,
    value: String,
}

impl Config {
    /// Parses INI text. Does not substitute `${...}` and does not read includes.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        if text.len() > MAX_TEXT {
            return Err(ConfigError::Limit { what: "ini bytes" });
        }
        let mut reader = Reader::new(text);
        let mut config = Self::default();
        let mut current: Option<usize> = None;
        while let Some(item) = reader.next_item()? {
            match item {
                Item::Empty => {}
                Item::Section(name) => {
                    if let Some(index) = config
                        .sections
                        .iter()
                        .position(|section| section.name == name)
                    {
                        config.sections.remove(index);
                        if let Some(current_index) = current.as_mut()
                            && *current_index > index
                        {
                            *current_index -= 1;
                        }
                    }
                    if config.sections.len() >= MAX_SECTIONS {
                        return Err(ConfigError::Limit { what: "sections" });
                    }
                    config.sections.push(Section {
                        name,
                        keys: Vec::new(),
                    });
                    current = Some(config.sections.len() - 1);
                }
                Item::Key { name, value } => {
                    let Some(index) = current else {
                        continue;
                    };
                    let section = &mut config.sections[index];
                    if section.keys.iter().any(|key| key.name == name) {
                        continue;
                    }
                    if section.keys.len() >= MAX_KEYS_PER_SECTION {
                        return Err(ConfigError::Limit { what: "keys" });
                    }
                    section.keys.push(Key { name, value });
                }
            }
        }
        Ok(config)
    }

    /// Reads a file as UTF-8. A leading byte-order mark is ignored.
    pub fn parse_file(path: &std::path::Path) -> Result<Self, ConfigError> {
        let bytes = read_bounded(path)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| ConfigError::Parse)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        Self::parse(text)
    }

    /// Copies sections from `other`. An existing key is overwritten in place.
    /// A new key is appended. A new section is appended.
    pub fn merge(&mut self, other: &Self) {
        for section in &other.sections {
            if let Some(existing) = self
                .sections
                .iter_mut()
                .find(|item| item.name == section.name)
            {
                for key in &section.keys {
                    if let Some(slot) = existing.keys.iter_mut().find(|item| item.name == key.name)
                    {
                        slot.value.clone_from(&key.value);
                    } else {
                        existing.keys.push(key.clone());
                    }
                }
            } else {
                self.sections.push(section.clone());
            }
        }
    }

    /// Value of `key` in `section`, or `None` when either is absent.
    /// An empty value is `Some("")`. Names are case-sensitive.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|item| item.name == section)
            .and_then(|item| item.keys.iter().find(|entry| entry.name == key))
            .map(|entry| entry.value.as_str())
    }

    /// Section names in load order.
    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|section| section.name.as_str())
    }

    /// Keys of `section` in load order.
    pub fn keys(&self, section: &str) -> impl Iterator<Item = (&str, &str)> {
        self.sections
            .iter()
            .find(|item| item.name == section)
            .into_iter()
            .flat_map(|item| {
                item.keys
                    .iter()
                    .map(|key| (key.name.as_str(), key.value.as_str()))
            })
    }

    /// Every section, key and value in load order.
    pub fn entries(&self) -> Vec<(&str, &str, &str)> {
        let mut rows = Vec::new();
        for section in &self.sections {
            for key in &section.keys {
                rows.push((section.name.as_str(), key.name.as_str(), key.value.as_str()));
            }
        }
        rows
    }

    /// Sets a key, appending the section or the key when it is new.
    /// An existing key is overwritten in place. Used for the command-line source.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        if let Some(existing) = self.sections.iter_mut().find(|item| item.name == section) {
            if let Some(slot) = existing.keys.iter_mut().find(|item| item.name == key) {
                slot.value = value.to_string();
            } else {
                existing.keys.push(Key {
                    name: key.to_string(),
                    value: value.to_string(),
                });
            }
        } else {
            self.sections.push(Section {
                name: section.to_string(),
                keys: vec![Key {
                    name: key.to_string(),
                    value: value.to_string(),
                }],
            });
        }
    }

    /// Keys present in `section` whose names are not in `known`.
    /// The load still succeeds; callers report this list and keep going.
    pub fn unknown_keys<'a>(&'a self, section: &str, known: &[&str]) -> Vec<&'a str> {
        self.keys(section)
            .filter(|(name, _)| !known.contains(name))
            .map(|(name, _)| name)
            .collect()
    }

    pub(crate) fn section_index(&self, name: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|section| section.name == name)
    }

    pub(crate) fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub(crate) fn key_name(&self, section: usize, key: usize) -> &str {
        &self.sections[section].keys[key].name
    }

    pub(crate) fn key_value(&self, section: usize, key: usize) -> &str {
        &self.sections[section].keys[key].value
    }

    pub(crate) fn key_count(&self, section: usize) -> usize {
        self.sections[section].keys.len()
    }

    pub(crate) fn set_key_value(&mut self, section: usize, key: usize, value: String) {
        self.sections[section].keys[key].value = value;
    }

    /// Sets every key of the `Environment` section from `environment`.
    /// A missing variable becomes an empty string, matching
    /// `Util.MergeEnvironmentToConfig` when the variable is unset.
    /// Returns whether the section existed.
    pub(crate) fn apply_environment(
        &mut self,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        let Some(index) = self.section_index("Environment") else {
            return false;
        };
        for key in &mut self.sections[index].keys {
            key.value = environment.get(&key.name).cloned().unwrap_or_default();
        }
        true
    }
}

pub(crate) fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>, ConfigError> {
    let file = std::fs::File::open(path).map_err(|error| ConfigError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if length > MAX_TEXT as u64 {
        return Err(ConfigError::Limit { what: "ini bytes" });
    }
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ConfigError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    if bytes.len() > MAX_TEXT {
        return Err(ConfigError::Limit { what: "ini bytes" });
    }
    Ok(bytes)
}

enum Item {
    Empty,
    Section(String),
    Key { name: String, value: String },
}

struct Reader {
    chars: Vec<char>,
    pos: usize,
    name: String,
    value: String,
}

impl Reader {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            name: String::new(),
            value: String::new(),
        }
    }

    fn next_item(&mut self) -> Result<Option<Item>, ConfigError> {
        loop {
            self.name.clear();
            self.value.clear();
            let ch = self.peek();
            if ch == Some(';') {
                self.bump();
                self.read_comment();
                return Ok(Some(Item::Empty));
            }
            match ch {
                Some(' ' | '\t' | '\r') => {
                    self.skip_whitespace();
                }
                Some('\n') => {
                    self.bump();
                    return Ok(Some(Item::Empty));
                }
                Some('[') => {
                    self.read_section()?;
                    return Ok(Some(Item::Section(std::mem::take(&mut self.name))));
                }
                None => return Ok(None),
                _ => {
                    self.read_key()?;
                    return Ok(Some(Item::Key {
                        name: std::mem::take(&mut self.name),
                        value: std::mem::take(&mut self.value),
                    }));
                }
            }
        }
    }

    fn read_comment(&mut self) {
        self.skip_whitespace();
        loop {
            let ch = self.bump();
            if end_of_line(ch) {
                break;
            }
        }
    }

    fn read_key(&mut self) -> Result<(), ConfigError> {
        loop {
            let ch = self.peek();
            if ch == Some('=') {
                self.bump();
                break;
            }
            if end_of_line(ch) {
                return Err(ConfigError::Parse);
            }
            let Some(ch) = self.bump() else {
                return Err(ConfigError::Parse);
            };
            self.name.push(ch);
        }
        self.read_key_value()?;
        self.search_for_comment();
        trim_end_ws(&mut self.name);
        Ok(())
    }

    fn read_key_value(&mut self) -> Result<(), ConfigError> {
        self.skip_whitespace();
        let mut found_quote = false;
        let mut characters = 0u32;
        loop {
            let ch = self.peek();
            if !is_nini_whitespace(ch) {
                characters += 1;
            }
            if ch == Some('"') {
                self.bump();
                if !found_quote && characters == 1 {
                    found_quote = true;
                    continue;
                }
                break;
            }
            if found_quote && end_of_line(ch) {
                return Err(ConfigError::Parse);
            }
            if ch == Some(';') && !found_quote {
                break;
            }
            if end_of_line(ch) {
                break;
            }
            let Some(ch) = self.bump() else {
                return Err(ConfigError::Parse);
            };
            self.value.push(ch);
        }
        if !found_quote {
            trim_end_ws(&mut self.value);
        }
        Ok(())
    }

    fn read_section(&mut self) -> Result<(), ConfigError> {
        self.bump();
        loop {
            let ch = self.peek();
            if ch == Some(']') {
                break;
            }
            if end_of_line(ch) {
                return Err(ConfigError::Parse);
            }
            let Some(ch) = self.bump() else {
                return Err(ConfigError::Parse);
            };
            self.name.push(ch);
        }
        self.consume_to_end();
        trim_end_ws(&mut self.name);
        Ok(())
    }

    fn search_for_comment(&mut self) {
        let mut ch = self.bump();
        while !end_of_line(ch) {
            if ch == Some(';') {
                self.read_comment();
                break;
            }
            ch = self.bump();
        }
    }

    fn consume_to_end(&mut self) {
        loop {
            let ch = self.bump();
            if end_of_line(ch) {
                break;
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while is_nini_whitespace(self.peek()) {
            if end_of_line(self.peek()) {
                break;
            }
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }
}

fn end_of_line(ch: Option<char>) -> bool {
    ch.is_none() || ch == Some('\n')
}

/// Nini's `IsWhitespace`: space, tab, CR and LF only.
fn is_nini_whitespace(ch: Option<char>) -> bool {
    matches!(ch, Some(' ' | '\t' | '\r' | '\n'))
}

fn trim_end_ws(text: &mut String) {
    let trimmed = text.trim_end();
    text.truncate(trimmed.len());
}
