// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Copyright (c) 2006-2016, openmetaverse.co
// All rights reserved.
//
// - Redistribution and use in source and binary forms, with or without
//   modification, are permitted provided that the following conditions are met:
//
// - Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
// - Neither the name of the openmetaverse.co nor the names
//   of its contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.Framework/Scenes/Animation/ScenePresenceAnimator.cs
//   Source/OpenSim.Region.Framework/Scenes/Animation/AnimationSet.cs
//   Source/OpenSim.Region.Framework/Scenes/Animation/DefaultAvatarAnimations.cs
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (CopyTo, CopyFrom,
//     RegionHeartbeatEnd)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (NextAnimationSequenceNumber, HandleAgentAnimation)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Movement animation ids from
//   OpenMetaverse/Animations.cs
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The avatar's movement animation: `ScenePresenceAnimator` and its
//! `AnimationSet`, on the frame thread ([D6], [D39]).
//!
//! `DetermineMovementAnimation` (lines 260–570) picks one of the movement
//! states from the held controls, the physics actor's flight, contact and
//! vertical speed, and four timers on the scene clock: a fall shows after
//! 800 ms below -3 m/s, a prejump lasts 450 ms, a jump 1050 ms, and a landing
//! holds for 600, 1500 or 3000 ms by fall speed. The walking stand-in
//! ([D38]) supplies contact: a step on the ground collides, a flying step
//! collides only when it snaps to the ground. Its actor never hovers on a PID
//! and its jump impulse does nothing (`BasicActor.AvatarJump` is empty).
//!
//! `UpdateMovementAnimations` (576) runs from `AddNewMovement`, from an
//! `AgentUpdate` that only holds finish-anim, and every second region
//! heartbeat for a root that is not sitting (`RegionHeartbeatEnd`,
//! `ScenePresence.cs` lines 1318–1342). A new state sets the default
//! animation with the next sequence number and the agent as its object,
//! which is when C# sends the animation pack ([`Animator::take_pack_due`]);
//! the frame then queues `AvatarAnimation` for every viewer
//! ([`crate::avatars`]). A viewer's `AgentAnimation` starts and stops the
//! other animations ([`Animator::viewer_change`]). The movement overrides that scripts set
//! (`MovementAnimationOverrides`) are empty: no script runs yet. Nothing
//! puts the presence in transit or sits it on an object, so those branches
//! are not here.
//!
//! The default-animation table here holds the movement states only, by the
//! names `avataranimations.xml` gives them.
//!
//! [D6]: ../../../docs/DECISIONS.md
//! [D38]: ../../../docs/DECISIONS.md
//! [D39]: ../../../docs/DECISIONS.md

use ferroan_diag::{Kind, REGION_AGENT_ANIMATIONS, drop_bound};
use uuid::Uuid;

use crate::motion::{
    CONTROL_AT_NEG, CONTROL_AT_POS, CONTROL_FINISH_ANIM, CONTROL_LEFT_NEG, CONTROL_LEFT_POS,
    CONTROL_NUDGE_AT_NEG, CONTROL_NUDGE_AT_POS, CONTROL_NUDGE_LEFT_NEG, CONTROL_NUDGE_LEFT_POS,
    CONTROL_NUDGE_UP_NEG, CONTROL_NUDGE_UP_POS, CONTROL_TURN_LEFT, CONTROL_TURN_RIGHT,
    CONTROL_UP_NEG, CONTROL_UP_POS,
};

/// `motionControlStates.sitted`.
pub const MOTION_SITTED: u8 = 0;
/// `motionControlStates.flying`.
pub const MOTION_FLYING: u8 = 1;
/// `motionControlStates.falling`.
pub const MOTION_FALLING: u8 = 2;
/// `motionControlStates.prejumping`.
pub const MOTION_PREJUMPING: u8 = 3;
/// `motionControlStates.jumping`.
pub const MOTION_JUMPING: u8 = 4;
/// `motionControlStates.landing`.
pub const MOTION_LANDING: u8 = 5;
/// `motionControlStates.onsurface`, where a new animator starts.
pub const MOTION_ON_SURFACE: u8 = 6;

const FALL_DELAY_MS: i64 = 800;
const PREJUMP_DELAY_MS: i64 = 450;
const JUMP_PERIOD_MS: i64 = 1050;
/// `RegionHeartbeatEnd` updates on every second heartbeat.
const HEARTBEATS_PER_UPDATE: u32 = 2;

const ANY_XY: u32 = CONTROL_AT_POS
    | CONTROL_NUDGE_AT_POS
    | CONTROL_AT_NEG
    | CONTROL_NUDGE_AT_NEG
    | CONTROL_LEFT_POS
    | CONTROL_NUDGE_LEFT_POS
    | CONTROL_LEFT_NEG
    | CONTROL_NUDGE_LEFT_NEG;
const ANY_XYZ: u32 =
    ANY_XY | CONTROL_UP_POS | CONTROL_NUDGE_UP_POS | CONTROL_UP_NEG | CONTROL_NUDGE_UP_NEG;

/// One movement state, by the name `DetermineMovementAnimation` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    Stand,
    Walk,
    Run,
    Crouch,
    CrouchWalk,
    TurnLeft,
    TurnRight,
    Fly,
    FlySlow,
    Hover,
    HoverUp,
    HoverDown,
    Land,
    SoftLand,
    StandUp,
    PreJump,
    Jump,
    FallDown,
    /// Sitting on the ground. The animation is the presence's sit animation.
    SitGround,
}

impl Movement {
    /// The state's name, as C# compares and logs it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stand => "STAND",
            Self::Walk => "WALK",
            Self::Run => "RUN",
            Self::Crouch => "CROUCH",
            Self::CrouchWalk => "CROUCHWALK",
            Self::TurnLeft => "TURNLEFT",
            Self::TurnRight => "TURNRIGHT",
            Self::Fly => "FLY",
            Self::FlySlow => "FLYSLOW",
            Self::Hover => "HOVER",
            Self::HoverUp => "HOVER_UP",
            Self::HoverDown => "HOVER_DOWN",
            Self::Land => "LAND",
            Self::SoftLand => "SOFT_LAND",
            Self::StandUp => "STANDUP",
            Self::PreJump => "PREJUMP",
            Self::Jump => "JUMP",
            Self::FallDown => "FALLDOWN",
            Self::SitGround => "SITGROUND",
        }
    }
}

/// The movement rows of `avataranimations.xml`, with the ids
/// libopenmetaverse's `Animations` gives them.
const DEFAULT_ANIMATIONS: [(&str, Uuid); 20] = [
    (
        "CROUCH",
        Uuid::from_u128(0x201f3fdf_cb1f_dbec_201f_7333e328ae7c),
    ),
    (
        "CROUCHWALK",
        Uuid::from_u128(0x47f5f6fb_22e5_ae44_f871_73aaaf4a6022),
    ),
    (
        "FALLDOWN",
        Uuid::from_u128(0x666307d9_a860_572d_6fd4_c3ab8865c094),
    ),
    (
        "FLY",
        Uuid::from_u128(0xaec4610c_757f_bc4e_c092_c6e9caf18daf),
    ),
    (
        "FLYSLOW",
        Uuid::from_u128(0x2b5a38b2_5e00_3a97_a495_4c826bc443e6),
    ),
    (
        "HOVER",
        Uuid::from_u128(0x4ae8016b_31b9_03bb_c401_b1ea941db41d),
    ),
    (
        "HOVER_DOWN",
        Uuid::from_u128(0x20f063ea_8306_2562_0b07_5c853b37b31e),
    ),
    (
        "HOVER_UP",
        Uuid::from_u128(0x62c5de58_cb33_5743_3d07_9e4cd4352864),
    ),
    (
        "JUMP",
        Uuid::from_u128(0x2305bd75_1ca9_b03b_1faa_b176b8a8c49e),
    ),
    (
        "LAND",
        Uuid::from_u128(0x7a17b059_12b2_41b1_570a_186368b6aa6f),
    ),
    (
        "PREJUMP",
        Uuid::from_u128(0x7a4e87fe_de39_6fcb_6223_024b00893244),
    ),
    (
        "RUN",
        Uuid::from_u128(0x05ddbff8_aaa9_92a1_2b74_8fe77a29b445),
    ),
    (
        "SIT_GROUND_CONSTRAINED",
        Uuid::from_u128(0x1a2bd58e_87ff_0df8_0b4c_53e047b0bb6e),
    ),
    (
        "SOFT_LAND",
        Uuid::from_u128(0xf4f00d6e_b9fe_9292_f4cb_0ae06ea58d57),
    ),
    ("STAND", STAND),
    (
        "STANDUP",
        Uuid::from_u128(0x3da1d753_028a_5446_24f3_9c9b856d9422),
    ),
    (
        "TURNLEFT",
        Uuid::from_u128(0x56e0ba0d_4a9f_7f27_6117_32f2ebbf6135),
    ),
    (
        "TURNRIGHT",
        Uuid::from_u128(0x2d6daa51_3192_6794_8e2e_a15f8338ec30),
    ),
    (
        "WALK",
        Uuid::from_u128(0x6ed24bd8_91aa_4b12_ccc7_c97c857ab4e0),
    ),
    // `SIT` is the sit animation once a presence can sit on an object.
    (
        "SIT",
        Uuid::from_u128(0x1a5fe8ac_a804_8a5d_7cbd_56bd83184568),
    ),
];

/// Animations a set holds besides the default. `AvatarAnimation` counts its
/// blocks in a byte, so the default and these fit one packet. C# has no cap.
pub const MAX_OTHER_ANIMATIONS: usize = 254;

/// `STAND`, the default a new animation set holds.
pub const STAND: Uuid = Uuid::from_u128(0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f);

/// `DefaultAvatarAnimations.AnimsUUIDbyName` for the movement states. The
/// lookup is case-sensitive, as `TrySetDefaultAnimation`'s is.
pub fn default_animation(name: &str) -> Option<Uuid> {
    DEFAULT_ANIMATIONS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, id)| *id)
}

/// `OpenSim.Framework.Animation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anim {
    pub animation: Uuid,
    pub sequence: i32,
    pub object_id: Uuid,
}

impl Anim {
    const STANDING: Self = Self {
        animation: STAND,
        sequence: 1,
        object_id: Uuid::nil(),
    };
}

/// The animator fields `AgentData` carries (`CopyTo`, `CopyFrom`). An
/// outgoing set always has both defaults. An incoming one has what the
/// sender put; a motion state of 0 leaves the animator's. The default is
/// an `AgentData` with none of these fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimationTransfer {
    pub animations: Vec<Anim>,
    pub default_animation: Option<Anim>,
    pub animation_state: Option<Anim>,
    pub motion_state: u8,
}

/// `AnimationSet`: the default (movement) animation, the implicit default
/// `CopyTo` sends as `animation_state`, and the other playing animations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationSet {
    default: Anim,
    implicit: Anim,
    others: Vec<Anim>,
}

impl Default for AnimationSet {
    fn default() -> Self {
        Self {
            default: Anim::STANDING,
            implicit: Anim::STANDING,
            others: Vec::new(),
        }
    }
}

impl AnimationSet {
    pub fn default_animation(&self) -> Anim {
        self.default
    }

    pub fn implicit_default(&self) -> Anim {
        self.implicit
    }

    /// The animations besides the default, in the order they were added.
    pub fn others(&self) -> &[Anim] {
        &self.others
    }

    /// `HasAnimation`: the default or any other.
    pub fn has(&self, animation: Uuid) -> bool {
        self.default.animation == animation
            || self.others.iter().any(|anim| anim.animation == animation)
    }

    /// `Add`: false when the set already plays it, or (Ferroan's bound)
    /// already holds [`MAX_OTHER_ANIMATIONS`] others.
    pub fn add(&mut self, animation: Uuid, sequence: i32, object_id: Uuid) -> bool {
        if self.has(animation) {
            return false;
        }
        if self.others.len() >= MAX_OTHER_ANIMATIONS {
            drop_bound(
                REGION_AGENT_ANIMATIONS,
                Kind::Hard,
                MAX_OTHER_ANIMATIONS as u64,
                self.others.len() as u64 + 1,
                "animation-add",
            );
            return false;
        }
        self.others.push(Anim {
            animation,
            sequence,
            object_id,
        });
        true
    }

    /// `Remove`. Removing the default resets it to `STAND`, or with
    /// `allow_no_default` leaves no default (nil, sequence 1), and answers
    /// false either way. Removing another answers true.
    pub fn remove(&mut self, animation: Uuid, allow_no_default: bool) -> bool {
        if self.default.animation == animation {
            if allow_no_default {
                self.default = Anim {
                    animation: Uuid::nil(),
                    sequence: 1,
                    object_id: Uuid::nil(),
                };
            } else {
                self.reset_default();
            }
            return false;
        }
        match self
            .others
            .iter()
            .position(|anim| anim.animation == animation)
        {
            Some(index) => {
                self.others.remove(index);
                true
            }
            None => false,
        }
    }

    /// `Clear`: `STAND` sequence 1 and nothing else. `ResetDefaultAnimation`
    /// keeps the default when it is already `STAND`, sequence and all.
    pub fn clear(&mut self) {
        self.reset_default();
        self.others.clear();
    }

    fn reset_default(&mut self) {
        self.try_set_default("STAND", 1, Uuid::nil());
    }

    /// `SetDefaultAnimation`: a different id replaces the default and the
    /// implicit default; the same id keeps both. Always true.
    pub fn set_default(&mut self, animation: Uuid, sequence: i32, object_id: Uuid) -> bool {
        if self.default.animation != animation {
            self.default = Anim {
                animation,
                sequence,
                object_id,
            };
            self.implicit = self.default;
        }
        true
    }

    /// `SetImplicitDefaultAnimation`, from `CopyFrom` only.
    pub fn set_implicit_default(&mut self, animation: Uuid, sequence: i32, object_id: Uuid) {
        self.implicit = Anim {
            animation,
            sequence,
            object_id,
        };
    }

    /// `TrySetDefaultAnimation`: false for a name the table does not know.
    pub fn try_set_default(&mut self, name: &str, sequence: i32, object_id: Uuid) -> bool {
        match default_animation(name) {
            Some(animation) => self.set_default(animation, sequence, object_id),
            None => false,
        }
    }

    /// `GetArrays`: the default first unless it is nil, then the others.
    pub fn arrays(&self) -> Vec<Anim> {
        let mut out = Vec::with_capacity(self.others.len() + 1);
        if !self.default.animation.is_nil() {
            out.push(self.default);
        }
        out.extend_from_slice(&self.others);
        out
    }

    /// `FromArray`: appends, and answers the highest sequence (0 for none).
    /// Past [`MAX_OTHER_ANIMATIONS`] the rest are dropped, with a readout.
    fn append(&mut self, anims: &[Anim]) -> i32 {
        let room = MAX_OTHER_ANIMATIONS.saturating_sub(self.others.len());
        if anims.len() > room {
            drop_bound(
                REGION_AGENT_ANIMATIONS,
                Kind::Hard,
                MAX_OTHER_ANIMATIONS as u64,
                (self.others.len() + anims.len()) as u64,
                "agent-data",
            );
        }
        let mut highest = 0;
        for anim in anims.iter().take(room) {
            self.others.push(*anim);
            highest = highest.max(anim.sequence);
        }
        highest
    }
}

/// `LLClientView.NextAnimationSequenceNumber`: a counter from a clock-masked
/// start that skips zero and negatives by starting again.
#[derive(Clone, Copy, Debug)]
struct Sequence(i32);

impl Sequence {
    fn start() -> Self {
        Self(clock_bits() & 0x5f_ffaf)
    }

    fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_add(1);
        if self.0 <= 0 {
            self.0 = (clock_bits() & 0xaf_ff5f).wrapping_add(1);
        }
        self.0
    }
}

/// `Util.GetTimeStampTicks()` stands in as the clock's sub-second part.
fn clock_bits() -> i32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    i32::try_from(nanos / 100).unwrap_or(0)
}

/// What the animator reads from the presence and its physics actor.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    /// `SitGround`.
    pub sit_ground: bool,
    /// `AgentControlFlags`.
    pub control_flags: u32,
    /// `PhysicsActor`; none while sitting on the ground.
    pub actor: Option<Actor>,
    /// `AgentControlStopActive`.
    pub stop_active: bool,
    /// `SetAlwaysRun`.
    pub always_run: bool,
    /// `Scene.m_useFlySlow`, `[Startup] enableflyslow` (false as shipped).
    pub fly_slow: bool,
    /// `Environment.TickCount`, here the scene clock.
    pub now_ms: i64,
}

/// The physics actor's flight, contact and vertical speed.
#[derive(Clone, Copy, Debug)]
pub struct Actor {
    pub flying: bool,
    pub colliding: bool,
    pub velocity_z: f32,
}

/// `ScenePresenceAnimator` for one presence.
#[derive(Clone, Debug)]
pub struct Animator {
    set: AnimationSet,
    current: Movement,
    tick_fall: i64,
    tick_land: i64,
    tick_jump: i64,
    jumping: bool,
    falling: bool,
    last_fall_velocity: f32,
    state: u8,
    /// `ScenePresence.sitAnimation`.
    sit_animation: &'static str,
    /// `m_movementAnimationUpdateCounter`.
    heartbeats: u32,
    sequence: Sequence,
    /// The object id of a default the animator sets: the agent.
    owner: Uuid,
    /// C# would have sent the animation pack since the last take.
    pack_due: bool,
}

impl Animator {
    pub fn new(owner: Uuid) -> Self {
        Self {
            set: AnimationSet::default(),
            current: Movement::Stand,
            tick_fall: 0,
            tick_land: 0,
            tick_jump: 0,
            jumping: false,
            falling: false,
            last_fall_velocity: 0.0,
            state: MOTION_ON_SURFACE,
            sit_animation: "SIT",
            heartbeats: 0,
            sequence: Sequence::start(),
            owner,
            pack_due: false,
        }
    }

    pub fn set_owner(&mut self, owner: Uuid) {
        self.owner = owner;
    }

    pub fn current(&self) -> Movement {
        self.current
    }

    pub fn state(&self) -> u8 {
        self.state
    }

    pub fn animations(&self) -> &AnimationSet {
        &self.set
    }

    /// Whether an animation pack is owed since the last call, and clears it.
    pub fn take_pack_due(&mut self) -> bool {
        std::mem::take(&mut self.pack_due)
    }

    /// `HandleAgentAnimation`: `avnChangeAnim` for each block, then the pack
    /// once (`avnChangeAnim(UUID.Zero, false, true)`), whether or not the set
    /// changed. A start adds the animation with the next sequence number and
    /// no object; the number is taken even when the set already plays it or
    /// is full. A stop removes it. Stopping the default resets it
    /// (`ResetDefaultAnimation`): another default becomes `STAND` with
    /// sequence 1, and a `STAND` default stays as it is, sequence and all.
    /// A nil id changes nothing. The caller refuses a child.
    pub fn viewer_change(&mut self, changes: &[(Uuid, bool)]) {
        for &(animation, start) in changes {
            if animation.is_nil() {
                continue;
            }
            if start {
                let sequence = self.sequence.next();
                self.set.add(animation, sequence, Uuid::nil());
            } else {
                self.set.remove(animation, false);
            }
        }
        self.pack_due = true;
    }

    /// `ResetAnimations` (`MakeChildAgent`). The movement state and the
    /// motion state stay.
    pub fn reset(&mut self) {
        self.set.clear();
    }

    /// `HandleAgentSitOnGround`: the sit animation, then `SITGROUND`.
    pub fn sit_on_ground(&mut self) {
        self.sit_animation = "SIT_GROUND_CONSTRAINED";
        self.set_movement(Movement::SitGround);
    }

    /// `StandUp`: the default sit animation again, then `STAND`. The motion
    /// state is the next update's.
    pub fn stand_up(&mut self) {
        self.sit_animation = "SIT";
        self.set_movement(Movement::Stand);
    }

    /// `SetMovementAnimations`.
    pub fn set_movement(&mut self, movement: Movement) -> bool {
        self.current = movement;
        self.try_set_movement(movement)
    }

    /// `UpdateMovementAnimations`. The answer is whether the actor stops
    /// flying: holding down onto the ground while flying lands.
    pub fn update(&mut self, inputs: &Inputs) -> Update {
        let (next, land) = self.determine(inputs);
        let changed = next != self.current;
        if changed {
            self.current = next;
            self.try_set_movement(next);
        }
        Update { changed, land }
    }

    /// `RegionHeartbeatEnd` for a root: every second call updates, unless
    /// the presence sits on the ground.
    pub fn heartbeat(&mut self, inputs: &Inputs) -> Update {
        self.heartbeats += 1;
        if self.heartbeats < HEARTBEATS_PER_UPDATE {
            return Update::default();
        }
        self.heartbeats = 0;
        if inputs.sit_ground {
            return Update::default();
        }
        self.update(inputs)
    }

    /// `TrySetMovementAnimation` with no override: a sit state plays the sit
    /// animation. A sequence number is taken whether or not the name is known.
    fn try_set_movement(&mut self, movement: Movement) -> bool {
        let name = match movement {
            Movement::SitGround => self.sit_animation,
            other => other.name(),
        };
        let sequence = self.sequence.next();
        if self.set.try_set_default(name, sequence, self.owner) {
            self.pack_due = true;
            true
        } else {
            false
        }
    }

    /// `CopyTo`: the other animations, both defaults and the motion state.
    pub fn transfer(&self) -> AnimationTransfer {
        AnimationTransfer {
            animations: self.set.others.clone(),
            default_animation: Some(self.set.default),
            animation_state: Some(self.set.implicit),
            motion_state: self.state,
        }
    }

    /// `CopyFrom`: reset, then the sent defaults with no object, the other
    /// animations appended, the sequence past the highest number seen, and
    /// the motion state unless it is 0.
    pub fn copy_from(&mut self, data: &AnimationTransfer) {
        self.reset();
        let mut highest = self.sequence.next();
        if let Some(anim) = data.default_animation {
            highest = highest.max(anim.sequence);
            self.set
                .set_default(anim.animation, anim.sequence, Uuid::nil());
        }
        if let Some(anim) = data.animation_state {
            highest = highest.max(anim.sequence);
            self.set
                .set_implicit_default(anim.animation, anim.sequence, Uuid::nil());
        }
        highest = highest.max(self.set.append(&data.animations));
        self.sequence = Sequence(highest.wrapping_add(1));
        if data.motion_state != 0 {
            self.state = data.motion_state;
        }
    }

    /// `DetermineMovementAnimation`, with whether the actor stops flying.
    fn determine(&mut self, inputs: &Inputs) -> (Movement, bool) {
        let now = inputs.now_ms;
        if inputs.sit_ground {
            self.state = MOTION_SITTED;
            self.jumping = false;
            self.falling = false;
            return (Movement::SitGround, false);
        }

        let flags = inputs.control_flags;
        // Nudge up is left out so that it does not jump.
        let held_up = flags & CONTROL_UP_POS != 0;
        let held_down = flags & (CONTROL_UP_NEG | CONTROL_NUDGE_UP_NEG) != 0;
        let held_xy = flags & ANY_XY != 0;
        let (turn_left, turn_right) = if flags & ANY_XYZ != 0 {
            (false, false)
        } else {
            (
                flags & CONTROL_TURN_LEFT != 0,
                flags & CONTROL_TURN_RIGHT != 0,
            )
        };
        let finish = flags & CONTROL_FINISH_ANIM != 0;

        let Some(actor) = inputs.actor else {
            self.jumping = false;
            self.falling = false;
            self.state = MOTION_ON_SURFACE;
            let movement = if held_xy {
                Movement::Walk
            } else {
                Movement::Stand
            };
            return (movement, false);
        };
        let colliding = actor.colliding;

        if actor.flying {
            self.tick_fall = 0;
            self.tick_jump = 0;
            self.jumping = false;
            self.falling = false;
            self.state = MOTION_FLYING;
            let movement = if held_xy {
                if inputs.fly_slow {
                    Movement::FlySlow
                } else {
                    Movement::Fly
                }
            } else if held_up {
                Movement::HoverUp
            } else if held_down {
                if colliding {
                    self.state = MOTION_LANDING;
                    self.tick_land = now;
                    return (Movement::Land, true);
                }
                Movement::HoverDown
            } else {
                Movement::Hover
            };
            return (movement, false);
        } else if colliding && self.state == MOTION_FLYING {
            self.state = MOTION_LANDING;
            self.tick_land = now;
            return (Movement::Land, false);
        }

        if !colliding && self.state != MOTION_JUMPING {
            // The stand-in's actor never hovers on a PID.
            let fall_velocity = actor.velocity_z;
            if fall_velocity < -2.5 {
                self.falling = true;
            }
            if self.tick_fall == 0 || fall_velocity >= -0.5 {
                self.tick_fall = now;
            } else {
                let elapsed = now.wrapping_sub(self.tick_fall);
                if elapsed > FALL_DELAY_MS && fall_velocity < -3.0 {
                    self.state = MOTION_FALLING;
                    self.last_fall_velocity = fall_velocity;
                    return (Movement::FallDown, false);
                }
            }
            // The keys were just released mid-air.
            if self.current == Movement::Walk && !held_xy && !held_down && !held_up {
                return (Movement::Stand, false);
            }
            return (self.current, false);
        }

        self.tick_fall = 0;

        if colliding && held_up && !self.jumping {
            self.jumping = true;
            self.falling = false;
            if !finish {
                self.state = MOTION_PREJUMPING;
                self.tick_jump = now;
                return (Movement::PreJump, false);
            }
            // `Jump(9.4f)` is `BasicActor.AvatarJump`, which does nothing.
            self.tick_jump = now - PREJUMP_DELAY_MS - 1;
            self.state = MOTION_JUMPING;
            return (Movement::Jump, false);
        }

        if self.state == MOTION_PREJUMPING {
            if !finish && now.wrapping_sub(self.tick_jump) < PREJUMP_DELAY_MS {
                return (self.current, false);
            }
            self.jumping = true;
            self.state = MOTION_JUMPING;
            return (Movement::Jump, false);
        }

        if self.state == MOTION_JUMPING {
            let jump_time = now.wrapping_sub(self.tick_jump);
            // `jumptime > JUMP_PERIOD * 1.5f` compares as float.
            if jump_time as f32 > JUMP_PERIOD_MS as f32 * 1.5 && colliding {
                self.jumping = false;
                self.falling = false;
                self.tick_land = now;
                self.state = MOTION_LANDING;
                return (Movement::Land, false);
            } else if jump_time > JUMP_PERIOD_MS {
                return (Movement::Jump, false);
            }
            return (self.current, false);
        }

        if self.state == MOTION_FALLING {
            self.falling = false;
            self.state = MOTION_LANDING;
            if !finish {
                let speed_squared = self.last_fall_velocity * self.last_fall_velocity;
                if speed_squared > 300.0 {
                    self.tick_land = now + 3000;
                    return (Movement::StandUp, false);
                }
                if speed_squared > 160.0 {
                    self.tick_land = now + 1500;
                    return (Movement::SoftLand, false);
                }
            }
            self.tick_land = now + 600;
            return (Movement::Land, false);
        }

        if self.state == MOTION_LANDING {
            self.falling = false;
            if !finish && self.tick_land != 0 && self.tick_land > now {
                return (self.current, false);
            }
            self.state = MOTION_ON_SURFACE;
            self.tick_land = 0;
            return (Movement::Stand, false);
        }

        self.state = MOTION_ON_SURFACE;
        self.falling = false;
        if held_xy {
            if held_down {
                return (Movement::CrouchWalk, false);
            }
            // Holding stop keeps the walk or run from starting.
            if !inputs.stop_active {
                let movement = if inputs.always_run {
                    Movement::Run
                } else {
                    Movement::Walk
                };
                return (movement, false);
            }
            return (self.current, false);
        }
        let movement = if held_down {
            Movement::Crouch
        } else if turn_left {
            Movement::TurnLeft
        } else if turn_right {
            Movement::TurnRight
        } else {
            Movement::Stand
        };
        (movement, false)
    }
}

/// One `UpdateMovementAnimations`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Update {
    /// The movement state changed.
    pub changed: bool,
    /// `actor.Flying = false`: the flying avatar held down onto the ground.
    pub land: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: Uuid = Uuid::from_u128(0xa9e17);

    fn ground(control_flags: u32, now_ms: i64) -> Inputs {
        Inputs {
            sit_ground: false,
            control_flags,
            actor: Some(Actor {
                flying: false,
                colliding: true,
                velocity_z: 0.0,
            }),
            stop_active: false,
            always_run: false,
            fly_slow: false,
            now_ms,
        }
    }

    fn air(flying: bool, colliding: bool, velocity_z: f32, flags: u32, now_ms: i64) -> Inputs {
        Inputs {
            actor: Some(Actor {
                flying,
                colliding,
                velocity_z,
            }),
            ..ground(flags, now_ms)
        }
    }

    fn walk_id() -> Uuid {
        default_animation("WALK").unwrap()
    }

    #[test]
    fn a_new_animator_stands_on_the_surface_with_stand_sequence_one() {
        let animator = Animator::new(AGENT);
        assert_eq!(animator.current(), Movement::Stand);
        assert_eq!(animator.state(), MOTION_ON_SURFACE);
        let set = animator.animations();
        assert_eq!(set.default_animation(), Anim::STANDING);
        assert_eq!(set.implicit_default(), Anim::STANDING);
        assert_eq!(set.arrays(), [Anim::STANDING]);
    }

    #[test]
    fn walk_then_release_stands_with_the_next_two_sequence_numbers() {
        let mut animator = Animator::new(AGENT);
        let walk = animator.update(&ground(CONTROL_AT_POS, 1000));
        assert_eq!(
            walk,
            Update {
                changed: true,
                land: false
            }
        );
        assert_eq!(animator.current(), Movement::Walk);
        assert!(animator.take_pack_due());
        let first = animator.animations().default_animation();
        assert_eq!(first.animation, walk_id());
        // The default carries the agent as its object.
        assert_eq!(first.object_id, AGENT);
        assert_eq!(animator.animations().implicit_default(), first);

        // Still held: no change, no pack, no sequence taken.
        assert!(!animator.update(&ground(CONTROL_AT_POS, 1100)).changed);
        assert!(!animator.take_pack_due());

        assert!(animator.update(&ground(0, 1200)).changed);
        let stand = animator.animations().default_animation();
        assert_eq!(stand.animation, STAND);
        assert_eq!(stand.sequence, first.sequence + 1);
        assert_eq!(animator.state(), MOTION_ON_SURFACE);
    }

    #[test]
    fn always_run_runs_and_stop_keeps_the_last_state() {
        let mut animator = Animator::new(AGENT);
        let mut run = ground(CONTROL_AT_POS, 0);
        run.always_run = true;
        animator.update(&run);
        assert_eq!(animator.current(), Movement::Run);
        assert_eq!(
            animator.animations().default_animation().animation,
            default_animation("RUN").unwrap()
        );

        let mut stopped = Animator::new(AGENT);
        let mut held = ground(CONTROL_AT_POS, 0);
        held.stop_active = true;
        assert!(!stopped.update(&held).changed);
        assert_eq!(stopped.current(), Movement::Stand);
    }

    #[test]
    fn crouch_turns_and_crouchwalk_follow_the_controls() {
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_UP_NEG, 0));
        assert_eq!(animator.current(), Movement::Crouch);
        animator.update(&ground(CONTROL_UP_NEG | CONTROL_AT_POS, 0));
        assert_eq!(animator.current(), Movement::CrouchWalk);
        animator.update(&ground(CONTROL_TURN_LEFT, 0));
        assert_eq!(animator.current(), Movement::TurnLeft);
        // A direction key hides the turn.
        animator.update(&ground(CONTROL_TURN_RIGHT | CONTROL_NUDGE_UP_POS, 0));
        assert_eq!(animator.current(), Movement::Stand);
        animator.update(&ground(CONTROL_TURN_RIGHT, 0));
        assert_eq!(animator.current(), Movement::TurnRight);
    }

    #[test]
    fn fly_hover_and_land_on_the_ground() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(true, false, 0.0, CONTROL_AT_POS, 0));
        assert_eq!(animator.current(), Movement::Fly);
        assert_eq!(animator.state(), MOTION_FLYING);
        animator.update(&air(true, false, 0.0, 0, 0));
        assert_eq!(animator.current(), Movement::Hover);
        animator.update(&air(true, false, 0.0, CONTROL_UP_POS, 0));
        assert_eq!(animator.current(), Movement::HoverUp);
        animator.update(&air(true, false, 0.0, CONTROL_UP_NEG, 0));
        assert_eq!(animator.current(), Movement::HoverDown);

        // Down onto the ground: land, and the actor stops flying.
        let update = animator.update(&air(true, true, 0.0, CONTROL_UP_NEG, 500));
        assert_eq!(
            update,
            Update {
                changed: true,
                land: true
            }
        );
        assert_eq!(animator.current(), Movement::Land);
        assert_eq!(animator.state(), MOTION_LANDING);
        // The land tick is "now", so the next update stands.
        animator.update(&ground(0, 500));
        assert_eq!(animator.current(), Movement::Stand);
        assert_eq!(animator.state(), MOTION_ON_SURFACE);
    }

    /// `[Startup] enableflyslow` plays `FLYSLOW` for a flight with a direction.
    #[test]
    fn enableflyslow_flies_slow() {
        let mut animator = Animator::new(AGENT);
        let mut inputs = air(true, false, 0.0, CONTROL_AT_POS, 0);
        inputs.fly_slow = true;
        animator.update(&inputs);
        assert_eq!(animator.current(), Movement::FlySlow);
        assert_eq!(
            animator.animations().default_animation().animation,
            default_animation("FLYSLOW").unwrap()
        );
        assert_eq!(animator.state(), MOTION_FLYING);
        // No direction still hovers.
        inputs.control_flags = 0;
        animator.update(&inputs);
        assert_eq!(animator.current(), Movement::Hover);
    }

    #[test]
    fn fly_released_onto_the_ground_lands_then_stands() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(true, false, 0.0, 0, 0));
        assert_eq!(animator.current(), Movement::Hover);
        // Fly off, still in the air: the hover stays until the ground.
        assert!(!animator.update(&air(false, false, 0.0, 0, 100)).changed);
        assert_eq!(animator.state(), MOTION_FLYING);
        let update = animator.update(&ground(0, 200));
        assert_eq!(
            update,
            Update {
                changed: true,
                land: false
            }
        );
        assert_eq!(animator.current(), Movement::Land);
        animator.update(&ground(0, 300));
        assert_eq!(animator.current(), Movement::Stand);
    }

    #[test]
    fn a_long_fast_fall_shows_falldown_then_lands_by_speed() {
        for (speed, landing, hold) in [
            (-5.0, Movement::Land, 600),
            (-13.0, Movement::SoftLand, 1500),
            (-18.0, Movement::StandUp, 3000),
        ] {
            let mut animator = Animator::new(AGENT);
            // The first update in the air starts the fall clock.
            animator.update(&air(false, false, speed, 0, 1000));
            assert_eq!(animator.current(), Movement::Stand);
            assert!(animator.falling);
            // 800 ms is not yet a fall.
            animator.update(&air(false, false, speed, 0, 1800));
            assert_eq!(animator.current(), Movement::Stand, "{speed}");
            animator.update(&air(false, false, speed, 0, 1801));
            assert_eq!(animator.current(), Movement::FallDown, "{speed}");
            assert_eq!(animator.state(), MOTION_FALLING);

            animator.update(&ground(0, 2000));
            assert_eq!(animator.current(), landing, "{speed}");
            assert_eq!(animator.state(), MOTION_LANDING);
            // The landing holds until its tick, then stands.
            animator.update(&ground(0, 1999 + hold));
            assert_eq!(animator.current(), landing, "{speed}");
            animator.update(&ground(0, 2000 + hold));
            assert_eq!(animator.current(), Movement::Stand, "{speed}");
            assert_eq!(animator.state(), MOTION_ON_SURFACE);
        }
    }

    #[test]
    fn a_slow_descent_is_not_a_fall_and_finish_anim_skips_the_hold() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(false, false, -0.4, 0, 0));
        animator.update(&air(false, false, -0.4, 0, 5000));
        assert_eq!(animator.current(), Movement::Stand);

        let mut animator = Animator::new(AGENT);
        // A fall clock of 0 counts as unset, as `m_animTickFall == 0` does.
        animator.update(&air(false, false, -20.0, 0, 0));
        animator.update(&air(false, false, -20.0, 0, 100));
        assert_eq!(animator.current(), Movement::Stand);
        animator.update(&air(false, false, -20.0, 0, 901));
        assert_eq!(animator.current(), Movement::FallDown);
        animator.update(&ground(CONTROL_FINISH_ANIM, 950));
        assert_eq!(animator.current(), Movement::Land);
        animator.update(&ground(CONTROL_FINISH_ANIM, 951));
        assert_eq!(animator.current(), Movement::Stand);
    }

    #[test]
    fn walking_off_into_the_air_stands() {
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_AT_POS, 0));
        assert_eq!(animator.current(), Movement::Walk);
        // Still held in the air: the walk stays.
        animator.update(&air(false, false, 0.0, CONTROL_AT_POS, 10));
        assert_eq!(animator.current(), Movement::Walk);
        animator.update(&air(false, false, 0.0, 0, 20));
        assert_eq!(animator.current(), Movement::Stand);
    }

    #[test]
    fn prejump_jump_and_land() {
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_UP_POS, 0));
        assert_eq!(animator.current(), Movement::PreJump);
        assert_eq!(animator.state(), MOTION_PREJUMPING);
        animator.update(&ground(CONTROL_UP_POS, 449));
        assert_eq!(animator.current(), Movement::PreJump);
        animator.update(&ground(0, 450));
        assert_eq!(animator.current(), Movement::Jump);
        assert_eq!(animator.state(), MOTION_JUMPING);
        // 1.5 jump periods after the prejump started, on the ground: land.
        animator.update(&ground(0, 1575));
        assert_eq!(animator.current(), Movement::Jump);
        animator.update(&ground(0, 1576));
        assert_eq!(animator.current(), Movement::Land);
        animator.update(&ground(0, 1576));
        assert_eq!(animator.current(), Movement::Stand);

        // Finish-anim skips the prejump.
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_UP_POS | CONTROL_FINISH_ANIM, 0));
        assert_eq!(animator.current(), Movement::Jump);
    }

    #[test]
    fn the_heartbeat_updates_every_second_call_and_not_on_the_ground_sit() {
        let mut animator = Animator::new(AGENT);
        assert!(!animator.heartbeat(&ground(CONTROL_AT_POS, 0)).changed);
        assert_eq!(animator.current(), Movement::Stand);
        assert!(animator.heartbeat(&ground(CONTROL_AT_POS, 0)).changed);
        assert_eq!(animator.current(), Movement::Walk);

        animator.sit_on_ground();
        assert_eq!(animator.current(), Movement::SitGround);
        assert_eq!(
            animator.animations().default_animation().animation,
            default_animation("SIT_GROUND_CONSTRAINED").unwrap()
        );
        let mut seated = ground(CONTROL_AT_POS, 0);
        seated.sit_ground = true;
        seated.actor = None;
        animator.heartbeat(&seated);
        animator.heartbeat(&seated);
        assert_eq!(animator.current(), Movement::SitGround);
        animator.stand_up();
        assert_eq!(animator.current(), Movement::Stand);
        assert_eq!(animator.animations().default_animation().animation, STAND);
    }

    #[test]
    fn a_missing_actor_walks_or_stands_on_the_surface() {
        let mut animator = Animator::new(AGENT);
        let mut inputs = ground(CONTROL_AT_POS, 0);
        inputs.actor = None;
        animator.update(&inputs);
        assert_eq!(animator.current(), Movement::Walk);
        inputs.control_flags = 0;
        animator.update(&inputs);
        assert_eq!(animator.current(), Movement::Stand);
    }

    #[test]
    fn copy_to_carries_the_live_default_and_state() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(true, false, 0.0, CONTROL_AT_POS, 0));
        let other = Anim {
            animation: Uuid::from_u128(5),
            sequence: 3,
            object_id: Uuid::from_u128(6),
        };
        assert!(
            animator
                .set
                .add(other.animation, other.sequence, other.object_id)
        );
        let sent = animator.transfer();
        let fly = sent.default_animation.unwrap();
        assert_eq!(fly.animation, default_animation("FLY").unwrap());
        assert_eq!(fly.object_id, AGENT);
        assert_eq!(sent.animation_state, Some(fly));
        assert_eq!(sent.animations, [other]);
        assert_eq!(sent.motion_state, MOTION_FLYING);
    }

    #[test]
    fn copy_from_resets_to_stand_and_keeps_the_sequence_for_the_same_default_id() {
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_AT_POS, 0));
        let incoming = Uuid::from_u128(77);
        let implicit = Uuid::from_u128(78);
        let object = Uuid::from_u128(79);
        let extra = Anim {
            animation: Uuid::from_u128(80),
            sequence: 1_000_000_000,
            object_id: object,
        };
        animator.copy_from(&AnimationTransfer {
            animations: vec![extra],
            default_animation: Some(Anim {
                animation: incoming,
                sequence: 7,
                object_id: object,
            }),
            animation_state: Some(Anim {
                animation: implicit,
                sequence: 8,
                object_id: object,
            }),
            motion_state: MOTION_LANDING,
        });
        let set = animator.animations();
        // The defaults lose their object; the others keep theirs.
        assert_eq!(
            set.default_animation(),
            Anim {
                animation: incoming,
                sequence: 7,
                object_id: Uuid::nil()
            }
        );
        assert_eq!(
            set.implicit_default(),
            Anim {
                animation: implicit,
                sequence: 8,
                object_id: Uuid::nil()
            }
        );
        assert_eq!(set.others(), [extra]);
        assert_eq!(animator.state(), MOTION_LANDING);
        // The next number is past the highest one seen: `++nanim`, then the increment.
        assert_eq!(animator.sequence.next(), 1_000_000_002);
        // The movement state is not part of the transfer.
        assert_eq!(animator.current(), Movement::Walk);

        // CopyFrom resets even when nothing came, and keeps the motion state for 0.
        animator.copy_from(&AnimationTransfer {
            animations: Vec::new(),
            default_animation: None,
            animation_state: None,
            motion_state: 0,
        });
        assert_eq!(animator.animations().default_animation(), Anim::STANDING);
        assert_eq!(animator.animations().implicit_default(), Anim::STANDING);
        assert!(animator.animations().others().is_empty());
        assert_eq!(animator.state(), MOTION_LANDING);

        // STAND keeps sequence 1 when the incoming default is STAND.
        animator.copy_from(&AnimationTransfer {
            animations: Vec::new(),
            default_animation: Some(Anim {
                animation: STAND,
                sequence: 99,
                object_id: Uuid::nil(),
            }),
            animation_state: None,
            motion_state: 0,
        });
        assert_eq!(animator.animations().default_animation(), Anim::STANDING);
        assert_eq!(animator.animations().implicit_default(), Anim::STANDING);
    }

    /// Falls for 801 ms at `speed`, lands, and answers the landing.
    fn land_after_falling_at(speed: f32) -> Movement {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(false, false, speed, 0, 1000));
        animator.update(&air(false, false, speed, 0, 1801));
        assert_eq!(animator.current(), Movement::FallDown, "{speed}");
        animator.update(&ground(0, 2000));
        animator.current()
    }

    /// `speed * speed > 300` stands up, `> 160` soft-lands. 160 has an f32
    /// root whose square is exactly 160, so that boundary is pinned on the
    /// value. No f32 squares to exactly 300, so `>` and `>=` agree there and
    /// the two neighbouring roots pin it instead.
    #[test]
    fn the_landing_speed_thresholds_are_strict() {
        let root_160 = 160.0_f32.sqrt();
        assert_eq!(root_160 * root_160, 160.0);
        assert_eq!(land_after_falling_at(-root_160), Movement::Land);
        let above_160 = f32::from_bits(root_160.to_bits() + 1);
        assert_eq!(land_after_falling_at(-above_160), Movement::SoftLand);

        let above_300 = 300.0_f32.sqrt();
        let below_300 = f32::from_bits(above_300.to_bits() - 1);
        assert!(below_300 * below_300 < 300.0 && above_300 * above_300 > 300.0);
        assert_eq!(land_after_falling_at(-below_300), Movement::SoftLand);
        assert_eq!(land_after_falling_at(-above_300), Movement::StandUp);
    }

    /// A fall needs more than 800 ms below -3.0: exactly -3.0 is not one.
    #[test]
    fn a_fall_at_exactly_minus_three_is_not_a_fall() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(false, false, -3.0, 0, 1000));
        animator.update(&air(false, false, -3.0, 0, 5000));
        assert_eq!(animator.current(), Movement::Stand);
        assert_eq!(animator.state(), MOTION_ON_SURFACE);
    }

    /// A vertical speed of exactly -0.5 restarts the fall clock.
    #[test]
    fn a_vertical_speed_of_minus_half_restarts_the_fall_clock() {
        let mut animator = Animator::new(AGENT);
        animator.update(&air(false, false, -20.0, 0, 1000));
        animator.update(&air(false, false, -0.5, 0, 1500));
        // 400 ms since the restart, 900 ms since the first update.
        animator.update(&air(false, false, -20.0, 0, 1900));
        assert_eq!(animator.current(), Movement::Stand);
        animator.update(&air(false, false, -20.0, 0, 2301));
        assert_eq!(animator.current(), Movement::FallDown);
    }

    /// Jumping, the jump plays once the time since the prejump passes
    /// `JUMP_PERIOD` (1050 ms), not when it reaches it. A returning motion
    /// state leaves the movement at `STAND` with the jump tick unset, so the
    /// answer before 1050 ms is the current movement.
    #[test]
    fn a_jump_plays_only_after_the_jump_period() {
        let mut animator = Animator::new(AGENT);
        animator.copy_from(&AnimationTransfer {
            motion_state: MOTION_JUMPING,
            ..AnimationTransfer::default()
        });
        animator.update(&ground(0, 1050));
        assert_eq!(animator.current(), Movement::Stand);
        assert_eq!(animator.state(), MOTION_JUMPING);
        animator.update(&ground(0, 1051));
        assert_eq!(animator.current(), Movement::Jump);
    }

    /// The finish-anim shortcut puts the jump tick 451 ms back, so the
    /// landing (more than 1575 ms on) comes 1125 ms after it.
    #[test]
    fn a_finish_anim_jump_starts_451_ms_back() {
        let mut animator = Animator::new(AGENT);
        animator.update(&ground(CONTROL_UP_POS | CONTROL_FINISH_ANIM, 0));
        assert_eq!(animator.current(), Movement::Jump);
        animator.update(&ground(0, 1124));
        assert_eq!(animator.current(), Movement::Jump);
        animator.update(&ground(0, 1125));
        assert_eq!(animator.current(), Movement::Land);
    }

    /// `CopyFrom` seeds the highest number from the animator's own next
    /// one, so lower incoming numbers never take the counter back.
    #[test]
    fn copy_from_never_takes_the_sequence_back() {
        let mut animator = Animator::new(AGENT);
        animator.sequence = Sequence(500);
        animator.copy_from(&AnimationTransfer {
            animations: vec![Anim {
                animation: Uuid::from_u128(80),
                sequence: 9,
                object_id: Uuid::nil(),
            }],
            default_animation: Some(Anim {
                animation: walk_id(),
                sequence: 7,
                object_id: Uuid::nil(),
            }),
            animation_state: None,
            motion_state: 0,
        });
        // `nanim` was 501, then `++nanim` makes 502, and the next is 503.
        assert_eq!(animator.sequence.next(), 503);
    }

    /// `region.agent-animations`: 254 others fit, and the 255th add is
    /// dropped with the set unchanged.
    #[test]
    fn an_add_past_the_animation_bound_is_dropped() {
        let mut set = AnimationSet::default();
        for n in 0..MAX_OTHER_ANIMATIONS {
            assert!(set.add(Uuid::from_u128(1_000 + n as u128), 2, Uuid::nil()));
        }
        assert_eq!(set.others().len(), 254);
        let past = Uuid::from_u128(9_999);
        assert!(!set.add(past, 2, Uuid::nil()));
        assert!(!set.has(past));
        assert_eq!(set.others().len(), 254);
        // The default can still change.
        assert!(set.set_default(walk_id(), 3, AGENT));
    }

    /// A returning `UpdateAgent` with 300 others keeps the first 254. The
    /// sequence seed reads only the kept ones, and the defaults and the
    /// motion state still apply.
    #[test]
    fn copy_from_keeps_at_most_the_animation_bound() {
        let mut animator = Animator::new(AGENT);
        animator.sequence = Sequence(10);
        let animations: Vec<Anim> = (0..300)
            .map(|n| Anim {
                animation: Uuid::from_u128(1_000 + n as u128),
                // Only a dropped entry carries a high number.
                sequence: if n < 254 { 20 + n } else { 900_000 },
                object_id: Uuid::nil(),
            })
            .collect();
        let default = Anim {
            animation: walk_id(),
            sequence: 15,
            object_id: Uuid::nil(),
        };
        animator.copy_from(&AnimationTransfer {
            animations: animations.clone(),
            default_animation: Some(default),
            animation_state: Some(default),
            motion_state: MOTION_FLYING,
        });
        let set = animator.animations();
        assert_eq!(set.others(), &animations[..254]);
        assert_eq!(set.default_animation(), default);
        assert_eq!(set.implicit_default(), default);
        assert_eq!(animator.state(), MOTION_FLYING);
        // The highest kept number is 273: `++nanim` makes 274, the next 275.
        assert_eq!(animator.sequence.next(), 275);
    }

    #[test]
    fn the_set_adds_removes_and_lists_as_csharp_does() {
        let mut set = AnimationSet::default();
        let one = Uuid::from_u128(1);
        assert!(set.add(one, 5, Uuid::nil()));
        assert!(!set.add(one, 6, Uuid::nil()));
        assert!(!set.add(STAND, 6, Uuid::nil()));
        assert!(set.has(one) && set.has(STAND));
        assert_eq!(set.arrays().len(), 2);
        // Removing the default resets it and answers false.
        assert!(set.set_default(walk_id(), 9, AGENT));
        assert!(!set.remove(walk_id(), false));
        assert_eq!(set.default_animation(), Anim::STANDING);
        // With no default allowed, the nil default is left out of the arrays.
        assert!(!set.remove(STAND, true));
        assert_eq!(set.default_animation().animation, Uuid::nil());
        assert_eq!(set.arrays().len(), 1);
        assert!(set.remove(one, false));
        assert!(set.arrays().is_empty());
        // An unknown name sets nothing.
        assert!(!set.try_set_default("stand", 2, Uuid::nil()));
    }

    #[test]
    fn the_sequence_skips_zero_and_negatives() {
        let mut sequence = Sequence(i32::MAX);
        assert!(sequence.next() > 0);
        let mut sequence = Sequence(-5);
        assert!(sequence.next() > 0);
        let mut sequence = Sequence(10);
        assert_eq!(sequence.next(), 11);
    }

    /// `HandleAgentAnimation` through `avnChangeAnim`: a start takes the next
    /// number and no object, even when the set already plays it; a stop of
    /// the default plays `STAND` sequence 1; a nil id takes no number; and
    /// the packet owes one pack whatever it changed.
    #[test]
    fn a_viewer_starts_and_stops_animations_with_one_pack_per_packet() {
        let dance = Uuid::from_u128(0xda_0001);
        let wave = Uuid::from_u128(0xda_0002);
        let mut animator = Animator::new(AGENT);
        animator.sequence = Sequence(100);
        animator.viewer_change(&[(dance, true), (Uuid::nil(), true), (dance, true)]);
        assert!(animator.take_pack_due());
        assert!(!animator.take_pack_due());
        assert_eq!(
            animator.animations().others(),
            [Anim {
                animation: dance,
                sequence: 101,
                object_id: Uuid::nil(),
            }]
        );
        // The repeat took 102.
        animator.viewer_change(&[(wave, true)]);
        assert_eq!(animator.animations().others()[1].sequence, 103);
        assert_eq!(animator.animations().arrays()[0], Anim::STANDING);

        animator.update(&ground(CONTROL_AT_POS, 0));
        let walking = animator.animations().default_animation();
        assert_eq!(walking.animation, walk_id());
        assert_eq!(walking.sequence, 104);
        let _ = animator.take_pack_due();
        animator.viewer_change(&[
            (walk_id(), false),
            (dance, false),
            (Uuid::from_u128(5), false),
        ]);
        assert!(animator.take_pack_due());
        assert_eq!(animator.animations().default_animation(), Anim::STANDING);
        assert_eq!(
            animator.animations().arrays(),
            [
                Anim::STANDING,
                Anim {
                    animation: wave,
                    sequence: 103,
                    object_id: Uuid::nil(),
                }
            ]
        );
        // An empty list still owes the pack.
        animator.viewer_change(&[]);
        assert!(animator.take_pack_due());
        assert_eq!(animator.sequence.next(), 105);

        // Stopping a `STAND` default keeps its sequence: the viewer is not
        // told to restart it.
        let mut animator = Animator::new(AGENT);
        animator.sequence = Sequence(200);
        animator.update(&ground(CONTROL_AT_POS, 1_000));
        animator.update(&ground(0, 1_100));
        let stand = animator.animations().default_animation();
        assert_eq!((stand.animation, stand.sequence), (STAND, 202));
        animator.viewer_change(&[(STAND, false)]);
        assert_eq!(animator.animations().default_animation(), stand);
    }

    /// A full set drops the start but still takes its number.
    #[test]
    fn a_start_past_the_bound_takes_a_number_and_adds_nothing() {
        let mut animator = Animator::new(AGENT);
        animator.sequence = Sequence(0);
        let starts: Vec<(Uuid, bool)> = (0..=MAX_OTHER_ANIMATIONS as u128)
            .map(|n| (Uuid::from_u128(1_000 + n), true))
            .collect();
        animator.viewer_change(&starts);
        assert_eq!(animator.animations().others().len(), MAX_OTHER_ANIMATIONS);
        assert_eq!(animator.animations().arrays().len(), 255);
        assert_eq!(animator.sequence.next(), 256);
    }
}
