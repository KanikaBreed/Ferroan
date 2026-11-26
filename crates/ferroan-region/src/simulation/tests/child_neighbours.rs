// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::grid::{GridClient, VisibleNeighbours};
use crate::simulation_client::{AgentPosition, SimulationClient};
use std::collections::HashSet;
use std::io::Read;

fn fixture_uuid(n: u128) -> Uuid {
    Uuid::from_u128(0x00000000_0000_4000_8000_000000000000 + n)
}

#[tokio::test]
async fn visible_child_requests_match_the_two_region_capture() {
    let trace: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/child-view/trace.json"
    ))
    .unwrap();
    let captured = trace["requests"].as_array().unwrap();
    let (sent, mut received) = mpsc::channel(3);
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, bytes: axum::body::Bytes| {
            let sent = sent.clone();
            async move {
                let mut json = Vec::new();
                flate2::read::GzDecoder::new(bytes.as_ref())
                    .read_to_end(&mut json)
                    .unwrap();
                sent.send((
                    method,
                    uri.path().to_owned(),
                    serde_json::from_slice::<Value>(&json).unwrap(),
                ))
                .await
                .unwrap();
                r#"{"success":true,"reason":"Authorized"}"#
            }
        },
    );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("http://{}/", socket.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let home = RegionEnd {
        server_uri: "http://home.invalid/".into(),
        region_id: fixture_uuid(2),
        name: "Fixture Home".into(),
        loc_x: 256000,
        loc_y: 256000,
    };
    let dest = RegionEnd {
        server_uri: uri.clone(),
        region_id: fixture_uuid(9),
        name: "Fixture Neighbour".into(),
        loc_x: 256256,
        loc_y: 256000,
    };
    let agent = fixture_uuid(4);
    let mut incoming: Map<String, Value> = serde_json::from_str(CREATE_REQUEST).unwrap();
    for (key, value) in [
        ("agent_id", json!(agent.to_string())),
        ("session_id", json!(fixture_uuid(7).to_string())),
        ("secure_session_id", json!(fixture_uuid(8).to_string())),
        ("circuit_code", json!("1")),
        ("first_name", json!("Fixture")),
        ("last_name", json!("Observer")),
        ("client_ip", json!("192.0.2.2")),
        ("mac", json!("0".repeat(32))),
        ("id0", json!("0".repeat(32))),
    ] {
        incoming.insert(key.into(), value);
    }
    let config = test_config(&uri, home.region_id);
    let (tx, mut arrivals) = mpsc::channel(1);
    let state = SimulationState::new(&config, CapsRegistry::new(&uri).unwrap(), tx);
    state
        .accept(AgentFields::parse(&incoming, agent, home.region_id).unwrap())
        .unwrap();
    let root = arrivals.recv().await.unwrap();
    let path = Uuid::new_v4().to_string();
    let mut child = root.visible_child_circuit(&home, &dest, [200., 128., 6.916], 255., 1.9, &path);
    let seed = Uuid::parse_str(&child.caps_path).unwrap();
    assert_eq!(seed.get_version_num(), 4);
    assert_ne!(child.caps_path, root.caps_path);
    let client = SimulationClient::new().unwrap();
    assert!(
        client
            .create_visible_child(&dest, &mut child)
            .await
            .unwrap()
            .success
    );
    let (method, path, mut body) = received.recv().await.unwrap();
    assert_eq!(method, "POST");
    assert_eq!(path, format!("/agent/{agent}/"));
    // Apply the fixture aliases only after checking the value/identity they replace.
    let want = &captured[0]["fields"];
    assert_eq!(body["caps_path"], seed.to_string());
    body["caps_path"] = want["caps_path"].clone();
    assert_eq!(body["source_server_uri"], uri);
    body["source_server_uri"] = want["source_server_uri"].clone();
    let mut items = HashSet::new();
    for (i, slot) in body["packed_appearance"]["wearables"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        for w in slot.as_array_mut().unwrap() {
            let item = Uuid::parse_str(w["item"].as_str().unwrap()).unwrap();
            assert!(!item.is_nil() && items.insert(item));
            w["item"] = json!(Uuid::from_u128(100 + i as u128).to_string());
        }
    }
    assert_eq!(body, *want);
    let throttles: [u8; 28] =
        serde_json::from_value(captured[1]["fields"]["throttles"].clone()).unwrap();
    for (i, (position, center)) in [
        ([210.05249, 128., 5.539], [205.36133, 128., 6.171]),
        ([220.10498, 128., 3.8189998], [215.6372, 128., 4.5309997]),
    ]
    .into_iter()
    .enumerate()
    {
        let update = AgentPosition {
            agent_id: root.expect.agent_id,
            session_id: root.expect.session_id,
            region_handle: config.handle(),
            position,
            center,
            velocity: [2.4576, 0., 0.],
            size: [0.45, 0.6, 1.9],
            far: 255.,
            viewer_ui_is_god: Some(false),
            throttles: Some(throttles),
        };
        assert!(client.update_child_position(&dest, update).await.unwrap());
        let (method, target, body) = received.recv().await.unwrap();
        assert_eq!(method, "PUT");
        assert_eq!(target, path);
        assert_eq!(body, captured[i + 1]["fields"]);
    }
    server.abort();
}

#[tokio::test]
async fn range_query_and_view_use_variable_sized_world_bounds() {
    let (tx, mut requests) = mpsc::unbounded_channel();
    let rows = [
        (1, 256000, 256000, 256, 512, Some(4)), // home
        (2, 256256, 256000, 256, 512, Some(0)), // offline
        (3, 256256, 256000, 512, 512, Some(4)), // touches east
        (4, 255040, 256000, 1024, 512, None),   // touches west, legacy grid
        (5, 256257, 256000, 256, 512, Some(4)), // one metre beyond east
        (6, 256160, 256193, 256, 512, Some(4)), // one metre beyond north
        (7, 256160, 255872, 256, 128, Some(4)), // touches south
        (8, 256160, 256192, 256, 512, Some(4)), // touches north
        (9, 256160, 255871, 256, 128, Some(4)), // one metre beyond south
    ];
    let mut xml = String::from("<ServerResponse>");
    for (id, x, y, sx, sy, flags) in rows {
        let flag = flags.map_or_else(String::new, |f| format!("<flags>{f}</flags>"));
        xml.push_str(&format!(
            "<region{id} type=\"List\"><uuid>{}</uuid><locX>{x}</locX><locY>{y}</locY>\
             <sizeX>{sx}</sizeX><sizeY>{sy}</sizeY>{flag}</region{id}>",
            Uuid::from_u128(id)
        ));
    }
    xml.push_str("</ServerResponse>");
    let app = axum::Router::new().route(
        "/grid",
        axum::routing::post(move |body: String| {
            tx.send(body).unwrap();
            let reply = xml.clone();
            async move { reply }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut home = test_config(&uri, Uuid::from_u128(1));
    home.scope_id = Uuid::from_u128(8);
    home.size = [256, 512];
    let distances = |max: u32| {
        let cfg =
            ferroan_config::Config::parse(&format!("[Startup]\nMaxRegionsViewDistance={max}\n"))
                .unwrap();
        crate::config::RegionViewDistances::read(&cfg).unwrap()
    };
    home.region_view = distances(512);
    let mut regions = VisibleNeighbours::new(GridClient::new(&home), &home);
    let pos = [160.95, 96.7, 40.];
    for max in [0, 1] {
        home.region_view = distances(max);
        let mut disabled = VisibleNeighbours::new(GridClient::new(&home), &home);
        assert!(disabled.in_view(pos, 32.).await.is_empty());
    }
    assert!(requests.try_recv().is_err());
    let visible = regions.in_view(pos, 32.).await;
    assert_eq!(
        visible
            .iter()
            .map(|r| r.region.region_id.as_u128())
            .collect::<Vec<_>>(),
        [3, 4, 7, 8]
    );
    let expected = format!(
        "SCOPEID={}&XMIN=255489&XMAX=256767&YMIN=255489&YMAX=257023&METHOD=get_region_range",
        home.scope_id
    );
    assert_eq!(requests.recv().await.unwrap(), expected);
    assert!(regions.in_view([4000., 4000., 0.], 32.).await.is_empty());
    assert!(requests.try_recv().is_err());
    // Refetch at 30s, even though the roots have only moved within the scene.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(29)).await;
    regions.in_view(pos, 32.).await;
    assert!(requests.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    regions.in_view(pos, 32.).await;
    assert_eq!(requests.recv().await.unwrap(), expected);
    server.abort();
}

#[tokio::test]
async fn position_puts_keep_the_latest_and_back_off_per_simulator() {
    let (tx, mut requests) = mpsc::channel(6);
    let gate = Arc::new(tokio::sync::Notify::new());
    let first = gate.clone();
    let app = axum::Router::new().fallback(move |bytes: axum::body::Bytes| {
        let gate = first.clone();
        let sent = tx.clone();
        async move {
            let body: Value =
                serde_json::from_reader(flate2::read::GzDecoder::new(bytes.as_ref())).unwrap();
            let position = body["position"].as_str().unwrap();
            sent.send(position.to_string()).await.unwrap();
            if position == "<1, 0, 0>" {
                gate.notified().await;
            }
            if position == "<4, 0, 0>" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dest = RegionEnd {
        server_uri: format!("http://{}/", listener.local_addr().unwrap()),
        region_id: Uuid::from_u128(3),
        name: "Neighbour".into(),
        loc_x: 0,
        loc_y: 0,
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SimulationClient::new().unwrap();
    let update = |agent, x| AgentPosition {
        agent_id: Uuid::from_u128(agent),
        session_id: Uuid::from_u128(2),
        region_handle: 0,
        position: [x, 0., 0.],
        velocity: [0.; 3],
        center: [0.; 3],
        size: [0.; 3],
        far: 255.,
        viewer_ui_is_god: None,
        throttles: None,
    };
    let first = tokio::spawn({
        let client = client.clone();
        let dest = dest.clone();
        async move { client.update_child_position(&dest, update(1, 1.)).await }
    });
    assert_eq!(requests.recv().await.unwrap(), "<1, 0, 0>");
    assert!(
        client
            .update_child_position(&dest, update(1, 2.))
            .await
            .unwrap()
    );
    assert!(
        client
            .update_child_position(&dest, update(1, 3.))
            .await
            .unwrap()
    );
    gate.notify_one();
    assert!(first.await.unwrap().unwrap());
    assert_eq!(requests.recv().await.unwrap(), "<3, 0, 0>");

    assert!(
        !client
            .update_child_position(&dest, update(1, 4.))
            .await
            .unwrap()
    );
    assert_eq!(requests.recv().await.unwrap(), "<4, 0, 0>");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(119)).await;
    assert!(
        !client
            .update_child_position(&dest, update(2, 5.))
            .await
            .unwrap()
    );
    assert!(requests.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    assert!(
        client
            .update_child_position(&dest, update(2, 6.))
            .await
            .unwrap()
    );
    assert_eq!(requests.recv().await.unwrap(), "<6, 0, 0>");
    server.abort();
}

#[tokio::test]
async fn each_visible_child_gets_a_fresh_seed_and_the_original_travel_session() {
    let travel = Uuid::new_v4().to_string();
    let mut incoming: Map<String, Value> = serde_json::from_str(CREATE_REQUEST).unwrap();
    incoming.insert("service_session_id".into(), json!(travel));
    let home = RegionEnd {
        server_uri: "http://home.invalid/".into(),
        name: "Home".into(),
        region_id: fixture_uuid(2),
        loc_x: 256,
        loc_y: 1024,
    };
    let dest = RegionEnd {
        loc_x: 0,
        loc_y: 1536,
        ..home.clone()
    };
    let (tx, mut arrivals) = mpsc::channel(1);
    let state = SimulationState::new(
        &test_config(&home.server_uri, home.region_id),
        CapsRegistry::new(&home.server_uri).unwrap(),
        tx,
    );
    state
        .accept(AgentFields::parse(&incoming, fixture_uuid(1), home.region_id).unwrap())
        .unwrap();
    let root = arrivals.recv().await.unwrap();
    let circuit =
        |path: &str| root.visible_child_circuit(&home, &dest, [13., 48., 75.], 128., 2.3, path);
    let first_path = Uuid::new_v4().to_string();
    let first = circuit(&first_path);
    assert_eq!(first.caps_path, first_path);
    assert_ne!(
        first.caps_path,
        circuit(&Uuid::new_v4().to_string()).caps_path
    );
    assert_eq!(first.start_pos, [269., -464., 75.]);
    assert_eq!(first.service_session_id, travel);
    assert_eq!(first.appearance.unwrap().height, 2.3);
}

#[tokio::test]
async fn a_visible_child_posts_the_claimed_seed() {
    let (sent, mut received) = mpsc::channel(1);
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, bytes: axum::body::Bytes| {
            let sent = sent.clone();
            async move {
                let mut json = Vec::new();
                flate2::read::GzDecoder::new(bytes.as_ref())
                    .read_to_end(&mut json)
                    .unwrap();
                sent.send((
                    method.to_string(),
                    uri.path().to_owned(),
                    serde_json::from_slice::<Value>(&json).unwrap(),
                ))
                .await
                .unwrap();
                r#"{"success":true,"reason":"Authorized"}"#
            }
        },
    );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("http://{}/", socket.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let home = RegionEnd {
        server_uri: "http://home.invalid/".into(),
        region_id: fixture_uuid(2),
        name: "Home".into(),
        loc_x: 256,
        loc_y: 256,
    };
    let dest = RegionEnd {
        server_uri: uri,
        region_id: fixture_uuid(9),
        name: "Neighbour".into(),
        loc_x: 512,
        loc_y: 256,
    };
    let agent = fixture_uuid(4);
    let mut incoming: Map<String, Value> = serde_json::from_str(CREATE_REQUEST).unwrap();
    incoming.insert("agent_id".into(), json!(agent.to_string()));
    let (tx, mut arrivals) = mpsc::channel(1);
    let state = SimulationState::new(
        &test_config(&home.server_uri, home.region_id),
        CapsRegistry::new(&home.server_uri).unwrap(),
        tx,
    );
    state
        .accept(AgentFields::parse(&incoming, agent, home.region_id).unwrap())
        .unwrap();
    let root = arrivals.recv().await.unwrap();
    let generation = state.root_generation(agent).unwrap();
    let claimed = state.claim_child(agent, generation, dest.clone());
    let crate::children::Claim::New { seed } = claimed else {
        panic!("the visibility admission did not own the destination");
    };
    assert!(matches!(
        state.claim_child(agent, generation, dest.clone()),
        crate::children::Claim::Existing { seed: ref again } if again == &seed
    ));
    let mut child = root.visible_child_circuit(&home, &dest, [10., 20., 30.], 255., 1.9, &seed);
    assert!(
        SimulationClient::new()
            .unwrap()
            .create_visible_child(&dest, &mut child)
            .await
            .unwrap()
            .success
    );
    let (method, path, body) = received.recv().await.unwrap();
    assert_eq!(method, "POST");
    assert_eq!(path, format!("/agent/{agent}/"));
    assert_eq!(body["caps_path"], seed);
    assert_eq!(body["child"], true);
    assert!(matches!(
        state.finish_child(agent, generation, dest.region_id),
        crate::children::Admission::Announce { seed: announced, .. } if announced == seed
    ));
    server.abort();
}

#[test]
fn child_view_distances_follow_scene_settings() {
    use crate::config::RegionViewDistances;
    let defaults = RegionViewDistances::read(&ferroan_config::Config::default()).unwrap();
    assert_eq!(defaults.for_draw_distance(0.), 96.);
    assert_eq!(defaults.for_draw_distance(128.), 192.);
    assert_eq!(defaults.for_draw_distance(512.), 255.);
    let adjusted = ferroan_config::Config::parse(
        "[Startup]\nMinRegionsViewDistance=200\nMaxRegionsViewDistance=512\nMaxDrawDistance=128\n",
    )
    .unwrap();
    let limits = RegionViewDistances::read(&adjusted).unwrap();
    assert_eq!(limits.for_draw_distance(32.), 128.);
    assert_eq!(limits.maximum, 128.);
}
