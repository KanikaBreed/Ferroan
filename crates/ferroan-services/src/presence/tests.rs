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

#[test]
fn two_null_providers_match_the_pin8_sequence() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/presence/null.json")).unwrap();
    let stores = [NullPresenceData::default(), NullPresenceData::default()];
    let mut held = None;
    for (step, case) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let store = &stores[case["instance"].as_u64().unwrap() as usize];
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
                store.store(Arc::new(row)).unwrap();
                json!(true)
            }
            "get" => {
                held = store.get(id(input));
                held.as_deref().map_or(Value::Null, snapshot)
            }
            "held" => held.as_deref().map_or(Value::Null, snapshot),
            "restore" => {
                store.store(Arc::clone(held.as_ref().unwrap())).unwrap();
                json!(true)
            }
            "find" => {
                let rows = store
                    .get_by_field(text(&input["field"]), text(&input["value"]))
                    .unwrap();
                Value::Array(rows.iter().map(|row| snapshot(row)).collect())
            }
            "delete" => json!(
                store
                    .delete(text(&input["field"]), text(&input["value"]))
                    .unwrap()
            ),
            "report" => json!(store.report_agent(id(&input["session"]), id(&input["region"]))),
            "logout" => {
                store.logout_region_agents(id(input));
                Value::Null
            }
            "verify" => json!(store.verify_agent(id(&input["user"]), id(&input["secure"]))),
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

fn row(i: usize) -> PresenceData {
    PresenceData::new(
        "user".into(),
        Uuid::from_u128(i as u128),
        Uuid::nil(),
        BTreeMap::new(),
    )
}

#[test]
fn full_store_still_replaces_sessions_and_reuses_deleted_slots() {
    let store = isolated();
    for i in 0..MAX_SESSIONS {
        store.store(Arc::new(row(i))).unwrap();
    }
    let error = store.store(Arc::new(row(MAX_SESSIONS))).unwrap_err();
    assert_eq!(error.id, PRESENCE_NULL_SESSIONS);
    assert!(store.get(Uuid::from_u128(MAX_SESSIONS as u128)).is_none());

    let old = store.get(Uuid::nil()).unwrap();
    let replacement = Arc::new(row(0));
    store.store(Arc::clone(&replacement)).unwrap();
    assert!(Arc::ptr_eq(&store.get(Uuid::nil()).unwrap(), &replacement));
    let region = Uuid::from_u128(1);
    assert!(store.report_agent(Uuid::nil(), region));
    assert_eq!(replacement.region_id(), region);
    assert_eq!(old.region_id(), Uuid::nil());
    assert!(store.delete("SessionID", &Uuid::nil().to_string()).unwrap());
    store.store(Arc::new(row(MAX_SESSIONS))).unwrap();
    assert_eq!(lock(&store.sessions).slots.len(), MAX_SESSIONS);
}

#[test]
fn oversized_results_are_refused_and_bulk_removal_still_visits_every_match() {
    let store = isolated();
    for i in 0..MAX_RESULTS {
        store.store(Arc::new(row(i))).unwrap();
    }
    assert_eq!(
        store.get_by_field("UserID", "user").unwrap().len(),
        MAX_RESULTS
    );
    store.store(Arc::new(row(MAX_RESULTS))).unwrap();
    let err = store.get_by_field("UserID", "user").err().unwrap();
    assert_eq!((err.id, err.observed), (PRESENCE_RESULTS, MAX_RESULTS + 1));
    assert!(store.delete("UserID", "user").unwrap());
    assert!(store.get_by_field("UserID", "user").unwrap().is_empty());
}

#[test]
fn oversized_row_or_query_preserves_existing_sessions() {
    let store = isolated();
    let mut record = row(0);
    for i in 0..MAX_FIELDS {
        record.data.insert(i.to_string(), String::new());
    }
    store.store(Arc::new(record)).unwrap();
    let mut record = row(0);
    for i in 0..=MAX_FIELDS {
        record.data.insert(i.to_string(), String::new());
    }
    assert_eq!(
        store.store(Arc::new(record)).unwrap_err().id,
        PRESENCE_FIELDS
    );
    assert_eq!(store.get(Uuid::nil()).unwrap().data.len(), MAX_FIELDS);

    let mut record = row(0);
    record.data.insert(
        "key".into(),
        "a".repeat(MAX_ROW_BYTES - record.user_id.len() - 3),
    );
    store.store(Arc::new(record)).unwrap();
    let mut record = row(0);
    record.data.insert(
        "key".into(),
        "a".repeat(MAX_ROW_BYTES - record.user_id.len() - 2),
    );
    assert_eq!(
        store.store(Arc::new(record)).unwrap_err().id,
        PRESENCE_ROW_BYTES
    );
    let oversized = "a".repeat(MAX_ROW_BYTES + 1);
    assert_eq!(
        store.delete("key", &oversized).unwrap_err().id,
        PRESENCE_ROW_BYTES
    );
    assert_eq!(
        store.get_by_field("key", &oversized).err().unwrap().id,
        PRESENCE_ROW_BYTES
    );
    assert_eq!(
        store.get(Uuid::nil()).unwrap().data["key"].len(),
        MAX_ROW_BYTES - 7
    );
}

#[test]
fn a_user_id_at_the_row_limit_can_be_found_and_deleted() {
    let store = isolated();
    let mut record = row(0);
    record.user_id = "u".repeat(MAX_ROW_BYTES);
    let handle = Arc::new(record);
    store.store(Arc::clone(&handle)).unwrap();
    assert_eq!(
        store.get_by_field("UserID", &handle.user_id).unwrap().len(),
        1
    );
    assert!(store.delete("UserID", &handle.user_id).unwrap());
}

#[test]
fn spare_string_capacity_counts_towards_the_resident_row_budget() {
    let store = isolated();
    let mut record = row(0);
    record
        .data
        .insert("empty".into(), String::with_capacity(MAX_ROW_BYTES + 1));
    assert_eq!(
        store.store(Arc::new(record)).unwrap_err().id,
        PRESENCE_ROW_BYTES
    );
    assert!(store.get(Uuid::nil()).is_none());
}

#[test]
fn concurrent_inserts_share_the_last_slot() {
    let store = isolated();
    for i in 0..MAX_SESSIONS - 1 {
        store.store(Arc::new(row(i))).unwrap();
    }
    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let try_store = |i| {
            start.wait();
            store.store(Arc::new(row(i)))
        };
        let other = scope.spawn(move || try_store(MAX_SESSIONS));
        let results = [try_store(MAX_SESSIONS - 1), other.join().unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let refused = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert_eq!(refused.id, PRESENCE_NULL_SESSIONS);
    });
    assert_eq!(lock(&store.sessions).index.len(), MAX_SESSIONS);
}
