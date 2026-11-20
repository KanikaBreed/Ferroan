// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every login under `xtask/src` goes through `login_when_settled`.
//! Scenario logins call the harness hook first; the bot's own `login` stays
//! in `tools/bot`.

use std::fs;
use std::path::Path;

pub(crate) fn check(root: &Path) -> bool {
    let mut problems = Vec::new();
    match fs::read_to_string(root.join("tools/bot/src/scenario.rs")) {
        Ok(text) => match function_body(&text, "async fn enter(") {
            Some(enter) => match (enter.find("before_login"), enter.find("login::login")) {
                (Some(hook), Some(call)) if hook < call => {}
                _ => problems.push(
                    "tools/bot/src/scenario.rs enter must call before_login before login::login"
                        .to_string(),
                ),
            },
            None => problems.push("tools/bot/src/scenario.rs has no enter function".to_string()),
        },
        Err(error) => problems.push(format!("could not read tools/bot/src/scenario.rs: {error}")),
    }
    match fs::read_to_string(root.join("xtask/src/grid_probe.rs")) {
        Ok(text) => {
            for needle in [
                "fn await_settled",
                "fn login_when_settled",
                "ferroan_bot::login(",
            ] {
                if !text.contains(needle) {
                    problems.push(format!("xtask/src/grid_probe.rs is missing {needle}"));
                }
            }
        }
        Err(error) => problems.push(format!("could not read xtask/src/grid_probe.rs: {error}")),
    }
    for (relative, needle) in [
        ("xtask/src/ferrobot.rs", "before_login"),
        ("xtask/src/ferrobot.rs", "login_when_settled"),
        ("xtask/src/inventory_probe.rs", "login_when_settled"),
        ("xtask/src/limits_probe.rs", "login_when_settled"),
    ] {
        match fs::read_to_string(root.join(relative)) {
            Ok(text) if text.contains(needle) => {}
            Ok(_) => problems.push(format!("{relative} must call {needle}")),
            Err(error) => problems.push(format!("could not read {relative}: {error}")),
        }
    }
    let mut sources = Vec::new();
    collect_rust(&root.join("xtask/src"), &mut sources);
    for path in sources {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if name == "grid_probe.rs" || name == "login_gate.rs" {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                problems.push(format!("could not read {}: {error}", path.display()));
                continue;
            }
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if direct_login(&text) {
            problems.push(format!(
                "{relative} calls login directly; use login_when_settled"
            ));
        }
    }
    if problems.is_empty() {
        return true;
    }
    for problem in &problems {
        eprintln!("login gate: {problem}");
    }
    false
}

fn direct_login(text: &str) -> bool {
    if text.contains("ferroan_bot::login(") || text.contains("login::login(") {
        return true;
    }
    for line in text.lines() {
        if line.contains("use ") && line.contains("ferroan_bot::login") && imports_login_fn(line) {
            return true;
        }
        if line_calls_login(line) {
            return true;
        }
    }
    false
}

fn imports_login_fn(line: &str) -> bool {
    let Some((_, names)) = line.split_once('{') else {
        return false;
    };
    names
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|name| name == "login")
}

fn line_calls_login(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index + 6 <= bytes.len() {
        if &bytes[index..index + 6] == b"login(" {
            let preceded = index
                .checked_sub(1)
                .and_then(|at| bytes.get(at))
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            if !preceded {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn function_body<'a>(text: &'a str, signature: &str) -> Option<&'a str> {
    let start = text.find(signature)?;
    let rest = &text[start + signature.len()..];
    let end = rest.find("\nasync fn ").or_else(|| rest.find("\nfn "));
    Some(match end {
        Some(end) => &rest[..end],
        None => rest,
    })
}

fn collect_rust(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
