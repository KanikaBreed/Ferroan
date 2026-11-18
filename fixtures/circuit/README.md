# C# circuit packet evidence

These datagrams were captured from the local test region at Tranquillity
`858321883f5487a5d192d36ea9d7b90fa468310c`, using `cargo xtask live run login grid`.
The capture patch only records datagrams at the C# socket boundary. The source
checkout remains unchanged; the patch is applied to the harness's scratch copy.

Regenerate after a successful C# run:

```sh
cargo xtask live run login grid
cargo xtask live circuit-fixtures .scratch/live/grid/capture-udp.cap
```

The generator validates the original selected packets with the existing codec
before applying exactly the P0.10d `scrub_datagram` identity folding. Agent and
session UUIDs become UUIDs ending in `1` and `2`; the circuit code becomes `1`.
The region name, estate owner, region ID, generated cache ID, texture UUIDs,
flags, water height, terrain heights and all other configuration remain as
captured from this public test configuration. Original packet sequence numbers,
reliability/zero-coding flags and appended ACKs are preserved. No additional
normalization is used to make the Rust comparison pass.

`sequence.cap` contains the selected inbound and outbound evidence in original
order, with capture timestamps intact. `manifest.json` records each selected
packet's original record index, every outgoing packet's envelope (including
unrelated packet types omitted from `sequence.cap`), and the handshake settings.
The individual `.bin` files contain complete LLUDP datagrams:

- `packet-ack.bin`: immediate `UseCircuitCode` ACK, sequence 0, unreliable.
- `region-handshake.bin`: first normal outbound packet, sequence 1, reliable
  and zero-coded, with one RegionInfo4 block.
- `packet-ack-established.bin`: ordinary queued ACK, sequence 14, unreliable;
  established ACKs participate in the outbound sequence.
- `logout-reply.bin`: sequence 25, reliable and zero-coded, with one nil
  inventory item UUID.

The relevant source methods are `LLUDPServer.SendAckImmediate`,
`LLUDPServer.SendAcks`, `LLUDPServer.LogoutHandler`,
`LLClientView.SendRegionHandshake`, `LLClientView.SendLogoutPacket`, and
`LLClientView.CloseWithoutChecks`, under
`Source/OpenSim.Region.ClientStack.LindenUDP/` at the revision above. The
`LogoutReplyPacket` constructor in `OpenMetaverse/_Packets_.cs` at
`a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` sets reliable and zero-coded flags.
The C# close path flushes queued packets and shuts the client down without
waiting for a logout ACK.

Server serializer tests supply the captured configuration and captured envelope
sequence when checking full bytes. Circuit flow tests separately verify that
the immediate ACK does not advance the next normal sequence. The Rust region
does not yet emit all intervening C# scene packets, so its logout sequence is
not expected to equal 25 during an ordinary login session.
