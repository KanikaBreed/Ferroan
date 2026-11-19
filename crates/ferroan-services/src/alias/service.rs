// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UserAliasService`.
//!
//! `GetUserAliases` returns null when the list is empty. `DeleteAlias`
//! returns the store result. The `throw` after that return in C# does not run.

use uuid::Uuid;

use super::model::Alias;
use super::store::{AliasStore, StoreError};

pub struct AliasService {
    store: AliasStore,
}

impl AliasService {
    pub fn new(store: AliasStore) -> Self {
        Self { store }
    }

    pub async fn create_alias(
        &self,
        alias_id: Uuid,
        user_id: Uuid,
        description: Option<String>,
    ) -> Result<Option<Alias>, StoreError> {
        if !self
            .store
            .store(alias_id, user_id, description.as_deref())
            .await?
        {
            return Ok(None);
        }
        Ok(Some(Alias {
            alias_id,
            user_id,
            description,
        }))
    }

    pub async fn get_user_for_alias(&self, alias_id: Uuid) -> Result<Option<Alias>, StoreError> {
        Ok(self.store.get_for_alias(alias_id).await?.map(|row| Alias {
            alias_id: row.alias_id,
            user_id: row.user_id,
            description: Some(row.description),
        }))
    }

    pub async fn get_user_aliases(&self, user_id: Uuid) -> Result<Option<Vec<Alias>>, StoreError> {
        let rows = self.store.get_for_user(user_id).await?;
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            rows.into_iter()
                .map(|row| Alias {
                    alias_id: row.alias_id,
                    user_id: row.user_id,
                    description: Some(row.description),
                })
                .collect(),
        ))
    }

    pub async fn delete_alias(&self, alias_id: Uuid) -> Result<bool, StoreError> {
        self.store.delete_alias(alias_id).await
    }
}
