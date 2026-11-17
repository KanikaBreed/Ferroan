# Porting rules

A Rust file ported from Tranquillity is a derivative work of the C# file it came from. These rules keep the licences and the trail back to the source intact ([D4](DECISIONS.md)).

## Before porting

1. Use a clean checkout of Tranquillity's `develop` and note the commit hash.
2. Read the C# files involved, and the C# tests that cover them.
3. Check the file headers: the licence of each C# file decides the licence of its port.

## The header of a ported file

Keep the original copyright and licence notice word for word, then add where the file came from:

```rust
// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// <the original licence text or notice, unchanged>
//
// Ported to Rust for Ferroan from
//   OpenSim/Region/Framework/Scenes/SceneObjectPart.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// Ferroan changes: Copyright (c) 2026 Ferroan contributors, MPL-2.0.
```

For a C# file under MPL-2.0, the port stays under MPL-2.0 with the original copyright lines. For Apache-2.0 (Phlox), keep the notice and state that the file was changed, as that licence requires.

A Rust file drawn from several C# files lists all of them. A Rust file written fresh, without porting C#, carries only the MPL-2.0 header:

```rust
// Copyright (c) 2026 Ferroan contributors
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
```

## Licences that need care

- **LGPL:** a few Tranquillity files are LGPL. Do not port them without a recorded decision; write the feature fresh or use a crate instead.
- **Unclear licence:** if a file has no notice or an unclear one, do not port it. Ask in the task's PR.
- **Assets:** textures, sounds, fonts and default content have their own licences. Check before copying them.

## After porting

- Record any new source origin in [NOTICE.md](../NOTICE.md).
- Note the C# revision in the PR.
- Because of the freeze ([D7](DECISIONS.md)), check the C# history since that revision for bug fixes and carry them over.
