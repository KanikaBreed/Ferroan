# LLUDP messages

Ferroan speaks the same datagrams as Tranquillity. This note records which message template the generator uses, every place that template differs from Aurora Viewer's, and how a datagram is framed.

## Which template

The generator reads `crates/ferroan-protocol/message_template.msg`. That file is a byte-for-byte copy of libopenmetaverse `data/message_template.msg` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` (478 messages). The copy has no added header: the reference oracle refuses to run when the two files differ, and the template parser treats `//` as a comment only after the first column of real tokens. Provenance is in this file, in [NOTICE.md](../NOTICE.md) and in the crate headers.

Aurora Viewer `crates/aurora-msg/message_template.msg` at `48cb76e830a13965ac080128bb874afedda86d69` has 483 messages. It is the Second Life viewer template. Ferroan does not generate from it. A region has to understand what Tranquillity understands, and Tranquillity links libopenmetaverse.

The codec is adapted from Aurora `crates/aurora-msg` at that revision. Where the two templates or the two encoders disagree, the bytes follow libopenmetaverse and Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`.

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

## Blocks Aurora adds

On eight shared messages Aurora appends blocks that libopenmetaverse does not have. The blocks already in libopenmetaverse are the same. Ferroan generates the libopenmetaverse layout and does not emit the extra blocks. A datagram that carries them does not re-encode to the same bytes: the decoder stops at the libopenmetaverse blocks, and the encoder writes only those.

| Message | Aurora blocks that libopenmetaverse lacks |
| --- | --- |
| `AlertMessage` | `AgentInfo` Variable: `AgentID` LLUUID |
| `AvatarAppearance` | `AttachmentBlock` Variable: `ID` LLUUID, `AttachmentPoint` U8 |
| `ChildAgentUpdate` | `AgentInventoryHost` Variable: `InventoryHost` Variable 1 |
| `ImprovedInstantMessage` | `EstateBlock` Single: `EstateID` U32. `MetaData` Variable: `Data` Variable 2 |
| `InviteGroupResponse` | `GroupData` Single: `GroupLimit` S32 |
| `RegionInfo` | `RegionInfo5` Variable: six chat-range and offset F32 fields and `ChatFlags` U32. `CombatSettings` Variable: `CombatFlags` U32, `OnDeath` U8, four F32 fields |
| `SimStatus` | `SimFlags` Single: `Flags` U64 |
| `TransferInventory` | `ValidationBlock` Single: `NeedsValidation` BOOL, `EstateID` U32 |

A trailing variable block whose count byte is absent decodes as zero blocks. That is how an older simulator omits `RegionHandshake`'s `RegionInfo4`, and the codec keeps it (`Reader::block_count`). OpenMetaverse throws if the count byte is missing. The fixtures always include the count. A truncated fixed field at the very end of the body is zero-filled up to 64 bytes (`Reader::take`); a short read earlier than that is an error. Complete fixtures do not hit the fill.

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

`reencode_datagram` follows the zerocode flag on the datagram, not the template bit. Tranquillity sets the flag on messages the template marks Unencoded: `LLClientView.SendViewerTime` sets `Header.Zerocoded` on `SimulatorViewerTimeMessage`, and `terseUpdateHeader` is `MSG_RELIABLE | MSG_ZEROCODED` for `ImprovedTerseObjectUpdate` with the comment "zero code is not as spec". A capture round-trip has to reproduce those bytes. Packets this crate sends still take the flag from the template.

The extra header sits before the message number. Tranquillity writes `extra_len` 0. Aurora's parser reads the message number first and then skips extra bytes after it. Ferroan skips extra before the number, which matches `Header.ToBytes` when the length is 0. `reencode_datagram` rejects a non-empty extra. A non-zero extra combined with zero-coding is not claimed: C# zero-codes everything after the 6-byte header, including the extra, and this crate zero-codes only the message number and body.

Appended acks stay in wire order. OpenMetaverse `Header.FromBytes` stores them reversed, so `FromBytes` then `ToBytes` swaps the ack bytes. Ferroan does not. Acks are a set for `Acknowledge`; the order does not change which packets are retired. Do not use that C# round trip as the ack oracle. The fixtures have no acks.

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

`LLQuaternion` uses `Quaternion.ToBytes` and `FromBytes(..., normalized: true)`, 12 bytes, not `GetBytes`. `ToBytes` writes `scale * X, Y, Z` with `scale = -1/sqrt(norm)` when W is negative and `1/sqrt(norm)` otherwise, when `norm > 1e-6 || norm < 0.9999`. The other branch writes three zeroes and is the non-finite path. `FromBytes` sets W to `sqrt(1 - x² - y² - z²)` when that sum is greater than `1e-6`, and 0 otherwise. Identity `(0, 0, 0, 1)` encodes as three zeroes. The bytes-family fixtures in `ferroan-types` still cover `GetBytes`. Packet fixtures use `ToBytes`.

## Fixtures

`cargo xtask oracle generate` writes one datagram per template message under `fixtures/oracle/packet/`. The family is `packet`, the format is `datagram`, and the id is `packet/{Name}`. Each body is filled the same way:

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

387 messages have an `OpenMetaverse.Packets.{Name}Packet` class. For each of those, the generic encoder's datagram equals `ToBytes` plus the same zero-coding. 385 of them also survive `Packet.BuildPacket`. Two do not, and that is an OpenMetaverse decoder bug, not a byte mismatch: the constructor allocates a fixed-count block as an array of nulls, and `FromBytes` fills the elements only when the length is not already the template count.

- `TestMessage.NeighborBlock` is `new NeighborBlockBlock[4]`.
- `ViewerStats.NetStats` is `new NetStatsBlock[2]`.

`ToBytes` still matches for both. The oracle counts the `BuildPacket` successes and does not require them. The decode recorded in the manifest checks the name and that the datagram is at least 7 bytes. It does not call `BuildPacket`.

The other 91 messages have no generated class. They use the same layout rules. The check that those rules match OpenMetaverse is the 387 class comparisons. The messages are `AddCircuitCode`, `NeighborList`, `AvatarTextureUpdate`, `SimulatorMapUpdate`, `SimulatorSetMap`, `SubscribeLoad`, `UnsubscribeLoad`, `SimulatorReady`, `SimulatorPresentAtLocation`, `SimulatorLoad`, `SimulatorShutdownRequest`, `RegionPresenceRequestByRegionID`, `RegionPresenceRequestByHandle`, `RegionPresenceResponse`, `UpdateSimulator`, `LogDwellTime`, `FeatureDisabled`, `LogFailedMoneyTransaction`, `UserReportInternal`, `SetSimStatusInDatabase`, `SetSimPresenceInDatabase`, `AvatarPickerRequestBackend`, `DirFindQueryBackend`, `DirPlacesQueryBackend`, `DirClassifiedQueryBackend`, `DirLandQueryBackend`, `DirPopularQueryBackend`, `GroupNoticeAdd`, `DataHomeLocationRequest`, `DataHomeLocationReply`, `NearestLandingRegionRequest`, `NearestLandingRegionReply`, `NearestLandingRegionUpdated`, `TeleportLandingStatusChanged`, `KickUserAck`, `SystemKickUser`, `AvatarPropertiesRequestBackend`, `RequestParcelTransfer`, `UpdateParcel`, `RemoveParcel`, `MergeParcel`, `LogParcelChanges`, `CheckParcelSales`, `ParcelSales`, `StartAuction`, `ConfirmAuctionStart`, `CompleteAuction`, `CancelAuction`, `CheckParcelAuctions`, `ParcelAuctions`, `ChatPass`, `EdgeDataPacket`, `SimStatus`, `AtomicPassObject`, `KillChildAgents`, `DataServerLogout`, `RequestInventoryAsset`, `InventoryAssetResponse`, `TransferInventory`, `TransferInventoryAck`, `FormFriendship`, `EventLocationRequest`, `EventLocationReply`, `MoneyTransferBackend`, `SetStartLocation`, `NetTest`, `SetCPURatio`, `SimCrashed`, `NameValuePair`, `RemoveNameValuePair`, `UpdateAttachment`, `RemoveAttachment`, `EmailMessageRequest`, `EmailMessageReply`, `InternalScriptMail`, `ScriptDataRequest`, `ScriptDataReply`, `InviteGroupResponse`, `TallyVotes`, `LogTextMessage`, `CreateTrustedCircuit`, `DenyTrustedCircuit`, `RequestTrustedCircuit`, `ParcelRename`, `SystemMessage`, `RpcChannelRequest`, `RpcChannelReply`, `RpcScriptRequestInbound`, `RpcScriptRequestInboundForward`, `RpcScriptReplyInbound` and `ScriptMailRegistration`.

`crates/ferroan-protocol/tests/oracle_fixtures.rs` calls `reencode_datagram` on every packet fixture and requires the same bytes, and requires 478 of them. `cargo xtask oracle verify` encodes each message again with the C# tool and compares. CI does not run the oracle.

Malformed datagrams return `DecodeError`. The unit tests cover a short header, an ack count that runs past the buffer, a zero-expansion bomb, and a zero run whose count is 0. `protocol-fuzz` reads stdin, truncates to 4096 bytes and calls `reencode_datagram`. `cargo xtask check` compiles it. It is not a coverage-guided fuzzer.

## Redaction

`scrub_datagram` is the capture scrub. It decodes one datagram, replaces session UUIDs with `00000000-0000-0000-0000-000000000002`, and replaces agent, user, owner, creator, abuser and reporter UUIDs, plus the circuit block's `ID`, with `00000000-0000-0000-0000-000000000001`. Circuit `Code` fields become 1. Every `IPADDR` becomes `127.0.0.1`. Variable fields named Name, FromName, FromAgentName, FirstName, LastName, UserName, NameValue or ObjectOwner, and variable fields whose names contain password, passwd, secret, token or email, become the bytes of `Redacted`. Ports, other object UUIDs and chat text stay. `SourceID` stays, because that field is an agent id on some messages and an object id on others. The datagram is then encoded again, so a redacted capture is a Ferroan encoding of the scrubbed fields. [CAPTURES.md](CAPTURES.md) says to prove the raw datagram round-trips before that replacement.

## Not claimed

No viewer has been shown a datagram from this crate. The comparison is against libopenmetaverse's encoder and Tranquillity's zero-coding, on Linux. A Windows oracle run was not done. Message splitting, resends, throttles and the reliable-ack policy of `SendPacketFinal` are not in this crate. A non-empty extra header is rejected rather than re-encoded.
