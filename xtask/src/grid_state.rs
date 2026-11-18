// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recovery records for grid processes. These contain no command arguments or
//! credentials: an exact argv fingerprint supplements PID, start time and exe.

#![cfg(target_os = "linux")]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const MAX_STATE: usize = 64 * 1024;
const MAX_PROCESSES: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    pid: u32,
    start: u64,
    exe: String,
    args: u64,
}

impl Identity {
    fn capture(pid: u32) -> Result<Self, String> {
        let start = crate::live::process_start_time(pid)
            .ok_or("the process exited before its identity was recorded")?;
        let exe = fs::read_link(format!("/proc/{pid}/exe"))
            .map_err(|error| format!("could not read process executable: {error}"))?;
        let exe = exe
            .to_str()
            .ok_or("the process executable path is not UTF-8")?;
        // Rebuilding a Rust binary can unlink its old inode while it runs.
        let exe = exe.strip_suffix(" (deleted)").unwrap_or(exe).to_owned();
        if exe.len() > 4096 {
            return Err("the process executable path is too long".to_owned());
        }
        let args = read_limited(Path::new(&format!("/proc/{pid}/cmdline")), MAX_STATE)?;
        if args.is_empty() {
            return Err("the process has exited".to_owned());
        }
        Ok(Self {
            pid,
            start,
            exe,
            args: fingerprint(&args),
        })
    }

    fn live(&self) -> bool {
        Self::capture(self.pid).is_ok_and(|current| current == *self)
    }

    fn json(&self) -> Value {
        json!({"pid":self.pid,"start":self.start,"exe":self.exe,"args":self.args})
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let pid = value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 1)
            .ok_or("invalid grid process pid")?;
        let start = value
            .get("start")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or("invalid grid process start time")?;
        let exe = value
            .get("exe")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty() && value.len() <= 4096 && Path::new(value).is_absolute()
            })
            .ok_or("invalid grid process executable")?
            .to_owned();
        let args = value
            .get("args")
            .and_then(Value::as_u64)
            .ok_or("invalid grid process argv fingerprint")?;
        Ok(Self {
            pid,
            start,
            exe,
            args,
        })
    }
}

struct State {
    owner: Identity,
    processes: Vec<Identity>,
}

#[derive(Clone)]
struct Store {
    path: PathBuf,
}

fn store() -> Store {
    Store {
        path: crate::live::live_root().join("grid-state"),
    }
}

impl Store {
    fn lock(&self) -> Result<fs::File, String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create grid state directory: {error}"))?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))
            .map_err(|error| format!("could not open grid state lock: {error}"))?;
        file.try_lock().map_err(|_| {
            "another grid command is active; interrupt it before cargo xtask live down".to_owned()
        })?;
        Ok(file)
    }

    fn read(&self) -> Result<Option<State>, String> {
        let bytes = match fs::File::open(&self.path) {
            Ok(file) => limited(file, MAX_STATE)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("could not read grid process state: {error}")),
        };
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid grid process state JSON")?;
        if value.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported grid process state version".to_owned());
        }
        let owner = Identity::parse(
            value
                .get("owner")
                .ok_or("grid process state has no owner")?,
        )?;
        let entries = value
            .get("processes")
            .and_then(Value::as_array)
            .filter(|items| items.len() <= MAX_PROCESSES)
            .ok_or("invalid grid process list or too many processes")?;
        let processes = entries
            .iter()
            .map(Identity::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(State { owner, processes }))
    }

    fn write(&self, state: &State) -> Result<(), String> {
        if state.processes.len() > MAX_PROCESSES {
            return Err("too many grid processes".to_owned());
        }
        let value = json!({"version":1,"owner":state.owner.json(),"processes":state.processes.iter().map(Identity::json).collect::<Vec<_>>()});
        let bytes =
            serde_json::to_vec(&value).map_err(|_| "could not serialize grid process state")?;
        if bytes.len() > MAX_STATE {
            return Err("grid process state exceeds size limit".to_owned());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create grid state directory: {error}"))?;
        }
        let temporary = self.path.with_extension("tmp");
        fs::write(&temporary, bytes)
            .map_err(|error| format!("could not write grid process state: {error}"))?;
        fs::rename(temporary, &self.path)
            .map_err(|error| format!("could not save grid process state: {error}"))
    }

    fn survivor(&self) -> Result<Option<u32>, String> {
        let Some(state) = self.read()? else {
            return Ok(None);
        };
        if state.owner.pid != std::process::id() && state.owner.live() {
            return Ok(Some(state.owner.pid));
        }
        Ok(state
            .processes
            .iter()
            .find(|identity| identity.live())
            .map(|identity| identity.pid))
    }

    fn claim(&self) -> Result<Run, String> {
        let lock = self.lock()?;
        if let Some(pid) = self.survivor()? {
            return Err(format!(
                "grid process {pid} is still running; cargo xtask live down stops an interrupted run"
            ));
        }
        let owner = Identity::capture(std::process::id())?;
        self.write(&State {
            owner: owner.clone(),
            processes: Vec::new(),
        })?;
        Ok(Run {
            store: self.clone(),
            owner,
            _lock: lock,
        })
    }

    fn record(&self, child: &Child) -> Result<Tracked, String> {
        // spawn's exec handshake can finish before /proc exposes the new
        // argv. Keep the child owned while waiting for its identity to settle.
        let deadline = Instant::now() + Duration::from_secs(1);
        let identity = loop {
            match Identity::capture(child.id()) {
                Ok(identity) => break identity,
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        };
        if process_group(identity.pid) != Some(identity.pid) {
            return Err("grid child is not its process group leader".to_owned());
        }
        let mut state = self.read()?.ok_or("grid process state was not claimed")?;
        if state.owner != Identity::capture(std::process::id())? {
            return Err("grid process state belongs to another command".to_owned());
        }
        state.processes.push(identity.clone());
        self.write(&state)?;
        Ok(Tracked {
            store: self.clone(),
            identity,
        })
    }

    fn forget(&self, identity: &Identity) -> Result<(), String> {
        if identity.live() {
            return Err("grid process is still alive".to_owned());
        }
        let Some(mut state) = self.read()? else {
            return Ok(());
        };
        state.processes.retain(|entry| entry != identity);
        self.write(&state)
    }

    fn down(&self) -> Result<(), String> {
        let _lock = self.lock()?;
        let Some(state) = self.read()? else {
            return Ok(());
        };
        if state.owner.live() {
            return Err(format!(
                "grid command {} is still running; interrupt it before cargo xtask live down",
                state.owner.pid
            ));
        }
        let mut failures = Vec::new();
        // Robust is recorded first. Stop regions before their grid services.
        for identity in state.processes.iter().rev() {
            if let Err(error) = stop_identity(identity) {
                failures.push(error);
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        fs::remove_file(&self.path)
            .map_err(|error| format!("could not remove grid process state: {error}"))
    }
}

pub(crate) struct Run {
    store: Store,
    owner: Identity,
    _lock: fs::File,
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Ok(Some(state)) = self.store.read()
            && state.owner == self.owner
            && !state.processes.iter().any(Identity::live)
        {
            let _ = fs::remove_file(&self.store.path);
        }
        // Explicit unlock also releases any descriptor inherited briefly by
        // another thread's child between fork and its close-on-exec step.
        let _ = self._lock.unlock();
    }
}

pub(crate) struct Tracked {
    store: Store,
    identity: Identity,
}

impl Tracked {
    pub(crate) fn forget(&self) -> Result<(), String> {
        self.store.forget(&self.identity)
    }
}

pub(crate) fn claim() -> Result<Run, String> {
    store().claim()
}
pub(crate) fn record(child: &Child) -> Result<Tracked, String> {
    store().record(child)
}
pub(crate) fn survivor() -> Result<Option<u32>, String> {
    store().survivor()
}
pub(crate) fn down() -> Result<(), String> {
    store().down()
}

fn process_group(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

fn stop_identity(identity: &Identity) -> Result<(), String> {
    if !identity.live() {
        return Ok(());
    }
    if process_group(identity.pid) != Some(identity.pid) {
        return Err(format!(
            "grid process {} is no longer its group leader",
            identity.pid
        ));
    }
    for (signal, wait) in [
        ("-TERM", Duration::from_secs(30)),
        ("-KILL", Duration::from_secs(5)),
    ] {
        if !identity.live() {
            return Ok(());
        }
        Command::new("kill")
            .args([signal, &format!("-{}", identity.pid)])
            .status()
            .map_err(|error| format!("could not signal grid process {}: {error}", identity.pid))?;
        let deadline = Instant::now() + wait;
        while identity.live() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
    }
    if identity.live() {
        Err(format!("grid process {} did not exit", identity.pid))
    } else {
        Ok(())
    }
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    limited(
        fs::File::open(path)
            .map_err(|error| format!("could not read process identity: {error}"))?,
        limit,
    )
}

fn limited(file: fs::File, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read process state: {error}"))?;
    if bytes.len() > limit {
        return Err("process state exceeds size limit".to_owned());
    }
    Ok(bytes)
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    struct TestDir(PathBuf);

    impl TestDir {
        fn store() -> (Self, Store) {
            let path = crate::repo_root()
                .join(".scratch")
                .join(format!("grid-state-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            (
                Self(path.clone()),
                Store {
                    path: path.join("state"),
                },
            )
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct TestChild(Child);

    impl TestChild {
        fn spawn() -> Self {
            Self(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "grid_state::tests::waiting_child", "--ignored"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .process_group(0)
                    .spawn()
                    .unwrap(),
            )
        }
    }

    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "subprocess fixture for process ownership tests"]
    fn waiting_child() {
        thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn interrupted_grid_processes_are_refused_on_restart_and_cleaned_by_down() {
        let (_directory, store) = TestDir::store();
        let run = store.claim().unwrap();
        let mut child = TestChild::spawn();
        store.record(&child.0).unwrap();
        let mut state = store.read().unwrap().unwrap();
        // Simulate the original controller having exited, while its child
        // remains. PID alone is insufficient: an old start token is dead.
        state.owner.start += 1;
        store.write(&state).unwrap();
        drop(run);
        assert_eq!(store.survivor().unwrap(), Some(child.0.id()));
        assert!(store.claim().is_err());
        store.down().unwrap();
        child.0.wait().unwrap();
        assert!(!store.path.exists());
        assert!(store.claim().is_ok());
    }

    #[test]
    fn stale_or_mismatched_identity_does_not_kill_an_unrelated_process() {
        for mismatch in ["start", "exe", "argv"] {
            let (_directory, store) = TestDir::store();
            let run = store.claim().unwrap();
            let mut child = TestChild::spawn();
            store.record(&child.0).unwrap();
            let mut state = store.read().unwrap().unwrap();
            state.owner.start += 1;
            match mismatch {
                "start" => state.processes[0].start += 1,
                "exe" => state.processes[0].exe.push_str("-other-worktree"),
                "argv" => state.processes[0].args ^= 1,
                _ => unreachable!(),
            }
            store.write(&state).unwrap();
            drop(run);
            store.down().unwrap();
            assert!(child.0.try_wait().unwrap().is_none(), "{mismatch}");
        }
    }

    #[test]
    fn active_controller_lock_prevents_concurrent_start_and_down() {
        let (_directory, store) = TestDir::store();
        let run = store.claim().unwrap();
        assert!(store.claim().is_err());
        assert!(store.down().is_err());
        drop(run);
        assert!(!store.path.exists());
        store.down().unwrap();
    }

    #[test]
    fn process_state_rejects_oversize_files_and_unbounded_or_invalid_process_lists() {
        let (_directory, store) = TestDir::store();
        fs::write(&store.path, vec![b' '; MAX_STATE + 1]).unwrap();
        assert!(store.read().err().unwrap().contains("size limit"));
        let identity = Identity::capture(std::process::id()).unwrap();
        let mut value = json!({"version":1,"owner":identity.json(),"processes":vec![identity.json(); MAX_PROCESSES + 1]});
        fs::write(&store.path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(store.read().is_err());
        value["processes"] = json!([]);
        value["owner"]["pid"] = json!(0);
        fs::write(&store.path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(store.read().is_err());
    }
}
