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
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptTokenize.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptObjCode.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptCodeGen.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptCompValu.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptReduce.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/XMRInstAbstract.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! YEngine token stream, source hash, object-file header and global slots.
//!
//! [`assign_global_slots`] gives an ordinary global the slot `globalVarNames`
//! records. The script thread and Linden's SLua engine are not in this crate yet.

mod error;
mod number;
mod object;
mod slots;
mod tokenize;

pub use error::Failure;
pub use object::{Global, HEADER_STRING, Header, OBJECT_BYTES, VERSION, read_header};
pub use slots::assign_global_slots;
pub use tokenize::{
    MAX_NAME_LEN, MAX_STRING_LEN, SOURCE_CHARS, Stream, Token, source_hash, tokenize,
};
