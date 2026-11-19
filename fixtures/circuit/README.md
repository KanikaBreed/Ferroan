# C# circuit packet evidence

These datagrams were captured from the local test region at Tranquillity
`40432e0f83d3802434f898a4e28687bd977d5cf5`, using `cargo xtask live run login grid`.
The capture patch only records datagrams at the C# socket boundary. The source
checkout remains unchanged; the patch is applied to the harness's scratch copy.

Regenerate after a successful C# run:

```sh
cargo xtask live run login grid
cargo xtask live circuit-fixtures .scratch/live/grid/capture-udp.cap
```

The generator validates the original selected packets with the existing codec
and requires the matching login's exact pinned C# channel/version
before applying exactly the P0.10d `scrub_datagram` identity folding. Agent and
session UUIDs become UUIDs ending in `1` and `2`; the circuit code becomes `1`.
The region name, estate owner, region ID, generated cache ID, texture UUIDs,
flags, water height, terrain heights and all other configuration remain as
captured from this public test configuration. Original packet sequence numbers,
reliability/zero-coding flags and appended ACKs are preserved. No additional
normalization is used to make the Rust comparison pass.

`sequence.cap` contains the complete scrubbed UDP evidence in original
order, with capture timestamps intact. `manifest.json` records each selected
packet's original record index, every outgoing packet's envelope (including
unrelated packet types), and the handshake settings.
The individual `.bin` files contain complete LLUDP datagrams:

- `packet-ack.bin`: immediate `UseCircuitCode` ACK, sequence 0, unreliable.
- `region-handshake.bin`: first normal outbound packet, sequence 1, reliable
  and zero-coded, with one RegionInfo4 block.
- `packet-ack-established.bin`: ordinary queued ACK, sequence 8, unreliable;
  established ACKs participate in the outbound sequence.
- `logout-reply.bin`: sequence 25, reliable and zero-coded, with one nil
  inventory item UUID.
- `shutdown-kick-8583218.bin`: historical sequence 26, with the captured reason
  `The simulator is going down.`. This is the shutdown path's notice,
  not an ordinary logout response. [historical-shutdown.json](historical-shutdown.json)
  records its original `8583218` capture and the current source audit; it is
  absent from the new normal-login capture.

The fresh 54-datagram capture has no `KillObject`; the earlier selected trace's
manifest showed one after shutdown. `Scene.RemoveClient` broadcasts it for the
avatar's local ID. That scene/avatar behavior remains assigned to P2.0f.
The generator retains these tail packets when present and removes obsolete
optional tail files when a later capture has none.

The relevant source methods are `LLUDPServer.SendAckImmediate`,
`LLUDPServer.SendAcks`, `LLUDPServer.LogoutHandler`,
`LLClientView.SendRegionHandshake`, `LLClientView.SendLogoutPacket`, and
`LLClientView.CloseWithoutChecks`, under
`Source/OpenSim.Region.ClientStack.LindenUDP/` at the revision above. The
`LogoutReplyPacket` constructor in `OpenMetaverse/_Packets_.cs` at
`a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` sets reliable and zero-coded flags.
The C# close path does not wait for a logout ACK. `Scene.Close` sends the
captured kick reason when the simulator shuts down; ordinary
`Scene.CloseAgent(false)` does not call `Kick`.

Server tests supply the captured configuration. The initial ACK and handshake
match complete captured datagrams. Established ACK and logout tests drive all
captured inbound packets through the circuit, without seeding its counter or
ACK queue. They flush ACKs at the recorded batch boundary, compare every wire
byte except the four outbound sequence bytes, and independently assert Rust's
consecutive sequence allocation. The Rust region does not emit C#'s intervening
terrain/appearance/world packets yet, so its ACK/logout sequences are 3/4
rather than 8/25. Arrival's separate capture still matches the full datagram
after its documented identity and timestamp folding.
