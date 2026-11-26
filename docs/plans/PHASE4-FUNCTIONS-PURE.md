# Phase 4 pure functions

Planning only. This file splits the two pure families from [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 2 into day-sized rows. It does not add a `TASKS.md` row or a `WORK_QUEUE.md` row, and it does not edit the ledger. The pin is `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. [D124](../DECISIONS.md) is settled: Phlox is the reference for the queue, the sleeps, and the 85 `iw*` functions. A YEngine difference is recorded here and is not the pass rule.

The placeholders this file replaces are P4.S1 (64), P4.S2 (32), the whole of P4.S13 (47), and the pure names inside P4.S27. Stage 2 may split a family. It does not move a method into a second family and add it again. An id is a label. Each row below is about a day. A row that overruns a day is split before the next id starts.

## Counts

| Family | Methods | Rows |
| --- | --- | --- |
| `ll` math, strings and crypto | 64 | P4.F1.1–P4.F1.10 |
| `ll` lists and JSON | 32 | P4.F2.1–P4.F2.7, including P4.F2.4b |
| `os` math, strings and crypto | 38 | P4.F1.3, P4.F1.5, P4.F1.9–P4.F1.13b, P4.F1.15 |
| `os` lists | 9 | P4.F2.1, P4.F2.2, P4.F2.3, P4.F2.4 |
| `iw` math and strings | 18 | P4.F1.6, P4.F1.7, P4.F1.9–P4.F1.12, P4.F1.13, P4.F1.13b, P4.F1.14–P4.F1.14d |
| `iw` lists | 6 | P4.F2.8, P4.F2.9 |

64 + 32 + 38 + 9 + 18 + 6 = 167. The `os` 38 and 9 are the 47 on P4.S13. The 24 `iw` names leave P4.S27, which keeps the other 61. No `iw*` name is overloaded. The overloads in this file are `llSHA256String` (2), `osApproxEquals` (6), `osSlerp` (2), `osStringIndexOf` (2), `osStringLastIndexOf` (2) and `osStringSubString` (2).

`llWorldPosToHUD` (`LSLSystemAPI.cs` 17094–17098) returns `<0.5, 0.5, 0>` and is not one of the 64. It stays with object and prim control. `iwFormatTime` (`LSLSystemAPI.cs` 1951–1958) calls `ToLocalTime` when `isUTC` is 0, so it reads the host zone. It stays in the 61. `osCheckODE` reads the physics engine name and stays with the physics `os*` family.

## 1. How a row is proven

P4.19 builds `tools/script-oracle` on `Tests/InWorldz.Phlox.Tests/SchedulerHarness.cs`: a real Phlox engine, a script rezzed the way `OnRezScript` does, and `DoWork` pumped by hand. `Tests/InWorldz.Phlox.Tests/ExprRunner.cs` already captures `llOwnerSay`. These rows do not add P4.19p or any later letter. The cases below are what that harness runs for the row. Expected text is the recording. It is not a number copied out of the cited lines. Ordinary CI does not start Tranquillity.

The compare, for every row in this file:

- An integer, a string, a key, a list element's type, list order, JSON text, error text, and a sleep duration match the recording exactly. Text a script sees, in chat or in a string cast, compares exactly.
- A float said with `llOwnerSay((string)...)` is the six-digit text from `Encoding.FloatToStringWith6FractionalDigits` (`Encoding.cs` 128–160). A vector or rotation said with `(string)` is five digits: `Vector3ToStringWith5FractionalDigits` and `QuaternionToStringWith5FractionalDigits` (`Interpreter.Actions.cs` 1385, 1390, 1805–1806). A raw float result, one the script does not print, compares within the named `libm` ulp in section 4 of [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md). The tolerance is not a residual measured from Ferroan. A case about an integer boundary says the integer result, not the float text.
- A formula that is arithmetic on values already rounded to float32 matches the recording when the port uses the same formula.
- `llFrand`, `llGenerateKey`, `iwIntRand`, `iwIntRandRange`, `iwFrandRange`, `llListRandomize` and `osAESEncrypt` compare the range, the type, and the empty-input or equal-bound result. `osAESEncrypt` compares the shape of the text and a decrypt that returns the plaintext. They do not compare bits.
- A denied `os*` call compares the thrown gate error, which is the recording P4.16 owns. `OsslGate.Check` throws `"OSSL Permission Error: " + why` (`OsslGate.cs` 126–127). The master switch, and a missing `[OSSL]` enable, throws `OSSL Permission Error: All unsafe OSSL funtions disabled` (124), with the `(OWNER)` prefix when `m_errorToOwner` is set (130). A parseable `Allow_<name> = false` leaves `AllowedControl` at 0 (146–148) and `Test` returns `{function} disabled in region configuration` (213). The threat test is not reached. An empty `Allow_` and `Creators_` pair falls through to the threat test, and the text is then `{function} permission denied.  Allowed threat level is {max} but function threat level is {level}` (211), still under the `OSSL Permission Error: ` prefix. The script is killed (`PhloxExecutionScheduler.cs` 1442–1449, `TerminateWithError` 3452–3476). The shout is `Script error: ` plus `Script {asset} encountered a problem and was stopped: {message}` (`LSLSystemAPI.cs` 658–662, scheduler 3475).

An `ll` row depends on P4.1c and P4.19. An `os` row also depends on P4.16. An `iw` row depends on P4.1c and P4.19, the same table dependency P4.S27 uses. The harness runs with the shipped `[OSSL]` (`config-include/osslDefaultEnable.ini`, `OSFunctionThreatLevel = VeryLow` at line 33). A denied call is a second case. The master switch off produces the master-switch text. `Allow_<name> = false` produces the disabled-in-configuration text. Removing the `Allow_` line, so the threat test runs, produces the threat text, and only when the function's level is above the configured maximum. `osMax` is threat `None`, so the threat test cannot refuse it at any configured maximum. Sleeps in this file are recorded on the harness. They do not wait on P4.23. When P4.23 is written, its table gains the 300 ms of `llXorBase64Strings`, the 100 ms of `iwFormatString`, the length sleeps of `iwStringCodec`, the `aes-key` sleep `Math.Max(100, rounds)`, and the 15 ms `ChatSleep` on a `ScriptShoutError`. The number in that table is the last `ScriptSleep` the call writes.

`ScriptSleep` (`LSLSystemAPI.cs` 241–249) writes `NextWakeup = Clock.Now + ms` when the call is on the script thread. A later sleep in the same call replaces that wake. It does not add. Inside one `SyscallContext` the delays add (`SyscallContext.AddDelay`). The pure calls in this file are inline, so they replace. The codec host says the same thing (`HalcyonStringCodec.cs` 62–63). The harness can inject the clock (`Clock.SetSourceForTesting` in `Source/InWorldz.Phlox/Util/Clock.cs`). A regex match timeout is the engine's 250 ms (`ScriptRegex.cs` 21–26), not that clock.

`Vector3` and `Quaternion` in the Phlox file are OpenMetaverse types (`using OpenMetaverse`, `LSLSystemAPI.cs` 56). They are not `System.Numerics`.

### What the script sees on a throw

`OsslCheck` throws `VMException` (`OsslGate.cs` 114–127). A raw `.NET` exception from a body is also an `Exception`. The tick catch (scheduler 1442) calls `TerminateWithError`. The script stops, the Running flag goes off, and the shout carries `e.Message`. There is no second return value. Rows that name a throw mean that shout and that stop. The message text is the recording, because it includes the runtime's wording and the asset id.

`ShoutError` (`LSLSystemAPI.cs` 658–662) speaks `Script error: ` plus the text on `DEBUG_CHANNEL` and does not sleep. `ScriptShoutError` (269–272) does that and then `ChatSleep`, 15 ms when `ChatThrottle` is on (`PhloxEngine.cs` 304, default true). `LSLError` prefixes `LSL Runtime Error: ` and goes through `ScriptShoutError` (277).

### Argument conversion used by more than one row

`LslConvert.FloatToInteger` (`Source/InWorldz.Phlox/Util/LslConvert.cs` 22–27): NaN, or `f >= 2147483648.0`, or `f <= -2147483649.0`, returns `int.MinValue`. Anything else is a truncating cast. The comment records that a plain C# cast on .NET 9 x64 saturates.

`Encoding.CastToInt` (`Encoding.cs` 45–83) strips one leading `+`, then takes a leading run of spaces and either a `0x` hex integer or a signed decimal integer. Overflow becomes -1. No match is 0. `CastToFloat` (85–118) uses the same leading-`+` strip and the `en-US` culture. On .NET 10, `float.Parse` does not throw on overflow, so the `catch (OverflowException)` that stores `+infinity` (111–116) does not run. `"-1e39"` is `-infinity`. A positive overflow is `+infinity`.

`Encoding.Fixed` (128–138) widens a float to double, formats `F6` or `F5` with the invariant culture, and strips a leading minus that has no non-zero digit. `NaN`, `Infinity` and `-Infinity` keep the invariant names. `-0` prints as `0`. List string form uses six digits for a float, a vector and a rotation (`LSLList.cs` 437–463). A bare `(string)` of a vector or rotation uses five (`Encoding.cs` 140–148). The oracle for a function that returns a vector says the value the way the script says it. A list element said through `llList2String` is the six-digit form.

List indexes on the `llList2*` getters, and the range on `llGetSubString`, `llDeleteSubString`, `llList2List`, `llDeleteSubList` and `llListReplaceList`, use `NormaliseLslRange` and `InLslRange` (`LSLSystemAPI.cs` 15579–15586). A negative index has the length added once. When start is past end, the range is `[0, end]` plus `[start, -1]`. An index outside the value selects nothing.

An LSL script cannot pass a null string, and it cannot build a nested list. A null check or a nested-list arm in a body is what that body does when the host hands it one. It is not an oracle case.

### YEngine, once

YEngine stores a float as a double and does not cast `Math.Sin` and its siblings down to float32 (`LSL_Api.cs` 963–990). Phlox does (`LSLSystemAPI.cs` 1365–1383). The handwritten `TokenDeclInline_Math` and `TokenDeclInline_LLAbs` are commented out (`MMRScriptInlines.cs` 530–547). A compiled call goes through `TokenDeclInline_BEApi` (626). Names in `noCheckRuns` (63–109) get the call alone. Every other pure `ll` call gets a `CheckRun` after it (58–59). The names from these families that are in that set are `llBase64ToString`, `llCSV2List`, `llDeleteSubList`, `llDeleteSubString`, `llDumpList2String`, `llEscapeURL`, `llEuler2Rot`, `llGetListEntryType`, `llGetListLength`, `llGetSubString`, `llInsertString`, `llList2CSV`, `llList2Float`, `llList2Integer`, `llList2Key`, `llList2Rot`, `llList2String`, `llList2Vector`, `llListFindList`, `llListInsertList`, `llListRandomize`, `llListReplaceList`, `llListSort`, `llListStatistics`, `llMD5String`, `llParseString2List`, `llParseStringKeepNulls`, `llRot2Euler`, `llStringLength`, `llStringToBase64`, `llStringTrim`, `llSubStringIndex`, `llUnescapeURL`, `osIsNotValidNumber`, `osSlerp`, `osApproxEquals`, `osVecDistSquare`, `osVecMagSquare`, `osRound`, `osMax`, `osMin` and `osIsUUID`. YEngine has no `iw*`. It does not declare `llSignRSA`, `llVerifyRSA`, `llSRGB2Linear` or `llSortListStrided`. It does declare `llsRGB2Linear` and `llListSortStrided` (`ILSL_Api.cs` 511, 271). Those gaps are recorded. The Phlox harness is the proof.

### The gate on a pure `os*` call

`OsslCheck()` with no arguments is the master switch only (`LSLSystemAPI.cs` 11747, `OsslGate.Check` 114–118). `OsslCheck(level, name)` looks up `Allow_` and `Creators_` under `name` (`OsslGate.cs` 132–210). An empty pair falls through to the threat test. `ThreatLevel.None` passes while the shipped maximum is `VeryLow`, because `None <= VeryLow` (208–210). A line `Allow_osMax` does not apply to a check whose name is `osGetRezzingObject`.

Gated in this file, master switch only: `osPerlinNoise2D`, `osListSortInPlace`, `osListSortInPlaceStrided`, `osStringStartsWith`, `osStringEndsWith`, both `osStringIndexOf`, both `osStringLastIndexOf`, both `osStringSubString`. Named: `osMax` (`None`, key `osGetRezzingObject`), `osFormatString` (`VeryLow`), `osMatchString` (`VeryLow`), `osRegexIsMatch` (`Low`), `osReplaceString` (`VeryLow`). The shipped file allows the four named ones that have an active line: `Allow_osFormatString` (116), `Allow_osMatchString` (170), `Allow_osRegexIsMatch` (123), `Allow_osReplaceString` (102). `Allow_osGetRezzingObject` is commented out (85), so `osMax` falls through to the threat test and `None` passes. The other pure `os*` bodies do not call `OsslCheck`. With the master switch off they still return. The denied case covers the gated names. The ungated names are run once with the switch off and still return.

YEngine does not gate `osPerlinNoise2D` (`OSSL_Api.cs` 6400–6402), `osListSortInPlace` (6504–6506) or `osListSortInPlaceStrided` (6509–6511). Phlox's own comments on those three say ungated upstream (`LSLSystemAPI.cs` 12188, 12077–12078, 12102). YEngine's in-place sorts treat only `ascending == 1` as ascending (6506, 6511). Recorded. YEngine `osMax` is ungated (4542–4545). The `CheckThreatLevel(ThreatLevel.None, "osGetRezzingObject")` at 4547–4549 belongs to `osGetRezzingObject`. Phlox's gate on `osMax` is the port. Proposed TB172.

## 2. Math and strings

The 64 `ll` methods are `llSin`, `llCos`, `llTan`, `llAtan2`, `llSqrt`, `llPow`, `llAbs`, `llFabs`, `llFrand`, `llFloor`, `llCeil`, `llRound`, `llVecMag`, `llVecNorm`, `llVecDist`, `llRot2Euler`, `llEuler2Rot`, `llAxes2Rot`, `llRot2Fwd`, `llRot2Left`, `llRot2Up`, `llRotBetween`, `llAxisAngle2Rot`, `llRot2Axis`, `llRot2Angle`, `llAcos`, `llAsin`, `llAngleBetween`, `llModPow`, `llLog10`, `llLog`, `llGetSubString`, `llDeleteSubString`, `llInsertString`, `llToUpper`, `llToLower`, `llStringLength`, `llSubStringIndex`, `llStringTrim`, `llReplaceSubString`, `llChar`, `llOrd`, `llEscapeURL`, `llUnescapeURL`, `llMD5String`, `llStringToBase64`, `llBase64ToString`, `llXorBase64Strings`, `llIntegerToBase64`, `llBase64ToInteger`, `llXorBase64StringsCorrect`, `llSHA1String`, `llGenerateKey`, `llComputeHash`, `llHMAC`, `llSHA256String` (both), `llXorBase64`, `llSignRSA`, `llVerifyRSA`, `llHash`, `llLinear2sRGB`, `llSRGB2Linear`, `llsRGB2Linear`.

The 38 `os` methods that are not lists are `osAESEncrypt`, `osAESDecrypt`, `osAESEncryptTo`, `osAESDecryptFrom`, `osAngleBetween`, `osTemperature2sRGB`, `osPerlinNoise2D`, `osApproxEquals` (6), `osFormatString`, `osIsNotValidNumber`, `osIsUUID`, `osMatchString`, `osMax`, `osMin`, `osRegexIsMatch`, `osRound`, `osSHA256`, `osSlerp` (2), `osStringStartsWith`, `osStringEndsWith`, `osStringIndexOf` (2), `osStringLastIndexOf` (2), `osStringRemove`, `osStringReplace`, `osStringSubString` (2), `osUnixTimeToTimestamp`, `osVecDistSquare`, `osVecMagSquare`, `osReplaceString`.

The 18 `iw` methods are `iwSHA256String`, `iwSubStringIndex`, `iwChar2Int`, `iwInt2Char`, `iwReplaceString`, `iwFormatString`, `iwMatchString`, `iwStringCodec`, `iwReverseString`, `iwIntRand`, `iwIntRandRange`, `iwFrandRange`, `iwClampInt`, `iwClampFloat`, `iwValidateURL`, `iwColorConvert`, `iwNameToColor`, `iwVerifyType`.

### P4.F1.1 Libm and absolute value

Depends on P4.1c, P4.19.

Functions: `llSin`, `llCos`, `llTan`, `llAtan2`, `llSqrt`, `llPow`, `llAcos`, `llAsin`, `llLog`, `llLog10`, `llFabs`, `llAbs`.

Source: `Source/Phlox.ScriptEngine/LSLSystemAPI.cs` 1365–1383. YEngine: `Source/OpenSim.Region.ScriptEngine.Shared/Api/LSL_Api.cs` 963–1005.

Each trig, log, power and square-root call is `(float)` of the matching `System.Math` function. `llFabs` is `Math.Abs` on the float (1372). `llAbs` returns `int.MinValue` unchanged and `Math.Abs` otherwise (1371). YEngine's `llAbs` is the same test (993–999). A NaN stays NaN. `llSqrt` of a negative is NaN. `llLog(0)` is negative infinity. `llLog` of a negative is NaN. `llAcos` and `llAsin` outside `[-1, 1]` are NaN. `llTan` near `π/2` is a large float. Nothing shouts. Nothing sleeps. `llPow(0, 0)` is `Math.Pow`. YEngine keeps the double (963–990) and emits `CheckRun`. Recorded.

Oracle: say each result. Cases are 0, a small angle, `π/2`, a value outside the domain, NaN, both infinities, `llPow(0, 0)`, `llPow(0, -1)`, `llPow` of a negative base with a fractional exponent, `llAbs` of 0, of a negative, and of -2147483648. `llAbs` is said as an integer.

### P4.F1.2 Integer conversion and modular power

Depends on P4.1c, P4.19.

Functions: `llFloor`, `llCeil`, `llRound`, `llModPow`.

Source: `LSLSystemAPI.cs` 1374–1379 and 1518–1527. `LslConvert.cs` 22–27. YEngine: `LSL_Api.cs` 1012–1026 and 14340–14345, sleep field 185.

`llFloor` and `llCeil` pass `Math.Floor` and `Math.Ceiling` through `FloatToInteger`. `llRound` passes `Math.Round` with `MidpointRounding.AwayFromZero` through the same converter (1379). NaN and an out-of-range magnitude return -2147483648. A value inside the range truncates toward zero after the rounding function. YEngine casts with `(int)` (1012–1026). On .NET 9 x64 that cast saturates. Recorded.

`llModPow` (1518–1527) reduces `a` modulo `c` before the loop. `c == 0` throws `DivideByZeroException` from that remainder, and the script dies with the shout in section 1. A negative or zero exponent skips the loop and returns 1. Phlox does not sleep. YEngine is `Math.DivRem((long)Math.Pow(a, b), c)` and then `ScriptSleep(1000)` (14340–14345, `m_sleepMsOnModPow` at 185). Recorded.

Oracle: `llFloor`, `llCeil` and `llRound` of 0, of 2.5, of -2.5, of a value just inside each integer bound, of 2147483648, of -2147483649, of NaN, and of both infinities. Said as integers. `llModPow` of a normal triple, of exponent 0, of a negative exponent, and of modulus 0. The modulus-0 case compares the shout and the stopped script. The sleep is 0 on Phlox. YEngine's 1000 ms is recorded and is not a pass failure.

### P4.F1.3 Vector length, distance and angle

Depends on P4.1c, P4.19. The `os*` names also depend on P4.16.

Functions: `llVecMag`, `llVecNorm`, `llVecDist`, `osVecDistSquare`, `osVecMagSquare`, `osAngleBetween`.

Source: `LSLSystemAPI.cs` 1385–1387 and 11920–11924, 12574–12582. YEngine vector helpers are `LSL_Api.cs` 1028 onward (`llVecMag` is `LSL_Vector.Mag`). `osVecDistSquare` and `osVecMagSquare` are in `noCheckRuns`.

`llVecMag` is `v.Length()`. `llVecDist` is `Vector3.Distance`. `llVecNorm` is `Vector3.Normalize`. `osVecMagSquare` is `LengthSquared`. `osVecDistSquare` is the length squared of the difference. `osAngleBetween` is `atan2` of the cross-product length and the dot product (11920–11924). The two square calls and the angle are ungated. The zero vector and a vector with a NaN component are library results. This plan does not state them. The harness records `llVecNorm(<0, 0, 0>)` and `llVecNorm` of a NaN component.

YEngine's `llVecMag` uses its own `LSL_Vector`, which is a double vector. Recorded.

Oracle: a unit axis, a non-unit vector, the zero vector, a vector with a NaN component, and two equal vectors (distance 0). Squares are said as floats. The angle cases are parallel, opposite, and perpendicular vectors, plus a zero vector. One run with the master switch off still returns the ungated `os*` values.

### P4.F1.4 Rotations, first half

Depends on P4.1c, P4.19.

Functions: `llRot2Euler`, `llEuler2Rot`, `llAxes2Rot`, `llRot2Fwd`, `llRot2Left`, `llRot2Up`, `llRotBetween`, `llAxisAngle2Rot`.

Source: `LSLSystemAPI.cs` 1389–1477.

`llRot2Euler` (1389–1401) rotates `<0, 0, 1>`, returns the zero vector when that length is 0, forces x to 0 when the sine is within `1e-6` of ±1, then takes `atan2` of the twice-rotated `<1, 0, 0>`. `llEuler2Rot` (1404–1411) is the half-angle product, then `Quaternion.Normalize`. `llAxes2Rot` (1414–1442) is the trace construction with the three branch cuts written there. `llRot2Fwd`, `llRot2Left` and `llRot2Up` (1444–1459) rotate `<1, 0, 0>`, `<0, 1, 0>` and `<0, 0, 1>` and normalise. `llRotBetween` (1462–1471) returns `Quaternion.Identity` when either length is 0 or `axis.X` is NaN (1470). A NaN in Y or Z with a finite X does not take that branch. Otherwise it is the axis-angle of the clamped dot product. `llAxisAngle2Rot` (1474–1477) normalises the axis and calls `Quaternion.CreateFromAxisAngle`. Nothing sleeps. Nothing shouts.

Oracle: identity, a 90-degree turn about Z, a zero quaternion into `llRot2Euler`, Euler of the zero vector, axes that are not orthonormal, `llRotBetween` of two zero vectors, of a pair whose cross product has a NaN X, and of two parallel vectors, and `llAxisAngle2Rot` of a zero axis. Say the vector or the rotation. A vector or rotation said with `(string)` is five digits, as section 1 says. `llEuler2Rot` and `llRot2Euler` are in YEngine's `noCheckRuns`. The bodies are still the Phlox ones above for the pass.

### P4.F1.5 Rotations, second half, and slerp

Depends on P4.1c, P4.19. `osSlerp` also depends on P4.16.

Functions: `llRot2Axis`, `llRot2Angle`, `llAngleBetween`, `osSlerp` (quaternion), `osSlerp` (vector).

Source: `LSLSystemAPI.cs` 1480–1516 and 12443–12475.

The comment at 1480 says these three match YEngine and no longer use OpenMetaverse's small-angle cutoff. `llRot2Axis` (1489–1494) returns the zero vector when the xyz length is under `1e-12`. The sign of W folds the axis to the turn of `π` or less. `llRot2Angle` (1498–1502) is `2 * atan2` of the xyz length and `|W|`. `llAngleBetween` (1506–1515) returns 0 when either quaternion has length 0 or the squared-cosine quotient is at least 1. Otherwise it is `acos(2 * quotient - 1)`.

`osSlerp` on quaternions (12443–12447) clamps the amount to `[0, 1]`, normalises both inputs, and calls `Quaternion.Slerp`. Ungated. `osSlerp` on vectors (12455–12475) clamps the amount and does not normalise. When the raw dot product is at least `0.999` it lerps. Otherwise it is the spherical blend written there. YEngine has `osSlerp` in `noCheckRuns`. The double-versus-float32 gap is the shared one.

Oracle: a rotation with a tiny angle, a rotation with negative W, a zero quaternion, two zero quaternions into `llAngleBetween`, and a quotient that lands on 1. `osSlerp` at amounts -1, 0, 0.5, 1 and 2, for a quaternion pair and for a vector pair, including a pair whose dot product is at least `0.999` and a pair under that. One run with the master switch off still returns.

### P4.F1.6 Random numbers and keys

Depends on P4.1c, P4.19.

Functions: `llFrand`, `llGenerateKey`, `iwIntRand`, `iwIntRandRange`, `iwFrandRange`.

Source: `LSLSystemAPI.cs` 1373, 16024, 18485–18491. YEngine `llFrand` is `Random.Shared.NextDouble() * mag` with no float cast (`LSL_Api.cs` 1007–1009). YEngine has no `iw*`.

`llFrand` is `(float)(ThreadRandom.NextDouble() * mag)` (1373). `llGenerateKey` is `UUID.Random().ToString()` (16024). `iwIntRand` (18485–18488) returns `ThreadRandom.Next` on `0 .. max` inclusive, or the negation of that on `max .. 0` when `max` is negative. The bound is clamped so the `Next` argument stays inside `int`. `iwIntRandRange` and `iwFrandRange` (18490–18491) use `new Random()`. Equal bounds return that bound. Reversed bounds are swapped. The integer form is `Next(min, max + 1)`. When `max` is 2147483647, `max + 1` wraps to `int.MinValue` and `Random.Next` throws `ArgumentOutOfRangeException`. The script dies with the shout in section 1. The float form is `NextDouble() * (max - min) + min`. Not bit-exact. Nothing sleeps.

Oracle: `llFrand(0)`, `llFrand` of a positive, and `llFrand` of a negative, checked for type and for lying in the range the call describes. `llGenerateKey` checked as a string `UUID.TryParse` accepts. `iwIntRand(0)`, `iwIntRand` of a positive, and of a negative. `iwIntRandRange` and `iwFrandRange` with equal bounds, with reversed bounds, and with a normal pair. `iwIntRandRange` of any lower bound with upper bound 2147483647 compares the shout and the stopped script. Several calls of the non-throwing forms are allowed to differ.

### P4.F1.7 String length, case, and ranges

Depends on P4.1c, P4.19.

Functions: `llStringLength`, `llToUpper`, `llToLower`, `llGetSubString`, `llDeleteSubString`, `llInsertString`, `llStringTrim`, `llSubStringIndex`, `llReplaceSubString`, `llChar`, `llOrd`, `iwSubStringIndex`, `iwReverseString`, `iwChar2Int`, `iwInt2Char`.

Source: `LSLSystemAPI.cs` 15568–15634, 15681–15805. YEngine `llToUpper` and `llToLower` are 3373–3380. YEngine `llSubStringIndex` is 6217–6223.

`llStringLength`, `llToUpper` and `llToLower` null-coalesce (15568–15570). `ToUpper` and `ToLower` use the current culture. There is no `Invariant`. The harness culture is `en-US`, as question 5 says. YEngine's `llToUpper` does not null-check (3373–3375). Recorded.

`llGetSubString` and `llDeleteSubString` (15588–15616) use the range rule in section 1. A null source is `""`. `llInsertString` (15618–15623) clamps the position into `[0, length]` and does not treat a negative as an end-relative index. `llStringTrim` (15807–15815): type 1 trims the head, 2 the tail, 3 both, and any other type returns the string. Whitespace is `.NET` `Trim`. A null is `""`. `llSubStringIndex` (15626–15627) is `source?.IndexOf` ordinal, or -1 when the source is null. A null pattern would throw `ArgumentNullException`. An LSL script cannot pass that null, so the case is not in the oracle. YEngine (6217–6223): an empty source returns -1, an empty pattern returns 0, and the compare is the default `IndexOf`. Recorded.

`llReplaceSubString` (15681–15731): a count of 0 replaces every match. A negative count replaces from the end. An empty pattern or an empty source returns the source. The search is ordinal. `llChar` (16027–16034): a negative returns `""`, 0 returns a NUL, and any other code point is `ConvertFromUtf32`, or `""` when that throws. `llOrd` (16036–16045) adds the length to a negative index once, returns 0 out of range, and reads a surrogate pair with `ConvertToUtf32`.

`iwSubStringIndex` (15629–15633) returns -1 when either argument is null. The offset is clamped up with `Math.Max(0, offset)`, so a negative offset starts at 0. `IndexOf(pattern, offset, comp)` throws `ArgumentOutOfRangeException` when that offset is past the end. The script dies. A non-zero `isCaseSensitive` is ordinal. Zero is ordinal-ignore-case. `iwReverseString` (15788–15792) reverses UTF-16 code units, so a surrogate pair swaps halves. The comment says that is Halcyon and is kept. A null is `""`. Length 0 or 1 returns the same string. `iwChar2Int` (15794–15799) reads one UTF-16 unit, not a surrogate pair. A negative index adds the length once. Out of range or empty is 0. `iwInt2Char` (15801–15804) returns `""` outside `0 .. 0xFFFF`. `iwInt2Char(0xD800)` is one lone surrogate.

Oracle: `llToUpper` and `llToLower` of an ordinary string under `en-US`, an ordinary substring, a negative index, an inverted range, an insert at a negative position and past the end, trim types 0, 1, 2 and 3, `llSubStringIndex` of an empty pattern and of a missing pattern, `llReplaceSubString` with counts 0, 1 and -1, `llChar` of -1, 0 and a surrogate code point, `llOrd` on a surrogate pair and on a negative index, `iwSubStringIndex` with a negative offset (a normal index) and with an offset past the end (the shout and the stop), `iwReverseString` of a character outside the BMP, `iwChar2Int` of that same character, and `iwInt2Char` of -1, of `0xD800`, and of `0x10000`. How closely the `en-US` case folding must match ICU is question 5.

### P4.F1.8 Base64 and URL escape

Depends on P4.1c, P4.19.

Functions: `llStringToBase64`, `llBase64ToString`, `llIntegerToBase64`, `llBase64ToInteger`, `llXorBase64Strings`, `llXorBase64StringsCorrect`, `llXorBase64`, `llEscapeURL`, `llUnescapeURL`.

Source: `LSLSystemAPI.cs` 15818–15920, 16025–16026, 21766–21788. YEngine `llBase64ToString` is 11806–11817. YEngine `llXorBase64Strings` sleeps 300 as well (14887–14891).

`llStringToBase64` is UTF-8 then `Convert.ToBase64String`. A null is `""`. `llBase64ToString` replaces U+FFFD with `?` and returns `""` on a bad payload, with no shout (15819). YEngine does not replace U+FFFD, and a bad payload shouts `Error` and returns `""` (11806–11816). Recorded.

`llIntegerToBase64` and `llBase64ToInteger` (15820–15851) use the tables `i2ctable` and `c2itable`, not `Convert.FromBase64String`. The integer encoder always writes eight characters, and the last two are `=`. The decoder returns 0 when the string is empty or longer than 8. `c2itable` has length 128 (2438–2448). An ASCII character whose table digit is negative returns 0 for the whole call. `=` is digit 0 and stops early with the partial value. A character at or above 128 makes the length test true before that character assigns `digit`, and the call returns the partial value. `llXorBase64Strings` (15862–15905) sleeps 300 ms first. The comment says the wiki documents 0.3 s and that the padding quirks are the behaviour. This plan does not file a ledger row for them. An empty first string returns `""` after the sleep. An empty second string returns the first. A bad payload returns `""`. `llXorBase64StringsCorrect` (15907–15920) decodes, XORs UTF-16 units, re-encodes, and does not sleep. An empty first string returns that string or `""`. An empty second string returns the first. When the second string is non-empty and `llBase64ToString` of it is `""`, the loop reads `src2[c]` and throws. The script dies. `llXorBase64` (21766–21788) XORs decoded bytes and re-encodes. A bad payload returns `""`. An empty second string returns the first.

`llEscapeURL` and `llUnescapeURL` (16025–16026) are `Uri.EscapeDataString` and `Uri.UnescapeDataString` with no catch. A null is `""`. On .NET 10 a bad `%` sequence comes back unchanged, and `EscapeDataString` of a long string returns normally. A lone surrogate is escaped as `%EF%BF%BD`. Neither call throws. Neither is on the throw list in question 7.

Oracle: a short ASCII string both ways, a non-ASCII character, a bad base64 payload, an integer 0 and a negative integer through the eight-character form, `llBase64ToInteger` of an ASCII non-base64 character (0), of a string that starts with valid digits and then `=` (the partial value), and of valid digits followed by a character at or above 128 (the partial value), `llXorBase64Strings` of two short strings and of an empty string (the wake is 300 ms), the same inputs on `llXorBase64StringsCorrect` and `llXorBase64` (no sleep), and `llXorBase64StringsCorrect` with a valid first payload and a non-empty second payload that decodes to `""` (the shout and the stop). `llEscapeURL` of a space, of a non-ASCII character, and of `iwInt2Char(0xD800)` (the text contains `%EF%BF%BD`). `llUnescapeURL` of `a%zzb%` returns `a%zzb%` and the script keeps running. The 300 ms is recorded here. It does not wait on P4.23.

### P4.F1.9 Hashes

Depends on P4.1c, P4.19. `osSHA256` also depends on P4.16.

Functions: `llMD5String`, `llSHA1String`, `llSHA256String` (both), `osSHA256`, `iwSHA256String`, `llComputeHash`, `llHMAC`, `llHash`.

Source: `LSLSystemAPI.cs` 15923–16023, 15937–15941, 21408–21419, 21560–21564. YEngine `llComputeHash` is `LSL_Api.cs` 21663–21719. YEngine `llHMAC` is 22314–22362, and the interface order is `ILSL_Api.cs` 549. Phlox's order is `ISystemAPI.cs` 970 and `ILSLSystemAPI.cs` 801.

`llMD5String` is MD5 of the UTF-8 bytes of `src + ":" + nonce`, lowercase hex (15923–15928). `llSHA1String` is SHA-1 of the UTF-8 source (15931–15935). `llSHA256String(src, nonce)` hashes `src + ":" + nonce` through `llComputeHash` with `sha256` (15985–15988). `llSHA256String(src)` is SHA-256 of the UTF-8 source with nothing appended (21560–21564). `osSHA256` (12436–12439) and `iwSHA256String` (15937–15941) are that same hex. `osSHA256` is ungated. `llHash` (21408–21419) is DJB2 from 5381 over the UTF-16 units, and 0 for null or empty.

`llComputeHash` (15943–15984) lowercases the algorithm and implements `md5`, `sha1`, `sha256`, `sha384`, `sha512`, and `md5_sha1`. `md5_sha1` is SHA-1 of the lowercase hex of the MD5. The comment at 15945 names `sha224`. There is no `sha224` case. An unknown algorithm shouts `llComputeHash: unsupported algorithm '...'` and returns `""`. YEngine (21663–21719) is case-sensitive, implements `sha224`, returns `""` with no shout on an unknown name, and builds `md5_sha1` by concatenating the raw MD5 bytes and the raw SHA-1 bytes of the original input. Recorded. Proposed TB170 and TB171.

`llHMAC` (15990–16022) takes `(msg, privateKey, algorithm)`, returns base64, and implements `md5`, `sha1`, `sha256`, `sha384`, `sha512`. An unknown algorithm shouts and returns `""`. There is no `sha224`. The comment at 15992 says SL returns a base64 HMAC. YEngine takes `(private_key, message, algo)` (`ILSL_Api.cs` 549), returns lowercase hex (22359), implements `sha224` (22290–22311), and returns `""` when the key or the message has length under 1 or the algorithm is unknown (22316–22317, 22342). Recorded. The default under [D124](../DECISIONS.md) is Phlox's order and Phlox's base64. Question 3 is the later fix decision. This row does not make it.

Oracle: `llMD5String` and both `llSHA256String` forms on `""` and on a short string, so the nonce colon is visible. `llSHA1String`, `osSHA256` and `iwSHA256String` on the same bytes. `llHash("")` and `llHash` of one character. `llComputeHash` for each implemented name, for `SHA256` (Phlox lowercases it), for `sha224`, and for a misspelled name (the shout, the script still running). `llHMAC` with Phlox's argument order, one known algorithm, `sha224`, an unknown name, and an empty key. Hex and base64 are exact. `osSHA256` still returns with the master switch off.

### P4.F1.10 RSA and colour

Depends on P4.1c, P4.19. `osTemperature2sRGB` and the `iw*` names also depend on P4.16 for the `os*` name only. The `iw*` names depend on P4.1c and P4.19.

Functions: `llSignRSA`, `llVerifyRSA`, `llLinear2sRGB`, `llSRGB2Linear`, `llsRGB2Linear`, `osTemperature2sRGB`, `iwColorConvert`, `iwNameToColor`.

Source: `LSLSystemAPI.cs` 20749–20799, 17068–17092, 21550, 11960–11985, 18245–18262, helpers 18266–18348. YEngine declares `llLinear2sRGB` and `llsRGB2Linear` and does not declare `llSRGB2Linear`, `llSignRSA` or `llVerifyRSA`.

`llSignRSA` (20749–20772) uppercases the algorithm and accepts `SHA256`, `SHA384`, `SHA512` and `SHA1`. It imports a PEM private key, signs UTF-8 with PKCS1, and returns base64. Any exception is logged and the result is `""`. A bad algorithm returns `""` with no throw. `llVerifyRSA` (20775–20799) is the same set plus `SHA224`, which builds `new HashAlgorithmName("SHA224")` (20786). On .NET 10 `VerifyData` throws because `SHA224` is not a known hash algorithm, and the catch at 20795 returns 0. `SHA224` is always 0. Success is 1, failure is 0, and any exception is 0. YEngine has neither method. Recorded.

`llLinear2sRGB` and `llSRGB2Linear` (17068–17092) use the piecewise sRGB curve and clamp each component to `[0, 1]`. `llsRGB2Linear` forwards to `llSRGB2Linear` (21550). `osTemperature2sRGB` (11960–11985) is the blackbody polynomial, clamped, with the 1000 and 40000 endpoints written there. Ungated. `iwColorConvert` (18245–18255): 0 is RGB, 1 is HSL, 2 is HSV. The same space returns the input. Any other source space is treated as already RGB. The helpers are `H2RGB`, `HSL_TO_RGB`, `HSV_TO_RGB`, `RGB_TO_HSL` and `RGB_TO_HSV` (18266–18348). `iwNameToColor` (18257–18261) is `System.Drawing.Color.FromName` after spaces are removed, then R, G and B divided by 255.

Oracle: `llSignRSA` and `llVerifyRSA` with a fixture PEM the harness generates, a bad algorithm, a bad PEM, and a bad signature. `llVerifyRSA` with `SHA224` returns 0 and the script keeps running. The bad paths return `""` and 0 and the script keeps running. One linear colour and one sRGB colour through both spellings, including a component under 0 and over 1. `osTemperature2sRGB` at 1000, at 40000, and at one value in each branch (under 6600, and over it). `iwColorConvert` for each pair of spaces, including a pair that is the same. `iwNameToColor` of `Red` and of an unknown name. A float said with `(string)` is six digits. A vector said with `(string)` is five, as section 1 says. `osTemperature2sRGB` still returns with the master switch off.

### P4.F1.11 Numeric helpers

Depends on P4.1c, P4.16, P4.19. `iwClampInt`, `iwClampFloat` and `iwVerifyType` depend on P4.1c and P4.19.

Functions: `osApproxEquals` (6), `osMax`, `osMin`, `osRound`, `osIsNotValidNumber`, `osIsUUID`, `osPerlinNoise2D`, `osUnixTimeToTimestamp`, `iwClampInt`, `iwClampFloat`, `iwVerifyType`.

Source: `LSLSystemAPI.cs` 12188–12191, 12284–12342, 12406–12433, 12568–12570, 18350–18376, 18472–18483. `TerrainUtil.cs` 110–121. `SlConst.cs` 409–415. YEngine `osPerlinNoise2D` returns the double (`OSSL_Api.cs` 6400–6402).

`osApproxEquals` (12284–12314): the float form uses margin `1.0e-6`. A passed margin is replaced by its absolute value. The vector form compares each component, not the length. The quaternion form compares all four. A component outside the margin makes the result 0. Ungated. In `noCheckRuns`.

`osMax` (12406–12409) calls `OsslCheck(ThreatLevel.None, "osGetRezzingObject")`. The comment at 12409 says upstream copy-pastes that key. At the pin, YEngine `osMax` is ungated (`OSSL_Api.cs` 4542–4545). The threat check with that key belongs to `osGetRezzingObject` (4547–4549). The gate is Phlox's port. With the shipped file the call is allowed, because the `Allow_` line is commented out (ini line 85) and threat `None` passes. `Allow_osMax` is not the key that is read. `Allow_osGetRezzingObject = false` leaves `AllowedControl` at 0, so the text is `osGetRezzingObject disabled in region configuration`. The threat test is not reached, and threat `None` cannot fail it at any configured maximum. `osMin` (12412–12415) is ungated. Proposed TB172.

`osRound` (12428–12432): `ndigits <= 0` rounds to an integer with `AwayFromZero`. `ndigits > 0` also uses `AwayFromZero`, with the digit count. `ndigits > 15` is clamped to 15 first. `osIsNotValidNumber` (12331–12336) returns 1 for NaN, 2 for negative infinity, 3 for positive infinity, and 0 otherwise. `osIsUUID` (12340–12342) is `UUID.TryParse`. `osUnixTimeToTimestamp` (12568–12570) is `Util.ToDateTime(time).ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ")`. `ToDateTime` adds the seconds to `UnixEpoch`, which is UTC (`Util.cs` 185–187, 1170–1172). The `Z` is a literal. The custom format reads the current culture's calendar and time separator. The harness culture is `en-US` (question 5). Under that culture epoch 0 is `1970-01-01T00:00:00.0000000Z`.

`osPerlinNoise2D` (12188–12191) is `(float)TerrainUtil.PerlinNoise2D`. That function (110–121) is a loop over octaves of `InterpolatedNoise`. It does not read the heightmap. `octaves <= 0` skips the loop and returns 0. It is pure. Phlox's master switch still gates the call. YEngine does not gate it and returns the double (6400–6402). Recorded.

`iwClampInt` and `iwClampFloat` (18472–18483): equal bounds return that bound. Reversed bounds still clamp into the interval. `iwVerifyType` (18350–18376): type 0 tries 1, 2, 4, 5, 6 and returns the first match, or 3. `TYPE_INTEGER` is 1, `TYPE_FLOAT` is 2, `TYPE_STRING` is 3, `TYPE_KEY` is 4, `TYPE_VECTOR` is 5, `TYPE_ROTATION` is 6, `TYPE_INVALID` is 0 (`SlConst.cs` 409–415). Integer, float, key, vector and rotation use `TryParse`. A vector needs exactly two commas. A rotation needs exactly three. `TYPE_STRING` returns 1 for any string. Any other type returns -1. `TryParse` uses the current culture. The harness culture is `en-US`. How closely ICU must match is question 5.

Oracle: `osApproxEquals` on each signature, inside the margin, outside it, and with a negative margin. `osMax` and `osMin` of NaN and of mixed infinities. `osMax` once under the shipped file (allowed) and once with `Allow_osGetRezzingObject = false` (the disabled-in-configuration text, and the name in it is `osGetRezzingObject`). `osRound` with `ndigits` -1, 0, 2 and 20, each `AwayFromZero`, and 20 clamped to 15. `osIsNotValidNumber` of a finite value, NaN, and both infinities. `osIsUUID` of a real UUID, of `""`, and of a 36-character string that is not a UUID. `osPerlinNoise2D` at a fixed point with 0 octaves and with 2, and once with the master switch off (the master-switch text). `osUnixTimeToTimestamp` of 0 and of a negative. The string is exact. `iwClamp*` with equal bounds, reversed bounds, and a value inside. `iwVerifyType` for each type constant, for auto-detect, and for an unknown type.

### P4.F1.12 Plain string helpers

Depends on P4.1c, P4.16, P4.19. `iwReplaceString` and `iwValidateURL` depend on P4.1c and P4.19.

Functions: `osStringStartsWith`, `osStringEndsWith`, `osStringIndexOf` (2), `osStringLastIndexOf` (2), `osStringRemove`, `osStringReplace`, `osStringSubString` (2), `iwReplaceString`, `iwValidateURL`.

Source: `LSLSystemAPI.cs` 12478–12565, 15674–15679, 15423–15427.

`osStringStartsWith` and `osStringEndsWith` (12478–12491) always pass `CultureInfo.CurrentCulture`. `ignorecase` only toggles the case flag (12483, 12491). An empty source or value returns 0. Master switch. The harness culture is `en-US`. How closely ICU must match is question 5. `osStringIndexOf` (12494–12509) uses ordinal or ordinal-ignore-case. The three-argument form returns -1 on an empty source or value. The five-argument form clamps a negative offset up to 0, returns -1 when the offset is past the end, and turns a count of 0 or less into the remainder of the string. Master switch.

`osStringLastIndexOf` of three arguments (12512–12517) is `LastIndexOf` over the whole string. The five-argument form (12521–12527) clamps the offset and the count the same way as `IndexOf`, then calls `String.LastIndexOf(value, offset, count, comparison)`. That search runs backward. .NET throws `ArgumentOutOfRangeException` when `offset - count + 1 < 0`. The call with offset 0 and count 0 sets the count to the whole length and throws whenever the string is longer than one character. It throws for any offset in the first half when the count is 0 or less. The script dies. The three-argument form does not throw. Proposed TB167. Executed on .NET 10. YEngine has the same body (`OSSL_Api.cs` 5446–5467). Recorded.

`osStringRemove` (12531–12537) is ungated. An empty source, or an offset past the end, returns `""`. A negative offset becomes 0. A count of 0 or less becomes the remainder. If that count is at least the length, the result is `""`, so offset 0 and count 0 return `""`. `osStringReplace` (12540–12545) is ungated. An empty source or an empty old value returns `""`. An empty new value is passed as null, which is how `String.Replace` deletes. `osStringSubString` of two arguments (12549–12555) is the master switch. An offset of 0 or less returns the whole string. It does not count from the end. The three-argument form (12558–12564) returns `""` when the length is 0 or less or the offset is past the end. When the offset is 0 or less and the length equals `src.Length`, it returns `src`. Otherwise that offset becomes 0 and the result is `Substring(0, length)` after the length is clamped to what remains.

`iwReplaceString` (15674–15679) returns the original when the string or the pattern is empty. An empty replacement is checked before the 1024 limit, so a pattern longer than 1024 with an empty replacement is still replaced (`Replace(pattern, null)`). A non-empty replacement longer than 1024, or a pattern longer than 1024 with a non-empty replacement, returns the original. `iwValidateURL` (15423–15427) is `Uri.TryCreate` with `UriKind.Absolute`, and the scheme must be `http` or `https`. The result is `Convert.ToInt32` of that boolean.

Oracle: starts and ends, case-sensitive and not, including an empty value. Both index forms, including offset 0 and count 0 on `IndexOf` (a number) and on `LastIndexOf` (the shout and the stop). `osStringRemove` of offset 0 and count 0, and of a middle offset. `osStringReplace` of an empty old value. `osStringSubString` of two arguments with a negative offset (the whole string), and of three arguments with a negative offset and a length shorter than the string (the prefix of that length), and with that length equal to the string (the whole string). One denied call of a master-switch name with the switch off, and one allowed call of `osStringRemove` with the switch off. `iwReplaceString` of a pattern over 1024 with an empty replacement (the pattern is removed) and of a pattern over 1024 with a non-empty replacement (the original string). `iwValidateURL` of `https`, of `ftp`, and of a relative string.

### P4.F1.13 Regex

Depends on P4.1c, P4.16, P4.19, and on the engine in [D131](../DECISIONS.md) (P4.34 through P4.34f). `iwMatchString` depends on P4.1c, P4.19, and that same engine. About a day.

Functions: `osMatchString`, `osRegexIsMatch`, `osReplaceString`, `iwMatchString`.

Source: `LSLSystemAPI.cs` 12380–12402, 12418–12424, 13108–13116, 15636–15672. `ScriptRegex.cs` 1–27. `ScriptRegex.cs` is MPL-2.0 (Legion Builds, 2026). A port of that wrapper keeps the notice. The engine it constructs is question 10.

`osMatchString` (12380–12402) is `VeryLow`, key `osMatchString`, allowed by ini 170. A negative start counts from the end. Out of range returns an empty list. Each successful group is pushed, then its index (12392–12393). Only `RegexMatchTimeoutException` is caught. A timeout shouts `regex timed out` through `ShoutError` (no 15 ms) and returns an empty list. A pattern that does not compile throws `RegexParseException` and the script dies. `osRegexIsMatch` (12418–12424) is `Low`, key `osRegexIsMatch`, allowed by ini 123 even though `Low` is above the shipped `VeryLow`. A timeout shouts `regex timed out` and returns 0. Any other exception, including a pattern that does not compile, shouts `Possible invalid regular expression detected.` and returns 0. The script keeps running. `Allow_osRegexIsMatch = false` is `osRegexIsMatch disabled in region configuration`. The threat test is not reached. `osReplaceString` (13108–13116) is `VeryLow`, key `osReplaceString`, allowed by ini 102. A negative start counts from the end. Out of range returns the source. `count` is passed to `Regex.Replace`. `count == 0` replaces nothing. `count == -1` replaces every match. `count < -1` throws, and the message is that the count cannot be less than -1. The script dies. A timeout shouts and returns the source. A pattern that does not compile is not caught, so that throw kills the script.

`ScriptRegex.Create` (21–26) sets a 250 ms match timeout. The message is `regex timed out` (23). `Clock.SetSourceForTesting` does not drive that timeout. The oracle case is a pattern the pin's `ScriptRegex` records as `regex timed out`. A pattern that returns on the pin is a normal result.

`iwMatchString` (15636–15672): types -2 include, -1 equal, 0 head, 1 tail, 2 regex anchored with `^` and `$`, 3 count of the escaped pattern, 4 regex count. Types -2, 0 and 1 use culture-sensitive `IndexOf(string)`, `StartsWith(string)` and `EndsWith(string)` (15660–15663). The harness culture is `en-US`. How closely ICU must match is question 5. An empty string returns 1 only when both are empty and the type is at most 1. Otherwise 0. A timeout shouts and returns 0. Types 2 and 4 build the pattern with `ScriptRegex.Create` and do not catch a parse error, so a bad pattern kills the script. Any other type returns 0.

The engine has to do what these four calls record: backtracking, lookaround, the group order `osMatchString` pushes, `$` substitutions in `Replace`, and the 250 ms timeout. That engine is [D131](../DECISIONS.md). This row ports the calls against that crate.

Oracle: `osMatchString` of one match with a group (the value and the index), of a pattern the pin records as `regex timed out` (the shout, an empty list, the script still running), and of a pattern that does not compile (the shout and the stop). `osRegexIsMatch` of a match, of a miss, of a bad pattern (the invalid-expression shout, script still running), and once with `Allow_osRegexIsMatch = false` (the disabled-in-configuration text). `osReplaceString` of a normal replace, of a negative start, of `count` 0 (the source unchanged), of `count` -1, of `count` -2 (the shout and the stop), and of a pattern that does not compile (the stop). `iwMatchString` of each type from -2 to 4, of a timeout, and of a bad pattern on types 2 and 4 (the shout and the stop).

### P4.F1.13b Format strings

Depends on P4.1c, P4.16, P4.19, and on the composite-format port in [D131](../DECISIONS.md) (P4.35 through P4.35c). `iwFormatString` depends on P4.1c and P4.19 and carries `FormatStringThrottle`, which P4.S1 named. It does not call `string.Format`. About a day.

Functions: `osFormatString`, `iwFormatString`.

Source: `LSLSystemAPI.cs` 12324–12327 and 15733–15766. `PhloxEngine.cs` 310.

`osFormatString` (12324–12327) is `VeryLow` under the key `osFormatString`. It calls `string.Format(str, strings.Members.ToArray())` with no catch. The culture is the current culture, which the harness sets to `en-US`. A float element uses `.NET` `ToString`, not six LSL digits. Alignment and format specifiers are the composite-format port in [D131](../DECISIONS.md). A bad format, or a null format string, throws, and the script dies. The shipped `Allow_osFormatString = true` (ini 116) allows it. `Allow_osFormatString = false` is the disabled-in-configuration text.

`iwFormatString` (15733–15766) does not call `string.Format`. It replaces `{0}`, `{1}`, and so on. An empty value leaves `{i}` in place (15747–15748). The loop breaks only when that `{i}` is missing (15750). A value longer than 1024 is cut to 1023 characters (15744). The length check runs after that replacement and before the throttle sleep (15751–15763). `LSLError` and `""` when the result would pass 32768. That path returns before the throttle sleep of that iteration. `LSLError` goes through `ScriptShoutError`, so the 15 ms `ChatSleep` is the last `ScriptSleep` and replaces any 100 ms already written. The recorded wake is 15 ms. When `FormatStringThrottle` is on and `Clock.Now` moves during a replacement that stays under the limit, the call sleeps 100 ms (`PhloxEngine.cs` 310, default true). The harness injects that clock. A null or empty format returns as it is, with no sleep.

Oracle: `osFormatString` of a plain `{0}`, of a float element (the `en-US` `.NET` text, not six LSL digits), of an alignment and a format specifier from the subset question 10 names, and of a bad format (the shout and the stop). `iwFormatString` with the throttle on and the clock advanced (100 ms), with the throttle off (no sleep), a value longer than 1024, an empty value (the `{i}` stays), a missing `{i}`, and a result over 32768 (the `LSL Runtime Error` shout and the 15 ms, with no 100 ms left on the wake). These sleeps do not wait on P4.23.

### P4.F1.14 Alphabets and validate

Depends on P4.1c, P4.19. About a day. The entry point is shared with P4.F1.14b, P4.F1.14c and P4.F1.14d. This row ports the alphabets.

Functions: `iwStringCodec`, for `ascii`, the empty codec, and the default-arm alphabets.

Source: `LSLSystemAPI.cs` 15768–15781 delegates to `Source/Phlox.ScriptEngine/Codecs/HalcyonStringCodec.cs`. `Run` is line 93. `HalcyonStringCodec.cs` carries the Halcyon BSD-style header (1–37) and the 2026 port note (39–47). `CodecUtil` starts at 405. This row uses `HasCodec` (407–419), `Encode`, `Decode`, `DecodeToString` and `Validate` (422–537), the base16, base64 and base4096 bodies (539–707), `StringToAscii` and `AsciiToString` (777–799), and the `ascii`, empty and default arms (126–155, 371–395). An ASCII decode error shouts through `FrameworkText.WithFrameworkMessage` (140). `FrameworkText.cs` is MPL-2.0 (Legion Builds, 2026). A port of that file keeps the notice. The host's `LSLError` shouts `LSL Runtime Error: ` plus the text and takes the 15 ms (`LSLSystemAPI.cs` 277). `ScriptSleep` is the host sleep (15781). A later sleep replaces an earlier one, as section 1 says.

Operations: `OP_DECODE` 0, `OP_ENCODE` 1, `OP_VALIDATE` 2 (95–97). Length sleeps, before the work (99–101): length at least 16000 sleeps `(len / 100) * 20`, at least 8000 sleeps `(len / 100) * 10`, at least 1000 sleeps `(len / 100) * 5`. The codec name is lowercased with the invariant culture (402). The comment says Halcyon ran `en-US`, where that matches.

`HasCodec` accepts `base16`, `uuid`, `base64`, `base64-safe`, `base4096` and `base4k` (407–419). A bad input codec shouts and returns `""` (116–117). An ASCII decode error shouts and then falls out of the catch with no return of its own (138–141), so the method returns `""` at 397. An empty codec shouts `Error: No codec specified for iwStringCodec!` (372). An unknown codec in the default arm shouts and returns `""` (390–393). Validate returns `VALID`, `INVALID`, `PASS`, `FAIL` or `INVALID CODEC` (143–153, 384–388).

Oracle: one encode, one decode and one validate of `base64`, and one of `base16` and of `base4096`. One bad codec name (the shout, `""`, script still running, 15 ms). One string of length 1000 and one of length 8000, with the sleep durations `(1000/100)*5` and `(8000/100)*10`. One `ascii` encode and one `ascii` decode error (the shout, `""`, 15 ms). One empty codec. A length sleep followed by a shout records the 15 ms, because the shout's `ChatSleep` replaces the length sleep. The sleeps do not wait on P4.23.

### P4.F1.14b Hashes

Depends on P4.1c, P4.19, and the alphabet encode in P4.F1.14. About a day.

Functions: `iwStringCodec`, for the hash names.

Source: `HalcyonStringCodec.cs` 184–224 and `CodecUtil.Hash` 877–905. The same BSD-style header as P4.F1.14. A throw that leaves `iwStringCodec` is rewritten by `FrameworkText.WithFrameworkMessage` (82–89). `FrameworkText.cs` is MPL-2.0.

The names are `md5`, `sha1`, `sha-1`, `sha128`, `sha-128`, `sha2`, `sha-2`, `sha256`, `sha-256`, `sha384`, `sha-384`, `sha512` and `sha-512` (184–196). The default output codec is `base16`. A `nonce` is appended as `str + ":" + nonce` (879). `sha-1` and `sha-128` hash as SHA-1. `sha-2` and `sha-256` hash as SHA-256. A bad output codec shouts through `LSLError` and then `Hash` still runs (219–224). `Encode` returns null for an output codec it does not know (477–478), so the call returns null. The comment at 75–76 says the VM refuses to push that null. The script dies after the shout. The 15 ms `ChatSleep` replaces a length sleep written at 99–101.

Oracle: `md5` and `sha256` of a short string, with and without a nonce, output `base16`. One alias pair (`sha1` and `sha-1`, `sha256` and `sha-256`) on the same bytes. A bad output codec: the `LSL Runtime Error` shout, the 15 ms, and the stopped script. The hex is exact.

### P4.F1.14c Gzip and ascii-zip

Depends on P4.1c, P4.19, and the alphabet encode and decode in P4.F1.14. About a day. The two operations do not share a compressor.

Functions: `iwStringCodec`, for `gzip` and `ascii-zip`.

Source: the `gzip` arm is `HalcyonStringCodec.cs` 156–178, and `gzipCompress` / `gzipDecompress` are 712–724. Compression goes through `Source/Phlox.ScriptEngine/Codecs/FrameworkGzip.cs`. That file carries the Microsoft MIT licence (30–51) and the zlib 1.2.1 notice (5–25). `FrameworkGzip.Compress` calls `ZlibDeflate.Compress` (91). `Source/Phlox.ScriptEngine/Codecs/ZlibDeflate.cs` is zlib 1.2.13, Jean-loup Gailly and Mark Adler (1–4). A port of those two files keeps both notices. `ascii-zip` is `HalcyonStringCodec.cs` 179–183 and `AsciiCompress` / `AsciiDecompress` (829–874), a 15-bit codec in the Halcyon file. It does not call `FrameworkGzip` or `ZlibDeflate`. Its notice is the Halcyon BSD-style header. Error text that leaves the call goes through `FrameworkText.cs`, which is MPL-2.0.

`gzip` defaults the output codec to `base4096`. A bad output codec shouts and returns `""` (171–174). An empty string returns as it is (157). `gzipDecompress` throws `ArgumentNullException("buffer", "Buffer cannot be null.")` when the decode is null (722). The outer catch rethrows the Framework wording (82–89), and the script dies. `ascii-zip` of an empty string returns as it is (180).

Oracle: one gzip encode with the default output codec and one decode of that result. One gzip encode with a bad output codec (the shout, `""`, script still running, 15 ms). One gzip decode whose alphabet decode is null (the killed-script shout). One `ascii-zip` encode and the matching decode. The bytes are exact.

### P4.F1.14d AES and the AES key

Depends on P4.1c, P4.19, and the alphabet encode and decode in P4.F1.14. About a day.

Functions: `iwStringCodec`, for `aes` and `aes-key`.

Source: `HalcyonStringCodec.cs` 225–370, `AesEncrypt` and `AesDecrypt` 727–775. The Halcyon BSD-style header. A throw that leaves the call is rewritten by `FrameworkText.WithFrameworkMessage` (82–89). `FrameworkText.cs` is MPL-2.0.

`aes-key` (225–285) rejects a blank password with `LSLError` and returns `""` before the key work (226–229). A blank salt does the same (244–247). No salt parameter throws `ArgumentNullException("value")` (277). A salt under 8 bytes throws `ArgumentException` (278–279). Both leave the call with the Framework message text, and the script dies. Those throws happen before the PBKDF sleep. A present salt of at least 8 bytes derives 32 bytes with PBKDF2-SHA1 (`Rfc2898DeriveBytes.Pbkdf2`, 281). The default rounds are 8 and the default output codec is `base16`. Rounds outside `1 .. 1024` shout and return `""` (261–265). Then `ScriptSleep(Math.Max(100, rounds))` (283). That sleep replaces the length sleep from 99–101. The recorded wake is `Math.Max(100, rounds)`.

`aes` (286–370) defaults the output codec to `base4096` and the key codec to `base16`. A blank key, vector or output codec shouts and returns `""` (319–322). The vector must be 32 hex characters after dashes are removed (324–327). Encrypt and decrypt are CBC with PKCS7 (727–765). A decrypt failure returns `""` (761–764).

Oracle: `aes-key` with a salt of 8 bytes and the default rounds (the key text, wake 100 ms). The same call with rounds 200 (wake 200 ms). A blank password (the shout, `""`, script still running, no PBKDF sleep). No salt, and a salt under 8 bytes (the killed-script shout, whose message is the Framework text, and no PBKDF sleep). One `aes` encrypt and the matching decrypt with a 32-character vector. A short vector (the shout, `""`, script still running). The sleeps do not wait on P4.23.

### P4.F1.15 `osAES`

Depends on P4.1c, P4.16, P4.19. About a day. Ungated.

Functions: `osAESEncrypt`, `osAESDecrypt`, `osAESEncryptTo`, `osAESDecryptFrom`.

Source: `LSLSystemAPI.cs` 11884–11917. The bodies call `Source/OpenSim.Framework/Util.cs` 1370–1479.

The key is SHA-256 of the secret. The IV is MD5 of a random UUID's bytes when no IV string is given, and MD5 of the IV string otherwise (1395–1398). The cipher is CBC with PKCS7. The text is uppercase hex of the IV, a colon, and lowercase hex of the ciphertext (1418). The format string is `:` with no spaces. The comment at 1388–1389 says the separator is ` : `. That comment is stale. An empty secret or an empty plaintext returns `""` from both the API and `Util`, with no shout (11887, 11904, 1392–1393). `osAESEncryptTo` and `osAESDecryptFrom` also return `""` when the IV string is empty (11904, 11913). A result that comes back empty shouts `osAESEncrypt: Failed to encrypt!`, `osAESDecrypt: Failed to Decrypt!`, `osAESEncryptTo: Failed to encrypt!` or `osAESDecryptFrom: Failed to decrypt!` through `ShoutError` and returns `""`. The script keeps running. Hex that does not parse, or text with no colon, is that empty result (1448–1463). The decryptor has no catch (1474–1479). A wrong key or a bad payload throws out of `ReadToEnd`, and the script dies with the shout in section 1.

`osAESEncrypt` uses a random IV, so it is on the non-bit-exact list in section 1. The oracle compares the shape `UPPERHEX:lowerhex` and a decrypt that returns the plaintext. `osAESEncryptTo` with a fixed IV string is deterministic, and that text compares exactly. YEngine's comments on the Phlox side say ungated upstream (11884, 11892, 11902, 11910). With the master switch off the four calls still return.

Oracle: empty secret and empty plaintext (`""`, script still running, no shout). `osAESEncrypt` of a short string (the shape, and `osAESDecrypt` of that text returns the plaintext). `osAESEncryptTo` with a fixed IV (the exact text) and `osAESDecryptFrom` of that pair (the plaintext). A payload with no colon (the failed-decrypt shout, `""`, script still running). A wrong key on a well-formed payload (the killed-script shout). One run with the master switch off still returns.

## 3. Lists and JSON

The 32 `ll` methods are `llListSort`, `llGetListLength`, `llList2Integer`, `llList2Float`, `llList2String`, `llList2Key`, `llList2Vector`, `llList2Rot`, `llList2List`, `llDeleteSubList`, `llGetListEntryType`, `llList2CSV`, `llCSV2List`, `llListRandomize`, `llList2ListStrided`, `llListInsertList`, `llListFindList`, `llParseString2List`, `llDumpList2String`, `llParseStringKeepNulls`, `llListReplaceList`, `llListStatistics`, `llJsonGetValue`, `llJsonValueType`, `llJsonSetValue`, `llList2Json`, `llJson2List`, `llListFindStrided`, `llList2ListSlice`, `llSortListStrided`, `llListFindListNext`, `llListSortStrided`.

The 9 `os` methods are `osOldList2ListStrided`, `osListFindListNext`, `osListSortInPlace`, `osListSortInPlaceStrided`, `osListAsFloat`, `osListAsInteger`, `osListAsString`, `osListAsVector`, `osListAsRotation`.

The 6 `iw` methods are `iwParseString2List`, `iwMatchList`, `iwListIncludesElements`, `iwReverseList`, `iwListRemoveElements`, `iwListRemoveDuplicates`.

### P4.F2.1 List accessors

Depends on P4.1c, P4.19. The `osListAs*` names also depend on P4.16.

Functions: `llGetListLength`, `llList2Integer`, `llList2Float`, `llList2String`, `llList2Key`, `llList2Vector`, `llList2Rot`, `llGetListEntryType`, `osListAsFloat`, `osListAsInteger`, `osListAsString`, `osListAsVector`, `osListAsRotation`.

Source: `LSLSystemAPI.cs` 16050–16134 and 12346–12377. `LSLList.cs` 391–463. `Encoding.cs` 45–118. `SlConst.cs` 409–415.

`llGetListLength` is 0 for a null list (16050). The six `llList2*` getters (16056–16095) add the length to a negative index once. `GetLSLIntegerItem` (414–434): an int is itself, a float goes through `FloatToInteger`, a string goes through `CastToInt` (so `"0x1F"` is 31, as the comment at 16051–16055 says), and anything else is 0. Out of range is 0. `GetLSLFloatItem` (391–411): an int widens, a string uses `CastToFloat` (`en-US`, overflow `+infinity` or `-infinity` as section 1 says), and anything else is 0. `GetLSLStringItem` (437–463): a string is itself, a float, vector or rotation is six digits, and anything else is `ToString()`. `llList2Key` (16074–16082) returns a string element as it is, including a string that is not a UUID, and `""` for any other type or an out-of-range index. `llList2Vector` and `llList2Rot` use `GetVector3Item` and `GetQuaternionItem`. A bad string is the zero vector or `Quaternion.Identity`.

`llGetListEntryType` (16119–16134): out of range or empty is 0. An int is `TYPE_INTEGER` (1). A float or a double is `TYPE_FLOAT` (2). A string that `UUID.TryParse` accepts is `TYPE_KEY` (4). Any other string is `TYPE_STRING` (3). A `UUID` is `TYPE_KEY`. A `Vector3` is `TYPE_VECTOR` (5). A `Quaternion` is `TYPE_ROTATION` (6). A nested `LSLList` is `TYPE_INVALID` (0). An LSL script cannot build that nested element, so it is not an oracle case. Anything else returns 3. The comment says the default is string.

`osListAs*` (12346–12377) do not coerce and do not wrap a negative index. A negative index, an out-of-range index, or a wrong type returns the typed zero: `0`, `0.0`, `""`, the zero vector, or `Quaternion.Identity`. Ungated.

Oracle: one list that holds an int, a float, a float string, `"0x1F"`, a key string, a non-key string, a vector and a rotation. Each getter at 0, at -1, and past the end. `llList2String` of the float, the vector and the rotation is the six-digit list form. `llList2Integer` of NaN and of `1e10` is -2147483648. `llGetListEntryType` of each element. `osListAsInteger` of the float element is 0. `osListAs*` at -1 is the typed zero. One `osListAs*` call with the master switch off still returns.

### P4.F2.2 Structural edits

Depends on P4.1c, P4.19. `osOldList2ListStrided` also depends on P4.16.

Functions: `llList2List`, `llDeleteSubList`, `llListInsertList`, `llListReplaceList`, `llList2ListStrided`, `llList2ListSlice`, `osOldList2ListStrided`.

Source: `LSLSystemAPI.cs` 16097–16117, 16211–16239, 16301–16321, 16363–16388, 11992–12025.

`llList2List` and `llDeleteSubList` use the exclusion range (section 1). A null source is an empty list. `llListInsertList` (16228–16239) adds the length to a negative start once, clamps into `[0, length]`, and inserts. An empty source returns the destination. `llListReplaceList` (16363–16388) uses the exclusion range. An inverted range keeps what lies between end and start and puts the source after it.

`llList2ListStrided` (16211–16226): a start past the end is not an exclusion range. It behaves as start 0 and end -1. A stride under 1 becomes 1. The selected indexes are the multiples of the stride inside the range. `llList2ListSlice` (16301–16320) does use the exclusion range, then takes `slice_index` of each stride. A negative `slice_index` counts from the stride's end. An index outside `-stride .. stride-1` returns `[]`. The comment says YEngine differs: the whole list for an inverted range, and strides counted from 0. Recorded.

`osOldList2ListStrided` (11992–12025) is the pre-2010 wrap. A start past the end is two passes. The stride is counted from index 0 of the whole list, not from the start. A stride of 0 becomes 1. A negative stride becomes its absolute value. Equal start and end select every multiple of the stride in the whole list. Ungated.

Oracle: a seven-element list. `llList2List` and `llDeleteSubList` with a normal range, a negative end, and an inverted range. `llListInsertList` at 0, at -1, and past the end. `llListReplaceList` with an inverted range. `llList2ListStrided` with start past end, and with stride 0. `llList2ListSlice` of the five ranges the comment at 16307–16309 names, plus a `slice_index` outside the stride. `osOldList2ListStrided` with start past end, so the two-pass order is visible. Types and order are exact.

### P4.F2.3 Search

Depends on P4.1c, P4.19. `osListFindListNext` also depends on P4.16.

Functions: `llListFindList`, `llListFindListNext`, `llListFindStrided`, `osListFindListNext`.

Source: `LSLSystemAPI.cs` 16241–16299 and 12033–12073.

All four compare `ToString()` of the elements, not the six-digit LSL string, and return -1 when there is no match. `llListFindList` (16241–16255) returns -1 when the test is empty or longer than the source. `llListFindListNext` (16257–16273) clamps a negative offset up to 0. It does not count from the end. `llListFindStrided` (16275–16299) adds the length to a negative bound once, forces a stride under 1 up to 1, and returns -1 when the test is longer than the stride. The walk starts at the clamped start and steps by the stride.

`osListFindListNext` (12033–12073) is ungated. An empty source returns 0 when the test is also empty, and -1 otherwise. An empty test returns the instance index, or that index counted from the end when the instance is negative. A negative start or end counts from the end. A negative instance counts from the last hit. No hit is -1. The compare is the same `ToString()` compare.

Oracle: a list with an int 1 and a float 1.0, so the `ToString()` match is what the recording shows. A missing test returns -1. An empty test on `llListFindList` returns -1. `llListFindListNext` with a negative offset starts at 0. `llListFindStrided` with a stride shorter than the test returns -1. `osListFindListNext` with an empty test, with instance 0, and with instance -1. One call with the master switch off still returns.

### P4.F2.4 Sort

Depends on P4.1c, P4.19. `osListSortInPlace` and `osListSortInPlaceStrided` also depend on P4.16. About a day. The order is exact only when the port uses the same sort .NET uses.

Functions: `llListSort`, `llSortListStrided`, `llListSortStrided`, `osListSortInPlace`, `osListSortInPlaceStrided`.

Source: `LSLSystemAPI.cs` 16159–16187, 16323–16361, 21554–21555, 12076–12108. `LSLList.Sort` is `Source/InWorldz.Phlox/Types/LSLList.cs` 534–602. YEngine `llListSort` is `src.Sort(stride, ascending == 1)` (`LSL_Api.cs` 6247–6249). That `src` is YEngine's own `LSL_Types.list`, not Phlox's `LSLList`. YEngine has `llListSortStrided` (6252–6254) and no `llSortListStrided`.

`llListSort` (16159–16187) does not call `LSLList.Sort`. It stringifies element 0 of each stride with `ToString()`, tries `double.TryParse` with the current culture, and compares the numbers when both parse. Otherwise it compares the strings ordinally. The harness culture is `en-US`. How closely ICU must match is question 5. A non-zero `ascending` sorts ascending, including the value 2. A stride under 1 becomes 1. When the length is not a multiple of the stride, the tail is dropped. The chunks are sorted with `List<T>.Sort` (16173–16182). That is .NET's introspective sort (`ArraySortHelper`). It is not stable. Equal keys, and a mixed int and string list whose comparison is not transitive, come out in the order that algorithm gives, including its insertion-sort threshold and its pivot choice. A stable `sort_by` will not match. The row ports that introsort. `LSLList.Sort` (534–602) is a different function. No API path at the pin calls it. Its homogeneous path uses `Array.Sort` (564–571). Its bubble path, used when the stride is not 1 or the types differ, reads `ret[j + k]` past the end when the stride does not divide the length and can throw (584–598). The in-place comment at 12081 says the order cannot drift from LSL's. The method it calls is `llListSort`, not `LSLList.Sort`. Proposed TB169. The default under [D124](../DECISIONS.md) is the API method. Question 1. YEngine uses `src.Sort` and treats only `ascending == 1` as ascending (6247–6249). Recorded.

`llSortListStrided` (16323–16361) returns the same list when the length is not a multiple of the stride and the stride is over 1. A `stride_index` outside `[-stride, stride)` returns a new empty list. Index 0 delegates to `llListSort` (16334–16335), so any non-zero `ascending` sorts ascending, including 2. Indexes other than 0 use `ascending == 1` (16356). Any other value, including 2 and 0, sorts descending. Those indexes compare ints, floats and doubles by value, and everything else by ordinal `ToString()`, and they sort with `List<T>.Sort` (16346–16356), the same unstable introsort. `llListSortStrided` forwards to it (21554–21555).

`osListSortInPlace` (12083–12088) and `osListSortInPlaceStrided` (12103–12108) are the Phlox master switch. YEngine does not gate them (6504–6512) and treats only `ascending == 1` as ascending. Recorded. They sort the caller's `LSLList` and write the result back with `SortInPlace` (12096–12099), which copies `src.Data.Length` elements. `Array.Copy` throws `ArgumentException` when the sorted list is shorter. Executed on .NET 10. `llListSort` is shorter when the stride does not divide the length. `llSortListStrided` is empty when the stride index is outside the range (16332–16333), and `osListSortInPlaceStrided` calls `llListSortStrided`, which forwards there. The script dies. Proposed TB168. When the stride divides the length and the index is in range, both stores (`Data` and `Members`) take the new order, so a later `llList2String` sees it.

Oracle: `llListSort` of a mixed int and string, stride 1, ascending 1 and ascending 2, with enough equal keys that a stable sort and the introsort disagree. A stride that does not divide the length, so the tail is gone. The pass is the API method's recording, which is the introsort's order. `llSortListStrided` with `stride_index` 0 and `ascending` 2 (ascending, because it delegates), with `stride_index` 1 and `ascending` 2 (descending), with `stride_index` 1 and `ascending` 1, with a bad index (empty list), and with a length that is not a multiple (the same list). `osListSortInPlace` on a dividing stride, then `llList2String` of element 0, so the in-place write is visible. The same call with a stride that does not divide: the shout and the stop. `osListSortInPlaceStrided` with a bad stride index: the shout and the stop. One allowed call with the master switch on, and one denial with it off (the master-switch text).

### P4.F2.4b Statistics and randomize

Depends on P4.1c, P4.19. About a day.

Functions: `llListStatistics`, `llListRandomize`.

Source: `LSLSystemAPI.cs` 16189–16208 and 16391–16437. Constants: `DefaultConstants.cs` 577–587 and the copies at `LSLSystemAPI.cs` 16391–16393.

`llListStatistics` (16401–16437): `LIST_STAT_RANGE` 0 through `LIST_STAT_GEOMETRIC_MEAN` 9, and `LIST_STAT_HARMONIC_MEAN` 100 (`DefaultConstants.cs` 577–587). Each element goes through `Convert.ToDouble` (16405–16407). An int or a float is counted. A string that parses is counted, so the string `"3"` counts as 3. Under `en-US`, `Convert.ToDouble` accepts a thousands separator, so `"1,000"` counts as 1000. A string that does not parse throws and the catch drops it. The harness culture is `en-US`. How closely that parse must match is question 5. An empty list or a list with no numbers returns 0. `STD_DEV` of fewer than two numbers is 0. The geometric mean is `exp(log(product) / n)` and is NaN when the product is negative. The harmonic mean is `n / sum(1/x)`. An unknown operation returns 0. The oracle uses the constant name, which compiles to 100, not the number 10.

`llListRandomize` (16189–16208) is a Fisher-Yates of stride chunks on `new Random()`. A stride of 0 or less becomes 1. When the length equals the stride, or the stride does not divide the length, the result is a copy in the same order. Not bit-exact.

Oracle: `llListStatistics` of each constant from `RANGE` through `GEOMETRIC_MEAN`, of `HARMONIC_MEAN`, of an unknown operation, of a one-element `STD_DEV`, of a negative product, of a list that contains the string `"3"` (counted), of the string `"1,000"` (counted as 1000), and of a string that does not parse (skipped). `llListRandomize` with a stride that does not divide (order unchanged) and with a dividing stride (a permutation of the chunks, type and length exact).

### P4.F2.5 List text

Depends on P4.1c, P4.19.

Functions: `llList2CSV`, `llCSV2List`, `llDumpList2String`, `llParseString2List`, `llParseStringKeepNulls`.

Source: `LSLSystemAPI.cs` 16136–16157, 16439–16451, 16613–16616, 19490–19515.

`llList2CSV` (16136–16143) joins `GetLSLStringItem` with `", "`. A null or empty list is `""`. `llDumpList2String` (16439–16447) joins the same strings with the separator. A null separator is `""`. `llCSV2List` (16151–16157) splits on `,` and trims each piece. An empty source is an empty list. The pieces are strings.

`ParseString2List` (19490–19515): a null source is `[]`. Separators are consumed. Spacers are kept. An empty separator or spacer string is skipped. The search is ordinal `IndexOf`. `keepNulls` false drops empty tokens. `llParseString2List` calls it with `keepNulls` false (16449–16451). `llParseStringKeepNulls` calls it with true (16613–16615).

Oracle: a list of an int, a float and a vector through `llList2CSV` and `llDumpList2String`. The float and the vector are six digits. `llCSV2List` of `""`, of `"a, b"`, and of a trailing comma. `llParseString2List` and `llParseStringKeepNulls` on `"a,,b"` with separator `","`, and once with a spacer that sits between two separators. Element types stay strings. Order is exact.

### P4.F2.6 JSON getters

Depends on P4.1c, P4.19.

Functions: `llJsonGetValue`, `llJsonValueType`, `llJson2List`.

Source: `LSLSystemAPI.cs` 17766–17768, 17770–17783, 17785–17798, 17802–17821, 17832, 17875–17883, 18204–18243. `JSON_APPEND` is -1 (`SlConst.cs` 117).

The constants (17875–17883) are U+FDD0 through U+FDD8 in the order invalid, object, array, number, string, null, true, false, delete. `TryJsonFind` (17802–17821) uses `System.Text.Json`, strips a leading BOM (17839), indexes an array with an integer specifier, and names an object member with a string specifier. Anything else, or a miss, is `JSON_INVALID`. `TryGetProperty` returns the last value when a key is repeated (17815). `llJsonGetValue` (17770–17783) returns a string unquoted, `JSON_TRUE`, `JSON_FALSE` or `JSON_NULL` for those kinds, and the raw JSON text for a number, an object or an array. `GetString()` is not caught (17777). A string whose JSON contains a lone `\ud800` parses, and `GetString()` throws `InvalidOperationException`. The script dies. `llJsonValueType` (17785–17798) returns the matching constant and does not call `GetString()`.

`JsonDocument.Parse` is called with no `MaxDepth` (17832, 17896, 18005). The runtime default depth is 64. Depth 65 throws `JsonException`. `TryJsonParse` (17832) turns that into a failed parse: `llJsonGetValue` and `llJsonValueType` return `JSON_INVALID`, and `llJson2List` returns a one-item list of the BOM-stripped source (18213). The setter's parse of the document (17896) is inside the catch that returns `JSON_INVALID` (17904). A framed value nested past the depth (18005) falls through its catch and is quoted as a string. When P4.F2.6 and P4.F2.7 land they add the limit id `script.json-depth` to [LIMITS.md](../LIMITS.md), value 64, hard. The id is not filed yet.

`llJson2List` (18210–18229): an empty string is an empty list. Text that is not JSON is a one-item list of the BOM-stripped source. An array becomes its elements. An object becomes key, value, key, value. A nested object or array is that element's raw JSON text. An integer literal that fits in int32 stays an int. `"1.0"` fails `TryGetInt32` and becomes a float (`JsonElementToLSL`, 18236–18237). Any other number is `GetSingle()`. true, false and null become the JSON constants. A string is unquoted. `GetString()` at 18238 has the same uncaught lone-surrogate throw.

Oracle: `llJsonGetValue("true", [])` is `JSON_TRUE`. A string value comes back unquoted. An integer specifier on an array, a string specifier on an object, a specifier of the wrong kind (`JSON_INVALID`), a BOM-prefixed document, a missing member, and a repeated key (the last value). `llJsonValueType` of each kind. A document nested 64 deep parses. A document nested 65 deep is `JSON_INVALID` from the getters and the one-item list from `llJson2List`. `llJson2List` of `""`, of `not json`, of an array of the integer literal `1` and of the number `1.0` (a float), of a number that does not fit in int32, of an object, and of a nested array (the nested text is one string element). `llJsonGetValue` and `llJson2List` of a string with a lone `\ud800` escape (the shout and the stop). Types and order are exact. The constants compare as the exact code points.

### P4.F2.7 JSON setters

Depends on P4.1c, P4.19.

Functions: `llJsonSetValue`, `llList2Json`.

Source: `LSLSystemAPI.cs` 17885–18201. `JSON_APPEND` is -1 (`SlConst.cs` 117).

`llJsonSetValue` (17885–17904): an empty specifier list returns `JSON_INVALID`. A BOM is stripped. An empty document is built from the specifiers. An integer specifier starts an array, so `llJsonSetValue("", [0], "x")` is `["x"]`. A string specifier starts an object. A parse failure, including depth 65 (`script.json-depth` in P4.F2.6), or a failed set, returns `JSON_INVALID`. The mutable tree keeps object member order (17912, 17956–17969). `JSON_APPEND` (-1) appends to an array (18034–18038). An index equal to the length appends. An index under 0 or past the length returns `JSON_INVALID`, except the append value. `JSON_DELETE` removes an array element or an object member (18023, 18049, 18073). A child of the wrong shape is replaced by the rest of the path (18058–18061, 18081–18085).

`JsonValueToNode` (17992–18011): null becomes JSON null. `JSON_TRUE`, `JSON_FALSE`, `JSON_NULL` and the words `true`, `false` and `null` become literals. A framed object or array that parses is kept. A JSON number (`JsonNumber`, 17871–17872) is written bare. Anything else is quoted by `JsonQuote` (17845–17867), which escapes quotes, backslashes and controls and writes other characters as themselves.

`llList2Json` (18133–18166): `JSON_ARRAY` or `JSON_OBJECT`, and any other type is `JSON_INVALID`. An object of odd length, or a key that is not a string, is `JSON_INVALID`. `JsonValueFromObject` (18170–18192): an int is invariant digits. A float or double NaN is `"NaN"`. Infinities are `"Inf"` and `"-Inf"`. A finite float uses invariant `ToString`. A string is trimmed. true, false, null and the three JSON constants become literals. A framed valid object, array or quoted string is kept (`IsFramedJson`, 18196–18201). Any other string is quoted. A vector or rotation is quoted from its six-digit list string.

Oracle: set into an empty string with index 0 and with a key. Set a nested path. Append with `JSON_APPEND`. Delete an element and delete a missing key. A bad document returns `JSON_INVALID` and the script keeps running. A document nested 65 deep returns `JSON_INVALID`. `llList2Json` of an array, of an odd-length object, of a non-string key, of true, false and null, of a framed object, of a JSON number string (quoted, because a number string is quoted), of NaN, and of both infinities. The text is exact.

### P4.F2.8 `iwParseString2List`

Depends on P4.1c, P4.19. About a day by itself.

Functions: `iwParseString2List`.

Source: `LSLSystemAPI.cs` 16453–16611.

An empty source returns `[]` (16456). Both separator and spacer lists empty returns `[]` even when the source is not empty (16511–16513). That is the behaviour this row reproduces. Question 8. A spacer that equals a separator is dropped (16496–16505). The walk uses culture-sensitive `src.IndexOf(ds)` from the start of the remaining source (16530). On .NET 10 with ICU, `"a\u200bb".IndexOf("ab")` is 0 and the ordinal search is -1. The harness culture is `en-US`. How closely that ICU result must match is question 5. Equal indexes keep the first delimiter in the combined array. A leading or doubled separator always keeps an empty field, even when `keepnulls` is false (16552–16556). The comment cites Halcyon.

The args walk pairs (16467–16493). Names are lowercased. `keepnulls` is true only when the integer is 1. `trimstrings` outside `0 .. 3` becomes 0. `maxsplits` under 0 becomes 0. `capitalize` outside `0 .. 2` becomes 0. `autocast` outside `1 .. 2` becomes 0. `ParseString2ListApply` (16588–16594) trims with `llStringTrim` and then `ToUpper` or `ToLower` (current culture, `en-US` on the harness). `AutoCastString` (16598–16611): one dot tries a float, zero dots try an int, angle brackets with two commas try a vector and with three try a rotation, and length 36 tries a UUID which is stored back as a string. `TryParse` uses the current culture. A field taken before the tail is cast when `autocast` is greater than 0 (16549). A spacer is cast only when `autocast` is 2 (16569). The tail is cast only when `autocast` is greater than 1 (16582), so `autocast` 1 leaves the final field a string. A field that fails the parse stays a string.

Oracle: an empty source. A non-empty source with both delimiter lists empty (the empty list). A leading separator with `keepnulls` 0 (the empty field is kept). One spacer that equals a separator (the spacer is not emitted). `maxsplits` of 1. `trimstrings` 3 and `capitalize` 1. `"a\u200bb"` with separator `"ab"` (the pin's `en-US` index). `autocast` 1 on an int, a float, a vector, a rotation and a 36-character UUID, with the last field still a string. `autocast` 2 on that same tail and on a spacer, both cast. A value that does not parse stays a string. Types and order are exact.

### P4.F2.9 List match and edit

Depends on P4.1c, P4.19.

Functions: `iwMatchList`, `iwListIncludesElements`, `iwReverseList`, `iwListRemoveElements`, `iwListRemoveDuplicates`.

Source: `LSLSystemAPI.cs` 16617–16758.

`listCompare` (16617–16632) requires equal length. Different lengths return 0. A type of 0 on either side, or unequal types, returns 0. Type 1 compares integers by value. Type 2 compares floats with `!=`, so a NaN fails the compare. Types 3 and 4 compare `GetLSLStringItem`, which for a string or a key is the string itself. Types 5 and 6 compare the vector and the quaternion with `!=`. The six-digit form is what a float, vector or rotation becomes when it is read as a string. The float arm does not use that text.

`iwMatchList` (16635–16665): both empty and type at most 1 returns 1. One side empty returns 0 for those types, and 0 for every other type. Type -1 is equal. Type 0 is a head. Type 1 is a tail. Types 2, 3 and 4 shout `IW_MATCH_REGEX not implemented for iwMatchList.` (and the count wordings at 16659 and 16662) through `ScriptShoutError`, so the 15 ms applies, and then return 0. The script keeps running.

`iwListIncludesElements` (16667–16690) uses `object.Equals`. An int 1 does not match a float 1.0. Empty on either side returns 0. `any == 1` returns 1 on the first hit and 0 when none hit. Any other `any` returns 1 only when every element is found.

`iwReverseList` (16692–16706): length 0 or 1 returns the same list. A stride under 1 reverses the whole list (16695). A length that is not divisible by the stride shouts `Error: stride argument is {n}, but source list length is not divisible by {n}` and returns `[]` (16696–16699). Otherwise the blocks stay in their own order and the blocks run last to first (16701–16704). `[1, 2, 3, 4]` with stride 2 is `[3, 4, 1, 2]`.

`iwListRemoveElements` (16708–16753): empty on either side returns the source. A count of 0 becomes -1, which means all. Mode 0 looks for a contiguous run equal to `elements`, using `Equals`, and removes up to `count` runs. The scan stops at `src.Length - elements.Length`, so a non-matching tail shorter than the pattern is left out of the result. Any other mode keeps an element that is not in `elements`, and only while the kept count is under `count` (or while `count` is -1). A matching element is dropped. Once `count` non-matches have been kept, the rest of the list is dropped too. This plan describes that control flow and reproduces it. Question 9. It is not a ledger row.

`iwListRemoveDuplicates` (16755–16758) returns the source when the length is 0 or 1, and otherwise `Distinct()`.

Oracle: `iwMatchList` of two empty lists, of a head, of a tail, of unequal types, of a float NaN, and of type 2 (the shout, 0, script still running, 15 ms). `iwListIncludesElements` of an int against a float, of `any` 1, and of `any` 0. `iwReverseList` of `[1, 2, 3, 4]` with stride 2 (`[3, 4, 1, 2]`), with stride 0 (the whole list reversed), and with a stride that does not divide (the shout and `[]`). `iwListRemoveElements` mode 0 of one run, mode 0 where the pattern is longer than the unmatched tail, mode 1 with count 1 (the tail drop), and count 0 (every match removed). `iwListRemoveDuplicates` of a list with two equal strings and of a one-element list (the same list). Order and types are exact.

## 4. Proposed ledger rows

The ledger's highest filed id is TB164. TB165 and TB166 are unused. This file does not take them. The six proposals are TB167 through TB172. They are not filed here. The maintainer adds the ledger rows. Ferroan reproduces Phlox on each of them until a decision says otherwise. None of them is a viewer protocol.

| # | C# | What goes wrong | Ferroan does |
| --- | --- | --- | --- |
| TB167 | `LSLSystemAPI.cs` `osStringLastIndexOf` (12521–12527) at `c401bceecd` | The five-argument form passes `offset` and a forward `count` to `String.LastIndexOf`, which searches backward and throws `ArgumentOutOfRangeException` when `offset - count + 1 < 0`. Offset 0 and count 0 set the count to the whole length and throw on a string longer than one character. It throws for any offset in the first half when the count is 0 or less. The three-argument form does not. The script dies with the shout in section 1. Executed on .NET 10. YEngine has the same body (`OSSL_Api.cs` 5446–5467). | Reproduce the throw and the stop (P4.F1.12). |
| TB168 | `LSLSystemAPI.cs` `SortInPlace` (12096–12099), `osListSortInPlace` (12083–12088), `osListSortInPlaceStrided` (12103–12108) at `c401bceecd` | `Array.Copy` asks for `src.Data.Length` elements and throws `ArgumentException` when the source is shorter. Executed on .NET 10. `llListSort` drops the tail when the stride does not divide the length (16159–16187). `llSortListStrided` returns an empty list when the stride index is outside `[-stride, stride)` (16332–16333), and the strided in-place call goes through `llListSortStrided` (21554). The script dies. | Reproduce the throw and the stop (P4.F2.4). |
| TB169 | `LSLSystemAPI.cs` `llListSort` (16159–16187) and `LSLList.Sort` (`LSLList.cs` 534–602) at `c401bceecd` | The API method stringifies, parses with the current culture, drops a tail that is not a whole stride, treats any non-zero `ascending` as ascending, and sorts with `List<T>.Sort`. It does not call `LSLList.Sort`. No API path calls `LSLList.Sort`. That bubble reads `ret[j + k]` past the end when the stride does not divide the length and can throw (584–598). The comment at 12081 says the in-place order cannot drift from LSL's. YEngine calls `src.Sort` on its own list type (`LSL_Api.cs` 6247–6249), not on Phlox's `LSLList`. | Reproduce the API method (P4.F2.4). Question 1. |
| TB170 | `LSLSystemAPI.cs` `llComputeHash` (15943–15984) and `llHMAC` (15990–16022) at `c401bceecd` | The comment at 15945 lists `sha224`. Neither switch has a `sha224` case. An unknown name shouts and returns `""`. YEngine implements `sha224` (`LSL_Api.cs` 21692–21697 and 22330–22331), is case-sensitive, and returns `""` with no shout. | Reproduce the shout and the empty string (P4.F1.9). Question 2. |
| TB171 | `LSLSystemAPI.cs` `llComputeHash` `md5_sha1` (15970–15978) and `llHMAC` (15990–16022). Signatures `ISystemAPI.cs` 970 and `ILSLSystemAPI.cs` 801. YEngine `ILSL_Api.cs` 549 and `LSL_Api.cs` 21673–21684, 22314–22359. At `c401bceecd` | Phlox `md5_sha1` is SHA-1 of the lowercase hex MD5. YEngine concatenates the raw MD5 bytes and the raw SHA-1 bytes of the input. Phlox HMAC takes `(msg, privateKey, algorithm)` and returns base64. SL's order is the key first and its output is base64 (15992). YEngine takes `(private_key, message, algo)` and returns lowercase hex. | Reproduce Phlox (P4.F1.9). Question 3. |
| TB172 | `LSLSystemAPI.cs` `osMax` (12406–12409) at `c401bceecd` | The gate key is `osGetRezzingObject`, threat `None`. The comment says upstream copy-pastes that key. YEngine `osMax` is ungated (`OSSL_Api.cs` 4542–4545). The threat check with that key belongs to `osGetRezzingObject` (4547–4549). The gate is Phlox's port. `osMin` is ungated (12412–12415). With the shipped file the call is allowed, because the `Allow_` line is commented out (`osslDefaultEnable.ini` 85) and `None` passes the threat test. `Allow_osMax` is not consulted. `Allow_osGetRezzingObject = false` is the disabled-in-configuration text. The threat test is not reached, and threat `None` cannot fail it. | Reproduce the key (P4.F1.11). Question 4. |

`llXorBase64Strings` is not a ledger row. The padding and the 300 ms sleep are the documented behaviour (`LSLSystemAPI.cs` 15856–15860).

## 5. Open questions

The earlier questions in [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 6 stay as they are. These are new.

1. List sort. P4.F2.4 reproduces `llListSort` as written (TB169), including .NET's unstable introsort. `LSLList.Sort` is a different function. No API path at the pin calls it. With a stride that does not divide the length, its bubble reads `ret[j + k]` past the end and can throw (`LSLList.cs` 584–598). YEngine calls `src.Sort` on its own `LSL_Types.list` (`LSL_Api.cs` 6247–6249), not on Phlox's `LSLList`. [D124](../DECISIONS.md) means the API method. This plan does not switch.
2. `sha224` and case. P4.F1.9 reproduces Phlox: lowercase the name, no `sha224`, shout and `""` (TB170). YEngine implements `sha224`, is case-sensitive, and is silent.
3. Later fix, not a stage-2 question. P4.F1.9 reproduces Phlox's positional `(msg, privateKey, algorithm)`, base64 output, and SHA-1 of the hex MD5 (TB171), on both tables, under [D124](../DECISIONS.md). SL takes the key first and returns a base64 HMAC (`LSLSystemAPI.cs` 15992). An SL script that calls `llHMAC(key, msg, algo)` is keyed on the message. YEngine takes `(private_key, message, algo)` and returns lowercase hex (`LSL_Api.cs` 22359). Changing the argument order or the output is a behaviour decision of its own. This plan does not make it.
4. `osMax` gate key. P4.F1.11 reproduces Phlox's key `osGetRezzingObject` (TB172). YEngine does not gate `osMax` (`OSSL_Api.cs` 4542–4545). The other choice is the key `osMax`.
5. ICU collation. The harness and Ferroan use `en-US`, which is what the pin sets. `Source/OpenSim.Server.RegionServer/RegionRuntime.cs` 74–77 applies `ProcessSetupOptions` and leaves `SetCurrentCulture` and `SetDefaultCurrentCulture` at their default true (`Source/OpenSim.Server.Base/Hosting/ProcessSetupOptions.cs` 18–19). `ProcessSetupService.Apply` calls `Culture.SetCurrentCulture` and `SetDefaultCurrentCulture` (`Source/OpenSim.Server.Base/Hosting/ProcessSetupService.cs` 29–35). `Source/OpenSim.Framework/Culture.cs` 84 and 99–107 set `CultureInfo("en-US", false)` on the thread and as `DefaultThreadCurrentCulture`. The calls that read that culture are `llToUpper`, `llToLower`, `double.TryParse` in `llListSort`, `Convert.ToDouble` in `llListStatistics` (including the thousands separator in `"1,000"`), `string.Format` in `osFormatString`, `osStringStartsWith` and `osStringEndsWith` (always `CurrentCulture`), `osUnixTimeToTimestamp`'s custom format, `TryParse` in `iwVerifyType` and `iwParseString2List`, `iwMatchString` types -2, 0 and 1, and `iwParseString2List`'s `src.IndexOf`. On .NET 10 those `en-US` comparisons follow ICU. The open question is how closely Ferroan must match that ICU collation, per function.
6. Float pass. Text a script sees, in chat or in a string cast, compares exactly. A float said with `(string)` is six digits. A vector or rotation said with `(string)` is five. A raw float result compares within the named `libm` ulp in section 4 of [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md). A 1 ulp difference that leaves the said text unchanged still matches that text.
7. Uncaught throws. Section 1 kills the script and shouts. The calls are `llModPow` with modulus 0, `osFormatString` of a bad format, `osMatchString` of a pattern that does not compile, `osReplaceString` of a pattern that does not compile, `osReplaceString` with `count < -1`, `iwMatchString` types 2 and 4 with a bad pattern, `iwSubStringIndex` with an offset past the end, `iwIntRandRange` with upper bound 2147483647, `llXorBase64StringsCorrect` when the second string is non-empty and decodes to `""`, `llJsonGetValue` or `llJson2List` of a lone `\ud800` escape, `iwStringCodec` `aes-key` with no salt, `iwStringCodec` `aes-key` with a salt under 8 bytes, `iwStringCodec` of a hash whose output codec makes `Hash` return null, `gzip` decompress of a null buffer, and the copy and `LastIndexOf` throws (TB167, TB168). `llEscapeURL` and `llUnescapeURL` return normally. A null string is not an oracle case. The other choice is to catch and return a value. This plan reproduces the kill.
8. `iwParseString2List` with both delimiter lists empty returns `[]` for a non-empty source (16511–16513). P4.F2.8 reproduces that.
9. `iwListRemoveElements` mode other than 0 drops matches and also drops the tail once `count` non-matches have been kept. Mode 0 leaves out a non-matching tail shorter than the pattern (16708–16753). P4.F2.9 reproduces that control flow.
10. Regex engine and composite formatting. Decided by the maintainer on 26 November 2026. [D131](../DECISIONS.md). Ferroan ports the needed part of .NET's `System.Text.RegularExpressions` into its own crate: exact matches, the group order `osMatchString` pushes (each successful group, then its index), `$` substitutions in `Replace`, the 250 ms match timeout (`ScriptRegex.cs` 21–26), and .NET's error text. The same decision ports the needed part of `string.Format` for `osFormatString` (alignment and format specifiers). The rows are P4.34 through P4.34f and P4.35 through P4.35c. Each derived file keeps the original copyright line and the MIT notice, and NOTICE.md records the origin when the code lands. The `regex` crate, `fancy-regex` and `regexr` are not that engine.

`osPerlinNoise2D` is a pure function of its arguments (`TerrainUtil.cs` 110–121). It stays in P4.F1.11. The 24 `iw` names that leave P4.S27 are the 18 in section 2 and the 6 in section 3. The other 61 stay on P4.S27, including `iwFormatTime`.
