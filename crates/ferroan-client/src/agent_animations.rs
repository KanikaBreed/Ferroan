// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The animator fields `AgentData` carries between regions. The region frame
//! owns the animator; these only carry its state across the circuit: out in a
//! source hop (`CopyTo`), in with a returning `UpdateAgent` (`CopyFrom`).

use uuid::Uuid;

/// `STAND`, the default of a new animation set.
const STAND: Uuid = Uuid::from_u128(0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f);
/// `motionControlStates.onsurface`.
const ON_SURFACE: u8 = 6;

/// One animation (`OpenSim.Framework.Animation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anim {
    pub animation: Uuid,
    pub object_id: Uuid,
    pub sequence: i32,
}

/// `Anims`, `DefaultAnim`, `AnimState` and `MotionState`. An incoming set
/// holds what the sender put, and the default is one with none of them. An
/// outgoing set holds both defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentAnimations {
    pub animations: Vec<Anim>,
    pub default_animation: Option<Anim>,
    pub animation_state: Option<Anim>,
    pub motion_state: u8,
}

impl AgentAnimations {
    /// What a new presence's animator copies: `STAND` sequence 1 for both
    /// defaults, on the surface.
    pub fn standing() -> Self {
        let stand = Anim {
            animation: STAND,
            object_id: Uuid::nil(),
            sequence: 1,
        };
        Self {
            animations: Vec::new(),
            default_animation: Some(stand),
            animation_state: Some(stand),
            motion_state: ON_SURFACE,
        }
    }
}
