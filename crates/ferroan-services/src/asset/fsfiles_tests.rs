// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! File layout and spool writer. The C# cross-read runs when
//! `cargo xtask asset fsassets-files` sets `FERROAN_FSASSET_FILES`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{
    FsFiles, WRITER_INTERVAL_MS, WRITER_MAX_RETRY_MS, WriterPass, hash_to_file, hash_to_path,
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
    assert_eq!(next_writer_interval(1000, WriterPass::Drained), 1000);
    assert_eq!(next_writer_interval(30_000, WriterPass::Drained), 1000);
    assert_eq!(next_writer_interval(0, WriterPass::Retry), 2000);
    assert_eq!(next_writer_interval(1000, WriterPass::Retry), 2000);
    assert_eq!(next_writer_interval(30_000, WriterPass::Retry), 60_000);
    assert_eq!(next_writer_interval(40_000, WriterPass::Retry), 60_000);
    assert_eq!(next_writer_interval(60_000, WriterPass::Retry), 60_000);
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
fn a_store_reads_from_the_spool_then_from_the_writer_gzip() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"ferroan fsassets file";
    let hash = sha256_hex(data);
    files.write_spool(&hash, data).unwrap();
    assert!(!files.gzip_path(&hash).exists());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
    assert_eq!(files.writer_pass(), WriterPass::Drained);
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

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Sync {
    File,
    Directory(PathBuf),
}

#[cfg(unix)]
thread_local! {
    static SYNCS: std::cell::RefCell<Option<Vec<Sync>>> = const { std::cell::RefCell::new(None) };
    static FAIL_SYNC: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
pub(super) fn record_sync(event: Sync) -> std::io::Result<()> {
    let fail = matches!(&event, Sync::Directory(dir) if FAIL_SYNC.with_borrow(|path| path.as_ref() == Some(dir)));
    SYNCS.with_borrow_mut(|syncs| {
        if let Some(syncs) = syncs {
            syncs.push(event);
        }
    });
    if fail {
        return Err(std::io::Error::other("injected directory sync failure"));
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn new_directory_entries_are_synced_and_the_spool_outlives_gzip_publication() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let hash = sha256_hex(b"durable");
    SYNCS.set(Some(Vec::new()));
    let temp_dir = dir.spool().join("spool");
    FAIL_SYNC.set(Some(temp_dir.clone()));
    files.write_spool(&hash, b"durable").unwrap();
    FAIL_SYNC.set(None);
    assert_eq!(
        SYNCS.replace(Some(Vec::new())).unwrap(),
        [
            Sync::Directory(dir.root.clone()),
            Sync::Directory(dir.spool()),
            Sync::File,
            Sync::Directory(dir.spool()),
            Sync::Directory(temp_dir)
        ]
    );

    let gzip = files.gzip_path(&hash);
    let leaf = gzip.parent().unwrap();
    FAIL_SYNC.set(Some(leaf.to_owned()));
    assert_eq!(files.writer_pass(), WriterPass::Retry);
    FAIL_SYNC.set(None);
    assert!(gzip.is_file());
    assert!(dir.spool().join(format!("{hash}.asset")).is_file());
    let mut expected: Vec<_> = leaf
        .ancestors()
        .skip(1)
        .take_while(|path| *path != dir.root)
        .chain([dir.root.as_path()])
        .map(|dir| Sync::Directory(dir.to_owned()))
        .collect();
    expected.reverse();
    expected.extend([Sync::File, Sync::Directory(leaf.to_owned())]);
    assert_eq!(SYNCS.replace(Some(Vec::new())).unwrap(), expected);
    assert_eq!(files.writer_pass(), WriterPass::Drained);
    assert_eq!(
        SYNCS.take().unwrap(),
        [
            Sync::File,
            Sync::Directory(leaf.to_owned()),
            Sync::Directory(dir.spool())
        ]
    );
    assert!(!dir.spool().join(format!("{hash}.asset")).exists());
}

#[test]
fn either_writer_may_have_already_removed_the_spool_copy() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let hash = sha256_hex(b"taken");
    let spool = files.spool().join(format!("{hash}.asset"));
    let dest = files.gzip_path(&hash);
    assert!(!super::write_store_file(&spool, &dest, &hash).unwrap());
    assert!(!dest.exists());
    super::remove_spool(&spool).unwrap();
}

#[test]
fn overlapping_publications_have_independent_temporary_files() {
    use std::io::Write;
    let dir = Scratch::new();
    let dest = dir.root.join("same.gz");
    super::publish_file(&dir.root, &dest, |mut first| {
        first.write_all(b"first")?;
        super::publish_file(&dir.root, &dest, |mut second| {
            second.write_all(b"second")?;
            second.sync_all()
        })?;
        assert_eq!(fs::read(&dest)?, b"second");
        first.sync_all()
    })
    .unwrap();
    assert_eq!(fs::read(dest).unwrap(), b"first");
    assert_eq!(fs::read_dir(dir.root.as_path()).unwrap().count(), 1);
}

#[test]
fn writer_leaves_another_temp_file_and_a_good_gzip_intact() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let data = b"good gzip";
    let hash = sha256_hex(data);
    let gzip = files.gzip_path(&hash);
    fs::create_dir_all(gzip.parent().unwrap()).unwrap();
    let foreign_temp = gzip.with_extension(format!("gz.{}.tmp", std::process::id()));
    fs::write(&foreign_temp, b"other writer").unwrap();
    files.write_spool(&hash, data).unwrap();
    assert_eq!(files.writer_pass(), WriterPass::Drained);
    assert_eq!(fs::read(&foreign_temp).unwrap(), b"other writer");
    let good = fs::read(&gzip).unwrap();
    fs::write(files.spool().join(format!("{hash}.asset")), b"damage").unwrap();
    assert_eq!(files.read_checked(&hash, 0).unwrap().unwrap(), data);
    assert_eq!(files.writer_pass(), WriterPass::Drained);
    assert_eq!(fs::read(&gzip).unwrap(), good);
}

#[test]
fn an_unreadable_entry_does_not_abort_the_batch() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    let hash = sha256_hex(b"later");
    files.write_spool(&hash, b"later").unwrap();
    let missing = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
    assert!(files.flush_batch(std::iter::once(missing)));
    let denied = Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    assert!(!files.flush_batch(std::iter::once(denied).chain(fs::read_dir(dir.spool()).unwrap())));
    assert!(files.gzip_path(&hash).is_file());
}

#[cfg(unix)]
#[test]
fn a_failed_spool_directory_sync_must_succeed_on_retry() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    fs::create_dir_all(dir.spool().join("spool")).unwrap();
    let hash = sha256_hex(b"retry");
    FAIL_SYNC.set(Some(dir.spool()));
    assert!(files.write_spool(&hash, b"retry").is_err());
    assert!(files.needs_spool(&hash, 0, |_| false).is_err());
    FAIL_SYNC.set(None);
    assert!(!files.needs_spool(&hash, 0, |_| false).unwrap());
}

#[test]
fn bounded_passes_make_progress_past_ignored_and_failing_entries() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    fs::create_dir_all(files.spool()).unwrap();
    let hash = sha256_hex(b"blocked");
    fs::write(files.spool().join(format!("{hash}.asset")), b"blocked").unwrap();
    fs::create_dir_all(files.gzip_path(&hash)).unwrap();
    // More ignored entries than the budget cannot starve later work.
    for i in 0..super::SPOOL_ENTRIES_PER_PASS {
        fs::write(files.spool().join(format!("{i}.other")), []).unwrap();
    }
    let good = sha256_hex(b"good");
    fs::write(files.spool().join(format!("{good}.asset")), b"good").unwrap();
    let first = files.writer_pass();
    assert!(files.scan.lock().unwrap().is_some());
    assert!(ferroan_diag::render().contains("asset.fsassets-spool-pass"));
    let second = files.writer_pass();
    assert!(first == WriterPass::Retry || second == WriterPass::Retry);
    assert!(files.scan.lock().unwrap().is_none());
    assert!(files.gzip_path(&good).is_file());
    assert!(files.spool().join(format!("{hash}.asset")).is_file());
}

#[test]
fn successful_spool_batches_continue_immediately_until_the_cursor_is_drained() {
    let dir = Scratch::new();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    fs::create_dir_all(files.spool()).unwrap();
    for i in 0..=super::SPOOL_ENTRIES_PER_PASS {
        fs::write(files.spool().join(format!("{i}.other")), []).unwrap();
    }
    let first = files.writer_pass();
    assert_eq!(first, WriterPass::More);
    assert_eq!(next_writer_interval(WRITER_MAX_RETRY_MS, first), 0);
    let last = files.writer_pass();
    assert_eq!(last, WriterPass::Drained);
    assert_eq!(next_writer_interval(0, last), WRITER_INTERVAL_MS);
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
    fs::create_dir(&gzip).unwrap();
    assert_eq!(files.write_once().unwrap(), WriterPass::Retry);
    assert!(dir.spool().join(format!("{hash}.asset")).is_file());
    assert!(gzip.is_dir());
    assert_eq!(fs::read(dir.spool().join("notes.txt")).unwrap(), b"keep");
    assert_eq!(
        fs::read(dir.spool().join("spool").join("nested.asset")).unwrap(),
        b"nested"
    );
    assert!(dir.spool().join("folder.asset").is_dir());

    fs::remove_dir(&gzip).unwrap();
    assert_eq!(files.writer_pass(), WriterPass::Drained);
    assert!(!dir.spool().join(format!("{hash}.asset")).exists());
    assert_eq!(files.read_checked(&hash, 1).unwrap().unwrap(), data);
    assert_eq!(fs::read(dir.spool().join("notes.txt")).unwrap(), b"keep");
    assert_eq!(files.writer_pass(), WriterPass::Drained);
}

#[test]
fn a_writer_pass_on_a_blocked_spool_does_not_end_the_process() {
    let dir = Scratch::new();
    fs::create_dir_all(dir.base()).unwrap();
    fs::write(dir.spool(), b"not a directory").unwrap();
    let files = FsFiles::new(dir.base(), dir.spool(), false);
    assert_eq!(files.writer_pass(), WriterPass::Retry);
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
    #[cfg(unix)]
    SYNCS.set(Some(Vec::new()));
    assert!(files.store_bytes(&hash, 1, data).unwrap());
    #[cfg(unix)]
    {
        // Renaming the obstacle and creating its replacement each sync BaseDirectory.
        let base = Sync::Directory(dir.base());
        assert!(
            SYNCS
                .take()
                .unwrap()
                .windows(2)
                .any(|events| events == [base.clone(), base.clone()])
        );
    }
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

    let shadow = dir.spool().join(format!("{hash}.asset"));
    fs::write(&shadow, b"damaged spool").unwrap();
    let pin = csharp(&oracle, "checked", &[&base, &spool, "0", &hash, "1"]);
    assert!(pin["data"].is_null(), "Pin8 damaged-spool result changed");
    assert_eq!(rust_files.read_checked(&hash, 1).unwrap().unwrap(), data);
    println!(
        "known difference TB91: damaged spool masks good gzip on Pin8; Ferroan reads the gzip"
    );
    assert_eq!(rust_files.writer_pass(), WriterPass::Drained);
    assert_eq!(rust_files.read_checked(&hash, 1).unwrap().unwrap(), data);
    let damaged = rust_files.gzip_path(&hash);
    let full = fs::read(&damaged).unwrap();
    fs::write(&damaged, &full[..full.len().min(12)]).unwrap();
    let pin = csharp(&oracle, "checked", &[&base, &spool, "0", &hash, "1"]);
    assert!(pin["data"].is_null(), "{pin}");
    assert!(rust_files.read_checked(&hash, 1).unwrap().is_none());

    let missing = sha256_hex(b"no-file-for-this-hash");
    let absent = csharp(&oracle, "checked", &[&base, &spool, "0", &missing, "1"]);
    assert!(absent["data"].is_null(), "{absent}");
    assert!(rust_files.read_checked(&missing, 1).unwrap().is_none());
    println!("fsassets files: paths and decompressed bytes match");
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

impl FsFiles {
    fn store_bytes(
        &self,
        hash: &str,
        type_code: i8,
        data: &[u8],
    ) -> Result<bool, super::StoreError> {
        if self.needs_spool(hash, type_code, |stored| stored == data)? {
            self.write_spool(hash, data)?;
        }
        Ok(self.write_once()? != WriterPass::Retry)
    }
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
