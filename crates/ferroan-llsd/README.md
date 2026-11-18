# ferroan-llsd

LLSD in XML, binary and notation, matching OpenMetaverse's encoder.

The value type and the three codecs were adapted from Aurora Viewer `crates/aurora-llsd` at `48cb76e830a13965ac080128bb874afedda86d69`. Aurora's crate is itself a port of the Second Life viewer's `llcommon/llsd.cpp` and `llsdserialize*.cpp` (originally LGPL 2.1). Where Aurora and OpenMetaverse disagree, this crate follows OpenMetaverse, because that is what Tranquillity sends and reads. Each difference is listed below.

The C# behaviour comes from libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`:

- `OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs`
- `OpenMetaverse.StructuredData/LLSD/BinaryLLSD.cs`
- `OpenMetaverse.StructuredData/LLSD/NotationLLSD.cs`
- `OSDReal.AsString` and `OSDDate.AsString` in `StructuredData.cs`
- `osUTF8.AppendASCII` and the XML constants in `OSUTF8Constants.cs`

`tests/oracle_fixtures.rs` decodes every `family = llsd` fixture under `fixtures/oracle` and, where the fixture says the value round-trips, encodes it back to those bytes. Non-finite reals and a string that contains NUL are compared from the known source value even when OpenMetaverse does not round-trip them. Ordinary CI runs those Rust tests. The [Linux live workflow](../../.github/workflows/live.yml) supplies the pinned references and .NET SDK and runs `cargo xtask oracle verify` to compare the fixtures with fresh C# output.

## Encodings

| Call | OpenMetaverse |
| --- | --- |
| `to_xml` / `to_xml_string` | `SerializeLLSDXmlToBytes(value, false)`: `<llsd>` and the value, no XML declaration |
| `to_xml_formal` | `SerializeLLSDXmlToBytes(value, true)`: `<?xml version="1.0" encoding="utf-8"?><llsd>` and `encoding="base64"` on binary |
| `to_serialized_xml` | Viewer envelope `<? LLSD/XML ?>\n<llsd>…</llsd>\n`. Inventory buckets use this. HTTP uses `to_xml` |
| `to_pretty_xml_string` | Indented viewer form, one element per line |
| `to_binary` | `SerializeLLSDBinary(value, false)`: the value only |
| `to_binary_framed` | `SerializeLLSDBinary(value, true)`: `<?llsd/binary?>\n` then the value |
| `to_notation` | Notation of the value, no header. A notation-header fixture prepends the 19 bytes `<? llsd/notation ?>` |

Binary tags are `!`, `1`/`0`, `i` plus a big-endian i32, `r` plus a big-endian f64, `d` plus a little-endian f64 of seconds, `u` plus 16 bytes, `s`/`l`/`b` plus a big-endian length, `[` count items `]`, and `{` count then `k` plus a length and the key bytes. Map order is insertion order.

## Differences from Aurora, kept to match OpenMetaverse

- Maps are an `IndexMap`. Aurora sorted keys with a `BTreeMap`, which re-encodes a map in a different order from the C# bytes. `IndexMap` equality cares about order.
- Empty maps and arrays are `<map></map>` and `<array></array>`. Undef is `<undef/>`. Informal binary is `<binary>` with no encoding attribute.
- XML binary parsing follows the C# oracle's malformed-input cases: unsupported encodings, invalid base64 and missing required padding are rejected. A self-closing `<binary/>` becomes empty binary before C# checks the encoding attribute; an explicit empty element with an unsupported encoding is rejected. The seed HTTP fixtures preserve this distinction in its 200/400 responses.
- XML map keys are written as raw text. They are not escaped.
- Booleans on the notation wire are `t` and `f`. The parser still accepts `1`, `0`, `true`, `TRUE`, `false` and `FALSE`.
- Notation strings and keys escape only the quote and the backslash. Newlines, tabs and `\xHH` are not escaped. A URI is `l"..."` with the text unescaped.
- A notation string stops at a NUL and returns the text so far, without an error. `'a\0b'` is the string `a`. Binary keeps the NUL. XML turns a string that contains a NUL into undef, because the XML reader throws and `DeserializeLLSDXml` returns undef.
- Notation rejects a non-finite real. `rNaN` and `r∞` are errors. XML `Infinity` and `-Infinity` become undef. XML `NaN` is the .NET quiet NaN, bits `fff8000000000000`.
- Real text is `double.ToString("g")` under `new CultureInfo("en-us", false)`. The fixtures were produced on Linux with ICU: infinity is U+221E, not the word `Infinity`, negative zero is `-0`, and the exponent is `e+` or `e-` with at least two digits. XML writes reals and dates through `AppendASCII`, which keeps the low 8 bits of each character, so U+221E becomes the single byte `0x1E`. `from_xml` returns undef when any byte below `0x20` other than tab, LF or CR is present, which is how that infinity byte comes back. A Windows NLS build of .NET may print `Infinity` instead. `cargo xtask oracle verify` on Windows can therefore disagree on the infinity fixtures. The committed bytes and CI oracle verification use Linux; ordinary Windows and macOS CI tests read those same committed bytes without rerunning C#.
- A binary header may be `<?llsd/binary?>` or `<? llsd/binary ?>`, ASCII case-insensitive, after leading whitespace, or there may be no header. `FindString` reads one byte past a match and steps back. When the buffer ends on the header, the last byte stays unconsumed and the value does not parse. Framed fixtures have a newline after the header, so that byte is whitespace and is skipped.
- Notation with a leading `<` skips 19 bytes and does not check that they are `<? llsd/notation ?>`.
- Dates are stored as `i64` ticks of 100 ns since the Unix epoch. One `f64` cannot hold both the XML text and the binary result. Binary seconds become ticks the way `DateTime.AddSeconds` does: the whole seconds, then the fraction truncated toward zero, not rounded. XML and notation keep the fraction written in the text, so `2020-01-02T03:04:05.12Z` stays 120 ms and re-encodes as `.12`. The same instant stored as binary seconds comes back as 119 ms and prints `.11`. `AsString` prints no fraction when the millisecond count is 0, otherwise hundredths cut off rather than rounded. A tick count that does not fit a UTC timestamp is written as `1970-01-01T00:00:00Z`.
- Binary dates go through `ToLocalTime` and back in OpenMetaverse. On the machine that generated the fixtures that was a tick identity, and the manifest strings are UTC with `Z`. That identity was not checked in another time zone.

## Limits

A binary or notation parse may build at most 16 estimated bytes of tree per input byte, plus a 64 KiB floor, and never more than 128 MiB (`MAX_PARSED_BYTES`). `from_binary_local` keeps the per-byte rule and drops the fixed cap, for files the viewer wrote itself. An `IndexMap` is larger than Aurora's sorted map: an inventory-shaped document is about 9 estimated bytes of tree per input byte, still under the allowance of 16. Depth stops at 128 for binary and notation and at 256 for XML. Past a limit the parse returns `LlsdError::TooLarge` or `LlsdError::Invalid`. It does not panic.

`llsd-fuzz` reads stdin and parses it as XML, binary and notation. It is a compile check (`cargo build -p ferroan-llsd --bin llsd-fuzz`), not a coverage-guided fuzzer.

## Not claimed

The fixture comparison is against libopenmetaverse's encoder on the Linux ICU runtime. The live CAPS probes also exercise this crate's LLSD through C# and Rust region HTTP endpoints. Viewer-gate evidence is recorded separately in [TASKS.md](../../TASKS.md).
