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
// Ported from Source/OpenSim.Framework/AvatarAppearance.cs (SetDefaultParams)
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

/// `AvatarAppearance.SetDefaultParams`. The template count is one byte, and this list fits.
pub const DEFAULT_VISUAL_PARAMS: &[u8] = &[
    33, 61, 85, 23, 58, 127, 63, 85, 63, 42, 0, 85, 63, 36, 85, 95, 153, 63, 34, 0, 63, 109, 88,
    132, 63, 136, 81, 85, 103, 136, 127, 0, 150, 150, 150, 127, 0, 0, 0, 0, 0, 127, 0, 0, 255, 127,
    114, 127, 99, 63, 127, 140, 127, 127, 0, 0, 0, 191, 0, 104, 0, 0, 0, 0, 0, 0, 0, 0, 0, 145,
    216, 133, 0, 127, 0, 127, 170, 0, 0, 127, 127, 109, 85, 127, 127, 63, 85, 42, 150, 150, 150,
    150, 150, 150, 150, 25, 150, 150, 150, 0, 127, 0, 0, 144, 85, 127, 132, 127, 85, 0, 127, 127,
    127, 127, 127, 127, 59, 127, 85, 127, 127, 106, 47, 79, 127, 127, 204, 2, 141, 66, 0, 0, 127,
    127, 0, 0, 0, 0, 127, 0, 159, 0, 0, 178, 127, 36, 85, 131, 127, 127, 127, 153, 95, 0, 140, 75,
    27, 127, 127, 0, 150, 150, 198, 0, 0, 63, 30, 127, 165, 209, 198, 127, 127, 153, 204, 51, 51,
    255, 255, 255, 204, 0, 255, 150, 150, 150, 150, 150, 150, 150, 150, 150, 150, 0, 150, 150, 150,
    150, 150, 0, 127, 127, 150, 150, 150, 150, 150, 150, 150, 150, 0, 0, 150, 51, 132, 150, 150,
    150,
];
