// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Movement intent from one viewer's `AgentUpdate` and `SetAlwaysRun`.
//!
//! The packet path drops a child, an in-transit avatar, and a sequence that
//! is not newer (`LLClientView.cs` `HandleAgentUpdate` at lines 8822–8846,
//! pin `c401bceecd`). A kept packet is significant for movement when controls
//! other than finish-anim are held and 20 ms have passed, when controls,
//! flags, state, or the far plane (32 m) change, or when the body quaternion
//! dot falls below `1 - 1e-5` (lines 8783–8798). The camera is significant
//! when center X/Y/Z, at-axis X/Y, or up-axis X/Y moves by more than 0.01
//! (lines 8808–8819). At-axis Z, the left axis, and up-axis Z are not part
//! of that test. Either way, both are forced when the last update that
//! changed both is more than 500 ms old (lines 8854–8857).
//!
//! The scene then stores the draw distance (clamped to 32..`MaxDrawDistance`,
//! the `DrawDistance` setter at `ScenePresence.cs` line 721), the title and mute flags, the
//! head, the control flags, and the fly, sit-on-ground, and stand flags
//! (`ScenePresence.HandleAgentUpdate`, lines 2747–3018). Direction bits,
//! including a nudge folded down by 19, become a body-space vector. Walk and
//! run speeds are 0.6 and 1.0 times 4.096; always-run or fast-at/fast-up
//! selects the run speed (`SetAlwaysRun` at `LLClientView.cs` line 9451 and
//! `ScenePresence.cs` lines 1141–1143 and 3953–3955). The vector is rotated
//! by the body, flattened to yaw when not flying, scaled, and multiplied by
//! four while flying. Flying into the ground (the last step was colliding and
//! the command's Z is negative) zeros the whole command instead; the animator
//! lands in a later row (`AddNewMovement`, lines 3987–4048). Releasing the keys
//! on the ground or in the air leaves that command for 250 ms. Stop replaces
//! it immediately, at 0.2 times 4.096.
//!
//! A camera-only update stores the far plane of the last movement-significant
//! update: `LLClientView` passes its stored arguments, and only the movement
//! branch copies `Far` into them (lines 8870–8894). Packet times are the
//! monotonic [`clock_ms`], as `Util.GetTimeStampMS` is a stopwatch.
//!
//! The frame tick integrates this command (`walk`). Terse updates, the camera
//! ray, scripted control grabs, the hide-title broadcast, and the animation
//! machine stay later rows. The animator starts on the surface, so a key
//! release takes the 250 ms path. The avatar counts as on the ground when it
//! is not flying. Move-to-target stays off: neither of these packets starts it.

use std::sync::OnceLock;
use std::time::Instant;

use crate::frame::Presence;

/// `Scene.m_maxDrawDistance`, the shipped default (`[Startup] MaxDrawDistance`
/// is commented out in `RegionServerDefaults.ini`).
pub const MAX_DRAW_DISTANCE: f32 = 512.0;
/// The `DrawDistance` setter's lower bound.
const MIN_DRAW_DISTANCE: f32 = 32.0;

/// Forward, from `AgentManager.ControlFlags`.
pub const CONTROL_AT_POS: u32 = 1 << 0;
/// Back.
pub const CONTROL_AT_NEG: u32 = 1 << 1;
/// Strafe left.
pub const CONTROL_LEFT_POS: u32 = 1 << 2;
/// Strafe right.
pub const CONTROL_LEFT_NEG: u32 = 1 << 3;
/// Up or jump.
pub const CONTROL_UP_POS: u32 = 1 << 4;
/// Down.
pub const CONTROL_UP_NEG: u32 = 1 << 5;
/// Fast forward. With fast-up, selects the run speed when always-run is off.
pub const CONTROL_FAST_AT: u32 = 1 << 10;
/// Fast up. With fast-at, selects the run speed when always-run is off.
pub const CONTROL_FAST_UP: u32 = 1 << 12;
/// Fly.
pub const CONTROL_FLY: u32 = 1 << 13;
/// Stop (space).
pub const CONTROL_STOP: u32 = 1 << 14;
/// Finish the current animation. Ignored by the 20 ms keep rule.
pub const CONTROL_FINISH_ANIM: u32 = 1 << 15;
/// Stand.
pub const CONTROL_STAND_UP: u32 = 1 << 16;
/// Sit on the ground.
pub const CONTROL_SIT_ON_GROUND: u32 = 1 << 17;
/// Mouselook.
pub const CONTROL_MOUSELOOK: u32 = 1 << 18;
/// A short forward tap.
pub const CONTROL_NUDGE_AT_POS: u32 = 1 << 19;
/// A short back tap.
pub const CONTROL_NUDGE_AT_NEG: u32 = 1 << 20;
/// A short left tap.
pub const CONTROL_NUDGE_LEFT_POS: u32 = 1 << 21;
/// A short right tap.
pub const CONTROL_NUDGE_LEFT_NEG: u32 = 1 << 22;
/// A short up tap.
pub const CONTROL_NUDGE_UP_POS: u32 = 1 << 23;
/// A short down tap.
pub const CONTROL_NUDGE_UP_NEG: u32 = 1 << 24;
/// Turn left. The fly roll needs this together with yaw-pos.
pub const CONTROL_TURN_LEFT: u32 = 1 << 25;
/// Turn right. The fly roll needs this together with yaw-neg.
pub const CONTROL_TURN_RIGHT: u32 = 1 << 26;
/// Yaw left, paired with turn-left for the fly roll.
pub const CONTROL_YAW_POS: u32 = 1 << 8;
/// Yaw right, paired with turn-right for the fly roll.
pub const CONTROL_YAW_NEG: u32 = 1 << 9;
/// Left button down.
pub const CONTROL_LBUTTON_DOWN: u32 = 1 << 28;

const NORM_MASK: u32 = CONTROL_AT_POS
    | CONTROL_AT_NEG
    | CONTROL_LEFT_POS
    | CONTROL_LEFT_NEG
    | CONTROL_UP_POS
    | CONTROL_UP_NEG;
const NUDGE_MASK: u32 = CONTROL_NUDGE_AT_POS
    | CONTROL_NUDGE_AT_NEG
    | CONTROL_NUDGE_LEFT_POS
    | CONTROL_NUDGE_LEFT_NEG
    | CONTROL_NUDGE_UP_POS
    | CONTROL_NUDGE_UP_NEG;
const DIR: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

const QDELTA_BODY: f32 = 1.0 - 0.00001;
const VDELTA: f32 = 0.01;
const MOVE_GAP_MS: f64 = 20.0;
const FORCE_GAP_MS: f64 = 500.0;
const STOP_HOLD_MS: i64 = 250;
/// `AgentControlNormalVel`.
const NORMAL_SPEED: f32 = 4.096;
/// `AgentControlMidVel`.
const MID_SPEED: f32 = 0.6 * 4.096;
/// `AgentControlStopSlowVel`.
const STOP_SPEED: f32 = 0.2 * 4.096;
const FLY_SPEED_SCALE: f32 = 4.0;
const FLY_ROLL_MAX: f32 = 1.1;
const FLY_ROLL_STEP: f32 = 0.06;
const FLY_ROLL_RESET: f32 = 0.02;
const LENGTH_EPSILON: f32 = 1.0e-7;
/// `AgentUpdateFlags.HideTitle`.
const HIDE_TITLE: u8 = 0x01;
/// `AgentUpdateFlags.MuteCollisions`.
const MUTE_COLLISIONS: u8 = 0x80;

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Milliseconds on the scene's monotonic clock. The circuit stamps each
/// `AgentUpdate` with its receive instant and the tick reads the same clock,
/// so the 20 ms, 500 ms and delayed-stop tests never see a wall-clock step.
/// An instant before the clock started reads 0.
pub fn clock_ms(at: Instant) -> u64 {
    let epoch = *EPOCH.get_or_init(Instant::now);
    u64::try_from(at.saturating_duration_since(epoch).as_millis()).unwrap_or(u64::MAX)
}

/// `ScenePresence.DrawDistance`'s setter: `Utils.Clamp(value, 32, MaxDrawDistance)`.
/// NaN passes through, as both comparisons are false.
pub(crate) fn clamp_draw_distance(far: f32) -> f32 {
    let far = if far > MAX_DRAW_DISTANCE {
        MAX_DRAW_DISTANCE
    } else {
        far
    };
    if far < MIN_DRAW_DISTANCE {
        MIN_DRAW_DISTANCE
    } else {
        far
    }
}

/// `IsUsableRotation`: finite and not too short to be a rotation.
pub(crate) fn usable_rotation(q: [f32; 4]) -> bool {
    finite4(q) && quat_dot(q, q) >= 1.0e-6
}

/// One decoded `AgentUpdate`. Times and the sequence come from the datagram.
#[derive(Clone, Copy, Debug)]
pub struct MotionUpdate {
    pub now_ms: u64,
    pub sequence: u32,
    pub child: bool,
    pub in_transit: bool,
    pub body_rotation: [f32; 4],
    pub head_rotation: [f32; 4],
    pub state: u8,
    pub camera_center: [f32; 3],
    pub camera_at: [f32; 3],
    pub camera_left: [f32; 3],
    pub camera_up: [f32; 3],
    pub far: f32,
    pub control_flags: u32,
    pub flags: u8,
}

/// What the frame stored for one avatar after the updates it kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub body_rotation: [f32; 4],
    pub head_rotation: [f32; 4],
    pub state: u8,
    pub flags: u8,
    pub control_flags: u32,
    pub draw_distance: f32,
    pub hide_title: bool,
    pub mute_collisions: bool,
    pub left_button: bool,
    pub mouse_look: bool,
    pub always_run: bool,
    pub flying: bool,
    pub sit_ground: bool,
    pub stop_active: bool,
    pub movement_flags: u32,
    pub target_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub delayed_stop_ms: i64,
    pub camera_center: [f32; 3],
    pub camera_at: [f32; 3],
    pub camera_left: [f32; 3],
    pub camera_up: [f32; 3],
    pub camera_rotation: [f32; 4],
    pub needs_camera_collision: bool,
}

/// The movement fields of a returning `UpdateAgent` (`AgentData.Unpack`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReturnData {
    pub velocity: [f32; 3],
    pub far: f32,
    pub body_rotation: [f32; 4],
    pub head_rotation: [f32; 4],
    pub control_flags: u32,
    pub always_run: bool,
}

/// Whether a packet was kept, and which half of it changed the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub dropped: bool,
    pub movement: bool,
    pub camera: bool,
}

#[derive(Clone, Copy, Debug)]
struct Filter {
    sequence: u32,
    last_update_ms: f64,
    last_move_ms: f64,
    body_rotation: [f32; 4],
    head_rotation: [f32; 4],
    state: u8,
    flags: u8,
    far: f32,
    control_flags: u32,
    camera_center: [f32; 3],
    camera_at: [f32; 3],
    camera_left: [f32; 3],
    camera_up: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Body {
    motion: Motion,
    filter: Filter,
    last_finite: [f32; 3],
    /// Velocity the tick integrates. A new actor starts at rest even when the
    /// presence velocity is not. A command copies the target here. The tick
    /// writes the result back and does not copy the target again.
    physics_velocity: [f32; 3],
    /// Ground contact from the last kinematic step. A fresh actor is not colliding.
    colliding: bool,
    /// No physics actor: sitting on the ground. Later updates skip the command.
    seated: bool,
    /// Away in another region. The record stays. The tick does not move it.
    child: bool,
    /// False until the arrival packet has sampled the pose. The tick does not
    /// move a seated record, so that packet and the GridUser report stay the
    /// position written for the login.
    actor: bool,
}

impl Body {
    pub(crate) fn new(pose: &Presence) -> Self {
        Self {
            motion: Motion {
                body_rotation: pose.rotation,
                head_rotation: [0.0, 0.0, 0.0, 1.0],
                state: 0,
                flags: 0,
                control_flags: pose.control_flags,
                draw_distance: pose.draw_distance,
                hide_title: false,
                mute_collisions: false,
                left_button: false,
                mouse_look: false,
                always_run: false,
                flying: false,
                sit_ground: false,
                stop_active: false,
                movement_flags: 0,
                target_velocity: [0.0, 0.0, 0.0],
                angular_velocity: [0.0, 0.0, 0.0],
                delayed_stop_ms: -1,
                camera_center: [0.0, 0.0, 0.0],
                camera_at: [0.0, 0.0, 0.0],
                camera_left: [0.0, 0.0, 0.0],
                camera_up: [0.0, 0.0, 0.0],
                camera_rotation: [0.0, 0.0, 0.0, 1.0],
                needs_camera_collision: false,
            },
            // A fresh client view has a zero quaternion, not the login heading.
            // Its timestamps are 0 on a stopwatch that has run since boot, so
            // the first update is always forced: minus infinity here.
            filter: Filter {
                sequence: 0,
                last_update_ms: f64::NEG_INFINITY,
                last_move_ms: f64::NEG_INFINITY,
                body_rotation: [0.0, 0.0, 0.0, 0.0],
                head_rotation: [0.0, 0.0, 0.0, 0.0],
                state: 0,
                flags: 0,
                far: 0.0,
                control_flags: 0,
                camera_center: [0.0, 0.0, 0.0],
                camera_at: [0.0, 0.0, 0.0],
                camera_left: [0.0, 0.0, 0.0],
                camera_up: [0.0, 0.0, 0.0],
            },
            last_finite: pose.position,
            physics_velocity: [0.0, 0.0, 0.0],
            colliding: false,
            seated: false,
            child: false,
            // Direct steps in tests have an actor. A login pose holds it.
            actor: true,
        }
    }

    pub(crate) fn set_child(&mut self, child: bool) {
        self.child = child;
    }

    /// Not root for `ScenePresence.Update`: away in another region, or not
    /// yet arrived (C# keeps a login or a return a child agent until
    /// `CompleteMovement` makes it root, which is when the arrival samples).
    pub(crate) fn is_child(&self) -> bool {
        self.child || !self.actor
    }

    pub(crate) fn set_actor(&mut self, actor: bool) {
        self.actor = actor;
    }

    pub(crate) fn view(&self) -> Motion {
        self.motion
    }

    pub(crate) fn set_always_run(&mut self, always_run: bool) {
        self.motion.always_run = always_run;
    }

    /// `sp.Velocity = Vector3.Zero` and a new physics actor at rest. The fly
    /// and sit flags, the angular velocity and a pending delayed stop stay:
    /// neither `TeleportAgentWithinRegion` nor `MakeRootAgent` clears them.
    pub(crate) fn halt_command(&mut self) {
        self.motion.target_velocity = [0.0, 0.0, 0.0];
        self.physics_velocity = [0.0, 0.0, 0.0];
    }

    /// `RotateToLookAt`: the body faces this way. The packet filter keeps its own copy.
    pub(crate) fn turn_to(&mut self, rotation: [f32; 4], pose: &mut Presence) {
        self.motion.body_rotation = rotation;
        pose.rotation = rotation;
    }

    /// `ScenePresence.Flying`: true only with a physics actor that flies. A
    /// ground sit, a child, and a pose the arrival has not sampled have no actor.
    pub(crate) fn flying(&self) -> bool {
        self.motion.flying && self.actor && !self.seated && !self.child
    }

    /// The returning `UpdateAgent`, as `CopyFrom` stores it, then the flying
    /// state `CompleteMovement` gives the new actor from the control flags.
    pub(crate) fn apply_return(&mut self, data: &ReturnData, pose: &mut Presence) {
        pose.velocity = data.velocity;
        let far = clamp_draw_distance(data.far);
        self.motion.draw_distance = far;
        pose.draw_distance = far;
        self.motion.head_rotation = data.head_rotation;
        if usable_rotation(data.body_rotation) {
            self.motion.body_rotation = data.body_rotation;
            pose.rotation = data.body_rotation;
        }
        self.motion.control_flags = data.control_flags;
        pose.control_flags = data.control_flags;
        self.motion.always_run = data.always_run;
        self.motion.flying = data.control_flags & CONTROL_FLY != 0;
    }

    pub(crate) fn apply(&mut self, update: &MotionUpdate, pose: &mut Presence) -> Applied {
        if update.child || update.in_transit || update.sequence <= self.filter.sequence {
            return Applied {
                dropped: true,
                movement: false,
                camera: false,
            };
        }
        self.filter.sequence = update.sequence;
        let now = update.now_ms as f64;
        let (movement, camera) = if now - self.filter.last_update_ms > FORCE_GAP_MS {
            (true, true)
        } else {
            (
                movement_significant(&self.filter, update, now),
                camera_significant(&self.filter, update),
            )
        };

        if movement {
            self.filter.last_move_ms = now;
            self.filter.body_rotation = update.body_rotation;
            self.filter.control_flags = update.control_flags;
            self.filter.far = update.far;
            self.filter.flags = update.flags;
            self.filter.head_rotation = update.head_rotation;
            self.filter.state = update.state;
            self.motion.needs_camera_collision = !camera;
            self.handle_movement(update, pose, now);
        }
        if camera {
            self.filter.camera_at = update.camera_at;
            self.filter.camera_center = update.camera_center;
            self.filter.camera_left = update.camera_left;
            self.filter.camera_up = update.camera_up;
            self.motion.needs_camera_collision = true;
            self.handle_camera(update, pose);
        }
        if movement && camera {
            self.filter.last_update_ms = now;
        }
        Applied {
            dropped: false,
            movement,
            camera,
        }
    }

    fn handle_movement(&mut self, update: &MotionUpdate, pose: &mut Presence, now: f64) {
        if !finite3(pose.position) {
            pose.position = if finite3(self.last_finite) {
                self.last_finite
            } else {
                [127.0, 127.0, 127.0]
            };
        } else {
            self.last_finite = pose.position;
        }

        let far = clamp_draw_distance(update.far);
        self.motion.draw_distance = far;
        pose.draw_distance = far;
        self.motion.flags = update.flags;
        self.motion.hide_title = update.flags & HIDE_TITLE != 0;
        self.motion.mute_collisions = update.flags & MUTE_COLLISIONS != 0;
        self.motion.left_button = update.control_flags & CONTROL_LBUTTON_DOWN != 0;
        self.motion.mouse_look = update.control_flags & CONTROL_MOUSELOOK != 0;
        self.motion.head_rotation = update.head_rotation;
        self.motion.state = update.state;

        // An experience-held seat would refuse the stand. This row has none.
        if update.control_flags & CONTROL_STAND_UP != 0 {
            self.motion.sit_ground = false;
            self.seated = false;
        }
        if update.control_flags & CONTROL_SIT_ON_GROUND != 0 {
            self.motion.sit_ground = true;
            self.seated = true;
            self.motion.angular_velocity = [0.0, 0.0, 0.0];
            self.motion.target_velocity = [0.0, 0.0, 0.0];
            self.physics_velocity = [0.0, 0.0, 0.0];
            pose.velocity = [0.0, 0.0, 0.0];
        }

        self.motion.control_flags = update.control_flags;
        pose.control_flags = update.control_flags;

        if self.seated {
            return;
        }

        // The `Rotation` setter keeps the previous value for one that is not usable.
        if usable_rotation(update.body_rotation) {
            self.motion.body_rotation = update.body_rotation;
            pose.rotation = update.body_rotation;
        }

        let mut movement_flags = update.control_flags & (NORM_MASK | NUDGE_MASK);
        let old_flags = self.motion.movement_flags & (NORM_MASK | NUDGE_MASK);
        let mut command = [0.0, 0.0, 0.0];
        let mut speed = NORMAL_SPEED;
        let mut changed = false;
        if movement_flags != 0 {
            changed = true;
            if !self.motion.always_run
                && update.control_flags & (CONTROL_FAST_AT | CONTROL_FAST_UP) == 0
            {
                speed = MID_SPEED;
            }
            if movement_flags & NUDGE_MASK != 0 {
                movement_flags |= movement_flags >> 19;
            }
            let mut bit = 1u32;
            for dir in DIR {
                if movement_flags & bit != 0 {
                    command = add3(command, dir);
                }
                bit <<= 1;
            }
        } else if old_flags != 0 {
            changed = true;
        }
        self.motion.movement_flags = movement_flags;

        let new_flying = update.control_flags & CONTROL_FLY != 0;
        if new_flying != self.motion.flying {
            self.motion.flying = new_flying;
            changed = true;
        }
        let stop = update.control_flags & CONTROL_STOP != 0;
        if stop != self.motion.stop_active {
            self.motion.stop_active = stop;
            changed = true;
        }

        if self.motion.flying {
            let left = CONTROL_TURN_LEFT | CONTROL_YAW_POS;
            let right = CONTROL_TURN_RIGHT | CONTROL_YAW_NEG;
            if update.control_flags & left == left {
                apply_flying_roll(
                    &mut self.motion.angular_velocity[2],
                    FLY_ROLL_STEP,
                    update.control_flags & CONTROL_UP_POS != 0,
                    update.control_flags & CONTROL_UP_NEG != 0,
                );
            } else if update.control_flags & right == right {
                apply_flying_roll(
                    &mut self.motion.angular_velocity[2],
                    -FLY_ROLL_STEP,
                    update.control_flags & CONTROL_UP_POS != 0,
                    update.control_flags & CONTROL_UP_NEG != 0,
                );
            } else if self.motion.angular_velocity[2] != 0.0 {
                let step = roll_reset(self.motion.angular_velocity[2], FLY_ROLL_RESET);
                self.motion.angular_velocity[2] += step;
            }
        } else if command[2] < 0.0 {
            // On the ground, a downward control does not enter the command.
            command[2] = 0.0;
        }

        if !changed {
            return;
        }
        if self.motion.stop_active {
            // Falling would pass the breaking flag. The animator still says
            // the avatar is on the surface, so the stop is the slow command.
            self.add_new_movement(command, STOP_SPEED, update.body_rotation);
            return;
        }
        if movement_flags != 0 {
            self.add_new_movement(command, speed, update.body_rotation);
        } else {
            self.motion.delayed_stop_ms = (now as i64).saturating_add(STOP_HOLD_MS);
        }
    }

    fn handle_camera(&mut self, update: &MotionUpdate, pose: &mut Presence) {
        self.motion.camera_center = update.camera_center;
        self.motion.camera_at = normalize3(update.camera_at);
        self.motion.camera_left = normalize3(update.camera_left);
        self.motion.camera_up = normalize3(update.camera_up);
        let rotation = axes_to_rotation(
            self.motion.camera_at,
            self.motion.camera_left,
            self.motion.camera_up,
        );
        if finite4(rotation) {
            self.motion.camera_rotation = rotation;
        }
        // The stored arguments: this packet's far only when the movement half kept it.
        let far = clamp_draw_distance(self.filter.far);
        self.motion.draw_distance = far;
        pose.draw_distance = far;
    }

    fn add_new_movement(&mut self, command: [f32; 3], speed: f32, body: [f32; 4]) {
        self.motion.delayed_stop_ms = -1;
        let mut rotation = body;
        if !self.motion.flying {
            rotation[0] = 0.0;
            rotation[1] = 0.0;
        }
        let mut direc = rotate_by_quat(command, rotation);
        normalize_in(&mut direc);
        if command[2] == 0.0 && !self.motion.flying {
            direc[2] = 0.0;
        }
        direc = scale3(direc, speed);
        // Flying into the ground zeros the whole command. The animator lands
        // in a later row. Otherwise flying is scaled by four. On the ground an
        // up or a downward result contributes no Z: the jump itself is the animator's.
        if self.motion.flying {
            if self.colliding && direc[2] < 0.0 {
                direc = [0.0, 0.0, 0.0];
            } else {
                direc = scale3(direc, FLY_SPEED_SCALE);
            }
        } else if self.motion.control_flags & CONTROL_UP_POS != 0 || direc[2] < 0.0 {
            direc[2] = 0.0;
        }
        self.motion.target_velocity = direc;
        self.physics_velocity = direc;
    }

    /// One tick. A seated avatar, a child, and a pose that the arrival packet
    /// has not sampled yet are skipped. The delayed stop is applied after the
    /// move, so the deadline's frame still travels and the next one rests.
    pub(crate) fn step_physics(
        &mut self,
        pose: &mut Presence,
        terrain: f32,
        extent_x: f32,
        extent_y: f32,
        dt: f32,
        now_ms: u64,
    ) {
        if self.seated || self.child || !self.actor {
            return;
        }
        let step = crate::walk::integrate(
            pose.position,
            self.physics_velocity,
            self.motion.flying,
            extent_x,
            extent_y,
            terrain,
            dt,
        );
        let (position, velocity) =
            crate::walk::fail_border(step.position, step.velocity, extent_x, extent_y, dt);
        if position != step.position || velocity != step.velocity {
            self.motion.target_velocity = [0.0, 0.0, 0.0];
            self.motion.angular_velocity = [0.0, 0.0, 0.0];
        }
        pose.position = position;
        pose.velocity = velocity;
        self.physics_velocity = velocity;
        self.colliding = step.colliding;
        self.last_finite = position;
        let deadline = self.motion.delayed_stop_ms;
        if deadline > 0 && now_ms as i64 > deadline {
            self.motion.delayed_stop_ms = -1;
            self.motion.target_velocity = [0.0, 0.0, 0.0];
            self.physics_velocity = [0.0, 0.0, 0.0];
            pose.velocity = [0.0, 0.0, 0.0];
        }
    }
}

fn movement_significant(filter: &Filter, update: &MotionUpdate, now: f64) -> bool {
    let held = update.control_flags & !CONTROL_FINISH_ANIM;
    if held != 0 && now > filter.last_move_ms + MOVE_GAP_MS {
        return true;
    }
    if update.control_flags != filter.control_flags
        || update.flags != filter.flags
        || update.state != filter.state
        || (update.far - filter.far).abs() >= 32.0
    {
        return true;
    }
    quat_dot(update.body_rotation, filter.body_rotation).abs() < QDELTA_BODY
}

fn camera_significant(filter: &Filter, update: &MotionUpdate) -> bool {
    axis_moved(update.camera_center, filter.camera_center)
        || (update.camera_at[0] - filter.camera_at[0]).abs() > VDELTA
        || (update.camera_at[1] - filter.camera_at[1]).abs() > VDELTA
        || (update.camera_up[0] - filter.camera_up[0]).abs() > VDELTA
        || (update.camera_up[1] - filter.camera_up[1]).abs() > VDELTA
}

fn axis_moved(next: [f32; 3], prev: [f32; 3]) -> bool {
    (next[0] - prev[0]).abs() > VDELTA
        || (next[1] - prev[1]).abs() > VDELTA
        || (next[2] - prev[2]).abs() > VDELTA
}

fn apply_flying_roll(z: &mut f32, amount: f32, up: bool, down: bool) {
    let mut roll = (*z + amount).clamp(-FLY_ROLL_MAX, FLY_ROLL_MAX);
    if amount > 0.0 {
        if up && roll >= FLY_ROLL_MAX - 0.04 {
            roll -= 0.9;
        }
        if down && (FLY_ROLL_MAX..FLY_ROLL_MAX + 0.6).contains(&roll) {
            roll += 0.6;
        }
    } else {
        if up && roll <= -FLY_ROLL_MAX {
            roll += 0.6;
        }
        if down && roll >= -FLY_ROLL_MAX - 0.6 {
            roll -= 0.6;
        }
    }
    *z = roll;
}

fn roll_reset(z: f32, amount: f32) -> f32 {
    if z > 0.0 {
        let left = z;
        if amount > left { -left } else { -amount }
    } else {
        let left = -z;
        if amount > left { left } else { amount }
    }
}

/// `Util.Axes2Rot` at `Util.cs` lines 337–388.
fn axes_to_rotation(fwd: [f32; 3], left: [f32; 3], up: [f32; 3]) -> [f32; 4] {
    let tr = fwd[0] + left[1] + up[2] + 1.0;
    if tr >= 1.0 {
        let s = 0.5 / tr.sqrt();
        return [
            (left[2] - up[1]) * s,
            (up[0] - fwd[2]) * s,
            (fwd[1] - left[0]) * s,
            0.25 / s,
        ];
    }
    let max = if left[1] > up[2] { left[1] } else { up[2] };
    if max < fwd[0] {
        let s = (fwd[0] - (left[1] + up[2]) + 1.0).sqrt();
        let x = s * 0.5;
        let s = 0.5 / s;
        [
            x,
            (fwd[1] + left[0]) * s,
            (up[0] + fwd[2]) * s,
            (left[2] - up[1]) * s,
        ]
    } else if max == left[1] {
        let s = (left[1] - (up[2] + fwd[0]) + 1.0).sqrt();
        let y = s * 0.5;
        let s = 0.5 / s;
        [
            (fwd[1] + left[0]) * s,
            y,
            (left[2] + up[1]) * s,
            (up[0] - fwd[2]) * s,
        ]
    } else {
        let s = (up[2] - (fwd[0] + left[1]) + 1.0).sqrt();
        let z = s * 0.5;
        let s = 0.5 / s;
        [
            (up[0] + fwd[2]) * s,
            (left[2] + up[1]) * s,
            z,
            (fwd[1] - left[0]) * s,
        ]
    }
}

fn rotate_by_quat(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    [
        w * w * v[0] + 2.0 * y * w * v[2] - 2.0 * z * w * v[1]
            + x * x * v[0]
            + 2.0 * y * x * v[1]
            + 2.0 * z * x * v[2]
            - z * z * v[0]
            - y * y * v[0],
        2.0 * x * y * v[0] + y * y * v[1] + 2.0 * z * y * v[2] + 2.0 * w * z * v[0] - z * z * v[1]
            + w * w * v[1]
            - 2.0 * x * w * v[2]
            - x * x * v[1],
        2.0 * x * z * v[0] + 2.0 * y * z * v[1] + z * z * v[2] - 2.0 * w * y * v[0] - y * y * v[2]
            + 2.0 * w * x * v[1]
            - x * x * v[2]
            + w * w * v[2],
    ]
}

fn quat_dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(v: [f32; 3], scale: f32) -> [f32; 3] {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

fn normalize3(mut v: [f32; 3]) -> [f32; 3] {
    normalize_in(&mut v);
    v
}

fn normalize_in(v: &mut [f32; 3]) {
    let mag = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if mag > LENGTH_EPSILON {
        let scale = 1.0 / mag;
        v[0] *= scale;
        v[1] *= scale;
        v[2] *= scale;
    } else {
        *v = [0.0, 0.0, 0.0];
    }
}

fn finite3(v: [f32; 3]) -> bool {
    v.iter().all(|n| n.is_finite())
}

fn finite4(v: [f32; 4]) -> bool {
    v.iter().all(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login_pose() -> Presence {
        Presence {
            position: [128.0, 128.0, 25.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            control_flags: 0,
            draw_distance: 255.0,
        }
    }

    fn update(now_ms: u64, sequence: u32, controls: u32) -> MotionUpdate {
        MotionUpdate {
            now_ms,
            sequence,
            child: false,
            in_transit: false,
            body_rotation: [0.0, 0.0, 0.0, 1.0],
            head_rotation: [0.0, 0.0, 0.0, 1.0],
            state: 0,
            camera_center: [128.0, 128.0, 26.0],
            camera_at: [1.0, 0.0, 0.0],
            camera_left: [0.0, 1.0, 0.0],
            camera_up: [0.0, 0.0, 1.0],
            far: 128.0,
            control_flags: controls,
            flags: 0,
        }
    }

    fn close(got: [f32; 3], expect: [f32; 3]) {
        for (a, b) in got.iter().zip(expect) {
            assert!((a - b).abs() < 1.0e-4, "{got:?} != {expect:?}");
        }
    }

    #[test]
    fn walk_run_fly_stop_and_a_key_release_set_the_command() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let first = body.apply(&update(1_000, 1, CONTROL_AT_POS), &mut pose);
        assert!(!first.dropped && first.movement && first.camera);
        close(body.view().target_velocity, [MID_SPEED, 0.0, 0.0]);
        assert_eq!(pose.control_flags, CONTROL_AT_POS);
        assert_eq!(pose.draw_distance, 128.0);
        assert_eq!(pose.rotation, [0.0, 0.0, 0.0, 1.0]);

        body.set_always_run(true);
        body.apply(&update(1_030, 2, CONTROL_AT_POS), &mut pose);
        close(body.view().target_velocity, [NORMAL_SPEED, 0.0, 0.0]);

        body.set_always_run(false);
        body.apply(
            &update(1_060, 3, CONTROL_AT_POS | CONTROL_FAST_AT),
            &mut pose,
        );
        close(body.view().target_velocity, [NORMAL_SPEED, 0.0, 0.0]);

        let yaw = std::f32::consts::FRAC_1_SQRT_2;
        let mut turned = update(1_090, 4, CONTROL_AT_POS);
        turned.body_rotation = [0.0, 0.0, yaw, yaw];
        body.apply(&turned, &mut pose);
        close(body.view().target_velocity, [0.0, MID_SPEED, 0.0]);

        body.apply(&update(1_120, 5, CONTROL_NUDGE_AT_POS), &mut pose);
        close(body.view().target_velocity, [MID_SPEED, 0.0, 0.0]);

        body.apply(&update(1_150, 6, CONTROL_FLY | CONTROL_AT_POS), &mut pose);
        assert!(body.view().flying);
        close(
            body.view().target_velocity,
            [MID_SPEED * FLY_SPEED_SCALE, 0.0, 0.0],
        );

        let before = body.view().target_velocity;
        body.apply(&update(1_180, 7, CONTROL_FLY), &mut pose);
        assert_eq!(body.view().target_velocity, before);
        assert_eq!(body.view().delayed_stop_ms, 1_180 + STOP_HOLD_MS);

        body.apply(&update(1_210, 8, CONTROL_AT_POS | CONTROL_STOP), &mut pose);
        assert!(body.view().stop_active);
        assert!(!body.view().flying);
        close(body.view().target_velocity, [STOP_SPEED, 0.0, 0.0]);
        assert_eq!(body.view().delayed_stop_ms, -1);
    }

    #[test]
    fn sit_stand_and_a_child_or_old_sequence_do_not_walk() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        body.apply(&update(1_000, 1, CONTROL_SIT_ON_GROUND), &mut pose);
        assert!(body.view().sit_ground);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
        body.apply(&update(1_040, 2, CONTROL_AT_POS), &mut pose);
        assert_eq!(body.view().target_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(pose.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(pose.control_flags, CONTROL_AT_POS);

        body.apply(
            &update(1_080, 3, CONTROL_STAND_UP | CONTROL_AT_POS),
            &mut pose,
        );
        assert!(!body.view().sit_ground);
        close(body.view().target_velocity, [MID_SPEED, 0.0, 0.0]);
        assert_eq!(pose.rotation, [0.0, 0.0, 0.0, 1.0]);

        let mut child = update(2_000, 4, CONTROL_AT_NEG);
        child.child = true;
        let dropped = body.apply(&child, &mut pose);
        assert!(dropped.dropped);
        assert_eq!(pose.control_flags, CONTROL_STAND_UP | CONTROL_AT_POS);

        let mut transit = update(2_001, 4, CONTROL_AT_NEG);
        transit.in_transit = true;
        assert!(body.apply(&transit, &mut pose).dropped);

        let replay = body.apply(&update(2_100, 3, CONTROL_AT_NEG), &mut pose);
        assert!(replay.dropped);
        close(body.view().target_velocity, [MID_SPEED, 0.0, 0.0]);
    }

    #[test]
    fn the_20_ms_hold_and_the_500_ms_force_match_the_filter() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        body.apply(&update(1_000, 1, CONTROL_AT_POS), &mut pose);
        let held = body.apply(&update(1_020, 2, CONTROL_AT_POS), &mut pose);
        assert!(!held.movement);
        let next = body.apply(&update(1_021, 3, CONTROL_AT_POS), &mut pose);
        assert!(next.movement);

        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let quiet = update(1_000, 1, 0);
        body.apply(&quiet, &mut pose);
        let mut again = quiet;
        again.now_ms = 1_200;
        again.sequence = 2;
        let idle = body.apply(&again, &mut pose);
        assert!(!idle.movement && !idle.camera);

        again.now_ms = 1_000 + 501;
        again.sequence = 3;
        let forced = body.apply(&again, &mut pose);
        assert!(forced.movement && forced.camera);

        let mut nudge_camera = again;
        nudge_camera.now_ms = 1_000 + 501 + 10;
        nudge_camera.sequence = 4;
        nudge_camera.camera_at = [1.0, 0.01, 0.0];
        let exact = body.apply(&nudge_camera, &mut pose);
        assert!(!exact.camera && !exact.movement);
        nudge_camera.sequence = 5;
        nudge_camera.camera_at = [1.0, 0.02, 0.0];
        let moved = body.apply(&nudge_camera, &mut pose);
        assert!(moved.camera && !moved.movement);
        let stored = normalize3(nudge_camera.camera_at);
        close(body.view().camera_at, stored);
    }

    #[test]
    fn a_non_finite_position_returns_to_the_last_finite_one() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        pose.position = [f32::NAN, 1.0, 2.0];
        body.apply(&update(1_000, 1, 0), &mut pose);
        assert_eq!(pose.position, [128.0, 128.0, 25.0]);
    }

    /// One circuit of the capture: a fresh client view and presence per
    /// `UseCircuitCode`, the sequences that circuit has seen, and its burst.
    struct Replay {
        body: Body,
        pose: Presence,
        seen: std::collections::HashSet<u32>,
        burst: usize,
        updates: u32,
    }

    /// `UseCircuitCode`s this close together are one login burst: the build
    /// scenario logs the owner and the chat account in together, and the
    /// redaction gives both the same ids.
    const BURST_MS: u64 = 100;

    #[test]
    fn session_capture_agent_updates_become_movement_intent() {
        use ferroan_capture::{Kind, read_capture};
        use ferroan_protocol::msgs::{AgentUpdate, SetAlwaysRun, UseCircuitCode};
        use ferroan_protocol::{Msg, parse_packet};

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/captures/session.cap"
        );
        let bytes = std::fs::read(path).unwrap();
        let records = read_capture(&bytes).unwrap();
        // The capture holds nine circuits under one redacted agent id. Each
        // `UseCircuitCode` starts a new circuit: LLClientView's filter and the
        // presence start over. Two circuits opened in one burst (the owner and
        // the chat account of `build_scenario`) share ids, so a datagram goes
        // to the newest circuit of the latest burst that has not seen its
        // sequence. One every circuit of that burst has seen is a reliable
        // duplicate the UDP server drops (`PacketArchive`).
        let mut circuits: Vec<Replay> = Vec::new();
        let mut agent_updates = 0u32;
        let mut always_runs = 0u32;
        let mut movement_kept = 0u32;
        let mut camera_kept = 0u32;
        let mut duplicates = 0u32;
        let mut filtered = 0u32;
        let mut last_open: Option<u64> = None;

        for record in records {
            if record.kind != Kind::UdpIn {
                continue;
            }
            let Ok(packet) = parse_packet(&record.bytes) else {
                continue;
            };
            if packet.id == UseCircuitCode::ID {
                let burst = match (circuits.last(), last_open) {
                    (Some(last), Some(at)) if record.time_ms.saturating_sub(at) <= BURST_MS => {
                        last.burst
                    }
                    (Some(last), _) => last.burst + 1,
                    (None, _) => 0,
                };
                last_open = Some(record.time_ms);
                let pose = login_pose();
                circuits.push(Replay {
                    body: Body::new(&pose),
                    pose,
                    seen: std::collections::HashSet::from([packet.sequence]),
                    burst,
                    updates: 0,
                });
                continue;
            }
            let Some(latest) = circuits.last().map(|circuit| circuit.burst) else {
                continue;
            };
            let Some(circuit) = circuits
                .iter_mut()
                .rev()
                .take_while(|circuit| circuit.burst == latest)
                .find(|circuit| !circuit.seen.contains(&packet.sequence))
            else {
                if packet.id == AgentUpdate::ID {
                    agent_updates += 1;
                    duplicates += 1;
                }
                continue;
            };
            circuit.seen.insert(packet.sequence);
            if packet.id == AgentUpdate::ID {
                let message = packet.decode::<AgentUpdate>().unwrap();
                agent_updates += 1;
                circuit.updates += 1;
                let update = MotionUpdate {
                    now_ms: record.time_ms,
                    sequence: packet.sequence,
                    child: false,
                    in_transit: false,
                    body_rotation: message.agent_data.body_rotation.to_array(),
                    head_rotation: message.agent_data.head_rotation.to_array(),
                    state: message.agent_data.state,
                    camera_center: message.agent_data.camera_center.to_array(),
                    camera_at: message.agent_data.camera_at_axis.to_array(),
                    camera_left: message.agent_data.camera_left_axis.to_array(),
                    camera_up: message.agent_data.camera_up_axis.to_array(),
                    far: message.agent_data.far,
                    control_flags: message.agent_data.control_flags,
                    flags: message.agent_data.flags,
                };
                let applied = circuit.body.apply(&update, &mut circuit.pose);
                if applied.dropped {
                    filtered += 1;
                    continue;
                }
                let view = circuit.body.view();
                if applied.movement {
                    movement_kept += 1;
                    assert_eq!(view.control_flags, update.control_flags);
                    assert_eq!(view.draw_distance, update.far);
                    if !view.sit_ground {
                        assert_eq!(circuit.pose.rotation, update.body_rotation);
                    }
                }
                if applied.camera {
                    camera_kept += 1;
                    assert_eq!(view.camera_center, update.camera_center);
                    assert_eq!(view.draw_distance, update.far);
                }
            } else if packet.id == SetAlwaysRun::ID {
                let message = packet.decode::<SetAlwaysRun>().unwrap();
                always_runs += 1;
                circuit.body.set_always_run(message.agent_data.always_run);
                assert_eq!(
                    circuit.body.view().always_run,
                    message.agent_data.always_run
                );
            }
        }
        let walked: Vec<&Replay> = circuits.iter().filter(|c| c.updates > 0).collect();
        let bursts = circuits.last().map_or(0, |circuit| circuit.burst + 1);
        assert_eq!(
            (
                circuits.len(),
                bursts,
                walked.len(),
                agent_updates,
                movement_kept,
                camera_kept,
                duplicates,
                filtered,
                always_runs,
            ),
            (9, 7, 3, 9, 9, 4, 0, 0, 0)
        );

        // The walk circuit: seven held-forward updates, all kept for movement.
        // The camera is forced on the first and again once 500 ms have passed.
        let walk = walked[0];
        let view = walk.body.view();
        assert_eq!(walk.updates, 7);
        assert_eq!(view.control_flags, CONTROL_AT_POS);
        let speed = (view.target_velocity[0] * view.target_velocity[0]
            + view.target_velocity[1] * view.target_velocity[1]
            + view.target_velocity[2] * view.target_velocity[2])
            .sqrt();
        assert!((speed - (0.6 * 4.096)).abs() < 1.0e-4, "{speed}");
        assert!((view.target_velocity[0] - 2.455_391_4).abs() < 1.0e-5);
        assert!((view.target_velocity[1] - 0.104_168_13).abs() < 1.0e-5);
        assert_eq!(view.target_velocity[2], 0.0);

        // The owner and the chat account of the build burst: each sends one
        // standing update (the same sequence 7 on its own circuit), and each
        // stands: no controls held, no command.
        assert_eq!(walked[1].burst, walked[2].burst);
        for stand in &walked[1..] {
            let view = stand.body.view();
            assert_eq!(stand.updates, 1);
            assert_eq!(stand.pose.control_flags, view.control_flags);
            assert_eq!(view.control_flags, 0);
            assert_eq!(view.movement_flags, 0);
            assert_eq!(view.target_velocity, [0.0, 0.0, 0.0]);
            assert_eq!(view.delayed_stop_ms, -1);
            assert!(!view.flying && !view.sit_ground && !view.always_run);
            assert_eq!(view.flags, 0);
            assert_eq!(view.state, 0);
            assert_eq!(view.draw_distance, 128.0);
            assert_eq!(stand.pose.draw_distance, 128.0);
        }
    }

    #[test]
    fn a_camera_only_update_keeps_the_far_of_the_last_movement_update() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        body.apply(&update(1_000, 1, 0), &mut pose);
        assert_eq!(body.view().draw_distance, 128.0);
        // Far moves by less than 32 m and the camera moves: camera only.
        let mut camera = update(1_100, 2, 0);
        camera.far = 150.0;
        camera.camera_center = [130.0, 128.0, 26.0];
        let applied = body.apply(&camera, &mut pose);
        assert!(applied.camera && !applied.movement);
        assert_eq!(body.view().draw_distance, 128.0);
        assert_eq!(pose.draw_distance, 128.0);
        // A movement update stores the new far, clamped as `DrawDistance` is.
        let mut near = update(1_200, 3, CONTROL_AT_POS);
        near.far = 4.0;
        body.apply(&near, &mut pose);
        assert_eq!(body.view().draw_distance, 32.0);
        let mut far = update(1_300, 4, 0);
        far.far = 1_000.0;
        body.apply(&far, &mut pose);
        assert_eq!(pose.draw_distance, MAX_DRAW_DISTANCE);
    }

    #[test]
    fn the_first_update_on_a_new_clock_is_forced() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let applied = body.apply(&update(0, 1, 0), &mut pose);
        assert!(applied.movement && applied.camera);
        let later = clock_ms(Instant::now() + std::time::Duration::from_millis(5));
        assert!(later >= clock_ms(Instant::now()));
    }

    #[test]
    fn a_halt_keeps_the_angular_velocity_and_the_delayed_stop() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let roll = CONTROL_FLY | CONTROL_TURN_LEFT | CONTROL_YAW_POS | CONTROL_AT_POS;
        body.apply(&update(1_000, 1, roll), &mut pose);
        body.apply(&update(1_030, 2, CONTROL_FLY), &mut pose);
        let before = body.view();
        assert!(before.angular_velocity[2] != 0.0);
        assert!(before.delayed_stop_ms > 0);
        body.halt_command();
        let after = body.view();
        assert_eq!(after.target_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(body.physics_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(after.angular_velocity, before.angular_velocity);
        assert_eq!(after.delayed_stop_ms, before.delayed_stop_ms);
        assert!(body.flying());
        body.set_child(true);
        assert!(!body.flying());
    }

    #[test]
    fn a_return_copies_the_agent_data_fields() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let yaw = std::f32::consts::FRAC_1_SQRT_2;
        body.apply_return(
            &ReturnData {
                velocity: [1.0, 2.0, 0.0],
                far: 16.0,
                body_rotation: [0.0, 0.0, yaw, yaw],
                head_rotation: [0.0, 0.0, 0.5, 0.5],
                control_flags: CONTROL_FLY,
                always_run: true,
            },
            &mut pose,
        );
        let view = body.view();
        assert_eq!(pose.velocity, [1.0, 2.0, 0.0]);
        assert_eq!(pose.draw_distance, 32.0);
        assert_eq!(pose.rotation, [0.0, 0.0, yaw, yaw]);
        assert_eq!(view.head_rotation, [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(pose.control_flags, CONTROL_FLY);
        assert!(view.always_run && view.flying && body.flying());
        // A zero rotation is not usable and the previous one stays.
        body.apply_return(
            &ReturnData {
                velocity: [0.0, 0.0, 0.0],
                far: 255.0,
                body_rotation: [0.0, 0.0, 0.0, 0.0],
                head_rotation: [0.0, 0.0, 0.0, 1.0],
                control_flags: 0,
                always_run: false,
            },
            &mut pose,
        );
        assert_eq!(pose.rotation, [0.0, 0.0, yaw, yaw]);
        assert!(!body.view().flying && !body.view().always_run);
    }

    #[test]
    fn flying_turn_rolls_and_then_decays() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let flags = CONTROL_FLY | CONTROL_TURN_LEFT | CONTROL_YAW_POS;
        body.apply(&update(1_000, 1, flags), &mut pose);
        assert!((body.view().angular_velocity[2] - FLY_ROLL_STEP).abs() < 1.0e-6);
        body.apply(&update(1_030, 2, CONTROL_FLY), &mut pose);
        assert!(body.view().angular_velocity[2] < FLY_ROLL_STEP);
        assert!(body.view().angular_velocity[2] > 0.0);
    }

    #[test]
    fn a_tick_uses_the_physics_velocity_and_the_stop_lands_after_that_move() {
        let mut pose = login_pose();
        pose.velocity = [9.0, 0.0, 0.0];
        let mut body = Body::new(&pose);
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_000,
        );
        assert_eq!(pose.position[0], 128.0);
        assert_eq!(pose.position[2], 21.0 + crate::walk::AVATAR_SIZE_Z);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);

        body.apply(&update(1_000, 1, CONTROL_AT_POS), &mut pose);
        let speed = body.view().target_velocity[0];
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_050,
        );
        assert!((pose.position[0] - (128.0 + speed * crate::frame::FRAME_TIME)).abs() < 1.0e-5);
        assert_eq!(pose.velocity[0], speed);
        assert_eq!(pose.velocity[2], 0.0);

        body.apply(&update(1_100, 2, 0), &mut pose);
        let deadline = body.view().delayed_stop_ms;
        assert!(deadline > 1_100);
        let before = pose.position[0];
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            deadline as u64,
        );
        assert!(pose.position[0] > before);
        assert_ne!(pose.velocity[0], 0.0);
        let moved = pose.position[0];
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            deadline as u64 + 1,
        );
        assert!(pose.position[0] > moved);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(body.view().target_velocity, [0.0, 0.0, 0.0]);
        let stopped = pose.position[0];
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            deadline as u64 + 200,
        );
        assert_eq!(pose.position[0], stopped);

        body.apply(&update(2_000, 3, CONTROL_AT_POS), &mut pose);
        body.apply(
            &update(2_030, 4, CONTROL_AT_POS | CONTROL_SIT_ON_GROUND),
            &mut pose,
        );
        let seated = pose.position;
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            2_100,
        );
        assert_eq!(pose.position, seated);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_border_crossing_stops_the_body() {
        let mut pose = login_pose();
        pose.position = [255.6, 128.0, 25.0];
        let mut body = Body::new(&pose);
        body.physics_velocity = [2.4576, 0.0, 0.0];
        body.motion.target_velocity = [2.4576, 0.0, 0.0];
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_000,
        );
        assert!(pose.position[0] < 256.0);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(body.view().target_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(body.physics_velocity, [0.0, 0.0, 0.0]);
        let stopped = pose.position;
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_200,
        );
        assert_eq!(pose.position[0], stopped[0]);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn flying_into_the_ground_zeros_the_command() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        let down = CONTROL_FLY | CONTROL_AT_POS | CONTROL_UP_NEG;
        body.apply(&update(1_000, 1, down), &mut pose);
        assert!(body.view().target_velocity[0] != 0.0);
        assert!(body.view().target_velocity[2] < 0.0);

        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_100,
        );
        // Drop onto the ground so this step is colliding. The next command
        // is still flying, and a downward result zeros the whole command.
        pose.position[2] = 23.0;
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_150,
        );
        assert!(body.colliding);
        body.apply(&update(1_200, 2, down), &mut pose);
        assert_eq!(body.view().target_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(body.physics_velocity, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_child_is_not_integrated() {
        let mut pose = login_pose();
        let mut body = Body::new(&pose);
        body.physics_velocity = [2.0, 0.0, 0.0];
        body.set_child(true);
        let stayed = pose.position;
        body.step_physics(
            &mut pose,
            21.0,
            256.0,
            256.0,
            crate::frame::FRAME_TIME,
            1_000,
        );
        assert_eq!(pose.position, stayed);
        assert_eq!(body.physics_velocity, [2.0, 0.0, 0.0]);
    }
}
