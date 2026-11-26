// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compares [`crate::load`] with the field bits and warnings
//! `tools/jolt-config-oracle` wrote from the pinned `JoltConfig.FromConfig`.
//! The `shipped` case reads the pinned Tranquillity tree when it is available
//! and is skipped otherwise.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ferroan_config::{Config, RegionLoad};

use crate::jolt_config::csharp_g;
use crate::{
    BasicForces, JoltConfig, JoltLoad, MaterialCombine, VehicleBehaviour, VehiclePresetSet, load,
    startup_physics_warning,
};

const FIELDS: &[&str] = &[
    "AllowUnrecordedNative",
    "AvatarEnhancedEdgeRemoval",
    "AvatarFlyDownSpeedFactor",
    "AvatarFlySpeedFactor",
    "AvatarJumpSpeed",
    "AvatarObjectPushMaxForce",
    "AvatarObjectPushPerMomentum",
    "AvatarPushMaxSpeed",
    "AvatarPushRecovery",
    "AvatarRunSpeedFactor",
    "AvatarWalkSpeedFactor",
    "BasicForcesBehaviour",
    "BodyEnhancedEdgeRemoval",
    "BodyMaxAngularSpeed",
    "BodyMaxLinearSpeed",
    "BodyUpdateBufferMax",
    "CameraRayBudgetMs",
    "CapacityLogIntervalSeconds",
    "CharacterUpdateBufferMax",
    "CollisionSteps",
    "ContactBufferMax",
    "DeterministicMode",
    "FrictionCombine",
    "Gravity",
    "HonourConvexMesh",
    "JobPoolFairHandoff",
    "JobPools",
    "KeyframedMotionCarriesRiders",
    "MaxBodies",
    "MaxBodyPairs",
    "MaxContactConstraints",
    "MaxPhysicalComplexity",
    "MetricsLogIntervalSeconds",
    "MovedPrimsCarryRiders",
    "PhysicsSit",
    "PhysicsStepCollisionSteps",
    "PhysicsStepRate",
    "PhysicsStepRateSet",
    "PositionIterations",
    "PrimAngularDamping",
    "PrimLinearDamping",
    "RayCastBudgetMs",
    "RayCastMaxHits",
    "RayCastMaxTestedHits",
    "RayCastSimulatorBudgetMs",
    "RestitutionCombine",
    "ScaleCapsWithArea",
    "SledContactFriction",
    "SleepVelocityThreshold",
    "SpeculativeContactDistance",
    "TerrainFriction",
    "TerrainRestitution",
    "ThreadCount",
    "TwoSidedMeshCollision",
    "VehicleAngularLimiterFloor",
    "VehicleBehaviour",
    "VehicleContactFriction",
    "VehicleGroundGravityFactor",
    "VehicleMaxAngularSpeed",
    "VehicleMaxAttractTimescale",
    "VehicleMaxDecayTimescale",
    "VehicleMaxDecayTimescaleSet",
    "VehicleMaxHoverHeight",
    "VehicleMaxHoverTimescale",
    "VehicleMaxLinearSpeed",
    "VehicleMaxMotorOffset",
    "VehicleMaxTimescale",
    "VehicleMinHoverHeight",
    "VehicleMinTimescale",
    "VehicleMotorTargetOnUndrivenAxes",
    "VehiclePresets",
    "VehiclePresetsSet",
    "VehicleReferenceSpeed",
    "VehicleRestSpeed",
    "VehicleSledAssist",
    "VehicleSpikeMitigation",
    "VelocityIterations",
];

#[test]
fn jolt_config_matches_the_csharp_reader() {
    let mut sorted = FIELDS.to_vec();
    sorted.sort_unstable();
    assert_eq!(FIELDS, sorted.as_slice(), "field names are not ordinal");
    assert_eq!(FIELDS.len(), 77);

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-config");
    let manifest = std::fs::read_to_string(root.join("manifest.txt"))
        .expect("fixtures/jolt-config/manifest.txt");
    let mut cases = Vec::new();
    for line in manifest.lines() {
        if line.is_empty() || line.starts_with("format ") || line.starts_with("revision ") {
            continue;
        }
        cases.push(line.to_string());
    }
    assert!(
        cases.len() >= 30,
        "expected the oracle cases, got {}",
        cases.len()
    );
    let reference = reference_root();
    let mut skipped = Vec::new();
    for id in &cases {
        let case_dir = root.join(id);
        let kind = std::fs::read_to_string(case_dir.join("kind.txt"))
            .unwrap()
            .trim()
            .to_string();
        if kind == "region-ref" && reference.is_none() {
            skipped.push(id.clone());
            continue;
        }
        let config = read_config(&case_dir, &kind, reference.as_deref());
        let recorded = std::fs::read_to_string(case_dir.join("result.txt")).unwrap();
        let parsed = parse_result(&recorded);
        let store = WarnStore(Arc::new(Mutex::new(Vec::new())));
        let loaded = tracing::subscriber::with_default(store.clone(), || load(&config));
        let logged = store.0.lock().unwrap().clone();
        assert_case(id, &config, &loaded, &parsed, &logged);
    }
    if !skipped.is_empty() {
        eprintln!(
            "skipped Tranquillity Jolt config cases (no reference checkout): {}",
            skipped.join(", ")
        );
    }
}

fn assert_case(id: &str, config: &Config, loaded: &JoltLoad, parsed: &Parsed, logged: &[String]) {
    assert_eq!(
        format!("{:08X}", loaded.heartbeat_seconds.to_bits()),
        parsed.heartbeat,
        "{id} heartbeat"
    );
    match parsed.test_commands.as_str() {
        "true" => {
            assert!(loaded.test_commands, "{id}");
            assert!(loaded.test_commands_warning.is_none(), "{id}");
        }
        "false" => assert!(!loaded.test_commands, "{id}"),
        "threw" => {
            assert!(!loaded.test_commands, "{id}");
            assert!(loaded.test_commands_warning.is_some(), "{id}");
        }
        other => panic!("{id} test_commands {other}"),
    }
    assert_eq!(
        format!("{:08X}", loaded.effective_step_rate.to_bits()),
        parsed.effective,
        "{id} effective"
    );
    assert_eq!(
        preset_name(loaded.config.effective_presets()),
        parsed.effective_presets,
        "{id} presets"
    );
    assert_eq!(
        loaded.step_warning.as_deref().unwrap_or(""),
        parsed.step_warning,
        "{id} step warning"
    );
    assert_eq!(
        loaded.step_note.as_deref().unwrap_or(""),
        parsed.step_note,
        "{id} step note"
    );
    assert_eq!(loaded.warnings, parsed.warnings, "{id} warnings");
    let physics = config.get("Startup", "physics").unwrap_or("");
    assert_eq!(
        loaded.engine_warning,
        startup_physics_warning(physics),
        "{id} engine"
    );
    assert_eq!(
        parsed.fields.len(),
        FIELDS.len(),
        "{id} field count {}",
        parsed.fields.len()
    );
    for (expected_name, (name, payload)) in FIELDS.iter().zip(&parsed.fields) {
        assert_eq!(name, expected_name, "{id}");
        assert_eq!(payload, &field_payload(&loaded.config, name), "{id} {name}");
    }
    let mut expected_logs = Vec::new();
    if let Some(warning) = &loaded.engine_warning {
        expected_logs.push(warning.clone());
    }
    expected_logs.extend(loaded.warnings.iter().cloned());
    if let Some(warning) = &loaded.test_commands_warning {
        expected_logs.push(warning.clone());
    }
    if let Some(warning) = &loaded.step_warning {
        expected_logs.push(warning.clone());
    }
    assert_eq!(logged, expected_logs, "{id} log");
}

fn field_payload(config: &JoltConfig, name: &str) -> String {
    match name {
        "AllowUnrecordedNative" => flag(config.allow_unrecorded_native),
        "AvatarEnhancedEdgeRemoval" => flag(config.avatar_enhanced_edge_removal),
        "AvatarFlyDownSpeedFactor" => float(config.avatar_fly_down_speed_factor),
        "AvatarFlySpeedFactor" => float(config.avatar_fly_speed_factor),
        "AvatarJumpSpeed" => float(config.avatar_jump_speed),
        "AvatarObjectPushMaxForce" => float(config.avatar_object_push_max_force),
        "AvatarObjectPushPerMomentum" => float(config.avatar_object_push_per_momentum),
        "AvatarPushMaxSpeed" => float(config.avatar_push_max_speed),
        "AvatarPushRecovery" => float(config.avatar_push_recovery),
        "AvatarRunSpeedFactor" => float(config.avatar_run_speed_factor),
        "AvatarWalkSpeedFactor" => float(config.avatar_walk_speed_factor),
        "BasicForcesBehaviour" => enum_payload(basic_forces_name(config.basic_forces_behaviour)),
        "BodyEnhancedEdgeRemoval" => flag(config.body_enhanced_edge_removal),
        "BodyMaxAngularSpeed" => float(config.body_max_angular_speed),
        "BodyMaxLinearSpeed" => float(config.body_max_linear_speed),
        "BodyUpdateBufferMax" => integer(config.body_update_buffer_max),
        "CameraRayBudgetMs" => float(config.camera_ray_budget_ms),
        "CapacityLogIntervalSeconds" => float(config.capacity_log_interval_seconds),
        "CharacterUpdateBufferMax" => integer(config.character_update_buffer_max),
        "CollisionSteps" => integer(config.collision_steps),
        "ContactBufferMax" => integer(config.contact_buffer_max),
        "DeterministicMode" => flag(config.deterministic_mode),
        "FrictionCombine" => enum_payload(combine_name(config.friction_combine)),
        "Gravity" => float(config.gravity),
        "HonourConvexMesh" => flag(config.honour_convex_mesh),
        "JobPoolFairHandoff" => flag(config.job_pool_fair_handoff),
        "JobPools" => integer(config.job_pools),
        "KeyframedMotionCarriesRiders" => flag(config.keyframed_motion_carries_riders),
        "MaxBodies" => integer(config.max_bodies),
        "MaxBodyPairs" => integer(config.max_body_pairs),
        "MaxContactConstraints" => integer(config.max_contact_constraints),
        "MaxPhysicalComplexity" => integer(config.max_physical_complexity),
        "MetricsLogIntervalSeconds" => float(config.metrics_log_interval_seconds),
        "MovedPrimsCarryRiders" => flag(config.moved_prims_carry_riders),
        "PhysicsSit" => flag(config.physics_sit),
        "PhysicsStepCollisionSteps" => integer(config.physics_step_collision_steps),
        "PhysicsStepRate" => float(config.physics_step_rate),
        "PhysicsStepRateSet" => flag(config.physics_step_rate_set),
        "PositionIterations" => integer(config.position_iterations),
        "PrimAngularDamping" => float(config.prim_angular_damping),
        "PrimLinearDamping" => float(config.prim_linear_damping),
        "RayCastBudgetMs" => float(config.ray_cast_budget_ms),
        "RayCastMaxHits" => integer(config.ray_cast_max_hits),
        "RayCastMaxTestedHits" => integer(config.ray_cast_max_tested_hits),
        "RayCastSimulatorBudgetMs" => float(config.ray_cast_simulator_budget_ms),
        "RestitutionCombine" => enum_payload(combine_name(config.restitution_combine)),
        "ScaleCapsWithArea" => flag(config.scale_caps_with_area),
        "SledContactFriction" => float(config.sled_contact_friction),
        "SleepVelocityThreshold" => float(config.sleep_velocity_threshold),
        "SpeculativeContactDistance" => float(config.speculative_contact_distance),
        "TerrainFriction" => float(config.terrain_friction),
        "TerrainRestitution" => float(config.terrain_restitution),
        "ThreadCount" => integer(config.thread_count),
        "TwoSidedMeshCollision" => flag(config.two_sided_mesh_collision),
        "VehicleAngularLimiterFloor" => float(config.vehicle_angular_limiter_floor),
        "VehicleBehaviour" => enum_payload(behaviour_name(config.vehicle_behaviour)),
        "VehicleContactFriction" => match config.vehicle_contact_friction {
            Some(value) => format!("f32?\t{}", float_body(value)),
            None => "f32?\tnull".to_string(),
        },
        "VehicleGroundGravityFactor" => float(config.vehicle_ground_gravity_factor),
        "VehicleMaxAngularSpeed" => float(config.vehicle_max_angular_speed),
        "VehicleMaxAttractTimescale" => float(config.vehicle_max_attract_timescale),
        "VehicleMaxDecayTimescale" => float(config.vehicle_max_decay_timescale),
        "VehicleMaxDecayTimescaleSet" => flag(config.vehicle_max_decay_timescale_set),
        "VehicleMaxHoverHeight" => float(config.vehicle_max_hover_height),
        "VehicleMaxHoverTimescale" => float(config.vehicle_max_hover_timescale),
        "VehicleMaxLinearSpeed" => float(config.vehicle_max_linear_speed),
        "VehicleMaxMotorOffset" => float(config.vehicle_max_motor_offset),
        "VehicleMaxTimescale" => float(config.vehicle_max_timescale),
        "VehicleMinHoverHeight" => float(config.vehicle_min_hover_height),
        "VehicleMinTimescale" => float(config.vehicle_min_timescale),
        "VehicleMotorTargetOnUndrivenAxes" => flag(config.vehicle_motor_target_on_undriven_axes),
        "VehiclePresets" => enum_payload(preset_name(config.vehicle_presets)),
        "VehiclePresetsSet" => flag(config.vehicle_presets_set),
        "VehicleReferenceSpeed" => float(config.vehicle_reference_speed),
        "VehicleRestSpeed" => float(config.vehicle_rest_speed),
        "VehicleSledAssist" => float(config.vehicle_sled_assist),
        "VehicleSpikeMitigation" => match config.vehicle_spike_mitigation {
            Some(value) => format!("bool?\t{}", bool_token(value)),
            None => "bool?\tnull".to_string(),
        },
        "VelocityIterations" => integer(config.velocity_iterations),
        other => panic!("JoltConfig field {other} is not in the port"),
    }
}

fn flag(value: bool) -> String {
    format!("bool\t{}", bool_token(value))
}

fn bool_token(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn integer(value: i32) -> String {
    format!("i32\t{value}")
}

fn float(value: f32) -> String {
    format!("f32\t{}", float_body(value))
}

fn float_body(value: f32) -> String {
    format!("{:08X}\t{}", value.to_bits(), csharp_g(value))
}

fn enum_payload(name: &str) -> String {
    format!("enum\t{name}")
}

fn preset_name(value: VehiclePresetSet) -> &'static str {
    match value {
        VehiclePresetSet::Documented => "Documented",
        VehiclePresetSet::Legacy => "Legacy",
    }
}

fn behaviour_name(value: VehicleBehaviour) -> &'static str {
    match value {
        VehicleBehaviour::SecondLife => "SecondLife",
        VehicleBehaviour::InWorldz => "InWorldz",
    }
}

fn basic_forces_name(value: BasicForces) -> &'static str {
    match value {
        BasicForces::InWorldz => "InWorldz",
        BasicForces::SecondLife => "SecondLife",
    }
}

fn combine_name(value: MaterialCombine) -> &'static str {
    match value {
        MaterialCombine::Average => "Average",
        MaterialCombine::GeometricMean => "GeometricMean",
        MaterialCombine::Multiply => "Multiply",
        MaterialCombine::Min => "Min",
        MaterialCombine::Max => "Max",
    }
}

struct Parsed {
    heartbeat: String,
    test_commands: String,
    effective: String,
    effective_presets: String,
    step_warning: String,
    step_note: String,
    warnings: Vec<String>,
    fields: Vec<(String, String)>,
}

fn parse_result(text: &str) -> Parsed {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("format 1"));
    let mut parsed = Parsed {
        heartbeat: String::new(),
        test_commands: String::new(),
        effective: String::new(),
        effective_presets: String::new(),
        step_warning: String::new(),
        step_note: String::new(),
        warnings: Vec::new(),
        fields: Vec::new(),
    };
    for line in lines {
        if let Some(rest) = line.strip_prefix("heartbeat\t") {
            parsed.heartbeat = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("test_commands\t") {
            parsed.test_commands = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("effective\t") {
            parsed.effective = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("effective_presets\t") {
            parsed.effective_presets = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("step_warning\t") {
            parsed.step_warning = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("step_note\t") {
            parsed.step_note = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("warning\t") {
            parsed.warnings.push(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("field\t") {
            let (name, payload) = rest.split_once('\t').expect(line);
            parsed.fields.push((name.to_string(), payload.to_string()));
        } else {
            panic!("unexpected result line: {line}");
        }
    }
    parsed
}

fn read_config(case_dir: &Path, kind: &str, reference: Option<&Path>) -> Config {
    if kind == "parse" {
        let text = std::fs::read_to_string(case_dir.join("source.ini")).unwrap();
        return Config::parse(&text).unwrap();
    }
    assert_eq!(kind, "region-ref");
    let spec = std::fs::read_to_string(case_dir.join("load.txt")).unwrap();
    let mut appdata = None;
    let mut master = None;
    let mut files = Vec::new();
    for line in spec.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (field, rest) = line.split_once(' ').unwrap_or((line, ""));
        match field {
            "reference_appdata" => appdata = Some(rest.to_string()),
            "ini_master" => master = Some(rest.to_string()),
            "ini_file" => files.push(rest.to_string()),
            other => panic!("unknown load field {other}"),
        }
    }
    let config_dir = reference
        .expect("reference checkout")
        .join("OpenSim-Tranquillity/Source")
        .join(appdata.expect("reference_appdata"));
    let mut argv = Config::default();
    if let Some(master) = master {
        argv.set("Startup", "inimaster", &master);
    }
    for (index, file) in files.iter().enumerate() {
        let key = if index == 0 {
            "inifile".to_string()
        } else {
            format!("inifile{}", index + 1)
        };
        argv.set("Startup", &key, file);
    }
    RegionLoad {
        config_dir,
        argv,
        environment: BTreeMap::new(),
    }
    .load()
    .unwrap()
    .config
}

fn reference_root() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("FERROAN_REF")
        && !value.is_empty()
    {
        return Some(PathBuf::from(value));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let path = PathBuf::from(home).join("Git/reference/OpenSim-Tranquillity/Source");
    path.is_dir()
        .then_some(path.parent().unwrap().parent().unwrap().to_path_buf())
}

#[derive(Clone)]
struct WarnStore(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for WarnStore {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut message = String::new();
        event.record(&mut Message(&mut message));
        if let Ok(mut guard) = self.0.lock() {
            guard.push(message);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

struct Message<'a>(&'a mut String);

impl tracing::field::Visit for Message<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}
