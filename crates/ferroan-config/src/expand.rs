// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `${Section|key}` and `${key}` substitution, in Nini's single pass.
//!
//! `ConfigSourceBase.Expand` writes each value back as it goes, so a later
//! key sees earlier substitutions. A replacement is inserted raw and the
//! scan starts again, which is how a forward reference still resolves.
//! Nini has no iteration cap; a value that expands to itself would loop, so
//! this pass stops at [`EXPAND_STEPS`].

use crate::error::ConfigError;
use crate::parse::Config;

const EXPAND_STEPS: usize = 4_096;

impl Config {
    /// Substitutes `${...}` in every value. See the module comment.
    pub fn expand(&mut self) -> Result<(), ConfigError> {
        for section_index in 0..self.section_count() {
            for key_index in 0..self.key_count(section_index) {
                let key_name = self.key_name(section_index, key_index).to_string();
                let mut result = self.key_value(section_index, key_index).to_string();
                let mut capped = true;
                for _ in 0..EXPAND_STEPS {
                    let Some(start) = result.find("${") else {
                        capped = false;
                        break;
                    };
                    let Some(relative_end) = result[start + 2..].find('}') else {
                        capped = false;
                        break;
                    };
                    let end = start + 2 + relative_end;
                    let search = result[start + 2..end].to_string();
                    if search == key_name {
                        return Err(ConfigError::ExpandSelf { key: key_name });
                    }
                    let replacement = self.expand_value(section_index, &search)?;
                    let token = format!("${{{search}}}");
                    result = result.replace(&token, &replacement);
                }
                if capped && closed_pair_remains(&result) {
                    return Err(ConfigError::Limit { what: "expansion" });
                }
                self.set_key_value(section_index, key_index, result);
            }
        }
        Ok(())
    }

    fn expand_value(&self, section_index: usize, search: &str) -> Result<String, ConfigError> {
        let mut parts = search.split('|');
        let first = parts.next().unwrap_or("");
        if parts.next().is_none() {
            let section_name = self.section_name_at(section_index);
            return self
                .get(&section_name, search)
                .map(str::to_string)
                .ok_or_else(|| ConfigError::ExpandKey {
                    key: search.to_string(),
                });
        }
        // Nini keeps only the first two pieces. `${Section|key|ignored}` looks up key.
        let section_name = first.to_string();
        let key_name = search.split('|').nth(1).unwrap_or("").to_string();
        if self.section_index(&section_name).is_none() {
            return Err(ConfigError::ExpandSection {
                section: section_name,
            });
        }
        self.get(&section_name, &key_name)
            .map(str::to_string)
            .ok_or(ConfigError::ExpandKey { key: key_name })
    }

    fn section_name_at(&self, index: usize) -> String {
        self.section_names().nth(index).unwrap_or("").to_string()
    }
}

fn closed_pair_remains(value: &str) -> bool {
    let Some(start) = value.find("${") else {
        return false;
    };
    value[start + 2..].contains('}')
}
