// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use serde_json::{Value, json};

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn id(value: &Value) -> Uuid {
    Uuid::parse_str(text(value)).unwrap()
}

fn snapshot(row: &PresenceData) -> Value {
    json!({
        "user": row.user_id,
        "session": row.session_id.to_string(),
        "region": row.region_id().to_string(),
        "data": row.data,
    })
}

#[tokio::test]
async fn two_null_providers_match_the_pin8_sequence() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/presence/null.json")).unwrap();
    let stores = [NullPresenceData::default(), NullPresenceData::default()];
    let mut held = None;
    for (step, case) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let store: &dyn PresenceStore = &stores[case["instance"].as_u64().unwrap() as usize];
        let input = &case["input"];
        let op = text(&case["op"]);
        let actual = match op {
            "store" => {
                let row = PresenceData::new(
                    text(&input["user"]).to_owned(),
                    id(&input["session"]),
                    id(&input["region"]),
                    serde_json::from_value(input["data"].clone()).unwrap(),
                );
                json!(store.store(Arc::new(row)).await.unwrap())
            }
            "get" => {
                held = store.get(id(input)).await.unwrap();
                held.as_deref().map_or(Value::Null, snapshot)
            }
            "held" => held.as_deref().map_or(Value::Null, snapshot),
            "restore" => {
                json!(
                    store
                        .store(Arc::clone(held.as_ref().unwrap()))
                        .await
                        .unwrap()
                )
            }
            "find" => {
                let rows = store
                    .get_by_field(text(&input["field"]), text(&input["value"]))
                    .await
                    .unwrap();
                Value::Array(rows.iter().map(|row| snapshot(row)).collect())
            }
            "delete" => json!(
                store
                    .delete(text(&input["field"]), text(&input["value"]))
                    .await
                    .unwrap()
            ),
            "report" => json!(
                store
                    .report_agent(id(&input["session"]), id(&input["region"]))
                    .await
                    .unwrap()
            ),
            "logout" => {
                store.logout_region_agents(id(input)).await.unwrap();
                Value::Null
            }
            "verify" => json!(
                store
                    .verify_agent(id(&input["user"]), id(&input["secure"]))
                    .await
                    .unwrap()
            ),
            _ => panic!("unknown fixture operation {op}"),
        };
        assert_eq!(actual, case["result"], "step {step}: {op}");
    }
}

fn isolated() -> NullPresenceData {
    NullPresenceData {
        sessions: Arc::default(),
    }
}

fn limit(error: PresenceStoreError) -> PresenceLimit {
    match error {
        PresenceStoreError::Limit(limit) => limit,
        other => panic!("expected a Presence limit, got {other}"),
    }
}

fn row(i: usize) -> PresenceData {
    PresenceData::new(
        "user".into(),
        Uuid::from_u128(i as u128),
        Uuid::nil(),
        BTreeMap::new(),
    )
}

#[tokio::test]
async fn full_store_still_replaces_sessions_and_reuses_deleted_slots() {
    let store = isolated();
    for i in 0..MAX_SESSIONS {
        assert!(store.store(Arc::new(row(i))).await.unwrap());
    }
    let error = limit(store.store(Arc::new(row(MAX_SESSIONS))).await.unwrap_err());
    assert_eq!(error.id, PRESENCE_NULL_SESSIONS);
    assert!(
        store
            .get(Uuid::from_u128(MAX_SESSIONS as u128))
            .await
            .unwrap()
            .is_none()
    );

    let old = store.get(Uuid::nil()).await.unwrap().unwrap();
    let replacement = Arc::new(row(0));
    assert!(store.store(Arc::clone(&replacement)).await.unwrap());
    assert!(Arc::ptr_eq(
        &store.get(Uuid::nil()).await.unwrap().unwrap(),
        &replacement
    ));
    let region = Uuid::from_u128(1);
    assert!(store.report_agent(Uuid::nil(), region).await.unwrap());
    assert_eq!(replacement.region_id(), region);
    assert_eq!(old.region_id(), Uuid::nil());
    assert!(
        store
            .delete("SessionID", &Uuid::nil().to_string())
            .await
            .unwrap()
    );
    assert!(store.store(Arc::new(row(MAX_SESSIONS))).await.unwrap());
    assert_eq!(lock(&store.sessions).slots.len(), MAX_SESSIONS);
}

#[tokio::test]
async fn oversized_results_are_refused_and_bulk_removal_still_visits_every_match() {
    let store = isolated();
    for i in 0..MAX_RESULTS {
        assert!(store.store(Arc::new(row(i))).await.unwrap());
    }
    assert_eq!(
        store.get_by_field("UserID", "user").await.unwrap().len(),
        MAX_RESULTS
    );
    assert!(store.store(Arc::new(row(MAX_RESULTS))).await.unwrap());
    let err = limit(store.get_by_field("UserID", "user").await.err().unwrap());
    assert_eq!((err.id, err.observed), (PRESENCE_RESULTS, MAX_RESULTS + 1));
    assert!(store.delete("UserID", "user").await.unwrap());
    assert!(
        store
            .get_by_field("UserID", "user")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn oversized_row_or_query_preserves_existing_sessions() {
    let store = isolated();
    let mut record = row(0);
    for i in 0..MAX_FIELDS {
        record.data.insert(i.to_string(), String::new());
    }
    assert!(store.store(Arc::new(record)).await.unwrap());
    let mut record = row(0);
    for i in 0..=MAX_FIELDS {
        record.data.insert(i.to_string(), String::new());
    }
    assert_eq!(
        limit(store.store(Arc::new(record)).await.unwrap_err()).id,
        PRESENCE_FIELDS
    );
    assert_eq!(
        store.get(Uuid::nil()).await.unwrap().unwrap().data.len(),
        MAX_FIELDS
    );

    let mut record = row(0);
    record.data.insert(
        "key".into(),
        "a".repeat(MAX_ROW_BYTES - record.user_id.len() - 3),
    );
    assert!(store.store(Arc::new(record)).await.unwrap());
    let mut record = row(0);
    record.data.insert(
        "key".into(),
        "a".repeat(MAX_ROW_BYTES - record.user_id.len() - 2),
    );
    assert_eq!(
        limit(store.store(Arc::new(record)).await.unwrap_err()).id,
        PRESENCE_ROW_BYTES
    );
    let oversized = "a".repeat(MAX_ROW_BYTES + 1);
    assert_eq!(
        limit(store.delete("key", &oversized).await.unwrap_err()).id,
        PRESENCE_ROW_BYTES
    );
    assert_eq!(
        limit(store.get_by_field("key", &oversized).await.err().unwrap()).id,
        PRESENCE_ROW_BYTES
    );
    assert_eq!(
        store.get(Uuid::nil()).await.unwrap().unwrap().data["key"].len(),
        MAX_ROW_BYTES - 7
    );
}

#[tokio::test]
async fn a_user_id_at_the_row_limit_can_be_found_and_deleted() {
    let store = isolated();
    let mut record = row(0);
    record.user_id = "u".repeat(MAX_ROW_BYTES);
    let handle = Arc::new(record);
    assert!(store.store(Arc::clone(&handle)).await.unwrap());
    assert_eq!(
        store
            .get_by_field("UserID", &handle.user_id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(store.delete("UserID", &handle.user_id).await.unwrap());
}

#[tokio::test]
async fn spare_string_capacity_counts_towards_the_resident_row_budget() {
    let store = isolated();
    let mut record = row(0);
    record
        .data
        .insert("empty".into(), String::with_capacity(MAX_ROW_BYTES + 1));
    assert_eq!(
        limit(store.store(Arc::new(record)).await.unwrap_err()).id,
        PRESENCE_ROW_BYTES
    );
    assert!(store.get(Uuid::nil()).await.unwrap().is_none());
}

#[tokio::test]
async fn setting_the_region_updates_only_the_fetched_row() {
    let store = isolated();
    let session = Uuid::from_u128(1);
    let region = Uuid::from_u128(2);
    assert!(store.store(Arc::new(row(1))).await.unwrap());
    let cached = store.get(session).await.unwrap().unwrap();
    cached.set_region_id(region);
    assert_eq!(
        store.get(session).await.unwrap().unwrap().region_id(),
        region
    );

    // A replacement detaches the first row. The holder can still update its
    // cached region without writing the replacement back to the provider.
    assert!(store.store(Arc::new(row(1))).await.unwrap());
    let next = Uuid::from_u128(3);
    cached.set_region_id(next);
    assert_eq!(cached.region_id(), next);
    assert_eq!(
        store.get(session).await.unwrap().unwrap().region_id(),
        Uuid::nil()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_inserts_share_the_last_slot() {
    let store = isolated();
    for i in 0..MAX_SESSIONS - 1 {
        assert!(store.store(Arc::new(row(i))).await.unwrap());
    }
    let start = Arc::new(tokio::sync::Barrier::new(2));
    let insert = |i| {
        let store = store.clone();
        let start = Arc::clone(&start);
        async move {
            start.wait().await;
            store.store(Arc::new(row(i))).await
        }
    };
    let other = tokio::spawn(insert(MAX_SESSIONS));
    let results = [insert(MAX_SESSIONS - 1).await, other.await.unwrap()];
    assert_eq!(results.iter().filter(|r| matches!(r, Ok(true))).count(), 1);
    let refused = results.into_iter().find_map(Result::err).unwrap();
    assert_eq!(limit(refused).id, PRESENCE_NULL_SESSIONS);
    assert_eq!(lock(&store.sessions).index.len(), MAX_SESSIONS);
}
