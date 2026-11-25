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
// Admission follows Source/OpenSim.Region.CoreModules/World/Permissions/PermissionsModule.cs,
// Source/OpenSim.Region.Framework/Scenes/{Scene,Scene.Permissions}.cs,
// Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/UserAccounts/UserAccountCache.cs,
// and Source/OpenSim.Framework/ExpiringCacheOS.cs at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Who may enter a full region.
//!
//! At pin `c401bceecd`, `Scene.CheckLandPositionAccess` returns true for
//! `Permissions.IsGod` and `Permissions.IsEstateManager` before the root-agent
//! count. Secure startup requires every listed module to be available (D75).
//! If the region starts without the default handler, empty callbacks admit
//! ordinary agents. Disabling server-side object permissions grants the bypass
//! to non-nil users.
//! `DefaultPermissionsModule.IsEstateManager` is
//! `EstateSettings.IsEstateManagerOrOwner`. `IsGod` is `IsAdministrator`: a
//! nil user is never an administrator; the estate owner and managers are gods
//! only when `region_owner_is_god` or `region_manager_is_god` is set; a grid
//! god is an account whose `UserLevel` is at least 200 when `allow_grid_gods`
//! is set. Presence is absent at login, so the account service is the level
//! source (`GodController.UserLevel` is that same account level once a
//! presence exists). `force_grid_gods_only` forces `allow_grid_gods` and
//! leaves the owner and manager god flags off. `implicit_gods` does not
//! affect this check.
//!
//! C# does not cap the bypass. The extra roots past `region.root-agents`
//! share one pool of one estate owner plus `Constants.MaxEstateManagers`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::config::{RegionConfig, StaffPermissions};
use ferroan_scene::MAX_ESTATE_MANAGERS;
use tokio::time::{Duration, Instant};

/// `PermissionsModule.IsGridAdministrator` compares with this level.
pub const GOD_USER_LEVEL: i32 = 200;

/// One estate owner plus [`MAX_ESTATE_MANAGERS`]. Grid gods share the pool.
pub const ROOT_BYPASS_LIMIT: usize = MAX_ESTATE_MANAGERS + 1;

pub(crate) struct AccountLevel {
    pub level: i32,
    pub local_to_grid: bool,
}

/// Remembered account levels. A full cache still uses the level just read;
/// it only stops storing another id.
const LEVEL_CACHE: usize = ROOT_BYPASS_LIMIT + 40;

/// UserAccountCache schedules its UUID purge every five minutes while it holds accounts.
/// Advance those sweeps on access so an idle region needs no timer.
const LEVEL_SWEEP_SECS: u64 = 300;

#[derive(Default)]
struct LevelCache {
    entries: HashMap<Uuid, (i32, Instant)>,
    last_sweep: Option<Instant>,
}

impl LevelCache {
    fn purge(&mut self, now: Instant) {
        let Some(last) = self.last_sweep else { return };
        let ticks = now.duration_since(last).as_secs() / LEVEL_SWEEP_SECS;
        if ticks != 0 {
            let sweep = last + Duration::from_secs(ticks * LEVEL_SWEEP_SECS);
            self.entries.retain(|_, (_, expires)| *expires > sweep);
            self.last_sweep = (!self.entries.is_empty()).then_some(sweep);
        }
    }
}

/// Estate staff and grid-god flags used by admission.
#[derive(Clone)]
pub struct StaffGate {
    owner: Uuid,
    managers: Vec<Uuid>,
    permissions: StaffPermissions,
    allow_grid_gods: bool,
    region_owner_is_god: bool,
    region_manager_is_god: bool,
    account_uri: String,
    scope_id: Uuid,
    levels: Arc<Mutex<LevelCache>>,
}

/// The three flags after `force_grid_gods_only`, in the order
/// `allow_grid_gods`, `region_owner_is_god`, `region_manager_is_god`.
pub fn fold_god_flags(allow: bool, owner: bool, manager: bool, force: bool) -> (bool, bool, bool) {
    if force {
        (true, false, false)
    } else {
        (allow, owner, manager)
    }
}

impl StaffGate {
    pub fn from_config(config: &RegionConfig) -> Self {
        Self {
            owner: config.estate_owner,
            permissions: config.staff_permissions,
            managers: Vec::new(),
            allow_grid_gods: config.allow_grid_gods,
            region_owner_is_god: config.region_owner_is_god,
            region_manager_is_god: config.region_manager_is_god,
            account_uri: config.user_account_server_uri.clone(),
            scope_id: config.scope_id,
            levels: Arc::new(Mutex::new(LevelCache::default())),
        }
    }

    pub fn apply_store(&mut self, access: ferroan_scene::EstateAccess) {
        if let Some(owner) = access.owner {
            self.owner = owner;
        }
        self.managers = access.managers;
    }

    pub fn set_managers(&mut self, managers: Vec<Uuid>) {
        self.managers = managers
            .into_iter()
            .filter(|id| !id.is_nil())
            .take(MAX_ESTATE_MANAGERS)
            .collect();
    }

    pub fn set_allow_grid_gods(&mut self, allow: bool) {
        self.allow_grid_gods = allow;
    }

    /// Remember a local account level.
    pub fn note_level(&self, agent: Uuid, level: i32) {
        self.note_account(
            agent,
            AccountLevel {
                level,
                local_to_grid: true,
            },
        );
    }

    pub(crate) fn note_account(&self, agent: Uuid, account: AccountLevel) {
        let mut levels = self
            .levels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        levels.purge(now);
        if levels.entries.len() >= LEVEL_CACHE && !levels.entries.contains_key(&agent) {
            return;
        }
        levels.last_sweep.get_or_insert(now);
        let ttl = Duration::from_secs(if account.local_to_grid { 3600 } else { 172800 });
        levels.entries.insert(agent, (account.level, now + ttl));
    }

    pub fn account_uri(&self) -> &str {
        &self.account_uri
    }

    pub fn scope_id(&self) -> Uuid {
        self.scope_id
    }

    /// A level is needed only for someone who is not already estate staff.
    pub fn needs_account_level(&self, agent: Uuid) -> bool {
        self.permissions == StaffPermissions::Enforce
            && self.allow_grid_gods
            && !agent.is_nil()
            && !self.is_estate_staff(agent)
    }

    pub fn cached_level(&self, agent: Uuid) -> Option<i32> {
        let mut cache = self
            .levels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        cache.purge(Instant::now());
        cache.entries.get(&agent).map(|(level, _)| *level)
    }

    /// `IsGod` or `IsEstateManager`. Either one skips the root-agent count.
    pub fn bypasses(&self, agent: Uuid, level: Option<i32>) -> bool {
        if self.permissions == StaffPermissions::NoModule {
            // ScenePermissions.IsGod has no registered handler.
            return true;
        }
        if agent.is_nil() {
            return false;
        }
        self.permissions == StaffPermissions::Bypass
            || self.is_god(agent, level)
            || self.is_estate_staff(agent)
    }

    fn is_estate_staff(&self, agent: Uuid) -> bool {
        agent == self.owner || self.managers.contains(&agent)
    }

    fn is_god(&self, agent: Uuid, level: Option<i32>) -> bool {
        if self.region_owner_is_god && agent == self.owner {
            return true;
        }
        if self.region_manager_is_god && self.is_estate_staff(agent) {
            return true;
        }
        let level = level.or_else(|| self.cached_level(agent));
        self.allow_grid_gods && level.is_some_and(|value| value >= GOD_USER_LEVEL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(allow: bool, owner_god: bool, manager_god: bool, force: bool) -> StaffGate {
        let (allow, owner_god, manager_god) = fold_god_flags(allow, owner_god, manager_god, force);
        let owner = Uuid::from_u128(1);
        let mut gate = StaffGate {
            owner,
            managers: vec![Uuid::from_u128(2)],
            permissions: StaffPermissions::Enforce,
            allow_grid_gods: allow,
            region_owner_is_god: owner_god,
            region_manager_is_god: manager_god,
            account_uri: String::new(),
            scope_id: Uuid::nil(),
            levels: Arc::new(Mutex::new(LevelCache::default())),
        };
        gate.set_managers(vec![Uuid::from_u128(2), Uuid::nil()]);
        gate
    }

    #[test]
    fn nil_is_never_staff_and_flags_do_not_enlarge_the_estate() {
        let gate = sample(false, true, true, false);
        assert!(!gate.bypasses(Uuid::nil(), Some(GOD_USER_LEVEL)));
        assert!(gate.bypasses(Uuid::from_u128(1), None));
        assert!(gate.bypasses(Uuid::from_u128(2), None));
        assert!(!gate.bypasses(Uuid::from_u128(3), Some(GOD_USER_LEVEL)));
        assert!(!gate.needs_account_level(Uuid::from_u128(1)));
        assert!(!gate.needs_account_level(Uuid::from_u128(3)));
    }

    #[tokio::test(start_paused = true)]
    async fn an_expiry_sweep_makes_room_without_removing_a_younger_account() {
        let gate = sample(true, false, false, false);
        for n in 1..LEVEL_CACHE {
            gate.note_level(Uuid::from_u128(n as u128), 200);
        }
        tokio::time::advance(Duration::from_secs(100)).await;
        let younger = Uuid::from_u128(1000);
        gate.note_level(younger, 199);
        let new = Uuid::from_u128(1001);
        gate.note_level(new, 200);
        assert_eq!(gate.cached_level(new), None);

        // The first accounts reached their lifetime at the hour mark.
        tokio::time::advance(Duration::from_secs(3501)).await;
        gate.note_level(new, 200);
        assert_eq!(gate.cached_level(new), Some(200));
        assert_eq!(gate.cached_level(Uuid::from_u128(1)), None);
        assert_eq!(gate.cached_level(younger), Some(199));
        tokio::time::advance(Duration::from_secs(200)).await;
        assert_eq!(gate.cached_level(younger), Some(199));
        tokio::time::advance(Duration::from_secs(100)).await;
        assert_eq!(gate.cached_level(younger), None);
    }

    #[test]
    fn redact_secrets_cover_the_root_and_bypass_limits() {
        let roots = crate::simulation::ROOT_AGENT_LIMIT + ROOT_BYPASS_LIMIT;
        let slots = roots * 2 * 4;
        assert_eq!(
            ferroan_log::MAX_SECRETS,
            slots,
            "log.redact-secrets does not cover {roots} roots with one overlapping login"
        );
    }

    #[test]
    fn a_grid_god_needs_the_flag_and_level_200() {
        let mut gate = sample(true, false, false, false);
        let god = Uuid::from_u128(4);
        assert!(!gate.bypasses(god, Some(199)));
        assert!(gate.bypasses(god, Some(GOD_USER_LEVEL)));
        gate.set_allow_grid_gods(false);
        assert!(!gate.bypasses(god, Some(240)));
        let forced = sample(false, true, true, true);
        assert!(forced.allow_grid_gods);
        assert!(!forced.region_owner_is_god);
        assert!(forced.bypasses(Uuid::from_u128(1), None));
        assert!(forced.bypasses(god, Some(GOD_USER_LEVEL)));
    }
}
