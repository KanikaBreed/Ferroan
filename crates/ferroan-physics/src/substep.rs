// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/SubstepAccumulator.cs
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! How many fixed physics steps each heartbeat runs.
//!
//! Every physics step is `1 / rate` seconds, with that division done in
//! `f64` and then narrowed for [`SubstepAccumulator::step_seconds`]. The
//! heartbeat's time is added to an accumulator and as many whole steps as
//! it holds are taken. The remainder carries into the next heartbeat. A
//! heartbeat never runs more than [`MAX_STEPS_PER_FRAME`] steps; time beyond
//! that cap is dropped, not carried.

use std::error::Error;
use std::fmt;

/// The most physics steps one heartbeat runs.
pub const MAX_STEPS_PER_FRAME: i32 = 16;

/// A physics step rate was not finite and greater than zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateError;

impl fmt::Display for RateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("physics step rate must be finite and greater than zero")
    }
}

impl Error for RateError {}

/// Fixed-size physics steps taken out of a heartbeat.
///
/// Pure: no scene and no backend.
#[derive(Debug)]
pub struct SubstepAccumulator {
    rate_hz: f32,
    /// `1.0 / rate_hz` in `f64`, the value C# stores in `_step`.
    step: f64,
    step_seconds: f32,
    carry: f64,
    steps: i64,
    capped_frames: i64,
}

impl SubstepAccumulator {
    /// Starts a new accumulator at `rate_hz` steps per second.
    ///
    /// Refuses zero, a negative rate, NaN and either infinity.
    pub fn new(rate_hz: f32) -> Result<Self, RateError> {
        if non_positive(rate_hz, 0.0) || rate_hz.is_infinite() {
            return Err(RateError);
        }
        let step = 1.0 / f64::from(rate_hz);
        Ok(Self {
            rate_hz,
            step,
            step_seconds: step as f32,
            carry: 0.0,
            steps: 0,
            capped_frames: 0,
        })
    }

    /// The rate given to [`Self::new`], in hertz.
    pub fn rate_hz(&self) -> f32 {
        self.rate_hz
    }

    /// The length of every physics step, `1 / rate`, narrowed to `f32`.
    pub fn step_seconds(&self) -> f32 {
        self.step_seconds
    }

    /// Physics steps taken in all.
    pub fn steps(&self) -> i64 {
        self.steps
    }

    /// Heartbeats that hit [`MAX_STEPS_PER_FRAME`] and dropped time.
    pub fn capped_frames(&self) -> i64 {
        self.capped_frames
    }

    /// Time carried into the next heartbeat, in seconds.
    ///
    /// On a frame that was not capped this stays under one step.
    pub fn carry(&self) -> f64 {
        self.carry
    }

    /// Adds one heartbeat's time and returns how many physics steps to run now.
    ///
    /// A frame that is not finite and greater than zero returns 0 and changes
    /// nothing. A frame that would run more than [`MAX_STEPS_PER_FRAME`] steps
    /// runs that many, drops the rest, and counts the frame.
    pub fn advance(&mut self, frame_seconds: f64) -> i32 {
        if non_positive(frame_seconds, 0.0) || frame_seconds.is_infinite() {
            return 0;
        }
        self.carry += frame_seconds;
        // The small epsilon keeps a heartbeat that is a whole number of steps
        // (in decimal) from losing one to binary rounding and taking it a
        // heartbeat late.
        let whole = (self.carry / self.step + 1e-9).floor();
        let n = if whole > f64::from(MAX_STEPS_PER_FRAME) {
            self.carry = 0.0;
            self.capped_frames += 1;
            MAX_STEPS_PER_FRAME
        } else {
            let n = whole as i32;
            self.carry = non_negative(self.carry - f64::from(n) * self.step);
            n
        };
        self.steps += i64::from(n);
        n
    }
}

/// `!(value > 0)` in C#. NaN and every non-positive value, including `-0`, are included.
fn non_positive<T: PartialOrd>(value: T, zero: T) -> bool {
    !matches!(value.partial_cmp(&zero), Some(std::cmp::Ordering::Greater))
}

/// `Math.Max(0.0, value)`. A negative remainder, including `-0.0`, becomes `+0.0`.
fn non_negative(value: f64) -> f64 {
    if value.is_nan() || value > 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn fixture() -> Value {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt/substeps.json");
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn hex_u32(text: &str) -> u32 {
        u32::from_str_radix(text, 16).unwrap()
    }

    fn hex_u64(text: &str) -> u64 {
        u64::from_str_radix(text, 16).unwrap()
    }

    fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
        value.get(key).and_then(Value::as_str).unwrap()
    }

    fn i64_field(value: &Value, key: &str) -> i64 {
        value.get(key).and_then(Value::as_i64).unwrap()
    }

    fn i32_field(value: &Value, key: &str) -> i32 {
        i32::try_from(i64_field(value, key)).unwrap()
    }

    fn rate_from(value: &Value) -> (SubstepAccumulator, f64) {
        let rate_bits = hex_u32(str_field(value, "rate_hz_bits"));
        let step_bits = hex_u64(str_field(value, "step_bits"));
        let step_seconds_bits = hex_u32(str_field(value, "step_seconds_bits"));
        let acc = SubstepAccumulator::new(f32::from_bits(rate_bits)).unwrap();
        assert_eq!(acc.rate_hz().to_bits(), rate_bits);
        assert_eq!(acc.step.to_bits(), step_bits);
        assert_eq!(acc.step_seconds().to_bits(), step_seconds_bits);
        (acc, f64::from_bits(step_bits))
    }

    #[test]
    fn the_fixture_is_the_pin8_accumulator() {
        let doc = fixture();
        assert_eq!(
            str_field(&doc, "pin"),
            "c401bceecd1684832ae8ba86a31b5ee01a4ded7b"
        );
        assert_eq!(
            str_field(&doc, "source"),
            "Source/OpenSim.Region.PhysicsModules.Jolt/SubstepAccumulator.cs"
        );
        assert_eq!(i32_field(&doc, "max_steps_per_frame"), MAX_STEPS_PER_FRAME);
    }

    #[test]
    fn step_length_bits_match_the_pinned_class() {
        let doc = fixture();
        let _primary = rate_from(doc.get("rate").unwrap());
        for rate in doc.get("also_rates").unwrap().as_array().unwrap() {
            let _checked = rate_from(rate);
        }
    }

    #[test]
    fn a_thousand_heartbeats_match_the_pinned_counts() {
        let doc = fixture();
        let sequence = doc.get("sequence").unwrap();
        let (mut acc, step) = rate_from(sequence);
        let frame = f64::from_bits(hex_u64(str_field(sequence, "heartbeat_f64_bits")));
        assert_eq!(
            f64::from(f32::from_bits(hex_u32(str_field(
                sequence,
                "heartbeat_f32_bits"
            ))))
            .to_bits(),
            frame.to_bits()
        );
        let counts = sequence.get("counts").unwrap().as_array().unwrap();
        assert_eq!(counts.len(), 1000);
        let mut fours = 0_i64;
        let mut fives = 0_i64;
        let mut other = 0_i64;
        for (index, expected) in counts.iter().enumerate() {
            let want = i32::try_from(expected.as_i64().unwrap()).unwrap();
            let got = acc.advance(frame);
            assert_eq!(got, want, "heartbeat {index}");
            match got {
                4 => fours += 1,
                5 => fives += 1,
                _ => other += 1,
            }
        }
        assert_eq!(fours, i64_field(sequence, "fours"));
        assert_eq!(fives, i64_field(sequence, "fives"));
        assert_eq!(other, i64_field(sequence, "other"));
        assert_eq!(acc.steps(), i64_field(sequence, "steps"));
        assert_eq!(acc.capped_frames(), i64_field(sequence, "capped_frames"));
        assert_eq!(
            acc.carry().to_bits(),
            hex_u64(str_field(sequence, "carry_bits"))
        );
        assert!(acc.carry() >= 0.0);
        assert!(acc.carry() < step);
    }

    #[test]
    fn a_capped_frame_drops_time_and_counts_the_frame() {
        let doc = fixture();
        let capped = doc.get("capped").unwrap();
        let (mut acc, _) = rate_from(capped);
        let frame = f64::from_bits(hex_u64(str_field(capped, "frame_f64_bits")));
        assert_eq!(acc.advance(frame), i32_field(capped, "returned"));
        assert_eq!(acc.steps(), i64_field(capped, "steps"));
        assert_eq!(acc.capped_frames(), i64_field(capped, "capped_frames"));
        assert_eq!(
            acc.carry().to_bits(),
            hex_u64(str_field(capped, "carry_bits"))
        );
        let next = f64::from_bits(hex_u64(str_field(capped, "next_frame_f64_bits")));
        assert_eq!(acc.advance(next), i32_field(capped, "next_returned"));
        assert_eq!(acc.steps(), i64_field(capped, "next_steps"));
        assert_eq!(acc.capped_frames(), i64_field(capped, "next_capped_frames"));
        assert_eq!(
            acc.carry().to_bits(),
            hex_u64(str_field(capped, "next_carry_bits"))
        );
    }

    #[test]
    fn an_exact_sixteen_keeps_the_remainder() {
        let doc = fixture();
        let section = doc.get("exact_sixteen").unwrap();
        let (mut acc, _) = rate_from(section);
        let frame = f64::from_bits(hex_u64(str_field(section, "frame_f64_bits")));
        let counts = section.get("counts").unwrap().as_array().unwrap();
        assert_eq!(counts.len(), 1);
        let got = acc.advance(frame);
        assert_eq!(got, i32::try_from(counts[0].as_i64().unwrap()).unwrap());
        assert_eq!(got, 16);
        assert_eq!(acc.steps(), i64_field(section, "steps"));
        assert_eq!(acc.capped_frames(), i64_field(section, "capped_frames"));
        assert_eq!(acc.capped_frames(), 0);
        assert_eq!(
            acc.carry().to_bits(),
            hex_u64(str_field(section, "carry_bits"))
        );
    }

    #[test]
    fn the_epsilon_sequences_match_the_pinned_counts() {
        let doc = fixture();
        let wide = doc.get("epsilon_90hz").unwrap();
        replay_heartbeats(wide);
        let wide_counts = wide.get("counts").unwrap().as_array().unwrap();
        assert_eq!(wide_counts.len(), 4);
        assert_eq!(
            i32::try_from(wide_counts[3].as_i64().unwrap()).unwrap(),
            12,
            "heartbeat 3"
        );

        let eleventh = doc.get("epsilon_11hz").unwrap();
        replay_heartbeats(eleventh);
        let eleventh_counts = eleventh.get("counts").unwrap().as_array().unwrap();
        assert_eq!(eleventh_counts.len(), 11);
        assert_eq!(
            i32::try_from(eleventh_counts[10].as_i64().unwrap()).unwrap(),
            5,
            "heartbeat 10"
        );
    }

    fn replay_heartbeats(section: &Value) {
        let (mut acc, _) = rate_from(section);
        let frame = f64::from_bits(hex_u64(str_field(section, "heartbeat_f64_bits")));
        if let Some(bits) = section.get("heartbeat_f32_bits").and_then(Value::as_str) {
            assert_eq!(
                f64::from(f32::from_bits(hex_u32(bits))).to_bits(),
                frame.to_bits()
            );
        }
        let counts = section.get("counts").unwrap().as_array().unwrap();
        for (index, expected) in counts.iter().enumerate() {
            let want = i32::try_from(expected.as_i64().unwrap()).unwrap();
            let got = acc.advance(frame);
            assert_eq!(got, want, "heartbeat {index}");
        }
        assert_eq!(acc.steps(), i64_field(section, "steps"));
        assert_eq!(acc.capped_frames(), i64_field(section, "capped_frames"));
        assert_eq!(
            acc.carry().to_bits(),
            hex_u64(str_field(section, "carry_bits"))
        );
    }

    #[test]
    fn a_rejected_rate_is_refused() {
        let doc = fixture();
        for bits in doc.get("rejected_rate_bits").unwrap().as_array().unwrap() {
            let rate = f32::from_bits(hex_u32(bits.as_str().unwrap()));
            assert!(SubstepAccumulator::new(rate).is_err(), "{bits}");
        }
    }

    #[test]
    fn a_frame_with_no_time_runs_no_step() {
        let doc = fixture();
        for case in doc.get("no_time").unwrap().as_array().unwrap() {
            let (mut acc, _) = rate_from(case);
            let frame = f64::from_bits(hex_u64(str_field(case, "frame_bits")));
            assert_eq!(acc.advance(frame), i32_field(case, "returned"));
            assert_eq!(acc.steps(), i64_field(case, "steps"));
            assert_eq!(acc.capped_frames(), i64_field(case, "capped_frames"));
            assert_eq!(
                acc.carry().to_bits(),
                hex_u64(str_field(case, "carry_bits"))
            );
        }
    }

    #[test]
    fn a_frame_with_no_time_leaves_the_carry() {
        let doc = fixture();
        let held = doc.get("held").unwrap();
        let (mut acc, _) = rate_from(held);
        let frame = f64::from_bits(hex_u64(str_field(held, "heartbeat_f64_bits")));
        assert_eq!(acc.advance(frame), i32_field(held, "first_returned"));
        for case in held.get("frames").unwrap().as_array().unwrap() {
            let frame = f64::from_bits(hex_u64(str_field(case, "frame_bits")));
            assert_eq!(acc.advance(frame), i32_field(case, "returned"));
            assert_eq!(acc.steps(), i64_field(case, "steps"));
            assert_eq!(acc.capped_frames(), i64_field(case, "capped_frames"));
            assert_eq!(
                acc.carry().to_bits(),
                hex_u64(str_field(case, "carry_bits"))
            );
        }
    }
}
