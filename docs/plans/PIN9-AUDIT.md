# C# history since Pin8

Audit date 2026-11-26. This is a planning note for [AGENTS.md](../../AGENTS.md) section 10. It does not move the pin, and it records no decision.

Pin8 is [KanikaBreed/OpenSim-Tranquillity](https://github.com/KanikaBreed/OpenSim-Tranquillity) `dev-build` `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` (2026-11-23, tag `ferroan-pin8`), as [REFERENCES.md](../REFERENCES.md) records it. Under [D21](../DECISIONS.md) that commit is OpenSim-NGC `develop` `923a4cd70b3794b82376f7c28d8619a08ca03af7` plus the 29 carried pull requests (#499 and #503 through #530), one merge each. The pin is the tip of the fork's `dev-build`. It is not an ancestor of OpenSim-NGC `develop`. Their merge base is `923a4cd70b`.

The history below was read from a fresh clone of the fork, with [OpenSim-NGC/OpenSim-Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) `develop` fetched beside it. The read-only reference checkout was not used.

## Range

| Branch | Tip | Commits after `c401bceecd` |
| --- | --- | --- |
| Fork `dev-build` | `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` (2026-11-23) | 0. The pin is the tip. |
| Fork `develop` | `578db569982a89935a0bbc20ee2934cfa6626b46` (2026-11-17, "Merge pull request #407") | 0. This tip is an ancestor of the pin. |
| OpenSim-NGC `develop` | `444008b48dca1189734b0262daa85401fc788f9f` (2026-11-25) | 3. They sit on `923a4cd70b`, beside the pin, and are not descendants of it. |

`c401bceecd` is 150 commits ahead of OpenSim-NGC `develop`. That set is the D21 carry already named in [REFERENCES.md](../REFERENCES.md). Replacing the pin with OpenSim-NGC `develop` would drop it.

## Commits

Newest first. "Class" says what the commit is on its own parent. The behaviour section says what that means against the pin.

| Commit | Date | Branch | Subject | Class | Ferroan |
| --- | --- | --- | --- | --- | --- |
| `444008b48dca1189734b0262daa85401fc788f9f` | 2026-11-25 13:59 -0500 | OpenSim-NGC `develop` | Merge pull request #547 from OpenSim-NGC/feature/fix-warnings | Merge of the two commits below. Parents `923a4cd70b` and `a225f558`. No change of its own. | Do not take this tip as the pin. |
| `a225f5581bebd46c76056e49a542292342a002d1` | 2026-11-25 13:31 -0500 | OpenSim-NGC `develop` | Move AGENTS.md in tests to Docs/TESTING.md | Docs move. `Tests/AGENTS.md` becomes `Docs/TESTING.md` (65 lines, same text). | Nothing. |
| `a40091ea5aec1e17b9c2821e92c82b2fae65c036` | 2026-11-25 13:27 -0500 | OpenSim-NGC `develop` | Updates to reference dlls within dotnet 10 scope. New AGENTS.md for toplevel and for testing. | Package version edits, one log sentence, analyzer edits in four tests, and an agent-instructions insertion. Parent is `923a4cd70b`. | Nothing to port. Do not take the Jolt lines or the appearance sentence. See below. |
| (none) | | Fork `dev-build` | | No commit after the pin. | Stay on Pin8. |
| (none) | | Fork `develop` | | No commit after the pin. | Nothing. |

`a40091ea` touches 111 files: `AGENTS.md`, `Source/OpenSim.Services.AvatarService/AppearanceService.cs`, four test `.cs` files, and package versions in `.csproj` files. No `.csproj` hunk changes anything other than a `PackageReference` version. `a225f558` touches two markdown paths. The merge adds `Docs/TESTING.md` on top of those.

## What changes behaviour

No commit in this range changes a wire format, a database schema, or an archive format. No commit is a new feature in a frozen phase ([D7](../DECISIONS.md)). The items below are the only hunks that could be read as behaviour. Each one is already covered by the pin, or it is an unfinished package edit.

### Appearance warning (`a40091ea`)

`Source/OpenSim.Services.AvatarService/AppearanceService.cs:66` on `444008b` changes the warning sentence from "remove AssetService from [{Section}]" to "remove the AssetService override". The condition at line 63 and the three logging arguments at line 67 stay as they are on `923a4cd70b`.

On that parent the sentence contains `{Section}` twice and the call passes three values. `Microsoft.Extensions.Logging` fills placeholders by position, so formatting throws `FormatException` and the service fails to start. Pin8 already fixed that in a different way. Carried pull request #515 (`c4b141fcdae6be449896c74485dbefdf19d108fb`, merged as `95d2205ef8`) keeps the original sentence and passes the section name twice, at `AppearanceService.cs:63-69` on `c401bceecd`. `Tests/OpenSim.Region.OptionalModules.ServerSideBaking.Tests/AppearanceServiceAssetMismatchTests.cs:52` asserts that full sentence, including "remove AssetService from [AppearanceService]".

Against the pin, the upstream sentence is an alternate wording of a fix the pin already has. It is not a second bug fix, and it does not change bake bytes or the asset service the process opens.

A trial merge of `c401bceecd` with `444008b` conflicts only in this file. The two test projects both sides edit (`Tests/OpenSim.Region.CoreModules.Tests/OpenSim.Region.CoreModules.Tests.csproj` and `Tests/OpenSim.Server.Handlers.Tests/OpenSim.Server.Handlers.Tests.csproj`) merge on their own: the pin adds `ProjectReference` lines, and `a40091ea` bumps package versions elsewhere in the same files. Resolving the appearance conflict by keeping the pin's two placeholders and taking upstream's three arguments would bring the `FormatException` back. Keep #515's call.

### Jolt package lines (`a40091ea`)

`Source/OpenSim.Region.PhysicsModules.Jolt.Backend/OpenSim.Region.PhysicsModules.Jolt.Backend.csproj:33` sets `JoltPhysicsSharp` to `2.26.0` (parent `2.19.1`). Line 38 sets `JoltPhysics.Native` to `1.3.0` (parent `1.0.4`). The comments on lines 31-36 still say the binding is `2.19.1` and the native is `1.0.4`.

The loader record was not updated:

- `Source/OpenSim.Region.PhysicsModules.Jolt.Backend/JoltNative.cs:128` still has `PackageVersion = "1.0.4"`.
- `JoltNative.cs:265-270` refuses a file whose hash is not that record, unless an unrecorded native is allowed.
- `Tests/OpenSim.Region.PhysicsModules.Jolt.Backend.Tests/JoltNativePackageTests.cs:64-65` fails when the restored package version is not `PackageVersion`, and lines 73-74 fail when a restored file's SHA-256 is not the recorded hash.
- `Source/OpenSim.Region.PhysicsModules.Jolt.Backend/assert-joltc-native.ps1:45` still sets `$stockVersion = "1.0.4"`.
- `Docs/JoltcNative.md:52-56` and `Docs/JoltPhysics.md:24-25` still name `1.0.4` and `2.19.1`.

No Jolt source file, hash, or baseline is in the diff. This audit did not restore the package and did not run the native. The version edit is unfinished: the project file names `1.3.0` and the record still names `1.0.4`. It is not a completed physics change, and it is not a Ferroan port.

[TASKS.md](../../TASKS.md) P3.0c is the planned C# move, and it is a different one: `JoltPhysicsSharp` 2.22 or later on the 5.6 generation whose joltc matches `oxijolt-sys` 1.6.0 (joltc `886e0886`, Jolt `e77f1755`). That row says a 5.7 package is the wrong generation. [PHASE3-PHYSICS.md](PHASE3-PHYSICS.md) records `JoltPhysicsSharp` 2.26.0 (19 November 2026, Native 1.3.0) and says the newest C# packages are probably 5.7, because joltc moved to Jolt 5.7.0 on 16 November 2026. P3.0c is still TODO. `a40091ea` does not do that row. P3.1 is already done and binds Ferroan's own `oxijolt-sys` 1.6.0 ([D8](../DECISIONS.md)); it does not read `JoltPhysics.Native`.

### Other package versions (`a40091ea`)

These are dependency version edits in `.csproj` files. Tranquillity's own logic is unchanged. Ferroan does not reference these packages (database access is sqlx, [D14](../DECISIONS.md); images and mail are not these libraries).

| Package | On `923a4cd70b` | On `444008b` | Projects |
| --- | --- | --- | --- |
| `SkiaSharp` | 4.153.1 | 4.157.0 | 12 |
| `SkiaSharp.NativeAssets.Linux` | 4.153.1 | 4.157.0 | 10 |
| `SkiaSharp.NativeAssets.Win32` | 4.153.1 | 4.157.0 | 2 |
| `CoreJ2K.Skia` | 2.4.1.108 | 2.4.2.112 | 10 |
| `Npgsql` | 10.0.3 | 10.0.4 | 2 |
| `MediatR` | 14.2.0 | 14.3.0 | 1 (`OpenSim.Data.Model`) |
| `Microsoft.EntityFrameworkCore` and the matching Design, Tools, and AspNetCore Identity packages | 10.0.13 | 10.0.15 | 1 |
| `Microting.EntityFrameworkCore.MySql` | 10.0.13 | 10.0.14 | 1 |
| `MailKit` | 4.18.1 | 4.18.2 | 1 |
| `System.CommandLine` | 2.0.13 | 2.0.15 | 5 |
| `Microsoft.Extensions.*` and `System.Configuration.ConfigurationManager`, `System.Runtime.Caching`, `System.Security.Permissions` | 10.0.13 | 10.0.15 | most projects |
| `Microsoft.Extensions.Logging.Console` | 9.0.7, and 10.0.10 in one project | 9.0.23, and 10.0.15 | those two projects, plus the 10.0.13 to 10.0.15 set |
| `Microsoft.NET.Test.Sdk` | 18.11.0 (two projects 17.14.1, one 18.8.1) | 18.12.0 (the 17.14 line goes to 17.14.2) | test projects |
| `NUnit` | 4.3.2 | 4.6.1 | 5 |
| `NUnit.Analyzers` | 4.9.2 | 4.15.0 | 5 |
| `NUnit3TestAdapter` | 5.0.0 | 5.2.0 | 5 |
| `coverlet.collector` | 10.1.0 | 10.1.1 | 7 |

`MySqlConnector` stays 2.6.2. `BouncyCastle.Cryptography` stays 2.7.0. `xunit` stays 2.9.3 where this diff shows it.

Skia and CoreJ2K are the image libraries the C# map and JPEG paths use. A later capture against a tree that includes these versions could differ in image bytes. This audit did not capture, and no expected byte in the diff changed. That is not a fixture regeneration under [AGENTS.md](../../AGENTS.md) section 1.

### Tests (`a40091ea`)

Four test files change so the .NET 10 analyzers accept them. The assertions still mean the same thing.

- `Tests/InWorldz.Phlox.Tests/LocalForceFlagTests.cs:32` adds `new` on `Force` and `Torque` so the fields hide the base properties. The force and torque asserts are unchanged.
- `Tests/InWorldz.Phlox.Tests/StateWriteRetryTests.cs:98` makes the discard test `async`. Lines 123 and 133 wait with `await` on the same 30 second conditions. Lines 126-127 and 134-135 still require the row to exist while the removal is in flight, and to be gone afterwards.
- `Tests/InWorldz.Phlox.Tests/YEngineOsslSettingsTests.cs:78` makes the two-harness test `async`. Lines 89 and 92 use `Task.WhenAny` with the same 4 second and 60 second bounds. Line 93 still expects `first down`, then `second up`.
- `Tests/OpenSim.Region.ScriptEngine.Tests/Shared/Tests/OSSL_ApiAttachmentTests.cs:50-51` marks the scene fields `null!`. Lines 110, 213 and 227 use `Assert.Single` where the count was 1. Lines 168, 172, 209 and 224 use `Assert.Empty` where the count was 0.

These are Tranquillity tests. They are not Ferroan fixtures, and they do not change a fixture's meaning.

### Docs (`a40091ea` and `a225f558`)

`a40091ea` inserts a generic .NET 10 agent guide at the top of `AGENTS.md` and renumbers the following headings. `a225f558` moves `Tests/AGENTS.md` to `Docs/TESTING.md`. Neither file is a Ferroan source.

## Ferroan areas

| Upstream hunk | Ferroan area that already has this | What Ferroan does |
| --- | --- | --- |
| Appearance warning sentence | Pin8 #515, described in [REFERENCES.md](../REFERENCES.md). No Rust copy of the sentence. | Nothing. The pin already logs the warning and starts. Keep #515's text and the fourth argument on any rebuild. |
| `JoltPhysicsSharp` 2.26.0 and `JoltPhysics.Native` 1.3.0 | P3.1 binds `oxijolt-sys` 1.6.0+jolt-5.6.0 ([D8](../DECISIONS.md), [D9](../DECISIONS.md)). P3.0c is the unfinished C# move to a 5.6-generation Sharp, not to this package pair. | Nothing in `ferroan-physics`. Do not treat `a40091ea` as P3.0c. P3.0d, the pin move that regenerates 5.4-native fixtures, still waits on P3.0c. |
| Skia, CoreJ2K, Npgsql, EF Core, MediatR, MailKit, and the `Microsoft.Extensions` / test-SDK bumps | No crate references these packages. Map drawing on the pin is carried pull request #510, already in Pin8. | Nothing to port. A future capture that includes the new Skia and CoreJ2K versions is the time to look at image bytes. |
| Phlox, YEngine, and OSSL attachment tests | Script work is later, and these edits do not change script results. | Nothing. |
| `AGENTS.md` and `Docs/TESTING.md` | Ferroan's agent instructions are its own. | Nothing. |

## TB rows

No commit in this range fixes a row in [TRANQUILLITY_BUGS.md](../TRANQUILLITY_BUGS.md). [AGENTS.md](../../AGENTS.md) section 1 therefore does not require a fixture regeneration for this range.

TB58 still names [pull request #542](https://github.com/OpenSim-NGC/OpenSim-Tranquillity/pull/542) as outside Pin8. #542 is not one of these three commits, and it is not on `dev-build` after `c401bceecd`. The appearance `FormatException` that #515 fixed is not a TB row. TB41 is a different `FormatException`, in `WebkeyAuthenticationService`, already covered by #521 on the pin.

## Proposed Pin9

Do not cut Pin9 yet. Stay on Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.

OpenSim-NGC `develop` moving to `444008b` is not, by itself, a reason to rebuild `dev-build`. [D21](../DECISIONS.md) rebuilds when a carried pull request changes in review or merges, and it rebuilds on the new `develop` at that time. None of #499 or #503 through #530 has merged here. The three new commits are warning cleanup, docs, and package versions.

When a rebuild does happen, the new base has to be a `develop` commit that still has `JoltPhysicsSharp` 2.19.1 and `JoltPhysics.Native` 1.0.4, or a later commit that updates the loader record, the script, the docs, and the baselines in the same change and matches the generation P3.0c names. `444008b` is not that commit. The rebuild then re-merges the carried pull requests that are still open. The appearance conflict is resolved by keeping #515 (`AppearanceService.cs:63-69` on the pin, and the test at `AppearanceServiceAssetMismatchTests.cs:52`).

Cost of moving the pin to `444008b` anyway:

- The 29 carried pull requests have to be merged again. The only content conflict with the pin is `AppearanceService.cs`. Mixing the two sides of that conflict reintroduces the startup `FormatException`.
- No TB fixture is regenerated, because no TB row is fixed.
- P3.0d's 5.4-native fixture regeneration does not start. This package edit is not the 5.6 move that row names.
- Image captures are not regenerated on this evidence. They would be compared again only if a later pin actually contains the new Skia and CoreJ2K versions and a capture is taken.
- `ferroan-physics` stays on `oxijolt-sys` 1.6.0. No crate change, no decision, no REFERENCES edit.

## What not to take

- Do not take `444008b` as the pin, and do not fast-forward off `dev-build` onto OpenSim-NGC `develop`. That drops the 150 commits the pin holds and the 29 carried pull requests.
- Do not take the `JoltPhysicsSharp` 2.26.0 and `JoltPhysics.Native` 1.3.0 lines from `a40091ea`. The record at `JoltNative.cs:128` still says `1.0.4`, so the package test and the loader disagree with the project file. The physics plan treats Native 1.3.0 as probably Jolt 5.7, which P3.0c calls the wrong generation. Those two lines are inside `a40091ea`, so the commit as one revision should not be cherry-picked onto the pin.
- Do not take the appearance sentence at `AppearanceService.cs:66` on `444008b` in place of #515. The pin's test expects the original sentence, and the pin's fourth argument is what stops the `FormatException`.
- `a225f558` is safe to take with a later base. It changes no behaviour.
- The remaining package bumps and the four analyzer test edits in `a40091ea` have nothing Ferroan must port. They can come along when `develop` is rebuilt for a real carried-PR change. They are not a reason to move the pin now.

## Open questions

- Whether the `JoltPhysicsSharp` 2.26.0 and `JoltPhysics.Native` 1.3.0 lines were intentional. The comments, `JoltNative.PackageVersion`, `assert-joltc-native.ps1`, and `Docs/JoltcNative.md` were left on 1.0.4 and 2.19.1. The native's Jolt generation was not checked by restoring the package.
- Pull request #542 (TB58) is still not on OpenSim-NGC `develop` or on `dev-build`.
- This audit lists `develop` and `dev-build` only. Topic branches on the fork were not reviewed.
