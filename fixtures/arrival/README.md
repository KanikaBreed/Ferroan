# C# login arrival evidence

Captured from the local C# test grid at Tranquillity
`40432e0f83d3802434f898a4e28687bd977d5cf5`, using the ordinary login scenario.
The capture patch records the socket bytes in the scratch build; the reference
checkout is unchanged. Only the test account `Test User` is selected.

```sh
cargo xtask live run login grid
cargo xtask live arrival-fixtures .scratch/live/grid/capture-udp.cap
```

`agent-movement-complete.bin` is the complete captured arrival datagram.
`sequence.cap` contains its `UseCircuitCode`, `CompleteAgentMovement`, home
`RegionHandshake`, and arrival, in original order. `manifest.json` records their
original record indexes, envelopes and the decoded arrival fields.

The generator checks the original datagrams before redaction and requires a
matching agent, session and circuit code, the harness home region, the exact
pinned C# channel/version, and a reliable
unencoded arrival after the request. It applies the existing P0.10d identity
scrubber (agent/session UUIDs ending in 1/2, circuit code 1), then zeros only the
arrival's timestamp bytes at offsets 74–77. Capture times are relative to the
circuit open. Position, look-at, region handle, channel/version bytes, sequence,
flags and appended ACKs remain unchanged. Raw captures stay under `.scratch/`.

The captured arrival has sequence 2, flags `0x40`, no appended ACKs, position
`(128,128,25.96)`, look-at `(0.99,0.042,0)`, and handle `1099511628032000`.
Its channel/version is the UTF-8 string
`OpenSim-NGC Tranquillity Release 1.2.154-alpha+40432e0f83 (X64/Unix/DotNet)`
with a terminating NUL. The version is preserved as captured evidence, not
used as the Rust process's version. The C# ground-height adjustment explains
the captured Z; P2.0d2 uses the accepted login start position until P2.0e.

Source derivation at the pinned revision:

- [LLClientView.MoveAgentIntoRegion](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs)
  writes reliable, unencoded low-frequency packet 250: agent/session UUIDs,
  position (falling back to the circuit start position only for a zero vector),
  look-at, region handle, Unix seconds, and the cached channel/version bytes.
- [ScenePresence.CompleteMovement and MakeRootAgent](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs)
  normalize the horizontal camera look, fall back to velocity and then
  `(0.99,0.042,0)`, apply landing and ground adjustments, send the handshake,
  then call `MoveAgentIntoRegion`.
- [Scene constructor and GetSimulatorVersion](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.Framework/Scenes/Scene.cs)
  append the runtime description to the server version. `LLClientView` caches
  `Util.StringToBytes1024(scene.GetSimulatorVersion())` for the packet.

The actual server-send test supplies captured configuration and sequence,
normalizes only the emitted timestamp, and compares the whole datagram with
this fixture. The extractor tests additionally reject missing requests/replies
and mismatched session/region evidence. The remaining root-login terrain,
parcel, avatar, appearance, animation, attachment and friends messages belong
to their later tasks; this fixture makes no viewer-rendering claim.
