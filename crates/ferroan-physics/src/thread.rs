// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The physics thread and its request queue.
//!
//! Pin8 takes `_simLock` for the whole step and for every query
//! (`JoltPhysicsBackend.cs` 4583–4597). A query issued during the step waits
//! for it. Ferroan does not put Jolt calls on the frame thread ([D6]). The
//! frame posts one heartbeat and reads the pose only after that reply. A pose
//! sample while the step runs is refused and does not wait. Shutdown answers
//! every request already queued and does not join the thread.
//!
//! The depth is [`REQUEST_QUEUE`], the same 256 as `region.scene-inbound`.
//! A caller past that bound is refused and does not wait. Script rays, the
//! arrival ray, and body create and remove are requests on this queue. The
//! stock stepped frame that reads a box pose is P3.64. Avatars stay on the
//! stand-in until P3.76.

use std::cell::Cell;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

use ferroan_config::Config;
use ferroan_diag::{Kind, PHYSICS_REQUEST_QUEUE, refuse};

use crate::jolt_config::{self, JoltLoad};
use crate::native::{NativeError, PhysicsSystem};
use crate::substep::SubstepAccumulator;

/// Waiting requests on the physics thread. Same depth as `region.scene-inbound`.
pub const REQUEST_QUEUE: usize = 256;

std::thread_local! {
    static OWNER: Cell<bool> = const { Cell::new(false) };
}

/// The calling thread is the one started by [`Physics::start`].
pub(crate) fn on_physics_thread() -> bool {
    OWNER.with(|flag| flag.get())
}

fn enter_owner() {
    OWNER.with(|flag| flag.set(true));
}

/// The physics thread did not start.
#[derive(Debug)]
pub struct SpawnError;

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("physics thread did not start")
    }
}

impl Error for SpawnError {}

/// The caller was not given a physics answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// [`REQUEST_QUEUE`] requests were already waiting. The caller did not wait.
    Full,
    /// A pose sample arrived while a heartbeat step was running.
    MidStep,
    /// The thread is gone. The caller did not stay blocked.
    Stopped,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "physics request queue is full",
            Self::MidStep => "physics pose was sampled during a step",
            Self::Stopped => "physics thread has stopped",
        })
    }
}

impl Error for Refusal {}

/// One heartbeat the physics thread has finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// Physics steps this heartbeat. From [`SubstepAccumulator`] when the
    /// effective rate is above zero, otherwise one step, as `Simulate` does
    /// when `_substeps` is null (`JoltScene.cs` 2795–2833).
    pub count: i32,
    /// Steps taken since the thread started.
    pub total_steps: i64,
    /// The rate [`JoltLoad::effective_step_rate`] stored at start.
    pub effective_rate: f32,
    /// The heartbeat passed to [`jolt_config::load`]. Not `[Startup] FrameTime`.
    pub heartbeat_seconds: f32,
}

/// The pose after the latest heartbeat reply. Empty of bodies until P3.64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pose {
    /// A heartbeat has been answered.
    pub ready: bool,
    /// [`Step::count`] of that heartbeat. Zero when none has finished.
    pub last_count: i32,
    /// [`Step::total_steps`] after that heartbeat.
    pub total_steps: i64,
}

/// A ray answered on the physics thread.
///
/// Pin8 `RayCast` returns no hit when `_system` is null
/// (`JoltPhysicsBackend.cs` 4604–4605). This thread has no system until a later
/// row creates bodies, so both rays miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayReply {
    pub hit: bool,
}

/// A body create or remove answered on the physics thread.
///
/// No backend exists on this row, so nothing is created or removed. P3.5 owns
/// the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyReply {
    pub changed: bool,
}

enum Job {
    Heartbeat {
        frame_seconds: f64,
        hold: Option<Hold>,
        reply: Sender<Result<Step, Refusal>>,
    },
    Sample {
        reply: Sender<Result<Pose, Refusal>>,
    },
    ScriptRay {
        reply: Sender<Result<RayReply, Refusal>>,
    },
    ArrivalRay {
        reply: Sender<Result<RayReply, Refusal>>,
    },
    CreateBody {
        reply: Sender<Result<BodyReply, Refusal>>,
    },
    RemoveBody {
        reply: Sender<Result<BodyReply, Refusal>>,
    },
    Native {
        reply: Sender<Result<(), NativeError>>,
    },
}

/// Test gate. The step signals `entered`, then waits until `release` or until
/// shutdown has sent `saw_stop` and received `proceed`.
struct Hold {
    entered: Sender<()>,
    release: Receiver<()>,
    saw_stop: Sender<()>,
    proceed: Receiver<()>,
}

#[cfg(test)]
struct Gates {
    entered: Receiver<()>,
    release: Sender<()>,
    saw_stop: Receiver<()>,
    proceed: Sender<()>,
}

#[cfg(test)]
fn hold_pair() -> (Hold, Gates) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (saw_tx, saw_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    (
        Hold {
            entered: entered_tx,
            release: release_rx,
            saw_stop: saw_tx,
            proceed: proceed_rx,
        },
        Gates {
            entered: entered_rx,
            release: release_tx,
            saw_stop: saw_rx,
            proceed: proceed_tx,
        },
    )
}

#[derive(Clone)]
struct Share {
    tx: SyncSender<Job>,
    stepping: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    heartbeat_seconds: f32,
    effective_rate: f32,
}

struct Engine {
    accumulator: Option<SubstepAccumulator>,
    total_steps: i64,
    last_count: i32,
    ready: bool,
    effective_rate: f32,
    heartbeat_seconds: f32,
    stepping: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
}

/// One region's physics thread.
///
/// Drop sets shutdown and detaches the thread. It does not join.
pub struct Physics {
    share: Share,
    thread: Option<JoinHandle<()>>,
}

impl Physics {
    /// Starts the thread for the frame. The ini is `[Startup] physics = Jolt`
    /// and the heartbeat is the frame's own period, not a `FrameTime` key.
    pub fn for_frame(heartbeat_seconds: f32) -> Result<Self, SpawnError> {
        let config = Config::parse("[Startup]\nphysics = Jolt\n").map_err(|_| SpawnError)?;
        Self::start(&config, heartbeat_seconds)
    }

    /// Starts the thread. `heartbeat_seconds` is passed to [`jolt_config::load`]
    /// and is not read from `[Startup] FrameTime`.
    pub fn start(config: &Config, heartbeat_seconds: f32) -> Result<Self, SpawnError> {
        let loaded = jolt_config::load(config, heartbeat_seconds);
        let (tx, rx) = mpsc::sync_channel(REQUEST_QUEUE);
        let stepping = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let share = Share {
            tx,
            stepping: Arc::clone(&stepping),
            stopping: Arc::clone(&stopping),
            heartbeat_seconds: loaded.heartbeat_seconds,
            effective_rate: loaded.effective_step_rate,
        };
        let engine = Engine::from_load(&loaded, stepping, stopping);
        let thread = std::thread::Builder::new()
            .name("ferroan-physics".to_owned())
            .spawn(move || run(engine, rx))
            .map_err(|_| SpawnError)?;
        Ok(Self {
            share,
            thread: Some(thread),
        })
    }

    /// The heartbeat passed to [`jolt_config::load`].
    pub fn heartbeat_seconds(&self) -> f32 {
        self.share.heartbeat_seconds
    }

    /// [`JoltLoad::effective_step_rate`] for that heartbeat.
    pub fn effective_rate(&self) -> f32 {
        self.share.effective_rate
    }

    /// What the frame does on one tick: post the heartbeat, wait for that
    /// reply, then read the pose. The sample is not sent first.
    pub fn frame_tick(&self, frame_seconds: f64) -> Result<Pose, Refusal> {
        let _step = self.heartbeat(frame_seconds)?;
        self.sample()
    }

    /// Post one heartbeat and wait for its step count.
    pub fn heartbeat(&self, frame_seconds: f64) -> Result<Step, Refusal> {
        self.share.heartbeat(frame_seconds, None)
    }

    /// Read the pose. A sample during a step returns [`Refusal::MidStep`]
    /// without waiting and without taking a queue slot.
    pub fn sample(&self) -> Result<Pose, Refusal> {
        self.share.sample()
    }

    /// A script ray. Misses until a later row has a system.
    pub fn script_ray(&self) -> Result<RayReply, Refusal> {
        self.share.ray(true)
    }

    /// The arrival ray. Misses until a later row has a system.
    pub fn arrival_ray(&self) -> Result<RayReply, Refusal> {
        self.share.ray(false)
    }

    /// Body create. Nothing is created on this row.
    pub fn create_body(&self) -> Result<BodyReply, Refusal> {
        self.share.body(true)
    }

    /// Body remove. Nothing is removed on this row.
    pub fn remove_body(&self) -> Result<BodyReply, Refusal> {
        self.share.body(false)
    }

    /// Create one [`PhysicsSystem`] on this thread and drop it there.
    pub fn create_and_drop_system(&self) -> Result<(), NativeError> {
        self.share.create_and_drop_system()
    }
}

impl Drop for Physics {
    fn drop(&mut self) {
        self.share.stopping.store(true, Ordering::Release);
        // Detach. Callers wait on a reply channel, not on this join.
        drop(self.thread.take());
    }
}

impl Engine {
    fn from_load(loaded: &JoltLoad, stepping: Arc<AtomicBool>, stopping: Arc<AtomicBool>) -> Self {
        Self {
            accumulator: SubstepAccumulator::new(loaded.effective_step_rate).ok(),
            total_steps: 0,
            last_count: 0,
            ready: false,
            effective_rate: loaded.effective_step_rate,
            heartbeat_seconds: loaded.heartbeat_seconds,
            stepping,
            stopping,
        }
    }

    fn pose(&self) -> Pose {
        Pose {
            ready: self.ready,
            last_count: self.last_count,
            total_steps: self.total_steps,
        }
    }

    fn take_steps(&mut self, frame_seconds: f64) -> Step {
        let count = if let Some(accumulator) = self.accumulator.as_mut() {
            accumulator.advance(frame_seconds)
        } else {
            // `_substeps` is null: one backend step for the heartbeat.
            1
        };
        self.total_steps += i64::from(count);
        self.last_count = count;
        self.ready = true;
        Step {
            count,
            total_steps: self.total_steps,
            effective_rate: self.effective_rate,
            heartbeat_seconds: self.heartbeat_seconds,
        }
    }
}

impl Share {
    fn heartbeat(&self, frame_seconds: f64, hold: Option<Hold>) -> Result<Step, Refusal> {
        self.call(|reply| Job::Heartbeat {
            frame_seconds,
            hold,
            reply,
        })
    }

    fn sample(&self) -> Result<Pose, Refusal> {
        if self.stepping.load(Ordering::Acquire) {
            return Err(Refusal::MidStep);
        }
        self.call(|reply| Job::Sample { reply })
    }

    fn ray(&self, script: bool) -> Result<RayReply, Refusal> {
        if script {
            self.call(|reply| Job::ScriptRay { reply })
        } else {
            self.call(|reply| Job::ArrivalRay { reply })
        }
    }

    fn body(&self, create: bool) -> Result<BodyReply, Refusal> {
        if create {
            self.call(|reply| Job::CreateBody { reply })
        } else {
            self.call(|reply| Job::RemoveBody { reply })
        }
    }

    fn create_and_drop_system(&self) -> Result<(), NativeError> {
        let (tx, rx) = mpsc::channel();
        if self.try_post(Job::Native { reply: tx }).is_err() {
            return Err(NativeError::OffThread);
        }
        rx.recv().unwrap_or(Err(NativeError::OffThread))
    }

    #[cfg(test)]
    fn enqueue_ray(&self) -> Result<Receiver<Result<RayReply, Refusal>>, Refusal> {
        let (tx, rx) = mpsc::channel();
        self.try_post(Job::ScriptRay { reply: tx })?;
        Ok(rx)
    }

    fn call<T>(
        &self,
        job_for: impl FnOnce(Sender<Result<T, Refusal>>) -> Job,
    ) -> Result<T, Refusal> {
        let (tx, rx) = mpsc::channel();
        self.try_post(job_for(tx))?;
        match rx.recv() {
            Ok(result) => result,
            Err(_) => Err(Refusal::Stopped),
        }
    }

    fn try_post(&self, job: Job) -> Result<(), Refusal> {
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                refuse(
                    PHYSICS_REQUEST_QUEUE,
                    Kind::Hard,
                    REQUEST_QUEUE as u64,
                    REQUEST_QUEUE as u64 + 1,
                    "physics",
                );
                Err(Refusal::Full)
            }
            Err(TrySendError::Disconnected(_)) => Err(Refusal::Stopped),
        }
    }
}

fn run(mut engine: Engine, rx: Receiver<Job>) {
    enter_owner();
    loop {
        if engine.stopping.load(Ordering::Acquire) {
            while let Ok(job) = rx.try_recv() {
                dispatch(&mut engine, job);
            }
            break;
        }
        match rx.recv() {
            Ok(job) => dispatch(&mut engine, job),
            Err(_) => break,
        }
    }
}

fn dispatch(engine: &mut Engine, job: Job) {
    match job {
        Job::Heartbeat {
            frame_seconds,
            hold,
            reply,
        } => {
            engine.stepping.store(true, Ordering::Release);
            if let Some(hold) = hold {
                pause(&hold, &engine.stopping);
            }
            let step = engine.take_steps(frame_seconds);
            engine.stepping.store(false, Ordering::Release);
            let _ = reply.send(Ok(step));
        }
        Job::Sample { reply } => {
            let answer = if engine.stepping.load(Ordering::Acquire) {
                Err(Refusal::MidStep)
            } else {
                Ok(engine.pose())
            };
            let _ = reply.send(answer);
        }
        Job::ScriptRay { reply } | Job::ArrivalRay { reply } => {
            debug_assert!(on_physics_thread());
            let _ = reply.send(Ok(RayReply { hit: false }));
        }
        Job::CreateBody { reply } | Job::RemoveBody { reply } => {
            debug_assert!(on_physics_thread());
            let _ = reply.send(Ok(BodyReply { changed: false }));
        }
        Job::Native { reply } => {
            let result = PhysicsSystem::new().map(drop);
            let _ = reply.send(result);
        }
    }
}

fn pause(hold: &Hold, stopping: &AtomicBool) {
    let _ = hold.entered.send(());
    loop {
        if hold.release.try_recv().is_ok() {
            return;
        }
        if stopping.load(Ordering::Acquire) {
            let _ = hold.saw_stop.send(());
            let _ = hold.proceed.recv();
            return;
        }
        std::thread::park_timeout(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;
    use std::thread;
    use std::time::Instant;

    fn fixture() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jolt/substeps.json");
        let text = std::fs::read_to_string(&path).expect("substeps fixture");
        serde_json::from_str(&text).expect("substeps json")
    }

    fn hex_u64(text: &str) -> u64 {
        u64::from_str_radix(text, 16).expect("hex")
    }

    fn hex_u32(text: &str) -> u32 {
        u32::from_str_radix(text, 16).expect("hex")
    }

    #[test]
    fn the_frame_tick_reads_the_pose_after_the_heartbeat() {
        let doc = fixture();
        let sequence = doc.get("sequence").expect("sequence");
        let heartbeat = f64::from_bits(hex_u64(
            sequence
                .get("heartbeat_f64_bits")
                .and_then(|value| value.as_str())
                .expect("bits"),
        ));
        let rate = f32::from_bits(hex_u32(
            sequence
                .get("rate_hz_bits")
                .and_then(|value| value.as_str())
                .expect("rate"),
        ));
        assert_eq!(rate.to_bits(), 0x4234_0000);
        assert_eq!(0.0909f32.to_bits(), 0x3dba_29c7);
        assert_eq!(heartbeat.to_bits(), f64::from(0.0909f32).to_bits());
        let physics = Physics::for_frame(0.0909).expect("thread");
        assert_eq!(physics.heartbeat_seconds().to_bits(), 0.0909f32.to_bits());
        assert_eq!(physics.effective_rate().to_bits(), rate.to_bits());
        let before = physics.sample().expect("sample before");
        assert!(!before.ready);
        let counts = sequence
            .get("counts")
            .and_then(|value| value.as_array())
            .expect("counts");
        assert_eq!(counts.len(), 1000);
        let fours = counts
            .iter()
            .filter(|count| count.as_i64() == Some(4))
            .count();
        let fives = counts
            .iter()
            .filter(|count| count.as_i64() == Some(5))
            .count();
        assert_eq!(fours, 910);
        assert_eq!(fives, 90);
        let mut total = 0i64;
        for count in counts {
            let expected = i32::try_from(count.as_i64().expect("count")).expect("i32");
            let pose = physics.frame_tick(heartbeat).expect("tick");
            total += i64::from(expected);
            assert!(pose.ready);
            assert_eq!(pose.last_count, expected);
            assert_eq!(pose.total_steps, total);
        }
        assert_eq!(total, 4090);
    }

    #[test]
    fn the_reader_uses_the_heartbeat_it_was_given() {
        let text = "[Startup]\nphysics = Jolt\nFrameTime = 0.0909\n[Jolt]\nPhysicsStepRate = 45\n";
        let config = Config::parse(text).expect("ini");
        let slow = Physics::start(&config, 0.01).expect("slow");
        assert_eq!(slow.heartbeat_seconds().to_bits(), 0.01f32.to_bits());
        assert_eq!(slow.effective_rate(), 0.0);
        let step = slow.heartbeat(f64::from(0.01f32)).expect("one step");
        assert_eq!(step.count, 1);
        assert_eq!(step.heartbeat_seconds.to_bits(), 0.01f32.to_bits());
        let region = Physics::start(&config, 0.0909).expect("region");
        assert_eq!(region.effective_rate().to_bits(), 45.0f32.to_bits());
        let step = region.heartbeat(f64::from(0.0909f32)).expect("four");
        assert_eq!(step.count, 4);
    }

    #[test]
    fn a_sample_during_the_step_is_refused_without_waiting() {
        let physics = Physics::for_frame(0.0909).expect("thread");
        let share = physics.share.clone();
        let (hold, gates) = hold_pair();
        let worker = thread::spawn(move || share.heartbeat(f64::from(0.0909f32), Some(hold)));
        gates
            .entered
            .recv_timeout(Duration::from_secs(2))
            .expect("entered");
        let started = Instant::now();
        let sample = physics.sample();
        let elapsed = started.elapsed();
        assert_eq!(sample, Err(Refusal::MidStep));
        assert!(elapsed < Duration::from_millis(50), "{elapsed:?}");
        gates.release.send(()).expect("release");
        let step = worker.join().expect("worker").expect("step");
        assert_eq!(step.count, 4);
        let pose = physics.sample().expect("after");
        assert!(pose.ready);
        assert_eq!(pose.last_count, 4);
    }

    #[test]
    fn the_257th_caller_is_refused_and_does_not_wait() {
        let physics = Physics::for_frame(0.0909).expect("thread");
        let share = physics.share.clone();
        let (hold, gates) = hold_pair();
        let worker = thread::spawn(move || share.heartbeat(f64::from(0.0909f32), Some(hold)));
        gates
            .entered
            .recv_timeout(Duration::from_secs(2))
            .expect("entered");
        let mut waiting = Vec::with_capacity(REQUEST_QUEUE);
        for _ in 0..REQUEST_QUEUE {
            waiting.push(physics.share.enqueue_ray().expect("queued"));
        }
        let started = Instant::now();
        let extra = physics.share.enqueue_ray();
        let elapsed = started.elapsed();
        assert_eq!(extra.err(), Some(Refusal::Full));
        assert!(elapsed < Duration::from_millis(50), "{elapsed:?}");
        assert!(ferroan_diag::metrics().contains("physics.request-queue"));
        gates.release.send(()).expect("release");
        worker.join().expect("worker").expect("step");
        for rx in waiting {
            let reply = rx.recv_timeout(Duration::from_secs(2)).expect("ray");
            assert_eq!(reply, Ok(RayReply { hit: false }));
        }
    }

    #[test]
    fn shutdown_answers_queued_callers_and_does_not_join() {
        let physics = Physics::for_frame(0.0909).expect("thread");
        let share = physics.share.clone();
        let (hold, gates) = hold_pair();
        let worker = thread::spawn(move || share.heartbeat(f64::from(0.0909f32), Some(hold)));
        gates
            .entered
            .recv_timeout(Duration::from_secs(2))
            .expect("entered");
        let mut waiting = Vec::new();
        for _ in 0..4 {
            waiting.push(physics.share.enqueue_ray().expect("queued"));
        }
        let late = physics.share.clone();
        drop(physics);
        gates
            .saw_stop
            .recv_timeout(Duration::from_secs(2))
            .expect("saw stop");
        assert!(matches!(waiting[0].try_recv(), Err(TryRecvError::Empty)));
        gates.proceed.send(()).expect("proceed");
        let step = worker.join().expect("worker").expect("step");
        assert_eq!(step.count, 4);
        for rx in waiting {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(2)).expect("queued"),
                Ok(RayReply { hit: false })
            );
        }
        // A caller that arrives once the thread has left must be refused
        // without waiting. A post that races the receiver's drop is answered
        // or disconnected, and the next post is the refusal.
        let started = Instant::now();
        let mut answer = late.enqueue_ray();
        while let Ok(rx) = answer {
            let _ = rx.recv_timeout(Duration::from_secs(2));
            answer = late.enqueue_ray();
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "caller still waiting"
            );
        }
        let refused_at = Instant::now();
        assert_eq!(answer.err(), Some(Refusal::Stopped));
        assert!(
            refused_at.saturating_duration_since(started) < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn rays_and_body_requests_are_answered_on_the_thread() {
        let physics = Physics::for_frame(0.0909).expect("thread");
        assert_eq!(physics.script_ray(), Ok(RayReply { hit: false }));
        assert_eq!(physics.arrival_ray(), Ok(RayReply { hit: false }));
        assert_eq!(physics.create_body(), Ok(BodyReply { changed: false }));
        assert_eq!(physics.remove_body(), Ok(BodyReply { changed: false }));
    }
}
