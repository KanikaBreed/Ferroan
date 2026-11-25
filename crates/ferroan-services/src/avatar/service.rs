// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported from OpenSim.Services.AvatarService/AvatarService.cs and OpenSim.Services.Interfaces/IAvatarService.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use ferroan_http::{MAX_FIELDS, MAX_QUERY_BYTES};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{AvatarStore, StoreError};

/// Raw service data. The request dictionary also retains a supplied AvatarType entry.
#[derive(Debug, PartialEq)]
pub struct AvatarData {
    pub avatar_type: i32,
    pub fields: Vec<(String, String)>,
}

pub struct AvatarService {
    store: AvatarStore,
    // Hold a principal's lock across the entire replace or cleanup. A fixed
    // number of locks avoids retaining one entry per avatar ever seen.
    locks: [Mutex<()>; 64],
}

impl AvatarService {
    pub fn new(store: AvatarStore) -> Self {
        Self {
            store,
            locks: std::array::from_fn(|_| Mutex::new(())),
        }
    }

    pub(super) fn pool_occupancy(&self) -> (u32, u32) {
        self.store.pool_occupancy()
    }

    fn lock(&self, id: Uuid) -> &Mutex<()> {
        &self.locks[usize::from(id.as_bytes()[15]) % self.locks.len()]
    }

    pub async fn get_avatar(&self, id: Uuid) -> Result<AvatarData, StoreError> {
        let _guard = self.lock(id).lock().await;
        let rows = self.store.service_rows(&id.to_string(), false).await?;
        let mut data = AvatarData {
            avatar_type: i32::from(rows.is_empty()),
            fields: Vec::new(),
        };
        for row in rows {
            if row.name == "AvatarType" {
                data.avatar_type = row
                    .value
                    .trim()
                    .parse()
                    .map_err(|_| StoreError::Row("invalid AvatarType".into()))?;
            } else {
                data.fields.push((row.name, row.value));
            }
        }
        Ok(data)
    }

    pub async fn set_avatar(&self, id: Uuid, data: &AvatarData) -> Result<bool, StoreError> {
        check_fields(
            data.fields.len(),
            data.fields
                .iter()
                .map(|(k, v)| k.len().saturating_add(v.len())),
        )?;
        let _guard = self.lock(id).lock().await;
        let principal = id.to_string();
        let mut keep: HashSet<_> = data.fields.iter().map(|(k, _)| k.as_str()).collect();
        keep.insert("AvatarType");
        self.delete_stale(&principal, &keep).await?;
        let stored = self.store_appearance(&principal, data).await?;
        if !stored {
            self.delete_stale(&principal, &HashSet::new()).await?;
        }
        Ok(stored)
    }

    async fn delete_stale(&self, principal: &str, keep: &HashSet<&str>) -> Result<(), StoreError> {
        for row in self.store.service_rows(principal, true).await? {
            if !row.name.starts_with("Bake") && !keep.contains(row.name.as_str()) {
                self.store.delete(principal, &row.name).await?;
            }
        }
        Ok(())
    }

    async fn store_appearance(
        &self,
        principal: &str,
        data: &AvatarData,
    ) -> Result<bool, StoreError> {
        if !self
            .store
            .upsert(principal, "AvatarType", &data.avatar_type.to_string())
            .await?
        {
            return Ok(false);
        }
        for (name, value) in &data.fields {
            let height;
            let value = if name == "AvatarHeight" {
                height = crate::griduser::format_f32(rectified_height(value));
                &height
            } else {
                value
            };
            if !self.store.upsert(principal, name, value).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn reset_avatar(&self, id: Uuid) -> Result<bool, StoreError> {
        let _guard = self.lock(id).lock().await;
        self.store.delete_principal(&id.to_string()).await
    }

    pub async fn set_items(
        &self,
        id: Uuid,
        names: &[String],
        values: &[String],
    ) -> Result<bool, StoreError> {
        if names.len() != values.len() {
            return Ok(false);
        }
        check_fields(
            names.len(),
            names
                .iter()
                .zip(values)
                .map(|(k, v)| k.len().saturating_add(v.len())),
        )?;
        let _guard = self.lock(id).lock().await;
        let principal = id.to_string();
        for (name, value) in names.iter().zip(values) {
            if !self.store.upsert(&principal, name, value).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn remove_items(&self, id: Uuid, names: &[String]) -> Result<bool, StoreError> {
        check_fields(names.len(), names.iter().map(String::len))?;
        let _guard = self.lock(id).lock().await;
        let principal = id.to_string();
        for name in names {
            self.store.delete(&principal, name).await?;
        }
        Ok(true)
    }
}

fn check_fields(count: usize, sizes: impl Iterator<Item = usize>) -> Result<(), StoreError> {
    check_bound(ferroan_diag::AVATAR_ROWS, MAX_FIELDS, count)?;
    check_bound(
        ferroan_diag::AVATAR_ROW_BYTES,
        MAX_QUERY_BYTES,
        sizes.fold(0usize, usize::saturating_add),
    )
}

fn check_bound(id: &'static str, limit: usize, used: usize) -> Result<(), StoreError> {
    if used > limit {
        ferroan_diag::refuse(
            id,
            ferroan_diag::Kind::Hard,
            limit as u64,
            used as u64,
            "avatar",
        );
        return Err(StoreError::Limit);
    }
    Ok(())
}

fn rectified_height(raw: &str) -> f32 {
    let text = raw.trim();
    let value = if !text.contains('.') && text.matches(',').count() == 1 {
        text.replace(',', ".").parse::<f32>().ok()
    } else if text.contains(',') {
        // The pin falls back to the current culture. In the dot-decimal
        // culture, group separators are accepted only in the integer part.
        let end = text.find(['.', 'e', 'E']).unwrap_or(text.len());
        let integer = text[..end].trim_start_matches(['+', '-']);
        if integer.starts_with(',') || text[end..].contains(',') {
            None
        } else {
            text.replace(',', "").parse::<f32>().ok()
        }
    } else {
        text.parse::<f32>().ok()
    };
    value
        .filter(|v| v.is_nan() || (0.0..=10.0).contains(v))
        .unwrap_or(1.771488)
}
