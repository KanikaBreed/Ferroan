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
//   Source/OpenSim.Data/IAvatarData.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One `Avatars` row. The C# service stores `Name` and `Value` in the generic
//! handler's data dictionary, so each appearance key is its own row.

/// Default realm from `[AvatarService]`. PostgreSQL folds the unquoted name
/// to `avatars`.
pub const DEFAULT_REALM: &str = "Avatars";

/// One stored appearance key.
///
/// `principal_id` is the text the engine returns. A PostgreSQL read is the
/// canonical uuid text. MySQL `char(36)` drops trailing spaces. `name` and
/// `value` are not trimmed: SQLite `VARCHAR(255)` does not enforce its width,
/// and MySQL and PostgreSQL store `Value` as `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarRow {
    pub principal_id: String,
    pub name: String,
    pub value: String,
}
