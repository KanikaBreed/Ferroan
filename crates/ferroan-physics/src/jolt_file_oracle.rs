// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replays `fixtures/jolt-file`, the pinned `JoltScene.NoteDilationSample`
//! record. The dilation bits for a full window stay in that file. This test
//! checks the private fields, the ring indices, the exact ratios and the
//! native-unloaded proof. It does not recompute the ratio.

use std::path::PathBuf;

const WINDOW: u32 = 11;

#[test]
fn note_dilation_sample_replays_the_committed_file() {
    assert_eq!(1f32.to_bits(), 0x3F80_0000);
    assert_eq!(0.5f32.to_bits(), 0x3F00_0000);
    assert_eq!(0.25f32.to_bits(), 0x3E80_0000);
    assert_eq!(0.75f32.to_bits(), 0x3F40_0000);

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file");
    let manifest = std::fs::read_to_string(root.join("manifest.txt")).expect("manifest");
    let config = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-config/manifest.txt"),
    )
    .expect("config manifest");
    let revision = value(&manifest, "revision ");
    assert_eq!(revision.len(), 40);
    assert!(revision.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(revision, value(&config, "revision "));
    assert!(manifest.contains("compile OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs\n"));
    assert!(manifest.lines().any(|line| line == "note-dilation"));

    let calls = std::fs::read_to_string(root.join("note-dilation/calls.txt")).expect("calls");
    assert_eq!(calls, expected_calls());

    let result = std::fs::read_to_string(root.join("note-dilation/result.txt")).expect("result");
    assert_result(&result);

    let native = std::fs::read_to_string(root.join("note-dilation/native.txt")).expect("native");
    assert_eq!(native, expected_native());
}

fn expected_calls() -> String {
    let mut text = String::from(
        "format 1\nclock Stopwatch\nunits time_step_seconds gap_ticks\nconfig unused\n",
    );
    text.push_str("\nscene A\n");
    text.push_str("call 3F800000 frequency\n");
    for _ in 0..10 {
        text.push_str("call 3F000000 frequency\n");
    }
    text.push_str("call 3E800000 frequency\n");
    text.push_str("\nscene B\n");
    text.push_str("call 3F800000 1\n");
    text
}

fn expected_native() -> String {
    let mut text = String::from("format 1\n");
    for when in ["after_type_init", "after_construct", "after_calls"] {
        text.push_str("observed ");
        text.push_str(when);
        text.push('\n');
        text.push_str("JoltNative.s_loaded null\n");
        text.push_str("JoltNative.s_handle 0\n");
        text.push_str("JoltPhysicsBackend.s_foundationRefCount 0\n");
        text.push_str("joltc_mapped false\n");
    }
    text.push_str(
        "witness s_foundationRefCount stays 0, so JoltPhysicsBackend.Initialize did not call Foundation.Init\n",
    );
    text.push_str("witness joltc is not mapped, so Foundation.Init did not call JPH_Init\n");
    text
}

fn assert_result(text: &str) {
    let (header, scenes) = parse(text);
    assert_eq!(
        header,
        vec![
            "format 1",
            "method NoteDilationSample",
            "source JoltScene.cs 2928-2942",
            "access private",
            "config unused",
            "initialise not_called",
            "clock Stopwatch",
            "units time_step_seconds gap_ticks dilation_ratio",
            "window 11",
            "property_matches_field true",
        ]
    );
    assert_eq!(scenes.len(), 2);
    assert_eq!(scenes[0].0, "A");
    assert_eq!(scenes[1].0, "B");
    let scene_a: Vec<&str> = std::iter::once("3F800000")
        .chain(std::iter::repeat_n("3F000000", 10))
        .chain(std::iter::once("3E800000"))
        .collect();
    assert_scene(&scenes[0].1, &scene_a, "frequency");
    assert_scene(&scenes[1].1, &["3F800000"], "1");

    let full = &scenes[0].1[11];
    let wrapped = &scenes[0].1[12];
    assert_ne!(full.dilation, format!("{:08X}", 1f32.to_bits()));
    assert_ne!(wrapped.dilation, format!("{:08X}", 1f32.to_bits()));
    assert_ne!(full.dilation, wrapped.dilation);
    assert_eq!(full.dilation.len(), 8);
    assert_eq!(wrapped.dilation.len(), 8);
}

fn assert_scene(samples: &[Sample], steps: &[&str], gap: &str) {
    assert_eq!(samples.len(), steps.len() + 1);
    assert_before(&samples[0]);
    for (index, sample) in samples.iter().enumerate().skip(1) {
        let call = index - 1;
        let calls = u32::try_from(index).unwrap();
        assert_eq!(sample.label, call.to_string());
        assert_eq!(sample.time_step.as_deref(), Some(steps[call]));
        assert_eq!(sample.gap.as_deref(), Some(gap));
        assert_eq!(sample.count, calls.min(WINDOW));
        assert_eq!(sample.next, calls % WINDOW);
        if call == 0 {
            assert_eq!(sample.dilation, format!("{:08X}", 1f32.to_bits()));
        } else if call == 1 && gap == "frequency" {
            assert_eq!(sample.dilation, format!("{:08X}", 0.75f32.to_bits()));
        }
        assert_eq!(sample.steps.len(), WINDOW as usize);
        assert_eq!(sample.gaps.len(), WINDOW as usize);
        for slot in 0..WINDOW {
            let step = sample.steps[slot as usize].clone();
            let stored_gap = sample.gaps[slot as usize].clone();
            assert_eq!(sample.step_index[slot as usize], slot);
            assert_eq!(sample.gap_index[slot as usize], slot);
            let written = written_call(call, slot);
            match written {
                None => {
                    assert_eq!(step, zero_double());
                    assert_eq!(stored_gap, zero_double());
                }
                Some(source) => {
                    assert_eq!(step, widened(steps[source]));
                    if gap == "frequency" {
                        assert_eq!(stored_gap, format!("{:016X}", 1f64.to_bits()));
                    } else {
                        assert_eq!(stored_gap, "frequency_divisor");
                    }
                }
            }
        }
    }
}

fn written_call(call: usize, slot: u32) -> Option<usize> {
    let slot = slot as usize;
    if call < slot {
        return None;
    }
    let mut latest = None;
    let mut cursor = slot;
    while cursor <= call {
        latest = Some(cursor);
        cursor += WINDOW as usize;
    }
    latest
}

fn assert_before(sample: &Sample) {
    assert_eq!(sample.label, "before");
    assert!(sample.time_step.is_none());
    assert!(sample.gap.is_none());
    assert_eq!(sample.count, 0);
    assert_eq!(sample.next, 0);
    assert_eq!(sample.dilation, format!("{:08X}", 1f32.to_bits()));
    assert!(sample.steps.iter().all(|step| step == &zero_double()));
    assert!(sample.gaps.iter().all(|gap| gap == &zero_double()));
}

fn widened(bits: &str) -> String {
    let value = f32::from_bits(u32::from_str_radix(bits, 16).unwrap());
    format!("{:016X}", f64::from(value).to_bits())
}

fn zero_double() -> String {
    format!("{:016X}", 0f64.to_bits())
}

fn value(text: &str, prefix: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix}"))
        .to_string()
}

struct Sample {
    label: String,
    time_step: Option<String>,
    gap: Option<String>,
    count: u32,
    next: u32,
    dilation: String,
    step_index: Vec<u32>,
    steps: Vec<String>,
    gap_index: Vec<u32>,
    gaps: Vec<String>,
}

fn parse(text: &str) -> (Vec<&str>, Vec<(&str, Vec<Sample>)>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut header = Vec::new();
    let mut scenes = Vec::new();
    let mut index = 0;
    while index < lines.len() && !lines[index].starts_with("scene ") {
        if !lines[index].is_empty() {
            header.push(lines[index]);
        }
        index += 1;
    }
    while index < lines.len() {
        while index < lines.len() && lines[index].is_empty() {
            index += 1;
        }
        if index >= lines.len() {
            break;
        }
        if lines[index] == "property_matches_field true" {
            header.push(lines[index]);
            index += 1;
            continue;
        }
        assert!(lines[index].starts_with("scene "), "{}", lines[index]);
        let name = &lines[index]["scene ".len()..];
        index += 1;
        let mut samples = Vec::new();
        while index < lines.len() && lines[index].starts_with("sample ") {
            let label = lines[index]["sample ".len()..].to_string();
            index += 1;
            let mut time_step = None;
            let mut gap = None;
            if lines[index].starts_with("time_step ") {
                time_step = Some(lines[index]["time_step ".len()..].to_string());
                index += 1;
                gap = Some(take(&lines, &mut index, "gap "));
            }
            let count = take(&lines, &mut index, "count ").parse().unwrap();
            let next = take(&lines, &mut index, "next ").parse().unwrap();
            let dilation = take(&lines, &mut index, "dilation ");
            let mut step_index = Vec::new();
            let mut steps = Vec::new();
            while index < lines.len() && lines[index].starts_with("step ") {
                let (slot, bits) = slot(lines[index], "step ");
                step_index.push(slot);
                steps.push(bits);
                index += 1;
            }
            let mut gap_index = Vec::new();
            let mut gaps = Vec::new();
            while index < lines.len() && lines[index].starts_with("gap_slot ") {
                let (slot_index, bits) = slot(lines[index], "gap_slot ");
                gap_index.push(slot_index);
                gaps.push(bits);
                index += 1;
            }
            samples.push(Sample {
                label,
                time_step,
                gap,
                count,
                next,
                dilation,
                step_index,
                steps,
                gap_index,
                gaps,
            });
        }
        scenes.push((name, samples));
    }
    (header, scenes)
}

fn take(lines: &[&str], index: &mut usize, prefix: &str) -> String {
    let line = lines[*index];
    *index += 1;
    line.strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{line} is not {prefix}"))
        .to_string()
}

fn slot(line: &str, prefix: &str) -> (u32, String) {
    let rest = line.strip_prefix(prefix).unwrap();
    let (index, bits) = rest.split_once(' ').unwrap();
    (index.parse().unwrap(), bits.to_string())
}
