# ferroan-scripting

YEngine's token stream, the object-file header and the global slot map, from Tranquillity at the pin in [PORTING.md](../../docs/PORTING.md).

`tokenize` walks the script the way `MMRScriptTokenize.cs` does. `source_hash` is the MD5 of that stream: a NUL byte, then each token's text, uppercase hex. `TokenBegin` and `TokenEnd` are not hashed. Line numbers use wrapping addition, the same arithmetic as that file's unchecked `lineNo++`. `read_header` reads the `YObjectCode` prefix through the global name, type and index triples and stops. The prefix is UTF-8, which is what `new BinaryWriter(stream)` writes for a `char[]`. A version other than -11 is refused before the source hash is read. The error text is `Object version is {found} but accept only -11`. The reader does not read the CIL or run the script.

`assign_global_slots` walks those tokens the way `MMRScriptCodeGen` walks `variablesStack`. A top-level `type name;` or `type name = ...;` takes the next slot of that type. A function, a property and a `constant` do not. A declaration inside braces does not. `string` and `key` share `iarStrings`. `quaternion` is already the token `rotation`, so it uses `iarRotations`. A script-defined static field would use the same slot call, and this corpus has none.

The script thread, Linden's SLua engine, and the `ll`, `os` and `bot` functions are not in this crate. There is no unsafe code and no native library.

Bounds are `yengine.source-chars`, `yengine.name-chars`, `yengine.string-chars`, `yengine.object-bytes` and `yengine.header-string` in [LIMITS.md](../../docs/LIMITS.md). The notices are in [NOTICE.md](../../NOTICE.md).

`fixtures/yengine-tokens` is produced by `tools/yengine-tokens-oracle`, which compiles the pinned YEngine. `cargo xtask yengine-tokens verify` prepares a clean pin, runs that oracle and compares `digest.json`, `manifest.json` and every `objects/*.yobj`. It is not part of `cargo xtask check`. `fixtures/yengine-slots` is a smaller compile from that same oracle: two integers, a global after a function, an initializer, and a `string` then a `key`. `constant` and a script-defined static field are not in either fixture.
