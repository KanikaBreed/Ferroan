// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo xtask live diag` reads each server's loopback `/diag` page.

use std::fs;
use std::process::ExitCode;
use std::time::Duration;

use crate::live;

pub(crate) fn run(urls: &[String]) -> ExitCode {
    let urls = if urls.is_empty() {
        default_urls()
    } else {
        urls.to_vec()
    };
    let dir = live::diag_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("diag: could not create {}: {error}", dir.display());
        return ExitCode::from(1);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("diag: could not start runtime: {error}");
            return ExitCode::from(1);
        }
    };
    let mut saved = 0usize;
    for (index, url) in urls.iter().enumerate() {
        let target = url.trim_end_matches('/').to_owned() + "/diag";
        let path = dir.join(file_name(index, urls.len()));
        match runtime.block_on(fetch(&target)) {
            Ok(body) => {
                if let Err(error) = fs::write(&path, &body) {
                    eprintln!("diag: could not write {}: {error}", path.display());
                    continue;
                }
                println!("diag: wrote {}", path.display());
                saved += 1;
            }
            Err(error) => eprintln!("diag: {target}: {error}"),
        }
    }
    if saved == 0 {
        eprintln!("diag: no server answered");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn default_urls() -> Vec<String> {
    let region = std::env::var("FERROAN_REGION_METRICS")
        .ok()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:9109".to_owned());
    let robust = std::env::var("FERROAN_ROBUST_METRICS")
        .ok()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:9110".to_owned());
    vec![region, robust]
}

fn file_name(index: usize, count: usize) -> String {
    if count == 2 && index == 0 {
        "region.txt".to_owned()
    } else if count == 2 && index == 1 {
        "robust.txt".to_owned()
    } else {
        format!("server-{index}.txt")
    }
}

async fn fetch(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.without_url().to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !status.is_success() {
        return Err(format!("status {}", status.as_u16()));
    }
    Ok(body)
}
