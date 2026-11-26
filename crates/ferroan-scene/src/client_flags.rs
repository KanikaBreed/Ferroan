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
// Ported from PermissionsModule.cs, Scene.Permissions.cs, SceneObjectGroup.Inventory.cs,
// SceneObjectPart.cs, SceneObjectPartInventory.cs and GodController.cs at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b. ObjectManipulate follows GroupManager.cs
// at libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Per-recipient permission bits for a prim update.

use ferroan_types::{PERM_ALL_EFFECTIVE, PERM_COPY, PERM_MODIFY, PERM_MOVE, PERM_TRANSFER};
use uuid::Uuid;

use crate::ObjectPart;

const MODIFY: u32 = 0x4;
const COPY: u32 = 0x8;
const ANY_OWNER: u32 = 0x10;
const YOU_OWNER: u32 = 0x20;
const MOVE: u32 = 0x100;
const TRANSFER: u32 = 0x2_0000;
const GROUP_OWNED: u32 = 0x4_0000;
const OWNER_MODIFY: u32 = 0x1000_0000;
const DEFAULT_FLAGS: u32 = MODIFY | COPY | MOVE | TRANSFER | YOU_OWNER | ANY_OWNER | OWNER_MODIFY;
pub(crate) const OBJECT_MANIPULATE: u64 = 1 << 38;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PermissionMode {
    #[default]
    Enforce,
    Bypass,
    NoModule,
}

/// Resolved presence and service data for the recipient of this part. A membership
/// with zero powers still grants the group's shared permissions.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewerPermissions {
    pub agent: Uuid,
    pub is_god: bool,
    pub mode: PermissionMode,
    pub friend_grant_from: Option<Uuid>,
    /// The part's group and the recipient's powers in it.
    pub membership: Option<(Uuid, u64)>,
}

impl From<Uuid> for ViewerPermissions {
    fn from(agent: Uuid) -> Self {
        Self {
            agent,
            ..Self::default()
        }
    }
}

/// Effective linkset permissions. Inventory in any linked part can narrow the masks.
#[derive(Clone, Copy, Debug)]
pub struct LinksetPermissions {
    pub(crate) root: Uuid,
    pub(crate) unlocked: bool,
    pub(crate) effective_owner: u32,
    pub(crate) group: u32,
    pub(crate) group_or_everyone: u32,
    pub(crate) everyone: u32,
}

impl LinksetPermissions {
    pub fn from_parts(root: &ObjectPart, children: &[ObjectPart]) -> Self {
        let mut owner = root.owner_mask;
        let mut group = root.group_mask;
        let mut everyone = root.everyone_mask;
        let modern = root.creation_date == 0 || root.creation_date > 1_493_574_994;
        for item in std::iter::once(root).chain(children).flat_map(|p| &p.items) {
            if item.inv_type != 3 {
                // InventoryType.Landmark
                owner &= item.current_permissions;
                group &= item.group_permissions;
                if modern {
                    everyone &= item.everyone_permissions;
                }
            }
        }
        owner |= root.owner_mask & (PERM_MOVE | PERM_MODIFY);
        if owner & (PERM_COPY | PERM_TRANSFER) == 0 {
            owner |= PERM_TRANSFER;
        }
        owner &= (root.base_mask & PERM_ALL_EFFECTIVE) | PERM_MOVE;
        group |= root.group_mask & (PERM_MOVE | PERM_MODIFY);
        everyone |= root.everyone_mask & PERM_MOVE;
        everyone &= !PERM_MODIFY;
        if root.base_mask & PERM_TRANSFER == 0 {
            group &= !PERM_COPY;
            everyone &= !PERM_COPY;
        } else {
            group |= owner & PERM_TRANSFER;
            everyone |= owner & PERM_TRANSFER;
        }
        Self {
            root: root.uuid,
            unlocked: root.owner_mask & PERM_MOVE != 0,
            effective_owner: owner,
            group: group & owner,
            group_or_everyone: (group | everyone) & owner,
            everyone: everyone & owner,
        }
    }
}

/// Permission inputs retained with the common update block.
#[derive(Clone, Copy, Debug)]
pub struct PartPermissions {
    owner: Uuid,
    group: Uuid,
    owner_mask: u32,
    attachment: bool,
    linkset: LinksetPermissions,
}

impl PartPermissions {
    pub fn new(part: &ObjectPart, linkset: LinksetPermissions, attachment: bool) -> Self {
        Self {
            owner: part.owner_id,
            group: part.group_id,
            owner_mask: part.owner_mask,
            attachment,
            linkset,
        }
    }

    /// Scene.Permissions seeds all default bits and intersects the handler's result
    /// with its input. In particular, a handler alone cannot introduce GroupOwned (TB95).
    pub fn viewer_flags(&self, effective_object_flags: u32, viewer: ViewerPermissions) -> u32 {
        let input = effective_object_flags | DEFAULT_FLAGS;
        if viewer.mode == PermissionMode::NoModule {
            return input;
        }
        input & self.module_flags(input, viewer)
    }

    pub(crate) fn selectable(&self, viewer: ViewerPermissions) -> bool {
        // These three bits are all in the dispatcher's initial DEFAULT_FLAGS.
        self.viewer_flags(0, viewer) & (MOVE | MODIFY | OWNER_MODIFY) != 0
    }

    fn module_flags(&self, input: u32, viewer: ViewerPermissions) -> u32 {
        if input == 0 {
            return 0;
        }
        let flags = input & !DEFAULT_FLAGS;
        let unlocked = self.linkset.unlocked;
        let own = viewer.agent == self.owner;
        if viewer.is_god {
            return flags
                | if own && !unlocked {
                    COPY | TRANSFER | YOU_OWNER | ANY_OWNER
                } else {
                    DEFAULT_FLAGS
                };
        }
        if viewer.mode == PermissionMode::Bypass {
            return flags | owner_flags(self.owner_mask, true);
        }
        let mut owner = self.linkset.effective_owner;
        if own {
            return flags | owner_flags(owner, unlocked);
        }

        let any_owner = if self.owner.is_nil() { 0 } else { ANY_OWNER };
        if !self.attachment {
            if owner & PERM_TRANSFER == 0 {
                owner &= !PERM_COPY;
            }
            let deeded = self.owner == self.group;
            if !deeded && viewer.friend_grant_from == Some(self.owner) {
                return flags | owner_flags(owner, unlocked);
            }
            if let Some((id, powers)) = viewer.membership
                && !self.group.is_nil()
                && self.group == id
            {
                if deeded && powers & OBJECT_MANIPULATE != 0 {
                    return flags | GROUP_OWNED | owner_flags(owner, unlocked);
                }
                let shared = permission_flags(self.linkset.group_or_everyone, unlocked);
                return flags | any_owner | shared | if deeded { GROUP_OWNED } else { 0 };
            }
        }
        flags | any_owner | permission_flags(self.linkset.everyone, unlocked)
    }
}

fn permission_flags(mask: u32, unlocked: bool) -> u32 {
    let mut flags = 0;
    if mask & PERM_COPY != 0 {
        flags |= COPY;
    }
    if mask & PERM_TRANSFER != 0 {
        flags |= TRANSFER;
    }
    if unlocked {
        if mask & PERM_MODIFY != 0 {
            flags |= MODIFY;
        }
        if mask & PERM_MOVE != 0 {
            flags |= MOVE;
        }
    }
    flags
}

fn owner_flags(mask: u32, unlocked: bool) -> u32 {
    let flags = permission_flags(mask, unlocked) | YOU_OWNER | ANY_OWNER;
    if flags & MODIFY != 0 {
        flags | OWNER_MODIFY
    } else {
        flags
    }
}

#[cfg(test)]
mod tests {
    mod actions;

    use super::*;
    use crate::object_permissions::ActionTarget;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        input: Input,
        settings: Settings,
        expected: Expected,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Input {
        name: String,
        owns_object: bool,
        deeded: bool,
        member: bool,
        #[serde(default)]
        member_of_other_group: bool,
        powers: u64,
        owner_mask: u32,
        group_mask: u32,
        everyone_mask: u32,
        base_mask: u32,
        flags: u32,
        attachment: bool,
        manager: bool,
        estate_owner: bool,
        user_level: i32,
        inventory_item: Option<Item>,
        friend_can_modify: bool,
        owns_land: bool,
        group_land: bool,
        has_land: bool,
        land_flags: u32,
        #[serde(default)]
        actions: actions::Probe,
        #[serde(default)]
        nil_owner: bool,
        #[serde(default)]
        nil_group: bool,
        creation_date: Option<i32>,
        child: Option<ChildPart>,
    }

    impl Input {
        fn membership_group(&self, object_group: Uuid) -> Uuid {
            if self.member_of_other_group {
                Uuid::from_u128(6)
            } else {
                object_group
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ChildPart {
        owner_mask: u32,
        inventory_item: Option<Item>,
        observe: bool,
    }

    #[derive(Deserialize)]
    struct Item {
        current: u32,
        group: u32,
        everyone: u32,
    }

    #[derive(Deserialize)]
    struct Settings {
        permissionmodules: String,
        serverside_object_permissions: bool,
        allow_grid_gods: bool,
        force_grid_gods_only: bool,
        region_owner_is_god: bool,
        region_manager_is_god: bool,
        automatic_gods: bool,
        implicit_gods: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Expected {
        #[serde(flatten)]
        decisions: actions::Decisions,
        is_god: bool,
        effective_owner_mask: u32,
        effective_everyone_mask: u32,
        effective_group_or_everyone_mask: u32,
        client_flag_input: u32,
        module_client_flags: u32,
        generate_client_flags: u32,
    }

    // XML2 provides the same part and inventory types used by every region store.
    fn part_with_item() -> ObjectPart {
        crate::from_xml2(concat!(
            "<SceneObjectPart><UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID>",
            "<TaskInventory><TaskInventoryItem><ItemID>bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1",
            "</ItemID></TaskInventoryItem></TaskInventory></SceneObjectPart>"
        ))
        .unwrap()
        .parts
        .remove(0)
    }

    #[test]
    fn legacy_linksets_keep_everyone_copy_when_child_inventory_forbids_it() {
        let mut child = part_with_item();
        let mut root = child.clone();
        root.items.clear();
        root.owner_mask = PERM_ALL_EFFECTIVE;
        root.base_mask = PERM_ALL_EFFECTIVE;
        root.everyone_mask = PERM_COPY;
        child.items[0].inv_type = 7;
        child.items[0].current_permissions = PERM_ALL_EFFECTIVE;
        child.items[0].everyone_permissions = 0;
        let children = [child];
        for (date, keep_copy) in [(1_493_574_994, true), (1_493_574_995, false), (0, false)] {
            root.creation_date = date;
            let perms = LinksetPermissions::from_parts(&root, &children);
            assert_eq!(perms.everyone & PERM_COPY != 0, keep_copy);
        }
    }

    #[test]
    fn without_a_module_the_scene_keeps_the_seeded_word() {
        let part = part_with_item();
        let perms = PartPermissions::new(&part, LinksetPermissions::from_parts(&part, &[]), false);
        let viewer = ViewerPermissions {
            mode: PermissionMode::NoModule,
            ..Default::default()
        };
        assert_eq!(perms.viewer_flags(0x400, viewer), 0x1002_053c);
    }

    fn set_inventory(part: &mut ObjectPart, item: Option<&Item>) {
        if let Some(item) = item {
            let entry = &mut part.items[0];
            entry.inv_type = 7; // notecard in the oracle
            entry.current_permissions = item.current;
            entry.group_permissions = item.group;
            entry.everyone_permissions = item.everyone;
        } else {
            part.items.clear();
        }
    }

    fn replay_cases(mut check: impl FnMut(&Case, &ActionTarget, ViewerPermissions, bool)) {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../fixtures/permissions/objects.json"))
                .unwrap();
        assert_eq!(fixture.cases.len(), 83);
        let template = part_with_item();
        let (agent, owner, group) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        for case in &fixture.cases {
            let Case {
                input,
                settings,
                expected,
            } = case;
            let name = &input.name;
            let mut root = template.clone();
            root.group_id = if input.nil_group { Uuid::nil() } else { group };
            root.owner_id = if input.nil_owner {
                Uuid::nil()
            } else if input.owns_object {
                agent
            } else if input.deeded {
                root.group_id
            } else {
                owner
            };
            root.base_mask = input.base_mask;
            root.owner_mask = input.owner_mask;
            root.group_mask = input.group_mask;
            root.everyone_mask = input.everyone_mask;
            root.creation_date = input.creation_date.unwrap_or_default();
            set_inventory(&mut root, input.inventory_item.as_ref());
            let observe_child = input.child.as_ref().is_some_and(|linked| linked.observe);
            let child = input.child.as_ref().map(|linked| {
                let mut child = template.clone();
                child.uuid = Uuid::from_u128(4);
                child.owner_id = root.owner_id;
                child.group_id = root.group_id;
                child.owner_mask = linked.owner_mask;
                set_inventory(&mut child, linked.inventory_item.as_ref());
                child
            });
            let linkset = LinksetPermissions::from_parts(&root, child.as_slice());
            let part = if observe_child {
                child.as_ref().unwrap()
            } else {
                &root
            };
            // GodController's initial presence state; the module reads IsGod, not viewer UI state.
            let grid_god = (settings.force_grid_gods_only || settings.allow_grid_gods)
                && input.user_level >= 200;
            let estate_god = !settings.force_grid_gods_only
                && ((settings.region_owner_is_god && input.estate_owner)
                    || (settings.region_manager_is_god && (input.manager || input.estate_owner)));
            assert_eq!(settings.permissionmodules, "DefaultPermissionsModule");
            let viewer = ViewerPermissions {
                agent,
                is_god: (grid_god || estate_god)
                    && (settings.automatic_gods || settings.implicit_gods),
                mode: if settings.serverside_object_permissions {
                    PermissionMode::Enforce
                } else {
                    PermissionMode::Bypass
                },
                membership: input
                    .member
                    .then_some((input.membership_group(root.group_id), input.powers)),
                friend_grant_from: input.friend_can_modify.then_some(root.owner_id),
            };
            assert_eq!(viewer.is_god, expected.is_god, "{name}: god state");
            let target = ActionTarget {
                root: &root,
                part,
                masks: linkset,
                attachment: input.attachment,
                deleted: input.actions.deleted_object,
            };
            check(case, &target, viewer, grid_god || estate_god);
        }
    }

    #[test]
    fn every_module_and_viewer_flag_word_matches_pin8() {
        replay_cases(|case, target, viewer, _| {
            let input = &case.input;
            let expected = &case.expected;
            let name = &input.name;
            let part = target.part;
            let linkset = target.masks;
            assert_eq!(
                linkset.effective_owner, expected.effective_owner_mask,
                "{name}: owner"
            );
            assert_eq!(
                linkset.everyone, expected.effective_everyone_mask,
                "{name}: everyone"
            );
            assert_eq!(
                linkset.group_or_everyone, expected.effective_group_or_everyone_mask,
                "{name}: group or everyone"
            );
            let perms = PartPermissions::new(part, linkset, input.attachment);
            let effective = input.flags | if part.items.is_empty() { 0x800 } else { 0 };
            let dispatcher_input = effective | DEFAULT_FLAGS;
            assert_eq!(
                dispatcher_input, expected.client_flag_input,
                "{name}: dispatcher input"
            );
            assert_eq!(
                perms.module_flags(dispatcher_input, viewer),
                expected.module_client_flags,
                "{name}: module"
            );
            assert_eq!(
                perms.viewer_flags(effective, viewer),
                expected.generate_client_flags,
                "{name}: viewer"
            );

            // The store restores world objects. Use a linked part so its live Scripted
            // bit survives the load path's separate clearing of that bit on the root.
            if !input.attachment {
                let mut child = part.clone();
                child.uuid = Uuid::from_u128(4);
                child.flags = input.flags;
                let update =
                    crate::prim_update::StoredPrimUpdate::from_store(&child, &linkset, 2, 1)
                        .unwrap();
                assert_eq!(
                    update.block_for(viewer).update_flags,
                    expected.generate_client_flags,
                    "{name}: ObjectUpdate"
                );
            }
        });
    }
}
