// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Searches a Ferroan run's logs for secrets the harness itself generated.
//!
//! The flag is off unless the live harness turns it on, so a C# region log is
//! not scanned. Errors name the class of secret and never include its value.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static CHECK: AtomicBool = AtomicBool::new(false);
static EXTRA: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_log_redaction_check(enabled: bool, extra_dir: Option<PathBuf>) {
    let mut slot = EXTRA.lock().unwrap_or_else(|poison| poison.into_inner());
    *slot = extra_dir;
    CHECK.store(enabled, Ordering::Release);
}

pub fn scan_login(
    password: &str,
    session_id: &str,
    secure_session_id: &str,
    seed_capability: &str,
    region_log: &Path,
) -> Result<(), String> {
    if !CHECK.load(Ordering::Acquire) {
        return Ok(());
    }
    let extra = EXTRA
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    scan_files(
        password,
        session_id,
        secure_session_id,
        seed_capability,
        region_log,
        extra.as_deref(),
    )
}

pub fn scan_passwords(passwords: &[&str], region_log: &Path) -> Result<(), String> {
    for password in passwords {
        scan_login(password, "", "", "", region_log)?;
    }
    Ok(())
}

fn scan_files(
    password: &str,
    session_id: &str,
    secure_session_id: &str,
    seed_capability: &str,
    region_log: &Path,
    extra_dir: Option<&Path>,
) -> Result<(), String> {
    let mut files = vec![region_log.to_path_buf()];
    if let Some(dir) = extra_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }
    }
    let seed = seed_capability
        .find("/CAPS/")
        .map(|index| &seed_capability[index..])
        .filter(|path| path.len() >= 8);
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let label = if path == region_log {
            "region log"
        } else {
            "docker log"
        };
        if password.len() >= 8 && text.contains(password) {
            return Err(format!("{label} contains the account password"));
        }
        if session_id.len() >= 8 && text.contains(session_id) {
            return Err(format!("{label} contains the session id"));
        }
        if secure_session_id.len() >= 8 && text.contains(secure_session_id) {
            return Err(format!("{label} contains the secure session id"));
        }
        if let Some(seed) = seed
            && text.contains(seed)
        {
            return Err(format!("{label} contains the seed capability path"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::scan_files;

    #[test]
    fn the_error_names_the_class_and_not_the_value() {
        let dir = std::env::temp_dir().join(format!("ferroan-redact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let log = dir.join("region.log");
        let secret = "session-secret-value";
        std::fs::write(&log, format!("seen {secret}\n")).expect("log");
        let error = scan_files(
            secret,
            "22222222-2222-2222-2222-222222222222",
            "33333333-3333-3333-3333-333333333333",
            "http://127.0.0.1/CAPS/seed0000/",
            &log,
            None,
        )
        .expect_err("password");
        assert!(error.contains("account password"));
        assert!(!error.contains(secret));
        std::fs::write(&log, "clean login accepted\n").expect("log");
        scan_files(
            secret,
            "22222222-2222-2222-2222-222222222222",
            "33333333-3333-3333-3333-333333333333",
            "http://127.0.0.1/CAPS/seed0000/",
            &log,
            None,
        )
        .expect("clean");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
