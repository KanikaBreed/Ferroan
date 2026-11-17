# Notices

Copyright (c) 2026 Ferroan contributors.

Ferroan is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License, version 3 or (at your option) any later version; see [LICENSE](LICENSE). It is provided without warranty, as the licence describes.

Code ported or copied from other projects keeps the copyright and licence notices of its source, as listed below and in the header of each such file. All of those licences allow their code to be combined into a work distributed under GPL-3.0-or-later.

## Porting sources

| Source | Licence | Text | How Ferroan uses it |
| --- | --- | --- | --- |
| [OpenSim Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity): OpenSimulator code, copyright Contributors, http://opensimulator.org/ | BSD-3-Clause | [licenses/OpenSimulator-BSD-3-Clause.txt](licenses/OpenSimulator-BSD-3-Clause.txt) | Behavior reference and porting source |
| Tranquillity's own changes and new files, copyright Utopia Skye LLC, Legion Builds and others | MPL-2.0 | [licenses/MPL-2.0.txt](licenses/MPL-2.0.txt) | Porting source. MPL-2.0 allows combination into a GPL work; the ported files keep their MPL notices |
| InWorldz Halcyon (in Tranquillity) | BSD-3-Clause | [licenses/Halcyon.txt](licenses/Halcyon.txt) | Porting source |
| HalcyonGrid/phlox (in Tranquillity) | Apache-2.0 | [licenses/Phlox.txt](licenses/Phlox.txt) | Porting source for the script host |
| Luau and lsl-definitions (in Tranquillity) | MIT | [licenses/Luau.txt](licenses/Luau.txt), [licenses/lsl-definitions.txt](licenses/lsl-definitions.txt) | Porting source |
| [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse), the OpenMetaverse library Tranquillity uses, copyright openmetaverse.co and contributors | BSD-3-Clause | added with the first port | Porting source for types, packets and LLSD |
| [Aurora Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer), copyright the Aurora Viewer contributors | GPL-3.0-or-later | [LICENSE](LICENSE) | Rust crates reused or adapted (LLSD, message template code generation, prim volumes); viewer protocol reference |
| Second Life viewer and Firestorm, copyright Linden Research, Inc. and The Phoenix Firestorm Project, Inc. | LGPL-2.1, used here under GPL-3.0-or-later as LGPL-2.1 section 3 permits | added with the first port | The message template and protocol behavior |

The reference revisions used for porting are listed in [docs/REFERENCES.md](docs/REFERENCES.md). Each ported file names its source files and revision ([PORTING.md](docs/PORTING.md)).

No code has been ported yet. The table lists the sources the plan draws on, so their terms are settled before the first port.

## Native libraries (planned)

- [Jolt Physics](https://github.com/jrouwe/JoltPhysics), MIT, through `oxijolt-sys` (MIT OR Apache-2.0).
- Linden Lab's [SLua engine](https://github.com/secondlife/slua), MIT (copyright Linden Lab), a fork of Luau (MIT, copyright Roblox).

Their licence texts will be added when they become dependencies.

## Not used as sources

Benthic's metaverse crates (AGPL-3.0-or-later) may be read for comparison but are not copied or depended on.

## Names

OpenSimulator, Tranquillity, Aurora Viewer, Second Life, SLua and Linden Lab names identify sources and compatibility targets. Ferroan is independent and does not claim their endorsement. Second Life is a trademark of Linden Research, Inc.
