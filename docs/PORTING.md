# Porting rules

Ferroan is GPL-3.0-or-later. A Rust file ported from C#, or copied and adapted from Aurora Viewer, is a derivative work of its source. These rules keep the licences and the trail back to each source intact ([D4](DECISIONS.md)).

## Before porting

1. Use the pinned, clean checkouts in [REFERENCES.md](REFERENCES.md), never a working checkout with local changes.
2. Read the source files involved, and the tests that cover them.
3. Check each source file's header. Its licence must be one the table below allows.

| Source licence | Allowed | What the header keeps |
| --- | --- | --- |
| BSD-3-Clause (OpenSimulator, Halcyon, OpenMetaverse) | Yes | The original copyright lines and the BSD notice |
| MPL-2.0 (Tranquillity's own files) | Yes | The original copyright lines and the MPL notice; the ported file stays available under MPL-2.0 as well |
| Apache-2.0 (Phlox) | Yes | The original notice, plus a statement that the file was changed |
| MIT (Luau, lsl-definitions, Linden's SLua) | Yes | The original copyright line and MIT notice |
| GPL-3.0-or-later (Aurora Viewer) | Yes | Aurora's notice, its source path and revision |
| LGPL-2.1 (Second Life viewer, Firestorm) | Yes, under GPL-3.0-or-later as LGPL-2.1 section 3 permits | The original copyright line and "originally LGPL 2.1" |
| AGPL (Benthic) | No | Read for comparison only |
| None or unclear | No | Ask in the task's PR |

## File headers

A file ported from C#:

```rust
// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// <the original licence notice, unchanged>
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.Framework/Scenes/SceneObjectPart.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later
```

A file copied or adapted from Aurora Viewer:

```rust
// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-llsd/src/xml.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// <and, if Aurora's file says so, "originally LGPL 2.1" with its viewer sources>
```

A file written fresh:

```rust
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
```

A file drawn from several sources lists all of them.

## Assets

Textures, sounds, fonts, avatar definitions and default content have their own licences. Check each before copying, and record it in [NOTICE.md](../NOTICE.md).

## After porting

- Record any new source origin in [NOTICE.md](../NOTICE.md).
- Note the source revisions in the PR.
- Because of the freeze ([D7](DECISIONS.md)), check the C# history since the pinned revision for bug fixes, and carry them over.
- Record a bug found in the C# in [TRANQUILLITY_BUGS.md](TRANQUILLITY_BUGS.md). A pin move that brings in its fix regenerates the affected fixtures against the fixed behavior, changes Ferroan to match, and moves the row to Fixed.

Ferroan code is GPL, so it cannot be copied back into Tranquillity's BSD/MPL tree. Fixes found while porting go to Tranquillity as separate C# changes.
