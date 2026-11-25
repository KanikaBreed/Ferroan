// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use ferroan_protocol::{Msg, Writer};
use tokio::sync::Semaphore;

use super::*;

/// The grid as a test sets it up. `gate`, when set, holds every account call
/// until the test adds permits.
#[derive(Default)]
pub(crate) struct Fake {
    pub(crate) accounts: HashMap<Uuid, UserName>,
    pub(crate) grid_users: HashMap<Uuid, UserName>,
    pub(crate) calls: Mutex<Vec<(&'static str, Uuid)>>,
    pub(crate) gate: Option<Arc<Semaphore>>,
}

impl Fake {
    pub(crate) fn with_account(id: Uuid, first: &str, last: &str) -> Self {
        let mut fake = Self::default();
        fake.accounts.insert(id, UserName::new(first, last));
        fake
    }

    fn calls(&self, kind: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(call, _)| *call == kind)
            .count()
    }
}

impl NameSource for Fake {
    fn account(&self, id: Uuid) -> NameFuture {
        self.calls.lock().unwrap().push(("account", id));
        let found = self.accounts.get(&id).cloned();
        let gate = self.gate.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            found
        })
    }

    fn grid_user(&self, id: Uuid) -> NameFuture {
        self.calls.lock().unwrap().push(("grid_user", id));
        let found = self.grid_users.get(&id).cloned();
        Box::pin(async move { found })
    }
}

fn body<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

/// Wait until no lookup is running for this circuit.
async fn settled(circuit: &CircuitNames) {
    for _ in 0..200 {
        if circuit.waiting.load(Ordering::Acquire) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the lookups did not finish");
}

async fn reply(rx: &mut mpsc::Receiver<(Uuid, UserName)>) -> (Uuid, UserName) {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("a name reply")
        .expect("the reply channel stays open")
}

#[test]
fn the_reply_is_one_block_with_terminated_names() {
    let id = Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
    let reply = name_reply(id, &UserName::new("Test", "User"));
    let mut expected = vec![1];
    expected.extend_from_slice(id.as_bytes());
    expected.extend_from_slice(&[5, b'T', b'e', b's', b't', 0]);
    expected.extend_from_slice(&[5, b'U', b's', b'e', b'r', 0]);
    assert_eq!(body(&reply), expected);
    const { assert!(!UUIDNameReply::ZEROCODED) };
}

#[test]
fn names_are_cut_to_255_bytes_on_a_character_and_empty_stays_empty() {
    let long = format!("{}é", "a".repeat(253));
    let reply = name_reply(Uuid::nil(), &UserName::new(long, ""));
    let block = &reply.uuid_name_block[0];
    assert_eq!(block.first_name, [vec![b'a'; 253], vec![0]].concat());
    assert!(block.last_name.is_empty());
    assert_eq!(clamp("é".repeat(200)).len(), 254);
}

#[tokio::test]
async fn an_account_is_answered_by_its_lookup_then_from_the_cache() {
    let id = Uuid::from_u128(7);
    let fake = Arc::new(Fake::with_account(id, "Chat", "Bot"));
    let region = RegionNames::start(fake.clone());
    let (circuit, mut rx) = region.circuit();
    assert_eq!(circuit.request(id), Requested::Queued);
    assert_eq!(reply(&mut rx).await, (id, UserName::new("Chat", "Bot")));
    settled(&circuit).await;
    assert_eq!(
        circuit.request(id),
        Requested::Answered(UserName::new("Chat", "Bot"))
    );
    assert_eq!(fake.calls("account"), 1);
    assert_eq!(fake.calls("grid_user"), 0);
}

#[tokio::test]
async fn an_unknown_id_gets_no_reply_then_unknown_from_the_cache() {
    let id = Uuid::from_u128(9);
    let fake = Arc::new(Fake::default());
    let region = RegionNames::start(fake.clone());
    let (circuit, mut rx) = region.circuit();
    assert_eq!(circuit.request(id), Requested::Queued);
    settled(&circuit).await;
    assert!(rx.try_recv().is_err(), "an unknown id was answered");
    assert_eq!((fake.calls("account"), fake.calls("grid_user")), (1, 1));
    // `GetUser` caches the miss as tried, and the cache path answers it.
    assert_eq!(
        circuit.request(id),
        Requested::Answered(UserName::new("Unknown", id.to_string()))
    );
    assert_eq!(fake.calls("account"), 1);
}

#[tokio::test]
async fn an_arrival_without_an_account_answers_its_names_and_stays_untried() {
    let id = Uuid::from_u128(11);
    let fake = Arc::new(Fake::default());
    let region = RegionNames::start(fake.clone());
    region.add_arrival(id, "Test", "User");
    let (circuit, mut rx) = region.circuit();
    for round in 1..=2 {
        assert_eq!(circuit.request(id), Requested::Queued);
        assert_eq!(reply(&mut rx).await, (id, UserName::new("Test", "User")));
        settled(&circuit).await;
        assert_eq!(fake.calls("account"), round);
    }
    assert_eq!(fake.calls("grid_user"), 0);
}

#[tokio::test]
async fn an_arrival_is_checked_against_its_account() {
    let id = Uuid::from_u128(12);
    let fake = Arc::new(Fake::with_account(id, "Real", "Name"));
    let region = RegionNames::start(fake.clone());
    region.add_arrival(id, "Arrival", "Name");
    let (circuit, mut rx) = region.circuit();
    assert_eq!(circuit.request(id), Requested::Queued);
    assert_eq!(reply(&mut rx).await, (id, UserName::new("Real", "Name")));
    settled(&circuit).await;
    // A known entry is not replaced by a later arrival.
    region.add_arrival(id, "Other", "Name");
    assert_eq!(
        circuit.request(id),
        Requested::Answered(UserName::new("Real", "Name"))
    );
}

#[tokio::test]
async fn grid_user_names_an_id_with_no_account() {
    let id = Uuid::from_u128(13);
    let mut fake = Fake::default();
    fake.grid_users.insert(
        id,
        UserName::new("Visiting.User", "@elsewhere.example:8002"),
    );
    let fake = Arc::new(fake);
    let region = RegionNames::start(fake.clone());
    let (circuit, mut rx) = region.circuit();
    assert_eq!(circuit.request(id), Requested::Queued);
    assert_eq!(
        reply(&mut rx).await,
        (
            id,
            UserName::new("Visiting.User", "@elsewhere.example:8002")
        )
    );
}

#[tokio::test]
async fn a_closed_circuit_is_not_looked_up_for() {
    let id = Uuid::from_u128(14);
    let fake = Arc::new(Fake::with_account(id, "Chat", "Bot"));
    let region = RegionNames::start(fake.clone());
    let (circuit, rx) = region.circuit();
    drop(rx);
    assert_eq!(circuit.request(id), Requested::Queued);
    settled(&circuit).await;
    assert_eq!(fake.calls("account"), 0);
}

#[tokio::test]
async fn one_circuit_waits_for_at_most_its_bound() {
    let gate = Arc::new(Semaphore::new(0));
    let fake = Arc::new(Fake {
        gate: Some(Arc::clone(&gate)),
        ..Fake::default()
    });
    let region = RegionNames::start(fake);
    let (circuit, _rx) = region.circuit();
    for n in 0..MAX_CIRCUIT_LOOKUPS {
        assert_eq!(
            circuit.request(Uuid::from_u128(n as u128 + 1)),
            Requested::Queued
        );
    }
    assert_eq!(
        circuit.request(Uuid::from_u128(u128::MAX)),
        Requested::Refused
    );
    // Another circuit still has room.
    let (other, _other_rx) = region.circuit();
    assert_eq!(other.request(Uuid::from_u128(u128::MAX)), Requested::Queued);
    gate.add_permits(MAX_CIRCUIT_LOOKUPS + 1);
    settled(&circuit).await;
    assert_eq!(
        circuit.request(Uuid::from_u128(u128::MAX - 1)),
        Requested::Queued
    );
}

#[tokio::test]
async fn the_region_queue_holds_5000_waiting_lookups() {
    let gate = Arc::new(Semaphore::new(0));
    let fake = Arc::new(Fake {
        gate: Some(Arc::clone(&gate)),
        ..Fake::default()
    });
    let region = RegionNames::start(fake);
    let mut circuits = Vec::new();
    let (mut queued, mut refused) = (0, 0);
    let mut next = 1_u128;
    for _ in 0..12 {
        let (circuit, rx) = region.circuit();
        for _ in 0..MAX_CIRCUIT_LOOKUPS {
            match circuit.request(Uuid::from_u128(next)) {
                Requested::Queued => queued += 1,
                Requested::Refused => refused += 1,
                Requested::Answered(_) => panic!("nothing is cached"),
            }
            next += 1;
            // Let the dispatcher take what it can before the queue fills.
            tokio::task::yield_now().await;
        }
        circuits.push((circuit, rx));
    }
    // The queue, the two running lookups and the one waiting for a worker.
    assert!(
        (MAX_NAME_LOOKUPS..=MAX_NAME_LOOKUPS + LOOKUP_WORKERS + 1).contains(&queued),
        "{queued} queued"
    );
    assert_eq!(queued + refused, 12 * MAX_CIRCUIT_LOOKUPS);
    gate.add_permits(queued);
}

#[tokio::test]
async fn the_cache_keeps_its_bound_and_drops_the_soonest_entry() {
    let region = RegionNames::start(Arc::new(Fake::default()));
    for n in 0..=MAX_NAME_CACHE {
        region.add_arrival(Uuid::from_u128(n as u128 + 1), "First", "Last");
    }
    let cache = region.shared.cache.lock().unwrap();
    assert_eq!(cache.len(), MAX_NAME_CACHE);
    assert!(cache.contains_key(&Uuid::from_u128(MAX_NAME_CACHE as u128 + 1)));
}

#[tokio::test(start_paused = true)]
async fn a_looked_up_entry_lasts_30_minutes() {
    let id = Uuid::from_u128(15);
    let fake = Arc::new(Fake::with_account(id, "Chat", "Bot"));
    let region = RegionNames::start(fake.clone());
    let (circuit, mut rx) = region.circuit();
    assert_eq!(circuit.request(id), Requested::Queued);
    reply(&mut rx).await;
    settled(&circuit).await;
    tokio::time::advance(Duration::from_millis(1_799_000)).await;
    assert!(matches!(circuit.request(id), Requested::Answered(_)));
    tokio::time::advance(Duration::from_millis(2_000)).await;
    assert_eq!(circuit.request(id), Requested::Queued);
    reply(&mut rx).await;
    assert_eq!(fake.calls("account"), 2);
}
