/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 *
 * Copyright (c) 2006-2016, openmetaverse.co
 * All rights reserved.
 *
 * - Redistribution and use in source and binary forms, with or without
 *   modification, are permitted provided that the following conditions are met:
 *
 * - Redistributions of source code must retain the above copyright notice, this
 *   list of conditions and the following disclaimer.
 * - Neither the name of the openmetaverse.co nor the names
 *   of its contributors may be used to endorse or promote products derived from
 *   this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */

// Ported to Rust for Ferroan from
//   Source/OpenSim.Framework/Util.cs (PermissionMask)
//   Source/OpenSim.Framework/PermissionsUtil.cs (ApplyFoldedPermissions,
//   ApplyNoModFoldedPermissions, FixAndFoldPermissions)
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
//   OpenMetaverse/Permissions.cs (the shared Transfer/Modify/Copy/Export/Move/
//   Damage/All bits)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Object permission masks.
//!
//! The shared bits match OpenMetaverse. Tranquillity adds the folded low bits,
//! `Slam`, and the named combinations. `All` does not include `Export`.
//! The five masks on an object update are little-endian, in the order
//! base, owner, group, everyone, next owner.

use crate::wire::WireError;

/// Five little-endian masks.
pub const PERMISSIONS_LEN: usize = 20;

pub const PERM_NONE: u32 = 0;
pub const PERM_FOLDED_TRANSFER: u32 = 1;
pub const PERM_FOLDED_MODIFY: u32 = 1 << 1;
pub const PERM_FOLDED_COPY: u32 = 1 << 2;
pub const PERM_FOLDED_EXPORT: u32 = 1 << 3;
/// Deprecated. Tranquillity still recognises it on existing objects.
pub const PERM_SLAM: u32 = 1 << 4;
pub const PERM_FOLDED_MASK: u32 = 0x0f;
pub const PERM_FOLDING_SHIFT: u32 = 13;
pub const PERM_TRANSFER: u32 = 1 << 13;
pub const PERM_MODIFY: u32 = 1 << 14;
pub const PERM_COPY: u32 = 1 << 15;
pub const PERM_EXPORT: u32 = 1 << 16;
pub const PERM_MOVE: u32 = 1 << 19;
pub const PERM_DAMAGE: u32 = 1 << 20;
pub const PERM_ALL: u32 = 0x8e000;
pub const PERM_ALL_AND_EXPORT: u32 = 0x9e000;
pub const PERM_ALL_AND_EXPORT_NO_MOD: u32 = 0x9a000;
pub const PERM_ALL_EFFECTIVE: u32 = 0x9e000;
pub const PERM_UNFOLDED_MASK: u32 = 0x1e000;

/// The five masks in packet order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub base: u32,
    pub owner: u32,
    pub group: u32,
    pub everyone: u32,
    pub next_owner: u32,
}

impl Permissions {
    pub fn to_bytes(self) -> [u8; PERMISSIONS_LEN] {
        let mut out = [0u8; PERMISSIONS_LEN];
        out[0..4].copy_from_slice(&self.base.to_le_bytes());
        out[4..8].copy_from_slice(&self.owner.to_le_bytes());
        out[8..12].copy_from_slice(&self.group.to_le_bytes());
        out[12..16].copy_from_slice(&self.everyone.to_le_bytes());
        out[16..20].copy_from_slice(&self.next_owner.to_le_bytes());
        out
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, WireError> {
        if data.len() != PERMISSIONS_LEN {
            return Err(WireError::Short {
                need: PERMISSIONS_LEN,
                got: data.len(),
            });
        }
        Ok(Self {
            base: u32::from_le_bytes(data[0..4].try_into().expect("4 bytes")),
            owner: u32::from_le_bytes(data[4..8].try_into().expect("4 bytes")),
            group: u32::from_le_bytes(data[8..12].try_into().expect("4 bytes")),
            everyone: u32::from_le_bytes(data[12..16].try_into().expect("4 bytes")),
            next_owner: u32::from_le_bytes(data[16..20].try_into().expect("4 bytes")),
        })
    }
}

/// `PermissionsUtil.ApplyFoldedPermissions`. A folded mask of `0` or of every
/// folded bit is left unchanged.
pub fn apply_folded_permissions(folded_source: u32, target: u32) -> u32 {
    let folded = folded_source & PERM_FOLDED_MASK;
    if folded == 0 || folded == PERM_FOLDED_MASK {
        return target;
    }
    let folded = (folded << PERM_FOLDING_SHIFT) | !PERM_UNFOLDED_MASK;
    target & folded
}

/// `PermissionsUtil.ApplyNoModFoldedPermissions`. Modify on the target is kept.
pub fn apply_no_mod_folded_permissions(folded_source: u32, target: u32) -> u32 {
    let folded = folded_source & PERM_FOLDED_MASK;
    if folded == 0 || folded == PERM_FOLDED_MASK {
        return target;
    }
    let folded = (folded << PERM_FOLDING_SHIFT) | (!PERM_UNFOLDED_MASK | PERM_MODIFY);
    target & folded
}

/// `PermissionsUtil.FixAndFoldPermissions`.
pub fn fix_and_fold_permissions(perms: u32) -> u32 {
    let mut tmp = perms;
    if tmp & (PERM_COPY | PERM_TRANSFER) == 0 {
        tmp |= PERM_TRANSFER;
    }
    tmp |= PERM_MOVE;
    tmp &= !PERM_FOLDED_MASK;
    tmp |= (tmp >> PERM_FOLDING_SHIFT) & PERM_FOLDED_MASK;
    tmp
}
