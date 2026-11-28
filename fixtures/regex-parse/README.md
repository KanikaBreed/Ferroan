# .NET pattern-parse outcomes

`patterns.json` is the corpus. `digest.json` is the SHA-256 of the canonical lines for that corpus, and `literals.json` is one full `RegexParseException` message for each distinct detail, the full message of each nested-quantifier pattern, and a few patterns that parse. .NET 10.0.14 wrote both result files. The pin in the digest is Tranquillity Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`, whose build targets `net10.0`. The oracle refuses to run unless `FrameworkDescription` contains `10.0.14`.

A canonical line is `OK\n` when `new Regex(pattern)` returns, and `ERR\n` plus `RegexParseException.Message` plus a newline when it throws. The offset in that message is a UTF-16 code-unit index. A pattern longer than 65,536 UTF-16 code units, and a class subtraction nested 65 deep, are Ferroan limits. They are not in this digest. A second quantifier after `^`, `$`, `.`, a class, a group, `\w` or `\p{L}` is in the corpus, and so is `?*`. The digest checks each full message.

Regenerate from this repository:

```
dotnet run --project tools/regex-oracle -- fixtures/regex-parse/patterns.json fixtures/regex-parse/digest.json fixtures/regex-parse/literals.json
```

[tools/regex-oracle](../../tools/regex-oracle/Program.cs) is new Ferroan code. It does not build Tranquillity. `cargo test -p ferroan-regex` checks the committed digest and the literal messages against [ferroan-regex](../../crates/ferroan-regex/README.md).
