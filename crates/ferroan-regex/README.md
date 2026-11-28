# ferroan-regex

`parse` follows `RegexParser.Parse` for `RegexOptions.None` on the .NET 10.0.14 runtime ([D131](../../docs/DECISIONS.md)). A pattern that runtime accepts parses. A pattern it rejects produces the same `RegexParseException` text, including the UTF-16 offset.

A pattern that parses carries concatenation, alternation, quantifiers and capturing groups. Group numbers and names are the sequence `Regex.GetGroupNumbers` and `Regex.GetGroupNames` return, which is the sequence `Match.Groups` yields and `osMatchString` walks. Group 0 is the whole pattern. A `{0}` quantifier drops its child from the tree and keeps the slot. The character-class set, backreferences, lookaround matching, replacement and the 250 ms timeout are later rows. A null pattern is not representable: the script wrapper turns one into an empty pattern before it compiles.

Two Ferroan bounds are not .NET messages. `regex.pattern-chars` refuses a pattern longer than 65,536 UTF-16 code units before the walk starts. `regex.class-depth` allows 64 nested class subtractions and refuses the 65th. The outer class is not a subtraction. Both report through [docs/LIMITS.md](../../docs/LIMITS.md).

The files `src/lib.rs`, `src/parse.rs` and `src/names.rs` are ported from `RegexParser.cs` and `RegexCharClass.cs` at dotnet/runtime tag `v10.0.14`. They keep the MIT notice. The C# source is not copied. [NOTICE.md](../../NOTICE.md) records the origin.

[fixtures/regex-parse](../../fixtures/regex-parse/README.md) holds the corpus, the SHA-256 digest and one full message for each error kind. [fixtures/regex-groups](../../fixtures/regex-groups/README.md) holds the group numbers and names. `tools/regex-oracle` runs `new Regex` on .NET 10.0.14 and writes those files. `cargo test -p ferroan-regex` rebuilds both canonical texts and checks the digests, the literal messages and the parse tree.
