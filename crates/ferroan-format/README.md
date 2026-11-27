# ferroan-format

`parse` splits a composite format into literal text and holes the way `StringBuilder.AppendFormat` does on the .NET 10.0.14 runtime ([D131](../../docs/DECISIONS.md)). `accept` applies that walk's argument-count check in the same order. A format the runtime rejects produces the same `FormatException` text, including the UTF-16 offset. A null format is `None`, and the message is the `ArgumentNullException` text for parameter `format`.

`{{` is a literal `{` and `}}` is a literal `}`. A hole stores its index, its signed alignment and its format specifier as text. Positive alignment pads on the left and negative alignment pads on the right. The pad length is a count of UTF-16 code units. `{0:}` keeps an empty specifier, which is distinct from a hole with no colon. Numeric and date specifiers are stored and not applied. Those are later rows. The script call `osFormatString` is a later row.

One Ferroan bound is not a .NET message. `format.chars` refuses a format longer than 65,536 UTF-16 code units before the walk starts. It reports through [docs/LIMITS.md](../../docs/LIMITS.md).

The files `src/lib.rs` and `src/parse.rs` are ported from `StringBuilder.AppendFormat` and `CompositeFormat.TryParseLiterals` at dotnet/runtime tag `v10.0.14`. They keep the MIT notice. The C# source is not copied. [NOTICE.md](../../NOTICE.md) records the origin.

[fixtures/format-items](../../fixtures/format-items/README.md) holds the corpus, the SHA-256 digest and one full message for each distinct error. `tools/format-oracle` runs `string.Format` on .NET 10.0.14 and writes those files. `cargo test -p ferroan-format` rebuilds the canonical text and checks the digest and the literal messages.
