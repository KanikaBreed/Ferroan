# ferroan-protocol

LLUDP messages and packet framing for the template Tranquillity speaks.

The generator and the packet codec were adapted from Aurora Viewer `crates/aurora-msg` at `48cb76e830a13965ac080128bb874afedda86d69`. `message_template.msg` keeps all 478 messages, headers and existing blocks from libopenmetaverse `data/message_template.msg` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`, and appends eight viewer layouts from `secondlife/viewer` `scripts/messages/message_template.msg` at `ff75ab037607d8fe305f43f8b1f43126693ba2db`. Framing follows OpenMetaverse `Header` and `Helpers.ZeroDecode` at that revision, and Tranquillity `LLUDPServer.ZeroEncode` and `SendPacketFinal` at `858321883f5487a5d192d36ea9d7b90fa468310c`.

[docs/PROTOCOL.md](../../docs/PROTOCOL.md) lists the template differences and audits the C# sender and reader for each added tail. Generated `omitted_trailing_blocks` metadata preserves missing Single blocks and absent Variable counts without turning them into explicit zero-count bytes. The five Aurora-only messages remain undefined.

`build.rs` emits encode and decode for every message. `scrub_datagram` replaces session UUIDs, agent, user, owner, creator, abuser and reporter UUIDs, circuit codes, every `IPADDR`, and variable fields named Name, FromName, FromAgentName, FirstName, LastName, UserName, NameValue or ObjectOwner. A variable field whose name contains password, passwd, secret, token or email is replaced the same way. The placeholders are fixed. Other object UUIDs stay. `SourceID` stays. Chat text stays. Quaternion fields use `Quaternion.ToBytes` / `FromBytes`, not `GetBytes` and not a glam normalize-and-flip. `IPADDR` is stored as the four little-endian wire bytes. `IPPORT` is big-endian.

## Fixtures

`tests/oracle_fixtures.rs` re-encodes every `family = packet` file under `fixtures/oracle` and requires the same bytes. There are 478 of them. `cargo xtask oracle generate` writes them and checks each class packet against `Packet.ToBytes`. `cargo xtask oracle verify` repeats that check in the [Linux live workflow](../../.github/workflows/live.yml). Ordinary CI runs the Rust fixture tests; fixture generation remains an explicit update step.

Eight `packet-extension` cases additionally check complete extended layouts and their unchanged raw libopenmetaverse prefixes. The separate [C# IM capture](../../fixtures/trailing/README.md) proves the actual server emits `EstateBlock` and omits the `MetaData` count; it re-encodes at exactly 191 bytes.

387 messages have an OpenMetaverse packet class and match `ToBytes`. `TestMessage` and `ViewerStats` then throw inside `BuildPacket` because a fixed-count block is left as nulls. The oracle does not require `BuildPacket`. The other 91 messages have no class. They use the same encoder, which the class matches justify.

## Limits

A datagram over 4096 bytes, or a zero-code expansion over 256 KiB, is `DecodeError::Malformed`. OpenMetaverse does not apply the expansion cap. Appended acks stop at 255 or when another ack would make the datagram reach `LLUDPServer.MTU` (1400). The framer appends the acks it is given. Tranquillity's send path usually does not append acks on a reliable or still-zerocoded packet. That policy belongs to the circuit.

`protocol-fuzz` reads one datagram from stdin and parses it. `cargo xtask check` compiles it.

## Not claimed

No viewer has been shown these datagrams. Message splitting (`ToBytesMultiple`, the 1200-byte variable-block size) is not implemented. A non-empty extra header is rejected on re-encode. Zero-coding with a non-empty extra is not claimed.
