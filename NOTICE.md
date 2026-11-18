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
| [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse), the OpenMetaverse library Tranquillity uses, copyright openmetaverse.co and contributors | BSD-3-Clause | [licenses/OpenMetaverse-BSD-3-Clause.txt](licenses/OpenMetaverse-BSD-3-Clause.txt) | Porting source for types, packets and LLSD. `crates/ferroan-types` `wire.rs`, `texture.rs`, `extra.rs` and `shape.rs` are ported from that tree at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. `crates/ferroan-protocol/message_template.msg` uses `data/message_template.msg` at that revision, with the eight viewer trailing layouts described below |
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

`crates/ferroan-protocol` is adapted from Aurora Viewer `crates/aurora-msg` at `48cb76e830a13965ac080128bb874afedda86d69` (GPL-3.0-or-later). `build.rs`, `src/lib.rs` and `src/packet.rs` name that path and revision. `message_template.msg` uses the libopenmetaverse layout plus only the trailing blocks of eight messages from Linden Research, Inc., `secondlife/viewer` `scripts/messages/message_template.msg` at `ff75ab037607d8fe305f43f8b1f43126693ba2db` (LGPL-2.1, used under GPL-3.0-or-later as section 3 permits). The oracle verifies unchanged libopenmetaverse headers and existing blocks, and checks the eight allowed suffixes separately. Quaternion packing follows OpenMetaverse `Quaternion.ToBytes` / `FromBytes` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. Zero-coding follows `Helpers.ZeroDecode` in that tree and Tranquillity `LLUDPServer.ZeroEncode` / `SendPacketFinal` at `713857f10b8e91a639a967d4354a4682b080793c`. The C# sources are not copied. The fixture test, `protocol-fuzz` and the packet half of `tools/reference-oracle` are new Ferroan code. The choices where Aurora's template differs are in [docs/PROTOCOL.md](docs/PROTOCOL.md).

`crates/ferroan-region` ports the local grid login boundary from Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c` (BSD-3-Clause). `config.rs` follows `Source/OpenSim.Framework/RegionInfo.cs`, `NetworkServersInfo.cs`, `Source/OpenSim.ApplicationPlugins.LoadRegions/RegionLoaderFileSystem.cs`, `Source/OpenSim.Server.RegionServer/OpenSimBase.cs` and `Source/OpenSim.Region.Framework/Scenes/Scene.cs`. `grid.rs` follows `Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs`, `Source/OpenSim.Services.Interfaces/IGridService.cs` and `Source/OpenSim.Server.Base/ProtocolVersions.cs`, read with `GridServerPostHandler.cs` and `RegionGridServiceConnector.cs`. `simulation.rs` follows `Source/OpenSim.Server.Handlers/Simulation/AgentHandlers.cs` and `Utils.cs`, `Source/OpenSim.Framework/AgentCircuitData.cs`, `EntityTransferContext.cs`, `VersionInfo.cs` and `CapsUtil.cs`, and `Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Simulation/LocalSimulationConnector.cs`. Each port keeps the full BSD notice and exact source paths. The process runner and capture comparison tools are new Ferroan code. Gzip decoding uses `flate2` with its Rust backend, not a native zlib library.

Q7's harness provenance: `xtask/src/grid.rs` adapts the configuration in Tranquillity's `Source/OpenSim.Server.GridServer/AppData/GridServer.ini.example`, `Source/OpenSim.Server.RegionServer/AppData/config-include/Grid.ini` and `GridCommon.ini.example` at `858321883f5487a5d192d36ea9d7b90fa468310c`, under the repository's OpenSimulator BSD notice. The process and capture orchestration are new Ferroan code. `tools/bot/src/caps.rs` also follows `Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetModule.cs` and `EventQueueGetHandlers.cs` at that pin (BSD-3-Clause); its header retains their notice alongside the existing Aurora provenance.

Q8's `crates/ferroan-client/src/circuit.rs` follows `Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs` and `LLClientView.cs` at Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c` and retains their BSD notice. `handshake.rs` also follows `LLUDPZeroEncoder.cs`, `Source/OpenSim.Framework/Util.cs`, `RegionSettings.cs`, `EstateSettings.cs`, and `Source/OpenSim.Region.Framework/Scenes/Scene.cs`, with flag definitions from libopenmetaverse `OpenMetaverse/Simulator.cs` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`; both original BSD notices are retained. `crates/ferroan-protocol/src/reliability.rs` extracts the Aurora-derived bot state with its existing GPL notices and revision, while preserving separate caller policies. Capture extraction and injected-error tests are original Ferroan code.

Q9's `crates/ferroan-client/src/caps.rs` and event queue follow Tranquillity `Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCaps.cs`, `EventQueue/EventQueueGetModule.cs`, `Source/OpenSim.Framework.Servers.HttpServer/BaseHttpServer.cs`, `PollServiceRequestManager.cs`, `PollServiceHttpRequest.cs`, `SimpleStreamHandler.cs` and `OSHttpServer/HttpResponse.cs` at `858321883f5487a5d192d36ea9d7b90fa468310c` (BSD-3-Clause). The files retain the source notices. Seed names use `OSD.AsString` conversion from libopenmetaverse `OpenMetaverse.StructuredData/StructuredData.cs` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`; the LLSD helper retains that BSD notice. The harness probes, capture extraction and fixture comparisons are original Ferroan code.

## Native libraries (planned)

- [Jolt Physics](https://github.com/jrouwe/JoltPhysics), MIT, through `oxijolt-sys` (MIT OR Apache-2.0).
- Linden Lab's [SLua engine](https://github.com/secondlife/slua), MIT (copyright Linden Lab), a fork of Luau (MIT, copyright Roblox).

Their licence texts will be added when they become dependencies.

## Not used as sources

Benthic's metaverse crates (AGPL-3.0-or-later) may be read for comparison but are not copied or depended on.

## Compatibility follow-ups

Q10b's LLSD XML binary decoder follows `OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs` at libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`, retaining its BSD notice alongside the existing Aurora provenance. Malformed and edge-case binary inputs are evaluated by the C# oracle. The XML reader preserves self-closing syntax to distinguish the reference's empty-binary paths. Bounded bot notification storage and the live seed probes are original Ferroan code.

Q10a's `crates/ferroan-client/src/arrival.rs` follows Tranquillity `Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs`, `Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs`, `Scene.cs`, and `Source/OpenSim.Framework/Util.cs` at `858321883f5487a5d192d36ea9d7b90fa468310c`, plus libopenmetaverse `OpenMetaverse.Types/UtilsConversions.cs` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. Both BSD notices are retained. The region configuration uses `ScenePresence.CompleteMovement`'s initial look fallback. Arrival capture extraction is original Ferroan code; fixtures come from the pinned C# server.

## Names

OpenSimulator, Tranquillity, Aurora Viewer, Second Life, SLua and Linden Lab names identify sources and compatibility targets. Ferroan is independent and does not claim their endorsement. Second Life is a trademark of Linden Research, Inc.
