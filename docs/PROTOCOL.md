# LLUDP messages

Ferroan follows Tranquillity's observable datagrams. This note records the generator's template sources, the eight appended viewer layouts needed beyond libopenmetaverse, and the framing and fixture checks.

## Which template

The generator reads [crates/ferroan-protocol/message_template.msg](../crates/ferroan-protocol/message_template.msg). Its base is libopenmetaverse `data/message_template.msg` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`: all 478 messages, their numbers, flags and existing fields remain. Eight messages append the blocks listed below from Linden's [scripts/messages/message_template.msg](https://github.com/secondlife/viewer/blob/ff75ab037607d8fe305f43f8b1f43126693ba2db/scripts/messages/message_template.msg) at `ff75ab037607d8fe305f43f8b1f43126693ba2db`. No other viewer message or layout change is imported. The oracle validates this exact set of appended layouts against the pinned libopenmetaverse base; it no longer requires the whole extended file to be byte-identical to that base. Provenance and licences are recorded in [NOTICE.md](../NOTICE.md) and the crate headers.

Aurora Viewer `crates/aurora-msg/message_template.msg` at `48cb76e830a13965ac080128bb874afedda86d69` has 483 messages. Its differences from the libopenmetaverse base are summarized below. Ferroan does not import that whole template. Tranquillity links libopenmetaverse but also writes some packets directly, so its wire layout can include viewer blocks missing from the linked library's generated classes.

The codec is adapted from Aurora `crates/aurora-msg` at that revision. The behavior reference is Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c`, together with its pinned libopenmetaverse implementation. A template-only comparison does not replace checking actual C# sends.

## Messages only Aurora has

These five messages are in the viewer template and not in libopenmetaverse. Ferroan does not define them. `reencode_datagram` returns an error for an unknown message number.

| Message | Aurora header |
| --- | --- |
| `RetrieveIMsExtended` | Low 427, Trusted, Unencoded |
| `JoinGroupRequestExtended` | Low 428, Trusted, Unencoded |
| `CreateGroupRequestExtended` | Low 429, Trusted, Unencoded |
| `ObjectBypassModUpdate` | Low 431, NotTrusted, Zerocoded |
| `GameControlInput` | High 32, NotTrusted, Zerocoded |

`GameControlInput` is the only one on the viewer-to-simulator path. The other four are trusted (simulator or grid) messages. None of them share a number with a libopenmetaverse message of the same frequency.

## Header flags

On the 478 shared messages, frequency, number, trust and zero-coding agree. Three Aurora headers add `UDPDeprecated` and the libopenmetaverse header does not:

| Message | Shared header | Aurora only |
| --- | --- | --- |
| `ParcelProperties` | High 23, Trusted, Zerocoded | `UDPDeprecated` |
| `RoutedMoneyBalanceReply` | Low 315, Trusted, Zerocoded | `UDPDeprecated` |
| `SetSimPresenceInDatabase` | Low 23, Trusted, Unencoded | `UDPDeprecated` |

The flag is a viewer annotation. It does not change the message number or the body. Ferroan follows libopenmetaverse and does not treat the messages as deprecated. `OpenCircuit` is `UDPBlackListed` in both templates.

## Appended viewer blocks

These eight layouts come from the pinned Linden viewer template above and are also present in the pinned Aurora template. The libopenmetaverse prefix is unchanged. Ferroan models their fields and preserves whether each trailing block was actually present. The four zero bytes in Tranquillity's `ImprovedInstantMessage` are `EstateBlock.EstateID`, not padding or a C# bug; dropping them caused the Q10 capture failure.

| Message | Blocks appended to the libopenmetaverse layout |
| --- | --- |
| `AlertMessage` | `AgentInfo` Variable: `AgentID` LLUUID |
| `AvatarAppearance` | `AttachmentBlock` Variable: `ID` LLUUID, `AttachmentPoint` U8 |
| `ChildAgentUpdate` | `AgentInventoryHost` Variable: `InventoryHost` Variable 1 |
| `ImprovedInstantMessage` | `EstateBlock` Single: `EstateID` U32. `MetaData` Variable: `Data` Variable 2 |
| `InviteGroupResponse` | `GroupData` Single: `GroupLimit` S32 |
| `RegionInfo` | `RegionInfo5` Variable: `ChatWhisperRange`, `ChatNormalRange`, `ChatShoutRange`, `ChatWhisperOffset`, `ChatNormalOffset`, `ChatShoutOffset` F32; `ChatFlags` U32. `CombatSettings` Variable: `CombatFlags` U32, `OnDeath` U8; `DamageThrottle`, `RegenerationRate`, `InvulnerabilyTime`, `DamageLimit` F32 (the spelling is the viewer template's) |
| `SimStatus` | `SimFlags` Single: `Flags` U64 |
| `TransferInventory` | `ValidationBlock` Single: `NeedsValidation` BOOL, `EstateID` U32 |

### Sender and reader audit

The audit covers Tranquillity's `Source/` at `858321883f5487a5d192d36ea9d7b90fa468310c`, and the generated readers in libopenmetaverse `OpenMetaverse/_Packets_.cs` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. A count described as absent is not an explicit count of zero.

| Message | What the pinned C# server emits | C# reader behavior | Current Ferroan runtime sender |
| --- | --- | --- | --- |
| `AlertMessage` | `LLClientView.SendAlertMessage` uses `AlertMessagePacket`. `AlertInfo` has its count (0 or 1 in these overloads); the added `AgentInfo` count is absent. Commented-out `AgentInfo` assignments are not a send path. | The library reads through `AlertInfo`, leaving any `AgentInfo` suffix unread. No Tranquillity UDP handler for this message. | None |
| `AvatarAppearance` | `LLClientView.SendAppearance` writes `AppearanceData` count 0 for the historical path or 1 for a baked appearance, then `AppearanceHover` count 1 and its vector. It stops there: no `AttachmentBlock` count. | The library reads through `AppearanceHover` and does not consume attachment blocks. No Tranquillity UDP handler for this message. | `ferroan-region` sends the historical path on root arrival: `AppearanceData` count 0, one `AppearanceHover`, no `AttachmentBlock`. No baking. |
| `ChildAgentUpdate` | No UDP sender found in Tranquillity. Its HTTP simulation agent-update path is a different format. No claim that it emits an `AgentInventoryHost` count. | The library has a reader through the existing `AgentInfo` block, ignoring any inventory-host suffix. No Tranquillity UDP handler. | None |
| `ImprovedInstantMessage` | `LLClientView.SendInstantMessage` manually writes the prefix, then `EstateID = 0` as four little-endian bytes. It omits the `MetaData` count. The actual packet is reliable and unencoded even though the template marks it Zerocoded. | The library stops after `MessageBlock.BinaryBucket`. Tranquillity's `HandlerImprovedInstantMessage` consumes only that prefix and does not inspect either appended block. | Q10's companion bot in `xtask/src/ferrobot.rs` sends a test IM using the default zero `EstateBlock` and absent `MetaData`; no region server sender |
| `InviteGroupResponse` | No UDP sender found; no emitted `GroupData` is claimed. | No generated library packet class and no Tranquillity UDP handler. | None |
| `RegionInfo` | `LLClientView.SendRegionInfoToEstateMenu` uses `RegionInfoPacket`, with the existing `RegionInfo3` count explicitly 0. Neither `RegionInfo5` nor `CombatSettings` has a count byte. | The library reads through `RegionInfo3`, ignoring the appended blocks. No Tranquillity handler for this message (`RequestRegionInfo` and `GodUpdateRegionInfo` are separate messages). | None (`RegionHandshake.RegionInfo` is a block in a different message) |
| `SimStatus` | No UDP sender found. The `/simstatus` HTTP endpoint is unrelated; no emitted `SimFlags` is claimed. | No generated library packet class and no Tranquillity UDP handler. | None |
| `TransferInventory` | No UDP sender found; no emitted `ValidationBlock` is claimed. | No generated library packet class and no Tranquillity UDP handler. | None |

The send methods and IM handler are in [LLClientView.cs](https://github.com/OpenSim-NGC/OpenSim-Tranquillity/blob/858321883f5487a5d192d36ea9d7b90fa468310c/Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs). The five library readers above are in [_Packets_.cs](https://github.com/OpenSim-NGC/libopenmetaverse/blob/a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b/OpenMetaverse/_Packets_.cs). They do not require the input to end after their known blocks, so they tolerate the suffix without exposing its values. Their `ToBytes` methods write only the libopenmetaverse prefix. The absence of a server sender or handler was checked across `Source/`, not inferred from a missing method in `LLClientView` alone. Local reference histories contain no later relevant fixes beyond these pins.

### Preserving an absent suffix

Generated messages with an optional trailing block sequence expose `omitted_trailing_blocks`. This records how many whole blocks at the end were absent. An absent Variable count and an explicitly encoded count of zero both decode to an empty collection, but have different omission metadata and therefore different re-encoded bytes. An entirely absent appended Single block is recorded the same way. Appended ACKs remain outside the message body and cannot be mistaken for a trailing block.

The encoder omits a requested trailing suffix only while those blocks are empty, walking from the end and stopping at the first block that holds data. A block that holds data is written, and so is everything before it. An empty block is still written when `omitted_trailing_blocks` is zero, including an explicit zero Variable count. Variable emptiness is an empty collection; a collection of default elements is data. Single emptiness is `Default::default()`. New appended blocks default to absent, except `ImprovedInstantMessage`: its default emits `EstateBlock` with zero and omits empty `MetaData`, matching the C# server. Existing blocks retain their normal default emission. Q10's companion uses the IM default for its client-to-server test message. None of the eight has a current Ferroan region server sender; codec defaults do not implement the missing services.

A partly present trailing Single is not an error and is not zero-filled. That block and every later optional block are treated as omitted, and the leftover bytes are ignored, so re-encoding equals the prefix without the partial suffix. This matches libopenmetaverse `FromBytes`, which ignores bytes past its shorter template. A partial Variable block still returns `DecodeError::Eof`.

A future region instant-message sender must pass `zerocoded: false` into the framer for that send. C# `LLClientView.SendInstantMessage` uses a reliable header and leaves `MSG_ZEROCODED` commented out ("not doing spec zeroencode on this"), even though the template marks `ImprovedInstantMessage` Zerocoded. `Circuit` `send_message` takes the flag from `M::ZEROCODED`, so that sender has to override it. This crate does not implement the sender.

This presence tracking also keeps an older `RegionHandshake` without a `RegionInfo4` count byte unchanged on replay. The old implementation decoded that missing count as zero but re-encoded a zero byte, which was not lossless. `Reader::take` is unchanged: a fixed-field read of up to 64 bytes at the exact end returns zero-filled bytes, while a partially available field returns an error. The message decoder does not ask `Reader` to read a short optional Single; it records that block as omitted instead.

## Message numbers

| Frequency | On the wire |
| --- | --- |
| High | one byte |
| Medium | `0xFF`, then one byte |
| Low | `0xFF 0xFF`, then a big-endian u16 |
| Fixed | the same four bytes as Low, with the low 16 bits of the template number |

The fixed messages in this template are `PacketAck` `0xFFFFFFFB` (Low `0xFFFB`), `OpenCircuit` `0xFFFFFFFC` (Low `0xFFFC`) and `CloseCircuit` `0xFFFFFFFD` (Low `0xFFFD`). Low numbers in the template stop at 430, so they do not collide with those three. The oracle stores the template number as a signed 32-bit integer, which is why `PacketAck`'s manifest `number` is `-5`. The wire bytes are `FF FF FF FB`.

## Framing

```text
[flags u8][sequence u32 BE][extra_len u8][extra...][message number][body...][ack u32 BE...][ack count u8]
```

| Flag | Bit | Meaning |
| --- | --- | --- |
| `APPENDED_ACKS` | `0x10` | ack list is present |
| `RESENT` | `0x20` | the sender is repeating the packet |
| `RELIABLE` | `0x40` | the receiver should ack it |
| `ZEROCODED` | `0x80` | the message number and body are zero-coded |

Zero-coding covers the message number and the body. The 6-byte header and the ack trailer stay raw. A run is `0x00` and a count. A count of 0 adds nothing, matching `Helpers.ZeroDecode`. A run of 256 zeroes is `00 FF` and a new run that starts at 1, because Tranquillity's `LLUDPServer.ZeroEncode` keeps the count in a `byte`. Aurora treated a count of 0 as 256; Ferroan does not.

`Packet.ToBytes` sets the zerocoded flag and writes the body raw. Encoding happens in `LLUDPServer.ZeroEncode`. The send path keeps the coded form when `coded length <= raw length`. Ferroan does the same. Acks are appended after that decision, so the trailer is never coded.

`reencode_datagram` follows the zerocode flag on the datagram, not the template bit. Tranquillity sets the flag on messages the template marks Unencoded: `LLClientView.SendViewerTime` sets `Header.Zerocoded` on `SimulatorViewerTimeMessage`, and `terseUpdateHeader` is `MSG_RELIABLE | MSG_ZEROCODED` for `ImprovedTerseObjectUpdate` with the comment "zero code is not as spec". A capture round-trip has to reproduce those bytes. Packets this crate sends still take the flag from the template, except a future region instant-message sender, which must pass `zerocoded: false` as described with the appended IM blocks.

The extra header sits before the message number. Tranquillity writes `extra_len` 0. Aurora's parser reads the message number first and then skips extra bytes after it. Ferroan skips extra before the number, which matches `Header.ToBytes` when the length is 0. `reencode_datagram` rejects a non-empty extra. A non-zero extra combined with zero-coding is not claimed: C# zero-codes everything after the 6-byte header, including the extra, and this crate zero-codes only the message number and body.

Appended acks stay in wire order. OpenMetaverse `Header.FromBytes` stores them reversed, so `FromBytes` then `ToBytes` swaps the ack bytes. Ferroan does not. Acks are a set for `Acknowledge`; the order does not change which packets are retired. Do not use that C# round trip as the ack oracle. The generated oracle packet fixtures have no acks.

`build_raw` appends as many caller-supplied acks as fit:

```text
while n < acks.len() && n < 255 && out.len() + 4 * n + 5 < MTU { n += 1 }
```

`MTU` is `LLUDPServer.MTU`, 1400. The final datagram is at most 1399 bytes when acks were appended. Tranquillity's `SendPacketFinal` uses that loop only when the flags byte is exactly `RELIABLE` (a still-set `ZEROCODED` bit skips it) and `UnackedMethod` is null. Reliable packets usually have `UnackedMethod` set, so they leave with no appended acks. This crate does not model that policy. It frames the acks it is given, including on a reliable or zerocoded packet. A circuit that wants Tranquillity's send policy passes an empty ack list in those cases.

A received datagram longer than 4096 bytes is rejected (`UDPPacketBuffer.BUFFER_SIZE`). Zero-expansion is capped at 256 KiB. OpenMetaverse does not cap it. A datagram of `00 FF` pairs that would grow past the cap is `DecodeError::Malformed`. An honest packet under 1400 bytes expands to well under the cap.

`OpenMetaverse.Packet.MTU` is 1200 and is the size at which `ToBytesMultiple` splits a variable block. This crate does not split messages. `CoarseLocationUpdate` cannot be split in C# either. Splitting waits for the circuit.

## Field layout

Numeric fields are little-endian. `BOOL` is a byte, 1 or 0. `LLUUID` is 16 bytes in network order. `LLVector3` is 12 bytes, `LLVector3d` 24, `LLVector4` 16, each component little-endian. `Fixed` is N raw bytes with no length (the template uses 4 and 32). `Variable 1` is a u8 length and then that many bytes. `Variable 2` is a little-endian u16 length and then that many bytes. The codec copies the bytes. It does not require a trailing NUL. `str_field` appends a NUL for a caller that wants the C# string shape.

A variable block is a count byte and then that many blocks. A multiple block is N copies and no count.

`IPADDR` is a little-endian u32. The template comment calls it "U32 encoded IP". The four bytes are stored in the order they appear on the wire, which is little-endian, not network order. `IPPORT` is a big-endian u16.

`LLQuaternion` uses `Quaternion.ToBytes` and `FromBytes(..., normalized: true)`, 12 bytes, not `GetBytes`. `ToBytes` writes `scale * X, Y, Z` with `scale = -1/sqrt(norm)` when W is negative and `1/sqrt(norm)` otherwise, when `norm > 1e-6 || norm < 0.9999`. `norm` is `LengthSquared`. On x86-64 with SSE4.1 that is `Sse41.DotProduct(q, q, 0xf1)`, and `DPPS` adds the products in pairs, `(x² + y²) + (z² + w²)`; the scalar branch other machines take adds left to right, and differs in the last bit of the written floats for about one quaternion in eleven. `quat_length_squared` and `ferroan-types`' `length_squared` use the x86-64 order ([D71](DECISIONS.md)). The oracle's `fixtures/oracle/bytes/quaternion-batch.bin` holds 4096 quaternions with their `ToBytes` and `GetBytes` bytes from OpenMetaverse on x86-64: raw components, normalized ones, normalized ones read back with `FromBytes`, scaled ones, and eight fixed edges including the zero quaternion, which C# on x86-64 writes as three `0xFFC00000` NaNs; Ferroan writes those bits on every machine, ARM64 included. Both crates' tests encode every record and compare the bytes, and check that the left-to-right sum would write other bytes for some of them and that some read-back records are not a fixed point. The other branch writes three zeroes and is the non-finite path. `FromBytes` sets W to `sqrt(1 - x² - y² - z²)` when that sum is greater than `1e-6`, and 0 otherwise. `pack_quat` is that rescaling for every outbound quaternion, including region packets. The rebuilt `w` is not the `w` that was scaled, so bytes just written are not always what a second call writes. Identity `(0, 0, 0, 1)` encodes as three zeroes. Ferrobot normalizes with glam and writes `x`, `y`, `z`. Some headings are not a fixed point of the rescaling. The capture check re-encodes a packet the server sent byte for byte, with each packed quaternion's x, y and z kept as read, so a valid C# packet cannot fail on its quaternions. A packet the client sent is compared field by field: a quaternion component may differ by at most `2e-6`, the header, the body length and every other byte must match, and the zero-coding must be canonical ([CAPTURES.md](CAPTURES.md)). A mismatch still writes `reencode-mismatch.log`. The bytes-family fixtures in `ferroan-types` still cover `GetBytes`. Packet fixtures use `ToBytes`.

## Fixtures

`cargo xtask oracle generate` retains the 478 libopenmetaverse datagrams under `fixtures/oracle/packet/` byte-for-byte and adds eight extended datagrams under `fixtures/oracle/packet-extension/`. The original family is `packet`, the format is `datagram`, and the id is `packet/{Name}`. Each body is filled the same way:

| Field | Value |
| --- | --- |
| u8, i8 | `0x2A` |
| u16, i16 | `0x1234` |
| u32, i32, IPADDR | `0x01020304` |
| u64 | `0x0102030405060708` |
| f32, f64 | 1.5 |
| BOOL | true |
| LLUUID | `00112233-4455-6677-8899-aabbccddeeff` |
| LLVector3, LLVector3d | (1, 2, 3) |
| LLVector4 | (1, 2, 3, 4) |
| LLQuaternion | (0, 0, 0, 1), through `ToBytes` |
| IPPORT | `0x1234` |
| Variable | `11 22 33` |
| Fixed | `0xA5` repeated |
| Variable block | one element |
| Multiple block | the template count |

The header is sequence 0, reliable, extra length 0, no acks. Zero-coding is applied when the template says so, and the coded form is kept when it is not larger than the raw form.

387 messages have an `OpenMetaverse.Packets.{Name}Packet` class. For each original `packet` fixture, the generic encoder's datagram equals `ToBytes` plus the same zero-coding. These comparisons are unchanged by the extensions. 385 of them also survive `Packet.BuildPacket`. Two do not, and that is an OpenMetaverse decoder bug, not a byte mismatch: the constructor allocates a fixed-count block as an array of nulls, and `FromBytes` fills the elements only when the length is not already the template count.

- `TestMessage.NeighborBlock` is `new NeighborBlockBlock[4]`.
- `ViewerStats.NetStats` is `new NetStatsBlock[2]`.

`ToBytes` still matches for both. The oracle counts the `BuildPacket` successes and does not require them. The decode recorded in the manifest checks the name and that the datagram is at least 7 bytes. It does not call `BuildPacket`.

The other 91 messages have no generated class. They use the same layout rules. The check that those rules match OpenMetaverse is the 387 class comparisons. The messages are `AddCircuitCode`, `NeighborList`, `AvatarTextureUpdate`, `SimulatorMapUpdate`, `SimulatorSetMap`, `SubscribeLoad`, `UnsubscribeLoad`, `SimulatorReady`, `SimulatorPresentAtLocation`, `SimulatorLoad`, `SimulatorShutdownRequest`, `RegionPresenceRequestByRegionID`, `RegionPresenceRequestByHandle`, `RegionPresenceResponse`, `UpdateSimulator`, `LogDwellTime`, `FeatureDisabled`, `LogFailedMoneyTransaction`, `UserReportInternal`, `SetSimStatusInDatabase`, `SetSimPresenceInDatabase`, `AvatarPickerRequestBackend`, `DirFindQueryBackend`, `DirPlacesQueryBackend`, `DirClassifiedQueryBackend`, `DirLandQueryBackend`, `DirPopularQueryBackend`, `GroupNoticeAdd`, `DataHomeLocationRequest`, `DataHomeLocationReply`, `NearestLandingRegionRequest`, `NearestLandingRegionReply`, `NearestLandingRegionUpdated`, `TeleportLandingStatusChanged`, `KickUserAck`, `SystemKickUser`, `AvatarPropertiesRequestBackend`, `RequestParcelTransfer`, `UpdateParcel`, `RemoveParcel`, `MergeParcel`, `LogParcelChanges`, `CheckParcelSales`, `ParcelSales`, `StartAuction`, `ConfirmAuctionStart`, `CompleteAuction`, `CancelAuction`, `CheckParcelAuctions`, `ParcelAuctions`, `ChatPass`, `EdgeDataPacket`, `SimStatus`, `AtomicPassObject`, `KillChildAgents`, `DataServerLogout`, `RequestInventoryAsset`, `InventoryAssetResponse`, `TransferInventory`, `TransferInventoryAck`, `FormFriendship`, `EventLocationRequest`, `EventLocationReply`, `MoneyTransferBackend`, `SetStartLocation`, `NetTest`, `SetCPURatio`, `SimCrashed`, `NameValuePair`, `RemoveNameValuePair`, `UpdateAttachment`, `RemoveAttachment`, `EmailMessageRequest`, `EmailMessageReply`, `InternalScriptMail`, `ScriptDataRequest`, `ScriptDataReply`, `InviteGroupResponse`, `TallyVotes`, `LogTextMessage`, `CreateTrustedCircuit`, `DenyTrustedCircuit`, `RequestTrustedCircuit`, `ParcelRename`, `SystemMessage`, `RpcChannelRequest`, `RpcChannelReply`, `RpcScriptRequestInbound`, `RpcScriptRequestInboundForward`, `RpcScriptReplyInbound` and `ScriptMailRegistration`.

The eight `packet-extension` fixtures populate the appended blocks using the same field values. Their manifest records the viewer pin, appended block names and original raw-prefix length before zero-coding. Each extended raw prefix must equal its original libopenmetaverse fixture. Five of these messages have library classes; three do not, as the audit records. The appended bytes are template-generated layout evidence, not evidence that a C# server sent those values.

`crates/ferroan-protocol/tests/oracle_fixtures.rs` calls `reencode_datagram` on every original and extended fixture, requiring identical bytes, exactly 478 original cases and eight extension cases. It also checks each extended body's original prefix. The ordinary CI jobs run those Rust tests. `cargo xtask oracle verify` encodes each case again with the C# tool and compares; the [Linux live workflow](../.github/workflows/live.yml) runs that command with the pinned references and .NET SDK.

The separate [trailing-block capture](../fixtures/trailing/README.md) contains an actual 191-byte C# `ImprovedInstantMessage`: its 187-byte library prefix is followed by four zero bytes for `EstateBlock`, with no `MetaData` count. `cargo xtask live trailing-fixtures <udp-capture>` requires the private raw capture to round-trip losslessly before replacing identities, timestamp and text directly at their existing lengths. The test `captured_csharp_im_keeps_estate_block_and_omits_metadata_count` checks the recorded suffix, its presence metadata and complete byte-for-byte re-encoding. This captured evidence is separate from the generic oracle's extension fixtures.

Malformed datagrams return `DecodeError`. The unit tests cover a short header, an ack count that runs past the buffer, a zero-expansion bomb, and a zero run whose count is 0. `protocol-fuzz` reads stdin, truncates to 4096 bytes and calls `reencode_datagram`. `cargo xtask check` compiles it. It is not a coverage-guided fuzzer.

## Redaction

`scrub_datagram` is the capture scrub. It decodes one datagram, replaces session UUIDs with `00000000-0000-0000-0000-000000000002`, and replaces agent, user, owner, creator, abuser and reporter UUIDs, plus the circuit block's `ID`, with `00000000-0000-0000-0000-000000000001`. Circuit `Code` fields become 1. Every `IPADDR` becomes `127.0.0.1`. Variable fields named Name, FromName, FromAgentName, FirstName, LastName, UserName, NameValue or ObjectOwner, and variable fields whose names contain password, passwd, secret, token or email, become the bytes of `Redacted`. Ports, other object UUIDs and chat text stay. `SourceID` stays, because that field is an agent id on some messages and an object id on others. The datagram is then encoded again, so a redacted capture is a Ferroan encoding of the scrubbed fields. [CAPTURES.md](CAPTURES.md) says to prove the raw datagram round-trips before that replacement.

## Not claimed

The oracle compares against libopenmetaverse's encoder and Tranquillity's zero-coding on Linux; a Windows oracle run was not done. Captured datagrams add C# send-path evidence, while the bot and region live scenarios are recorded in [LIVE.md](LIVE.md). Firestorm validation remains pending. The shared `reliability` module supplies bounded resend accounting. Category token buckets gate resends, land and task sends; the rates come from the viewer's `AgentThrottle`. Message splitting and the rest of the `SendPacketFinal` policy remain unimplemented. A non-empty extra header is rejected rather than re-encoded.
