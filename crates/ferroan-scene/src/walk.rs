// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One walking step for an avatar that is not sitting on the ground.
//!
//! Each frame tick moves X and Y by the physics velocity times `FrameTime`.
//! A position that was already outside the region is pulled to the 0.1 m
//! inset. A step that would leave the region stops on that inset and keeps
//! the velocity, so the avatar does not sit outside for a frame. The captured
//! BasicPhysics trace does store that outside sample and pulls it back on the
//! next tick; [D38](../../../docs/DECISIONS.md) says stop with no crossing, so
//! this stand-in does not keep the outside sample. Nothing in this region sets
//! the carried-avatar flag, so that branch is not taken.
//!
//! Ground height is the integer terrain cell of the position before the step,
//! clamped into the region. The stored map is `y * width + x`. A missing map
//! is height 0. Login and teleport still use the interpolated height. On the
//! ground, Z becomes terrain plus the default avatar box (1.9 plus the 0.2
//! box adjust) and the vertical velocity becomes 0. Flying adds the vertical
//! velocity unless the old Z plus that step is below terrain plus 2 m, in
//! which case Z snaps to the same box height and the vertical velocity
//! becomes 0. The next tick uses the velocity this step wrote. It does not
//! copy the last command again, so a snapped vertical speed stays 0 until
//! the next movement packet.
//!
//! Prims are ignored. An avatar walks through them and cannot stand on them.
//! The default box stays until appearance exists. The stand-in is removed in
//! Phase 3.

use crate::height::Heightmap;

/// `1.9` box plus the `0.2` box adjust. Appearance replaces this in a later row.
pub(crate) const AVATAR_SIZE_Z: f32 = 2.1;

/// Inset used when a coordinate is already outside, or would leave the region.
pub(crate) const BORDER_INSET: f32 = 0.1;

/// Flying snaps when the stepped Z is below terrain plus this clearance.
pub(crate) const FLY_GROUND_CLEARANCE: f32 = 2.0;

/// Region edge when the frame has no heightmap. P2.0d serves 256 m regions.
pub(crate) const DEFAULT_EXTENT: f32 = 256.0;

/// Position, physics velocity and ground contact after one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Step {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub colliding: bool,
}

/// Integer terrain cell of the position before the step. No map is height 0.
pub(crate) fn terrain_height(
    map: Option<&Heightmap>,
    position: [f32; 3],
    extent_x: f32,
    extent_y: f32,
) -> f32 {
    let Some(map) = map else {
        return 0.0;
    };
    let x = position[0].clamp(0.0, extent_x - BORDER_INSET);
    let y = position[1].clamp(0.0, extent_y - BORDER_INSET);
    map.cell_height(x as i32, y as i32)
}

/// One kinematic step. `terrain` is the cell of `position` before the move.
pub(crate) fn integrate(
    mut position: [f32; 3],
    mut velocity: [f32; 3],
    flying: bool,
    extent_x: f32,
    extent_y: f32,
    terrain: f32,
    dt: f32,
) -> Step {
    let old = position;
    position[0] = stop_at_border(old[0], old[0] + velocity[0] * dt, extent_x);
    position[1] = stop_at_border(old[1], old[1] + velocity[1] * dt, extent_y);
    let height = terrain + AVATAR_SIZE_Z;
    let colliding = if flying {
        let next_z = old[2] + velocity[2] * dt;
        if next_z < terrain + FLY_GROUND_CLEARANCE {
            position[2] = height;
            velocity[2] = 0.0;
            true
        } else {
            position[2] = next_z;
            false
        }
    } else {
        position[2] = height;
        velocity[2] = 0.0;
        true
    };
    Step {
        position,
        velocity,
        colliding,
    }
}

/// Pull a coordinate that is already outside back to the inset, then stop a
/// step that would still finish outside. The test uses the pre-step value.
fn stop_at_border(old: f32, moved: f32, extent: f32) -> f32 {
    let pulled = if (0.0..extent).contains(&old) {
        moved
    } else if old >= extent {
        extent - BORDER_INSET
    } else {
        BORDER_INSET
    };
    if (0.0..extent).contains(&pulled) {
        pulled
    } else if pulled >= extent {
        extent - BORDER_INSET
    } else {
        BORDER_INSET
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FRAME_TIME;
    use crate::height::Heightmap;

    fn capture() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/walk/basicphysics-steps.json");
        std::fs::read_to_string(path).unwrap()
    }

    fn scalar(json: &str, key: &str) -> f32 {
        let marker = format!("\"{key}\"");
        let at = json
            .find(&marker)
            .unwrap_or_else(|| panic!("missing {key}"));
        let rest = json[at + marker.len()..].trim_start();
        let rest = rest.trim_start_matches(':').trim_start();
        let end = rest
            .find([',', '\n', '}', ' '])
            .unwrap_or_else(|| panic!("missing value for {key}"));
        rest[..end]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("bad {key}"))
    }

    fn rows(json: &str, key: &str) -> Vec<Vec<f32>> {
        let marker = format!("\"{key}\"");
        let at = json
            .find(&marker)
            .unwrap_or_else(|| panic!("missing {key}"));
        let rest = &json[at + marker.len()..];
        let open = rest
            .find('[')
            .unwrap_or_else(|| panic!("missing array {key}"));
        let mut depth = 0i32;
        let mut end = None;
        for (index, ch) in rest[open..].char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + index + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &rest[open..end.unwrap()];
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut token = String::new();
        let mut depth = 0i32;
        for ch in body.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    if depth == 2 {
                        row.clear();
                    }
                }
                ']' => {
                    if depth == 2 {
                        push_token(&mut token, &mut row);
                        rows.push(std::mem::take(&mut row));
                    }
                    depth -= 1;
                }
                ',' if depth == 2 => push_token(&mut token, &mut row),
                ch if depth == 2
                    && (ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E')) =>
                {
                    token.push(ch);
                }
                _ => {}
            }
        }
        rows
    }

    fn push_token(token: &mut String, row: &mut Vec<f32>) {
        if token.is_empty() {
            return;
        }
        row.push(token.parse().unwrap());
        token.clear();
    }

    fn replay(samples: &[Vec<f32>], extent: f32, terrain: f32) {
        let first = &samples[0];
        let mut position = [first[0], first[1], first[2]];
        let mut velocity = [first[3], first[4], first[5]];
        let flying = first[6] != 0.0;
        for sample in &samples[1..] {
            let step = integrate(
                position, velocity, flying, extent, extent, terrain, FRAME_TIME,
            );
            assert_eq!(
                step.position[0].to_bits(),
                sample[0].to_bits(),
                "{sample:?}"
            );
            assert_eq!(step.position[1].to_bits(), sample[1].to_bits());
            assert_eq!(step.position[2].to_bits(), sample[2].to_bits());
            assert_eq!(step.velocity[0].to_bits(), sample[3].to_bits());
            assert_eq!(step.velocity[1].to_bits(), sample[4].to_bits());
            assert_eq!(step.velocity[2].to_bits(), sample[5].to_bits());
            assert_eq!(step.colliding, sample[6] != 0.0);
            position = step.position;
            velocity = step.velocity;
        }
    }

    #[test]
    fn the_capture_matches_the_step_inside_the_region() {
        let json = capture();
        assert_eq!(scalar(&json, "dt").to_bits(), FRAME_TIME.to_bits());
        assert_eq!(scalar(&json, "size_z").to_bits(), AVATAR_SIZE_Z.to_bits());
        // Flat ground in the capture is 21, so the snapped box is 23.1.
        let ground = 21.0;
        replay(&rows(&json, "flat_walk"), DEFAULT_EXTENT, ground);
        replay(&rows(&json, "fall"), DEFAULT_EXTENT, ground);
        replay(&rows(&json, "fly_hold"), DEFAULT_EXTENT, ground);
        replay(&rows(&json, "fly_snap"), DEFAULT_EXTENT, ground);
        replay(&rows(&json, "outside"), 16.0, ground);
        replay(&rows(&json, "slope"), 16.0, 30.0);
        let landed = &rows(&json, "no_map")[0];
        let step = integrate(
            [4.0, 5.0, 26.0],
            [0.0, 0.0, 0.0],
            false,
            DEFAULT_EXTENT,
            DEFAULT_EXTENT,
            0.0,
            FRAME_TIME,
        );
        assert_eq!(step.position[0].to_bits(), landed[0].to_bits());
        assert_eq!(step.position[1].to_bits(), landed[1].to_bits());
        assert_eq!(step.position[2].to_bits(), landed[2].to_bits());
        assert_eq!(step.velocity[2].to_bits(), landed[3].to_bits());
        assert!(step.colliding);
    }

    #[test]
    fn a_leaving_step_stops_on_the_inset() {
        let json = capture();
        let first = &rows(&json, "border")[0];
        let mut position = [first[0], first[1], first[2]];
        let velocity = [first[3], first[4], first[5]];
        for _ in 0..4 {
            let step = integrate(position, velocity, false, 16.0, 16.0, 21.0, FRAME_TIME);
            assert_eq!(step.position[0], 15.9);
            assert!(step.position[0] < 16.0);
            assert_eq!(step.velocity[0], velocity[0]);
            assert!(step.colliding);
            position = step.position;
        }
    }

    #[test]
    fn the_integer_cell_is_the_ground_under_the_old_position() {
        let mut plain = Vec::new();
        plain.extend_from_slice(&16i32.to_le_bytes());
        plain.extend_from_slice(&16i32.to_le_bytes());
        for index in 0..256 {
            let height: f32 = if index == 3 * 16 + 2 { 30.0 } else { 21.0 };
            plain.extend_from_slice(&height.to_le_bytes());
        }
        let map = Heightmap::from_blob(22, &plain, 16, 16).unwrap();
        assert_eq!(
            terrain_height(Some(&map), [2.9, 3.9, 40.0], 16.0, 16.0),
            30.0
        );
        assert_eq!(terrain_height(None, [4.0, 5.0, 26.0], 256.0, 256.0), 0.0);
        assert_eq!(map.cell_height(-1, 0), 0.0);
        assert_eq!(map.cell_height(16, 0), 0.0);
    }
}
