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
// SendFullAnimUpdateToClient at Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b,
// and Utils.Clamp at libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Restored world objects share one set of local IDs and cache revisions across viewers.

use std::collections::HashMap;
use std::sync::Arc;

use crate::entity_queue::{FULL_UPDATE, FULL_UPDATE_WITH_ANIM, MATERIAL_OVR};
use crate::prim_update::{LinksetUpdateFlags, PrimUpdateError, StoredPrimUpdate};
use crate::{ObjectGroup, ObjectPart};

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

struct PreparedPart {
    group: usize,
    part: usize,
    flags: u32,
    update: Result<StoredPrimUpdate, PrimUpdateError>,
}

#[derive(Default)]
pub(crate) struct StoredScene {
    pub(crate) groups: Arc<Vec<ObjectGroup>>,
    parts: HashMap<u32, PreparedPart>,
}

impl StoredScene {
    pub(crate) fn admit(
        mut groups: Arc<Vec<ObjectGroup>>,
        limits: Option<PrimScaleLimits>,
        mut allocate: impl FnMut() -> u32,
    ) -> Self {
        let mut parts = HashMap::new();
        for (group_index, group) in Arc::make_mut(&mut groups).iter_mut().enumerate() {
            let Some((root, children)) = group.parts.split_first() else {
                continue;
            };
            let permissions = LinksetUpdateFlags::from_parts(root, children);
            let root_id = allocate();
            let mut full = FULL_UPDATE;
            // The store has already restored PartsInLinkOrder, root first.
            for (index, part) in group.parts.iter_mut().enumerate() {
                if let Some(limits) = limits {
                    limits.apply(&mut part.shape.scale);
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
                let flags = if overrides && !probe {
                    full | MATERIAL_OVR
                } else {
                    full
                };
                let update =
                    StoredPrimUpdate::from_store(part, &permissions, part.local_id, root_id);
                parts.insert(
                    part.local_id,
                    PreparedPart {
                        group: group_index,
                        part: index,
                        flags,
                        update,
                    },
                );
            }
        }
        Self { groups, parts }
    }

    pub(crate) fn initial_parts(&self) -> impl Iterator<Item = (&ObjectPart, &ObjectPart, u32)> {
        self.groups.iter().flat_map(move |group| {
            group.parts.iter().map(move |part| {
                // Each nonempty group begins with its root.
                (&group.parts[0], part, self.parts[&part.local_id].flags)
            })
        })
    }

    pub(crate) fn get(
        &self,
        id: u32,
    ) -> Option<(&ObjectPart, Result<&StoredPrimUpdate, &PrimUpdateError>)> {
        let entry = self.parts.get(&id)?;
        Some((
            &self.groups[entry.group].parts[entry.part],
            entry.update.as_ref(),
        ))
    }
}
