// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structure of `fixtures/jolt-record`, the in-process harness `drop`
//! and the analytic-shape mass and ray recording.
//! Ordinary tests replay the committed files. They do not run the native.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[test]
fn drop_recording_keeps_the_pinned_structure() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-record");
    let file_manifest = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file/manifest.txt"),
    )
    .unwrap();
    let manifest = std::fs::read_to_string(root.join("manifest.txt")).unwrap();
    assert_eq!(value(&manifest, "format "), "1");
    let revision = value(&manifest, "revision ");
    assert_eq!(revision.len(), 40);
    assert!(revision.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(revision, value(&file_manifest, "revision "));
    assert_eq!(value(&manifest, "baseline "), "det-886e0886-linux-x64");
    assert_eq!(value(&manifest, "scenario "), "drop");
    assert!(manifest.lines().any(|line| line == "drop"));

    let case_dir = root.join("drop");
    let provenance = read(&case_dir, "provenance.txt");
    let samples = read(&case_dir, "samples.txt");
    let events = read(&case_dir, "events.txt");
    let clock = read(&case_dir, "clock.txt");
    let config = read(&case_dir, "config.txt");
    let materials = read(&case_dir, "materials.txt");
    let providers = read(&case_dir, "providers.txt");
    let assets = read(&case_dir, "assets.txt");
    let cache = read(&case_dir, "cache.txt");
    for text in [
        &manifest,
        &provenance,
        &samples,
        &events,
        &clock,
        &config,
        &materials,
        &providers,
        &assets,
        &cache,
    ] {
        assert!(!text.contains("stock-1.0.4"), "{text}");
        assert!(!text.contains("stock-1.1.0"), "{text}");
        assert!(!text.contains("/home/"), "{text}");
        assert!(!text.contains("/mnt/"), "{text}");
        assert!(!text.contains("/tmp/"), "{text}");
    }

    assert_eq!(value(&provenance, "scenario "), "drop");
    assert_eq!(value(&provenance, "revision "), revision);
    assert_eq!(value(&provenance, "baseline "), "det-886e0886-linux-x64");
    assert_eq!(value(&provenance, "heartbeat "), "1/11");
    assert_eq!(value(&provenance, "rate_hz "), "11");
    assert_eq!(value(&provenance, "physics_rate_hz "), "0");
    assert_eq!(value(&provenance, "substepping "), "false");
    assert_eq!(value(&provenance, "step_source "), "one_step");
    assert_eq!(value(&provenance, "warmup_steps "), "1");
    assert_eq!(value(&provenance, "subscribe_ms "), "100");
    assert_eq!(value(&provenance, "native_file "), "libjoltc.so");
    assert_eq!(value(&provenance, "native_size "), "2701768");
    assert_eq!(
        value(&provenance, "native_sha256 "),
        "27EFC48C813E75294DA11EAF22618BC1E0A5426A50EDDE7870EF4A60C8E4288B"
    );
    assert_eq!(value(&provenance, "native_rid "), "linux-x64");
    assert_eq!(value(&provenance, "native_origin "), "Rebuilt");
    assert_eq!(value(&provenance, "package "), "JoltPhysics.Native");
    assert_eq!(value(&provenance, "package_version "), "1.1.0");
    assert_eq!(value(&provenance, "harness_commit "), revision);
    assert_eq!(value(&provenance, "first_heartbeat "), "0");
    assert_eq!(value(&provenance, "samples "), "31");
    assert_eq!(value(&provenance, "events "), "16");
    assert_eq!(value(&provenance, "phases "), "19");
    assert_eq!(value(&provenance, "provider_calls "), "6");
    assert_eq!(value(&provenance, "stock_providers "), "0");
    assert_eq!(value(&provenance, "seam_calls "), "6");
    assert_eq!(value(&provenance, "seam_forwarded "), "6");
    assert_eq!(value(&provenance, "body_local "), "1000");
    assert_eq!(value(&provenance, "body_handle "), "16777217");
    assert_eq!(value(&provenance, "effective_step_rate "), "00000000");
    assert_eq!(value(&provenance, "effective_warning "), "none");
    assert_eq!(value(&provenance, "effective_note "), "none");
    assert!(!provenance.contains("45 Hz"));

    let dt = format!("{:016X}", (1.0f64 / 11.0).to_bits());
    assert_eq!(value(&provenance, "dt_f64 "), dt);
    assert_eq!(value(&provenance, "first_t "), dt);
    assert_eq!(
        value(&provenance, "simulate_dt_f32 "),
        format!("{:08X}", ((1.0f64 / 11.0) as f32).to_bits())
    );

    let sample_lines: Vec<_> = samples.lines().skip(1).collect();
    assert_eq!(sample_lines.len(), 31);
    for line in &sample_lines {
        assert!(line.contains(" steps=1 "), "{line}");
        assert!(line.contains(" awake_bodies=1 "), "{line}");
        assert!(line.contains(" awake=1 "), "{line}");
        assert!(!line.contains(" active="), "{line}");
    }
    assert!(sample_lines[0].contains(&format!(" t={dt} ")));

    assert_eq!(
        events
            .lines()
            .filter(|line| line.starts_with("raw "))
            .count(),
        16
    );
    assert_eq!(
        events
            .lines()
            .filter(|line| line.starts_with("phase "))
            .count(),
        19
    );
    assert!(events.contains("collider=0 "));
    assert!(events.contains("phase=land-start"));

    let clock_lines: Vec<_> = clock.lines().skip(1).collect();
    assert_eq!(clock_lines.len(), 31);
    assert!(clock_lines[0].contains("step_ticks=unobserved"));
    assert!(
        clock_lines
            .iter()
            .any(|line| !line.contains("step_ticks=unobserved"))
    );

    assert!(config.contains("PhysicsStepRate 00000000\n"));
    assert!(config.contains("PhysicsStepRateSet true\n"));
    assert!(config.contains("DeterministicMode false\n"));
    assert!(config.contains("ThreadCount 0\n"));
    assert!(config.contains("CollisionSteps 6\n"));
    assert!(config.contains("VehicleBehaviour InWorldz\n"));
    assert!(config.contains(&format!("Gravity {:08X}\n", (-9.80665f32).to_bits())));

    assert!(materials.contains(&format!("friction {:08X}\n", 0.6f32.to_bits())));
    assert!(materials.contains(&format!("restitution {:08X}\n", 0.5f32.to_bits())));
    assert!(materials.contains(&format!("density {:08X}\n", 1000f32.to_bits())));
    assert!(materials.contains(&format!("gravity {:08X}\n", 1f32.to_bits())));
    assert!(materials.contains("buoyancy 00000000\n"));

    assert_eq!(
        providers,
        "format 1\nseam RecordingStepForceProvider\nstock_attached 0\n"
    );
    let empty = sha256(b"");
    assert_eq!(
        assets,
        format!("format 1\nbytes 0\nsha256 {empty}\nkind analytic-box\n")
    );
    assert_eq!(
        cache,
        format!("format 1\ncold true\nfiles_before 0\nfiles_after 0\ncontent_sha256 {empty}\n")
    );

    for name in [
        "samples",
        "events",
        "providers",
        "clock",
        "config",
        "materials",
        "assets",
        "cache",
    ] {
        let bytes = std::fs::read(case_dir.join(format!("{name}.txt"))).unwrap();
        let line = format!("channel {name} {}", sha256(&bytes));
        assert!(provenance.contains(&format!("{line}\n")), "{line}");
    }

    one_number_later(
        &clock,
        &read(&case_dir, "negative-clock.txt"),
        "input_ticks=",
    );
    one_number_later(
        &events,
        &read(&case_dir, "negative-collider.txt"),
        "collider=",
    );

    assert!(manifest.lines().any(|line| line == "analytic-envelope"));
    let envelope = std::fs::read_to_string(root.join("analytic-envelope.txt")).unwrap();
    for banned in ["/home/", "/mnt/", "/tmp/"] {
        assert!(!envelope.contains(banned), "{banned}");
    }
    assert_eq!(value(&envelope, "format "), "1");
    assert_eq!(value(&envelope, "channel "), "harness");
    assert_eq!(value(&envelope, "scenario "), "analytic-envelope");
    assert_eq!(value(&envelope, "baseline "), "det-886e0886-linux-x64");
    assert_eq!(value(&envelope, "mass "), "JoltPrim.Mass");
    assert_eq!(value(&envelope, "mass_source "), "JoltPrim.cs 1228-1236");
    assert_eq!(value(&envelope, "mass_read "), "MassReadoutTests.cs 78-82");
    assert_eq!(value(&envelope, "ray "), "JoltScene.RaycastWorld");
    assert_eq!(value(&envelope, "ray_source "), "JoltScene.cs 2274-2280");
    assert_eq!(value(&envelope, "extent "), "PhysicsActor.Size");
    assert_eq!(value(&envelope, "rate_hz "), "11");
    assert_eq!(value(&envelope, "physics_rate_hz "), "0");
    assert_eq!(value(&envelope, "samples "), "2");
    assert_eq!(value(&envelope, "reads "), "2");
    assert_eq!(
        line_after(&envelope, "shape box-static "),
        "physical 0 local 4101 pos 42800000 42800000 42340000 size 3F800000 3F800000 3F800000 mass 41200000"
    );
    assert_eq!(
        line_after(&envelope, "shape box-dynamic "),
        "physical 1 local 4102 pos 42A00000 42A00000 42340000 size 3F800000 3F800000 3F800000 mass 41200000"
    );
    assert_eq!(
        line_after(&envelope, "shape sphere-static "),
        "physical 0 local 4103 pos 42C00000 42C00000 42340000 size 40000000 40000000 40000000 mass 42278D36"
    );
    assert_eq!(
        line_after(&envelope, "shape sphere-dynamic "),
        "physical 1 local 4104 pos 42E00000 42E00000 42340000 size 40000000 40000000 40000000 mass 42278D36"
    );
    assert_eq!(
        line_after(&envelope, "shape cylinder-static "),
        "physical 0 local 4105 pos 43000000 42400000 42340000 size 3F800000 3F800000 40000000 mass 417B53D2"
    );
    assert_eq!(
        line_after(&envelope, "shape cylinder-dynamic "),
        "physical 1 local 4106 pos 43100000 42400000 42340000 size 3F800000 3F800000 40000000 mass 417B53D2"
    );
    assert!(line_after(&envelope, "ray box-static ").contains(
        "heartbeats 1 start 426C0000 42800000 42340000 dir 3F800000 00000000 00000000 length 41200000 hits 1 consumer 4101 pos 427E0000 42800000 42340000 normal BF800000 00000000 00000000 depth 40900000"
    ));
    assert!(line_after(&envelope, "ray sphere-static ").contains(
        "heartbeats 1 start 42B60000 42C00000 42340000 dir 3F800000 00000000 00000000 length 41200000 hits 1 consumer 4103 pos 42BE0000 42C00000 42340000 normal BF800000 00000000 00000000 depth 40800000"
    ));
    assert!(line_after(&envelope, "ray cylinder-static ").contains(
        "heartbeats 1 start 42F60000 42400000 42340000 dir 3F800000 00000000 00000000 length 41200000 hits 1 consumer 4105 pos 42FF0000 42400000 42340000 normal BF800000 00000000 00000000 depth 40900000"
    ));
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

fn value<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn line_after<'a>(text: &'a str, key: &str) -> &'a str {
    value(text, key)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            out.push_str(&format!("{byte:02X}"));
            out
        })
}

fn one_number_later(original: &str, mutated: &str, key: &str) {
    let (start, end) = number_at(original, key);
    let (changed_start, changed_end) = number_at(mutated, key);
    assert_eq!(&original[..start], &mutated[..changed_start]);
    assert_eq!(&original[end..], &mutated[changed_end..]);
    let value: i128 = original[start..end].parse().unwrap();
    let changed: i128 = mutated[changed_start..changed_end].parse().unwrap();
    assert_eq!(changed, value + 1);
}

fn number_at(text: &str, key: &str) -> (usize, usize) {
    let start = text.find(key).unwrap() + key.len();
    let end = start
        + text[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .count();
    assert!(end > start, "{key}");
    (start, end)
}
