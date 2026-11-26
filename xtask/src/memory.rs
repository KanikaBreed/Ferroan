// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Resident memory of the servers a live run started (P0.10n11).
//!
//! The harness reads `VmRSS` and `VmHWM` from `/proc/<pid>/status` of the
//! processes it launched: the home region, the C# neighbour, C# Robust and
//! `ferroan-robust`. A [`Meter`] takes one sample of each watched process at
//! a named point (`idle`, `online 1`, `after logout`, ...) and prints one line
//! per process:
//!
//! ```text
//! memory: region csharp idle 412.0 MiB, online 1 420.3 MiB, after logout 431.0 MiB, peak 530.1 MiB
//! ```
//!
//! `peak` is the largest `VmHWM` or `VmRSS` read, the process's high-water
//! mark since it started. Nothing here signals a process or reads one the harness did not
//! start. Two limits bound the sampling: the status read
//! ([`STATUS_BYTES`], `live.memory-status-bytes`) and the points kept for one
//! process ([`MAX_POINTS`], `live.memory-points`).
//!
//! A later check that wants a point of its own (Q65d's 500-prim viewer load)
//! calls [`Meter::point`] with its label while its viewer is still in the
//! region, and the line carries that point beside the others.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::{Arc, Mutex};

/// Bytes read from one `/proc/<pid>/status`. Linux writes about 1.5 KiB.
pub(crate) const STATUS_BYTES: usize = 16 * 1024;

/// Points kept for one process in one run. The last slot is kept for the
/// run's final point ([`Meter::last_point`]), so a run with many logins keeps
/// `idle`, its first logins and `after logout`.
pub(crate) const MAX_POINTS: usize = 16;

/// One read of `/proc/<pid>/status`, in KiB as the kernel reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sample {
    pub rss_kib: u64,
    pub hwm_kib: u64,
}

/// `VmRSS` and `VmHWM` from the text of `/proc/<pid>/status`.
pub(crate) fn parse_status(bytes: &[u8]) -> Result<Sample, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the status file is not UTF-8")?;
    let mut rss = None;
    let mut hwm = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss = Some(kib(rest, "VmRSS")?);
        } else if let Some(rest) = line.strip_prefix("VmHWM:") {
            hwm = Some(kib(rest, "VmHWM")?);
        }
    }
    match (rss, hwm) {
        (Some(rss_kib), Some(hwm_kib)) => Ok(Sample { rss_kib, hwm_kib }),
        // A zombie or a kernel thread has no memory lines.
        _ => Err("the status file has no VmRSS and VmHWM lines".to_string()),
    }
}

fn kib(rest: &str, field: &str) -> Result<u64, String> {
    let mut words = rest.split_whitespace();
    let value = words
        .next()
        .and_then(|word| word.parse::<u64>().ok())
        .ok_or_else(|| format!("{field} is not a number"))?;
    if words.next() != Some("kB") || words.next().is_some() {
        return Err(format!("{field} is not in kB"));
    }
    Ok(value)
}

/// Read one process's memory. The caller started that process.
#[cfg(target_os = "linux")]
pub(crate) fn sample(pid: u32) -> Result<Sample, String> {
    use std::io::Read;
    if pid <= 1 {
        return Err(format!("pid {pid} is not a harness process"));
    }
    let path = format!("/proc/{pid}/status");
    let file =
        std::fs::File::open(&path).map_err(|error| format!("could not read {path}: {error}"))?;
    let mut bytes = Vec::new();
    file.take(STATUS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {path}: {error}"))?;
    if bytes.len() > STATUS_BYTES {
        ferroan_diag::refuse(
            ferroan_diag::LIVE_MEMORY_STATUS_BYTES,
            ferroan_diag::Kind::Hard,
            STATUS_BYTES as u64,
            bytes.len() as u64,
            "memory sample",
        );
        return Err(format!(
            "{path} is longer than {STATUS_BYTES} bytes (limit {})",
            ferroan_diag::LIVE_MEMORY_STATUS_BYTES
        ));
    }
    parse_status(&bytes).map_err(|error| format!("{path}: {error}"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn sample(_pid: u32) -> Result<Sample, String> {
    Err("memory samples read /proc and run on Linux".to_string())
}

/// One process a run watches. `role` is `region`, `neighbour` or `robust`;
/// `server` is `csharp` or `ferroan`.
struct Watched {
    role: String,
    server: String,
    pid: u32,
    points: Vec<(String, Sample)>,
    /// Points not kept because of [`MAX_POINTS`].
    dropped: usize,
}

#[derive(Default)]
struct State {
    watched: Vec<Watched>,
    online: usize,
    errors: Vec<String>,
    /// Printed after the server name: `region csharp (500 parts) idle …`.
    scene: Option<String>,
}

/// The memory points of one run, shared with the bot's after-login hook.
#[derive(Default)]
pub(crate) struct Meter {
    state: Mutex<State>,
}

impl Meter {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A meter whose lines name the scene the region holds.
    pub(crate) fn for_scene(scene: &str) -> Arc<Self> {
        let meter = Self::default();
        meter.lock().scene = Some(scene.to_string());
        Arc::new(meter)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Sample this process at every later point. `pid` is a process the
    /// harness started and still owns.
    pub(crate) fn watch(&self, role: &str, server: &str, pid: u32) {
        self.lock().watched.push(Watched {
            role: role.to_string(),
            server: server.to_string(),
            pid,
            points: Vec::new(),
            dropped: 0,
        });
    }

    /// Sample every watched process now, under `label`. Past
    /// `MAX_POINTS - 1` points the sample is not taken and is counted on the
    /// line and through `live.memory-points`.
    pub(crate) fn point(&self, label: &str) -> Result<(), String> {
        self.point_with(label, MAX_POINTS - 1, sample)
    }

    /// The run's final point (`after logout`), which may use the last slot.
    pub(crate) fn last_point(&self, label: &str) -> Result<(), String> {
        self.point_with(label, MAX_POINTS, sample)
    }

    fn point_with(
        &self,
        label: &str,
        room: usize,
        mut read: impl FnMut(u32) -> Result<Sample, String>,
    ) -> Result<(), String> {
        let mut state = self.lock();
        let mut failures = Vec::new();
        for watched in &mut state.watched {
            if watched.points.len() >= room {
                ferroan_diag::drop_bound(
                    ferroan_diag::LIVE_MEMORY_POINTS,
                    ferroan_diag::Kind::Hard,
                    MAX_POINTS as u64,
                    watched.points.len() as u64 + 1,
                    "memory point",
                );
                watched.dropped += 1;
                continue;
            }
            match read(watched.pid) {
                Ok(sample) => watched.points.push((label.to_string(), sample)),
                Err(error) => failures.push(format!(
                    "memory: {} {} at {label}: {error}",
                    watched.role, watched.server
                )),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            state.errors.extend(failures.iter().cloned());
            Err(failures.join("\n"))
        }
    }

    /// The point a bot hook takes: `handshake 1`, `handshake 2`, ... right
    /// after each login's handshake, in login order across the run, before
    /// the region has sent the scene; `scene arrived` once a stored-prim
    /// viewer has every part; `settled` once both `roots` bots have been
    /// driven for two seconds. A failure is kept and returned by
    /// [`Meter::lines`], since the hook cannot fail the login.
    pub(crate) fn online(&self, moment: ferroan_bot::scenario::Moment) {
        let label = {
            let mut state = self.lock();
            match moment {
                ferroan_bot::scenario::Moment::Login => {
                    state.online += 1;
                    format!("handshake {}", state.online)
                }
                ferroan_bot::scenario::Moment::Scene => "scene arrived".to_string(),
                ferroan_bot::scenario::Moment::Settled => "settled".to_string(),
            }
        };
        let _ = self.point(&label);
    }

    /// One line per watched process, in the order they were watched, or
    /// every sampling failure of the run.
    pub(crate) fn lines(&self) -> Result<Vec<String>, String> {
        let state = self.lock();
        if !state.errors.is_empty() {
            return Err(state.errors.join("\n"));
        }
        Ok(state
            .watched
            .iter()
            .map(|watched| {
                let server = match &state.scene {
                    Some(scene) => format!("{} ({scene})", watched.server),
                    None => watched.server.clone(),
                };
                let mut text = line(&watched.role, &server, &watched.points);
                if watched.dropped > 0 {
                    text.push_str(&format!(
                        ", {} {} dropped (limit {})",
                        watched.dropped,
                        if watched.dropped == 1 {
                            "point"
                        } else {
                            "points"
                        },
                        ferroan_diag::LIVE_MEMORY_POINTS
                    ));
                }
                text
            })
            .collect())
    }

    /// The points of the process watched `index`-th, or none.
    pub(crate) fn points(&self, index: usize) -> Vec<(String, Sample)> {
        self.lock()
            .watched
            .get(index)
            .map(|watched| watched.points.clone())
            .unwrap_or_default()
    }

    /// The bot hook that takes [`Meter::online`] after each login.
    pub(crate) fn after_login(self: &Arc<Self>) -> ferroan_bot::scenario::AfterLogin {
        let meter = Arc::clone(self);
        Arc::new(move |_principal, moment| meter.online(moment))
    }
}

/// `memory: <role> <server> <label> <MiB>, ..., peak <MiB>`.
pub(crate) fn line(role: &str, server: &str, points: &[(String, Sample)]) -> String {
    let mut text = format!("memory: {role} {server}");
    for (index, (label, sample)) in points.iter().enumerate() {
        text.push_str(if index == 0 { " " } else { ", " });
        text.push_str(&format!("{label} {}", mib(sample.rss_kib)));
    }
    // The kernel batches its RSS counters per CPU, so a `VmHWM` read can be
    // a little below an earlier `VmRSS` read. The peak is the larger.
    match points
        .iter()
        .map(|(_, sample)| sample.hwm_kib.max(sample.rss_kib))
        .max()
    {
        Some(peak) => text.push_str(&format!(", peak {}", mib(peak))),
        None => text.push_str(" no samples"),
    }
    text
}

/// KiB as MiB, rounded to the nearest tenth.
pub(crate) fn mib(kib: u64) -> String {
    format!("{} MiB", mib_number(kib))
}

/// [`mib`] without the unit.
pub(crate) fn mib_number(kib: u64) -> String {
    let tenths = (kib * 10 + 512) / 1024;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// `memory: machine <n> CPUs, C# GC <mode>`. C#'s Server GC sizes its heaps by
/// the CPU count, so its numbers are only comparable on the same machine
/// shape. `publish` is the C# region's publish directory.
pub(crate) fn machine_line(publish: &std::path::Path) -> String {
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get().to_string())
        .unwrap_or_else(|_| "an unknown number of".to_string());
    let path = publish.join("OpenSim.Server.RegionServer.runtimeconfig.json");
    let gc = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > 64 * 1024 => "unknown (runtimeconfig over 64 KiB)".to_string(),
        Ok(_) => std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| gc_mode(&text))
            .unwrap_or_else(|error| format!("unknown ({error})")),
        Err(error) => format!("unknown ({error})"),
    };
    format!("memory: machine {cpus} CPUs, C# GC {gc}")
}

/// The GC mode a .NET runtimeconfig asks for. Without the keys .NET uses
/// workstation, concurrent GC.
fn gc_mode(text: &str) -> Result<String, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "the runtimeconfig is not JSON".to_string())?;
    let properties = &json["runtimeOptions"]["configProperties"];
    let server = properties["System.GC.Server"].as_bool().unwrap_or(false);
    let concurrent = properties["System.GC.Concurrent"].as_bool().unwrap_or(true);
    Ok(format!(
        "{}, {}",
        if server { "server" } else { "workstation" },
        if concurrent {
            "concurrent"
        } else {
            "not concurrent"
        }
    ))
}

/// Append the lines under a heading to the GitHub job summary, when the run
/// has one (`GITHUB_STEP_SUMMARY`). Elsewhere this does nothing.
pub(crate) fn publish_summary(title: &str, lines: &[String]) -> Result<(), String> {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    if lines.is_empty() {
        return Ok(());
    }
    append_summary(std::path::Path::new(&path), title, lines)
}

fn append_summary(path: &std::path::Path, title: &str, lines: &[String]) -> Result<(), String> {
    use std::io::Write;
    let mut text = format!("\n#### {title}\n\n```text\n");
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("```\n");
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("could not write the job summary: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tcat\nVmPeak:\t    6320 kB\nVmHWM:\t  542832 kB\nVmRSS:\t  421888 kB\nRssAnon:\t     108 kB\n";

    #[test]
    fn the_status_text_gives_rss_and_the_high_water_mark() {
        assert_eq!(
            parse_status(STATUS.as_bytes()).unwrap(),
            Sample {
                rss_kib: 421_888,
                hwm_kib: 542_832
            }
        );
    }

    #[test]
    fn a_status_without_memory_lines_or_with_another_unit_is_refused() {
        assert!(parse_status(b"Name:\tkthreadd\nState:\tS\n").is_err());
        assert!(parse_status(b"VmRSS:\t12 MB\nVmHWM:\t12 kB\n").is_err());
        assert!(parse_status(b"VmRSS:\tlots kB\nVmHWM:\t12 kB\n").is_err());
        assert!(parse_status(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn one_line_per_process_names_every_point_and_the_peak() {
        let meter = Meter::new();
        meter.watch("region", "csharp", 100);
        meter.watch("robust", "ferroan", 200);
        let mut rss = 400 * 1024;
        let mut read = |pid: u32| {
            rss += 1024;
            Ok(Sample {
                rss_kib: rss + u64::from(pid),
                hwm_kib: 600 * 1024,
            })
        };
        meter.point_with("idle", MAX_POINTS - 1, &mut read).unwrap();
        meter
            .point_with("handshake 1", MAX_POINTS - 1, &mut read)
            .unwrap();
        meter
            .point_with("after logout", MAX_POINTS, &mut read)
            .unwrap();
        assert_eq!(
            meter.lines().unwrap(),
            vec![
                "memory: region csharp idle 401.1 MiB, handshake 1 403.1 MiB, after logout 405.1 MiB, peak 600.0 MiB",
                "memory: robust ferroan idle 402.2 MiB, handshake 1 404.2 MiB, after logout 406.2 MiB, peak 600.0 MiB",
            ]
        );
    }

    #[test]
    fn the_peak_is_never_below_a_resident_sample() {
        let points = [
            (
                "scene arrived".to_string(),
                Sample {
                    rss_kib: 262_760,
                    hwm_kib: 262_760,
                },
            ),
            (
                "after logout".to_string(),
                Sample {
                    rss_kib: 249_548,
                    hwm_kib: 262_246,
                },
            ),
        ];
        assert_eq!(
            line("region", "csharp", &points),
            "memory: region csharp scene arrived 256.6 MiB, after logout 243.7 MiB, peak 256.6 MiB"
        );
    }

    #[test]
    fn past_the_bound_points_are_counted_and_after_logout_is_kept() {
        let meter = Meter::new();
        meter.watch("region", "ferroan", 100);
        let one = |_| {
            Ok(Sample {
                rss_kib: 1024,
                hwm_kib: 1024,
            })
        };
        for index in 0..MAX_POINTS + 2 {
            meter
                .point_with(&format!("p{index}"), MAX_POINTS - 1, one)
                .unwrap();
        }
        meter.point_with("after logout", MAX_POINTS, one).unwrap();
        let lines = meter.lines().unwrap();
        let points = meter.points(0);
        assert_eq!(points.len(), MAX_POINTS);
        assert_eq!(points[0].0, "p0");
        assert_eq!(points[MAX_POINTS - 1].0, "after logout");
        assert!(
            lines[0].ends_with(", peak 1.0 MiB, 3 points dropped (limit live.memory-points)"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn a_failed_sample_fails_the_lines() {
        let meter = Meter::new();
        meter.watch("region", "ferroan", 100);
        let error = meter
            .point_with("idle", MAX_POINTS - 1, |_| Err("gone".to_string()))
            .unwrap_err();
        assert!(error.contains("region ferroan at idle: gone"), "{error}");
        assert!(meter.lines().is_err());
    }

    #[test]
    fn the_gc_mode_comes_from_the_runtimeconfig() {
        let pin8 = r#"{"runtimeOptions":{"configProperties":{"System.GC.Server":true,"System.GC.Concurrent":true}}}"#;
        assert_eq!(gc_mode(pin8).unwrap(), "server, concurrent");
        assert_eq!(
            gc_mode(r#"{"runtimeOptions":{}}"#).unwrap(),
            "workstation, concurrent"
        );
        assert!(gc_mode("not json").is_err());
    }

    #[test]
    fn mebibytes_round_to_the_nearest_tenth() {
        assert_eq!(mib(0), "0.0 MiB");
        assert_eq!(mib(1023), "1.0 MiB");
        assert_eq!(mib(1024 + 51), "1.0 MiB");
        assert_eq!(mib(1024 + 52), "1.1 MiB");
    }

    #[test]
    fn the_summary_is_a_titled_text_block() {
        let directory = std::env::temp_dir().join(format!("ferroan-memory-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("summary.md");
        append_summary(&path, "live run login grid", &["memory: a".to_string()]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(
            text,
            "\n#### live run login grid\n\n```text\nmemory: a\n```\n"
        );
    }

    /// The sampler on this process: a live process has both lines, and its
    /// high-water mark is at least its resident size.
    #[test]
    fn the_sampler_reads_a_running_process() {
        if !cfg!(target_os = "linux") {
            eprintln!("skipped: memory samples read /proc/<pid>/status, which only Linux has");
            return;
        }
        let sample = sample(std::process::id()).unwrap();
        assert!(sample.rss_kib > 0);
        assert!(sample.hwm_kib >= sample.rss_kib);
        assert!(super::sample(1).is_err());
    }
}
