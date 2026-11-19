// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite `UserAccounts` rows.
//!
//! The column list is fixed in this file. `SQLiteGenericTableHandler` fills a
//! shared name list and then publishes it (TB8, `835d6cfe77`). sqlx has no
//! equivalent list, so a query cannot observe a half-built one.
//!
//! `REPLACE` omits `active`, which makes SQLite apply the default `TRUE`.
//! `UserCountry` is not a column; a non-empty country fails the store, which
//! is what a `REPLACE` that names that column does. `GetUsersWhere` returns
//! null on SQLite, so a multi-id fetch is empty. The search `LIKE` is the
//! SQLite ASCII case-insensitive operator. Equality stays case-sensitive.

use ferroan_db::Pool;
use sqlx::Row;
use uuid::Uuid;

use super::model::{Account, GOD_ID, ZERO_ID};
use super::search::search_words;
use super::urls::{service_urls_db, service_urls_from_db};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("this account service only opens a SQLite store")]
    NotSqlite,
    #[error("account database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("account row is not readable: {0}")]
    Row(String),
    #[error("account table name is not an identifier")]
    Realm,
}

pub struct AccountStore {
    pool: Pool,
    realm: String,
}

impl AccountStore {
    pub fn new(pool: Pool, realm: &str) -> Result<Self, StoreError> {
        if pool.sqlite().is_none() {
            return Err(StoreError::NotSqlite);
        }
        if !is_identifier(realm) {
            return Err(StoreError::Realm);
        }
        Ok(Self {
            pool,
            realm: realm.to_string(),
        })
    }

    fn pool(&self) -> &sqlx::sqlite::SqlitePool {
        self.pool.sqlite().expect("checked in new")
    }

    /// Insert the grid god account when that principal is absent.
    ///
    /// The C# service does this on the first instance. Each database is
    /// checked on its own; there is no process-wide flag.
    pub async fn ensure_god(&self, created: i32) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        let existing = sqlx::query(&format!(
            "SELECT 1 FROM \"{}\" WHERE PrincipalID = ?",
            self.realm
        ))
        .bind(GOD_ID)
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_none() {
            sqlx::query(&format!(
                "REPLACE INTO \"{}\" \
                 (PrincipalID, ScopeID, FirstName, LastName, Email, Created, UserLevel, UserFlags, ServiceURLs) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                self.realm
            ))
            .bind(GOD_ID)
            .bind(ZERO_ID)
            .bind("GRID")
            .bind("SERVICES")
            .bind("")
            .bind(i64::from(created))
            .bind(240_i64)
            .bind(0_i64)
            .bind("")
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_where(&self, pairs: &[(&str, &str)]) -> Result<Vec<Account>, StoreError> {
        for (column, _) in pairs {
            if !is_known_column(column) {
                return Err(StoreError::Row(format!("unknown column {column}")));
            }
        }
        let sql = match pairs.len() {
            1 => format!(
                "SELECT {COLUMNS} FROM \"{}\" WHERE \"{}\" = ?",
                self.realm, pairs[0].0
            ),
            2 => format!(
                "SELECT {COLUMNS} FROM \"{}\" WHERE \"{}\" = ? AND \"{}\" = ?",
                self.realm, pairs[0].0, pairs[1].0
            ),
            3 => format!(
                "SELECT {COLUMNS} FROM \"{}\" WHERE \"{}\" = ? AND \"{}\" = ? AND \"{}\" = ?",
                self.realm, pairs[0].0, pairs[1].0, pairs[2].0
            ),
            _ => return Ok(Vec::new()),
        };
        let rows = match pairs.len() {
            1 => {
                sqlx::query(&sql)
                    .bind(pairs[0].1)
                    .fetch_all(self.pool())
                    .await?
            }
            2 => {
                sqlx::query(&sql)
                    .bind(pairs[0].1)
                    .bind(pairs[1].1)
                    .fetch_all(self.pool())
                    .await?
            }
            _ => {
                sqlx::query(&sql)
                    .bind(pairs[0].1)
                    .bind(pairs[1].1)
                    .bind(pairs[2].1)
                    .fetch_all(self.pool())
                    .await?
            }
        };
        rows.iter().map(account_from_row).collect()
    }

    pub async fn search(&self, scope_id: Uuid, query: &str) -> Result<Vec<Account>, StoreError> {
        let words = search_words(query.trim());
        if words.is_empty() || words.len() > 2 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {COLUMNS} FROM \"{}\" WHERE (ScopeID = ? OR ScopeID = '{ZERO_ID}') \
             AND (FirstName LIKE ? OR LastName LIKE ?)",
            self.realm
        );
        let scope = scope_id.to_string();
        let rows = if words.len() == 1 {
            let pattern = format!("{}%", words[0]);
            sqlx::query(&sql)
                .bind(scope)
                .bind(pattern.clone())
                .bind(pattern)
                .fetch_all(self.pool())
                .await?
        } else {
            sqlx::query(&sql)
                .bind(scope)
                .bind(format!("{}%", words[0]))
                .bind(format!("{}%", words[1]))
                .fetch_all(self.pool())
                .await?
        };
        rows.iter().map(account_from_row).collect()
    }

    /// SQLite `GetUsersWhere` returns null. Callers see an empty list.
    pub async fn get_by_ids(
        &self,
        _scope_id: Uuid,
        _ids: &[Uuid],
    ) -> Result<Vec<Account>, StoreError> {
        Ok(Vec::new())
    }

    /// `StoreUserAccount`. Returns false when `UserCountry` is non-empty,
    /// because that column is not in the SQLite table.
    pub async fn store_account(&self, account: &Account) -> Result<bool, StoreError> {
        if account
            .user_country
            .as_ref()
            .is_some_and(|text| !text.is_empty())
        {
            return Ok(false);
        }
        let title = account
            .user_title
            .as_ref()
            .filter(|text| !text.is_empty())
            .cloned();
        let urls = service_urls_db(&account.service_urls);
        let done = if let Some(title) = title {
            sqlx::query(&format!(
                "REPLACE INTO \"{}\" (PrincipalID, ScopeID, FirstName, LastName, Email, ServiceURLs, \
                 Created, UserLevel, UserFlags, DisplayName, NameChanged, UserTitle) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                self.realm
            ))
            .bind(account.principal_id.to_string())
            .bind(account.scope_id.to_string())
            .bind(&account.first_name)
            .bind(&account.last_name)
            .bind(&account.email)
            .bind(urls)
            .bind(i64::from(account.created))
            .bind(i64::from(account.user_level))
            .bind(i64::from(account.user_flags))
            .bind(account.display_name.clone())
            .bind(i64::from(account.name_changed))
            .bind(title)
            .execute(self.pool())
            .await?
        } else {
            sqlx::query(&format!(
                "REPLACE INTO \"{}\" (PrincipalID, ScopeID, FirstName, LastName, Email, ServiceURLs, \
                 Created, UserLevel, UserFlags, DisplayName, NameChanged) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                self.realm
            ))
            .bind(account.principal_id.to_string())
            .bind(account.scope_id.to_string())
            .bind(&account.first_name)
            .bind(&account.last_name)
            .bind(&account.email)
            .bind(urls)
            .bind(i64::from(account.created))
            .bind(i64::from(account.user_level))
            .bind(i64::from(account.user_flags))
            .bind(account.display_name.clone())
            .bind(i64::from(account.name_changed))
            .execute(self.pool())
            .await?
        };
        Ok(done.rows_affected() > 0)
    }
}

const COLUMNS: &str = "PrincipalID, ScopeID, FirstName, LastName, Email, ServiceURLs, Created, \
     UserLevel, UserFlags, UserTitle, DisplayName, NameChanged";

fn is_known_column(name: &str) -> bool {
    matches!(
        name,
        "PrincipalID" | "ScopeID" | "FirstName" | "LastName" | "Email"
    )
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    name.len() <= 64 && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn account_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Account, StoreError> {
    let principal = text(row, "PrincipalID")?;
    let scope = text(row, "ScopeID")?;
    let principal_id = Uuid::parse_str(&principal).map_err(|_| StoreError::Row(principal))?;
    let scope_id = Uuid::parse_str(&scope).unwrap_or(Uuid::nil());
    let created = required_i32(row, "Created")?;
    Ok(Account {
        first_name: text(row, "FirstName")?,
        last_name: text(row, "LastName")?,
        email: text(row, "Email")?,
        principal_id,
        scope_id,
        user_level: optional_i32(row, "UserLevel")?,
        user_flags: optional_i32(row, "UserFlags")?,
        user_title: Some(text(row, "UserTitle")?),
        user_country: Some(String::new()),
        local_to_grid: true,
        service_urls: service_urls_from_db(&text(row, "ServiceURLs")?),
        created,
        display_name: Some(text(row, "DisplayName")?),
        name_changed: optional_u32(row, "NameChanged")?,
    })
}

fn text(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<String, StoreError> {
    let value: Option<String> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    Ok(value.unwrap_or_default())
}

fn required_i32(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<i32, StoreError> {
    let value: Option<i64> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    let Some(value) = value else {
        return Err(StoreError::Row(format!("{name} is missing")));
    };
    i32::try_from(value).map_err(|_| StoreError::Row(format!("{name} is out of range")))
}

fn optional_i32(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<i32, StoreError> {
    let value: Option<i64> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    match value {
        Some(value) => Ok(i32::try_from(value).unwrap_or(0)),
        None => Ok(0),
    }
}

fn optional_u32(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<u32, StoreError> {
    let value: Option<i64> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    match value {
        Some(value) => Ok(u32::try_from(value).unwrap_or(0)),
        None => Ok(0),
    }
}
