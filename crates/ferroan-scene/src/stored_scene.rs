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
// Ported from SceneGraph.AddSceneObject, SceneObjectGroup.AttachToScene,
// SendFullAnimUpdateToClient and DeleteGroupFromScene, Scene.Inventory.cs
// DeRezObjects, and SceneObjectPart.ApplyPhysics, AddToPhysics, ClampScale
// and SendUpdateToClient at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b, and Utils.Clamp at libopenmetaverse
// a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Restored world objects share one set of local IDs and cache revisions across viewers.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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

/// Facts the frame needs after it drops the part borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PartFacts {
    pub root_local_id: u32,
    /// Owner of the root. `CanDeleteObject` matches the agent against that owner.
    pub owner_id: Uuid,
    pub is_root: bool,
    /// `SceneObjectGroup.IsAttachment` (`m_isAttachment`). The setter is the
    /// only writer, and a group restored from the store never calls it.
    pub attachment: bool,
}

/// One live part and the rest of its linkset. The slices borrow the stored group.
pub(crate) struct LiveLinkset<'a> {
    pub part: &'a ObjectPart,
    pub root: &'a ObjectPart,
    pub children: &'a [ObjectPart],
}

/// A hidden part of a removed group, for the deleted-update kill conversion.
pub(crate) struct RemovedPart {
    pub root_local_id: u32,
    pub is_root: bool,
}

#[derive(Default)]
pub(crate) struct StoredScene {
    pub(crate) groups: Arc<Vec<ObjectGroup>>,
    // Allocation and initial viewer delivery both use this stable link order.
    parts: Vec<PreparedPart>,
    by_local_id: HashMap<u32, usize>,
    by_uuid: HashMap<Uuid, u32>,
    /// In-memory hide. The store row stays. `get` pretends the part is gone.
    removed: Mutex<HashSet<u32>>,
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
        let mut by_uuid = HashMap::new();
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
                by_uuid.insert(part.uuid, part.local_id);
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
            by_uuid,
            removed: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn initial_parts(&self) -> impl Iterator<Item = (PriorityEntity, u32, u32)> {
        self.parts.iter().filter_map(|prepared| {
            let group = &self.groups[prepared.group];
            let root = &group.parts[0];
            let part = &group.parts[prepared.part];
            if self.hidden(part.local_id) {
                return None;
            }
            let entity = PriorityEntity::Prim {
                id: part.uuid,
                linkset: root.uuid,
                position: root.group_position,
                attachment: false,
                physical: prepared.physical_root,
            };
            Some((entity, part.local_id, prepared.flags))
        })
    }

    pub(crate) fn get(
        &self,
        id: u32,
    ) -> Option<(&ObjectPart, Result<&StoredPrimUpdate, &PrimUpdateError>)> {
        if self.hidden(id) {
            return None;
        }
        let entry = &self.parts[*self.by_local_id.get(&id)?];
        Some((
            &self.groups[entry.group].parts[entry.part],
            entry.update.as_ref(),
        ))
    }

    /// The part and its root. A removed id is absent. The root stays `parts[0]`.
    pub(crate) fn parts_of(&self, local_id: u32) -> Option<(&ObjectPart, &ObjectPart)> {
        if self.hidden(local_id) {
            return None;
        }
        let entry = self.parts.get(*self.by_local_id.get(&local_id)?)?;
        let group = &self.groups[entry.group];
        Some((&group.parts[entry.part], &group.parts[0]))
    }

    pub(crate) fn describe(&self, local_id: u32) -> Option<PartFacts> {
        if self.hidden(local_id) {
            return None;
        }
        let entry = self.parts.get(*self.by_local_id.get(&local_id)?)?;
        let group = &self.groups[entry.group];
        let root = &group.parts[0];
        Some(PartFacts {
            root_local_id: root.local_id,
            owner_id: root.owner_id,
            is_root: entry.part == 0,
            attachment: false,
        })
    }

    /// The addressed part, with the root first in `parts`. A removed id is absent.
    pub(crate) fn live_linkset(&self, local_id: u32) -> Option<LiveLinkset<'_>> {
        if self.hidden(local_id) {
            return None;
        }
        let index = *self.by_local_id.get(&local_id)?;
        let entry = self.parts.get(index)?;
        let group = &self.groups[entry.group];
        let root = group.parts.first()?;
        let part = group.parts.get(entry.part)?;
        let children = if group.parts.len() > 1 {
            &group.parts[1..]
        } else {
            &[]
        };
        Some(LiveLinkset {
            part,
            root,
            children,
        })
    }

    /// A queued update whose part was hidden with its group. `ProcessEntityUpdates`
    /// turns the root's update into one kill and drops a child.
    pub(crate) fn removed_part(&self, local_id: u32) -> Option<RemovedPart> {
        if !self.hidden(local_id) {
            return None;
        }
        let index = *self.by_local_id.get(&local_id)?;
        let entry = self.parts.get(index)?;
        let root = self.groups[entry.group].parts.first()?;
        Some(RemovedPart {
            root_local_id: root.local_id,
            is_root: entry.part == 0,
        })
    }

    /// Root local id of a live part. A child and its root share that id.
    pub(crate) fn root_of_uuid(&self, uuid: Uuid) -> Option<u32> {
        let local_id = *self.by_uuid.get(&uuid)?;
        self.describe(local_id).map(|facts| facts.root_local_id)
    }

    /// Hide every part of the group whose root has `root_local_id`.
    /// A child id, an unknown id, and a second call return false.
    pub(crate) fn remove_group(&self, root_local_id: u32) -> bool {
        let Some(&index) = self.by_local_id.get(&root_local_id) else {
            return false;
        };
        if self.parts[index].part != 0 {
            return false;
        }
        let group = self.parts[index].group;
        let mut ids = Vec::new();
        for entry in &self.parts {
            if entry.group == group {
                ids.push(entry.part);
            }
        }
        let ids: Vec<u32> = ids
            .into_iter()
            .map(|part| self.groups[group].parts[part].local_id)
            .collect();
        let mut removed = self
            .removed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if removed.contains(&root_local_id) {
            return false;
        }
        removed.extend(ids);
        true
    }

    fn hidden(&self, local_id: u32) -> bool {
        self.removed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(&local_id)
    }
}
