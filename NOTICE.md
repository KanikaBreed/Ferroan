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
| [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse), the OpenMetaverse library Tranquillity uses, copyright openmetaverse.co and contributors | BSD-3-Clause | [licenses/OpenMetaverse-BSD-3-Clause.txt](licenses/OpenMetaverse-BSD-3-Clause.txt) | Porting source for types, packets and LLSD. `crates/ferroan-types` `wire.rs`, `texture.rs`, `extra.rs` and `shape.rs` are ported from that tree at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. `crates/ferroan-protocol/message_template.msg` is a byte-for-byte copy of `data/message_template.msg` at that revision |
| [Aurora Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer), copyright the Aurora Viewer contributors | GPL-3.0-or-later | [LICENSE](LICENSE) | Rust crates reused or adapted (LLSD, message code generation, prim volumes); viewer protocol reference |
| Second Life viewer and Firestorm, copyright Linden Research, Inc. and The Phoenix Firestorm Project, Inc. | LGPL-2.1, used here under GPL-3.0-or-later as LGPL-2.1 section 3 permits | [licenses/LGPL-2.1.txt](licenses/LGPL-2.1.txt) | The message template and protocol behavior. `ferroan-llsd` also follows the viewer's `llcommon/llsd.cpp` and `llsdserialize*.cpp` through Aurora's port |

The reference revisions used for porting are listed in [docs/REFERENCES.md](docs/REFERENCES.md). Each ported file names its source files and revision ([PORTING.md](docs/PORTING.md)).

`crates/ferroan-llsd` is adapted from Aurora Viewer `crates/aurora-llsd` at `48cb76e830a13965ac080128bb874afedda86d69` (GPL-3.0-or-later). Aurora describes that crate as a port of the Second Life viewer's `llcommon/llsd.cpp` and `llsdserialize*.cpp`, originally LGPL 2.1. Each adapted file names that path and revision. `number.rs`, the fixture test and `llsd-fuzz` are new Ferroan files. The encodings follow libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`; that C# source is not copied into this repository.

`crates/ferroan-types/src/wire.rs` is ported from libopenmetaverse `OpenMetaverse.Types` at the same revision (BSD-3-Clause). `src/texture.rs` is ported from `OpenMetaverse/Primitives/TextureEntry.cs` and `OpenMetaverse/Helpers.cs`. `src/extra.rs` and `src/shape.rs` are ported from `OpenMetaverse/Primitives/Primitive.cs`, `OpenMetaverse.Types/EnumsPrimitive.cs` and `UtilsConversions.cs`. Those files keep the BSD notice. `src/permissions.rs` is ported from Tranquillity `Source/OpenSim.Framework/Util.cs` and `PermissionsUtil.cs` at `713857f10b8e91a639a967d4354a4682b080793c` (BSD-3-Clause) and also names OpenMetaverse `Permissions.cs` for the shared bits. The rest of the crate is new Ferroan code.

`tools/config-oracle` calls the NuGet package `nini-core` 0.9.2.16 (X11, copyright Brent R. Matzelle, maintained by Sjofn LLC). `ferroan-config` reimplements that reader. Nini's source is not copied into this repository.

`crates/ferroan-http/src/query.rs` and `src/xml.rs` are ported from Tranquillity `Source/OpenSim.Server.Base/ServerUtils.cs` at `713857f10b8e91a639a967d4354a4682b080793c` (BSD-3-Clause). The files keep that notice. Percent-encoding and XML name escapes follow `HttpUtility` and `XmlConvert` as those methods call them; that .NET source is not copied. The HTTP server, client and LLSD framing are new Ferroan code.

`crates/ferroan-services` and `crates/ferroan-robust` follow Tranquillity's asset handlers, `AssetService` and `SQLiteAssetData` at `713857f10b8e91a639a967d4354a4682b080793c` (BSD-3-Clause). The C# sources are not copied. The XML bytes are what `XmlSerializer` emits; that .NET source is not copied. `tools/asset-oracle` calls those types and is new Ferroan code.

`tools/capture/tranquillity-capture.patch` modifies Tranquillity `Source/OpenSim.Region.ClientStack.LindenUDP/OpenSimUDPBase.cs` (copyright Clutch, Inc. 2006, BSD) and `LLUDPServer.cs` (copyright Contributors, http://opensimulator.org/, BSD-3-Clause) at `713857f10b8e91a639a967d4354a4682b080793c`. The new `DatagramCapture.cs` in that patch is Ferroan code (GPL-3.0-or-later). The patch is applied only to a scratch build and is not sent upstream. Those C# sources are not otherwise copied.

`tools/bot` (`ferroan-bot`) adapts Aurora Viewer `crates/aurora-net` `xmlrpc.rs`, `login.rs`, `circuit.rs`, the `group` function in `throttle.rs`, and the seed POST in `caps.rs` at `48cb76e830a13965ac080128bb874afedda86d69` (GPL-3.0-or-later). The handshake order follows `session.rs` and `session/object_cache.rs` in that tree. The throttle channel shares follow the Second Life viewer's `llviewerthrottle.cpp` (LGPL-2.1) through that Aurora file. Inventory-offer receipts, packet-loss statistics and the dynamic throttle are not included. Each adapted file names its source. The command, the session driver and the tests are new Ferroan code.

`crates/ferroan-protocol` is adapted from Aurora Viewer `crates/aurora-msg` at `48cb76e830a13965ac080128bb874afedda86d69` (GPL-3.0-or-later). `build.rs`, `src/lib.rs` and `src/packet.rs` name that path and revision. `message_template.msg` is the libopenmetaverse copy described above, originally the Second Life message template (LGPL-2.1). No header is prepended to it, because the oracle requires the bytes to match the checkout. Quaternion packing follows OpenMetaverse `Quaternion.ToBytes` / `FromBytes` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. Zero-coding follows `Helpers.ZeroDecode` in that tree and Tranquillity `LLUDPServer.ZeroEncode` / `SendPacketFinal` at `713857f10b8e91a639a967d4354a4682b080793c`. The C# sources are not copied. The fixture test, `protocol-fuzz` and the packet half of `tools/reference-oracle` are new Ferroan code. The choices where Aurora's template differs are in [docs/PROTOCOL.md](docs/PROTOCOL.md).

`crates/ferroan-region` ports the local grid login boundary from Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c` (BSD-3-Clause). `config.rs` follows `Source/OpenSim.Framework/RegionInfo.cs`, `NetworkServersInfo.cs`, `Source/OpenSim.ApplicationPlugins.LoadRegions/RegionLoaderFileSystem.cs`, `Source/OpenSim.Server.RegionServer/OpenSimBase.cs` and `Source/OpenSim.Region.Framework/Scenes/Scene.cs`. `grid.rs` follows `Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs`, `Source/OpenSim.Services.Interfaces/IGridService.cs` and `Source/OpenSim.Server.Base/ProtocolVersions.cs`, read with `GridServerPostHandler.cs` and `RegionGridServiceConnector.cs`. `simulation.rs` follows `Source/OpenSim.Server.Handlers/Simulation/AgentHandlers.cs` and `Utils.cs`, `Source/OpenSim.Framework/AgentCircuitData.cs`, `EntityTransferContext.cs`, `VersionInfo.cs` and `CapsUtil.cs`, and `Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Simulation/LocalSimulationConnector.cs`. Each port keeps the full BSD notice and exact source paths. The process runner and capture comparison tools are new Ferroan code. Gzip decoding uses `flate2` with its Rust backend, not a native zlib library.

## Native libraries (planned)

- [Jolt Physics](https://github.com/jrouwe/JoltPhysics), MIT, through `oxijolt-sys` (MIT OR Apache-2.0).
- Linden Lab's [SLua engine](https://github.com/secondlife/slua), MIT (copyright Linden Lab), a fork of Luau (MIT, copyright Roblox).

Their licence texts will be added when they become dependencies.

## Not used as sources

Benthic's metaverse crates (AGPL-3.0-or-later) may be read for comparison but are not copied or depended on.

## Names

OpenSimulator, Tranquillity, Aurora Viewer, Second Life, SLua and Linden Lab names identify sources and compatibility targets. Ferroan is independent and does not claim their endorsement. Second Life is a trademark of Linden Research, Inc.
