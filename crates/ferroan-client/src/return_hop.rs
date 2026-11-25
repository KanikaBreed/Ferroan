// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared state for a child that can become root again.
//!
//! The simulation service stores an `UpdateAgent` here. The parked circuit
//! waits for that update, then asks the region to mark the child root before
//! `AgentMovementComplete`. The callback URI is held only so the region can
//! release the source; nothing in this type logs it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

use crate::circuit::RootArrival;

/// How long `CompleteMovement` waits for the child update.
pub const UPDATE_WAIT: Duration = Duration::from_secs(10);
/// How long an `UpdateAgent` with `wait_for_root` waits to see a root.
pub const ROOT_WAIT: Duration = Duration::from_secs(25);

/// Position and heading the return arrival packet uses, the source region,
/// and the movement fields `CopyFrom` takes from the returning `UpdateAgent`.
/// The frame stores those on the presence when the child becomes root.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReturnPose {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub origin: Uuid,
    pub velocity: [f32; 3],
    pub far: f32,
    pub body_rotation: [f32; 4],
    pub head_rotation: [f32; 4],
    pub control_flags: u32,
    pub always_run: bool,
}

impl Default for ReturnPose {
    /// The arrival defaults, and `AgentData`'s field defaults for the rest.
    fn default() -> Self {
        Self {
            position: [128.0, 128.0, 30.0],
            look_at: [1.0, 0.0, 0.0],
            origin: Uuid::nil(),
            velocity: [0.0, 0.0, 0.0],
            far: 0.0,
            body_rotation: [0.0, 0.0, 0.0, 0.0],
            head_rotation: [0.0, 0.0, 0.0, 0.0],
            control_flags: 0,
            always_run: false,
        }
    }
}

/// One agent's child-return gate. Cloned into the circuit and the session.
#[derive(Clone)]
pub struct ReturnSlot {
    inner: Arc<ReturnInner>,
}

struct ReturnInner {
    child: AtomicBool,
    ready: AtomicBool,
    rooted: AtomicBool,
    closed: AtomicBool,
    pose: Mutex<ReturnPose>,
    callback: Mutex<Option<String>>,
    update_notify: Notify,
    root_notify: Notify,
    promote: Mutex<Option<mpsc::Sender<RootArrival>>>,
    promotions: Mutex<Option<mpsc::Receiver<RootArrival>>>,
}

impl std::fmt::Debug for ReturnSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The callback URI is a capability address and stays out of Debug.
        formatter
            .debug_struct("ReturnSlot")
            .field("child", &self.is_child())
            .finish_non_exhaustive()
    }
}

impl ReturnSlot {
    /// `child` is true for a destination that has not been root here yet.
    pub fn new(child: bool) -> Self {
        let (promote, promotions) = mpsc::channel(2);
        Self {
            inner: Arc::new(ReturnInner {
                child: AtomicBool::new(child),
                ready: AtomicBool::new(false),
                rooted: AtomicBool::new(false),
                closed: AtomicBool::new(false),
                pose: Mutex::new(ReturnPose::default()),
                callback: Mutex::new(None),
                update_notify: Notify::new(),
                root_notify: Notify::new(),
                promote: Mutex::new(Some(promote)),
                promotions: Mutex::new(Some(promotions)),
            }),
        }
    }

    pub fn is_child(&self) -> bool {
        self.inner.child.load(Ordering::Acquire) && !self.inner.closed.load(Ordering::Acquire)
    }

    /// The session was demoted, or a child create reused it. A stale update is dropped.
    pub fn arm_child(&self) {
        self.inner.ready.store(false, Ordering::Release);
        self.inner.rooted.store(false, Ordering::Release);
        self.inner.child.store(true, Ordering::Release);
        *self.pose_mut() = ReturnPose::default();
        *self.callback_mut() = None;
    }

    /// The hop failed and the avatar is root here again.
    pub fn disarm(&self) {
        self.inner.child.store(false, Ordering::Release);
        self.inner.ready.store(false, Ordering::Release);
    }

    /// The generation ended. Waiters stop instead of sitting out the budget.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.child.store(false, Ordering::Release);
        self.inner.update_notify.notify_waiters();
        self.inner.root_notify.notify_waiters();
    }

    /// `AgentData` passed the session check. A nil origin does not wake the circuit.
    pub fn store_update(&self, pose: ReturnPose, callback: Option<String>) -> bool {
        if pose.origin.is_nil() || !finite_vec(pose.position) || !finite_vec(pose.look_at) {
            return false;
        }
        *self.pose_mut() = pose;
        *self.callback_mut() = callback.filter(|value| !value.is_empty());
        self.inner.ready.store(true, Ordering::Release);
        self.inner.update_notify.notify_one();
        true
    }

    /// `AgentPosition` on a child. It does not make the movement wait ready.
    pub fn store_position(&self, position: [f32; 3]) -> bool {
        if !finite_vec(position) {
            return false;
        }
        self.pose_mut().position = position;
        true
    }

    pub fn position(&self) -> [f32; 3] {
        self.pose_mut().position
    }

    /// The stored update, or the defaults before one arrives.
    pub fn pose(&self) -> ReturnPose {
        *self.pose_mut()
    }

    /// Take the release URI once. The caller must not log it.
    pub fn take_callback(&self) -> Option<String> {
        self.callback_mut().take()
    }

    /// The region loop takes this once, before the circuit is served.
    pub fn take_promotions(&self) -> Option<mpsc::Receiver<RootArrival>> {
        self.inner
            .promotions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }

    pub async fn promote(&self, arrival: RootArrival) -> bool {
        let sender = self
            .inner
            .promote
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let Some(sender) = sender else {
            return false;
        };
        sender.send(arrival).await.is_ok()
    }

    /// The child is root. `UpdateAgent` is waiting on this.
    pub fn note_rooted(&self) {
        self.inner.child.store(false, Ordering::Release);
        self.inner.rooted.store(true, Ordering::Release);
        self.inner.root_notify.notify_one();
    }

    pub async fn wait_update(&self, budget: Duration) -> Option<ReturnPose> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let notified = self.inner.update_notify.notified();
            tokio::pin!(notified);
            if self.inner.closed.load(Ordering::Acquire) {
                return None;
            }
            if let Some(pose) = self.ready_pose() {
                return Some(pose);
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return self.ready_pose();
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep(left) => return self.ready_pose(),
            }
        }
    }

    pub async fn wait_root(&self, budget: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let notified = self.inner.root_notify.notified();
            tokio::pin!(notified);
            if self.inner.closed.load(Ordering::Acquire) {
                return false;
            }
            if self.is_rooted() {
                return true;
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return self.is_rooted();
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep(left) => return self.is_rooted(),
            }
        }
    }

    fn ready_pose(&self) -> Option<ReturnPose> {
        if !self.inner.ready.load(Ordering::Acquire) {
            return None;
        }
        let pose = *self.pose_mut();
        if pose.origin.is_nil() {
            None
        } else {
            Some(pose)
        }
    }

    fn is_rooted(&self) -> bool {
        self.inner.rooted.load(Ordering::Acquire)
            && !self.inner.child.load(Ordering::Acquire)
            && !self.inner.closed.load(Ordering::Acquire)
    }

    fn pose_mut(&self) -> std::sync::MutexGuard<'_, ReturnPose> {
        self.inner
            .pose
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn callback_mut(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.inner
            .callback
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn finite_vec(value: [f32; 3]) -> bool {
    value.iter().all(|number| number.is_finite())
}
