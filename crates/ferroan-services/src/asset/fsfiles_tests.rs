// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! File layout and spool writer. The C# cross-read runs when
//! `cargo xtask asset fsassets-files` sets `FERROAN_FSASSET_FILES`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{
    FsFiles, WRITER_INTERVAL_MS, WRITER_MAX_RETRY_MS, hash_to_file, hash_to_path,
    next_writer_interval, sha256_hex,
};

const LAYOUT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn hash_layout_matches_the_pin() {
    assert_eq!(
        hash_to_path(LAYOUT, false),
        PathBuf::from("01").join("23").join("45").join("6789")
    );
    assert_eq!(
        hash_to_file(LAYOUT, false),
        PathBuf::from("01")
            .join("23")
            .join("45")
            .join("6789")
            .join(LAYOUT)
    );
    assert_eq!(hash_to_path(LAYOUT, true), PathBuf::from("012").join("345"));
    assert_eq!(
        hash_to_file(LAYOUT, true),
        PathBuf::from("012").join("345").join(LAYOUT)
    );
    assert_eq!(hash_to_path("short", false), PathBuf::from("junkyard"));
    assert_eq!(hash_to_path("", true), PathBuf::from("junkyard"));
    assert_eq!(
        sha256_hex(&[]),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn empty_content_needs_no_file() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let hash = sha256_hex(&[]);
    assert_eq!(files.read_checked(&hash, 1).unwrap(), Some(Vec::new()));
    assert!(!files.gzip_path(&hash).exists());
    assert!(files.store_bytes(&hash, 1, &[]).unwrap());
    assert!(!files.gzip_path(&hash).exists());
}

#[test]
fn writer_interval_backs_off_and_resets() {
    assert_eq!(WRITER_INTERVAL_MS, 1000);
    assert_eq!(WRITER_MAX_RETRY_MS, 60_000);
    assert_eq!(next_writer_interval(1000, true), 1000);
    assert_eq!(next_writer_interval(30_000, true), 1000);
    assert_eq!(next_writer_interval(0, false), 2000);
    assert_eq!(next_writer_interval(1000, false), 2000);
    assert_eq!(next_writer_interval(30_000, false), 60_000);
    assert_eq!(next_writer_interval(40_000, false), 60_000);
    assert_eq!(next_writer_interval(60_000, false), 60_000);
}

#[test]
fn a_temporary_spool_warns_and_another_path_does_not() {
    let dir = Scratch::new();
    let temporary = std::env::temp_dir().join(format!(
        "ferroan-fs-temp-spool-{}-{}",
        std::process::id(),
        unique()
    ));
    let _files = FsFiles::new(dir.base(), &temporary, false);
    let text = ferroan_diag::render();
    assert!(
        text.contains(&temporary.display().to_string()),
        "missing temporary-spool warning"
    );
    assert!(text.contains("as at a restart"));

    let kept = std::env::current_dir()
        .unwrap()
        .join(format!("ferroan-fs-not-temp-{}", unique()));
    let _files = FsFiles::new(dir.base(), &kept, false);
    assert!(!ferroan_diag::render().contains("ferroan-fs-not-temp-"));
}

#[cfg(unix)]
#[test]
fn a_path_beside_tmp_is_not_the_temporary_directory() {
    let dir = Scratch::new();
    let spool = PathBuf::from(format!("/tmpfoo/ferroan-not-tmp-{}", unique()));
    let _files = FsFiles::new(dir.base(), &spool, false);
    assert!(!ferroan_diag::render().contains(&spool.display().to_string()));
}

#[test]
fn store_writes_gzip_and_reads_it_back() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"ferroan fsassets file";
    let hash = sha256_hex(data);
    assert!(files.store_bytes(&hash, 1, data).unwrap());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
    assert!(files.gzip_path(&hash).is_file());
    assert!(!dir.spool().join(format!("{hash}.asset")).exists());
    assert!(
        dir.spool()
            .join("spool")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    let upper = hash.to_ascii_uppercase();
    let upper_files = FsFiles::new(dir.base(), dir.spool(), false);
    let upper_gzip = upper_files.gzip_path(&upper);
    // The path follows the hash text. A case-sensitive volume keeps the
    // lowercase file on another path. A case-insensitive volume already
    // has that file, and the hash compare still matches.
    if !upper_gzip.exists() {
        fs::create_dir_all(upper_gzip.parent().unwrap()).unwrap();
        fs::copy(files.gzip_path(&hash), &upper_gzip).unwrap();
    }
    assert_eq!(upper_files.read_checked(&upper, 1).unwrap().unwrap(), data);
}

#[test]
fn missing_empty_and_mismatching_files_are_missing_and_a_store_rewrites() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"bytes that must match the hash";
    let hash = sha256_hex(data);
    assert!(files.read_checked(&hash, 1).unwrap().is_none());

    let gzip = files.gzip_path(&hash);
    fs::create_dir_all(gzip.parent().unwrap()).unwrap();
    fs::write(&gzip, []).unwrap();
    assert!(files.read_checked(&hash, 1).unwrap().is_none());

    fs::write(&gzip, b"this is not gzip").unwrap();
    assert!(files.read_checked(&hash, 1).unwrap().is_none());
    assert!(ferroan_diag::render().contains("could not read"));

    let plain = dir.base().join(files.relative_file(&hash));
    fs::remove_file(&gzip).unwrap();
    fs::write(&plain, []).unwrap();
    assert!(files.read_checked(&hash, 1).unwrap().is_none());

    fs::write(&plain, b"not the hashed bytes").unwrap();
    assert!(files.read_checked(&hash, 1).unwrap().is_none());
    assert!(ferroan_diag::render().contains("do not match hash"));

    assert!(files.store_bytes(&hash, 1, data).unwrap());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
}

#[test]
fn an_object_matches_a_trailing_nul_or_a_whole_file() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let body = b"object-xml";
    let mut stored = body.to_vec();
    stored.push(0);
    let hash = sha256_hex(body);
    assert!(files.store_bytes(&hash, 6, &stored).unwrap());
    assert_eq!(files.read_checked(&hash, 6).unwrap().unwrap(), stored);

    let other = b"different-object";
    let expected = sha256_hex(b"expected-object");
    let plain = dir.base().join(files.relative_file(&expected));
    fs::create_dir_all(plain.parent().unwrap()).unwrap();
    fs::write(&plain, other).unwrap();
    assert_eq!(files.read_checked(&expected, 6).unwrap().unwrap(), other);
    assert!(files.read_checked(&expected, 1).unwrap().is_none());
}

#[test]
fn a_failed_spool_write_leaves_no_partial_file() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"spool failure";
    let hash = sha256_hex(data);
    fs::create_dir_all(dir.spool()).unwrap();
    fs::write(dir.spool().join("spool"), b"not a directory").unwrap();
    assert!(files.write_spool(&hash, data).is_err());
    assert!(!dir.spool().join(format!("{hash}.asset")).exists());
    assert!(dir.spool().join("spool").is_file());

    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    fs::create_dir_all(dir.spool().join("spool")).unwrap();
    fs::create_dir(dir.spool().join(format!("{hash}.asset"))).unwrap();
    assert!(files.write_spool(&hash, data).is_err());
    assert!(dir.spool().join(format!("{hash}.asset")).is_dir());
    let left: Vec<_> = fs::read_dir(dir.spool().join("spool"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "partial spool file left: {left:?}");
    assert!(ferroan_diag::render().contains("the asset is not stored"));
}

#[test]
fn the_writer_retries_and_leaves_other_files_alone() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"writer retry";
    let hash = sha256_hex(data);
    files.write_spool(&hash, data).unwrap();
    fs::write(dir.spool().join("notes.txt"), b"keep").unwrap();
    fs::create_dir_all(dir.spool().join("spool")).unwrap();
    fs::write(dir.spool().join("spool").join("nested.asset"), b"nested").unwrap();
    fs::create_dir(dir.spool().join("folder.asset")).unwrap();

    let gzip = files.gzip_path(&hash);
    fs::create_dir_all(gzip.parent().unwrap()).unwrap();
    let temp = gzip_temp(&gzip);
    fs::create_dir(&temp).unwrap();
    assert!(!files.write_once().unwrap());
    assert!(dir.spool().join(format!("{hash}.asset")).is_file());
    assert!(!gzip.exists());
    assert_eq!(fs::read(dir.spool().join("notes.txt")).unwrap(), b"keep");
    assert_eq!(
        fs::read(dir.spool().join("spool").join("nested.asset")).unwrap(),
        b"nested"
    );
    assert!(dir.spool().join("folder.asset").is_dir());

    fs::remove_dir(&temp).unwrap();
    assert!(files.writer_pass());
    assert!(!dir.spool().join(format!("{hash}.asset")).exists());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
    assert_eq!(fs::read(dir.spool().join("notes.txt")).unwrap(), b"keep");
    assert!(files.writer_pass());
}

#[test]
fn a_writer_pass_on_a_blocked_spool_does_not_end_the_process() {
    let dir = Scratch::new();
    fs::create_dir_all(dir.base()).unwrap();
    fs::write(dir.spool(), b"not a directory").unwrap();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    assert!(!files.writer_pass());
    assert!(ferroan_diag::render().contains("writer pass over"));
}

#[test]
fn a_blocking_file_is_renamed_aside_and_kept() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"asset past a chicken bone";
    let hash = sha256_hex(data);
    let first = files
        .relative_file(&hash)
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_os_string();
    fs::create_dir_all(dir.base()).unwrap();
    let bone = dir.base().join(&first);
    fs::write(&bone, b"sr-as-bone").unwrap();
    assert!(files.store_bytes(&hash, 1, data).unwrap());
    assert!(bone.is_dir());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
    let renamed = fs::read_dir(dir.base())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("{}.blocked-", first.to_string_lossy()))
        })
        .expect("renamed blocking file");
    assert_eq!(fs::read(&renamed).unwrap(), b"sr-as-bone");
    let text = ferroan_diag::render();
    assert!(text.contains("renamed blocking file"));
    assert!(text.contains("it is not an asset"));
}

#[test]
fn fsassets_files_cross_read() {
    let Ok(oracle) = std::env::var("FERROAN_ASSET_ORACLE") else {
        eprintln!("FERROAN_ASSET_ORACLE is unset; FSAssets file cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_FSASSET_FILES").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_FSASSET_FILES is unset; FSAssets file cross-read was not run");
        return;
    }
    let dir = Scratch::new();
    let rust_files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"cross-read-fsassets";
    let hash = sha256_hex(data);
    assert!(rust_files.store_bytes(&hash, 1, data).unwrap());
    let base = dir.base_arg();
    let spool = dir.spool_arg();
    let read = csharp(&oracle, "read", &[&base, &spool, "0", &hash]);
    assert_eq!(bytes_of(&read["data"]), data);
    assert_eq!(
        slash(&read["file"]),
        slash_path(&rust_files.relative_file(&hash))
    );

    let osgrid = csharp(&oracle, "path", &["1", LAYOUT]);
    assert_eq!(
        slash(&osgrid["file"]),
        slash_path(&hash_to_file(LAYOUT, true))
    );
    let normal = csharp(&oracle, "path", &["0", LAYOUT]);
    assert_eq!(
        slash(&normal["file"]),
        slash_path(&hash_to_file(LAYOUT, false))
    );

    let from_csharp = b"from-csharp-gzip";
    let csharp_hash = sha256_hex(from_csharp);
    let bytes_path = dir.root.join("csharp-bytes");
    fs::write(&bytes_path, from_csharp).unwrap();
    let cs_base = dir.root.join("cs-base");
    let cs_spool = dir.root.join("cs-spool");
    csharp(
        &oracle,
        "write",
        &[
            &cs_base.display().to_string(),
            &cs_spool.display().to_string(),
            "0",
            &csharp_hash,
            &bytes_path.display().to_string(),
        ],
    );
    let cs_files = FsFiles::new(&cs_base, &cs_spool, false);
    assert_eq!(
        cs_files.read_checked(&csharp_hash, 1).unwrap().unwrap(),
        from_csharp
    );

    let plain = b"from-csharp-plain";
    let plain_hash = sha256_hex(plain);
    fs::write(&bytes_path, plain).unwrap();
    csharp(
        &oracle,
        "plain",
        &[
            &cs_base.display().to_string(),
            &cs_spool.display().to_string(),
            "0",
            &plain_hash,
            &bytes_path.display().to_string(),
        ],
    );
    assert_eq!(
        cs_files.read_checked(&plain_hash, 1).unwrap().unwrap(),
        plain
    );

    let damaged = rust_files.gzip_path(&hash);
    let full = fs::read(&damaged).unwrap();
    fs::write(&damaged, &full[..full.len().min(12)]).unwrap();
    let pin = csharp(&oracle, "read", &[&base, &spool, "0", &hash]);
    assert_ne!(bytes_of(&pin["data"]), data);
    assert!(rust_files.read_checked(&hash, 1).unwrap().is_none());

    let missing = sha256_hex(b"no-file-for-this-hash");
    let absent = csharp(&oracle, "read", &[&base, &spool, "0", &missing]);
    assert!(bytes_of(&absent["data"]).is_empty());
    assert!(rust_files.read_checked(&missing, 1).unwrap().is_none());
    println!("fsassets files: paths and decompressed bytes match");
    println!("known differences: TB23");
}

fn csharp(oracle: &str, op: &str, args: &[&str]) -> serde_json::Value {
    let output = std::process::Command::new("dotnet")
        .arg(oracle)
        .arg("ffile")
        .arg(op)
        .args(args)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .expect("dotnet");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ffile {op} failed: {}",
        stderr.trim()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|error| panic!("ffile json {error}: {stdout}"))
}

fn bytes_of(value: &serde_json::Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|number| {
            let byte = number.as_u64().unwrap();
            assert!(byte < 256);
            byte as u8
        })
        .collect()
}

fn slash(value: &serde_json::Value) -> String {
    value.as_str().unwrap().replace('\\', "/")
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn gzip_temp(gzip: &Path) -> PathBuf {
    let mut name = gzip.as_os_str().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(name)
}

fn unique() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ferroan-fsfiles-{}-{}",
            std::process::id(),
            unique()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn base(&self) -> PathBuf {
        self.root.join("base")
    }

    fn spool(&self) -> PathBuf {
        self.root.join("spool")
    }

    fn base_arg(&self) -> String {
        self.base().display().to_string()
    }

    fn spool_arg(&self) -> String {
        self.spool().display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
