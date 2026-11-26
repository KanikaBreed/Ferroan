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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//   (the constructor's owner options, ModifySceneObject, ResolveUserUuid,
//   ResolveGroupUuid and ResolveUserAliasOrDefault)
//   OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The creator, owner and group rewrite an OAR load applies to each object.
//!
//! [`OwnerResolver::modify_scene_object`] is `ModifySceneObject`. For every
//! part, in the group's part order, and every task-inventory item, in stored
//! order:
//!
//! - A creator with no creator data is kept when the user account service has
//!   that account. Otherwise the alias lookup or the default user replaces it.
//!   A group id is not a creator. A creator with creator data is kept as it is.
//! - Every creator, kept or replaced, is handed to the user management module
//!   with its creator data (`AddCreatorUser`).
//! - The owner, and the part's last owner, are kept when they are an account
//!   or a group record. Otherwise the alias lookup or the default user
//!   replaces them. An item's last owner is not touched.
//! - A group id with no group record becomes the zero id.
//!
//! The alias lookup runs only with `lookup-aliases`. An alias whose user is
//! an account replaces the id. Without a usable alias the default user is
//! used, unless `no-defaultuser` is set; then the id stays as it was. The
//! default user is the `default-user` option, or the estate owner.
//!
//! The zero id is never an account or a group, so it is never looked up as
//! one. Every answer, including an absent alias, is remembered for the whole
//! load, so each id is asked about once. A cache that reaches
//! [`MAX_OWNER_LOOKUPS`] stops remembering new ids and reports
//! `oar.owner-lookups`. Those ids are asked about again each time they
//! appear, so their answers match Pin8's only while the services answer
//! consistently. The absent-alias warning is still written once per id while
//! a second set of the same size has room.
//!
//! This is the console path, `ArchiveReadRequest(Scene, string, …)`. The
//! stream constructor does not read `default-user` and does not seed the
//! zero group, so there the zero group id is asked about once.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use ferroan_diag::{Kind, OAR_OWNER_LOOKUPS, drop_bound};
use uuid::Uuid;

use crate::objects::{InventoryItem, ObjectGroup, ObjectPart};

/// Entries each of the three lookup caches keeps for one load. The pin has no bound.
pub const MAX_OWNER_LOOKUPS: usize = 65_536;

/// The region services the rewrite asks. Each answer is the C# call's result
/// reduced to what `ArchiveReadRequest` reads from it. A failed call answers
/// as the C# connector does: no account, no group record, no alias.
pub trait OwnerLookups {
    /// `UserAccountService.GetUserAccount(RegionInfo.ScopeID, id)` is not null.
    fn user_exists(&mut self, id: Uuid) -> impl Future<Output = bool> + Send;
    /// `IGroupsModule.GetGroupRecord(id)` is not null. With no groups module
    /// this is `false`.
    fn group_exists(&mut self, id: Uuid) -> impl Future<Output = bool> + Send;
    /// `IUserAliasService.GetUserForAlias(id).UserID`. With no alias service,
    /// or no alias row, this is `None`.
    fn alias_user(&mut self, alias: Uuid) -> impl Future<Output = Option<Uuid>> + Send;
    /// `IUserManagement.AddCreatorUser(id, creator_data)`. With no user
    /// management module this does nothing.
    fn add_creator_user(&mut self, id: Uuid, creator_data: &str);
}

/// The three load options `ModifySceneObject` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerOptions {
    /// `default-user`, or the estate owner when that option is absent.
    pub default_user: Uuid,
    /// `no-defaultuser`: an unresolved id stays as it was.
    pub no_default_user: bool,
    /// `lookup-aliases`: an unresolved id is looked up as a user alias.
    pub lookup_aliases: bool,
}

impl OwnerOptions {
    /// The constructor's choice of default user: the `default-user` option
    /// when given, otherwise the estate owner.
    pub fn new(default_user: Option<Uuid>, estate_owner: Uuid) -> Self {
        Self {
            default_user: default_user.unwrap_or(estate_owner),
            no_default_user: false,
            lookup_aliases: false,
        }
    }
}

/// One load's lookups and their caches (`m_validUserUuids`,
/// `m_validGroupUuids` and `m_userAliases`).
pub struct OwnerResolver<L> {
    lookups: L,
    options: OwnerOptions,
    users: HashMap<Uuid, bool>,
    groups: HashMap<Uuid, bool>,
    aliases: HashMap<Uuid, Option<Uuid>>,
    /// Absent aliases already warned about that the full alias cache could not keep.
    warned: HashSet<Uuid>,
    cache_limit: usize,
}

impl<L: OwnerLookups> OwnerResolver<L> {
    pub fn new(lookups: L, options: OwnerOptions) -> Self {
        Self::with_cache_limit(lookups, options, MAX_OWNER_LOOKUPS)
    }

    fn with_cache_limit(lookups: L, options: OwnerOptions, cache_limit: usize) -> Self {
        // Zero can never be a valid user id (or group).
        let users = HashMap::from([(Uuid::nil(), false)]);
        let groups = HashMap::from([(Uuid::nil(), false)]);
        Self {
            lookups,
            options,
            users,
            groups,
            aliases: HashMap::new(),
            warned: HashSet::new(),
            cache_limit,
        }
    }

    pub fn lookups(&self) -> &L {
        &self.lookups
    }

    pub fn into_lookups(self) -> L {
        self.lookups
    }

    /// `ModifySceneObject`: rewrite the creators, owners and groups of every
    /// part and every task-inventory item.
    pub async fn modify_scene_object(&mut self, group: &mut ObjectGroup) {
        for part in &mut group.parts {
            self.modify_part(part).await;
        }
    }

    async fn modify_part(&mut self, part: &mut ObjectPart) {
        let (mut creator, data) = split_creator(
            part.xml_creator_data.as_deref(),
            &part.creator_id,
            crate::xml::part_creator,
        );
        if data.is_empty() {
            // Creators can't be groups.
            creator = self.resolve_user_alias_or_default(creator, false).await;
            part.creator_id = creator.to_string();
        }
        self.lookups.add_creator_user(creator, &data);
        part.owner_id = self
            .resolve_user_alias_or_default(part.owner_id, true)
            .await;
        part.last_owner_id = self
            .resolve_user_alias_or_default(part.last_owner_id, true)
            .await;
        if !self.resolve_group(part.group_id).await {
            part.group_id = Uuid::nil();
        }
        for item in &mut part.items {
            self.modify_item(item).await;
        }
    }

    async fn modify_item(&mut self, item: &mut InventoryItem) {
        let (mut creator, data) = split_creator(
            item.xml_creator_data.as_deref(),
            &item.creator_id,
            crate::xml::item_creator,
        );
        if data.is_empty() {
            // Creator can't be a group.
            creator = self.resolve_user_alias_or_default(creator, false).await;
            item.creator_id = creator.to_string();
        }
        self.lookups.add_creator_user(creator, &data);
        item.owner_id = self
            .resolve_user_alias_or_default(item.owner_id, true)
            .await;
        if !self.resolve_group(item.group_id).await {
            item.group_id = Uuid::nil();
        }
    }

    /// `ResolveUserUuid`: the account exists. Asked once per id.
    async fn resolve_user(&mut self, id: Uuid) -> bool {
        if let Some(known) = self.users.get(&id) {
            return *known;
        }
        let exists = self.lookups.user_exists(id).await;
        remember(&mut self.users, id, exists, self.cache_limit);
        exists
    }

    /// `ResolveGroupUuid`: the group record exists. Asked once per id.
    async fn resolve_group(&mut self, id: Uuid) -> bool {
        if let Some(known) = self.groups.get(&id) {
            return *known;
        }
        let exists = self.lookups.group_exists(id).await;
        remember(&mut self.groups, id, exists, self.cache_limit);
        exists
    }

    /// `ResolveUserAliasOrDefault`.
    async fn resolve_user_alias_or_default(&mut self, id: Uuid, check_group: bool) -> Uuid {
        if self.resolve_user(id).await {
            return id;
        }
        if check_group && self.resolve_group(id).await {
            return id;
        }
        if self.options.lookup_aliases {
            let alias = match self.aliases.get(&id) {
                Some(alias) => *alias,
                None => {
                    let alias = self.lookups.alias_user(id).await;
                    let kept = remember(&mut self.aliases, id, alias, self.cache_limit);
                    // Pin8 warns the first time an id has no alias. An id the
                    // full cache did not keep is warned about once while
                    // `warned` has room.
                    if alias.is_none()
                        && (kept
                            || (self.warned.len() < self.cache_limit && self.warned.insert(id)))
                    {
                        tracing::warn!(
                            "[ARCHIVEREADREQUEST] No alias found for User {id} and not a local user"
                        );
                    }
                    alias
                }
            };
            if let Some(user) = alias
                && self.resolve_user(user).await
            {
                return user;
            }
        }
        if self.options.no_default_user {
            id
        } else {
            self.options.default_user
        }
    }
}

/// Keep the answer while the cache has room. False when it was full.
fn remember<V>(cache: &mut HashMap<Uuid, V>, id: Uuid, value: V, limit: usize) -> bool {
    if cache.len() < limit {
        cache.insert(id, value);
        true
    } else {
        let observed = u64::try_from(cache.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let configured = u64::try_from(limit).unwrap_or(u64::MAX);
        drop_bound(
            OAR_OWNER_LOOKUPS,
            Kind::Soft,
            configured,
            observed,
            "oar-load",
        );
        false
    }
}

/// `CreatorID` and `CreatorData` as `ModifySceneObject` reads them. An
/// object-XML read keeps the element text as the data. A store load splits
/// the identification as `CreatorIdentification`'s setter does.
fn split_creator(
    xml: Option<&str>,
    identification: &str,
    split: fn(&str) -> (Uuid, Option<String>),
) -> (Uuid, String) {
    let (id, stored) = split(identification);
    let data = match xml {
        Some(text) => text.to_owned(),
        None => stored.unwrap_or_default(),
    };
    (id, data)
}

#[cfg(test)]
#[path = "oar_owners_tests.rs"]
mod tests;
