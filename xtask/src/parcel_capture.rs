// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Seed {
    Empty,
    Multi,
    Lone,
    Legacy,
}

impl Seed {
    pub(super) fn scenario(name: &str) -> Option<Self> {
        match ferroan_bot::register::get(name).and_then(|row| row.seed) {
            Some(ferroan_bot::register::Seed::Empty) => Some(Self::Empty),
            Some(ferroan_bot::register::Seed::Multi) => Some(Self::Multi),
            Some(ferroan_bot::register::Seed::Lone) => Some(Self::Lone),
            Some(ferroan_bot::register::Seed::Legacy) => Some(Self::Legacy),
            None => None,
        }
    }
    pub(super) fn packets(self) -> usize {
        if self == Self::Legacy { 16 } else { 4 }
    }

    pub(super) fn install(self, cwd: &Path) -> Result<(), String> {
        let fixtures = crate::repo_root().join("fixtures");
        let capture = fixtures.join("parcel-overlay");
        let land = cwd.join("OpenSim.db");
        fs::copy(fixtures.join("regionstore/objects.db"), &land).map_err(|e| e.to_string())?;
        let groups = cwd.join("groups.db");
        fs::copy(fixtures.join("db/sqlite/os_groups_Store.db"), &groups)
            .map_err(|e| e.to_string())?;
        let mut scripts = Vec::new();
        if self != Self::Empty {
            scripts.push(fixtures.join("regionstore/parcels.sql"));
            match self {
                Self::Lone => scripts.push(capture.join("lone.sql")),
                Self::Legacy => scripts.push(capture.join("legacy.sql")),
                _ => {}
            }
            scripts.push(capture.join("home-region.sql"));
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(async {
                apply(&land, &scripts).await?;
                apply(&groups, &[capture.join("membership.sql")]).await
            })
    }
}

/// The groups store Robust serves for a remote connector. The same membership
/// script the region install applies, on a copy of the unredacted groups file.
pub(super) fn seed_memberships(dest: &Path) -> Result<(), String> {
    let fixtures = crate::repo_root().join("fixtures");
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(fixtures.join("db/sqlite/os_groups_Store.db"), dest).map_err(|e| e.to_string())?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(apply(
            dest,
            &[fixtures.join("parcel-overlay/membership.sql")],
        ))
}

async fn apply(db: &Path, scripts: &[std::path::PathBuf]) -> Result<(), String> {
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
    let mut conn = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(db))
        .await
        .map_err(|e| e.to_string())?;
    for path in scripts {
        let sql = fs::read_to_string(path).map_err(|e| e.to_string())?;
        sqlx::raw_sql(&sql)
            .execute(&mut conn)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    conn.close().await.map_err(|e| e.to_string())
}
