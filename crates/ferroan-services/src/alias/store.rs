// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite `UserAlias` rows.
//!
//! The DDL follows `Source/OpenSim.Data.MySQL/Resources/UserAlias.migrations`
//! (version 1): `Id`, `AliasID`, `UserID`, `Description`. SQLite has no
//! `AUTO_INCREMENT`, so `Id` is an integer primary key. There is no
//! `UserAlias` row in `migrations`, and this store does not add one.

use ferroan_db::Pool;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("this alias service only opens a SQLite store")]
    NotSqlite,
    #[error("alias database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("alias row is not readable: {0}")]
    Row(String),
    #[error("alias table name is not an identifier")]
    Realm,
    #[error(
        "UserAlias has no SQLite migration at pin 40432e0; create the table before starting this service"
    )]
    MissingTable,
}

/// Table body shared by tests and the harness. `name` must already be an identifier.
pub fn sqlite_alias_ddl(name: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS \"{name}\" (\
         Id INTEGER PRIMARY KEY AUTOINCREMENT, \
         AliasID TEXT NOT NULL UNIQUE, \
         UserID TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000', \
         Description TEXT)"
    )
}

pub const SQLITE_ALIAS_DDL: &str = "CREATE TABLE IF NOT EXISTS \"UserAlias\" (\
     Id INTEGER PRIMARY KEY AUTOINCREMENT, \
     AliasID TEXT NOT NULL UNIQUE, \
     UserID TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000', \
     Description TEXT)";

pub struct AliasStore {
    pool: Pool,
    realm: String,
}

impl AliasStore {
    pub async fn new(pool: Pool, realm: &str) -> Result<Self, StoreError> {
        if pool.sqlite().is_none() {
            return Err(StoreError::NotSqlite);
        }
        if !is_identifier(realm) {
            return Err(StoreError::Realm);
        }
        let store = Self {
            pool,
            realm: realm.to_string(),
        };
        store.require_table().await?;
        Ok(store)
    }

    pub async fn create_table(pool: &Pool, realm: &str) -> Result<(), StoreError> {
        if !is_identifier(realm) {
            return Err(StoreError::Realm);
        }
        let Some(sqlite) = pool.sqlite() else {
            return Err(StoreError::NotSqlite);
        };
        sqlx::query(&sqlite_alias_ddl(realm))
            .execute(sqlite)
            .await?;
        Ok(())
    }

    fn pool(&self) -> &sqlx::sqlite::SqlitePool {
        self.pool.sqlite().expect("checked in new")
    }

    async fn require_table(&self) -> Result<(), StoreError> {
        let row = sqlx::query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(&self.realm)
            .fetch_optional(self.pool())
            .await?;
        if row.is_none() {
            Err(StoreError::MissingTable)
        } else {
            Ok(())
        }
    }

    pub async fn store(
        &self,
        alias_id: Uuid,
        user_id: Uuid,
        description: Option<&str>,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(&format!(
            "INSERT INTO \"{}\" (AliasID, UserID, Description) VALUES (?, ?, ?) \
             ON CONFLICT(AliasID) DO UPDATE SET UserID = excluded.UserID, Description = excluded.Description",
            self.realm
        ))
        .bind(alias_id.to_string())
        .bind(user_id.to_string())
        .bind(description)
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn get_for_alias(&self, alias_id: Uuid) -> Result<Option<AliasRow>, StoreError> {
        let row = sqlx::query(&format!(
            "SELECT AliasID, UserID, Description FROM \"{}\" WHERE AliasID = ?",
            self.realm
        ))
        .bind(alias_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.as_ref().map(alias_from_row).transpose()
    }

    pub async fn get_for_user(&self, user_id: Uuid) -> Result<Vec<AliasRow>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT AliasID, UserID, Description FROM \"{}\" WHERE UserID = ? ORDER BY Id",
            self.realm
        ))
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(alias_from_row).collect()
    }

    pub async fn delete_alias(&self, alias_id: Uuid) -> Result<bool, StoreError> {
        let done = sqlx::query(&format!("DELETE FROM \"{}\" WHERE AliasID = ?", self.realm))
            .bind(alias_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

pub struct AliasRow {
    pub alias_id: Uuid,
    pub user_id: Uuid,
    pub description: String,
}

fn alias_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AliasRow, StoreError> {
    let alias = text(row, "AliasID")?;
    let user = text(row, "UserID")?;
    Ok(AliasRow {
        alias_id: Uuid::parse_str(&alias).map_err(|_| StoreError::Row(alias))?,
        user_id: Uuid::parse_str(&user).unwrap_or(Uuid::nil()),
        description: text(row, "Description")?,
    })
}

fn text(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<String, StoreError> {
    let value: Option<String> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    Ok(value.unwrap_or_default())
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    name.len() <= 64 && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
