# ferroan-scripting

YEngine's token stream and the object-file header, from Tranquillity at the pin in [PORTING.md](../../docs/PORTING.md).

`tokenize` walks the script the way `MMRScriptTokenize.cs` does. `source_hash` is the MD5 of that stream: a NUL byte, then each token's text, uppercase hex. `TokenBegin` and `TokenEnd` are not hashed. `read_header` reads the `YObjectCode` prefix through the global name, type and index triples and stops. The prefix is UTF-8, which is what `new BinaryWriter(stream)` writes for a `char[]`. It does not read the CIL, assign slots, or run the script.

The script thread, Linden's SLua engine, and the `ll`, `os` and `bot` functions are not in this crate. There is no unsafe code and no native library.

Bounds are `yengine.source-chars`, `yengine.name-chars`, `yengine.string-chars`, `yengine.object-bytes` and `yengine.header-string` in [LIMITS.md](../../docs/LIMITS.md). The notices are in [NOTICE.md](../../NOTICE.md).

`fixtures/yengine-tokens` is produced by `tools/yengine-tokens-oracle`, which compiles the pinned YEngine. `constant` and a script-defined static field are not in that fixture.
