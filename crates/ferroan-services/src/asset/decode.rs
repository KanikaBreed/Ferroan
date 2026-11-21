// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Column readers shared by the three asset stores.
//!
//! sqlx reports the same logical column as a different Rust type on each
//! driver. The C# readers use `Convert.ToInt32`, `Convert.ToBoolean` and
//! `ToString`, so a few representations are accepted here too.

use sqlx::Row;
use uuid::Uuid;

use super::store::StoreError;

pub(super) trait ReadColumn {
    fn i64_col(&self, name: &str) -> Result<i64, StoreError>;
    fn bool_col(&self, name: &str) -> Result<bool, StoreError>;
    fn mysql_local_col(&self, name: &str) -> Result<bool, StoreError>;
    fn string_col(&self, name: &str) -> Result<String, StoreError>;
    fn bytes_col(&self, name: &str) -> Result<Vec<u8>, StoreError>;
    fn uuid_col(&self, name: &str) -> Result<Uuid, StoreError>;
    fn optional_i32_col(&self, name: &str) -> Result<Option<i32>, StoreError>;
}

macro_rules! try_unsigned {
    (unsigned, $self:ident, $name:ident) => {
        if let Ok(value) = $self.try_get::<u8, _>($name) {
            return Ok(i64::from(value));
        }
        if let Ok(value) = $self.try_get::<u32, _>($name) {
            return i64::try_from(value).map_err(|_| StoreError::Row($name.to_string()));
        }
    };
    (signed, $self:ident, $name:ident) => {};
}

macro_rules! impl_read {
    ($row:ty, $kind:tt) => {
        impl ReadColumn for $row {
            fn i64_col(&self, name: &str) -> Result<i64, StoreError> {
                if let Ok(value) = self.try_get::<i64, _>(name) {
                    return Ok(value);
                }
                if let Ok(value) = self.try_get::<i32, _>(name) {
                    return Ok(i64::from(value));
                }
                if let Ok(value) = self.try_get::<i16, _>(name) {
                    return Ok(i64::from(value));
                }
                if let Ok(value) = self.try_get::<i8, _>(name) {
                    return Ok(i64::from(value));
                }
                try_unsigned!($kind, self, name);
                Err(StoreError::Row(format!("{name} is not an integer")))
            }

            fn bool_col(&self, name: &str) -> Result<bool, StoreError> {
                if let Ok(value) = self.try_get::<bool, _>(name) {
                    return Ok(value);
                }
                Ok(self.i64_col(name)? != 0)
            }

            fn mysql_local_col(&self, name: &str) -> Result<bool, StoreError> {
                if let Ok(value) = self.try_get::<bool, _>(name) {
                    return Ok(value);
                }
                if let Ok(value) = self.i64_col(name) {
                    return Ok(value == 1);
                }
                if let Ok(value) = self.try_get::<String, _>(name) {
                    let text = value.to_ascii_lowercase();
                    return Ok(text == "1" || text == "true");
                }
                Err(StoreError::Row(format!("{name} is not a local flag")))
            }

            fn string_col(&self, name: &str) -> Result<String, StoreError> {
                self.try_get(name)
                    .map_err(|_| StoreError::Row(format!("{name} is not text")))
            }

            fn bytes_col(&self, name: &str) -> Result<Vec<u8>, StoreError> {
                self.try_get(name)
                    .map_err(|_| StoreError::Row(format!("{name} is not bytes")))
            }

            fn uuid_col(&self, name: &str) -> Result<Uuid, StoreError> {
                if let Ok(value) = self.try_get::<Uuid, _>(name) {
                    return Ok(value);
                }
                if let Ok(value) = self.try_get::<String, _>(name) {
                    return Uuid::parse_str(value.trim()).map_err(|_| StoreError::Row(value));
                }
                Err(StoreError::Row(format!("{name} is not a UUID")))
            }

            fn optional_i32_col(&self, name: &str) -> Result<Option<i32>, StoreError> {
                if let Ok(value) = self.try_get::<Option<i32>, _>(name) {
                    return Ok(value);
                }
                if let Ok(value) = self.try_get::<Option<i64>, _>(name) {
                    return match value {
                        None => Ok(None),
                        Some(value) => i32::try_from(value)
                            .map(Some)
                            .map_err(|_| StoreError::Row(format!("{name} {value}"))),
                    };
                }
                Ok(Some(i32_from(self.i64_col(name)?, name)?))
            }
        }
    };
}

impl_read!(sqlx::sqlite::SqliteRow, unsigned);
impl_read!(sqlx::mysql::MySqlRow, unsigned);
impl_read!(sqlx::postgres::PgRow, signed);

pub(super) fn column_i32(row: &impl ReadColumn, name: &str) -> Result<i32, StoreError> {
    i32_from(row.i64_col(name)?, name)
}

pub(super) fn column_i8(row: &impl ReadColumn, name: &str) -> Result<i8, StoreError> {
    let value = row.i64_col(name)?;
    i8::try_from(value).map_err(|_| StoreError::Row(format!("{name} {value}")))
}

pub(super) fn column_bool(row: &impl ReadColumn, name: &str) -> Result<bool, StoreError> {
    row.bool_col(name)
}

pub(super) fn mysql_local(row: &impl ReadColumn, name: &str) -> Result<bool, StoreError> {
    row.mysql_local_col(name)
}

pub(super) fn column_string(row: &impl ReadColumn, name: &str) -> Result<String, StoreError> {
    row.string_col(name)
}

pub(super) fn column_bytes(row: &impl ReadColumn, name: &str) -> Result<Vec<u8>, StoreError> {
    row.bytes_col(name)
}

pub(super) fn column_uuid(row: &impl ReadColumn, name: &str) -> Result<Uuid, StoreError> {
    row.uuid_col(name)
}

pub(super) fn optional_i32(row: &impl ReadColumn, name: &str) -> Result<Option<i32>, StoreError> {
    row.optional_i32_col(name)
}

fn i32_from(value: i64, name: &str) -> Result<i32, StoreError> {
    i32::try_from(value).map_err(|_| StoreError::Row(format!("{name} {value}")))
}

pub(super) fn is_unique(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|db| db.is_unique_violation())
}
