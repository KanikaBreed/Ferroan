// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Objects loaded from the region store and kept for the life of the process.
//!
//! `Scene.LoadPrimsFromStorage` reads every group for the region id. This load
//! keeps that result, including each prim's task inventory. The frame assigns local IDs
//! and prepares their full updates when it admits these groups.

use std::sync::Arc;
use std::time::Instant;

use ferroan_scene::ObjectGroup;
use uuid::Uuid;

use crate::config::ObjectStore;

pub struct StoredObjects {
    pub groups: Arc<Vec<ObjectGroup>>,
    pub elapsed_ms: u128,
}

/// `None` is an empty region. A missing SQLite file is empty. A MySQL or
/// PostgreSQL store loads its region. A store that cannot be read is an error,
/// and startup stops.
pub async fn load_stored_objects(
    store: Option<&ObjectStore>,
    region: Uuid,
) -> Result<StoredObjects, String> {
    let started = Instant::now();
    let groups = match store {
        Some(store) => ferroan_scene::open_objects(&store.connection, region, store.kind)
            .await
            .map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    Ok(StoredObjects {
        groups: Arc::new(groups),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use ferroan_scene::ObjectStoreKind;

    use super::*;

    const REGION: &str = "11111111-1111-1111-1111-111111111111";

    #[tokio::test]
    async fn the_seeded_store_is_kept_for_its_region() {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-region-objects-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("OpenSim.db");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/regionstore/objects.db");
        std::fs::copy(fixture, &path).unwrap();
        let connection = format!("Data Source={};Version=3", path.display());
        let region = Uuid::parse_str(REGION).unwrap();
        let stored = load_stored_objects(
            Some(&ObjectStore {
                connection: connection.clone(),
                kind: ObjectStoreKind::Sqlite,
            }),
            region,
        )
        .await
        .unwrap();
        let prims: usize = stored.groups.iter().map(|group| group.parts.len()).sum();
        assert_eq!(stored.groups.len(), 4);
        assert_eq!(prims, 7);
        let other = load_stored_objects(
            Some(&ObjectStore {
                connection,
                kind: ObjectStoreKind::Sqlite,
            }),
            Uuid::nil(),
        )
        .await
        .unwrap();
        assert!(other.groups.is_empty());
        let missing = load_stored_objects(None, region).await.unwrap();
        assert!(missing.groups.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
