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
// Ported from PermissionsModule.cs and Scene.Permissions.cs at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b. Parcel flags and group powers
// follow ParcelManager.cs and GroupManager.cs at libopenmetaverse
// a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Scene permission decisions for building and inventory operations.

use ferroan_types::{PERM_ALL_EFFECTIVE, PERM_COPY, PERM_MODIFY, PERM_MOVE, PERM_TRANSFER};
use uuid::Uuid;

use crate::client_flags::{
    LinksetPermissions, OBJECT_MANIPULATE, PartPermissions, PermissionMode, ViewerPermissions,
};
use crate::{ObjectPart, parcels::Parcel};

const ALLOW_REZ: u64 = 1 << 25;
const CREATE_OBJECTS: u32 = 1 << 6;
const CREATE_GROUP_OBJECTS: u32 = 1 << 26;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresenceState {
    Active,
    Deleted,
    Missing,
}

/// The addressed part and the root of its linkset, resolved by the caller.
pub struct ActionTarget<'a> {
    pub root: &'a ObjectPart,
    pub part: &'a ObjectPart,
    pub masks: LinksetPermissions,
    pub attachment: bool,
    pub deleted: bool,
}

/// Link, delink and the ById variants take their privileges from account and group
/// services. Client operations take the current presence's god and membership state.
#[derive(Clone, Copy)]
pub enum ObjectAction<'a> {
    Edit,
    Move,
    Delete,
    Take,
    TakeCopy,
    EditById,
    DeleteById,
    Link,
    Delink,
    /// Parcel at the source object's position (CanDuplicateObject).
    Duplicate(Option<&'a Parcel>),
}

/// Inputs resolved for one actor. Group service powers may differ from the client's
/// membership cache. The friendship lookup takes (grantee, owner), including the
/// object UUID in the three Pin8 TB96 paths.
pub struct ActionPermissions<'a> {
    pub agent: Uuid,
    pub is_god: bool,
    pub mode: PermissionMode,
    pub membership: Option<(Uuid, u64)>,
    pub presence: PresenceState,
    pub administrator: bool,
    /// The permissions console's forced result in bypass mode (initially true).
    pub bypass_value: bool,
    pub service_group_powers: &'a dyn Fn(Uuid) -> Option<u64>,
    pub friend_grant: &'a dyn Fn(Uuid, Uuid) -> bool,
}

impl ActionPermissions<'_> {
    /// Rez at the requested position. The land channel can return no parcel.
    pub fn can_rez(&self, parcel: Option<&Parcel>) -> bool {
        match self.mode {
            PermissionMode::NoModule => return true,
            PermissionMode::Bypass => return self.bypass_value,
            PermissionMode::Enforce => {}
        }
        let Some(parcel) = parcel else { return false };
        if parcel.owner == self.agent || parcel.flags & CREATE_OBJECTS != 0 || self.administrator {
            return true;
        }
        if parcel.group.is_nil() {
            return false;
        }
        if parcel.flags & CREATE_GROUP_OBJECTS != 0 {
            return (self.service_group_powers)(parcel.group).is_some();
        }
        parcel.group_owned
            && (self.service_group_powers)(parcel.group).is_some_and(|p| p & ALLOW_REZ != 0)
    }

    /// Selection uses the addressed part's viewer flags, including when no
    /// permission handler is installed.
    pub fn can_select(&self, target: Option<&ActionTarget<'_>>) -> bool {
        target.is_some_and(|obj| {
            (self.mode == PermissionMode::NoModule || self.presence != PresenceState::Missing)
                && PartPermissions::new(obj.part, obj.masks, obj.attachment).selectable(
                    ViewerPermissions {
                        agent: self.agent,
                        is_god: self.is_god,
                        mode: self.mode,
                        membership: self.membership,
                        friend_grant_from: self
                            .has_friend_grant(self.agent, obj.part.owner_id)
                            .then_some(obj.part.owner_id),
                    },
                )
        })
    }

    /// An unresolved object is passed as None. The scene's null/deleted guards
    /// run at different points for UUID and live-client calls.
    pub fn allows(&self, action: ObjectAction<'_>, target: Option<&ActionTarget<'_>>) -> bool {
        use ObjectAction::*;
        if self.mode == PermissionMode::NoModule {
            return true;
        }
        let by_id = matches!(action, EditById | DeleteById | Link | Delink);
        let bypass = self.mode == PermissionMode::Bypass;
        if by_id && bypass {
            return self.bypass_value;
        }
        let Some(obj) = target else { return false };
        if !by_id {
            if self.presence == PresenceState::Missing {
                return false;
            }
            if obj.deleted || self.presence == PresenceState::Deleted {
                return bypass && self.bypass_value && !matches!(action, Move | Duplicate(_));
            }
        }
        let own = obj.root.owner_id == self.agent;
        if bypass {
            return self.bypass_value && !(matches!(action, Move) && obj.attachment && !own);
        }
        let privileged = if by_id {
            self.administrator
        } else {
            self.is_god
        };
        let mask = || self.object_mask(obj, by_id, privileged, !matches!(action, Duplicate(_)));
        match action {
            Delete | DeleteById | Take => {
                if obj.attachment {
                    return false;
                }
                if own || privileged {
                    return true;
                }
                if matches!(action, Take) && obj.masks.effective_owner & PERM_TRANSFER == 0 {
                    return false;
                }
                // TB96: the UUID overload asks about the addressed part. The two
                // presence overloads ask about the linkset root, even on a child.
                let grantee = if by_id { obj.part.uuid } else { obj.root.uuid };
                self.has_friend_grant(grantee, obj.root.owner_id)
                    || self
                        .member_powers(obj.root.group_id, by_id)
                        .is_some_and(|p| {
                            (obj.root.group_id == obj.root.owner_id && p & OBJECT_MANIPULATE != 0)
                                || obj.masks.group & PERM_MODIFY != 0
                        })
            }
            Move => mask() & PERM_MOVE != 0,
            TakeCopy => (!obj.attachment || privileged) && can_copy(mask(), own),
            Duplicate(parcel) => can_copy(mask(), own) && self.can_rez(parcel),
            Edit | EditById | Link | Delink => mask() & PERM_MODIFY != 0,
        }
    }

    fn has_friend_grant(&self, grantee: Uuid, owner: Uuid) -> bool {
        !grantee.is_nil() && (self.friend_grant)(grantee, owner)
    }

    fn member_powers(&self, group: Uuid, by_id: bool) -> Option<u64> {
        if group.is_nil() {
            None
        } else if by_id {
            (self.service_group_powers)(group)
        } else {
            self.membership
                .filter(|(id, _)| *id == group)
                .map(|(_, powers)| powers)
        }
    }

    fn object_mask(
        &self,
        obj: &ActionTarget<'_>,
        by_id: bool,
        privileged: bool,
        honor_lock: bool,
    ) -> u32 {
        let own = obj.root.owner_id == self.agent;
        let mut mask = if privileged {
            PERM_ALL_EFFECTIVE
        } else if own {
            obj.masks.effective_owner
        } else if obj.attachment {
            0
        } else {
            let deeded = obj.root.owner_id == obj.root.group_id;
            let owner = obj.masks.effective_owner;
            let transferable_owner = if owner & PERM_TRANSFER == 0 {
                owner & !PERM_COPY
            } else {
                owner
            };
            if !deeded && self.has_friend_grant(self.agent, obj.root.owner_id) {
                transferable_owner
            } else {
                match self.member_powers(obj.root.group_id, by_id) {
                    Some(p) if deeded && p & OBJECT_MANIPULATE != 0 => transferable_owner,
                    Some(_) => obj.masks.group_or_everyone,
                    None => obj.masks.everyone,
                }
            }
        };
        if honor_lock && !obj.masks.unlocked && (!privileged || own) {
            mask &= !(PERM_MODIFY | PERM_MOVE);
        }
        mask
    }
}

fn can_copy(mask: u32, own: bool) -> bool {
    mask & PERM_COPY != 0 && (own || mask & PERM_TRANSFER != 0)
}
