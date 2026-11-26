// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{AppearanceItem, AvatarAppearance, AvatarData, StoreError, WearableLayers};
use serde_json::{Value, json};
use uuid::Uuid;

fn observations() -> Value {
    serde_json::from_str(include_str!("../../../../fixtures/avatar/appearance.json")).unwrap()
}

fn pair(item: &AppearanceItem) -> Value {
    json!([item.item.to_string(), item.asset.to_string()])
}

fn snapshot(a: &AvatarAppearance) -> Value {
    let wearables: Vec<Vec<_>> = a
        .wearables
        .iter()
        .map(|w| w.items().iter().map(pair).collect())
        .collect();
    let attachments: Vec<_> = a.attachments.iter()
        .map(|(point,items)| json!({"point":point, "items":items.iter().map(pair).collect::<Vec<_>>() }))
        .collect();
    json!({ "serial":a.serial, "height":a.height,
        "wearables":wearables, "attachments":attachments,
        "visual":a.visual_params.iter().map(|b| format!("{b:02x}")).collect::<String>() })
}

#[test]
fn stored_rows_decode_like_the_pin8_appearance_reader() {
    let observations = observations();
    for case in observations["reads"].as_array().unwrap() {
        let fields = serde_json::from_value(case["fields"].clone()).unwrap();
        let result = AvatarData {
            avatar_type: 1,
            fields,
        }
        .to_appearance()
        .unwrap();
        let mut expected = case["appearance"].clone();
        if !expected.is_null() {
            // Pin8's JSON decimals describe Singles. Compare at the source precision.
            expected["height"] = json!(expected["height"].as_f64().unwrap() as f32);
        }
        assert_eq!(
            result.as_ref().map(snapshot).unwrap_or(Value::Null),
            expected,
            "{}",
            case["name"]
        );
    }
}

fn read_pair(v: &Value) -> AppearanceItem {
    AppearanceItem {
        item: v[0].as_str().unwrap().parse().unwrap(),
        asset: v[1].as_str().unwrap().parse().unwrap(),
    }
}

fn constructed(v: &Value) -> AvatarAppearance {
    let visual = v["visual"].as_str().unwrap();
    AvatarAppearance {
        serial: v["serial"].as_i64().unwrap() as i32,
        height: v["height"].as_f64().unwrap() as f32,
        visual_params: (0..visual.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&visual[i..i + 2], 16).unwrap())
            .collect(),
        wearables: v["wearables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|items| {
                let mut slot = WearableLayers::default();
                for item in items.as_array().unwrap() {
                    slot.add(read_pair(item));
                }
                slot
            })
            .collect(),
        attachments: v["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| {
                (
                    group["point"].as_i64().unwrap() as i32,
                    group["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(read_pair)
                        .collect(),
                )
            })
            .collect(),
    }
}

#[tokio::test]
async fn constructed_appearances_write_the_pin8_rows() {
    let copy = super::service_tests::CopiedStore::new();
    let pool = ferroan_db::open_sqlite_file(&copy.path(), &["Avatar"])
        .await
        .unwrap();
    let db = pool.sqlite().unwrap().clone();
    let service = super::AvatarService::new(super::AvatarStore::open(pool).unwrap());
    let id = Uuid::new_v4();
    assert_eq!(
        service.get_appearance(id).await.unwrap(),
        Some(AvatarAppearance::default())
    );
    for case in observations()["writes"].as_array().unwrap() {
        let a = constructed(&case["appearance"]);
        assert!(service.set_appearance(id, &a).await.unwrap());
        let saved = service.get_avatar(id).await.unwrap();
        for data in [&AvatarData::from_appearance(&a).unwrap(), &saved] {
            assert_eq!(json!(data.avatar_type), case["type"]);
            let fields: serde_json::Map<String, Value> = data
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            assert_eq!(Value::Object(fields), case["fields"], "{}", case["name"]);
        }
        assert_eq!(
            service.get_appearance(id).await.unwrap(),
            saved.to_appearance().unwrap()
        );
    }
    drop(service);
    db.close().await;
}

#[test]
fn sparse_types_and_oversized_values_hit_service_bounds() {
    let raw = AvatarData {
        avatar_type: 1,
        fields: vec![(
            "Wearable 2147483647:0".into(),
            format!("{}:{}", Uuid::nil(), Uuid::nil()),
        )],
    };
    assert!(matches!(raw.to_appearance(), Err(StoreError::Limit)));
    let a = AvatarAppearance {
        visual_params: vec![255; ferroan_http::MAX_QUERY_BYTES / 4 + 1],
        ..Default::default()
    };
    assert!(matches!(
        AvatarData::from_appearance(&a),
        Err(StoreError::Limit)
    ));
}
