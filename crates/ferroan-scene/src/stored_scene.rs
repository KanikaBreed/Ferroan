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
// Ported from SceneGraph.AddSceneObject, SceneObjectGroup.AttachToScene and
// SendFullAnimUpdateToClient, and SceneObjectPart.ApplyPhysics, AddToPhysics,
// ClampScale and SendUpdateToClient at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b, and Utils.Clamp at libopenmetaverse
// a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Restored world objects share one set of local IDs and cache revisions across viewers.

use std::collections::HashMap;
use std::sync::Arc;

use crate::entity_queue::{FULL_UPDATE, FULL_UPDATE_WITH_ANIM, MATERIAL_OVR};
use crate::prim_update::{LinksetUpdateFlags, PrimUpdateError, StoredPrimUpdate};
use crate::{ObjectGroup, ObjectPart, PriorityEntity};

const PHYSICS: u32 = 1; // PrimFlags.Physics
const PHANTOM: u32 = 0x400; // PrimFlags.Phantom
const VOLUME_DETECT: u32 = 0x4000; // PrimFlags.JointLP2P, read by VolumeDetectActive
const PHYS_SHAPE_NONE: u8 = 1; // PhysShapeType.none
const FLEXIBLE: u8 = 0x80; // Extrusion.Flexible

/// Effective nonphysical scale limits when ClampPrimSize is enabled.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PrimScaleLimits {
    pub min: f32,
    pub max: f32,
}

impl PrimScaleLimits {
    fn apply(self, scale: &mut [f32; 3]) {
        for axis in scale {
            // Utils.Clamp tests the upper bound first, including for reversed limits.
            *axis = if *axis < self.max {
                axis.max(self.min)
            } else {
                self.max
            };
        }
    }
}

/// Physical and nonphysical size limits from Scene's fields.
///
/// [`Default`] is those fields before configuration is read: nonphysical
/// 0.001 to 256, physical 0.01 to 10.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PhysicsScaleLimits {
    pub nonphys_min: f32,
    pub nonphys_max: f32,
    pub phys_min: f32,
    pub phys_max: f32,
}

impl Default for PhysicsScaleLimits {
    fn default() -> Self {
        Self {
            nonphys_min: 0.001,
            nonphys_max: 256.0,
            phys_min: 0.01,
            phys_max: 10.0,
        }
    }
}

/// ClampScale assigns an axis below `min` to `min`, else an axis above `max` to `max`.
fn clamp_scale(scale: &mut [f32; 3], min: f32, max: f32) {
    for axis in scale {
        if *axis < min {
            *axis = min;
        } else if *axis > max {
            *axis = max;
        }
    }
}

/// AddToPhysics runs ClampScale only for a part that would receive an actor.
fn actor_clamps_scale(
    part: &ObjectPart,
    is_root: bool,
    root_flags: u32,
    attachment: bool,
    collidable: bool,
) -> bool {
    if !collidable {
        return false;
    }
    // A root stored as none is replaced with the default shape and still clamped.
    // A child stored as none returns before ClampScale.
    if part.physics_shape_type == PHYS_SHAPE_NONE && !is_root {
        return false;
    }
    let physical = root_flags & PHYSICS != 0;
    let phantom = root_flags & PHANTOM != 0;
    // The column and the JointLP2P bit are the same VolumeDetectActive flag after a load.
    let volume = part.volume_detect || part.flags & VOLUME_DETECT != 0;
    (!phantom || physical || volume) && !attachment && part.shape.path_curve != FLEXIBLE
}

struct PreparedPart {
    group: usize,
    part: usize,
    flags: u32,
    physical_root: bool,
    update: Result<StoredPrimUpdate, PrimUpdateError>,
}

#[derive(Default)]
pub(crate) struct StoredScene {
    pub(crate) groups: Arc<Vec<ObjectGroup>>,
    // Allocation and initial viewer delivery both use this stable link order.
    parts: Vec<PreparedPart>,
    by_local_id: HashMap<u32, usize>,
}

impl StoredScene {
    pub(crate) fn admit(
        mut groups: Arc<Vec<ObjectGroup>>,
        limits: Option<PrimScaleLimits>,
        physics: PhysicsScaleLimits,
        collidable_prims: bool,
        mut allocate: impl FnMut() -> u32,
    ) -> Self {
        let mut parts = Vec::new();
        let mut by_local_id = HashMap::new();
        let mut link_order = Vec::new();
        for (group_index, group) in Arc::make_mut(&mut groups).iter_mut().enumerate() {
            let Some((root, children)) = group.parts.split_first() else {
                continue;
            };
            let permissions = LinksetUpdateFlags::from_parts(root, children);
            // IsAttachmentCheckFull recognizes a stored primitive with a nonzero
            // attachment state. Vegetation uses State to identify its species.
            let attachment = root.shape.pcode == 9 && root.shape.state != 0;
            let root_flags = root.flags;
            let physical_root = collidable_prims
                && root_flags & PHYSICS != 0
                && root.shape.path_curve != FLEXIBLE
                && !attachment;
            let root_id = allocate();
            let mut full = FULL_UPDATE;
            // PartsInLinkOrder keeps the root first and breaks ties by storage
            // order. The store-facing group retains its original array.
            link_order.clear();
            link_order.extend(0..group.parts.len());
            link_order.sort_by_key(|&index| (index != 0, group.parts[index].link_num));
            for &index in &link_order {
                let part = &mut group.parts[index];
                // ClampPrimSize, when enabled, clamps every part to the nonphysical
                // limits. AttachToScene then ClampScales each part that would get an actor.
                if let Some(limits) = limits {
                    limits.apply(&mut part.shape.scale);
                }
                if actor_clamps_scale(part, index == 0, root_flags, attachment, collidable_prims) {
                    let physical = root_flags & PHYSICS != 0;
                    let (min, max) = if physical {
                        (physics.phys_min, physics.phys_max)
                    } else {
                        (physics.nonphys_min, physics.nonphys_max)
                    };
                    clamp_scale(&mut part.shape.scale, min, max);
                }
                let (mesh_flags, probe) =
                    crate::xml::shape_update_entries(&part.shape.extra_params);
                if index == 0 {
                    part.local_id = root_id;
                    part.parent_id = 0;
                    if mesh_flags {
                        full = FULL_UPDATE_WITH_ANIM;
                    }
                } else {
                    part.local_id = allocate();
                    part.parent_id = root_id;
                }
                let overrides = part
                    .shape
                    .materials
                    .as_ref()
                    .and_then(|m| m.overrides.as_ref())
                    .is_some_and(|entries| !entries.is_empty());
                let update_flags = if part.animations.is_some() {
                    full
                } else {
                    FULL_UPDATE
                };
                let flags = if overrides && !probe {
                    update_flags | MATERIAL_OVR
                } else {
                    update_flags
                };
                let update =
                    StoredPrimUpdate::from_store(part, &permissions, part.local_id, root_id);
                by_local_id.insert(part.local_id, parts.len());
                parts.push(PreparedPart {
                    group: group_index,
                    part: index,
                    flags,
                    physical_root,
                    update,
                });
            }
        }
        Self {
            groups,
            parts,
            by_local_id,
        }
    }

    pub(crate) fn initial_parts(&self) -> impl Iterator<Item = (PriorityEntity, u32, u32)> {
        self.parts.iter().map(|prepared| {
            let group = &self.groups[prepared.group];
            let root = &group.parts[0];
            let part = &group.parts[prepared.part];
            let entity = PriorityEntity::Prim {
                id: part.uuid,
                linkset: root.uuid,
                position: root.group_position,
                attachment: false,
                physical: prepared.physical_root,
            };
            (entity, part.local_id, prepared.flags)
        })
    }

    pub(crate) fn get(
        &self,
        id: u32,
    ) -> Option<(&ObjectPart, Result<&StoredPrimUpdate, &PrimUpdateError>)> {
        let entry = &self.parts[*self.by_local_id.get(&id)?];
        Some((
            &self.groups[entry.group].parts[entry.part],
            entry.update.as_ref(),
        ))
    }
}
