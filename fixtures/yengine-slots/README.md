# YEngine global slots

`cases.json` is four ordinary scripts. `objects/` holds the `.yobj` the pinned YEngine compiler wrote for each one. `digest.json` is the SHA-256 of the global triples in those files: the case name, then each `globalVarNames` entry as its name, type array and index.

The scripts are two integers, a global after a function that has a local, a global with an initializer, and a `string` with a `key` after an integer. A `constant` and a script-defined static field are not here. The larger token corpus stays in `fixtures/yengine-tokens`. Its digest is unchanged.

`tools/yengine-tokens-oracle` compiled these files at Tranquillity Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4` on the runtime whose `FrameworkDescription` contains `10.0.14`. `cargo test -p ferroan-scripting` tokenizes each source, assigns slots, and checks them against `read_header` of the object file.
