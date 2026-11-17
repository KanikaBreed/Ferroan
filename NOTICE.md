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
| [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse), the OpenMetaverse library Tranquillity uses, copyright openmetaverse.co and contributors | BSD-3-Clause | [licenses/OpenMetaverse-BSD-3-Clause.txt](licenses/OpenMetaverse-BSD-3-Clause.txt) | Porting source for types, packets and LLSD. `crates/ferroan-types` `wire.rs`, `texture.rs`, `extra.rs` and `shape.rs` are ported from that tree at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` |
| [Aurora Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer), copyright the Aurora Viewer contributors | GPL-3.0-or-later | [LICENSE](LICENSE) | Rust crates reused or adapted (LLSD, message template code generation, prim volumes); viewer protocol reference |
| Second Life viewer and Firestorm, copyright Linden Research, Inc. and The Phoenix Firestorm Project, Inc. | LGPL-2.1, used here under GPL-3.0-or-later as LGPL-2.1 section 3 permits | [licenses/LGPL-2.1.txt](licenses/LGPL-2.1.txt) | The message template and protocol behavior. `ferroan-llsd` also follows the viewer's `llcommon/llsd.cpp` and `llsdserialize*.cpp` through Aurora's port |

The reference revisions used for porting are listed in [docs/REFERENCES.md](docs/REFERENCES.md). Each ported file names its source files and revision ([PORTING.md](docs/PORTING.md)).

`crates/ferroan-llsd` is adapted from Aurora Viewer `crates/aurora-llsd` at `48cb76e830a13965ac080128bb874afedda86d69` (GPL-3.0-or-later). Aurora describes that crate as a port of the Second Life viewer's `llcommon/llsd.cpp` and `llsdserialize*.cpp`, originally LGPL 2.1. Each adapted file names that path and revision. `number.rs`, the fixture test and `llsd-fuzz` are new Ferroan files. The encodings follow libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`; that C# source is not copied into this repository.

`crates/ferroan-types/src/wire.rs` is ported from libopenmetaverse `OpenMetaverse.Types` at the same revision (BSD-3-Clause). `src/texture.rs` is ported from `OpenMetaverse/Primitives/TextureEntry.cs` and `OpenMetaverse/Helpers.cs`. `src/extra.rs` and `src/shape.rs` are ported from `OpenMetaverse/Primitives/Primitive.cs`, `OpenMetaverse.Types/EnumsPrimitive.cs` and `UtilsConversions.cs`. Those files keep the BSD notice. `src/permissions.rs` is ported from Tranquillity `Source/OpenSim.Framework/Util.cs` and `PermissionsUtil.cs` at `713857f10b8e91a639a967d4354a4682b080793c` (BSD-3-Clause) and also names OpenMetaverse `Permissions.cs` for the shared bits. The rest of the crate is new Ferroan code.

`tools/config-oracle` calls the NuGet package `nini-core` 0.9.2.16 (X11, copyright Brent R. Matzelle, maintained by Sjofn LLC). `ferroan-config` reimplements that reader. Nini's source is not copied into this repository.

`crates/ferroan-http/src/query.rs` and `src/xml.rs` are ported from Tranquillity `Source/OpenSim.Server.Base/ServerUtils.cs` at `713857f10b8e91a639a967d4354a4682b080793c` (BSD-3-Clause). The files keep that notice. Percent-encoding and XML name escapes follow `HttpUtility` and `XmlConvert` as those methods call them; that .NET source is not copied. The HTTP server, client and LLSD framing are new Ferroan code.

## Native libraries (planned)

- [Jolt Physics](https://github.com/jrouwe/JoltPhysics), MIT, through `oxijolt-sys` (MIT OR Apache-2.0).
- Linden Lab's [SLua engine](https://github.com/secondlife/slua), MIT (copyright Linden Lab), a fork of Luau (MIT, copyright Roblox).

Their licence texts will be added when they become dependencies.

## Not used as sources

Benthic's metaverse crates (AGPL-3.0-or-later) may be read for comparison but are not copied or depended on.

## Names

OpenSimulator, Tranquillity, Aurora Viewer, Second Life, SLua and Linden Lab names identify sources and compatibility targets. Ferroan is independent and does not claim their endorsement. Second Life is a trademark of Linden Research, Inc.
