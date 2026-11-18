# Oracle fixtures

Bytes produced by `cargo xtask oracle generate` from the libopenmetaverse revision named in `manifest.json`. Do not hand-edit them. Regenerate with the xtask, which also checks that a second run is identical.

`manifest.json` is format 1:

| Field | Meaning |
| --- | --- |
| `libopenmetaverse` | Revision of the checkout that produced the bytes |
| `cases[].family` | `llsd`, `bytes` or `packet` |
| `cases[].format` | Which OpenMetaverse encoder (`xml`, `binary`, `notation`, `uuid`, `vector3`, …) |
| `cases[].file` | Payload path relative to this directory |
| `cases[].params` | Extra inputs the decoder needs, such as a quantization range, as hex float bits |
| `cases[].round_trip` | False when decoding these bytes and encoding again does not restore them |
| `cases[].decoded` | The value OpenMetaverse produced by reading the payload |

`decoded.type` is `undef`, `boolean`, `integer`, `real` (`bits` is the IEEE hex), `string`, `uuid`, `date`, `uri`, `binary`, `array`, `map` (pairs in wire order), or `decode-error` when the C# parser throws. A C# entry point that catches errors may instead return `undef`. Byte fixtures use `f32`, `f64`, `quaternion`, `color4`, `u16`, `u16x4` or `vector3-clamped`. Packet fixtures use `packet`, with `name`, `length` and `flags`.

LLSD cases cover undef, booleans, integers, reals (including negative zero, NaN and infinities), strings, UUIDs, dates, URIs, binary, empty and nested maps and arrays, in XML, binary and notation. `xml-formal`, `binary-raw` and `notation-header` are one nested map each, so the non-default switches are in the set. Byte cases cover UUID order, vectors, the 12-byte quaternion, quaternion shorts, colour bytes and `FloatToUInt16`.

XML binary cases also supply deliberate parser inputs rather than encoder output: malformed or unpadded base64, unsupported and case-sensitive encoding names, whitespace and padding edges, and explicit-empty versus self-closing elements. Their expected values come from the pinned C# decoder. The [CAPS fixtures](../caps/README.md) separately check the seed's HTTP responses for representative cases.

The integer `1234843`, the real `947835.234` and the UUID `97f4aeca-88a1-42a1-b385-b97b18abb255` are the vectors published in `OpenMetaverse.Tests/BinaryLLSDTests.cs`. The tool refuses to generate if the linked library does not reproduce them.

Packet files are one datagram per message in libopenmetaverse's template. The header is sequence 0, reliable, no extra and no acks, zero-coded when the template says so and the coded form is not larger. Field values and the messages that have no generated class are listed in [docs/PROTOCOL.md](../../docs/PROTOCOL.md). `params.has_class` says whether `ToBytes` was compared. `BuildPacket` is not part of `decoded`.

## What this revision does not round-trip

These are OpenMetaverse's results at the revision in `manifest.json`, not Ferroan choices. The payload is still what the encoder writes.

- Notation `NaN`, `Infinity` and `-Infinity` fail to parse. The notation reader accepts only a numeric token, and the exception text is `Notation LLSD parsing: Can't parse real value: `. Binary and XML NaN do round-trip. XML NaN comes back as bits `fff8000000000000`.
- XML `Infinity` and `-Infinity` throw inside `DeserializeLLSDXml`, which catches every error and returns undef. Binary infinities round-trip.
- A string containing a NUL becomes undef in XML. Notation stops at the NUL, so `a\0b` decodes as `a`. Binary keeps the NUL.
- `Vector3.ClampedToShortsBytes` for a vector outside the range does not come back from `UInt16ToFloat` followed by another clamp. The ushorts in the file are the wire value.
- Negative zero (`8000000000000000`) survives XML, binary and notation.
