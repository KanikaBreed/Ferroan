# C# trailing-block capture

`improved-instant-message.bin` is the 191-byte reliable datagram captured from
Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c` during the preserved Q10
Ferrobot standalone scenario. `LLClientView.SendInstantMessage` writes the
187-byte libopenmetaverse prefix, then four bytes for `EstateBlock.EstateID = 0`.
It omits the `MetaData` count byte. This is a viewer template extension, not a
C# bug.

Pin3 (`40432e0f83d3802434f898a4e28687bd977d5cf5`) leaves the complete `SendInstantMessage` method unchanged. Pin4 (`f67db54399c027fd74e5aefcb2193905b4dc1646`) changes `LLClientView.cs` only in `Kick` throttle category and `PartsInLinkOrder` iteration; `SendInstantMessage` is not in that diff. This historical fixture retains its original capture revision and bytes; `audited_reference_revision` records the source audit, not a new capture. The selector accepts fresh traffic that carries the current pin's channel, so the historical raw capture cannot be passed off as Pin4 evidence.

Regenerate from the private raw UDP capture under `.scratch/`:

```sh
cargo xtask live trailing-fixtures <udp-capture>
```

The selector first checks every raw datagram for lossless re-encoding. It requires
a preceding arrival with the exact pinned C# channel/version for the IM recipient
and the harness's synthetic IM text marker. It then replaces identities,
timestamp and text directly in the captured bytes, preserving their lengths.
The manifest records these replacements and the selected record. No serializer
constructs the fixture. Raw captures and generated test credentials stay ignored.

`captured_csharp_im_keeps_estate_block_and_omits_metadata_count` checks the exact
captured framing, the four-byte suffix, the omitted metadata count and a complete
byte-for-byte re-encoding. The generated template extension fixtures under
`fixtures/oracle` are separate layout tests, not captured server traffic.
