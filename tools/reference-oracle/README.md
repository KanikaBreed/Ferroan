# Reference oracle

A .NET 10 console program that encodes and decodes fixtures with the pinned [libopenmetaverse](../../docs/REFERENCES.md) checkout. Rust tests use those bytes instead of checking an encoder against itself. The program is the C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own library. Nothing from that checkout is copied into this repository.

`cargo xtask oracle` builds it. Build products go under `.scratch/reference-oracle/`, not into the reference checkout.

```sh
cargo xtask oracle build
cargo xtask oracle generate          # writes fixtures/oracle, then checks two runs match
cargo xtask oracle verify            # decodes the fixtures with C# and compares bytes
cargo xtask oracle verify some/dir   # same check for bytes Rust wrote in that shape
```

`FERROAN_REF` is the reference checkouts root. It defaults to `~/Git/reference` (`HOME` or `USERPROFILE`). The checkout's `libopenmetaverse` revision must match `docs/REFERENCES.md`, and the checkout must be clean before and after the build.

The tool forces `Utils.EnUsCulture` before it calls OpenMetaverse. Integer and date text follow that culture; reals already do. It refuses to run on a big-endian machine because the packed types are little-endian. CI does not run this command. The generated fixtures are committed, and `cargo xtask oracle verify` stays a manual step until a Linux CI job is added.

`verify` decodes each payload with OpenMetaverse and compares the value with `manifest.json`. It also encodes the same inputs again and fails if the file bytes differ. `--semantic` on the console program skips the byte compare and reports only field differences. A payload over 16 MiB is rejected.
