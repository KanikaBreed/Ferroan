# ferroan-protocol

LLUDP messages and packet framing for the template Tranquillity speaks.

The generator and the packet codec were adapted from Aurora Viewer `crates/aurora-msg` at `48cb76e830a13965ac080128bb874afedda86d69`. The template is not Aurora's. `message_template.msg` is a byte-for-byte copy of libopenmetaverse `data/message_template.msg` at `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` (478 messages). Framing follows OpenMetaverse `Header` and `Helpers.ZeroDecode` at that revision, and Tranquillity `LLUDPServer.ZeroEncode` and `SendPacketFinal` at `713857f10b8e91a639a967d4354a4682b080793c`.

[docs/PROTOCOL.md](../../docs/PROTOCOL.md) lists every message that differs between the two templates and the choice made for each. Ferroan generates the libopenmetaverse layout. It does not define Aurora's five extra messages and does not emit Aurora's extra trailing blocks.

`build.rs` emits encode and decode for every message. `scrub_datagram` replaces session UUIDs, agent, user, owner, creator, abuser and reporter UUIDs, circuit codes, every `IPADDR`, and variable fields named Name, FromName, FromAgentName, FirstName, LastName, UserName, NameValue or ObjectOwner. A variable field whose name contains password, passwd, secret, token or email is replaced the same way. The placeholders are fixed. Other object UUIDs stay. `SourceID` stays. Chat text stays. Quaternion fields use `Quaternion.ToBytes` / `FromBytes`, not `GetBytes` and not a glam normalize-and-flip. `IPADDR` is stored as the four little-endian wire bytes. `IPPORT` is big-endian.

## Fixtures

`tests/oracle_fixtures.rs` re-encodes every `family = packet` file under `fixtures/oracle` and requires the same bytes. There are 478 of them. `cargo xtask oracle generate` writes them and checks each class packet against `Packet.ToBytes`. `cargo xtask oracle verify` repeats that check in the [Linux live workflow](../../.github/workflows/live.yml). Ordinary CI runs the Rust fixture tests; fixture generation remains an explicit update step.

387 messages have an OpenMetaverse packet class and match `ToBytes`. `TestMessage` and `ViewerStats` then throw inside `BuildPacket` because a fixed-count block is left as nulls. The oracle does not require `BuildPacket`. The other 91 messages have no class. They use the same encoder, which the class matches justify.

## Limits

A datagram over 4096 bytes, or a zero-code expansion over 256 KiB, is `DecodeError::Malformed`. OpenMetaverse does not apply the expansion cap. Appended acks stop at 255 or when another ack would make the datagram reach `LLUDPServer.MTU` (1400). The framer appends the acks it is given. Tranquillity's send path usually does not append acks on a reliable or still-zerocoded packet. That policy belongs to the circuit.

`protocol-fuzz` reads one datagram from stdin and parses it. `cargo xtask check` compiles it.

## Not claimed

No viewer has been shown these datagrams. Message splitting (`ToBytesMultiple`, the 1200-byte variable-block size) is not implemented. A non-empty extra header is rejected on re-encode. Zero-coding with a non-empty extra is not claimed.
