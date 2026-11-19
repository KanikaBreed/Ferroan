// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fixed C# policy observations plus Ferroan's bounded async execution.
use super::*;
use serde_json::{Value, json};

fn ip(value: &str) -> IpAddr {
    parse_address(value).unwrap()
}
fn policy(ini: &str) -> ControlPlaneAccess {
    ControlPlaneAccess::from_config(&Config::parse(ini).unwrap()).unwrap()
}
fn answers(value: &Value, host: &str) -> io::Result<Vec<IpAddr>> {
    value[host]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| ip(item.as_str().unwrap()))
                .collect()
        })
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}
fn fixture() -> Value {
    serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/pin3/trust.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn fixed_csharp_host_forms_and_combined_lists_match_captured_observations() {
    let fixture = fixture();
    for case in fixture["host_forms"].as_array().unwrap() {
        assert_eq!(
            json!(
                parse_address(case["entry"].as_str().unwrap().trim())
                    .map(normalize_address)
                    .map(|address| address.to_string())
            ),
            case["address"],
            "{}",
            case["entry"]
        );
        assert_eq!(
            host_of(case["entry"].as_str().unwrap()),
            case["host"].as_str().unwrap(),
            "{}",
            case["entry"]
        );
    }
    let case = &fixture["combined_lists"];
    let policy = policy(case["ini"].as_str().unwrap());
    let mut calls = Vec::new();
    policy
        .resolve_with(|host| {
            calls.push(host.clone());
            std::future::ready(answers(&case["answers"], &host))
        })
        .await
        .unwrap();
    assert_eq!(json!(calls), case["lookups"]);
    for query in case["queries"].as_array().unwrap() {
        assert_eq!(
            policy
                .allows_with(ip(query["address"].as_str().unwrap()), |_| async {
                    panic!("unexpected refresh")
                })
                .await,
            query["allowed"].as_bool().unwrap(),
            "{}",
            query["address"]
        );
    }
}

#[tokio::test(start_paused = true)]
async fn minute_refresh_failure_retention_and_address_replacement_match_csharp_trace() {
    let fixture = fixture();
    let case = &fixture["refresh"];
    let policy = policy(case["ini"].as_str().unwrap());
    let mut calls = Vec::new();
    policy
        .resolve_with(|host| {
            calls.push(host.clone());
            std::future::ready(answers(&case["startup_answers"], &host))
        })
        .await
        .unwrap();
    assert_eq!(json!(calls), case["startup_lookups"]);
    let mut now = 0;
    for step in case["steps"].as_array().unwrap() {
        let next = step["seconds"].as_u64().unwrap();
        tokio::time::advance(Duration::from_secs(next - now)).await;
        now = next;
        calls.clear();
        let allowed = policy
            .allows_with(ip(step["query"].as_str().unwrap()), |host| {
                calls.push(host.clone());
                std::future::ready(answers(&step["answers"], &host))
            })
            .await;
        assert_eq!(allowed, step["allowed"].as_bool().unwrap(), "{step}");
        assert_eq!(json!(calls), step["lookups"], "{step}");
    }
}

#[tokio::test(start_paused = true)]
async fn concurrent_and_cancelled_requests_do_not_multiply_dns_batches() {
    let policy = policy("[Network]\nControlPlaneTrustedHosts=host.example.invalid\n");
    policy
        .resolve_with(|_| async { Ok(vec![ip("192.0.2.1")]) })
        .await
        .unwrap();
    tokio::time::advance(REFRESH_INTERVAL).await;
    let clone = policy.clone();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let mut held = Box::pin(policy.allows_with(ip("192.0.2.2"), |_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::pending()
    }));
    tokio::select! {
        biased;
        _ = &mut held => panic!("DNS unexpectedly completed"),
        _ = tokio::task::yield_now() => {},
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        !clone
            .allows_with(ip("192.0.2.2"), |_| async { panic!("parallel lookup") })
            .await
    );
    assert!(
        clone
            .allows_with(ip("192.0.2.1"), |_| async {
                panic!("trusted address lookup")
            })
            .await
    );
    drop(held);
    assert!(
        !clone
            .allows_with(ip("192.0.2.2"), |_| async {
                panic!("cancelled lookup retried before interval")
            })
            .await
    );
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(
        clone
            .allows_with(ip("192.0.2.2"), |_| async { Ok(vec![ip("192.0.2.2")]) })
            .await
    );
    assert!(
        policy.cached(ip("192.0.2.2")),
        "clones must share the new snapshot"
    );
    assert!(!policy.cached(ip("192.0.2.1")));
}

#[tokio::test(start_paused = true)]
async fn invalid_entries_warn_without_empty_dns_or_private_values_and_inputs_are_bounded() {
    let policy =
        policy("[Network]\nControlPlaneTrustedHosts=\"[]:8003,:8003,[broken,192.0.2.0/24\"\n");
    policy
        .resolve_with(|_| async { panic!("invalid entry reached DNS") })
        .await
        .unwrap();
    assert_eq!(policy.0.warned.lock().unwrap().len(), 4);
    assert!(!policy.allows(ip("192.0.2.9")).await);
    for value in [
        "x".repeat(MAX_HOST_BYTES + 1),
        "192.0.2.1,".repeat(MAX_HOSTS + 1),
    ] {
        let mut config = Config::default();
        config.set("Network", "ControlPlaneTrustedHosts", &value);
        let error = ControlPlaneAccess::from_config(&config).err().unwrap();
        assert!(matches!(error, ControlPlaneError::Limit));
        assert!(!error.to_string().contains(&value));
    }
}

#[tokio::test(start_paused = true)]
async fn oversized_and_stalled_dns_batches_preserve_the_last_complete_snapshot() {
    let ini = "[Network]\nControlPlaneTrustedHosts=host.example.invalid\n";
    let initial = policy(ini);
    assert!(matches!(
        initial
            .resolve_with(|_| async { Ok(vec![ip("192.0.2.1"); MAX_ADDRESSES + 1]) })
            .await,
        Err(ControlPlaneError::Limit)
    ));
    initial
        .resolve_with(|_| async { Ok(vec![ip("192.0.2.1")]) })
        .await
        .unwrap();
    assert!(
        initial.cached(ip("192.0.2.1")),
        "a failed startup must not masquerade as successful initialization on retry"
    );
    let initial = policy(ini);
    assert!(matches!(
        initial.resolve_with(|_| std::future::pending()).await,
        Err(ControlPlaneError::ResolveTimeout)
    ));
    let policy = policy(ini);
    policy
        .resolve_with(|_| async { Ok(vec![ip("192.0.2.1")]) })
        .await
        .unwrap();
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(
        !policy
            .allows_with(ip("192.0.2.2"), |_| async {
                Ok(vec![ip("192.0.2.2"); MAX_ADDRESSES + 1])
            })
            .await
    );
    assert!(policy.cached(ip("192.0.2.1")));
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(
        !policy
            .allows_with(ip("192.0.2.2"), |_| std::future::pending())
            .await
    );
    assert!(policy.cached(ip("192.0.2.1")));
}

#[tokio::test(start_paused = true)]
async fn aggregate_dns_cache_bound_discards_the_whole_refresh() {
    let policy = policy(
        "[Network]\nControlPlaneTrustedHosts=first.example.invalid,second.example.invalid\n",
    );
    policy
        .resolve_with(|name| async move {
            Ok(vec![if name.starts_with("first") {
                ip("192.0.2.1")
            } else {
                ip("192.0.2.2")
            }])
        })
        .await
        .unwrap();
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(
        !policy
            .allows_with(ip("2001:db8::1"), |_| async {
                Ok((0..MAX_ADDRESSES)
                    .map(|index| {
                        IpAddr::V6(std::net::Ipv6Addr::new(
                            0x2001,
                            0xdb8,
                            0,
                            0,
                            0,
                            0,
                            0,
                            index as u16,
                        ))
                    })
                    .collect())
            })
            .await
    );
    assert!(policy.cached(ip("192.0.2.1")));
    assert!(policy.cached(ip("192.0.2.2")));
    assert!(!policy.cached(ip("2001:db8::1")));
}
