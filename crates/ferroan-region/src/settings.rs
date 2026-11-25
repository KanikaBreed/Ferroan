// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Apply stored region settings to the handshake and keep the spawn points.
//!
//! SQLite is the store this load reads. MySQL and PostgreSQL stay on the fresh
//! handshake defaults. A missing file or a missing row does the same, and does
//! not insert a row. Spawn points are kept for the life of the value the
//! caller holds. They are not part of the handshake.

use std::sync::Arc;

use ferroan_client::{HandshakeConfig, StoredHandshake};
use ferroan_scene::{RegionSettings, SettingsSource};
use uuid::Uuid;

use crate::config::ObjectStore;

/// Load settings for `region` and overlay them on `handshake`.
///
/// `Ok(None)` keeps the handshake as the caller built it. `Ok(Some)` is the
/// loaded row, including its spawn points. A store that cannot be read is an error.
pub async fn apply_stored_settings(
    store: Option<&ObjectStore>,
    region: Uuid,
    handshake: &mut HandshakeConfig,
) -> Result<Option<Arc<RegionSettings>>, String> {
    let Some(store) = store else {
        tracing::info!("ferroan-region: no region store; handshake keeps fresh defaults");
        return Ok(None);
    };
    match ferroan_scene::open_region_settings(&store.connection, region, store.kind)
        .await
        .map_err(|error| error.to_string())?
    {
        SettingsSource::OtherDatabase => {
            tracing::info!(
                "ferroan-region: region settings stay at the handshake defaults; MySQL and PostgreSQL settings are not read yet"
            );
            Ok(None)
        }
        SettingsSource::Sqlite(None) => {
            tracing::info!(
                "ferroan-region: no stored region settings; handshake keeps fresh defaults"
            );
            Ok(None)
        }
        SettingsSource::Sqlite(Some(settings)) => {
            handshake.apply_stored_settings(&handshake_of(&settings));
            tracing::info!(
                "ferroan-region: loaded region settings, {} spawn points",
                settings.spawn_points.len()
            );
            Ok(Some(Arc::from(settings)))
        }
    }
}

fn handshake_of(settings: &RegionSettings) -> StoredHandshake {
    StoredHandshake {
        water_height: narrow(settings.water_height),
        cache_id: settings.cache_id,
        terrain_textures: settings.terrain_textures,
        terrain_pbr: settings.terrain_pbr,
        terrain_start_heights: settings.terrain_start_heights.map(narrow),
        terrain_height_ranges: settings.terrain_height_ranges.map(narrow),
        allow_damage: settings.allow_damage,
        fixed_sun: settings.fixed_sun,
        block_terraform: settings.block_terraform,
        allow_land_resell: settings.allow_land_resell,
        sandbox: settings.sandbox,
        casino: settings.casino,
        god_block_search: settings.god_block_search,
        disable_collisions: settings.disable_collisions,
        disable_scripts: settings.disable_scripts,
        disable_physics: settings.disable_physics,
        block_fly: settings.block_fly,
        restrict_pushing: settings.restrict_pushing,
        allow_land_join_divide: settings.allow_land_join_divide,
        block_show_in_search: settings.block_show_in_search,
    }
}

fn narrow(value: f64) -> f32 {
    let narrow = value as f32;
    if narrow.is_finite() { narrow } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use ferroan_scene::ObjectStoreKind;

    use super::*;
    use crate::config::ObjectStore;

    const REGION: &str = "11111111-1111-1111-1111-111111111111";

    fn dump() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/regionstore/settings.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn copied_store() -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-region-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.db");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/regionstore/settings.db"),
            &path,
        )
        .unwrap();
        (dir, format!("Data Source={};Version=3", path.display()))
    }

    #[tokio::test]
    async fn the_handshake_matches_the_csharp_region_except_fixed_sun() {
        let dump = dump();
        let (dir, connection) = copied_store();
        let region = Uuid::parse_str(REGION).unwrap();
        let mut handshake = HandshakeConfig::for_region(region);
        handshake.sim_name = "Named".to_owned();
        handshake.estate_owner = Uuid::from_u128(0x0e57);
        let owner = handshake.estate_owner;
        let access = handshake.sim_access;
        let billable = handshake.billable_factor;
        let held = apply_stored_settings(
            Some(&ObjectStore {
                connection: connection.clone(),
                kind: ObjectStoreKind::Sqlite,
            }),
            region,
            &mut handshake,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(held.spawn_points.len(), 2);
        assert_eq!(handshake.sim_name, "Named");
        assert_eq!(handshake.estate_owner, owner);
        assert_eq!(handshake.sim_access, access);
        assert_eq!(handshake.billable_factor, billable);
        assert_eq!(
            handshake.water_height,
            dump["water_height"].as_f64().unwrap() as f32
        );
        assert_eq!(
            handshake.cache_id.as_hyphenated().to_string(),
            dump["cache_id"].as_str().unwrap()
        );
        let textures = dump["terrain_textures"].as_array().unwrap();
        let pbr = dump["terrain_pbr"].as_array().unwrap();
        let starts = dump["terrain_start_heights"].as_array().unwrap();
        let ranges = dump["terrain_height_ranges"].as_array().unwrap();
        for index in 0..4 {
            assert_eq!(
                handshake.terrain_textures[index]
                    .as_hyphenated()
                    .to_string(),
                textures[index].as_str().unwrap()
            );
            assert_eq!(
                handshake.terrain_pbr[index].as_hyphenated().to_string(),
                pbr[index].as_str().unwrap()
            );
            assert_eq!(
                handshake.terrain_start_heights[index],
                starts[index].as_f64().unwrap() as f32
            );
            assert_eq!(
                handshake.terrain_height_ranges[index],
                ranges[index].as_f64().unwrap() as f32
            );
        }
        let csharp = u32::try_from(dump["csharp_region_flags"].as_u64().unwrap()).unwrap();
        let mut without_fixed_sun = HandshakeConfig::for_region(region);
        let mut stored = handshake_of(&held);
        stored.fixed_sun = false;
        without_fixed_sun.apply_stored_settings(&stored);
        assert_eq!(without_fixed_sun.region_flags, csharp);
        assert_eq!(csharp & (1 << 4), 0);
        assert_eq!(handshake.region_flags, csharp | (1 << 4));
        let classic = handshake.build(false, false);
        let pbr_packet = handshake.build(false, true);
        assert_eq!(classic.region_info.region_flags, handshake.region_flags);
        assert_eq!(classic.region_info.water_height, handshake.water_height);
        assert_eq!(
            classic.region_info.terrain_detail0,
            handshake.terrain_textures[0]
        );
        assert_eq!(classic.region_info.terrain_base0, Uuid::nil());
        assert!(pbr_packet.region_info.terrain_detail0.is_nil());
        assert_eq!(
            pbr_packet.region_info.terrain_detail1,
            handshake.terrain_pbr[1]
        );
        assert_eq!(
            classic.region_info.terrain_start_height00,
            handshake.terrain_start_heights[0]
        );
        assert_eq!(
            classic.region_info.terrain_height_range11,
            handshake.terrain_height_ranges[3]
        );
        let mut untouched = HandshakeConfig::for_region(region);
        let before = untouched.region_flags;
        let cache = untouched.cache_id;
        let missing = apply_stored_settings(
            Some(&ObjectStore {
                connection,
                kind: ObjectStoreKind::Sqlite,
            }),
            Uuid::nil(),
            &mut untouched,
        )
        .await
        .unwrap();
        assert!(missing.is_none());
        assert_eq!(untouched.region_flags, before);
        assert_eq!(untouched.cache_id, cache);
        assert_eq!(untouched.water_height, 20.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
