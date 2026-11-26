// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Queue the `ParcelProperties` element `LLClientView.SendLandProperties` builds.
//!
//! Arrival, the initial-data gate, a parcel-changing walk and a
//! `ParcelPropertiesRequest` queue through the frame. This queues the raw
//! element for one agent.

use ferroan_client::{CapsRegistry, EventQueueError};
use ferroan_scene::parcels::{ParcelMap, ParcelPropertiesSend};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LandPropertiesError {
    #[error("no parcel {0}")]
    Missing(i32),
    #[error(transparent)]
    Queue(#[from] EventQueueError),
}

/// Encode `local_id` and append that element to the agent's event queue.
pub fn enqueue_land_properties(
    caps: &CapsRegistry,
    agent_id: Uuid,
    map: &ParcelMap,
    local_id: i32,
    send: &ParcelPropertiesSend,
) -> Result<(), LandPropertiesError> {
    let element = map
        .parcel_properties(local_id, send)
        .ok_or(LandPropertiesError::Missing(local_id))?;
    caps.enqueue_raw_for(agent_id, &element, "parcel-properties")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_scene::parcels::{ParcelPropertiesSend, PrimCounts, open_parcels_at};
    use ferroan_scene::{Heightmap, InitialTerrain};

    const REGION: Uuid = uuid::uuid!("11111111-1111-1111-1111-111111111111");
    const ESTATE: Uuid = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
    const CLAIM: i32 = 1_795_674_348;

    #[tokio::test]
    async fn the_queue_keeps_each_encoded_element() {
        let dir = std::env::temp_dir().join(format!("ferroan-land-properties-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let store = dir.join("store.db");
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/regionstore/objects.db");
        std::fs::copy(source, &store).unwrap();
        let connection = format!("Data Source={};Version=3", store.display());
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", store.display()))
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../../fixtures/regionstore/parcels.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let mut map = open_parcels_at(Some(&connection), REGION, [256, 256], ESTATE, CLAIM)
            .await
            .unwrap();
        let height: Heightmap = InitialTerrain::PinheadIsland.generate(256, 256).unwrap();
        map.update_geometry(Some(&height));
        let agent = Uuid::from_u128(9);
        let caps = CapsRegistry::new("http://127.0.0.1:9").unwrap();
        caps.register_agent(&format!("/CAPS/{agent}0000/"), agent)
            .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/parcel-properties");
        let moments = [
            ("arrival.bin", "Your Parcel", 0, 0),
            ("request-cell.bin", "West", 7, 0),
            ("request-span-west.bin", "West", 8, 1),
            ("request-span-east.bin", "East", 8, 1),
            ("walk.bin", "East", 0, 0),
        ];
        for (file, name, sequence, result) in moments {
            let local_id = map
                .parcels()
                .iter()
                .find(|parcel| parcel.name == name)
                .unwrap()
                .local_id;
            let send = send(sequence, result);
            enqueue_land_properties(&caps, agent, &map, local_id, &send).unwrap();
            let queued = caps.queued_events(agent).unwrap();
            let expected = std::fs::read(root.join(file)).unwrap();
            assert_eq!(queued.last().unwrap(), &expected, "{file}");
        }
        assert!(enqueue_land_properties(&caps, agent, &map, 99, &send(0, 0)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_4096_metre_element_fits_on_the_queue() {
        let mut map = open_parcels_at(None, REGION, [4096, 4096], ESTATE, CLAIM)
            .await
            .unwrap();
        map.update_geometry(None);
        let local_id = map.parcels()[0].local_id;
        let send = ParcelPropertiesSend {
            sequence_id: 0,
            snap_selection: false,
            request_result: 0,
            region_flags: 0,
            tax_free: false,
            object_capacity: 15_000,
            object_bonus: 1.0,
            prims: PrimCounts::default(),
            agent_environment_version: 0,
        };
        let agent = Uuid::from_u128(9);
        let caps = CapsRegistry::new("http://127.0.0.1:9").unwrap();
        caps.register_agent(&format!("/CAPS/{agent}0000/"), agent)
            .unwrap();
        for _ in 0..6 {
            enqueue_land_properties(&caps, agent, &map, local_id, &send).unwrap();
        }
        let queued = caps.queued_events(agent).unwrap();
        assert_eq!(queued.len(), 6);
        assert!(queued.iter().all(|element| element.len() == 177_823));
    }

    fn send(sequence: i32, result: i32) -> ParcelPropertiesSend {
        ParcelPropertiesSend {
            sequence_id: sequence,
            snap_selection: false,
            request_result: result,
            region_flags: 0,
            tax_free: false,
            object_capacity: 15_000,
            object_bonus: 1.0,
            prims: PrimCounts::default(),
            agent_environment_version: 0,
        }
    }
}
