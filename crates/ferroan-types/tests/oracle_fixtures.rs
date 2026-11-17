// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every `family = bytes` fixture from the OpenMetaverse oracle.
//!
//! The file bytes are what libopenmetaverse wrote. Decoding them must produce
//! the manifest value. Encoding the value the oracle encoded must produce the
//! file again, including `vector3-clamped-outside`, which does not round-trip
//! through the reconstructed floats.

use std::fs;
use std::path::PathBuf;

use ferroan_types::{
    Color4, DVec3, ExtraParams, FlexibleData, LightData, LightImage, PERM_ALL, PERM_ALL_AND_EXPORT,
    PERM_ALL_AND_EXPORT_NO_MOD, PERM_ALL_EFFECTIVE, PERM_COPY, PERM_DAMAGE, PERM_EXPORT,
    PERM_FOLDED_COPY, PERM_FOLDED_EXPORT, PERM_FOLDED_MASK, PERM_FOLDED_MODIFY,
    PERM_FOLDED_TRANSFER, PERM_FOLDING_SHIFT, PERM_MODIFY, PERM_MOVE, PERM_NONE, PERM_SLAM,
    PERM_TRANSFER, PERM_UNFOLDED_MASK, Permissions, Quat, SculptData, ShapeParams, TextureEntry,
    TextureFaceSet, TextureFaceView, Vec2, Vec3, Vec4, apply_folded_permissions,
    apply_no_mod_folded_permissions, fix_and_fold_permissions, float_to_u16_bounds,
    float_to_u16_range, float_to_unit_u16, read_color4, read_color4_floats, read_dvec3,
    read_quaternion_full, read_quaternion_packed, read_quaternion_shorts, read_u16_value,
    read_uuid, read_vec2, read_vec3, read_vec3_clamped, read_vec4, swap_nibbles, u16_to_float,
    write_color4, write_color4_floats, write_dvec3, write_quaternion_full,
    write_quaternion_get_bytes, write_quaternion_shorts, write_quaternion_to_bytes, write_u16,
    write_uuid, write_vec2, write_vec3, write_vec3_clamped, write_vec4,
};
use serde_json::Value;
use uuid::Uuid;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle")
}

fn hex_f32(text: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(text, 16).expect("f32 hex"))
}

fn bits_of(values: &[f32]) -> Vec<String> {
    values
        .iter()
        .map(|v| format!("{:08x}", v.to_bits()))
        .collect()
}

fn bits_of_f64(values: &[f64]) -> Vec<String> {
    values
        .iter()
        .map(|v| format!("{:016x}", v.to_bits()))
        .collect()
}

fn expect_bits(got: &[String], node: &Value, id: &str) {
    let expect: Vec<&str> = node["bits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(got, &expect, "{id}");
}

fn source_quat(name: &str) -> Quat {
    match name {
        "identity" => Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
        "tilt" => Quat::from_xyzw(0.1, -0.2, 0.3, 0.9),
        "neg-w" => Quat::from_xyzw(0.0, 0.70710677, 0.0, -0.70710677),
        other => panic!("unknown quaternion fixture {other}"),
    }
}

fn quat_kind(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix("bytes/quaternion-")?;
    for kind in ["getbytes", "tobytes", "shorts", "full"] {
        if let Some(name) = rest.strip_prefix(kind).and_then(|s| s.strip_prefix('-')) {
            return Some((kind, name));
        }
    }
    None
}

fn fill(n: usize, write: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut buf = vec![0; n];
    write(&mut buf);
    buf
}

#[test]
fn every_byte_fixture_matches_openmetaverse() {
    let root = fixtures();
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    let mut seen = 0u32;

    for case in cases {
        if case["family"] != "bytes" {
            continue;
        }
        seen += 1;
        let id = case["id"].as_str().unwrap();
        let format = case["format"].as_str().unwrap();
        let file = fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let decoded = &case["decoded"];
        let round_trip = case["round_trip"].as_bool().unwrap();

        match format {
            "uuid" => {
                let id_value = read_uuid(&file).unwrap();
                assert_eq!(
                    id_value.to_string(),
                    decoded["value"].as_str().unwrap(),
                    "{id}"
                );
                let again = fill(16, |buf| write_uuid(&id_value, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "vector2" => {
                let v = read_vec2(&file).unwrap();
                expect_bits(&bits_of(&[v.x, v.y]), decoded, id);
                let again = fill(8, |buf| write_vec2(v, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "vector3" => {
                let v = read_vec3(&file).unwrap();
                expect_bits(&bits_of(&[v.x, v.y, v.z]), decoded, id);
                let again = fill(12, |buf| write_vec3(v, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "vector3d" => {
                let v = read_dvec3(&file).unwrap();
                expect_bits(&bits_of_f64(&[v.x, v.y, v.z]), decoded, id);
                let again = fill(24, |buf| write_dvec3(v, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "vector4" => {
                let v = read_vec4(&file).unwrap();
                expect_bits(&bits_of(&[v.x, v.y, v.z, v.w]), decoded, id);
                let again = fill(16, |buf| write_vec4(v, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "vector3-clamped" => {
                let raw = read_vec3_clamped(&file).unwrap();
                assert_eq!(i64::from(raw[0]), decoded["x"].as_i64().unwrap(), "{id}");
                assert_eq!(i64::from(raw[1]), decoded["y"].as_i64().unwrap(), "{id}");
                assert_eq!(i64::from(raw[2]), decoded["z"].as_i64().unwrap(), "{id}");
                let range = hex_f32(case["params"]["range_bits"].as_str().unwrap());
                let floats = [
                    u16_to_float(raw[0], -range, range),
                    u16_to_float(raw[1], -range, range),
                    u16_to_float(raw[2], -range, range),
                ];
                assert_eq!(
                    format!("{:08x}", floats[0].to_bits()),
                    decoded["x_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", floats[1].to_bits()),
                    decoded["y_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", floats[2].to_bits()),
                    decoded["z_bits"].as_str().unwrap(),
                    "{id}"
                );
                if round_trip {
                    let again = fill(6, |buf| {
                        write_vec3_clamped(Vec3::new(floats[0], floats[1], floats[2]), range, buf)
                            .unwrap()
                    });
                    assert_eq!(again, file, "{id}");
                }
            }
            "quaternion-getbytes" | "quaternion-tobytes" => {
                let q = read_quaternion_packed(&file).unwrap();
                assert_eq!(
                    format!("{:08x}", q.x.to_bits()),
                    decoded["x_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", q.y.to_bits()),
                    decoded["y_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", q.z.to_bits()),
                    decoded["z_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", q.w.to_bits()),
                    decoded["w_bits"].as_str().unwrap(),
                    "{id}"
                );
                let (kind, name) = quat_kind(id).unwrap();
                let source = source_quat(name);
                let again = fill(12, |buf| match kind {
                    "getbytes" => write_quaternion_get_bytes(source, buf).unwrap(),
                    "tobytes" => write_quaternion_to_bytes(source, buf).unwrap(),
                    _ => unreachable!(),
                });
                assert_eq!(again, file, "{id}");
            }
            "quaternion-shorts" => {
                let raw = read_quaternion_shorts(&file).unwrap();
                assert_eq!(i64::from(raw[0]), decoded["x"].as_i64().unwrap(), "{id}");
                assert_eq!(i64::from(raw[1]), decoded["y"].as_i64().unwrap(), "{id}");
                assert_eq!(i64::from(raw[2]), decoded["z"].as_i64().unwrap(), "{id}");
                assert_eq!(i64::from(raw[3]), decoded["w"].as_i64().unwrap(), "{id}");
                let (_, name) = quat_kind(id).unwrap();
                let again = fill(8, |buf| {
                    write_quaternion_shorts(source_quat(name), buf).unwrap()
                });
                assert_eq!(again, file, "{id}");
            }
            "quaternion-full" => {
                let q = read_quaternion_full(&file).unwrap();
                expect_bits(&bits_of(&[q.x, q.y, q.z, q.w]), decoded, id);
                let (_, name) = quat_kind(id).unwrap();
                let again = fill(16, |buf| {
                    write_quaternion_full(source_quat(name), buf).unwrap()
                });
                assert_eq!(again, file, "{id}");
            }
            "color4" | "color4-inverted" => {
                let inverted = format == "color4-inverted";
                let color = read_color4(&file, inverted).unwrap();
                assert_eq!(
                    format!("{:08x}", color.r.to_bits()),
                    decoded["r_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", color.g.to_bits()),
                    decoded["g_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", color.b.to_bits()),
                    decoded["b_bits"].as_str().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    format!("{:08x}", color.a.to_bits()),
                    decoded["a_bits"].as_str().unwrap(),
                    "{id}"
                );
                let raw = decoded["raw"].as_array().unwrap();
                let bytes: Vec<u64> = file.iter().map(|b| u64::from(*b)).collect();
                let expect: Vec<u64> = raw.iter().map(|v| v.as_u64().unwrap()).collect();
                assert_eq!(bytes, expect, "{id}");
                let source = Color4::from_bytes(255, 128, 0, 64);
                let again = fill(4, |buf| write_color4(source, inverted, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "color4-floats" => {
                let color = read_color4_floats(&file).unwrap();
                expect_bits(&bits_of(&[color.r, color.g, color.b, color.a]), decoded, id);
                let source = Color4::from_floats(0.25, 0.5, 0.75, 1.0);
                let again = fill(16, |buf| write_color4_floats(source, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "float-u16" => {
                let bits = read_u16_value(&file).unwrap();
                assert_eq!(i64::from(bits), decoded["value"].as_i64().unwrap(), "{id}");
                let params = &case["params"];
                let mode = params["mode"].as_str().unwrap();
                let input = hex_f32(params["input_bits"].as_str().unwrap());
                let encoded = match mode {
                    "range" => {
                        let range = hex_f32(params["range_bits"].as_str().unwrap());
                        let back = u16_to_float(bits, -range, range);
                        assert_eq!(
                            format!("{:08x}", back.to_bits()),
                            decoded["float_bits"].as_str().unwrap(),
                            "{id}"
                        );
                        float_to_u16_range(input, range)
                    }
                    "bounds" => {
                        let lower = hex_f32(params["lower_bits"].as_str().unwrap());
                        let upper = hex_f32(params["upper_bits"].as_str().unwrap());
                        let back = u16_to_float(bits, lower, upper);
                        assert_eq!(
                            format!("{:08x}", back.to_bits()),
                            decoded["float_bits"].as_str().unwrap(),
                            "{id}"
                        );
                        float_to_u16_bounds(input, lower, upper)
                    }
                    "unit" => float_to_unit_u16(input),
                    other => panic!("{id}: unknown mode {other}"),
                };
                let again = fill(2, |buf| write_u16(encoded, buf).unwrap());
                assert_eq!(again, file, "{id}");
            }
            "texture-entry" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_texture(&file, decoded, params, round_trip, id);
            }
            "extra-params" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_extra(&file, decoded, params, round_trip, id);
            }
            "shape" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_shape(&file, decoded, params, round_trip, id);
            }
            "attachment-state" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_attachment(&file, decoded, params, id);
            }
            "permissions" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_permissions(&file, decoded, params, id);
            }
            "permission-fold" => {
                let params = case.get("params").unwrap_or(&Value::Null);
                check_fold(&file, decoded, params, id);
            }
            "permission-masks" => check_masks(&file, decoded, id),
            other => panic!("{id}: unknown byte format {other}"),
        }
    }

    assert!(seen > 50, "saw {seen} byte fixtures");
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn f32_node(node: &Value) -> f32 {
    hex_f32(node.as_str().expect("f32 hex"))
}

fn opt_f32(node: &Value, key: &str) -> Option<f32> {
    node.get(key).map(f32_node)
}

fn opt_uuid(node: &Value, key: &str) -> Option<Uuid> {
    node.get(key)
        .and_then(|value| value.as_str())
        .map(|text| text.parse().expect("uuid"))
}

fn texture_set(node: &Value) -> TextureFaceSet {
    let rgba = node.get("r_bits").map(|_| {
        Color4::from_floats(
            f32_node(&node["r_bits"]),
            f32_node(&node["g_bits"]),
            f32_node(&node["b_bits"]),
            f32_node(&node["a_bits"]),
        )
    });
    TextureFaceSet {
        texture_id: opt_uuid(node, "texture_id"),
        rgba,
        repeat_u: opt_f32(node, "repeat_u_bits"),
        repeat_v: opt_f32(node, "repeat_v_bits"),
        offset_u: opt_f32(node, "offset_u_bits"),
        offset_v: opt_f32(node, "offset_v_bits"),
        rotation: opt_f32(node, "rotation_bits"),
        glow: opt_f32(node, "glow_bits"),
        bump: node.get("bump").and_then(|v| v.as_u64()).map(|n| n as u8),
        shiny: node.get("shiny").and_then(|v| v.as_u64()).map(|n| n as u8),
        fullbright: node.get("fullbright").and_then(|v| v.as_bool()),
        media_flags: node.get("media").and_then(|v| v.as_bool()),
        mapping: node
            .get("mapping")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8),
        material_id: opt_uuid(node, "material_id"),
    }
}

fn assert_view(view: &TextureFaceView, node: &Value, id: &str) {
    assert_eq!(
        view.texture_id.to_string(),
        node["texture_id"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        view.material_id.to_string(),
        node["material_id"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.rgba.r.to_bits()),
        node["r_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.rgba.g.to_bits()),
        node["g_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.rgba.b.to_bits()),
        node["b_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.rgba.a.to_bits()),
        node["a_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.repeat_u.to_bits()),
        node["repeat_u_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.repeat_v.to_bits()),
        node["repeat_v_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.offset_u.to_bits()),
        node["offset_u_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.offset_v.to_bits()),
        node["offset_v_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.rotation.to_bits()),
        node["rotation_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        format!("{:08x}", view.glow.to_bits()),
        node["glow_bits"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(i64::from(view.bump), node["bump"].as_i64().unwrap(), "{id}");
    assert_eq!(
        i64::from(view.shiny),
        node["shiny"].as_i64().unwrap(),
        "{id}"
    );
    assert_eq!(
        view.fullbright,
        node["fullbright"].as_bool().unwrap(),
        "{id}"
    );
    assert_eq!(view.media_flags, node["media"].as_bool().unwrap(), "{id}");
    assert_eq!(
        i64::from(view.mapping),
        node["mapping"].as_i64().unwrap(),
        "{id}"
    );
}

fn check_texture(file: &[u8], decoded: &Value, params: &Value, round_trip: bool, id: &str) {
    let parsed = TextureEntry::from_bytes(file).unwrap_or_else(|err| panic!("{id}: {err}"));
    match parsed.default_view() {
        None => assert!(decoded["default"].is_null(), "{id}"),
        Some(view) => assert_view(&view, &decoded["default"], id),
    }
    let faces = decoded["faces"].as_array().unwrap();
    let mut present = [false; 45];
    for face in faces {
        let index = usize::try_from(face["index"].as_u64().unwrap()).unwrap();
        present[index] = true;
        let view = parsed
            .face_view(index)
            .unwrap_or_else(|| panic!("{id}: missing face {index}"));
        assert_view(&view, face, id);
    }
    for (index, was_present) in present.iter().enumerate() {
        if !was_present {
            assert!(
                parsed.face_view(index).is_none(),
                "{id}: unexpected face {index}"
            );
        }
    }
    let again = parsed.to_bytes();
    assert_eq!(
        hex_of(&again),
        decoded["again_hex"].as_str().unwrap(),
        "{id}"
    );
    if round_trip {
        assert_eq!(again, file, "{id}");
    }
    if params.get("build").and_then(|value| value.as_bool()) == Some(true) {
        let mut built = TextureEntry::new(params["texture_id"].as_str().unwrap().parse().unwrap());
        if let Some(defaults) = params.get("default") {
            built.apply_default(&texture_set(defaults));
        }
        if let Some(faces) = params.get("faces").and_then(|value| value.as_array()) {
            for face in faces {
                let index = usize::try_from(face["index"].as_u64().unwrap()).unwrap();
                built.apply_face(index, &texture_set(face)).unwrap();
            }
        }
        let max_faces = usize::try_from(params["max_faces"].as_u64().unwrap()).unwrap();
        assert_eq!(built.to_bytes_max(max_faces), file, "{id} source");
    }
}

fn vec3_bits(node: &Value) -> Vec3 {
    let bits = node.as_array().unwrap();
    Vec3::new(f32_node(&bits[0]), f32_node(&bits[1]), f32_node(&bits[2]))
}

fn assert_f32(got: f32, node: &Value, id: &str) {
    assert_eq!(
        format!("{:08x}", got.to_bits()),
        node.as_str().unwrap(),
        "{id}"
    );
}

fn check_extra(file: &[u8], decoded: &Value, params: &Value, round_trip: bool, id: &str) {
    let parsed = ExtraParams::from_bytes(file).unwrap_or_else(|err| panic!("{id}: {err}"));
    assert_flex(parsed.flexible, &decoded["flexible"], id);
    assert_light(parsed.light, &decoded["light"], id);
    assert_image(parsed.light_image, &decoded["light_image"], id);
    assert_sculpt(parsed.sculpt, &decoded["sculpt"], id);
    let again = parsed.to_bytes();
    assert_eq!(
        hex_of(&again),
        decoded["again_hex"].as_str().unwrap(),
        "{id}"
    );
    if round_trip {
        assert_eq!(again, file, "{id}");
    }
    if params.get("build").and_then(|value| value.as_bool()) == Some(true) {
        assert_eq!(extra_from_params(params).to_bytes(), file, "{id} source");
    }
}

fn assert_flex(got: Option<FlexibleData>, node: &Value, id: &str) {
    if node.is_null() {
        assert!(got.is_none(), "{id}");
        return;
    }
    let got = got.expect(id);
    assert_eq!(
        i64::from(got.softness),
        node["softness"].as_i64().unwrap(),
        "{id}"
    );
    assert_f32(got.tension, &node["tension_bits"], id);
    assert_f32(got.drag, &node["drag_bits"], id);
    assert_f32(got.gravity, &node["gravity_bits"], id);
    assert_f32(got.wind, &node["wind_bits"], id);
    assert_eq!(got.force, vec3_bits(&node["force_bits"]), "{id}");
}

fn assert_light(got: Option<LightData>, node: &Value, id: &str) {
    if node.is_null() {
        assert!(got.is_none(), "{id}");
        return;
    }
    let got = got.expect(id);
    assert_f32(got.color.r, &node["r_bits"], id);
    assert_f32(got.color.g, &node["g_bits"], id);
    assert_f32(got.color.b, &node["b_bits"], id);
    assert_f32(got.color.a, &node["a_bits"], id);
    assert_f32(got.intensity, &node["intensity_bits"], id);
    assert_f32(got.radius, &node["radius_bits"], id);
    assert_f32(got.cutoff, &node["cutoff_bits"], id);
    assert_f32(got.falloff, &node["falloff_bits"], id);
}

fn assert_image(got: Option<LightImage>, node: &Value, id: &str) {
    if node.is_null() {
        assert!(got.is_none(), "{id}");
        return;
    }
    let got = got.expect(id);
    assert_eq!(
        got.texture.to_string(),
        node["texture_id"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(got.params, vec3_bits(&node["params_bits"]), "{id}");
}

fn assert_sculpt(got: Option<SculptData>, node: &Value, id: &str) {
    if node.is_null() {
        assert!(got.is_none(), "{id}");
        return;
    }
    let got = got.expect(id);
    assert_eq!(
        got.texture.to_string(),
        node["texture_id"].as_str().unwrap(),
        "{id}"
    );
    assert_eq!(
        i64::from(got.type_byte),
        node["type_byte"].as_i64().unwrap(),
        "{id}"
    );
    assert_eq!(
        i64::from(got.sculpt_type()),
        node["type"].as_i64().unwrap(),
        "{id}"
    );
    assert_eq!(got.invert(), node["invert"].as_bool().unwrap(), "{id}");
    assert_eq!(got.mirror(), node["mirror"].as_bool().unwrap(), "{id}");
}

fn extra_from_params(params: &Value) -> ExtraParams {
    ExtraParams {
        flexible: params.get("flexible").map(|node| FlexibleData {
            softness: node["softness"].as_i64().unwrap() as i32,
            tension: f32_node(&node["tension_bits"]),
            drag: f32_node(&node["drag_bits"]),
            gravity: f32_node(&node["gravity_bits"]),
            wind: f32_node(&node["wind_bits"]),
            force: vec3_bits(&node["force_bits"]),
        }),
        light: params.get("light").map(|node| LightData {
            color: Color4::from_floats(
                f32_node(&node["r_bits"]),
                f32_node(&node["g_bits"]),
                f32_node(&node["b_bits"]),
                f32_node(&node["a_bits"]),
            ),
            intensity: f32_node(&node["intensity_bits"]),
            radius: f32_node(&node["radius_bits"]),
            cutoff: f32_node(&node["cutoff_bits"]),
            falloff: f32_node(&node["falloff_bits"]),
        }),
        light_image: params.get("light_image").map(|node| LightImage {
            texture: node["texture_id"].as_str().unwrap().parse().unwrap(),
            params: vec3_bits(&node["params_bits"]),
        }),
        sculpt: params.get("sculpt").map(|node| SculptData {
            texture: node["texture_id"].as_str().unwrap().parse().unwrap(),
            type_byte: node["type_byte"].as_u64().unwrap() as u8,
        }),
    }
}

fn shape_from(node: &Value) -> ShapeParams {
    ShapeParams {
        path_curve: node["path_curve"].as_u64().unwrap() as u8,
        profile_curve: node["profile_curve"].as_u64().unwrap() as u8,
        path_begin: f32_node(&node["path_begin_bits"]),
        path_end: f32_node(&node["path_end_bits"]),
        path_scale_x: f32_node(&node["path_scale_x_bits"]),
        path_scale_y: f32_node(&node["path_scale_y_bits"]),
        path_shear_x: f32_node(&node["path_shear_x_bits"]),
        path_shear_y: f32_node(&node["path_shear_y_bits"]),
        path_twist: f32_node(&node["path_twist_bits"]),
        path_twist_begin: f32_node(&node["path_twist_begin_bits"]),
        path_radius_offset: f32_node(&node["path_radius_offset_bits"]),
        path_taper_x: f32_node(&node["path_taper_x_bits"]),
        path_taper_y: f32_node(&node["path_taper_y_bits"]),
        path_revolutions: f32_node(&node["path_revolutions_bits"]),
        path_skew: f32_node(&node["path_skew_bits"]),
        profile_begin: f32_node(&node["profile_begin_bits"]),
        profile_end: f32_node(&node["profile_end_bits"]),
        profile_hollow: f32_node(&node["profile_hollow_bits"]),
    }
}

fn assert_shape(got: &ShapeParams, node: &Value, id: &str) {
    assert_eq!(
        u64::from(got.path_curve),
        node["path_curve"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(got.profile_curve),
        node["profile_curve"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(ferroan_types::profile_curve(got.profile_curve)),
        node["profile_shape"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(ferroan_types::profile_hole(got.profile_curve)),
        node["profile_hole"].as_u64().unwrap(),
        "{id}"
    );
    assert_f32(got.path_begin, &node["path_begin_bits"], id);
    assert_f32(got.path_end, &node["path_end_bits"], id);
    assert_f32(got.path_scale_x, &node["path_scale_x_bits"], id);
    assert_f32(got.path_scale_y, &node["path_scale_y_bits"], id);
    assert_f32(got.path_shear_x, &node["path_shear_x_bits"], id);
    assert_f32(got.path_shear_y, &node["path_shear_y_bits"], id);
    assert_f32(got.path_twist, &node["path_twist_bits"], id);
    assert_f32(got.path_twist_begin, &node["path_twist_begin_bits"], id);
    assert_f32(got.path_radius_offset, &node["path_radius_offset_bits"], id);
    assert_f32(got.path_taper_x, &node["path_taper_x_bits"], id);
    assert_f32(got.path_taper_y, &node["path_taper_y_bits"], id);
    assert_f32(got.path_revolutions, &node["path_revolutions_bits"], id);
    assert_f32(got.path_skew, &node["path_skew_bits"], id);
    assert_f32(got.profile_begin, &node["profile_begin_bits"], id);
    assert_f32(got.profile_end, &node["profile_end_bits"], id);
    assert_f32(got.profile_hollow, &node["profile_hollow_bits"], id);
}

fn check_shape(file: &[u8], decoded: &Value, params: &Value, round_trip: bool, id: &str) {
    let parsed = ShapeParams::from_bytes(file).unwrap_or_else(|err| panic!("{id}: {err}"));
    assert_shape(&parsed, decoded, id);
    let again = parsed.to_bytes();
    assert_eq!(
        hex_of(&again),
        decoded["again_hex"].as_str().unwrap(),
        "{id}"
    );
    if round_trip {
        assert_eq!(again.as_slice(), file, "{id}");
    }
    if params.get("build").and_then(|value| value.as_bool()) == Some(true) {
        assert_eq!(
            shape_from(params).to_bytes().as_slice(),
            file,
            "{id} source"
        );
    }
}

fn check_attachment(file: &[u8], decoded: &Value, params: &Value, id: &str) {
    assert_eq!(file.len(), 1, "{id}");
    assert_eq!(
        u64::from(file[0]),
        decoded["state"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(swap_nibbles(file[0])),
        decoded["attachment"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(swap_nibbles(swap_nibbles(file[0])), file[0], "{id}");
    if params.get("build").and_then(|value| value.as_bool()) == Some(true) {
        let point = params["attachment"].as_u64().unwrap() as u8;
        assert_eq!(swap_nibbles(point), file[0], "{id} source");
    }
}

fn check_permissions(file: &[u8], decoded: &Value, params: &Value, id: &str) {
    let parsed = Permissions::from_bytes(file).unwrap_or_else(|err| panic!("{id}: {err}"));
    assert_eq!(
        u64::from(parsed.base),
        decoded["base"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(parsed.owner),
        decoded["owner"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(parsed.group),
        decoded["group"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(parsed.everyone),
        decoded["everyone"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(
        u64::from(parsed.next_owner),
        decoded["next_owner"].as_u64().unwrap(),
        "{id}"
    );
    assert_eq!(parsed.to_bytes().as_slice(), file, "{id}");
    if params.get("build").and_then(|value| value.as_bool()) == Some(true) {
        let built = Permissions {
            base: params["base"].as_u64().unwrap() as u32,
            owner: params["owner"].as_u64().unwrap() as u32,
            group: params["group"].as_u64().unwrap() as u32,
            everyone: params["everyone"].as_u64().unwrap() as u32,
            next_owner: params["next_owner"].as_u64().unwrap() as u32,
        };
        assert_eq!(built.to_bytes().as_slice(), file, "{id} source");
    }
}

fn check_fold(file: &[u8], decoded: &Value, params: &Value, id: &str) {
    assert_eq!(file.len(), 4, "{id}");
    let result = u32::from_le_bytes(file.try_into().unwrap());
    assert_eq!(
        u64::from(result),
        decoded["result"].as_u64().unwrap(),
        "{id}"
    );
    let source = params["source"].as_u64().unwrap() as u32;
    let target = params["target"].as_u64().unwrap() as u32;
    let got = match params["op"].as_str().unwrap() {
        "apply" => apply_folded_permissions(source, target),
        "nomod" => apply_no_mod_folded_permissions(source, target),
        "fix" => fix_and_fold_permissions(target),
        other => panic!("{id}: unknown fold {other}"),
    };
    assert_eq!(got, result, "{id}");
}

fn check_masks(file: &[u8], decoded: &Value, id: &str) {
    const MASKS: [u32; 19] = [
        PERM_NONE,
        PERM_FOLDED_TRANSFER,
        PERM_FOLDED_MODIFY,
        PERM_FOLDED_COPY,
        PERM_FOLDED_EXPORT,
        PERM_SLAM,
        PERM_FOLDED_MASK,
        PERM_FOLDING_SHIFT,
        PERM_TRANSFER,
        PERM_MODIFY,
        PERM_COPY,
        PERM_EXPORT,
        PERM_MOVE,
        PERM_DAMAGE,
        PERM_ALL,
        PERM_ALL_AND_EXPORT,
        PERM_ALL_AND_EXPORT_NO_MOD,
        PERM_ALL_EFFECTIVE,
        PERM_UNFOLDED_MASK,
    ];
    let mut packed = Vec::with_capacity(MASKS.len() * 4);
    for mask in MASKS {
        packed.extend_from_slice(&mask.to_le_bytes());
    }
    assert_eq!(packed, file, "{id}");
    let values = decoded["values"].as_array().unwrap();
    assert_eq!(values.len(), MASKS.len(), "{id}");
    for (mask, value) in MASKS.iter().zip(values) {
        assert_eq!(u64::from(*mask), value.as_u64().unwrap(), "{id}");
    }
}

#[test]
fn known_vector_literals_match_the_files() {
    let root = fixtures();
    let check = |name: &str, bytes: &[u8]| {
        let file = fs::read(root.join(format!("bytes/{name}.bin"))).unwrap();
        assert_eq!(bytes, file.as_slice(), "{name}");
    };
    check(
        "vector2",
        &fill(8, |buf| write_vec2(Vec2::new(1.5, -2.0), buf).unwrap()),
    );
    check(
        "vector3",
        &fill(12, |buf| write_vec3(Vec3::new(1.0, 2.0, 3.0), buf).unwrap()),
    );
    check(
        "vector3-tenth",
        &fill(12, |buf| {
            write_vec3(Vec3::new(0.1, -0.1, 0.1), buf).unwrap()
        }),
    );
    check(
        "vector3-neg-zero",
        &fill(12, |buf| {
            write_vec3(Vec3::new(f32::from_bits(0x8000_0000), 1.0, 2.0), buf).unwrap()
        }),
    );
    check(
        "vector3d",
        &fill(24, |buf| {
            write_dvec3(DVec3::new(1.5, -2.25, 4.0), buf).unwrap()
        }),
    );
    check(
        "vector4",
        &fill(16, |buf| {
            write_vec4(Vec4::new(1.0, 2.0, 3.0, 4.0), buf).unwrap()
        }),
    );
    check(
        "vector3-clamped-inside",
        &fill(6, |buf| {
            write_vec3_clamped(Vec3::new(0.25, -0.5, 0.125), 1.0, buf).unwrap()
        }),
    );
    check(
        "vector3-clamped-outside",
        &fill(6, |buf| {
            write_vec3_clamped(Vec3::new(2.0, -3.0, 0.5), 1.0, buf).unwrap()
        }),
    );
    check(
        "uuid-ordered",
        &fill(16, |buf| {
            write_uuid(
                &"00112233-4455-6677-8899-aabbccddeeff".parse().unwrap(),
                buf,
            )
            .unwrap();
        }),
    );
}
