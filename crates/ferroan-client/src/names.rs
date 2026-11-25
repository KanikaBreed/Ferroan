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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.CoreModules/Framework/UserManagement/UserManagementModule.cs
//     (HandleUUIDNameRequest, GetUser, AddUser, IsResidentAwaitingAccount)
//   Source/OpenSim.Region.CoreModules/Framework/ServiceThrottle/ServiceThrottleModule.cs (Enqueue)
//   Source/OpenSim.Framework.Monitoring/JobEngine.cs (the 5000-job bound, two workers)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs (SendNameReply)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// The per-circuit bound, the cache bound and the reply bound are new Rust.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Avatar names for `UUIDNameRequest`.
//!
//! A cached entry that has been looked up is answered at once, including an
//! id the lookup did not find, which answers `Unknown` and the id: C# answers
//! that cached miss although its first lookup sent nothing (TB89), and
//! Ferroan copies both so every region on a grid answers alike (D73). Any other
//! id waits for a lookup on one region queue served by two workers: the
//! account service, then GridUser. A found id is answered; an id neither
//! service knows gets no reply and is cached as unknown. A resident whose
//! names came with its arrival and whose account is missing is answered with
//! those names and stays untried. Entries last 30 minutes after a lookup and
//! an hour after an arrival. The lookups never run on the circuit or the
//! frame thread.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferroan_diag::{
    Kind, REGION_NAME_CACHE, REGION_NAME_CIRCUIT_LOOKUPS, REGION_NAME_LOOKUPS, drop_bound,
};
use ferroan_protocol::msgs::{UUIDNameReply, uuid_name_reply};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::Instant;
use uuid::Uuid;

/// `JobEngine.m_jobQueue`, a `BlockingCollection` of 5000 (`JobEngine.cs`
/// line 66). A job past it is not recorded. The queue really holds 5,001:
/// `serve_lookups` holds one job while it waits for a worker.
pub const MAX_NAME_LOOKUPS: usize = 5000;
/// `new JobEngine("ServiceThrottle", "ServiceThrottle", 5000, 2)`: two
/// lookups at a time. The 5000 there is the worker's idle timeout in ms.
const LOOKUP_WORKERS: usize = 2;
/// Lookups one circuit may have waiting. C# has no per-client bound.
pub const MAX_CIRCUIT_LOOKUPS: usize = 512;
/// Cached ids. `ExpiringCacheOS` has no entry bound.
pub const MAX_NAME_CACHE: usize = 8192;
/// `GetUser` adds a looked-up entry for 1,800,000 ms.
const LOOKED_UP_FOR: Duration = Duration::from_millis(1_800_000);
/// `AddUser` keeps a local arrival for `LOCALEXPIRE`, 3,600,000 ms.
const ARRIVAL_FOR: Duration = Duration::from_millis(3_600_000);
/// `Util.StringToBytes256` keeps 255 bytes with the terminator.
const NAME_FIELD: usize = 255;

/// A first and a last name as the reply carries them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserName {
    pub first: String,
    pub last: String,
}

impl UserName {
    pub fn new(first: impl Into<String>, last: impl Into<String>) -> Self {
        Self {
            first: first.into(),
            last: last.into(),
        }
    }

    /// Only what the wire can carry is kept, so a service cannot make the
    /// cache hold more than two 255-byte fields per id.
    fn clamped(self) -> Self {
        Self {
            first: clamp(self.first),
            last: clamp(self.last),
        }
    }
}

fn clamp(text: String) -> String {
    let bytes = crate::arrival::string_to_bytes(&text, NAME_FIELD);
    let end = bytes.len().saturating_sub(1);
    String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
}

pub type NameFuture = Pin<Box<dyn Future<Output = Option<UserName>> + Send>>;

/// The grid services `GetUser` asks. `None` is a miss or a failed call.
pub trait NameSource: Send + Sync + 'static {
    /// `UserAccountService.GetUserAccount`: the account's first and last name.
    fn account(&self, id: Uuid) -> NameFuture;
    /// `GridUserService.GetGridUserInfo`, its universal identifier parsed and
    /// checked against this grid as `GetUser` does. `None` leaves it unknown.
    fn grid_user(&self, id: Uuid) -> NameFuture;
}

/// `UserData`, the fields name replies read.
#[derive(Clone, Debug)]
struct Entry {
    name: UserName,
    tried: bool,
    unknown: bool,
    local: bool,
    expires: Instant,
}

impl Entry {
    /// `IsResidentAwaitingAccount`.
    fn awaiting_account(&self) -> bool {
        self.local && !self.unknown && !self.tried
    }
}

struct Shared {
    source: Arc<dyn NameSource>,
    cache: Mutex<HashMap<Uuid, Entry>>,
}

impl Shared {
    fn get(&self, id: Uuid) -> Option<Entry> {
        let now = Instant::now();
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match cache.get(&id) {
            Some(entry) if entry.expires > now => Some(entry.clone()),
            Some(_) => {
                cache.remove(&id);
                None
            }
            None => None,
        }
    }

    fn put(&self, id: Uuid, mut entry: Entry, lasts: Duration) {
        let now = Instant::now();
        entry.expires = now + lasts;
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !cache.contains_key(&id) && cache.len() >= MAX_NAME_CACHE {
            cache.retain(|_, kept| kept.expires > now);
            if cache.len() >= MAX_NAME_CACHE
                && let Some(victim) = cache
                    .iter()
                    .min_by_key(|(key, kept)| (kept.expires, **key))
                    .map(|(key, _)| *key)
            {
                cache.remove(&victim);
                let configured = MAX_NAME_CACHE as u64;
                drop_bound(
                    REGION_NAME_CACHE,
                    Kind::Hard,
                    configured,
                    configured.saturating_add(1),
                    "names",
                );
            }
        }
        cache.insert(id, entry);
    }

    /// `GetUser`: the names to answer, or `None` for an unknown id.
    async fn get_user(&self, id: Uuid) -> Option<UserName> {
        let mut entry = match self.get(id) {
            Some(entry) if entry.tried => return (!entry.unknown).then_some(entry.name),
            Some(entry) => entry,
            None => Entry {
                name: UserName::new("Unknown", id.to_string()),
                tried: false,
                unknown: true,
                local: false,
                expires: Instant::now(),
            },
        };
        match self.source.account(id).await {
            Some(name) => {
                entry.name = name.clamped();
                entry.unknown = false;
                entry.local = true;
                entry.tried = true;
            }
            // Left untried so a later request reads the account again.
            None if entry.awaiting_account() => return Some(entry.name),
            None => {}
        }
        if !entry.tried {
            if let Some(name) = self.source.grid_user(id).await {
                entry.name = name.clamped();
                entry.unknown = false;
            }
            entry.tried = true;
        }
        self.put(id, entry.clone(), LOOKED_UP_FOR);
        (!entry.unknown).then_some(entry.name)
    }
}

struct Job {
    id: Uuid,
    circuit: CircuitNames,
}

/// The region's name cache and its lookup queue. Clones share both.
#[derive(Clone)]
pub struct RegionNames {
    shared: Arc<Shared>,
    jobs: mpsc::Sender<Job>,
}

impl RegionNames {
    /// Start the lookup queue on the current tokio runtime.
    pub fn start(source: Arc<dyn NameSource>) -> Self {
        let shared = Arc::new(Shared {
            source,
            cache: Mutex::new(HashMap::new()),
        });
        let (jobs, rx) = mpsc::channel(MAX_NAME_LOOKUPS);
        tokio::spawn(serve_lookups(Arc::clone(&shared), rx));
        Self { shared, jobs }
    }

    /// `Scene.CacheUserName` for a resident of this grid: the names its
    /// arrival carried, left for the first lookup to check against the
    /// account. A known entry is kept, as `AddUser` keeps it.
    pub fn add_arrival(&self, id: Uuid, first: &str, last: &str) {
        if self.shared.get(id).is_some_and(|entry| !entry.unknown) {
            return;
        }
        let entry = Entry {
            name: UserName::new(first, last).clamped(),
            tried: false,
            unknown: false,
            local: true,
            expires: Instant::now(),
        };
        self.shared.put(id, entry, ARRIVAL_FOR);
    }

    /// One circuit's handle and the receiver its replies arrive on.
    pub(crate) fn circuit(&self) -> (CircuitNames, mpsc::Receiver<(Uuid, UserName)>) {
        let (replies, rx) = mpsc::channel(crate::circuit::MAX_NAME_REPLIES);
        (
            CircuitNames {
                region: self.clone(),
                replies,
                waiting: Arc::new(AtomicUsize::new(0)),
            },
            rx,
        )
    }
}

async fn serve_lookups(shared: Arc<Shared>, mut rx: mpsc::Receiver<Job>) {
    let workers = Arc::new(Semaphore::new(LOOKUP_WORKERS));
    while let Some(job) = rx.recv().await {
        let Ok(permit) = Arc::clone(&workers).acquire_owned().await else {
            break;
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            // `deferedcli.IsActive`: a closed circuit is not looked up for.
            if !job.circuit.replies.is_closed()
                && let Some(name) = shared.get_user(job.id).await
            {
                job.circuit.answer(job.id, name);
            }
            job.circuit.waiting.fetch_sub(1, Ordering::AcqRel);
            drop(permit);
        });
    }
}

/// What one requested id became.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Requested {
    /// Answered from the cache.
    Answered(UserName),
    /// A lookup will answer it, or not, later.
    Queued,
    /// A bound refused the lookup. No reply will come.
    Refused,
}

#[cfg(test)]
impl RegionNames {
    /// A looked-up entry, as `GetUser` leaves a found id.
    pub(crate) fn cache_found(&self, id: Uuid, name: UserName) {
        let entry = Entry {
            name,
            tried: true,
            unknown: false,
            local: true,
            expires: Instant::now(),
        };
        self.shared.put(id, entry, LOOKED_UP_FOR);
    }
}

/// One circuit's side of the region names.
#[derive(Clone)]
pub(crate) struct CircuitNames {
    region: RegionNames,
    replies: mpsc::Sender<(Uuid, UserName)>,
    waiting: Arc<AtomicUsize>,
}

impl CircuitNames {
    /// `HandleUUIDNameRequest` for one id.
    pub(crate) fn request(&self, id: Uuid) -> Requested {
        if let Some(entry) = self.region.shared.get(id)
            && entry.tried
        {
            return Requested::Answered(entry.name);
        }
        let waiting = self.waiting.fetch_add(1, Ordering::AcqRel);
        if waiting >= MAX_CIRCUIT_LOOKUPS {
            self.waiting.fetch_sub(1, Ordering::AcqRel);
            drop_bound(
                REGION_NAME_CIRCUIT_LOOKUPS,
                Kind::Hard,
                MAX_CIRCUIT_LOOKUPS as u64,
                waiting as u64 + 1,
                "names",
            );
            return Requested::Refused;
        }
        let job = Job {
            id,
            circuit: self.clone(),
        };
        match self.region.jobs.try_send(job) {
            Ok(()) => Requested::Queued,
            Err(_) => {
                self.waiting.fetch_sub(1, Ordering::AcqRel);
                drop_bound(
                    REGION_NAME_LOOKUPS,
                    Kind::Hard,
                    MAX_NAME_LOOKUPS as u64,
                    MAX_NAME_LOOKUPS as u64 + 1,
                    "names",
                );
                Requested::Refused
            }
        }
    }

    fn answer(&self, id: Uuid, name: UserName) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.replies.try_send((id, name)) {
            drop_bound(
                ferroan_diag::REGION_NAME_REPLIES,
                Kind::Hard,
                crate::circuit::MAX_NAME_REPLIES as u64,
                crate::circuit::MAX_NAME_REPLIES as u64 + 1,
                "names",
            );
        }
    }
}

/// `SendNameReply`: one block, names through `Util.StringToBytes256`.
pub(crate) fn name_reply(id: Uuid, name: &UserName) -> UUIDNameReply {
    let mut reply = UUIDNameReply::default();
    reply.uuid_name_block.push(uuid_name_reply::UUIDNameBlock {
        id,
        first_name: crate::arrival::string_to_bytes(&name.first, NAME_FIELD),
        last_name: crate::arrival::string_to_bytes(&name.last, NAME_FIELD),
    });
    reply
}

#[cfg(test)]
#[path = "names_tests.rs"]
pub(crate) mod tests;
