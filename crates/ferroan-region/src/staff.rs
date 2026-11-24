// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Who may enter a full region.
//!
//! At pin `c401bceecd`, `Scene.CheckLandPositionAccess` returns true for
//! `Permissions.IsGod` and `Permissions.IsEstateManager` before the root-agent
//! count. `DefaultPermissionsModule.IsEstateManager` is
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

use crate::config::RegionConfig;
use ferroan_scene::MAX_ESTATE_MANAGERS;

/// `PermissionsModule.IsGridAdministrator` compares with this level.
pub const GOD_USER_LEVEL: i32 = 200;

/// One estate owner plus [`MAX_ESTATE_MANAGERS`]. Grid gods share the pool.
pub const ROOT_BYPASS_LIMIT: usize = MAX_ESTATE_MANAGERS + 1;

/// Remembered account levels. A full cache still uses the level just read;
/// it only stops storing another id.
const LEVEL_CACHE: usize = ROOT_BYPASS_LIMIT + 40;

/// Estate staff and grid-god flags used by admission.
#[derive(Clone)]
pub struct StaffGate {
    owner: Uuid,
    managers: Vec<Uuid>,
    allow_grid_gods: bool,
    region_owner_is_god: bool,
    region_manager_is_god: bool,
    account_uri: String,
    scope_id: Uuid,
    levels: Arc<Mutex<HashMap<Uuid, i32>>>,
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
            managers: Vec::new(),
            allow_grid_gods: config.allow_grid_gods,
            region_owner_is_god: config.region_owner_is_god,
            region_manager_is_god: config.region_manager_is_god,
            account_uri: config.user_account_server_uri.clone(),
            scope_id: config.scope_id,
            levels: Arc::new(Mutex::new(HashMap::new())),
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

    pub fn note_level(&self, agent: Uuid, level: i32) {
        let mut levels = self
            .levels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if levels.len() >= LEVEL_CACHE && !levels.contains_key(&agent) {
            return;
        }
        levels.insert(agent, level);
    }

    pub fn account_uri(&self) -> &str {
        &self.account_uri
    }

    pub fn scope_id(&self) -> Uuid {
        self.scope_id
    }

    /// A level is needed only for someone who is not already estate staff.
    pub fn needs_account_level(&self, agent: Uuid) -> bool {
        self.allow_grid_gods && !agent.is_nil() && !self.is_estate_staff(agent)
    }

    pub fn cached_level(&self, agent: Uuid) -> Option<i32> {
        self.levels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&agent)
            .copied()
    }

    /// `IsGod` or `IsEstateManager`. Either one skips the root-agent count.
    pub fn bypasses(&self, agent: Uuid, level: Option<i32>) -> bool {
        if agent.is_nil() {
            return false;
        }
        self.is_god(agent, level) || self.is_estate_staff(agent)
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
            allow_grid_gods: allow,
            region_owner_is_god: owner_god,
            region_manager_is_god: manager_god,
            account_uri: String::new(),
            scope_id: Uuid::nil(),
            levels: Arc::new(Mutex::new(HashMap::new())),
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

    #[test]
    fn captured_full_region_bypass_matches_the_staff_gate() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/access/full-region.json"))
                .unwrap();
        assert_eq!(
            fixture["pin"].as_str().unwrap(),
            "c401bceecd1684832ae8ba86a31b5ee01a4ded7b"
        );
        assert_eq!(fixture["agent_limit"].as_u64().unwrap(), 0);
        let owner = Uuid::parse_str(fixture["owner"].as_str().unwrap()).unwrap();
        let manager = Uuid::parse_str(fixture["manager"].as_str().unwrap()).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 12);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let (allow, owner_god, manager_god) = fold_god_flags(
                case["allow_grid_gods"].as_bool().unwrap(),
                case["region_owner_is_god"].as_bool().unwrap(),
                case["region_manager_is_god"].as_bool().unwrap(),
                case["force_grid_gods_only"].as_bool().unwrap(),
            );
            let mut gate = StaffGate {
                owner,
                managers: Vec::new(),
                allow_grid_gods: allow,
                region_owner_is_god: owner_god,
                region_manager_is_god: manager_god,
                account_uri: String::new(),
                scope_id: Uuid::nil(),
                levels: Arc::new(Mutex::new(HashMap::new())),
            };
            gate.set_managers(vec![manager]);
            let agent = Uuid::parse_str(case["agent"].as_str().unwrap()).unwrap();
            let level = i32::try_from(case["user_level"].as_i64().unwrap()).unwrap();
            let success = case["success"].as_bool().unwrap();
            assert_eq!(
                gate.bypasses(agent, Some(level)),
                success,
                "{name} disagreed with CheckLandPositionAccess"
            );
            let reason = case["reason"].as_str().unwrap();
            if success {
                assert_eq!(reason, "", "{name}");
            } else {
                assert_eq!(reason, "The region is full", "{name}");
            }
        }
    }
}
