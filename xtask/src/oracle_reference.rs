// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Check out the `docs/REFERENCES.md` pins in task-owned clones.
//!
//! The shared checkout is an object source. Its HEAD is not what gets built.
//! A pin that cannot be checked out stops the command and names both revisions.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Pins {
    pub tranquillity: String,
    pub libopenmetaverse: String,
}

pub fn prepare(reference: PathBuf) -> Result<PathBuf, String> {
    let destination = crate::repo_root().join(".scratch/oracle-sources");
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let pins = read_pins()?;
    for (name, pin) in [
        ("OpenSim-Tranquillity", pins.tranquillity.as_str()),
        ("libopenmetaverse", pins.libopenmetaverse.as_str()),
    ] {
        let source = reference.join(name);
        if !source.exists() {
            continue;
        }
        checkout_pin(&source, &destination.join(name), name, pin, false)?;
    }
    Ok(destination)
}

pub fn read_pins() -> Result<Pins, String> {
    let path = crate::repo_root().join("docs/REFERENCES.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    pins_from(&text)
}

pub fn pins_from(markdown: &str) -> Result<Pins, String> {
    Ok(Pins {
        tranquillity: row_revision(markdown, "OpenSim-Tranquillity")?,
        libopenmetaverse: row_revision(markdown, "libopenmetaverse")?,
    })
}

pub fn mismatch(name: &str, actual: &str, pin: &str) -> String {
    format!("{name} is at {actual}, docs/REFERENCES.md pins {pin}")
}

/// `git rev-parse HEAD` of a clone that must already be `pin`.
pub fn require_head(checkout: &Path, name: &str, pin: &str) -> Result<String, String> {
    let actual = git(checkout, &["rev-parse", "HEAD"])?;
    if actual != pin {
        return Err(mismatch(name, &actual, pin));
    }
    Ok(actual)
}

/// Clone `source` into `target` when needed and check out `pin`.
///
/// `source` is never modified. `reset_target` cleans `target` before the
/// checkout; otherwise a dirty clone is left untouched and the call fails.
/// The returned revision is `git rev-parse HEAD` after the checkout.
pub fn checkout_pin(
    source: &Path,
    target: &Path,
    name: &str,
    pin: &str,
    reset_target: bool,
) -> Result<String, String> {
    if same_dir(source, target)? {
        return Err("refusing to modify the reference checkout".to_string());
    }
    let head = git(source, &["rev-parse", "HEAD"])?;
    let status = git(source, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "{}\nThe reference checkout is not clean:\n{status}",
            mismatch(name, &head, pin)
        ));
    }
    if target.exists() && !target.join(".git").exists() {
        return Err(format!(
            "{} exists and is not a git clone",
            target.display()
        ));
    }
    if !target.join(".git").exists() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let cloned = Command::new("git")
            .args(["clone", "--no-hardlinks", "--no-checkout"])
            .arg(source)
            .arg(target)
            .output()
            .map_err(|error| error.to_string())?;
        if !cloned.status.success() {
            let detail = String::from_utf8_lossy(&cloned.stderr);
            return Err(format!(
                "{} (could not clone: {})",
                mismatch(name, &head, pin),
                detail.trim()
            ));
        }
    } else if !git(target, &["status", "--porcelain"])?.is_empty() && !reset_target {
        return Err(format!(
            "oracle source {name} contains edits; preserve them before rebuilding"
        ));
    }
    fetch_pin(target, source, name, pin, &head)?;
    let mut checkout = vec!["checkout", "--detach", pin];
    if reset_target {
        checkout.insert(1, "--force");
    }
    if let Err(error) = git(target, &checkout) {
        return Err(format!("{} ({error})", mismatch(name, &head, pin)));
    }
    if reset_target {
        git(target, &["clean", "-fdx"])?;
    }
    require_head(target, name, pin)
}

pub fn fetch_pin(
    target: &Path,
    source: &Path,
    name: &str,
    pin: &str,
    source_head: &str,
) -> Result<(), String> {
    if git(target, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_ok() {
        return Ok(());
    }
    let source_arg = source
        .to_str()
        .ok_or("oracle reference path is not UTF-8")?;
    if let Err(error) = git(target, &["fetch", source_arg, pin]) {
        return Err(format!("{} ({error})", mismatch(name, source_head, pin)));
    }
    if git(target, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err() {
        return Err(mismatch(name, source_head, pin));
    }
    Ok(())
}

fn row_revision(markdown: &str, checkout: &str) -> Result<String, String> {
    let marker = format!("| `{checkout}`");
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(&marker) {
            continue;
        }
        let bytes = trimmed.as_bytes();
        let mut index = 0;
        while index + 42 <= bytes.len() {
            if bytes[index] == b'`' && bytes[index + 41] == b'`' {
                let hash = &trimmed[index + 1..index + 41];
                if hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Ok(hash.to_ascii_lowercase());
                }
            }
            index += 1;
        }
        return Err(format!("{checkout} row has no 40-character revision"));
    }
    Err(format!("docs/REFERENCES.md has no {checkout} row"))
}

fn same_dir(left: &Path, right: &Path) -> Result<bool, String> {
    if !left.exists() || !right.exists() {
        return Ok(false);
    }
    let left = left.canonicalize().map_err(|error| error.to_string())?;
    let right = right.canonicalize().map_err(|error| error.to_string())?;
    Ok(left == right)
}

fn git(directory: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let verb = args.first().copied().unwrap_or("git");
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {verb} failed: {}", detail.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Pin Test",
                "-c",
                "user.email=pin-test@example.com",
            ])
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn work(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ferroan-qr49-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_pair(dir: &Path) -> (String, String) {
        fs::create_dir_all(dir).unwrap();
        let init = Command::new("git")
            .args(["init", "-b", "main"])
            .arg(dir)
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        fs::write(dir.join("marker"), "one\n").unwrap();
        git_ok(dir, &["add", "marker"]);
        git_ok(dir, &["commit", "-m", "pin"]);
        let pin = git_ok(dir, &["rev-parse", "HEAD"]);
        fs::write(dir.join("marker"), "two\n").unwrap();
        git_ok(dir, &["add", "marker"]);
        git_ok(dir, &["commit", "-m", "head"]);
        let head = git_ok(dir, &["rev-parse", "HEAD"]);
        assert_ne!(pin, head);
        (pin, head)
    }

    #[test]
    fn references_md_names_both_pins() {
        let pins = pins_from(include_str!("../../docs/REFERENCES.md")).unwrap();
        assert_eq!(pins.tranquillity.len(), 40);
        assert_eq!(pins.libopenmetaverse.len(), 40);
        assert!(
            pins.tranquillity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert!(
            pins.libopenmetaverse
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert_ne!(pins.tranquillity, pins.libopenmetaverse);
    }

    #[test]
    fn an_off_pin_reference_is_checked_out_in_the_clone() {
        let root = work("correct");
        let reference = root.join("reference");
        let (tranq_pin, tranq_head) = init_pair(&reference.join("OpenSim-Tranquillity"));
        let (omv_pin, omv_head) = init_pair(&reference.join("libopenmetaverse"));
        let destination = root.join("oracle-sources");
        checkout_pin(
            &reference.join("OpenSim-Tranquillity"),
            &destination.join("OpenSim-Tranquillity"),
            "OpenSim-Tranquillity",
            &tranq_pin,
            false,
        )
        .unwrap();
        checkout_pin(
            &reference.join("libopenmetaverse"),
            &destination.join("libopenmetaverse"),
            "libopenmetaverse",
            &omv_pin,
            false,
        )
        .unwrap();
        assert_eq!(
            git_ok(
                &destination.join("OpenSim-Tranquillity"),
                &["rev-parse", "HEAD"]
            ),
            tranq_pin
        );
        assert_eq!(
            git_ok(
                &destination.join("libopenmetaverse"),
                &["rev-parse", "HEAD"]
            ),
            omv_pin
        );
        assert_eq!(
            git_ok(
                &reference.join("OpenSim-Tranquillity"),
                &["rev-parse", "HEAD"]
            ),
            tranq_head
        );
        assert_eq!(
            git_ok(&reference.join("libopenmetaverse"), &["rev-parse", "HEAD"]),
            omv_head
        );
        assert!(
            git_ok(
                &reference.join("OpenSim-Tranquillity"),
                &["status", "--porcelain"]
            )
            .is_empty()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_existing_clone_at_the_wrong_commit_is_corrected() {
        let root = work("existing");
        let source = root.join("source");
        let (pin, head) = init_pair(&source);
        let target = root.join("clone");
        let cloned = Command::new("git")
            .args(["clone", "--no-hardlinks"])
            .arg(&source)
            .arg(&target)
            .output()
            .unwrap();
        assert!(cloned.status.success());
        assert_eq!(git_ok(&target, &["rev-parse", "HEAD"]), head);
        let actual = checkout_pin(&source, &target, "OpenSim-Tranquillity", &pin, false).unwrap();
        assert_eq!(actual, pin);
        assert_eq!(git_ok(&target, &["rev-parse", "HEAD"]), pin);
        assert_eq!(git_ok(&source, &["rev-parse", "HEAD"]), head);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shallow_clone_fetches_the_missing_pin() {
        let root = work("shallow");
        let source = root.join("source");
        let (pin, _head) = init_pair(&source);
        let target = root.join("shallow");
        // A path clone ignores --depth and would already contain the pin.
        let cloned = Command::new("git")
            .args(["clone", "--depth", "1", "--no-checkout", "--no-local"])
            .arg(&source)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            cloned.status.success(),
            "{}",
            String::from_utf8_lossy(&cloned.stderr)
        );
        assert!(
            git(&target, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err(),
            "the shallow clone already had the pin"
        );
        let actual = checkout_pin(&source, &target, "OpenSim-Tranquillity", &pin, true).unwrap();
        assert_eq!(actual, pin);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_pin_names_both_revisions() {
        let root = work("missing");
        let source = root.join("source");
        let (_pin, head) = init_pair(&source);
        let missing = "0123456789abcdef0123456789abcdef01234567";
        let target = root.join("clone");
        let error =
            checkout_pin(&source, &target, "OpenSim-Tranquillity", missing, false).unwrap_err();
        assert!(error.contains(&head), "{error}");
        assert!(error.contains(missing), "{error}");
        assert_eq!(git_ok(&source, &["rev-parse", "HEAD"]), head);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dirty_reference_names_both_revisions() {
        let root = work("dirty");
        let source = root.join("source");
        let (pin, head) = init_pair(&source);
        fs::write(source.join("extra"), "dirty\n").unwrap();
        let error = checkout_pin(
            &source,
            &root.join("clone"),
            "libopenmetaverse",
            &pin,
            false,
        )
        .unwrap_err();
        assert!(error.contains(&head), "{error}");
        assert!(error.contains(&pin), "{error}");
        assert!(error.contains("not clean"), "{error}");
        assert_eq!(git_ok(&source, &["rev-parse", "HEAD"]), head);
        assert!(source.join("extra").is_file());
        assert!(!root.join("clone").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
