// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replays `fixtures/jolt-file/character` against the `JoltCharacter` port.
//! The expected bits are the file oracle's. Default speed factors are also
//! the stock drop recording's config.

use std::path::PathBuf;

use crate::JoltConfig;
use crate::character::{
    DEFAULT_AVATAR_MASS, DEFAULT_AVATAR_SIZE, EDGE_MARGIN, Intent, MarkerRequest, ShapeState,
    SpeedFactors, capsule_from_size, contact_wanted_after_subscribe,
    contact_wanted_after_unsubscribe, desired_velocity, ground_flags, jump_latch_after_drain,
    jump_latch_on_press, keep_in_region, marker_body_user_data, marker_descriptor, mass_for,
    movement_sends_latch, stand_half_for, store_buoyancy, store_target,
};

#[test]
fn character_fixture_matches_the_pinned_methods() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file");
    let manifest = std::fs::read_to_string(root.join("manifest.txt")).unwrap();
    assert!(manifest.contains("compile OpenSim.Region.PhysicsModules.Jolt/JoltCharacter.cs\n"));
    assert!(manifest.lines().any(|line| line == "character"));
    let native = std::fs::read_to_string(root.join("note-dilation/native.txt")).unwrap();
    assert_eq!(native.matches("joltc_mapped false\n").count(), 3);
    assert!(!native.contains("joltc_mapped true"));

    let result = std::fs::read_to_string(root.join("character/result.txt")).unwrap();
    let calls = std::fs::read_to_string(root.join("character/calls.txt")).unwrap();
    assert!(result.starts_with("format 1\n"));
    assert!(calls.starts_with("format 1\n"));
    assert!(result.contains("native not_started\n"));
    assert!(result.contains("marker_query not_recorded\n"));
    assert!(result.contains("body_cap not_recorded\n"));
    assert!(result.contains("matched_native waits\n"));

    let header = header(&result);
    assert_eq!(header.region, [256.0, 256.0]);
    assert_eq!(hex(EDGE_MARGIN), header.edge);
    assert_eq!(hex(DEFAULT_AVATAR_MASS), header.default_mass);
    assert_eq!(vec3(DEFAULT_AVATAR_SIZE), header.default_size);
    assert_eq!(hex(mass_for(DEFAULT_AVATAR_SIZE)), header.default_mass);
    anchor_drop_factors(&header.factors);

    let mut capsules = 0u32;
    let mut shape = ShapeState::new(parse3(&header.default_size));
    let mut buoyancy = 0.0f32;
    let mut saw_half = false;
    let mut saw_above = false;
    for line in result.lines() {
        let f = fields(line);
        match f.first().copied() {
            Some("capsule") => {
                capsules += 1;
                check_capsule(&f, &header);
            }
            Some("desc") => check_desc(&f, &calls, &header),
            Some("move") => check_move(&f, &header),
            Some("intent") => check_intent(&f),
            Some("jump") => check_jump(&f),
            Some("hold") => check_hold(&f),
            Some("drain") => {
                saw_half |= f[2] == "3F000000";
                saw_above |= f[2] == "3F000001";
                check_drain(&f, &header);
            }
            Some("size") => check_size(&f, &mut shape),
            Some("buoyancy") => buoyancy = check_buoyancy(&f, buoyancy),
            Some("subscribe") => check_subscribe(&f),
            Some("reject_target") => check_reject(&f),
            Some("keep") => check_keep(&f),
            _ => {}
        }
    }
    assert!(capsules >= 2, "capsule rows");
    assert!(saw_half && saw_above, "0.5 and the next float");
    replay_calls(&calls, &result);
}

fn anchor_drop_factors(factors: &Factors) {
    let config = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jolt-record/drop/config.txt"),
    )
    .unwrap();
    assert_eq!(config_hex(&config, "AvatarWalkSpeedFactor"), factors.walk);
    assert_eq!(config_hex(&config, "AvatarRunSpeedFactor"), factors.run);
    assert_eq!(config_hex(&config, "AvatarFlySpeedFactor"), factors.fly);
    assert_eq!(
        config_hex(&config, "AvatarFlyDownSpeedFactor"),
        factors.fly_down
    );
    assert_eq!(config_hex(&config, "AvatarJumpSpeed"), factors.jump);
    let defaults = JoltConfig::default();
    assert_eq!(hex(defaults.avatar_walk_speed_factor), factors.walk);
    assert_eq!(hex(defaults.avatar_run_speed_factor), factors.run);
    assert_eq!(hex(defaults.avatar_fly_speed_factor), factors.fly);
    assert_eq!(hex(defaults.avatar_fly_down_speed_factor), factors.fly_down);
    assert_eq!(hex(defaults.avatar_jump_speed), factors.jump);
}

fn check_capsule(f: &[&str], header: &Header) {
    assert_eq!(f.len(), 9, "{f:?}");
    let size = [bits(f[2]), bits(f[3]), bits(f[4])];
    let capsule = capsule_from_size(size);
    assert_eq!(hex(capsule.radius), f[5], "radius {f:?}");
    assert_eq!(hex(capsule.half_height), f[6], "half {f:?}");
    assert_eq!(hex(stand_half_for(size)), f[7], "stand {f:?}");
    assert_eq!(hex(mass_for(size)), f[8], "mass {f:?}");
    if capsule.radius.is_finite() {
        assert!(
            capsule.radius >= 0.2_f32,
            "radius {} is below 0.2",
            capsule.radius
        );
    }
    if f[1] == "1" {
        assert_eq!(f[8], header.default_mass);
        assert_eq!(vec3(size), header.default_size);
    }
}

fn check_desc(f: &[&str], calls: &str, header: &Header) {
    assert_eq!(f.len(), 19, "{f:?}");
    let create = fields(line_starting(calls, "create "));
    assert_eq!(create.len(), 13, "{create:?}");
    let subscribed_ms = create[12].parse().unwrap();
    let desc = marker_descriptor(MarkerRequest {
        user_data: create[1].parse().unwrap(),
        position: [bits(create[2]), bits(create[3]), bits(create[4])],
        size: [bits(create[5]), bits(create[6]), bits(create[7])],
        orientation: [
            bits(create[8]),
            bits(create[9]),
            bits(create[10]),
            bits(create[11]),
        ],
        jump_speed: bits(&header.factors.jump),
        subscribed_ms,
    });
    assert_eq!(desc.user_data.to_string(), f[1]);
    assert_eq!(create[1], f[1], "create user data");
    assert_eq!(vec3(desc.position), owned3([f[2], f[3], f[4]]));
    assert_eq!(vec4(desc.orientation), owned4([f[5], f[6], f[7], f[8]]));
    assert_eq!(hex(desc.half_height), f[9]);
    assert_eq!(hex(desc.radius), f[10]);
    if desc.radius.is_finite() {
        assert!(desc.radius >= 0.2_f32);
    }
    assert_eq!(hex(desc.mass), f[11]);
    assert_eq!(hex(desc.jump_speed), f[12]);
    assert_eq!(bit(desc.wants_contact_events), f[13]);
    assert_eq!(hex(desc.friction), f[14]);
    assert_eq!(hex(desc.max_slope_angle), f[15]);
    assert_eq!(hex(desc.step_height), f[16]);
    assert_eq!(hex(desc.push_strength), f[17]);
    let marker = marker_body_user_data(desc.user_data);
    assert_eq!(marker.to_string(), f[18], "marker user data");
    assert_eq!(marker, desc.user_data);
}

fn check_move(f: &[&str], header: &Header) {
    assert_eq!(f.len(), 6, "{f:?}");
    let velocity = desired_velocity(Intent {
        target: [0.0, 0.0, 0.0],
        flying: false,
        always_run: false,
        factors: factors_of(header),
    });
    assert_eq!(vec3(velocity), owned3([f[1], f[2], f[3]]));
    assert_eq!(f[4], bit(movement_sends_latch(false)));
    assert_eq!(f[5], "0");
}

fn check_intent(f: &[&str]) {
    assert_eq!(f.len(), 18, "{f:?}");
    let intent = Intent {
        flying: flag(f[1]),
        always_run: flag(f[2]),
        target: [bits(f[3]), bits(f[4]), bits(f[5])],
        factors: SpeedFactors {
            walk: bits(f[6]),
            run: bits(f[7]),
            fly: bits(f[8]),
            fly_down: bits(f[9]),
        },
    };
    let velocity = desired_velocity(intent);
    assert_eq!(
        vec3(velocity),
        owned3([f[10], f[11], f[12]]),
        "movement {f:?}"
    );
    assert_eq!(
        vec3(velocity),
        owned3([f[13], f[14], f[15]]),
        "desired {f:?}"
    );
    assert_eq!(f[16], "0");
    assert_eq!(bit(intent.flying), f[17]);
    if !intent.flying {
        assert_eq!(hex(velocity[2]), f[5], "walk and run leave Z unscaled");
    }
}

fn check_jump(f: &[&str]) {
    assert_eq!(f.len(), 4, "{f:?}");
    assert!(jump_latch_on_press(bits(f[1])));
    assert_eq!(f[2], "1");
    assert_eq!(f[3], bit(movement_sends_latch(true)));
}

fn check_hold(f: &[&str]) {
    assert_eq!(f.len(), 3, "{f:?}");
    assert_eq!(f[1], "1");
    assert_eq!(f[2], bit(movement_sends_latch(true)));
}

fn check_drain(f: &[&str], header: &Header) {
    assert_eq!(f.len(), 17, "{f:?}");
    let before = flag(f[1]);
    let vz = bits(f[2]);
    let supported = flag(f[3]);
    let ground_valid = flag(f[4]);
    let terrain = flag(f[5]);
    let input = [bits(f[6]), bits(f[7]), bits(f[8])];
    assert_eq!(
        bit(jump_latch_after_drain(before, vz, supported)),
        f[9],
        "latch {f:?}"
    );
    let flags = ground_flags(supported, ground_valid, terrain);
    assert_eq!(bit(flags.colliding), f[10]);
    assert_eq!(bit(flags.ground), f[11]);
    assert_eq!(bit(flags.object), f[12]);
    let clamped = keep_in_region(input, header.region[0], header.region[1], false, true);
    assert_eq!(
        vec3(clamped.position),
        owned3([f[13], f[14], f[15]]),
        "position {f:?}"
    );
    assert_eq!(bit(clamped.wrote_transform), f[16], "transform {f:?}");
}

fn check_size(f: &[&str], shape: &mut ShapeState) {
    assert_eq!(f.len(), 11, "{f:?}");
    let next = [bits(f[1]), bits(f[2]), bits(f[3])];
    let called = shape.set_size(next);
    assert_eq!(bit(called), f[4], "called {f:?}");
    assert_eq!(hex(shape.mass), f[5]);
    assert_eq!(hex(shape.half_height), f[6]);
    assert_eq!(hex(shape.radius), f[7]);
    assert_eq!(vec3(shape.size), owned3([f[8], f[9], f[10]]));
    assert!(shape.radius >= 0.2_f32);
}

fn check_buoyancy(f: &[&str], stored: f32) -> f32 {
    assert_eq!(f.len(), 4, "{f:?}");
    let (forwarded, next) = store_buoyancy(stored, bits(f[1]));
    assert_eq!(bit(forwarded), f[2], "buoyancy {f:?}");
    assert_eq!(hex(next), f[3]);
    next
}

fn check_subscribe(f: &[&str]) {
    assert_eq!(f.len(), 3, "{f:?}");
    let wanted = if f[1] == "-1" {
        contact_wanted_after_unsubscribe()
    } else {
        contact_wanted_after_subscribe(f[1].parse().unwrap())
    };
    assert_eq!(bit(wanted), f[2], "subscribe {f:?}");
}

fn check_keep(f: &[&str]) {
    assert_eq!(f.len(), 12, "{f:?}");
    let clamped = keep_in_region(
        [bits(f[5]), bits(f[6]), bits(f[7])],
        f[3].parse().unwrap(),
        f[4].parse().unwrap(),
        flag(f[1]),
        flag(f[2]),
    );
    assert_eq!(
        vec3(clamped.position),
        owned3([f[8], f[9], f[10]]),
        "keep {f:?}"
    );
    assert_eq!(bit(clamped.wrote_transform), f[11], "keep transform {f:?}");
}

fn replay_calls(calls: &str, result: &str) {
    let paired: Vec<&str> = result
        .lines()
        .filter(|line| {
            matches!(
                line.split_whitespace().next(),
                Some(
                    "capsule"
                        | "desc"
                        | "intent"
                        | "drain"
                        | "size"
                        | "buoyancy"
                        | "subscribe"
                        | "keep"
                )
            )
        })
        .collect();
    let mut index = 0usize;
    for line in calls.lines() {
        let call = fields(line);
        if call.first().copied() == Some("format") {
            continue;
        }
        assert!(index < paired.len(), "no result for {line}");
        let got = fields(paired[index]);
        match call.first().copied() {
            Some("capsule") => {
                assert_eq!(got[0], "capsule", "{line}");
                assert_eq!(&got[2..5], &call[1..4], "{line}");
            }
            Some("create") => {
                assert_eq!(call.len(), 13, "{line}");
                assert_eq!(got[0], "desc", "{line}");
                assert_eq!(got[1], call[1], "user {line}");
                assert_eq!(&got[2..5], &call[2..5], "position {line}");
                assert_eq!(&got[5..9], &call[8..12], "orientation {line}");
                assert_eq!(got[18], call[1], "marker user data {line}");
            }
            Some("intent" | "drain" | "size" | "buoyancy" | "subscribe" | "keep") => {
                assert_eq!(got[0], call[0], "{line}");
                assert!(got.len() >= call.len(), "{line}");
                assert_eq!(&got[..call.len()], call.as_slice(), "{line}");
            }
            other => panic!("unexpected call {other:?}: {line}"),
        }
        index += 1;
    }
    assert_eq!(index, paired.len(), "a result row has no call");
    assert!(result.contains("FFC00000"), "NaN rows");
    assert!(result.contains("7F800000"), "infinity rows");
    assert!(calls.contains("intent 1 1 "), "flight with always-run");
    assert!(calls.contains("keep 1 "), "allow outside");
    assert!(calls.contains("keep 0 0 "), "invalid character");
    assert!(calls.contains("256 128"), "non-square region");
}

fn check_reject(f: &[&str]) {
    assert_eq!(f.len(), 6, "{f:?}");
    let finite = store_target([1.0, 2.0, 3.0]).unwrap();
    assert!(store_target([f32::NAN, 2.0, 3.0]).is_none());
    assert_eq!(vec3(finite), owned3([f[1], f[2], f[3]]));
    assert_eq!(f[4], f[5], "a rejected target adds no movement");
}

struct Header {
    region: [f32; 2],
    edge: String,
    default_mass: String,
    default_size: [String; 3],
    factors: Factors,
}

struct Factors {
    walk: String,
    run: String,
    fly: String,
    fly_down: String,
    jump: String,
}

fn header(result: &str) -> Header {
    let region = fields(line_starting(result, "region "));
    let edge = fields(line_starting(result, "edge "));
    let mass = fields(line_starting(result, "default_mass "));
    let size = fields(line_starting(result, "default_size "));
    let factors = fields(line_starting(result, "factors "));
    assert_eq!(region.len(), 3);
    assert_eq!(factors.len(), 6);
    Header {
        region: [region[1].parse().unwrap(), region[2].parse().unwrap()],
        edge: edge[1].to_string(),
        default_mass: mass[1].to_string(),
        default_size: [
            size[1].to_string(),
            size[2].to_string(),
            size[3].to_string(),
        ],
        factors: Factors {
            walk: factors[1].to_string(),
            run: factors[2].to_string(),
            fly: factors[3].to_string(),
            fly_down: factors[4].to_string(),
            jump: factors[5].to_string(),
        },
    }
}

fn factors_of(header: &Header) -> SpeedFactors {
    SpeedFactors {
        walk: bits(&header.factors.walk),
        run: bits(&header.factors.run),
        fly: bits(&header.factors.fly),
        fly_down: bits(&header.factors.fly_down),
    }
}

fn line_starting<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing {prefix}"))
}

fn fields(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn config_hex(config: &str, key: &str) -> String {
    let prefix = format!("{key} ");
    line_starting(config, &prefix)
        .strip_prefix(&prefix)
        .unwrap()
        .to_string()
}

fn bits(text: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(text, 16).unwrap())
}

fn parse3(hexes: &[String; 3]) -> [f32; 3] {
    [bits(&hexes[0]), bits(&hexes[1]), bits(&hexes[2])]
}

fn hex(value: f32) -> String {
    format!("{:08X}", value.to_bits())
}

fn vec3(value: [f32; 3]) -> [String; 3] {
    [hex(value[0]), hex(value[1]), hex(value[2])]
}

fn owned3(hexes: [&str; 3]) -> [String; 3] {
    [
        hexes[0].to_string(),
        hexes[1].to_string(),
        hexes[2].to_string(),
    ]
}

fn vec4(value: [f32; 4]) -> [String; 4] {
    [hex(value[0]), hex(value[1]), hex(value[2]), hex(value[3])]
}

fn owned4(hexes: [&str; 4]) -> [String; 4] {
    [
        hexes[0].to_string(),
        hexes[1].to_string(),
        hexes[2].to_string(),
        hexes[3].to_string(),
    ]
}

fn bit(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn flag(text: &str) -> bool {
    match text {
        "1" => true,
        "0" => false,
        other => panic!("not a bit: {other}"),
    }
}
