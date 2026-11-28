// Copyright (c) Contributors, http://opensimulator.org/
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// The stats transform is ported from SimStatsReporter.cs. Cite that file's
// lines 1-26. The text is licenses/OpenSimulator-BSD-3-Clause.txt.

// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs
//   (NoteDilationSample, lines 2928-2942)
//   and
//   Source/OpenSim.Region.Framework/Scenes/SimStatsReporter.cs
//   (lines 178-183, 319-338, 368-370 and 390)
//   KanikaBreed/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// The dilation window remains available under MPL-2.0. The stats transform
// remains available under BSD-3-Clause. Both form part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause

//! The dilation window and the sim-stats FPS transform.
//!
//! [`DilationWindow::note_sample`] is `JoltScene.NoteDilationSample`. A
//! successful `Simulate` contributes 1. [`statistics_fps_factor`] is the
//! `Normalized55FPS` scale, and [`report_stats`] is the reporter's FPS
//! transform. `SimPhysicsStepMs` stays 20.

/// Heartbeats in the dilation window.
pub const DILATION_WINDOW: usize = 11;

/// The reporter's hard-coded physics step, in milliseconds.
pub const PHYSICS_STEP_MS: f32 = 20.0;

/// Simulated seconds and real seconds for the last
/// [`DILATION_WINDOW`] heartbeats.
#[derive(Clone, Debug)]
pub struct DilationWindow {
    steps: [f64; DILATION_WINDOW],
    gaps: [f64; DILATION_WINDOW],
    count: usize,
    next: usize,
    time_dilation: f32,
}

impl Default for DilationWindow {
    fn default() -> Self {
        Self {
            steps: [0.0; DILATION_WINDOW],
            gaps: [0.0; DILATION_WINDOW],
            count: 0,
            next: 0,
            time_dilation: 1.0,
        }
    }
}

impl DilationWindow {
    /// Stores one heartbeat and recomputes the ratio.
    ///
    /// `gap_ticks / frequency` is the real seconds. The sum is the first
    /// `count` slots. Once the window is full, the next write replaces the
    /// oldest slot. The ratio is 1 when the real gap is not longer than the
    /// simulated time.
    pub fn note_sample(&mut self, time_step: f32, gap_ticks: i64, frequency: i64) {
        let index = self.next;
        self.steps[index] = f64::from(time_step);
        self.gaps[index] = gap_ticks as f64 / frequency as f64;
        self.next = (index + 1) % DILATION_WINDOW;
        if self.count < DILATION_WINDOW {
            self.count += 1;
        }
        let mut steps = 0.0;
        let mut gaps = 0.0;
        for slot in 0..self.count {
            steps += self.steps[slot];
            gaps += self.gaps[slot];
        }
        self.time_dilation = if gaps <= steps {
            1.0
        } else {
            (steps / gaps) as f32
        };
    }

    /// `JoltScene.TimeDilation`. Starts at 1.
    pub fn time_dilation(&self) -> f32 {
        self.time_dilation
    }

    /// Samples stored, at most [`DILATION_WINDOW`].
    pub fn count(&self) -> usize {
        self.count
    }

    /// Slot the next sample will replace.
    pub fn next_slot(&self) -> usize {
        self.next
    }

    /// Simulated seconds stored in `slot`.
    pub fn step_seconds(&self, slot: usize) -> f64 {
        self.steps[slot]
    }

    /// Real seconds stored in `slot`.
    pub fn gap_seconds(&self, slot: usize) -> f64 {
        self.gaps[slot]
    }
}

/// One `AddFrameStats` call. `frames` grows by 1. A successful simulate
/// passes `physics` of 1.
pub fn accumulate_frame(
    frames: &mut u32,
    dilation_sum: &mut f32,
    physics_sum: &mut f32,
    dilation: f32,
    physics: f32,
) {
    *frames += 1;
    *dilation_sum += dilation;
    *physics_sum += physics;
}

/// `55 * FrameTime` when `Normalized55FPS` is on, otherwise 1.
pub fn statistics_fps_factor(normalized_55: bool, frame_time: f32) -> f32 {
    if normalized_55 {
        55.0 * frame_time
    } else {
        1.0
    }
}

/// Inputs of the reporter's FPS transform.
///
/// `frame_elapsed_seconds` is the copy taken under the stats lock, in
/// seconds, before it is inverted. `update_elapsed_seconds` is the stats
/// period. A healthy region inverts the frame window. A frame window under
/// 0.8 of the period uses the period instead.
#[derive(Clone, Copy, Debug)]
pub struct StatsInputs {
    /// Heartbeats accumulated since the last report.
    pub frames: u32,
    /// Sum of those heartbeats' dilation values.
    pub dilation_sum: f32,
    /// Sum of those heartbeats' physics contributions.
    pub physics_sum: f32,
    /// Seconds from the previous report's last frame to this one.
    pub frame_elapsed_seconds: f32,
    /// Seconds since the previous stats report.
    pub update_elapsed_seconds: f32,
    /// [`statistics_fps_factor`].
    pub factor: f32,
}

/// Viewer stats the transform writes.
#[derive(Clone, Copy, Debug)]
pub struct StatsReport {
    /// Rounded to 3 decimal places. NaN becomes 0.
    pub time_dilation: f32,
    /// Rounded to 1 decimal place.
    pub sim_fps: f32,
    /// Rounded to 1 decimal place, and not above [`Self::sim_fps`]'s pre-round value.
    pub physics_fps: f32,
    /// Always [`PHYSICS_STEP_MS`].
    pub physics_step_ms: f32,
}

/// The reporter's FPS transform.
pub fn report_stats(inputs: StatsInputs) -> StatsReport {
    let update_factor = 1.0 / inputs.update_elapsed_seconds;
    let inv = if f64::from(inputs.frame_elapsed_seconds / inputs.update_elapsed_seconds) < 0.8 {
        update_factor
    } else {
        1.0 / inputs.frame_elapsed_seconds
    };
    let mut reported = inputs.frames as f32;
    let mut physics = inputs.physics_sum;
    let mut dilation = inputs.dilation_sum;
    if reported <= 0.0 {
        reported = 0.0;
        physics = 0.0;
        dilation = 0.0;
    } else {
        dilation /= reported;
        reported *= inputs.factor;
        reported *= inv;
        physics *= inv * inputs.factor;
    }
    if physics > reported {
        physics = reported;
    }
    StatsReport {
        time_dilation: if dilation.is_nan() {
            0.0
        } else {
            round_digits(dilation, 3)
        },
        sim_fps: round_digits(reported, 1),
        physics_fps: round_digits(physics, 1),
        physics_step_ms: PHYSICS_STEP_MS,
    }
}

/// `Math.Round(value, digits)` with the midpoint rounded to even, then
/// narrowed back to `f32`. A negative value that rounds to zero stays
/// negative: `Math.Round(-0.0004, 3)` is `-0`, not `+0`.
fn round_digits(value: f32, digits: i32) -> f32 {
    let scale = 10f64.powi(digits);
    let scaled = round_to_even(f64::from(value) * scale);
    (scaled / scale) as f32
}

fn round_to_even(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    let floor = magnitude.floor();
    let diff = magnitude - floor;
    let rounded = if diff < 0.5 || (diff == 0.5 && floor % 2.0 == 0.0) {
        floor
    } else {
        floor + 1.0
    };
    if rounded == 0.0 {
        if negative { -0.0 } else { 0.0 }
    } else if negative {
        -rounded
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PHYSICS_STEP_MS, StatsInputs, accumulate_frame, report_stats, statistics_fps_factor,
    };

    #[test]
    fn stats_transform_matches_the_pinned_reporter() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file");
        let manifest = std::fs::read_to_string(root.join("manifest.txt")).expect("manifest");
        assert!(manifest.lines().any(|line| line == "stats"));
        let text = std::fs::read_to_string(root.join("stats/transform.txt")).expect("transform");
        let (header, body) = text.split_once("\ncase ").expect("case");
        assert!(header.contains("source JoltScene.cs 2928-2942\n"));
        assert!(header.contains("source_simulate JoltScene.cs 2786-2833\n"));
        assert!(header.contains("source_substeps JoltScene.cs 3107-3156\n"));
        assert!(header.contains("factor SimStatsReporter.cs 178-183\n"));
        assert!(header.contains("transform SimStatsReporter.cs 319-338\n"));
        assert!(header.contains("round SimStatsReporter.cs 368-370\n"));
        assert!(header.contains("branch SimStatsReporter.cs 312-317\n"));
        assert!(header.contains("clock SimStatsReporter.cs 274-276\n"));
        assert!(header.contains("step SimStatsReporter.cs 390\n"));
        assert!(header.contains("native not_called\n"));
        assert_eq!(value(header, "healthy_frame_ms "), "1000");
        assert_eq!(value(header, "healthy_update_ms "), "500");
        assert_eq!(value(header, "stall_update_ms "), "1048576000");
        assert_eq!(value(header, "physics_step_ms "), bits(PHYSICS_STEP_MS));
        let mut seen = [false; 4];
        for line in header.lines() {
            let Some(rest) = line.strip_prefix("simulate ") else {
                continue;
            };
            let mut parts = rest.split(' ');
            let name = parts.next().expect("name");
            let returned = parts.next().expect("return");
            let steps: u32 = parts.next().expect("steps").parse().expect("steps");
            assert_eq!(returned, bits(1.0));
            match name {
                "null" => {
                    assert_eq!(steps, 0);
                    seen[0] = true;
                }
                "once" => {
                    assert_eq!(steps, 1);
                    seen[1] = true;
                }
                "substep_empty" => {
                    assert_eq!(steps, 0);
                    seen[2] = true;
                }
                "substeps" => {
                    assert_eq!(steps, 4);
                    seen[3] = true;
                }
                other => panic!("unexpected simulate {other}"),
            }
        }
        assert_eq!(seen, [true; 4]);

        let mut cases = 0u32;
        for case_text in body.split("\ncase ") {
            let name = case_text.lines().next().expect("name");
            let normalized = value(case_text, "normalized ") == "1";
            let frame_time = parse_f32(&value(case_text, "frame_time "));
            let frames: u32 = value(case_text, "frames ").parse().expect("frames");
            let dilation_each = parse_f32(&value(case_text, "dilation_each "));
            let physics_each = parse_f32(&value(case_text, "physics_each "));
            let frame_elapsed = elapsed_seconds(&value(case_text, "frame_ms "));
            let update_elapsed = elapsed_seconds(&value(case_text, "update_ms "));
            let mut got_frames = 0u32;
            let mut dilation_sum = 0.0f32;
            let mut physics_sum = 0.0f32;
            for _ in 0..frames {
                accumulate_frame(
                    &mut got_frames,
                    &mut dilation_sum,
                    &mut physics_sum,
                    dilation_each,
                    physics_each,
                );
            }
            assert_eq!(
                got_frames,
                value(case_text, "frames_read ")
                    .parse::<u32>()
                    .expect("read")
            );
            let recorded_dilation = value(case_text, "dilation_sum ");
            if recorded_dilation == "nan" {
                assert!(dilation_sum.is_nan());
            } else {
                assert_eq!(bits(dilation_sum), recorded_dilation);
            }
            assert_eq!(bits(physics_sum), value(case_text, "physics_sum "));
            let factor = statistics_fps_factor(normalized, frame_time);
            assert_eq!(bits(factor), value(case_text, "factor "));
            let inputs = StatsInputs {
                frames: got_frames,
                dilation_sum,
                physics_sum,
                frame_elapsed_seconds: frame_elapsed,
                update_elapsed_seconds: update_elapsed,
                factor,
            };
            let report = report_stats(inputs);
            assert_eq!(
                bits(report.time_dilation),
                value(case_text, "time_dilation ")
            );
            assert_eq!(bits(report.sim_fps), value(case_text, "sim_fps "));
            assert_eq!(bits(report.physics_fps), value(case_text, "physics_fps "));
            assert_eq!(bits(report.physics_step_ms), value(case_text, "step_ms "));
            assert_eq!(report.physics_step_ms, PHYSICS_STEP_MS);
            match name {
                "normalized-11" | "constructor-frame" => {
                    assert!(report.sim_fps > 50.0 && report.sim_fps < 60.0);
                    assert_eq!(report.physics_fps, report.sim_fps);
                    assert_eq!(physics_each, 1.0);
                }
                "raw-11" => {
                    assert!((report.sim_fps - 11.0).abs() < 0.2);
                    assert_eq!(report.physics_fps, report.sim_fps);
                    assert!(!normalized);
                }
                "physics-off" => {
                    assert!(report.sim_fps > 50.0);
                    assert_eq!(report.physics_fps, 0.0);
                }
                "physics-capped" => {
                    assert_eq!(report.physics_fps, report.sim_fps);
                    assert!(physics_sum > got_frames as f32);
                }
                "no-frames" => {
                    assert_eq!(report.sim_fps, 0.0);
                    assert_eq!(report.physics_fps, 0.0);
                    assert_eq!(report.time_dilation, 0.0);
                }
                "nan-dilation" => {
                    assert!(dilation_sum.is_nan());
                    assert_eq!(report.time_dilation, 0.0);
                    assert!(report.sim_fps > 50.0);
                }
                "signed-zero" => {
                    assert_eq!(frames, 1);
                    assert_eq!(bits(dilation_each), "B9D1B717");
                    assert_eq!(bits(report.time_dilation), "80000000");
                }
                "stalled" => {
                    assert!(f64::from(frame_elapsed / update_elapsed) < 0.8);
                    let healthy = report_stats(StatsInputs {
                        update_elapsed_seconds: frame_elapsed,
                        ..inputs
                    });
                    assert_ne!(bits(report.sim_fps), bits(healthy.sim_fps));
                    assert_ne!(report.sim_fps, 0.0);
                }
                other => panic!("unexpected case {other}"),
            }
            cases += 1;
        }
        assert_eq!(cases, 9);
    }

    fn elapsed_seconds(milliseconds: &str) -> f32 {
        let milliseconds: f64 = milliseconds.parse().expect("milliseconds");
        (milliseconds / 1000.0) as f32
    }

    fn bits(value: f32) -> String {
        format!("{value:08X}", value = value.to_bits())
    }

    fn parse_f32(text: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(text, 16).expect("bits"))
    }

    fn value(text: &str, prefix: &str) -> String {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing {prefix}"))
            .to_string()
    }
}
