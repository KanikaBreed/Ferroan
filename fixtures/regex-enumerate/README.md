# What `Match.Groups` enumerates

`cases.json` is the corpus: a pattern and a subject that `Regex.Match` accepts. `digest.json` is the SHA-256 of the canonical text, and `literals.json` is every enumerated group plus a lookup of each declared number. .NET 10.0.14 wrote both result files. The pin in the digest is Tranquillity Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`, whose build targets `net10.0`. The oracle refuses to run unless `FrameworkDescription` contains `10.0.14`.

`numbers` and `names` are `Regex.GetGroupNumbers` and `Regex.GetGroupNames`. `enumerated` is `foreach (Group g in match.Groups)`, in that order. `Count` equals the slot count. An index that is not itself a declared number is an empty unsuccessful group. A declared number greater than or equal to `Count` is not in that walk, so `osMatchString` never reads it even when `Groups[number]` captured text. `declared` is that lookup. Its `name` is `Group.Name`. When the capture map is sparse, `GroupCollection` labels the cached group by its implementation slot, so `Group.Name` can be empty, or can be another slot's name, while `GetGroupNames` still has the declared name. `osMatchString` reads `Success`, `Value` and `Index`, not `Name`.

A canonical record is the pattern, the subject, the numbers joined with commas, the names joined with U+001F, then `E` and one block per enumerated group (`1` or `0`, `Index`, `Group.Name`, `Value`), then `D` and one block per declared number, then a line `.`. None of those fields contain a newline.

Regenerate from this repository:

```
dotnet run --project tools/regex-oracle -- --enumerate fixtures/regex-enumerate/cases.json fixtures/regex-enumerate/digest.json fixtures/regex-enumerate/literals.json
```

[tools/regex-oracle](../../tools/regex-oracle/Program.cs) is new Ferroan code. It does not build Tranquillity. `cargo test -p ferroan-regex` checks the committed digest, the declared slots and the captured rows against [ferroan-regex](../../crates/ferroan-regex/README.md). The matcher that would reproduce `Value` is a later row.
