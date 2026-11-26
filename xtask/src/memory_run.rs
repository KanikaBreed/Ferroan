// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo xtask live memory [--bulk]` (P0.10n11): the C# and Ferroan region
//! processes side by side, on the same scenes, with bots logged in.
//!
//! One C# Robust serves every step. Each step starts one region process,
//! waits [`super::IDLE_SETTLE`] after its last `INITIALIZATION COMPLETE` line,
//! reads `VmRSS` and `VmHWM`, and stops it. The first line names the CPU
//! count and the C# GC mode, since C#'s Server GC sizes its heaps by CPU
//! count. The steps:
//!
//! - each server on the empty region, then on the 500-part store P2.5a's
//!   `prim-load` delivers (`fixtures/prim-load/README.md`, written by the pin's
//!   `SQLiteSimulationData.StoreObject`). Points: `idle`; `handshake 1` and
//!   `handshake 2` as the two `roots` bots finish their handshakes, before the
//!   region sends the scene, then `settled` once both have been driven for two
//!   seconds; or `handshake 1` and `scene arrived` once the `prim-load` bot
//!   has all 500 parts; then `after logout`;
//! - one and four C# regions in one process at idle, [`REPEATS`] times each in
//!   turn, which gives the median C# cost of each extra region and its range;
//! - with `--bulk`, both servers at idle on the `regionstore bulk` stores
//!   QR22 measured (single-prim groups with no shape row, [`PRIMS`]), and the
//!   idle growth per prim over the median one-region C# idle and the empty
//!   Ferroan region.
//!
//! The Ferroan region is the release build. The C# regions run BasicPhysics
//! and Ferroan has no physics or scripts yet, so no prim has a physics body or
//! a script in either server.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::thread;

use super::{
    ADVERTISED, AccountSeed, ExtraRegion, HOME_BIND, HOME_NAME, HOME_X, HOME_Y, Proc, ROBUST_BIND,
    RegionExtras, RegionStart, RegionStore, ServiceUris, grid_root, provision_account,
    start_proxies, start_region_with, start_robust,
};
use crate::live;
use crate::memory::{self, Meter, Sample};

/// Bulk store sizes, as QR22 measured them (docs/TESTING.md).
pub(super) const PRIMS: [u32; 5] = [500, 1_000, 5_000, 10_000, 40_000];

/// Regions in the one C# process of the per-region step.
const REGIONS: usize = 4;

/// One-region and four-region C# starts, in turn, for `per extra region`.
const REPEATS: usize = 3;

/// The scenes the bots visit: the label, the home store and the bot scenario.
const SCENES: [(&str, RegionStore, &str); 2] = [
    ("empty", RegionStore::Standard, "roots"),
    ("500 parts", RegionStore::LoadFixture, "prim-load"),
];

const SERVERS: [(&str, bool); 2] = [("csharp", false), ("ferroan", true)];

pub(super) fn run(bulk: bool) -> Result<String, String> {
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); cargo xtask live down stops an interrupted run"
        ));
    }
    let _state = crate::grid_state::claim()?;
    live::ensure_region_built()?;
    let ports = crate::ports::alloc_grid(5)?;
    let (robust_public, robust_private, public_proxy, private_proxy, home_port) =
        (ports[0], ports[1], ports[2], ports[3], ports[4]);
    let mut more = Vec::with_capacity(REGIONS - 1);
    for index in 1..REGIONS {
        let x = u32::try_from(index).map_err(|_| "too many regions")?;
        more.push(ExtraRegion {
            name: format!("Ferroan Memory {index}"),
            uuid: format!("6d3c1f20-8e4b-4a57-9c21-0000000000{index:02}"),
            x: HOME_X + x,
            y: HOME_Y + 2,
            udp_port: crate::ports::alloc_dynamic_region()?,
        });
    }
    let root = grid_root();
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|error| format!("could not clear {}: {error}", root.display()))?;
    }
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    let stores: Vec<(PathBuf, u32)> = if bulk {
        PRIMS
            .iter()
            .map(|count| (root.join(format!("objects-{count}.db")), *count))
            .collect()
    } else {
        Vec::new()
    };
    if bulk {
        crate::regionstore::bulk_stores(&stores, live::REGION_UUID)?;
    }
    let estate = live::generate_password()?;
    let chat = live::generate_password()?;
    let passwords = vec![estate.clone(), chat.clone()];
    let proxies = start_proxies(&[
        (
            SocketAddr::from((ADVERTISED, private_proxy)),
            SocketAddr::from((ROBUST_BIND, robust_private)),
            root.join("proxy-robust-private.cap"),
        ),
        (
            SocketAddr::from((ADVERTISED, public_proxy)),
            SocketAddr::from((ROBUST_BIND, robust_public)),
            root.join("proxy-robust-public.cap"),
        ),
        // Robust hands the bot the region's advertised address.
        (
            SocketAddr::from((ADVERTISED, home_port)),
            SocketAddr::from((HOME_BIND, home_port)),
            root.join("proxy-home.cap"),
        ),
    ])?;
    // Every step stops its region process on purpose, and the next one
    // starts on the same port.
    proxies.upstream_stopping(2);
    let step = Step {
        root: &root,
        estate: &estate,
        chat: &chat,
        passwords: &passwords,
        home_port,
        public_proxy,
        private_proxy,
    };
    let result = start_robust(
        &root,
        robust_public,
        robust_private,
        &passwords,
        ServiceUris {
            asset: None,
            account: None,
            auth: None,
            groups: None,
        },
        false,
    )
    .and_then(|mut robust| {
        let accounts = [
            (
                "Test",
                "User",
                &estate,
                "test@example.invalid",
                live::ESTATE_OWNER,
            ),
            (
                "Chat",
                "Bot",
                &chat,
                "chat@example.invalid",
                live::CHAT_USER,
            ),
        ];
        for (first, last, password, email, uuid) in accounts {
            provision_account(
                &mut robust,
                None,
                None,
                robust_private,
                AccountSeed {
                    first,
                    last,
                    password,
                    email,
                    uuid,
                },
                &passwords,
            )?;
        }
        let result = measure(&step, &stores, &more, robust.listen_pid);
        proxies.upstream_stopping(0);
        proxies.upstream_stopping(1);
        drop(robust);
        result
    });
    let finished = proxies.finish();
    let lines = result?;
    finished?;
    memory::publish_summary("live memory", &lines)?;
    Ok(format!("{}\nlive: memory passed", lines.join("\n")))
}

struct Step<'a> {
    root: &'a Path,
    estate: &'a str,
    chat: &'a str,
    passwords: &'a [String],
    home_port: u16,
    public_proxy: u16,
    private_proxy: u16,
}

fn measure(
    step: &Step<'_>,
    stores: &[(PathBuf, u32)],
    more: &[ExtraRegion],
    robust_pid: u32,
) -> Result<Vec<String>, String> {
    let robust = Meter::new();
    robust.watch("robust", "csharp", robust_pid);
    // The accounts were just provisioned; settle as the regions do.
    thread::sleep(super::IDLE_SETTLE);
    robust.point("idle")?;
    let mut lines = vec![memory::machine_line(&live::publish_dir())];
    let mut ferroan_empty = None;
    for (scene, store, bot) in SCENES {
        for (server, ferroan) in SERVERS {
            let (line, idle) = visit(step, scene, store, bot, server, ferroan)?;
            lines.push(line);
            if ferroan && store == RegionStore::Standard {
                ferroan_empty = Some(idle);
            }
            robust.point(&format!("after {server} {scene}"))?;
        }
    }
    let ferroan_empty = ferroan_empty.ok_or("the empty Ferroan region was not measured")?;
    // One and four C# regions in turn, so a slow first start or a drift of
    // the machine falls on both sides.
    let mut one = Vec::with_capacity(REPEATS);
    let mut four = Vec::with_capacity(REPEATS);
    for index in 1..=REPEATS {
        one.push(idle(
            step,
            &format!("csharp-1-{index}"),
            false,
            &RegionExtras::default(),
        )?);
        four.push(idle(
            step,
            &format!("csharp-4-{index}"),
            false,
            &RegionExtras {
                more,
                ..RegionExtras::default()
            },
        )?);
    }
    let (one_line, four_line, csharp_empty) = region_count_lines(&one, &four);
    lines.push(one_line);
    lines.push(four_line);
    robust.point("after the region-count steps")?;
    let mut largest = None;
    for (store, count) in stores {
        let mut pair = [None, None];
        for (index, (server, ferroan)) in SERVERS.into_iter().enumerate() {
            let sample = idle(
                step,
                &format!("{server}-{count}"),
                ferroan,
                &RegionExtras {
                    store: Some(store),
                    release: ferroan,
                    more: &[],
                },
            )?;
            lines.push(memory::line(
                "region",
                &format!("{server} ({count} bulk prims)"),
                &[("idle".to_string(), sample)],
            ));
            pair[index] = Some(sample);
        }
        if let [Some(csharp), Some(rust)] = pair {
            largest = Some((*count, csharp, rust));
        }
    }
    if let Some((count, csharp, rust)) = largest {
        lines.push(format!(
            "memory: per prim at {count} bulk prims over the empty region (C#: median 1-region idle): csharp {}, ferroan {}",
            per_prim(csharp_empty, csharp.rss_kib, count),
            per_prim(ferroan_empty.rss_kib, rust.rss_kib, count)
        ));
        robust.point("after the bulk stores")?;
    }
    lines.extend(robust.lines()?);
    Ok(lines)
}

/// One server on one scene: idle, the bot's logins, after logout. Returns the
/// report line and the idle sample.
fn visit(
    step: &Step<'_>,
    scene: &str,
    store: RegionStore,
    bot: &str,
    server: &'static str,
    ferroan: bool,
) -> Result<(String, Sample), String> {
    let directory = format!("{server}-{}", scene.replace(' ', "-"));
    let region = start(
        step,
        &directory,
        ferroan,
        store,
        &RegionExtras {
            release: ferroan,
            ..RegionExtras::default()
        },
    )?;
    let meter = Meter::for_scene(scene);
    meter.watch("region", server, region.listen_pid);
    thread::sleep(super::IDLE_SETTLE);
    meter.point("idle")?;
    let mut harness = live::harness(
        &format!("http://{ADVERTISED}:{}/", step.public_proxy),
        &format!("http://{ADVERTISED}:{}", step.private_proxy),
        step.estate,
        step.chat,
        &region.log,
    )?;
    harness.after_login = Some(meter.after_login());
    println!("live: memory {server} {scene}: bot {bot}");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    runtime.block_on(ferroan_bot::scenario::run(bot, &harness))?;
    drop(runtime);
    thread::sleep(super::IDLE_SETTLE);
    meter.last_point("after logout")?;
    let idle = meter
        .points(0)
        .first()
        .map(|(_, sample)| *sample)
        .ok_or("the idle point is missing")?;
    let line = meter.lines()?.pop().ok_or("the region line is missing")?;
    drop(region);
    Ok((line, idle))
}

/// Start one region process, take its idle sample, stop it.
fn idle(
    step: &Step<'_>,
    directory: &str,
    ferroan: bool,
    extras: &RegionExtras<'_>,
) -> Result<Sample, String> {
    let region = start(step, directory, ferroan, RegionStore::Standard, extras)?;
    thread::sleep(super::IDLE_SETTLE);
    let sample = memory::sample(region.listen_pid);
    drop(region);
    sample
}

fn start(
    step: &Step<'_>,
    directory: &str,
    ferroan: bool,
    store: RegionStore,
    extras: &RegionExtras<'_>,
) -> Result<Proc, String> {
    let cwd = step.root.join(directory);
    println!(
        "live: memory step {directory} on {HOME_BIND}:{}",
        step.home_port
    );
    start_region_with(
        &cwd,
        (&live::publish_dir(), &live::source_dir()),
        step.estate,
        &RegionStart {
            ferroan,
            login_only: true,
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
            advertised: ADVERTISED,
            udp_bind: ADVERTISED,
            http_port: step.home_port,
            public_proxy: step.public_proxy,
            private_proxy: step.private_proxy,
            capture: None,
            asset_uri: None,
            account_uri: None,
            auth_uri: None,
            metrics_port: None,
            parcel_seed: None,
            store,
            patch_materials: false,
            basic_physics: !ferroan,
            groups_uri: None,
        },
        step.passwords,
        false,
        extras,
    )
}

/// The one-region and four-region C# lines, and the median one-region idle
/// `VmRSS` in KiB. `per extra region` is the median over the runs of a third
/// of each four-region idle over the one-region idle started just before it.
fn region_count_lines(one: &[Sample], four: &[Sample]) -> (String, String, u64) {
    let rss = |samples: &[Sample]| {
        samples
            .iter()
            .map(|sample| sample.rss_kib)
            .collect::<Vec<_>>()
    };
    let peak = |samples: &[Sample]| {
        samples
            .iter()
            .map(|sample| sample.hwm_kib.max(sample.rss_kib))
            .max()
            .unwrap_or(0)
    };
    let extra: Vec<u64> = one
        .iter()
        .zip(four)
        .map(|(one, four)| four.rss_kib.saturating_sub(one.rss_kib) / (REGIONS as u64 - 1))
        .collect();
    let base = spread(&rss(one));
    let one_line = format!(
        "memory: region csharp (1 region) idle {}, peak {}",
        spread_text(base, one.len()),
        memory::mib(peak(one))
    );
    let four_line = format!(
        "memory: region csharp ({REGIONS} regions) idle {}, peak {}, per extra region {}",
        spread_text(spread(&rss(four)), four.len()),
        memory::mib(peak(four)),
        spread_text(spread(&extra), extra.len())
    );
    (one_line, four_line, base.0)
}

/// Median, smallest and largest of `values`; the lower middle for an even
/// count, and zeros for none.
fn spread(values: &[u64]) -> (u64, u64, u64) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    match (sorted.first(), sorted.last()) {
        (Some(min), Some(max)) => (sorted[(sorted.len() - 1) / 2], *min, *max),
        _ => (0, 0, 0),
    }
}

/// `18.0 MiB (median of 3, 8.6-19.9)`.
fn spread_text((median, min, max): (u64, u64, u64), count: usize) -> String {
    format!(
        "{} (median of {count}, {}-{})",
        memory::mib(median),
        memory::mib_number(min),
        memory::mib_number(max)
    )
}

/// Idle growth per prim, in KiB with one decimal.
fn per_prim(empty_kib: u64, loaded_kib: u64, count: u32) -> String {
    let grown = loaded_kib.saturating_sub(empty_kib) * 10;
    let tenths = grown / u64::from(count.max(1));
    format!("{}.{} KiB", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kib(rss: u64) -> Sample {
        Sample {
            rss_kib: rss,
            hwm_kib: rss + 100,
        }
    }

    #[test]
    fn the_extra_region_cost_is_the_median_of_alternating_runs() {
        let one = [kib(300 * 1024), kib(330 * 1024), kib(290 * 1024)];
        let four = [kib(354 * 1024), kib(355 * 1024), kib(350 * 1024)];
        let (one_line, four_line, base) = region_count_lines(&one, &four);
        assert_eq!(base, 300 * 1024);
        assert_eq!(
            one_line,
            "memory: region csharp (1 region) idle 300.0 MiB (median of 3, 290.0-330.0), peak 330.1 MiB"
        );
        assert_eq!(
            four_line,
            "memory: region csharp (4 regions) idle 354.0 MiB (median of 3, 350.0-355.0), peak 355.1 MiB, per extra region 18.0 MiB (median of 3, 8.3-20.0)"
        );
    }

    #[test]
    fn the_prim_cost_is_the_growth_over_the_empty_region() {
        assert_eq!(per_prim(17_000, 97_000, 40_000), "2.0 KiB");
        assert_eq!(per_prim(17_000, 16_000, 500), "0.0 KiB");
    }
}
