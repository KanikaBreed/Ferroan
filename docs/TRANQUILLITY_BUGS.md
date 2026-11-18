# Tranquillity bugs

Bugs found in the C# reference while porting or testing Ferroan. Each one gets a row here, so that a finding is not lost between a note, a fix in Tranquillity and a pin move.

## What happens to a bug

1. **Record it.** Whoever finds it, coder or maintainer, adds a row: the C# file and the pinned revision, what goes wrong, and how it was seen (a fixture, a harness run, a viewer). The coder also puts one line under "Found while working" in [WORK_QUEUE.md](../WORK_QUEUE.md).
2. **Decide what Ferroan does** ([AGENTS.md](../AGENTS.md) §1). If viewers or other servers depend on the behavior, Ferroan matches it until it is fixed upstream, and says so in the row. Otherwise Ferroan does what the fixed C# would do, and that choice is recorded as a decision in [DECISIONS.md](DECISIONS.md). Ferroan never changes a protocol, schema or file format to work around a bug.
3. **Fix it upstream.** The fix goes to Tranquillity as its own C# change (D7: bug fixes go into both). Ferroan's GPL code is never copied into Tranquillity ([PORTING.md](PORTING.md)). The row records the Tranquillity PR.
4. **Move the pin, then regenerate.** Once the fix is on Tranquillity `develop`, a pin move brings it in ([REFERENCES.md](REFERENCES.md)), and every oracle and fixture that touches the fixed code is regenerated against the fixed behavior. A fixture that captured the bug is replaced, not kept beside the new one. Ferroan code that matched the buggy behavior changes to match the fix in the same pin move, and the row moves to **Fixed** with the revision.

## Open

| # | Found | C# file (revision) | What goes wrong | Who depends on it | Ferroan does | Upstream |
| --- | --- | --- | --- | --- | --- | --- |
| TB1 | 17 Nov 2026, P0.9 asset oracle | `Source/OpenSim.Framework/AssetBase.cs` `AssetFlags` (`713857f`, unchanged at `8583218`) | `XmlSerializer` throws when an asset's flags hold a value with no named bit, such as 8. The named bits are 1, 2 and 4 | Not known. A stored `asset_flags` of 8 would make the asset service fail to serialize that asset | The oracle records the C# rejection, but current asset XML `flags_text` emits decimal text for unnamed bits. This discrepancy was found in Q10b; behavior correction awaits an asset task | Not reported yet |

TB4 was removed on 18 November 2026: the four trailing `ImprovedInstantMessage` bytes are the viewer-defined `EstateBlock`. The lossless capture failure was a gap in Ferroan's template, not a C# bug. Q10c/P0.10j corrects that gap; the source comparison and captured evidence are in [PROTOCOL.md](PROTOCOL.md). No upstream fix or pin move is needed.

## Fixed

| # | Found | C# file | What went wrong | Fixed in | Pin that brought it in | Regenerated |
| --- | --- | --- | --- | --- | --- | --- |
| TB2 | 17 Nov 2026, live harness | `Source/OpenSim.Data.SQLite/Resources/UserAccount.migrations` | SQLite stopped at version 3 without `DisplayName` and `NameChanged`, so creating a user failed | `bd0600003c` (Tranquillity PR #411) | `c85783e` (#17) | SQLite `UserAccount` fixture and migration catalog (#17) |
| TB3 | 18 Nov 2026, live harness | `Source/OpenSim.Framework.Console/CommandConsole.cs` | A console question with no input to answer it threw `NullReferenceException` | `158fb979aa` (Tranquillity PR #416) | `8583218` (#24) | No fixture captured it; the harness supplies its answers |
