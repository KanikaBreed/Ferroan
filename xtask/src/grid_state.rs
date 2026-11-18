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
const SPAWN_MARKER: &str = "FERROAN_GRID_SPAWN_ID";
const MAX_PROC_ENTRIES: usize = 32_768;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct Pending {
    marker: String,
    exe: String,
    cwd: String,
    args: u64,
    after: u64,
}

impl Pending {
    fn json(&self) -> Value {
        json!({"marker":self.marker,"exe":self.exe,"cwd":self.cwd,"args":self.args,"after":self.after})
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let marker = value
            .get("marker")
            .and_then(Value::as_str)
            .filter(|text| {
                text.len() == 36 && uuid::Uuid::parse_str(text).is_ok_and(|id| !id.is_nil())
            })
            .ok_or("invalid pending grid launch marker")?
            .to_owned();
        let path = |key| -> Result<String, String> {
            let text = value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= 4096)
                .ok_or("invalid pending grid launch path")?;
            let root = fs::canonicalize(crate::repo_root())
                .map_err(|_| "could not resolve the worktree root")?;
            if !Path::new(text).is_absolute() || !Path::new(text).starts_with(root) {
                return Err("pending grid launch is outside this worktree".to_owned());
            }
            Ok(text.to_owned())
        };
        Ok(Self {
            marker,
            exe: path("exe")?,
            cwd: path("cwd")?,
            args: value
                .get("args")
                .and_then(Value::as_u64)
                .ok_or("invalid pending grid argv fingerprint")?,
            after: value
                .get("after")
                .and_then(Value::as_u64)
                .filter(|start| *start > 0)
                .ok_or("invalid pending grid launch start bound")?,
        })
    }

    fn matches(&self, identity: &Identity) -> bool {
        identity.exe == self.exe
            && identity.args == self.args
            && fs::read_link(format!("/proc/{}/cwd", identity.pid))
                .is_ok_and(|cwd| cwd == Path::new(&self.cwd))
            && process_group(identity.pid) == Some(identity.pid)
    }
}

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
    pending: Option<Pending>,
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
        let pending = match value.get("pending") {
            None | Some(Value::Null) => None,
            Some(value) => Some(Pending::parse(value)?),
        };
        if pending.is_some() && processes.len() >= MAX_PROCESSES {
            return Err("too many grid processes and pending launches".to_owned());
        }
        Ok(Some(State {
            owner,
            processes,
            pending,
        }))
    }

    fn write(&self, state: &State) -> Result<(), String> {
        if state.processes.len() > MAX_PROCESSES {
            return Err("too many grid processes".to_owned());
        }
        let value = json!({"version":1,"owner":state.owner.json(),"processes":state.processes.iter().map(Identity::json).collect::<Vec<_>>(),"pending":state.pending.as_ref().map(Pending::json)});
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
        let Some(mut state) = self.read()? else {
            return Ok(None);
        };
        if state.owner.pid != std::process::id() && state.owner.live() {
            return Ok(Some(state.owner.pid));
        }
        if state.pending.is_some() && !state.owner.live() {
            let _lock = self.lock()?;
            state = self
                .read_recovered()?
                .ok_or("grid process state disappeared during recovery")?;
        }
        Ok(state
            .processes
            .iter()
            .find(|identity| identity.live())
            .map(|identity| identity.pid))
    }

    fn claim(&self) -> Result<Run, String> {
        let lock = self.lock()?;
        let state = self.read_recovered()?;
        let survivor = state.as_ref().and_then(|state| {
            if state.owner.pid != std::process::id() && state.owner.live() {
                Some(state.owner.pid)
            } else {
                state
                    .processes
                    .iter()
                    .find(|identity| identity.live())
                    .map(|identity| identity.pid)
            }
        });
        if let Some(pid) = survivor {
            return Err(format!(
                "grid process {pid} is still running; cargo xtask live down stops an interrupted run"
            ));
        }
        let owner = Identity::capture(std::process::id())?;
        self.write(&State {
            owner: owner.clone(),
            processes: Vec::new(),
            pending: None,
        })?;
        Ok(Run {
            store: self.clone(),
            owner,
            _lock: lock,
        })
    }

    fn prepare(&self, binary: &Path, args: &[&str], cwd: &Path) -> Result<Launch, String> {
        let mut state = self.read()?.ok_or("grid process state was not claimed")?;
        if state.owner != Identity::capture(std::process::id())? {
            return Err("grid process state belongs to another command".to_owned());
        }
        if state.pending.is_some() || state.processes.len() >= MAX_PROCESSES {
            return Err("grid already has a pending launch or too many processes".to_owned());
        }
        let canonical = |path| -> Result<String, String> {
            fs::canonicalize(path)
                .map_err(|_| "could not resolve grid launch path")?
                .into_os_string()
                .into_string()
                .map_err(|_| "grid launch path is not UTF-8".to_owned())
        };
        let mut argv = binary.as_os_str().as_encoded_bytes().to_vec();
        argv.push(0);
        for arg in args {
            if argv.len().saturating_add(arg.len()).saturating_add(1) > MAX_STATE {
                return Err("grid launch arguments exceed size limit".to_owned());
            }
            argv.extend_from_slice(arg.as_bytes());
            argv.push(0);
        }
        let pending = Pending {
            marker: uuid::Uuid::new_v4().to_string(),
            exe: canonical(binary)?,
            cwd: canonical(cwd)?,
            args: fingerprint(&argv),
            after: state.owner.start,
        };
        Pending::parse(&pending.json())?;
        state.pending = Some(pending.clone());
        self.write(&state)?;
        Ok(Launch {
            store: self.clone(),
            pending,
        })
    }

    fn record(&self, child: &Child, pending: &Pending) -> Result<Tracked, String> {
        // spawn's exec handshake can finish before /proc exposes the new
        // argv. Keep the child owned while waiting for its identity to settle.
        let deadline = Instant::now() + Duration::from_secs(1);
        let identity = loop {
            match Identity::capture(child.id()) {
                Ok(identity) if pending.matches(&identity) => break identity,
                Ok(_) if Instant::now() >= deadline => {
                    return Err("grid launch identity did not match its intent".to_owned());
                }
                Ok(_) => thread::sleep(Duration::from_millis(5)),
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
        if state.pending.as_ref().map(|intent| &intent.marker) != Some(&pending.marker) {
            return Err("grid launch intent changed before its process was recorded".to_owned());
        }
        state.processes.push(identity.clone());
        state.pending = None;
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
        let Some(state) = self.read_recovered()? else {
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

    /// Called only under the controller lock. A forked child keeps that lock
    /// until exec closes its inherited descriptor, including after SIGKILL of
    /// its parent. Thus recovery cannot race a child that has not exec'd yet.
    fn read_recovered(&self) -> Result<Option<State>, String> {
        let Some(mut state) = self.read()? else {
            return Ok(None);
        };
        if !state.owner.live()
            && let Some(pending) = &state.pending
        {
            let mut recovered = scan_pending(pending)?;
            if recovered.is_empty() {
                // /proc can briefly expose empty argv/environment at exec.
                thread::sleep(Duration::from_millis(25));
                recovered = scan_pending(pending)?;
            }
            if recovered.len() > 1 {
                return Err(
                    "multiple processes match one pending grid launch; state preserved".to_owned(),
                );
            }
            state.processes.extend(recovered);
            state.pending = None;
            self.write(&state)?;
        }
        Ok(Some(state))
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
            && state.pending.is_none()
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

pub(crate) struct Launch {
    store: Store,
    pending: Pending,
}

impl Launch {
    pub(crate) fn configure(&self, command: &mut Command) {
        command.env(SPAWN_MARKER, &self.pending.marker);
    }

    pub(crate) fn record(&self, child: &Child) -> Result<Tracked, String> {
        self.store.record(child, &self.pending)
    }

    // Only call before spawn, after spawn failed, or after the child was reaped.
    // Dropping Launch deliberately leaves the intent available for recovery.
    pub(crate) fn cancel(&self) -> Result<(), String> {
        let mut state = self.store.read()?.ok_or("grid launch state disappeared")?;
        if state.owner != Identity::capture(std::process::id())?
            || state.pending.as_ref().map(|pending| &pending.marker) != Some(&self.pending.marker)
        {
            return Err("grid launch intent changed before cancellation".to_owned());
        }
        state.pending = None;
        self.store.write(&state)
    }
}

impl Tracked {
    pub(crate) fn forget(&self) -> Result<(), String> {
        self.store.forget(&self.identity)
    }
}

pub(crate) fn claim() -> Result<Run, String> {
    store().claim()
}
pub(crate) fn prepare_spawn(binary: &Path, args: &[&str], cwd: &Path) -> Result<Launch, String> {
    store().prepare(binary, args, cwd)
}
pub(crate) fn survivor() -> Result<Option<u32>, String> {
    store().survivor()
}
pub(crate) fn down() -> Result<(), String> {
    store().down()
}

fn scan_pending(pending: &Pending) -> Result<Vec<Identity>, String> {
    scan_pending_bounded(pending, MAX_PROC_ENTRIES, MAX_SCAN_BYTES)
}

fn scan_pending_bounded(
    pending: &Pending,
    max_entries: usize,
    max_bytes: usize,
) -> Result<Vec<Identity>, String> {
    use std::os::unix::fs::MetadataExt;

    let uid = fs::metadata("/proc/self")
        .map_err(|_| "could not identify controller uid during grid recovery")?
        .uid();
    let entries = fs::read_dir("/proc")
        .map_err(|_| "could not scan processes for the pending grid launch")?;
    let marker = format!("{SPAWN_MARKER}={}", pending.marker);
    let mut bytes_left = max_bytes;
    let mut found = Vec::new();
    let scan_deadline = Instant::now() + Duration::from_secs(5);
    for (index, entry) in entries.enumerate() {
        if index >= max_entries || Instant::now() >= scan_deadline {
            return Err(
                "grid recovery process scan exceeded its entry or time limit; state preserved"
                    .to_owned(),
            );
        }
        let entry = entry.map_err(|_| "could not enumerate processes for grid recovery")?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|text| text.parse::<u32>().ok())
        else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(
                    "could not inspect a process during grid recovery; state preserved".to_owned(),
                );
            }
        };
        let start = crate::live::process_start_time(pid);
        if metadata.uid() != uid || start.is_none_or(|start| start < pending.after) {
            continue;
        }
        let file =
            match fs::File::open(entry.path().join("environ")) {
                Ok(file) => file,
                Err(_) if crate::live::process_start_time(pid).is_none() => continue,
                Err(_) => return Err(
                    "could not inspect a process environment during grid recovery; state preserved"
                        .to_owned(),
                ),
            };
        // Never log, persist or include environment contents in errors.
        let environment = limited(file, MAX_STATE.min(bytes_left))?;
        bytes_left = bytes_left
            .checked_sub(environment.len())
            .ok_or("grid recovery environment scan exceeded its byte limit; state preserved")?;
        if !environment
            .split(|byte| *byte == 0)
            .any(|entry| entry == marker.as_bytes())
        {
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match Identity::capture(pid) {
                Ok(identity) if pending.matches(&identity) => {
                    found.push(identity);
                    break;
                }
                _ if crate::live::process_start_time(pid).is_none() => break,
                _ if Instant::now() >= deadline => return Err("a pending grid launch marker has a mismatched process identity; state preserved".to_owned()),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        if found.len() > 1 {
            return Err(
                "multiple processes match one pending grid launch; state preserved".to_owned(),
            );
        }
    }
    Ok(found)
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
    for (signal, wait) in [
        ("-TERM", Duration::from_secs(30)),
        ("-KILL", Duration::from_secs(5)),
    ] {
        if !identity.live() {
            return Ok(());
        }
        if process_group(identity.pid) != Some(identity.pid) {
            return Err(format!(
                "grid process {} is no longer its group leader",
                identity.pid
            ));
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
            Self(Self::command().spawn().unwrap())
        }

        fn command() -> Command {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "grid_state::tests::waiting_child", "--ignored"])
                .current_dir(crate::repo_root())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            command
        }

        fn pending(store: &Store) -> (Launch, Self) {
            let launch = store
                .prepare(
                    &std::env::current_exe().unwrap(),
                    &["--exact", "grid_state::tests::waiting_child", "--ignored"],
                    &crate::repo_root(),
                )
                .unwrap();
            let mut command = Self::command();
            launch.configure(&mut command);
            let child = Self(command.spawn().unwrap());
            (launch, child)
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
        let (launch, mut child) = TestChild::pending(&store);
        launch.record(&child.0).unwrap();
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
            let (launch, mut child) = TestChild::pending(&store);
            launch.record(&child.0).unwrap();
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
    fn interrupted_spawn_before_record_is_recovered_without_touching_identical_unmarked_process() {
        let (_directory, store) = TestDir::store();
        let run = store.claim().unwrap();
        let (_launch, mut marked) = TestChild::pending(&store);
        let mut unrelated = TestChild::spawn();
        let mut state = store.read().unwrap().unwrap();
        assert!(state.processes.is_empty());
        assert!(state.pending.is_some());
        state.owner.start += 1;
        store.write(&state).unwrap();
        drop(run);
        assert_eq!(store.survivor().unwrap(), Some(marked.0.id()));
        assert!(store.read().unwrap().unwrap().pending.is_none());
        assert!(store.claim().is_err());
        store.down().unwrap();
        marked.0.wait().unwrap();
        assert!(unrelated.0.try_wait().unwrap().is_none());
        assert!(!store.path.exists());
    }

    #[test]
    fn pending_identity_mismatch_preserves_state_and_does_not_signal() {
        let (_directory, store) = TestDir::store();
        let run = store.claim().unwrap();
        let (_launch, mut child) = TestChild::pending(&store);
        let mut state = store.read().unwrap().unwrap();
        state.pending.as_mut().unwrap().args ^= 1;
        state.owner.start += 1;
        store.write(&state).unwrap();
        drop(run);
        assert!(store.down().unwrap_err().contains("mismatched"));
        assert!(store.read().unwrap().unwrap().pending.is_some());
        assert!(child.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn pending_scan_limits_fail_closed_and_unspawned_intent_can_be_cleared() {
        let (_directory, store) = TestDir::store();
        let run = store.claim().unwrap();
        let launch = store
            .prepare(&std::env::current_exe().unwrap(), &[], &crate::repo_root())
            .unwrap();
        assert!(scan_pending_bounded(&launch.pending, 0, MAX_SCAN_BYTES).is_err());
        assert!(scan_pending_bounded(&launch.pending, MAX_PROC_ENTRIES, 0).is_err());
        assert!(store.read().unwrap().unwrap().pending.is_some());
        launch.cancel().unwrap();
        assert!(store.read().unwrap().unwrap().pending.is_none());
        drop(run);
        assert!(!store.path.exists());
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
