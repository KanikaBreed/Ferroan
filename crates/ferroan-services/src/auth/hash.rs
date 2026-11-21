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
//   Source/OpenSim.Framework/Util.cs Md5Hash
//   Source/OpenSim.Services.AuthenticationService/AuthenticationServiceBase.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Salted password hashes as `AuthenticationServiceBase.SetPassword` stores them.
//!
//! `Util.Md5Hash` is the lowercase hex of MD5 over `Encoding.Default` bytes.
//! On the pin's Linux runtime that encoding is UTF-8. The `$1$` prefix is the
//! viewer's "already MD5" marker; login strips it before `Authenticate`.

use md5::{Digest, Md5};
use uuid::Uuid;

pub(crate) fn md5_text(text: &str) -> String {
    format!("{:x}", Md5::digest(text.as_bytes()))
}

/// `Util.Md5Hash(UUID.Random().ToString())`.
pub(crate) fn new_salt() -> String {
    md5_text(&Uuid::new_v4().to_string())
}

/// `Util.Md5Hash(Util.Md5Hash(password) + ":" + salt)`.
pub(crate) fn stored_hash(password: &str, salt: &str) -> String {
    let inner = md5_text(password);
    md5_text(&format!("{inner}:{salt}"))
}

/// `Util.Md5Hash(presented + ":" + salt)`. `presented` is the login MD5.
pub(crate) fn presented_hash(presented: &str, salt: &str) -> String {
    md5_text(&format!("{presented}:{salt}"))
}

/// `UUID.Zero.ToString()`.
pub(crate) const ZERO_KEY: &str = "00000000-0000-0000-0000-000000000000";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_matches_the_pin_runtime() {
        assert_eq!(md5_text("secret"), "5ebe2294ecd0e0f08eab7690d2a6ee69");
        let salt = "0123456789abcdef0123456789abcdef";
        let stored = stored_hash("harness-secret", salt);
        let presented = md5_text("harness-secret");
        assert_eq!(presented_hash(&presented, salt), stored);
        assert_eq!(new_salt().len(), 32);
    }
}
