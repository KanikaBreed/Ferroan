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
    Color4, DVec3, Quat, Vec2, Vec3, Vec4, float_to_u16_bounds, float_to_u16_range,
    float_to_unit_u16, read_color4, read_color4_floats, read_dvec3, read_quaternion_full,
    read_quaternion_packed, read_quaternion_shorts, read_u16_value, read_uuid, read_vec2,
    read_vec3, read_vec3_clamped, read_vec4, u16_to_float, write_color4, write_color4_floats,
    write_dvec3, write_quaternion_full, write_quaternion_get_bytes, write_quaternion_shorts,
    write_quaternion_to_bytes, write_u16, write_uuid, write_vec2, write_vec3, write_vec3_clamped,
    write_vec4,
};
use serde_json::Value;

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
            other => panic!("{id}: unknown byte format {other}"),
        }
    }

    assert!(seen > 30, "saw {seen} byte fixtures");
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
