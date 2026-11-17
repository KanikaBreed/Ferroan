// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/throttle.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// The channel shares are LLViewerThrottle::getThrottleGroup
// (indra/newview/llviewerthrottle.cpp, originally LGPL 2.1).
// The dynamic updater is not included: the bot sends one group.

//! The seven AgentThrottle channel rates.

/// Resend, Land, Wind, Cloud, Task, Texture, Asset.
pub const CHANNELS: usize = 7;

/// `BW_PRESET_50`, `_300`, `_500` and `_1000`.
const PRESETS: [[f32; CHANNELS]; 4] = [
    [5.0, 10.0, 3.0, 3.0, 10.0, 10.0, 9.0],
    [30.0, 40.0, 9.0, 9.0, 86.0, 86.0, 40.0],
    [50.0, 70.0, 14.0, 14.0, 136.0, 136.0, 80.0],
    [100.0, 100.0, 20.0, 20.0, 310.0, 310.0, 140.0],
];

fn total(group: &[f32; CHANNELS]) -> f32 {
    group.iter().sum()
}

/// Channel rates in kb/s for a total of `kbps` (`getThrottleGroup`).
/// Totals outside 50..=6000 are clamped. Past the last preset the step
/// between the last two presets is extended.
pub fn group(kbps: f32) -> [f32; CHANNELS] {
    let kbps = kbps.clamp(50.0, 6000.0);
    let above = PRESETS.iter().position(|preset| total(preset) > kbps);
    let (from, to) = match above {
        Some(0) => return PRESETS[0],
        Some(index) => (index - 1, index),
        None => (PRESETS.len() - 2, PRESETS.len() - 1),
    };
    let base = if above.is_some() { from } else { to };
    let step = std::array::from_fn(|channel| PRESETS[to][channel] - PRESETS[from][channel]);
    let share = (kbps - total(&PRESETS[base])) / total(&step);
    std::array::from_fn(|channel| PRESETS[base][channel] + step[channel] * share)
}

/// Bits per second for the bot's one AgentThrottle.
///
/// Aurora asks 1.5 times its 4_000 kb/s setting at the start of a session,
/// which is this 6_000 kb/s group. The simulator reads the blob as seven
/// little-endian `f32`s (`LLViewerThrottleGroup::sendToSim`).
pub fn initial_throttle_bits() -> [f32; CHANNELS] {
    group(6_000.0).map(|kbps| kbps * 1024.0)
}

pub fn initial_throttle_bytes() -> Vec<u8> {
    initial_throttle_bits()
        .into_iter()
        .flat_map(|rate| rate.to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_thousand_kbps_is_the_extended_top_preset() {
        let rates = group(6_000.0);
        let sum: f32 = rates.iter().sum();
        assert!((sum - 6_000.0).abs() < 0.01, "{sum}");
        assert_eq!(rates[0], 600.0);
        assert_eq!(rates[4], 2050.0);
        let bits = initial_throttle_bits();
        assert_eq!(bits[0], 600.0 * 1024.0);
        assert_eq!(initial_throttle_bytes().len(), 28);
    }
}
