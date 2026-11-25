# Reference oracle

A .NET 10 console program that encodes and decodes fixtures with the pinned [libopenmetaverse](../../docs/REFERENCES.md) checkout. Rust tests use those bytes instead of checking an encoder against itself. The program is the C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own library. The oracle does not copy that checkout into the repository. It structurally validates `crates/ferroan-protocol/message_template.msg`: all libopenmetaverse headers and blocks must remain unchanged, with only eight explicitly defined viewer suffixes permitted.

LLSD and the scalar types come from `OpenMetaverse.StructuredData` and `OpenMetaverse.Types`. Texture entries, extra parameters and shape quantizers come from the `OpenMetaverse` project (`TextureEntry` and `Primitive`). `SculptData.Invert` and `SculptData.Mirror` have no setters; the oracle writes the private type byte and then calls `GetExtraParamsBytes`. Permission folding comes from Tranquillity `OpenSim.Framework`. The oracle checks that the shared `PermissionMask` bits match OpenMetaverse before it writes them.

`cargo xtask oracle` builds it. Build products go under `.scratch/reference-oracle/`, not into the reference checkout.

```sh
cargo xtask oracle build
cargo xtask oracle generate          # writes fixtures/oracle, then checks two runs match
cargo xtask oracle verify            # decodes the fixtures with C# and compares bytes
cargo xtask oracle verify some/dir   # same check for bytes Rust wrote in that shape
```

`FERROAN_REF` is the reference checkouts root. It defaults to `~/Git/reference` (`HOME` or `USERPROFILE`). The checkout's `libopenmetaverse` revision must match `docs/REFERENCES.md`, and the checkout must be clean before and after the build.

The tool forces `Utils.EnUsCulture` before it calls OpenMetaverse. Integer and date text follow that culture; reals already do. It refuses to run on a big-endian machine because the packed types are little-endian. The generated fixtures are committed. The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask oracle verify`; ordinary CI uses the committed bytes in Rust tests without building this tool.

`bytes/quaternion-batch.bin` is 4096 records of 40 bytes: x, y, z and w as little-endian floats, then `ToBytes`, then `GetBytes`. The inputs come from a fixed SplitMix64 seed. `LengthSquared` takes its `Sse41.DotProduct` branch only on x86-64 with SSE4.1, which is the reference ([D71](../../docs/DECISIONS.md)), so `generate` and `verify` refuse this case on a machine without it.

`verify` decodes each payload with OpenMetaverse and compares the value with `manifest.json`. It also encodes the same inputs again and fails if the file bytes differ. `--semantic` on the console program skips the byte compare and reports only field differences. A payload over 16 MiB is rejected.

Packet cases are family `packet`. One datagram is written for every message in the template (478). Fields are the deterministic values in [docs/PROTOCOL.md](../../docs/PROTOCOL.md). Where OpenMetaverse generated a packet class, that class's `ToBytes` plus Tranquillity's zero-coding must equal the generic encoder. `BuildPacket` is counted and not required: `TestMessage` and `ViewerStats` throw because a fixed-count block is left null. Messages with no class use the same encoder. The manifest records the name, length and flags. It does not call `BuildPacket`. `cargo xtask` passes `FERROAN_REF` into the process. The packet cases need that variable, and they need to be started with the repository as the working directory, which the xtask does.

Eight additional `packet-extension` cases contain the viewer tails. Their raw prefixes, before zero-coding, must equal the original libopenmetaverse encoding; all 387 original `ToBytes` comparisons remain required. Their manifests record viewer provenance, prefix length and appended block names. These are generated layout cases, not captures of C# server sends. `verify` also compares their parameter metadata with freshly generated evidence. Actual sender evidence is kept separately in [fixtures/trailing](../../fixtures/trailing/README.md).
