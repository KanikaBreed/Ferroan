// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structure of `fixtures/jolt-matched`, the matched-native harness `drop`.
//! Ordinary tests read the committed JSON. They do not run the native.

use std::path::PathBuf;

#[test]
fn drop_matched_trace_names_the_run() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-matched");
    let file_manifest = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file/manifest.txt"),
    )
    .unwrap();
    let text = std::fs::read_to_string(root.join("drop.json")).unwrap();
    for needle in ["/home/", "/mnt/", "/tmp/", "/Users/"] {
        assert!(!text.contains(needle), "{needle}");
    }
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["format"].as_i64(), Some(1));
    let revision = doc["pin"].as_str().unwrap();
    assert_eq!(revision.len(), 40);
    assert!(revision.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(file_manifest.contains(&format!("revision {revision}\n")));
    assert_eq!(doc["baseline"], "det-886e0886-linux-x64");
    assert_eq!(doc["scenario"], "drop");
    assert_eq!(doc["heartbeat"], "1/11");
    assert_eq!(doc["rate_hz"].as_f64(), Some(11.0));
    assert_eq!(doc["physics_rate_hz"].as_f64(), Some(45.0));
    assert_eq!(doc["physics_step_rate_set"].as_f64(), Some(45.0));
    assert_eq!(doc["substepping"].as_bool(), Some(true));
    assert_eq!(doc["step_source"], "substeps");
    assert_eq!(doc["step_total"].as_i64(), Some(118));
    assert_eq!(doc["samples"].as_i64(), Some(29));
    assert_eq!(
        doc["meshing"],
        "OpenSim.Region.PhysicsModules.Meshing.Meshmerizer"
    );
    assert_eq!(doc["deterministic_mode"].as_bool(), Some(false));
    assert_eq!(doc["native_file"], "libjoltc.so");
    assert_eq!(doc["native_size"].as_i64(), Some(2_701_768));
    assert_eq!(
        doc["native_sha256"],
        "27EFC48C813E75294DA11EAF22618BC1E0A5426A50EDDE7870EF4A60C8E4288B"
    );
    assert_eq!(doc["native_rid"], "linux-x64");
    assert_eq!(doc["native_origin"], "Rebuilt");

    let steps: Vec<i64> = serde_json::from_value(doc["steps"].clone()).unwrap();
    assert_eq!(
        steps,
        vec![
            4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 4, 4, 4, 4, 4, 4, 4, 4,
        ]
    );
    assert_eq!(steps.iter().sum::<i64>(), 118);

    let clock = doc["clock"].as_array().unwrap();
    assert_eq!(clock.len(), 29);
    assert_eq!(clock[0]["i"].as_i64(), Some(0));
    assert!(clock[0]["input_ticks"].as_i64().unwrap() > 0);
    assert!(clock[0]["step_ticks"].is_null());
    assert_eq!(
        clock
            .iter()
            .filter(|sample| !sample["step_ticks"].is_null())
            .count(),
        14
    );

    let channels = &doc["channels"];
    let provenance = channels["provenance.txt"].as_str().unwrap();
    assert!(provenance.contains("heartbeat 1/11\n"));
    assert!(provenance.contains("physics_rate_hz 45\n"));
    assert!(provenance.contains("substepping true\n"));
    assert!(provenance.contains("step_source substeps\n"));
    assert!(provenance.contains("region_mesher true\n"));
    assert!(provenance.contains("samples 29\n"));
    assert!(provenance.contains("events 14\n"));
    assert!(provenance.contains("phases 17\n"));
    assert!(provenance.contains("seam_calls 8\n"));
    assert!(provenance.contains("seam_forwarded 8\n"));
    assert!(!provenance.contains("physics_rate_hz 0\n"));
    let config = channels["config.txt"].as_str().unwrap();
    assert!(config.contains("DeterministicMode false\n"));
    assert!(config.contains("PhysicsStepRate 42340000\n"));
    let samples = channels["samples.txt"].as_str().unwrap();
    assert_eq!(samples.lines().count(), 30);
    assert!(samples.lines().nth(1).unwrap().contains(" steps=4 "));
    assert!(samples.lines().nth(10).unwrap().contains(" steps=5 "));
    assert!(
        channels["events.txt"]
            .as_str()
            .unwrap()
            .contains("collider=0 ")
    );
    assert_ne!(
        channels["negative-clock.txt"].as_str().unwrap(),
        channels["clock.txt"].as_str().unwrap()
    );
    assert_ne!(
        channels["negative-collider.txt"].as_str().unwrap(),
        channels["events.txt"].as_str().unwrap()
    );
}
