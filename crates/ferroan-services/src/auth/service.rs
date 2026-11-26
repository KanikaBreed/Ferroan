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
//   Source/OpenSim.Services.AuthenticationService/AuthenticationServiceBase.cs
//   Source/OpenSim.Services.AuthenticationService/PasswordAuthenticationService.cs
//   Source/OpenSim.Services.AuthenticationService/WebkeyAuthenticationService.cs
//   Source/OpenSim.Services.AuthenticationService/WebkeyOrPasswordAuthenticationService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Webkey clear reviewed against KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b (D79).
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Password, web-key and web-key-or-password authentication.
//!
//! Robust's connector constructs the service with the config only, so
//! `m_UserAccountService` stays null and a wrong password returns an empty
//! token. Impersonation (user flag bit 6, level >= 200) runs only when that
//! service is injected, which this process does not do.
//!
//! Pin8 rejects malformed and zero web keys. `WebkeyOrPassword` tries the
//! password after a failed web-key check. D79 requires a successful conditional
//! clear before issuing a token for a matching key.

use uuid::Uuid;

use super::hash;
use super::model::{AuthKind, AuthRow};
use super::store::{AuthStore, StoreError};

pub(crate) struct AuthService {
    store: AuthStore,
    kind: AuthKind,
}

impl AuthService {
    pub(crate) fn new(store: AuthStore, kind: AuthKind) -> Self {
        Self { store, kind }
    }

    pub(crate) fn pool_occupancy(&self) -> (u32, u32) {
        self.store.pool_occupancy()
    }

    pub(crate) async fn set_password(
        &self,
        principal: Uuid,
        password: &str,
    ) -> Result<bool, StoreError> {
        let salt = hash::new_salt();
        let hashed = hash::stored_hash(password, &salt);
        let stored = self.store.set_password(principal, hashed, salt).await?;
        if stored {
            tracing::info!("authentication: set password for principal {principal}");
        } else {
            tracing::debug!("authentication: failed to store password for principal {principal}");
        }
        Ok(stored)
    }

    pub(crate) async fn get(&self, principal: Uuid) -> Result<Option<AuthRow>, StoreError> {
        self.store.get(principal).await
    }

    pub(crate) async fn set_info(&self, row: &AuthRow) -> Result<bool, StoreError> {
        let stored = self.store.store(row).await?;
        if !stored {
            tracing::error!(
                "authentication: failed to store authentication info for principal {}",
                row.principal_id
            );
        }
        Ok(stored)
    }

    /// Empty string is a failed authentication. A token is a hyphenated UUID.
    pub(crate) async fn authenticate(
        &self,
        principal: Uuid,
        password: &str,
        lifetime: i32,
    ) -> Result<String, StoreError> {
        match self.kind {
            AuthKind::Password => {
                self.authenticate_password(principal, password, lifetime)
                    .await
            }
            AuthKind::Webkey => {
                self.authenticate_webkey(principal, password, lifetime)
                    .await
            }
            AuthKind::WebkeyOrPassword => {
                let Some(row) = self.store.get(principal).await? else {
                    tracing::debug!("authentication: principal {principal} or its data not found");
                    return Ok(String::new());
                };
                let mut token = String::new();
                if !row.web_login_key.is_empty() {
                    token = self
                        .authenticate_webkey(principal, password, lifetime)
                        .await?;
                }
                if token.is_empty() {
                    token = self
                        .authenticate_password(principal, password, lifetime)
                        .await?;
                }
                Ok(token)
            }
        }
    }

    pub(crate) async fn verify(
        &self,
        principal: Uuid,
        token: &str,
        lifetime: i32,
    ) -> Result<bool, StoreError> {
        self.store.check_token(principal, token, lifetime).await
    }

    pub(crate) async fn release(&self, principal: Uuid, token: &str) -> Result<bool, StoreError> {
        self.store.check_token(principal, token, 0).await
    }

    async fn authenticate_password(
        &self,
        principal: Uuid,
        password: &str,
        lifetime: i32,
    ) -> Result<String, StoreError> {
        let Some(row) = self.store.get(principal).await? else {
            tracing::debug!("authentication: principal {principal} or its data not found");
            return Ok(String::new());
        };
        let hashed = hash::presented_hash(password, &row.password_salt);
        if hashed == row.password_hash {
            return self.issue(principal, lifetime).await;
        }
        tracing::debug!("authentication: no user record for {principal}");
        Ok(String::new())
    }

    async fn authenticate_webkey(
        &self,
        principal: Uuid,
        password: &str,
        lifetime: i32,
    ) -> Result<String, StoreError> {
        match classify_webkey(password) {
            Presented::Zero => {
                tracing::debug!(
                    "authentication: UUID.Zero is not a valid web_login_key on principal {principal}"
                );
                Ok(String::new())
            }
            Presented::Invalid => {
                tracing::debug!(
                    "authentication: web login key for principal {principal} is not a UUID"
                );
                Ok(String::new())
            }
            Presented::Key => {
                let Some(row) = self.store.get(principal).await? else {
                    tracing::debug!("authentication: principal {principal} or its data not found");
                    return Ok(String::new());
                };
                if row.web_login_key == password {
                    // D79: a web login key must be consumed before issuing its token.
                    if !self
                        .store
                        .consume_web_key(principal, &row.web_login_key)
                        .await?
                    {
                        return Ok(String::new());
                    }
                    self.issue(principal, lifetime).await
                } else {
                    tracing::debug!(
                        "authentication: web login auth failed for principal {principal}"
                    );
                    Ok(String::new())
                }
            }
        }
    }

    async fn issue(&self, principal: Uuid, lifetime: i32) -> Result<String, StoreError> {
        let token = Uuid::new_v4().to_string();
        if self.store.set_token(principal, &token, lifetime).await? {
            Ok(token)
        } else {
            Ok(String::new())
        }
    }
}

enum Presented {
    Zero,
    Invalid,
    Key,
}

/// Pin8 rejects malformed and zero UUIDs before looking up the key.
fn classify_webkey(password: &str) -> Presented {
    if password.len() < 32 {
        return Presented::Invalid;
    }
    match Uuid::parse_str(password) {
        Ok(id) if id.is_nil() => Presented::Zero,
        Ok(_) => Presented::Key,
        Err(_) => Presented::Invalid,
    }
}
