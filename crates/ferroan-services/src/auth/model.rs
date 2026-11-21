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
//   Source/OpenSim.Services.Interfaces/IAuthenticationService.cs AuthInfo
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One `auth` row. Column names are the pin's, not a shared schema cache.

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthRow {
    pub(crate) principal_id: Uuid,
    pub(crate) password_hash: String,
    pub(crate) password_salt: String,
    pub(crate) web_login_key: String,
    pub(crate) account_type: String,
}

/// Which `LocalServiceModule` the process serves.
///
/// `WebkeyOrPasswordAuthenticationService` ends with
/// `PasswordAuthenticationService`, so callers match this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthKind {
    WebkeyOrPassword,
    Webkey,
    Password,
}

impl AuthKind {
    pub(crate) fn from_module(module: &str) -> Option<Self> {
        let module = module.trim();
        if module == "WebkeyOrPasswordAuthenticationService"
            || module.ends_with(":WebkeyOrPasswordAuthenticationService")
        {
            Some(Self::WebkeyOrPassword)
        } else if module == "WebkeyAuthenticationService"
            || module.ends_with(":WebkeyAuthenticationService")
        {
            Some(Self::Webkey)
        } else if module == "PasswordAuthenticationService"
            || module.ends_with(":PasswordAuthenticationService")
        {
            Some(Self::Password)
        } else {
            None
        }
    }
}
