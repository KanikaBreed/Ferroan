// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Jolt bind: one physics system, created and destroyed.
//!
//! Pin8's backend calls `Foundation.Init(false)` and then builds a
//! `PhysicsSystem` from three layer tables (`JoltPhysicsBackend.cs`). Precision
//! here is the same single-precision native: the `double-precision` feature is
//! not enabled. The region's layer matrix, job pools and step are later rows.
//! This type is the smallest system joltc can create.

use std::error::Error;
use std::fmt;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use oxijolt_sys::{
    JPH_BroadPhaseLayerInterface_Destroy, JPH_BroadPhaseLayerInterfaceTable_Create,
    JPH_BroadPhaseLayerInterfaceTable_MapObjectToBroadPhaseLayer, JPH_Init,
    JPH_ObjectLayerPairFilter_Destroy, JPH_ObjectLayerPairFilterTable_Create,
    JPH_ObjectVsBroadPhaseLayerFilter_Destroy, JPH_ObjectVsBroadPhaseLayerFilterTable_Create,
    JPH_PhysicsSystem, JPH_PhysicsSystem_Create, JPH_PhysicsSystem_Destroy,
    JPH_PhysicsSystemSettings,
};

/// Jolt Physics release linked through `oxijolt-sys` 1.6.0.
///
/// The crate stamps [`JOLT_COMMIT`] from that release and does not export the
/// version number. `build.rs` of `oxijolt-sys` 1.6.0 names this string as the
/// release of that commit.
pub const JOLT_VERSION: &str = "5.6.0";

/// Full Jolt commit the native library is built from.
pub const JOLT_COMMIT: &str = oxijolt_sys::JOLT_COMMIT;

/// Full joltc commit the native library is built from.
pub const JOLTC_COMMIT: &str = oxijolt_sys::JOLTC_COMMIT;

/// Revision of the `joltc_ext` sources compiled into that library.
pub const JOLTC_EXT_REVISION: &str = oxijolt_sys::JOLTC_EXT_REVISION;

/// The linked native is single precision, cross-platform deterministic, and
/// built without Jolt's asserts. A feature change fails this crate at compile
/// time.
const _: () = {
    assert!(oxijolt_sys::CROSS_PLATFORM_DETERMINISTIC_ENABLED);
    assert!(!oxijolt_sys::ASSERTS_ENABLED);
    assert!(std::mem::size_of::<oxijolt_sys::Real>() == std::mem::size_of::<f32>());
};

/// One object layer and one broad-phase layer. Enough to construct a system.
/// The region's matrix is a later row. The mapped indices are the only ones
/// those counts allow.
const OBJECT_LAYERS: u32 = 1;
const BROAD_PHASE_LAYERS: u32 = 1;
const OBJECT_LAYER: u32 = 0;
const BROAD_PHASE_LAYER: u8 = 0;

/// Capacities passed to `JPH_PhysicsSystem_Create`. Zero would select joltc's
/// own defaults. These are the bind's smoke values, not the region's caps.
const MAX_BODIES: u32 = 1024;
const MAX_BODY_PAIRS: u32 = 1024;
const MAX_CONTACT_CONSTRAINTS: u32 = 1024;

/// `JPH_PhysicsSystem_Create` or `JPH_Init` failed, or a layer table did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeError {
    /// `JPH_Init` returned false.
    Init,
    /// A layer table allocator returned null.
    Layers,
    /// `JPH_PhysicsSystem_Create` returned null.
    System,
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Init => "JPH_Init failed",
            Self::Layers => "Jolt layer table creation failed",
            Self::System => "Jolt physics system creation failed",
        })
    }
}

impl Error for NativeError {}

/// One Jolt physics system.
///
/// Dropping the value destroys the native system and the three layer tables
/// joltc stored inside it. The value is not `Send` because it holds a raw
/// pointer. Create and destroy take a process-local lock, because joltc's
/// system map is unsynchronised.
pub struct PhysicsSystem {
    system: *mut JPH_PhysicsSystem,
}

impl PhysicsSystem {
    /// Creates one physics system.
    ///
    /// `JPH_Init` runs once per process. This function does not call
    /// `JPH_Shutdown`. A later drop destroys this system only.
    pub fn new() -> Result<Self, NativeError> {
        ensure_foundation()?;
        let _guard = system_lock();
        Ok(Self {
            system: create_system()?,
        })
    }
}

impl Drop for PhysicsSystem {
    fn drop(&mut self) {
        let _guard = system_lock();
        // SAFETY:
        // Ownership: `system` is the pointer `JPH_PhysicsSystem_Create` returned
        // to this value, and this is the one destroy. That call also deletes the
        // three layer tables it took ownership of.
        // Lifetime: the pointer is not used again after this call. No other
        // value holds it.
        // Threading: `system_lock` is held, so the unsynchronised system map is
        // not written by another create or destroy.
        // Bounds: the call takes that one pointer. It does not index a layer
        // or a body.
        unsafe {
            JPH_PhysicsSystem_Destroy(self.system);
        }
    }
}

/// Calls `JPH_Init` once. A false result stays false: joltc's init flag is a
/// plain bool, and a second call is not a retry.
fn ensure_foundation() -> Result<(), NativeError> {
    static INIT: OnceLock<bool> = OnceLock::new();
    let initialized = *INIT.get_or_init(|| {
        // SAFETY:
        // Ownership: `JPH_Init` takes no pointer. This process does not call
        // `JPH_Shutdown`; the foundation stays up so a later system can be
        // created without racing the init flag.
        // Lifetime: no pointer is passed or returned.
        // Threading: `OnceLock` runs this once, and `PhysicsSystem::new` calls
        // it before taking `system_lock`, so init does not overlap a system
        // create on another thread.
        // Bounds: no buffer and no layer index.
        unsafe { JPH_Init() }
    });
    if initialized {
        Ok(())
    } else {
        Err(NativeError::Init)
    }
}

/// Serialises `JPH_PhysicsSystem_Create` and `JPH_PhysicsSystem_Destroy`.
fn system_lock() -> MutexGuard<'static, ()> {
    static SYSTEMS: Mutex<()> = Mutex::new(());
    SYSTEMS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Builds the three layer tables and one physics system.
///
/// The caller holds [`system_lock`] and has already initialised Jolt.
fn create_system() -> Result<*mut JPH_PhysicsSystem, NativeError> {
    // SAFETY:
    // Ownership: each `*_Create` returns one heap object this block owns.
    // `JPH_PhysicsSystem_Create` copies the three pointers into the system and
    // `JPH_PhysicsSystem_Destroy` deletes them. If a create returns null, this
    // block deletes the objects it still owns and does not hand them over.
    // Lifetime: `settings` is a local. Create copies the pointers out of it
    // before it returns. The returned system pointer is the only one kept.
    // Threading: the caller holds `system_lock` and has called `JPH_Init`, so
    // the unsynchronised system map is not written concurrently.
    // Bounds: the tables are created with one object layer and one broad-phase
    // layer. The only map call uses object layer 0 and broad-phase layer 0,
    // both inside those counts. The three capacities are the non-zero constants
    // above. joltc passes `numBodyMutexes` 0 through to Jolt, and
    // `BodyManager::Init` auto-detects `2 * hardware_concurrency`, rounds that
    // up to a power of two, and caps it at 64.
    unsafe {
        let pair = JPH_ObjectLayerPairFilterTable_Create(OBJECT_LAYERS);
        if pair.is_null() {
            return Err(NativeError::Layers);
        }
        let broad = JPH_BroadPhaseLayerInterfaceTable_Create(OBJECT_LAYERS, BROAD_PHASE_LAYERS);
        if broad.is_null() {
            JPH_ObjectLayerPairFilter_Destroy(pair);
            return Err(NativeError::Layers);
        }
        JPH_BroadPhaseLayerInterfaceTable_MapObjectToBroadPhaseLayer(
            broad,
            OBJECT_LAYER,
            BROAD_PHASE_LAYER,
        );
        let versus = JPH_ObjectVsBroadPhaseLayerFilterTable_Create(
            broad,
            BROAD_PHASE_LAYERS,
            pair,
            OBJECT_LAYERS,
        );
        if versus.is_null() {
            JPH_BroadPhaseLayerInterface_Destroy(broad);
            JPH_ObjectLayerPairFilter_Destroy(pair);
            return Err(NativeError::Layers);
        }
        let settings = JPH_PhysicsSystemSettings {
            maxBodies: MAX_BODIES,
            numBodyMutexes: 0,
            maxBodyPairs: MAX_BODY_PAIRS,
            maxContactConstraints: MAX_CONTACT_CONSTRAINTS,
            _padding: 0,
            broadPhaseLayerInterface: broad,
            objectLayerPairFilter: pair,
            objectVsBroadPhaseLayerFilter: versus,
        };
        let system = JPH_PhysicsSystem_Create(&settings);
        if system.is_null() {
            JPH_ObjectVsBroadPhaseLayerFilter_Destroy(versus);
            JPH_BroadPhaseLayerInterface_Destroy(broad);
            JPH_ObjectLayerPairFilter_Destroy(pair);
            return Err(NativeError::System);
        }
        Ok(system)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_physics_system_is_created_and_dropped() {
        assert_eq!(JOLT_VERSION, "5.6.0");
        assert_eq!(JOLT_COMMIT, "e77f175595e64cb44218cc9d9d56fc365ad0e36a");
        assert_eq!(JOLTC_COMMIT, "886e088675bae3a086f8318c7803f8ee962c2f2c");
        assert_eq!(JOLTC_EXT_REVISION, "30");

        let system = PhysicsSystem::new().expect("one physics system");
        drop(system);
    }
}
