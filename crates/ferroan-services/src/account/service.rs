// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UserAccountService` without its optional plugins.
//!
//! `CreateUser` trims the names, rejects a duplicate at scope zero, and stores
//! scope zero even when the caller passed another scope. Authentication,
//! inventory, appearance, grid and grid-user work runs only when those
//! services are set. This process leaves them unset, so the account row is
//! still created and the password is not stored. The password is not logged.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::model::{Account, ZERO_ID};
use super::store::{AccountStore, StoreError};

pub trait Clock: Send + Sync {
    fn unix_time(&self) -> i32;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_time(&self) -> i32 {
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return 0;
        };
        i32::try_from(elapsed.as_secs()).unwrap_or(i32::MAX)
    }
}

/// A clock tests use so `Created` is not "now".
pub struct FixedClock {
    pub unix_time: i32,
}

impl Clock for FixedClock {
    fn unix_time(&self) -> i32 {
        self.unix_time
    }
}

pub struct NewAccount {
    pub principal_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
    pub email: String,
    pub model: String,
}

pub struct AccountService {
    store: AccountStore,
    clock: Arc<dyn Clock>,
}

impl AccountService {
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.store.pool_occupancy()
    }

    pub async fn open(store: AccountStore, clock: Arc<dyn Clock>) -> Result<Self, StoreError> {
        let service = Self { store, clock };
        service.store.ensure_god(service.clock.unix_time()).await?;
        Ok(service)
    }

    pub async fn get_by_name(
        &self,
        scope_id: Uuid,
        first_name: &str,
        last_name: &str,
    ) -> Result<Option<Account>, StoreError> {
        let rows = if scope_id.is_nil() {
            self.store
                .get_where(&[("FirstName", first_name), ("LastName", last_name)])
                .await?
        } else {
            let scope = scope_id.to_string();
            let scoped = self
                .store
                .get_where(&[
                    ("ScopeID", scope.as_str()),
                    ("FirstName", first_name),
                    ("LastName", last_name),
                ])
                .await?;
            if !scoped.is_empty() {
                scoped
            } else {
                self.store
                    .get_where(&[
                        ("ScopeID", ZERO_ID),
                        ("FirstName", first_name),
                        ("LastName", last_name),
                    ])
                    .await?
            }
        };
        Ok(rows.into_iter().next())
    }

    pub async fn get_by_email(
        &self,
        scope_id: Uuid,
        email: &str,
    ) -> Result<Option<Account>, StoreError> {
        let rows = if scope_id.is_nil() {
            self.store.get_where(&[("Email", email)]).await?
        } else {
            let scope = scope_id.to_string();
            let scoped = self
                .store
                .get_where(&[("ScopeID", scope.as_str()), ("Email", email)])
                .await?;
            if !scoped.is_empty() {
                scoped
            } else {
                self.store
                    .get_where(&[("ScopeID", ZERO_ID), ("Email", email)])
                    .await?
            }
        };
        Ok(rows.into_iter().next())
    }

    pub async fn get_by_principal(
        &self,
        scope_id: Uuid,
        principal_id: Uuid,
    ) -> Result<Option<Account>, StoreError> {
        let principal = principal_id.to_string();
        let rows = if scope_id.is_nil() {
            self.store
                .get_where(&[("PrincipalID", principal.as_str())])
                .await?
        } else {
            let scope = scope_id.to_string();
            let scoped = self
                .store
                .get_where(&[
                    ("ScopeID", scope.as_str()),
                    ("PrincipalID", principal.as_str()),
                ])
                .await?;
            if !scoped.is_empty() {
                scoped
            } else {
                self.store
                    .get_where(&[("ScopeID", ZERO_ID), ("PrincipalID", principal.as_str())])
                    .await?
            }
        };
        Ok(rows.into_iter().next())
    }

    pub async fn search(&self, scope_id: Uuid, query: &str) -> Result<Vec<Account>, StoreError> {
        self.store.search(scope_id, query).await
    }

    /// SQLite returns no rows from `GetUsersWhere`.
    pub async fn get_many(&self, scope_id: Uuid, ids: &[Uuid]) -> Result<Vec<Account>, StoreError> {
        self.store.get_by_ids(scope_id, ids).await
    }

    pub async fn store_user(&self, account: &Account) -> Result<bool, StoreError> {
        self.store.store_account(account).await
    }

    pub async fn set_display_name(
        &self,
        principal_id: Uuid,
        display_name: &str,
    ) -> Result<bool, StoreError> {
        let Some(mut account) = self.get_by_principal(Uuid::nil(), principal_id).await? else {
            return Ok(false);
        };
        account.display_name = Some(display_name.to_string());
        account.name_changed = self.name_changed_now();
        self.store_user(&account).await
    }

    /// Returns the in-memory account, not a row read back after the insert.
    ///
    /// `scope_id` is accepted and ignored. The stored scope is zero.
    pub async fn create_user(
        &self,
        scope_id: Uuid,
        new_account: NewAccount,
    ) -> Result<Option<Account>, StoreError> {
        let _ = (scope_id, new_account.password, new_account.model);
        let first_name = new_account.first_name.trim().to_string();
        let last_name = new_account.last_name.trim().to_string();
        if self
            .get_by_name(Uuid::nil(), &first_name, &last_name)
            .await?
            .is_some()
        {
            return Ok(None);
        }
        let account = Account {
            first_name,
            last_name,
            email: new_account.email,
            principal_id: new_account.principal_id,
            scope_id: Uuid::nil(),
            user_level: 0,
            user_flags: 0,
            user_title: None,
            user_country: None,
            local_to_grid: true,
            service_urls: vec![
                ("HomeURI".to_string(), String::new()),
                ("InventoryServerURI".to_string(), String::new()),
                ("AssetServerURI".to_string(), String::new()),
            ],
            created: self.clock.unix_time(),
            display_name: None,
            name_changed: 0,
        };
        if !self.store_user(&account).await? {
            return Ok(None);
        }
        Ok(Some(account))
    }

    fn name_changed_now(&self) -> u32 {
        let value = self.clock.unix_time();
        if value < 0 { 0 } else { value as u32 }
    }
}
