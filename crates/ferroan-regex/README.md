# ferroan-regex

`parse` follows `RegexParser.Parse` for `RegexOptions.None` on the .NET 10.0.14 runtime ([D131](../../docs/DECISIONS.md)). A pattern that runtime accepts parses. A pattern it rejects produces the same `RegexParseException` text, including the UTF-16 offset.

The capture tree, the character-class set, matching, replacement and the 250 ms timeout are later rows. A null pattern is not representable: the script wrapper turns one into an empty pattern before it compiles.

Two Ferroan bounds are not .NET messages. `regex.pattern-chars` refuses a pattern longer than 65,536 UTF-16 code units before the walk starts. `regex.class-depth` allows 64 nested class subtractions and refuses the 65th. The outer class is not a subtraction. Both report through [docs/LIMITS.md](../../docs/LIMITS.md).

The files `src/lib.rs`, `src/parse.rs` and `src/names.rs` are ported from `RegexParser.cs` and `RegexCharClass.cs` at dotnet/runtime tag `v10.0.14`. They keep the MIT notice. The C# source is not copied. [NOTICE.md](../../NOTICE.md) records the origin.

[fixtures/regex-parse](../../fixtures/regex-parse/README.md) holds the corpus, the SHA-256 digest and one full message for each error kind. `tools/regex-oracle` runs `new Regex` on .NET 10.0.14 and writes those files. `cargo test -p ferroan-regex` rebuilds the canonical accept/reject text and checks the digest and the literal messages.
