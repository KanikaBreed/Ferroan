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
        match name {
            "parcel-empty" => Some(Self::Empty),
            "parcel-multi" | "parcel-timing" | "parcel-properties" => Some(Self::Multi),
            "parcel-lone" => Some(Self::Lone),
            "parcel-legacy" => Some(Self::Legacy),
            _ => None,
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
