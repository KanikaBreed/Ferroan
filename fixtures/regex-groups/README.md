# .NET capture-group numbers

`patterns.json` is the corpus. `digest.json` is the SHA-256 of the canonical lines for that corpus, and `literals.json` is the group numbers and names for the named groups and a few unnamed patterns. .NET 10.0.14 wrote both result files. The pin in the digest is Tranquillity Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`, whose build targets `net10.0`. The oracle refuses to run unless `FrameworkDescription` contains `10.0.14`.

A canonical record is the pattern, then `Regex.GetGroupNumbers` joined with commas, then `Regex.GetGroupNames` joined with U+001F. That is the order `Match.Groups` enumerates, which is the order `osMatchString` walks. The parse tree (concatenation, alternation, quantifiers and captures) is checked beside this digest. It follows `ScanRegex` before `FinalOptimize`, so a lazy quantifier stays in the tree.

Regenerate from this repository:

```
dotnet run --project tools/regex-oracle -- --groups fixtures/regex-groups/patterns.json fixtures/regex-groups/digest.json fixtures/regex-groups/literals.json
```

[tools/regex-oracle](../../tools/regex-oracle/Program.cs) is new Ferroan code. It does not build Tranquillity. `cargo test -p ferroan-regex` checks the committed digest, the literal numbers and the parse tree against [ferroan-regex](../../crates/ferroan-regex/README.md).
