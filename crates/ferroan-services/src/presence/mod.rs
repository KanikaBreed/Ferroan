/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

// Ported from Source/OpenSim.Data/IPresenceData.cs and
// Source/OpenSim.Data.Null/NullPresenceData.cs in KanikaBreed/OpenSim-Tranquillity
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b (Pin8).
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Presence rows and the process-wide in-memory provider used by standalone regions.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use ferroan_diag::{
    Kind, PRESENCE_FIELDS, PRESENCE_NULL_SESSIONS, PRESENCE_RESULTS, PRESENCE_ROW_BYTES,
};
use uuid::Uuid;

const MAX_SESSIONS: usize = 4096;
const MAX_RESULTS: usize = 1024;
const MAX_FIELDS: usize = 64;
const MAX_ROW_BYTES: usize = 16 * 1024;

/// A Presence row. The region can change while two services hold the same row.
/// UserID is also used for Hypergrid identities and need not be a UUID.
pub struct PresenceData {
    pub user_id: String,
    pub session_id: Uuid,
    region_id: Mutex<Uuid>,
    pub data: BTreeMap<String, String>,
}

impl PresenceData {
    pub fn new(
        user_id: String,
        session_id: Uuid,
        region_id: Uuid,
        data: BTreeMap<String, String>,
    ) -> Self {
        Self {
            user_id,
            session_id,
            region_id: Mutex::new(region_id),
            data,
        }
    }

    pub fn region_id(&self) -> Uuid {
        *lock(&self.region_id)
    }

    fn validate(&self) -> Result<(), PresenceLimit> {
        check(PRESENCE_FIELDS, MAX_FIELDS, self.data.len())?;
        let bytes = self
            .data
            .iter()
            .fold(self.user_id.capacity(), |total, (key, value)| {
                total
                    .saturating_add(key.capacity())
                    .saturating_add(value.capacity())
            });
        check(PRESENCE_ROW_BYTES, MAX_ROW_BYTES, bytes)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("Presence exceeds {id} ({observed} > {maximum})")]
pub struct PresenceLimit {
    pub id: &'static str,
    pub maximum: usize,
    pub observed: usize,
}

fn check(id: &'static str, maximum: usize, observed: usize) -> Result<(), PresenceLimit> {
    if observed <= maximum {
        return Ok(());
    }
    ferroan_diag::refuse(id, Kind::Hard, maximum as u64, observed as u64, "presence");
    Err(PresenceLimit {
        id,
        maximum,
        observed,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

enum Filter<'a> {
    Session(Option<Uuid>),
    Region(Option<Uuid>),
    User(&'a str),
    Data(&'a str, &'a str),
}

impl<'a> Filter<'a> {
    fn new(field: &'a str, value: &'a str) -> Result<Self, PresenceLimit> {
        check(
            PRESENCE_ROW_BYTES,
            MAX_ROW_BYTES,
            field.len().max(value.len()),
        )?;
        Ok(match field {
            "SessionID" => Self::Session(parse_id(value)),
            "RegionID" => Self::Region(parse_id(value)),
            "UserID" => Self::User(value),
            _ => Self::Data(field, value),
        })
    }

    fn matches(&self, row: &PresenceData) -> bool {
        match *self {
            Self::Session(id) => Some(row.session_id) == id,
            Self::Region(id) => Some(row.region_id()) == id,
            Self::User(user) => row.user_id == user,
            Self::Data(key, value) => row.data.get(key).is_some_and(|stored| stored == value),
        }
    }
}

fn parse_id(value: &str) -> Option<Uuid> {
    let value = value.trim_matches(' ');
    match value.len() {
        32 | 36 => Uuid::parse_str(value).ok(),
        _ => None,
    }
}

/// Dictionary enumeration follows slot order. Replacing a session keeps its
/// slot; insertion after deletion reuses the last slot freed. The service
/// may select the first of several sessions returned for a user.
#[derive(Default)]
struct Sessions {
    index: HashMap<Uuid, usize>,
    slots: Vec<Option<Arc<PresenceData>>>,
    free: Vec<usize>,
}

impl Sessions {
    fn get(&self, session: Uuid) -> Option<&Arc<PresenceData>> {
        self.index
            .get(&session)
            .and_then(|&i| self.slots[i].as_ref())
    }

    fn remove_matching(&mut self, filter: &Filter<'_>) -> bool {
        let before = self.index.len();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if let Some(row) = slot
                && filter.matches(row)
            {
                self.index.remove(&row.session_id);
                *slot = None;
                self.free.push(i);
            }
        }
        self.index.len() < before
    }
}

static PROCESS_SESSIONS: LazyLock<Arc<Mutex<Sessions>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Sessions::default())));

#[derive(Clone)]
pub struct NullPresenceData {
    sessions: Arc<Mutex<Sessions>>,
}

impl Default for NullPresenceData {
    fn default() -> Self {
        Self {
            sessions: Arc::clone(&PROCESS_SESSIONS),
        }
    }
}

impl NullPresenceData {
    /// Store by session, preserving references held by callers. Replacing a
    /// session detaches its old row; subsequent reports update the new row.
    pub fn store(&self, row: Arc<PresenceData>) -> Result<(), PresenceLimit> {
        row.validate()?;
        let mut sessions = lock(&self.sessions);
        if let Some(&i) = sessions.index.get(&row.session_id) {
            sessions.slots[i] = Some(row);
            return Ok(());
        }
        check(
            PRESENCE_NULL_SESSIONS,
            MAX_SESSIONS,
            sessions.index.len() + 1,
        )?;
        let id = row.session_id;
        let i = if let Some(i) = sessions.free.pop() {
            sessions.slots[i] = Some(row);
            i
        } else {
            sessions.slots.push(Some(row));
            sessions.slots.len() - 1
        };
        sessions.index.insert(id, i);
        Ok(())
    }

    pub fn get(&self, session: Uuid) -> Option<Arc<PresenceData>> {
        lock(&self.sessions).get(session).cloned()
    }

    pub fn get_by_field(
        &self,
        field: &str,
        value: &str,
    ) -> Result<Vec<Arc<PresenceData>>, PresenceLimit> {
        let filter = Filter::new(field, value)?;
        if let Filter::Session(id) = filter {
            return Ok(id.and_then(|id| self.get(id)).into_iter().collect());
        }
        let sessions = lock(&self.sessions);
        let mut result = Vec::new();
        for row in sessions
            .slots
            .iter()
            .flatten()
            .filter(|row| filter.matches(row))
        {
            check(PRESENCE_RESULTS, MAX_RESULTS, result.len() + 1)?;
            result.push(Arc::clone(row));
        }
        Ok(result)
    }

    pub fn delete(&self, field: &str, value: &str) -> Result<bool, PresenceLimit> {
        let filter = Filter::new(field, value)?;
        Ok(lock(&self.sessions).remove_matching(&filter))
    }

    pub fn report_agent(&self, session: Uuid, region: Uuid) -> bool {
        let sessions = lock(&self.sessions);
        if let Some(row) = sessions.get(session) {
            *lock(&row.region_id) = region;
            true
        } else {
            false
        }
    }

    pub fn logout_region_agents(&self, region: Uuid) {
        lock(&self.sessions).remove_matching(&Filter::Region(Some(region)));
    }

    pub fn verify_agent(&self, _agent: Uuid, _secure_session: Uuid) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
