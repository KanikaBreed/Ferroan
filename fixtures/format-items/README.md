# .NET composite-format outcomes

`cases.json` is the corpus. Each case is a format, or JSON `null` for a null format, and the string arguments. `digest.json` is the SHA-256 of the canonical text for that corpus. `literals.json` is one full exception message for each distinct error, and the rendered text of each short success. .NET 10.0.14 wrote both result files. The pin in the digest is Tranquillity Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`, whose build targets `net10.0`. The oracle refuses to run unless `FrameworkDescription` contains `10.0.14`. The harness culture is `en-US`. Item syntax does not use the culture.

A success is `OK`, then `R` and the UTF-16 length of `string.Format`'s result, then that result, then the segments. An error is `ERR`, the exception type name, and the message. Segments follow an index error only when `CompositeFormat` parsed the whole format and every index and alignment is in the range `AppendFormat` can store. A parse error has no segments. A segment is `L` and a literal, or `H` with the index, the signed alignment and either `-` or the specifier text. Lengths are UTF-16 code units. Empty literals around a hole are kept. `{0:}` has an empty specifier and `{0}` has none.

A format longer than 65,536 UTF-16 code units is a Ferroan limit. It is not in this digest. Widths of one million and more are not rendered here: `{0,1000000}` is passed with no arguments, so `string.Format` reports the index and does not pad.

Regenerate from this repository:

```
dotnet run --project tools/format-oracle -- fixtures/format-items/cases.json fixtures/format-items/digest.json fixtures/format-items/literals.json
```

[tools/format-oracle](../../tools/format-oracle/Program.cs) is new Ferroan code. It does not build Tranquillity. `cargo test -p ferroan-format` checks the committed digest and the literal messages against [ferroan-format](../../crates/ferroan-format/README.md).
