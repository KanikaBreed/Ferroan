# Testing

## The local check

```sh
python3 scripts/check.py            # links, formatting, clippy, tests
python3 scripts/check.py --docs-only  # Markdown links only
```

On Windows, use `py -3`. CI runs the same script on Linux, Windows and macOS.

## Evidence by kind of change

| Change | Evidence expected |
| --- | --- |
| Parsing or encoding (packets, LLSD, XML, archives) | Tests against bytes captured from Tranquillity, plus malformed and oversized input. A round trip through our own encoder and decoder alone is not enough |
| A grid service | Captured-traffic fixtures, then a run behind a test grid with C# regions |
| Region behavior | A viewer check (Aurora scripted, Firestorm by hand), recorded with viewer version and steps |
| Database access | Tests on all three backends, on unchanged Tranquillity schemas |
| Native code (planned physics and scripting crates) | `SAFETY` comments on every unsafe block, and fuzzing of the boundary |

## Honest status

A compile check proves the code builds. It does not prove a viewer can log in, a script behaves as it did in C#, or a grid stays up. Record those as pending until someone has seen them work, with the revision, viewer or grid, and result.
