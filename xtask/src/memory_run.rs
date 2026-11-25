// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo xtask live memory` (P0.10n11): idle memory of region processes the
//! scenarios do not start.
//!
//! One C# Robust serves every step. Each step starts one region process,
//! waits [`super::IDLE_SETTLE`] after its last `INITIALIZATION COMPLETE` line,
//! reads `VmRSS` and `VmHWM`, and stops it:
//!
//! - the C# region alone, then four C# regions in one process, which gives
//!   the C# cost of each extra region;
//! - the Ferroan region alone (release build);
//! - both servers on the same seeded object store, at each size in
//!   [`PRIMS`]. The stores are the `regionstore bulk` files QR22 measured:
//!   single-prim groups with no shape row, written through the pin's
//!   `SQLiteSimulationData`.
//!
//! The C# regions run BasicPhysics, so no prim gets a physics body; Ferroan has
//! no physics yet. Nothing logs in. The scenarios' `online` points are in
//! every `live run … grid`.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::thread;

use super::{
    ADVERTISED, AccountSeed, ExtraRegion, HOME_BIND, HOME_NAME, HOME_X, HOME_Y, Proc,
    ROBUST_BIND, RegionExtras, RegionStart, ServiceUris, grid_root, provision_account,
    start_proxies, start_region_with, start_robust,
};
use crate::live;
use crate::memory::{self, Sample};

/// Object store sizes, as QR22 measured them (docs/TESTING.md).
pub(super) const PRIMS: [u32; 5] = [500, 1_000, 5_000, 10_000, 40_000];

/// Regions in the one C# process of the per-region step.
const REGIONS: usize = 4;

pub(super) fn run() -> Result<String, String> {
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
    let stores: Vec<(PathBuf, u32)> = PRIMS
        .iter()
        .map(|count| (root.join(format!("objects-{count}.db")), *count))
        .collect();
    crate::regionstore::bulk_stores(&stores, live::REGION_UUID)?;
    let estate = live::generate_password()?;
    let passwords = vec![estate.clone()];
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
    ])?;
    let mut robust = start_robust(
        &root,
        robust_public,
        robust_private,
        &passwords,
        ServiceUris {
            asset: None,
            account: None,
            auth: None,
        },
        false,
    )?;
    provision_account(
        &mut robust,
        None,
        None,
        robust_private,
        AccountSeed {
            first: "Test",
            last: "User",
            password: &estate,
            email: "test@example.invalid",
            uuid: live::ESTATE_OWNER,
        },
        &passwords,
    )?;
    let step = Step {
        root: &root,
        estate: &estate,
        passwords: &passwords,
        home_port,
        public_proxy,
        private_proxy,
    };
    let result = measure(&step, &stores, &more, robust.listen_pid);
    proxies.upstream_stopping(0);
    proxies.upstream_stopping(1);
    drop(robust);
    let finished = proxies.finish();
    let lines = result?;
    finished?;
    memory::publish_summary("live memory", &lines)?;
    Ok(format!("{}\nlive: memory passed", lines.join("\n")))
}

struct Step<'a> {
    root: &'a Path,
    estate: &'a str,
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
    let mut lines = Vec::new();
    let one = idle(step, "csharp-1", false, &RegionExtras::default())?;
    lines.push(memory::line("1 region", "csharp", &[("idle".to_string(), one)]));
    let four = idle(
        step,
        "csharp-4",
        false,
        &RegionExtras {
            more,
            ..RegionExtras::default()
        },
    )?;
    lines.push(per_region_line(one, four));
    let ferroan = idle(
        step,
        "ferroan-1",
        true,
        &RegionExtras {
            release: true,
            ..RegionExtras::default()
        },
    )?;
    lines.push(memory::line(
        "1 region",
        "ferroan",
        &[("idle".to_string(), ferroan)],
    ));
    let mut largest = None;
    for (store, count) in stores {
        let mut pair = Vec::with_capacity(2);
        for (server, is_ferroan) in [("ferroan", true), ("csharp", false)] {
            let sample = idle(
                step,
                &format!("{server}-{count}"),
                is_ferroan,
                &RegionExtras {
                    store: Some(store),
                    release: is_ferroan,
                    more: &[],
                },
            )?;
            lines.push(memory::line(
                &format!("{count} prims"),
                server,
                &[("idle".to_string(), sample)],
            ));
            pair.push(sample);
        }
        largest = Some((*count, pair[0], pair[1]));
    }
    if let Some((count, rust, csharp)) = largest {
        lines.push(format!(
            "memory: per prim at {count} prims against an empty region: ferroan {}, csharp {}",
            per_prim(ferroan, rust, count),
            per_prim(one, csharp, count)
        ));
    }
    let robust = memory::sample(robust_pid)?;
    lines.push(memory::line(
        "robust",
        "csharp",
        &[("after the regions".to_string(), robust)],
    ));
    Ok(lines)
}

/// Start one region process, take its idle sample, stop it.
fn idle(
    step: &Step<'_>,
    directory: &str,
    ferroan: bool,
    extras: &RegionExtras<'_>,
) -> Result<Sample, String> {
    let cwd = step.root.join(directory);
    println!(
        "live: memory step {directory} on {HOME_BIND}:{}",
        step.home_port
    );
    let region: Proc = start_region_with(
        &cwd,
        (&live::publish_dir(), &live::source_dir()),
        step.estate,
        RegionStart {
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
            basic_physics: !ferroan,
        },
        step.passwords,
        false,
        extras,
    )?;
    thread::sleep(super::IDLE_SETTLE);
    let sample = memory::sample(region.listen_pid);
    drop(region);
    sample
}

/// `memory: 4 regions csharp idle …, peak …, per extra region …`.
fn per_region_line(one: Sample, four: Sample) -> String {
    let extra = four.rss_kib.saturating_sub(one.rss_kib) / (REGIONS as u64 - 1);
    format!(
        "{}, per extra region {}",
        memory::line(
            &format!("{REGIONS} regions"),
            "csharp",
            &[("idle".to_string(), four)]
        ),
        memory::mib(extra)
    )
}

/// Idle growth per prim, in KiB with one decimal.
fn per_prim(empty: Sample, loaded: Sample, count: u32) -> String {
    let grown = loaded.rss_kib.saturating_sub(empty.rss_kib) * 10;
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
    fn the_extra_region_cost_is_a_third_of_the_growth_to_four() {
        assert_eq!(
            per_region_line(kib(300 * 1024), kib(360 * 1024)),
            "memory: 4 regions csharp idle 360.0 MiB, peak 360.0 MiB, per extra region 20.0 MiB"
        );
    }

    #[test]
    fn the_prim_cost_is_the_growth_over_the_empty_region() {
        assert_eq!(per_prim(kib(17_000), kib(97_000), 40_000), "2.0 KiB");
        assert_eq!(per_prim(kib(17_000), kib(16_000), 500), "0.0 KiB");
    }
}
