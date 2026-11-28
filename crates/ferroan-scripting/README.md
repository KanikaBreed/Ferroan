# ferroan-scripting

YEngine's token stream and the object-file header, from Tranquillity at the pin in [PORTING.md](../../docs/PORTING.md).

`tokenize` walks the script the way `MMRScriptTokenize.cs` does. `source_hash` is the MD5 of that stream: a NUL byte, then each token's text, uppercase hex. `TokenBegin` and `TokenEnd` are not hashed. Line numbers use wrapping addition, the same arithmetic as that file's unchecked `lineNo++`. `read_header` reads the `YObjectCode` prefix through the global name, type and index triples and stops. The prefix is UTF-8, which is what `new BinaryWriter(stream)` writes for a `char[]`. A version other than -11 is refused before the source hash is read. The error text is `Object version is {found} but accept only -11`. The reader does not read the CIL, assign slots, or run the script.

The script thread, Linden's SLua engine, and the `ll`, `os` and `bot` functions are not in this crate. There is no unsafe code and no native library.

Bounds are `yengine.source-chars`, `yengine.name-chars`, `yengine.string-chars`, `yengine.object-bytes` and `yengine.header-string` in [LIMITS.md](../../docs/LIMITS.md). The notices are in [NOTICE.md](../../NOTICE.md).

`fixtures/yengine-tokens` is produced by `tools/yengine-tokens-oracle`, which compiles the pinned YEngine. `cargo xtask yengine-tokens verify` prepares a clean pin, runs that oracle and compares `digest.json`, `manifest.json` and every `objects/*.yobj`. It is not part of `cargo xtask check`. `constant` and a script-defined static field are not in that fixture.
