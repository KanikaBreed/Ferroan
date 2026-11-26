# Phase 4 pure functions

Planning only. This file splits the two pure families from [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 2 into day-sized rows. It does not add a `TASKS.md` row or a `WORK_QUEUE.md` row, and it does not edit the ledger. The pin is `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. [D124](../DECISIONS.md) is settled: Phlox is the reference for the queue, the sleeps, and the 85 `iw*` functions. A YEngine difference is recorded here and is not the pass rule.

The placeholders this file replaces, when the queue is next edited, are P4.S1 (64), P4.S2 (32), the pure part of P4.S13 (47), and the pure names inside P4.S27. Stage 2 may split a family. It does not move a method into a second family and add it again. An id is a label. Each row below is about a day. A row that overruns a day is split before the next id starts.

## Counts

| Family | Methods | Rows |
| --- | --- | --- |
| `ll` math, strings and crypto | 64 | P4.F1.1–P4.F1.10 |
| `ll` lists and JSON | 32 | P4.F2.1–P4.F2.7 |
| `os` math, strings and crypto | 38 | P4.F1.3, P4.F1.5, P4.F1.9–P4.F1.13 |
| `os` lists | 9 | P4.F2.1, P4.F2.2, P4.F2.3, P4.F2.4 |
| `iw` math and strings | 18 | P4.F1.6, P4.F1.7, P4.F1.9–P4.F1.14 |
| `iw` lists | 6 | P4.F2.8, P4.F2.9 |

64 + 32 + 38 + 9 + 18 + 6 = 167. The `os` 38 and 9 are the 47 on P4.S13. The 24 `iw` names leave P4.S27, which keeps the other 61. No `iw*` name is overloaded. The overloads in this file are `llSHA256String` (2), `osApproxEquals` (6), `osSlerp` (2), `osStringIndexOf` (2), `osStringLastIndexOf` (2) and `osStringSubString` (2).

`llWorldPosToHUD` (`LSLSystemAPI.cs` 17094–17098) returns `<0.5, 0.5, 0>` and is not one of the 64. It stays with object and prim control. `iwFormatTime` (`LSLSystemAPI.cs` 1951–1958) calls `ToLocalTime` when `isUTC` is 0, so it reads the host zone. It stays in the 61. `osCheckODE` reads the physics engine name and stays with the physics `os*` family.

## 1. How a row is proven

P4.19 builds `tools/script-oracle` on `Tests/InWorldz.Phlox.Tests/SchedulerHarness.cs`: a real Phlox engine, a script rezzed the way `OnRezScript` does, and `DoWork` pumped by hand. `Tests/InWorldz.Phlox.Tests/ExprRunner.cs` already captures `llOwnerSay`. These rows do not add P4.19p or any later letter. The cases below are what that harness runs for the row. Expected text is the recording. It is not a number copied out of the cited lines. Ordinary CI does not start Tranquillity.

The compare, for every row in this file:

- An integer, a string, a key, a list element's type, list order, JSON text, error text, and a sleep duration match the recording exactly.
- A float, vector, or rotation said with `llOwnerSay((string)...)` is the six-digit text from `Encoding.FloatToStringWith6FractionalDigits` and the six-digit vector and rotation printers (`Encoding.cs` 128–160). The pass is that chat text. A libm difference that does not change the sixth digit still matches. A difference that changes the sixth digit fails. The tolerance is not a residual measured from Ferroan. A case about an integer boundary says the integer result, not the float text.
- A formula that is arithmetic on values already rounded to float32 matches the recording when the port uses the same formula.
- `llFrand`, `llGenerateKey`, `iwIntRand`, `iwIntRandRange`, `iwFrandRange` and `llListRandomize` compare the range, the type, and the empty-input or equal-bound result. They do not compare bits.
- A denied `os*` call compares the thrown gate error, which is the recording P4.16 owns. The master-switch text is `OSSL Permission Error: All unsafe OSSL funtions disabled` (`OsslGate.cs` 118), with the `(OWNER)` prefix when `m_errorToOwner` is set (130). A named denial is `{function} permission denied.  Allowed threat level is {max} but function threat level is {level}` (210). The script is killed (`PhloxExecutionScheduler.cs` 1442–1449, `TerminateWithError` 3452–3476). The shout is `Script error: ` plus `Script {asset} encountered a problem and was stopped: {message}` (`LSLSystemAPI.cs` 658–662, scheduler 3475).

An `ll` row depends on P4.1c and P4.19. An `os` row also depends on P4.16. An `iw` row depends on P4.1c and P4.19, the same table dependency P4.S27 uses. The harness runs with the shipped `[OSSL]` (`config-include/osslDefaultEnable.ini`, `OSFunctionThreatLevel = VeryLow` at line 33). A denied call is a second case with the gate off, or with that function's `Allow_` line set so the threat test refuses it. Sleeps in this file are recorded on the harness. They do not wait on P4.23. When P4.23 is written, its table gains the 300 ms of `llXorBase64Strings`, the 100 ms of `iwFormatString`, the length sleeps of `iwStringCodec`, and the 15 ms `ChatSleep` on a `ScriptShoutError`.

`Vector3` and `Quaternion` in the Phlox file are OpenMetaverse types (`using OpenMetaverse`, `LSLSystemAPI.cs` 56). They are not `System.Numerics`.

### What the script sees on a throw

`OsslCheck` throws `VMException` (`OsslGate.cs` 114–127). A raw `.NET` exception from a body is also an `Exception`. The tick catch (scheduler 1442) calls `TerminateWithError`. The script stops, the Running flag goes off, and the shout carries `e.Message`. There is no second return value. Rows that name a throw mean that shout and that stop. The message text is the recording, because it includes the runtime's wording and the asset id.

`ShoutError` (`LSLSystemAPI.cs` 658–662) speaks `Script error: ` plus the text on `DEBUG_CHANNEL` and does not sleep. `ScriptShoutError` (269–272) does that and then `ChatSleep`, 15 ms when `ChatThrottle` is on (`PhloxEngine.cs` 304, default true). `LSLError` prefixes `LSL Runtime Error: ` and goes through `ScriptShoutError` (277).

### Argument conversion used by more than one row

`LslConvert.FloatToInteger` (`Source/InWorldz.Phlox/Util/LslConvert.cs` 22–27): NaN, or `f >= 2147483648.0`, or `f <= -2147483649.0`, returns `int.MinValue`. Anything else is a truncating cast. The comment records that a plain C# cast on .NET 9 x64 saturates.

`Encoding.CastToInt` (`Encoding.cs` 45–83) strips one leading `+`, then takes a leading run of spaces and either a `0x` hex integer or a signed decimal integer. Overflow becomes -1. No match is 0. `CastToFloat` (85–118) uses the same leading-`+` strip and the `en-US` culture. Overflow is `+infinity`.

`Encoding.Fixed` (128–138) widens a float to double, formats `F6` or `F5` with the invariant culture, and strips a leading minus that has no non-zero digit. `NaN`, `Infinity` and `-Infinity` keep the invariant names. `-0` prints as `0`. List string form uses six digits for a float, a vector and a rotation (`LSLList.cs` 437–463). A bare `(string)` of a vector or rotation uses five (`Encoding.cs` 140–148). The oracle for a function that returns a vector says the value the way the script says it. A list element said through `llList2String` is the six-digit form.

List indexes on the `llList2*` getters, and the range on `llGetSubString`, `llDeleteSubString`, `llList2List`, `llDeleteSubList` and `llListReplaceList`, use `NormaliseLslRange` and `InLslRange` (`LSLSystemAPI.cs` 15579–15586). A negative index has the length added once. When start is past end, the range is `[0, end]` plus `[start, -1]`. An index outside the value selects nothing.

### YEngine, once

YEngine stores a float as a double and does not cast `Math.Sin` and its siblings down to float32 (`LSL_Api.cs` 963–990). Phlox does (`LSLSystemAPI.cs` 1365–1383). The handwritten `TokenDeclInline_Math` and `TokenDeclInline_LLAbs` are commented out (`MMRScriptInlines.cs` 530–547). A compiled call goes through `TokenDeclInline_BEApi` (626). Names in `noCheckRuns` (63–109) get the call alone. Every other pure `ll` call gets a `CheckRun` after it (58–59). The names from these families that are in that set are `llBase64ToString`, `llCSV2List`, `llDeleteSubList`, `llDeleteSubString`, `llDumpList2String`, `llEscapeURL`, `llEuler2Rot`, `llGetListEntryType`, `llGetListLength`, `llGetSubString`, `llInsertString`, `llList2CSV`, `llList2Float`, `llList2Integer`, `llList2Key`, `llList2Rot`, `llList2String`, `llList2Vector`, `llListFindList`, `llListInsertList`, `llListRandomize`, `llListReplaceList`, `llListSort`, `llListStatistics`, `llMD5String`, `llParseString2List`, `llParseStringKeepNulls`, `llRot2Euler`, `llStringLength`, `llStringToBase64`, `llStringTrim`, `llSubStringIndex`, `llUnescapeURL`, `osIsNotValidNumber`, `osSlerp`, `osApproxEquals`, `osVecDistSquare`, `osVecMagSquare`, `osRound`, `osMax`, `osMin` and `osIsUUID`. YEngine has no `iw*`. It does not declare `llSignRSA`, `llVerifyRSA`, `llSRGB2Linear` or `llSortListStrided`. It does declare `llsRGB2Linear` and `llListSortStrided` (`ILSL_Api.cs` 511, 271). Those gaps are recorded. The Phlox harness is the proof.

### The gate on a pure `os*` call

`OsslCheck()` with no arguments is the master switch only (`LSLSystemAPI.cs` 11747, `OsslGate.Check` 114–118). `OsslCheck(level, name)` looks up `Allow_` and `Creators_` under `name` (`OsslGate.cs` 132–210). An empty pair falls through to the threat test. `ThreatLevel.None` passes while the shipped maximum is `VeryLow`, because `None <= VeryLow` (208–210). A line `Allow_osMax` does not apply to a check whose name is `osGetRezzingObject`.

Gated in this file, master switch only: `osPerlinNoise2D`, `osListSortInPlace`, `osListSortInPlaceStrided`, `osStringStartsWith`, `osStringEndsWith`, both `osStringIndexOf`, both `osStringLastIndexOf`, both `osStringSubString`. Named: `osMax` (`None`, key `osGetRezzingObject`), `osFormatString` (`VeryLow`), `osMatchString` (`VeryLow`), `osRegexIsMatch` (`Low`), `osReplaceString` (`VeryLow`). The shipped file allows the four named ones that have an active line: `Allow_osFormatString` (116), `Allow_osMatchString` (170), `Allow_osRegexIsMatch` (123), `Allow_osReplaceString` (102). `Allow_osGetRezzingObject` is commented out (85), so `osMax` is allowed by the threat test. The other pure `os*` bodies do not call `OsslCheck`. With the master switch off they still return. The denied case covers the gated names. The ungated names are run once with the switch off and still return.

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

`llRot2Euler` (1389–1401) rotates `<0, 0, 1>`, returns the zero vector when that length is 0, forces x to 0 when the sine is within `1e-6` of ±1, then takes `atan2` of the twice-rotated `<1, 0, 0>`. `llEuler2Rot` (1404–1411) is the half-angle product, then `Quaternion.Normalize`. `llAxes2Rot` (1414–1442) is the trace construction with the three branch cuts written there. `llRot2Fwd`, `llRot2Left` and `llRot2Up` (1444–1459) rotate `<1, 0, 0>`, `<0, 1, 0>` and `<0, 0, 1>` and normalise. `llRotBetween` (1462–1471) returns `Quaternion.Identity` when either length is 0 or the normalised cross product has a NaN component. Otherwise it is the axis-angle of the clamped dot product. `llAxisAngle2Rot` (1474–1477) normalises the axis and calls `Quaternion.CreateFromAxisAngle`. Nothing sleeps. Nothing shouts.

Oracle: identity, a 90-degree turn about Z, a zero quaternion into `llRot2Euler`, Euler of the zero vector, axes that are not orthonormal, `llRotBetween` of two zero vectors, of a NaN axis, and of two parallel vectors, and `llAxisAngle2Rot` of a zero axis. Say the vector or the rotation. `llEuler2Rot` and `llRot2Euler` are in YEngine's `noCheckRuns`. The bodies are still the Phlox ones above for the pass.

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

`llFrand` is `(float)(ThreadRandom.NextDouble() * mag)` (1373). `llGenerateKey` is `UUID.Random().ToString()` (16024). `iwIntRand` (18485–18488) returns `ThreadRandom.Next` on `0 .. max` inclusive, or the negation of that on `max .. 0` when `max` is negative. The bound is clamped so the `Next` argument stays inside `int`. `iwIntRandRange` and `iwFrandRange` (18490–18491) use `new Random()`. Equal bounds return that bound. Reversed bounds are swapped. The integer form is `Next(min, max + 1)`. The float form is `NextDouble() * (max - min) + min`. Not bit-exact. Nothing sleeps.

Oracle: `llFrand(0)`, `llFrand` of a positive, and `llFrand` of a negative, checked for type and for lying in the range the call describes. `llGenerateKey` checked as a string `UUID.TryParse` accepts. `iwIntRand(0)`, `iwIntRand` of a positive, and of a negative. `iwIntRandRange` and `iwFrandRange` with equal bounds, with reversed bounds, and with a normal pair. Several calls are allowed to differ.

### P4.F1.7 String length, case, and ranges

Depends on P4.1c, P4.19.

Functions: `llStringLength`, `llToUpper`, `llToLower`, `llGetSubString`, `llDeleteSubString`, `llInsertString`, `llStringTrim`, `llSubStringIndex`, `llReplaceSubString`, `llChar`, `llOrd`, `iwSubStringIndex`, `iwReverseString`, `iwChar2Int`, `iwInt2Char`.

Source: `LSLSystemAPI.cs` 15568–15634, 15681–15805. YEngine `llToUpper` and `llToLower` are 3373–3380. YEngine `llSubStringIndex` is 6217–6223.

`llStringLength`, `llToUpper` and `llToLower` null-coalesce (15568–15570). `ToUpper` and `ToLower` use the current culture. There is no `Invariant`. YEngine's `llToUpper` does not null-check (3373–3375). Recorded. The culture is question 5.

`llGetSubString` and `llDeleteSubString` (15588–15616) use the range rule in section 1. A null source is `""`. `llInsertString` (15618–15623) clamps the position into `[0, length]` and does not treat a negative as an end-relative index. `llStringTrim` (15807–15815): type 1 trims the head, 2 the tail, 3 both, and any other type returns the string. Whitespace is `.NET` `Trim`. A null is `""`. `llSubStringIndex` (15626–15627) is `IndexOf` ordinal, or -1 when the source is null. A null pattern throws `ArgumentNullException` and the script dies. YEngine (6217–6223): an empty source returns -1, an empty pattern returns 0, and the compare is the default `IndexOf`. Recorded.

`llReplaceSubString` (15681–15731): a count of 0 replaces every match. A negative count replaces from the end. An empty pattern or an empty source returns the source. The search is ordinal. `llChar` (16027–16034): a negative returns `""`, 0 returns a NUL, and any other code point is `ConvertFromUtf32`, or `""` when that throws. `llOrd` (16036–16045) adds the length to a negative index once, returns 0 out of range, and reads a surrogate pair with `ConvertToUtf32`.

`iwSubStringIndex` (15629–15633) returns -1 when either argument is null. The offset is clamped up to 0. A non-zero `isCaseSensitive` is ordinal. Zero is ordinal-ignore-case. `iwReverseString` (15788–15792) reverses UTF-16 code units, so a surrogate pair swaps halves. The comment says that is Halcyon and is kept. A null is `""`. Length 0 or 1 returns the same string. `iwChar2Int` (15794–15799) reads one UTF-16 unit, not a surrogate pair. A negative index adds the length once. Out of range or empty is 0. `iwInt2Char` (15801–15804) returns `""` outside `0 .. 0xFFFF`.

Oracle: null-coalesce where the host can pass a null, Turkish `i` if question 5 keeps the current culture, an ordinary substring, a negative index, an inverted range, an insert at a negative position and past the end, trim types 0, 1, 2 and 3, `llSubStringIndex` of an empty pattern and of a missing pattern, `llReplaceSubString` with counts 0, 1 and -1, `llChar` of -1, 0 and a surrogate code point, `llOrd` on a surrogate pair and on a negative index, `iwReverseString` of a character outside the BMP, `iwChar2Int` of that same character, and `iwInt2Char` of -1 and of `0x10000`. A null pattern on `llSubStringIndex` compares the killed-script shout.

### P4.F1.8 Base64 and URL escape

Depends on P4.1c, P4.19.

Functions: `llStringToBase64`, `llBase64ToString`, `llIntegerToBase64`, `llBase64ToInteger`, `llXorBase64Strings`, `llXorBase64StringsCorrect`, `llXorBase64`, `llEscapeURL`, `llUnescapeURL`.

Source: `LSLSystemAPI.cs` 15818–15920, 16025–16026, 21766–21788. YEngine `llBase64ToString` is 11806–11817. YEngine `llXorBase64Strings` sleeps 300 as well (14887–14891).

`llStringToBase64` is UTF-8 then `Convert.ToBase64String`. A null is `""`. `llBase64ToString` replaces U+FFFD with `?` and returns `""` on a bad payload, with no shout (15819). YEngine does not replace U+FFFD, and a bad payload shouts `Error` and returns `""` (11806–11816). Recorded.

`llIntegerToBase64` and `llBase64ToInteger` (15820–15851) use the tables `i2ctable` and `c2itable`, not `Convert.FromBase64String`. The integer encoder always writes eight characters, and the last two are `=`. The decoder returns 0 when the string is empty or longer than 8, and it stops early on a bad digit. `llXorBase64Strings` (15862–15905) sleeps 300 ms first. The comment says the wiki documents 0.3 s and that the padding quirks are the behaviour. This plan does not file a ledger row for them. An empty first string returns `""` after the sleep. An empty second string returns the first. A bad payload returns `""`. `llXorBase64StringsCorrect` (15907–15920) decodes, XORs UTF-16 units, re-encodes, and does not sleep. `llXorBase64` (21766–21788) XORs decoded bytes and re-encodes. A bad payload returns `""`. An empty second string returns the first.

`llEscapeURL` and `llUnescapeURL` (16025–16026) are `Uri.EscapeDataString` and `Uri.UnescapeDataString` with no catch. A null is `""`. A bad `%` sequence throws, and the script dies with the shout in section 1. A very long string can throw from `EscapeDataString` the same way.

Oracle: a short ASCII string both ways, a non-ASCII character, a bad base64 payload, an integer 0 and a negative integer through the eight-character form, `llXorBase64Strings` of two short strings and of an empty string (the wake is 300 ms), the same inputs on `llXorBase64StringsCorrect` and `llXorBase64` (no sleep), `llEscapeURL` of a space and of a non-ASCII character, and `llUnescapeURL` of a bad `%` sequence (the shout and the stop). The 300 ms is recorded here. It does not wait on P4.23.

### P4.F1.9 Hashes

Depends on P4.1c, P4.19. `osSHA256` also depends on P4.16.

Functions: `llMD5String`, `llSHA1String`, `llSHA256String` (both), `osSHA256`, `iwSHA256String`, `llComputeHash`, `llHMAC`, `llHash`.

Source: `LSLSystemAPI.cs` 15923–16023, 15937–15941, 21408–21419, 21560–21564. YEngine `llComputeHash` is `LSL_Api.cs` 21663–21719. YEngine `llHMAC` is 22314–22362, and the interface order is `ILSL_Api.cs` 549. Phlox's order is `ISystemAPI.cs` 970 and `ILSLSystemAPI.cs` 801.

`llMD5String` is MD5 of the UTF-8 bytes of `src + ":" + nonce`, lowercase hex (15923–15928). `llSHA1String` is SHA-1 of the UTF-8 source (15931–15935). `llSHA256String(src, nonce)` hashes `src + ":" + nonce` through `llComputeHash` with `sha256` (15985–15988). `llSHA256String(src)` is SHA-256 of the UTF-8 source with nothing appended (21560–21564). `osSHA256` (12436–12439) and `iwSHA256String` (15937–15941) are that same hex. `osSHA256` is ungated. `llHash` (21408–21419) is DJB2 from 5381 over the UTF-16 units, and 0 for null or empty.

`llComputeHash` (15943–15984) lowercases the algorithm and implements `md5`, `sha1`, `sha256`, `sha384`, `sha512`, and `md5_sha1`. `md5_sha1` is SHA-1 of the lowercase hex of the MD5. The comment at 15945 names `sha224`. There is no `sha224` case. An unknown algorithm shouts `llComputeHash: unsupported algorithm '...'` and returns `""`. YEngine (21663–21719) is case-sensitive, implements `sha224`, returns `""` with no shout on an unknown name, and builds `md5_sha1` by concatenating the raw MD5 bytes and the raw SHA-1 bytes of the original input. Recorded. Proposed TB170 and TB171.

`llHMAC` (15990–16022) takes `(msg, privateKey, algorithm)`, returns base64, and implements `md5`, `sha1`, `sha256`, `sha384`, `sha512`. An unknown algorithm shouts and returns `""`. There is no `sha224`. YEngine takes `(private_key, message, algo)` (`ILSL_Api.cs` 549), returns lowercase hex (22359), implements `sha224` (22290–22311), and returns `""` when the key or the message has length under 1 or the algorithm is unknown (22316–22317, 22342). Recorded. The default under [D124](../DECISIONS.md) is Phlox's order and Phlox's base64. Question 3 asks whether the SLua `ll` table should take the key first.

Oracle: `llMD5String` and both `llSHA256String` forms on `""` and on a short string, so the nonce colon is visible. `llSHA1String`, `osSHA256` and `iwSHA256String` on the same bytes. `llHash("")` and `llHash` of one character. `llComputeHash` for each implemented name, for `SHA256` (Phlox lowercases it), for `sha224`, and for a misspelled name (the shout, the script still running). `llHMAC` with Phlox's argument order, one known algorithm, `sha224`, an unknown name, and an empty key. Hex and base64 are exact. `osSHA256` still returns with the master switch off.

### P4.F1.10 RSA and colour

Depends on P4.1c, P4.19. `osTemperature2sRGB` and the `iw*` names also depend on P4.16 for the `os*` name only. The `iw*` names depend on P4.1c and P4.19.

Functions: `llSignRSA`, `llVerifyRSA`, `llLinear2sRGB`, `llSRGB2Linear`, `llsRGB2Linear`, `osTemperature2sRGB`, `iwColorConvert`, `iwNameToColor`.

Source: `LSLSystemAPI.cs` 20749–20799, 17068–17092, 21550, 11960–11985, 18245–18262, helpers 18266–18348. YEngine declares `llLinear2sRGB` and `llsRGB2Linear` and does not declare `llSRGB2Linear`, `llSignRSA` or `llVerifyRSA`.

`llSignRSA` (20749–20772) uppercases the algorithm and accepts `SHA256`, `SHA384`, `SHA512` and `SHA1`. It imports a PEM private key, signs UTF-8 with PKCS1, and returns base64. Any exception is logged and the result is `""`. A bad algorithm returns `""` with no throw. `llVerifyRSA` (20775–20799) is the same set plus `SHA224` as a `HashAlgorithmName` string. Success is 1, failure is 0, and any exception is 0. YEngine has neither method. Recorded.

`llLinear2sRGB` and `llSRGB2Linear` (17068–17092) use the piecewise sRGB curve and clamp each component to `[0, 1]`. `llsRGB2Linear` forwards to `llSRGB2Linear` (21550). `osTemperature2sRGB` (11960–11985) is the blackbody polynomial, clamped, with the 1000 and 40000 endpoints written there. Ungated. `iwColorConvert` (18245–18255): 0 is RGB, 1 is HSL, 2 is HSV. The same space returns the input. Any other source space is treated as already RGB. The helpers are `H2RGB`, `HSL_TO_RGB`, `HSV_TO_RGB`, `RGB_TO_HSL` and `RGB_TO_HSV` (18266–18348). `iwNameToColor` (18257–18261) is `System.Drawing.Color.FromName` after spaces are removed, then R, G and B divided by 255.

Oracle: `llSignRSA` and `llVerifyRSA` with a fixture PEM the harness generates, a bad algorithm, a bad PEM, and a bad signature. The bad paths return `""` and 0 and the script keeps running. One linear colour and one sRGB colour through both spellings, including a component under 0 and over 1. `osTemperature2sRGB` at 1000, at 40000, and at one value in each branch (under 6600, and over it). `iwColorConvert` for each pair of spaces, including a pair that is the same. `iwNameToColor` of `Red` and of an unknown name. Float text is the six-digit chat. `osTemperature2sRGB` still returns with the master switch off.

### P4.F1.11 Numeric helpers

Depends on P4.1c, P4.16, P4.19. `iwClampInt`, `iwClampFloat` and `iwVerifyType` depend on P4.1c and P4.19.

Functions: `osApproxEquals` (6), `osMax`, `osMin`, `osRound`, `osIsNotValidNumber`, `osIsUUID`, `osPerlinNoise2D`, `osUnixTimeToTimestamp`, `iwClampInt`, `iwClampFloat`, `iwVerifyType`.

Source: `LSLSystemAPI.cs` 12188–12191, 12284–12342, 12406–12433, 12568–12570, 18350–18376, 18472–18483. `TerrainUtil.cs` 110–121. `SlConst.cs` 409–415. YEngine `osPerlinNoise2D` returns the double (`OSSL_Api.cs` 6400–6402).

`osApproxEquals` (12284–12314): the float form uses margin `1.0e-6`. A passed margin is replaced by its absolute value. The vector form compares each component, not the length. The quaternion form compares all four. A component outside the margin makes the result 0. Ungated. In `noCheckRuns`.

`osMax` (12406–12409) calls `OsslCheck(ThreatLevel.None, "osGetRezzingObject")`. The comment says upstream copy-pastes that key and Phlox honours it. With the shipped file the call is allowed, because the threat is `None` and `Allow_osGetRezzingObject` is commented out (ini line 85). `Allow_osMax` is not the key that is read. `osMin` (12412–12415) is ungated. Proposed TB172.

`osRound` (12428–12432): `ndigits <= 0` rounds to an integer with `AwayFromZero`. `ndigits > 15` is clamped to 15. Otherwise it is `Math.Round` of the widened value. `osIsNotValidNumber` (12331–12336) returns 1 for NaN, 2 for negative infinity, 3 for positive infinity, and 0 otherwise. `osIsUUID` (12340–12342) is `UUID.TryParse`. `osUnixTimeToTimestamp` (12568–12570) is `Util.ToDateTime(time).ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ")`. `ToDateTime` adds the seconds to `UnixEpoch`, which is UTC (`Util.cs` 185–187, 1170–1172). The `Z` is a literal.

`osPerlinNoise2D` (12188–12191) is `(float)TerrainUtil.PerlinNoise2D`. That function (110–121) is a loop over octaves of `InterpolatedNoise`. It does not read the heightmap. `octaves <= 0` skips the loop and returns 0. It is pure. The master switch still gates the call. YEngine returns the double (6400–6402). Recorded.

`iwClampInt` and `iwClampFloat` (18472–18483): equal bounds return that bound. Reversed bounds still clamp into the interval. `iwVerifyType` (18350–18376): type 0 tries 1, 2, 4, 5, 6 and returns the first match, or 3. `TYPE_INTEGER` is 1, `TYPE_FLOAT` is 2, `TYPE_STRING` is 3, `TYPE_KEY` is 4, `TYPE_VECTOR` is 5, `TYPE_ROTATION` is 6, `TYPE_INVALID` is 0 (`SlConst.cs` 409–415). Integer, float, key, vector and rotation use `TryParse`. A vector needs exactly two commas. A rotation needs exactly three. `TYPE_STRING` returns 1 for any string. Any other type returns -1. `TryParse` uses the current culture. Question 5.

Oracle: `osApproxEquals` on each signature, inside the margin, outside it, and with a negative margin. `osMax` and `osMin` of NaN and of mixed infinities. `osMax` once under the shipped file (allowed) and once with `Allow_osGetRezzingObject = false` (the named denial, and the key in the message is `osGetRezzingObject`). `osRound` with `ndigits` -1, 0, 2 and 20. `osIsNotValidNumber` of a finite value, NaN, and both infinities. `osIsUUID` of a real UUID, of `""`, and of a 36-character string that is not a UUID. `osPerlinNoise2D` at a fixed point with 0 octaves and with 2, and once with the master switch off (the denial). `osUnixTimeToTimestamp` of 0 and of a negative. The string is exact. `iwClamp*` with equal bounds, reversed bounds, and a value inside. `iwVerifyType` for each type constant, for auto-detect, and for an unknown type.

### P4.F1.12 Plain string helpers

Depends on P4.1c, P4.16, P4.19. `iwReplaceString` and `iwValidateURL` depend on P4.1c and P4.19.

Functions: `osStringStartsWith`, `osStringEndsWith`, `osStringIndexOf` (2), `osStringLastIndexOf` (2), `osStringRemove`, `osStringReplace`, `osStringSubString` (2), `iwReplaceString`, `iwValidateURL`.

Source: `LSLSystemAPI.cs` 12478–12565, 15674–15679, 15423–15427.

`osStringStartsWith` and `osStringEndsWith` (12478–12491) use `CurrentCulture` when `ignorecase` is not 0. An empty source or value returns 0. Master switch. `osStringIndexOf` (12494–12509) uses ordinal or ordinal-ignore-case. The three-argument form returns -1 on an empty source or value. The five-argument form clamps a negative offset up to 0, returns -1 when the offset is past the end, and turns a count of 0 or less into the remainder of the string. Master switch.

`osStringLastIndexOf` of three arguments (12512–12517) is `LastIndexOf` over the whole string. The five-argument form (12521–12527) clamps the offset and the count the same way as `IndexOf`, then calls `String.LastIndexOf(value, offset, count, comparison)`. That search runs backward. .NET throws `ArgumentOutOfRangeException` when `offset - count + 1 < 0`. The call with offset 0 and count 0 sets the count to the whole length and throws whenever the string is longer than one character. The script dies. The three-argument form does not throw. Proposed TB167. This was read from the signature and the call. It was not executed.

`osStringRemove` (12531–12537) is ungated. An empty source, or an offset past the end, returns `""`. A negative offset becomes 0. A count of 0 or less becomes the remainder. If that count is at least the length, the result is `""`, so offset 0 and count 0 return `""`. `osStringReplace` (12540–12545) is ungated. An empty source or an empty old value returns `""`. An empty new value is passed as null, which is how `String.Replace` deletes. `osStringSubString` (12548–12564) is the master switch. An offset of 0 or less returns the whole string. It does not count from the end. A length of 0 or less returns `""`.

`iwReplaceString` (15674–15679) returns the original when the string or the pattern is empty, or when either the pattern or the replacement is longer than 1024. An empty replacement calls `Replace(pattern, null)`. `iwValidateURL` (15423–15427) is `Uri.TryCreate` with `UriKind.Absolute`, and the scheme must be `http` or `https`. The result is `Convert.ToInt32` of that boolean.

Oracle: starts and ends, case-sensitive and not, including an empty value. Both index forms, including offset 0 and count 0 on `IndexOf` (a number) and on `LastIndexOf` (the shout and the stop). `osStringRemove` of offset 0 and count 0, and of a middle offset. `osStringReplace` of an empty old value. `osStringSubString` of a negative offset. One denied call of a master-switch name with the switch off, and one allowed call of `osStringRemove` with the switch off. `iwReplaceString` over the 1024 limit. `iwValidateURL` of `https`, of `ftp`, and of a relative string.

### P4.F1.13 Regex and format

Depends on P4.1c, P4.16, P4.19. `iwMatchString` and `iwFormatString` depend on P4.1c and P4.19. `iwFormatString` carries `FormatStringThrottle`, which P4.S1 named.

Functions: `osFormatString`, `osMatchString`, `osRegexIsMatch`, `osReplaceString`, `iwMatchString`, `iwFormatString`.

Source: `LSLSystemAPI.cs` 12324–12327, 12380–12402, 12418–12424, 13108–13116, 15636–15672, 15733–15766. `ScriptRegex.cs` 21–26. `PhloxEngine.cs` 310.

`osFormatString` (12324–12327) is `VeryLow` under the key `osFormatString`. It calls `string.Format(str, strings.Members.ToArray())` with no catch. The culture is the current culture. A float element uses `.NET` `ToString`, not six LSL digits. A bad format, or a null format string, throws, and the script dies. The shipped `Allow_osFormatString = true` (ini 116) allows it.

`osMatchString` (12380–12402) is `VeryLow`, key `osMatchString`, allowed by ini 170. A negative start counts from the end. Out of range returns an empty list. Each successful group adds the value and the index. A timeout shouts `regex timed out` through `ShoutError` (no 15 ms) and returns an empty list. `osRegexIsMatch` (12418–12424) is `Low`, key `osRegexIsMatch`, allowed by ini 123 even though `Low` is above the shipped `VeryLow`. A timeout shouts `regex timed out` and returns 0. Any other exception shouts `Possible invalid regular expression detected.` and returns 0. `osReplaceString` (13108–13116) is `VeryLow`, key `osReplaceString`, allowed by ini 102. A negative start counts from the end. Out of range returns the source. A timeout shouts and returns the source. There is no catch for a pattern that fails to compile, so that throw kills the script.

`ScriptRegex.Create` (21–26) sets a 250 ms match timeout. The message is `regex timed out` (23).

`iwMatchString` (15636–15672): types -2 include, -1 equal, 0 head, 1 tail, 2 regex anchored with `^` and `$`, 3 count of the escaped pattern, 4 regex count. An empty string returns 1 only when both are empty and the type is at most 1. Otherwise 0. A timeout shouts and returns 0. Any other type returns 0. `iwFormatString` (15733–15766): sleeps 100 ms when `FormatStringThrottle` is on and `Clock.Now` moves during a replacement (`PhloxEngine.cs` 310, default true). A value longer than 1024 is cut to 1023 characters. The loop stops at the first missing `{i}`. `LSLError` and `""` when the result would pass 32768, and that path takes the 15 ms `ChatSleep`. A null or empty format returns as it is.

Oracle: `osFormatString` of a plain `{0}`, of a float element (record the culture and the text), and of a bad format (the shout and the stop). `osMatchString` of one match and of a pattern that times out. `osRegexIsMatch` of a match, of a miss, of a bad pattern (the invalid-expression shout, script still running), and once with `Allow_osRegexIsMatch = false` (the named denial). `osReplaceString` of a normal replace, of a negative start, and of a pattern that does not compile (the stop). `iwMatchString` of each type from -2 to 4, plus a timeout. `iwFormatString` with the throttle on and the clock advanced (100 ms), with the throttle off (no sleep), a value longer than 1024, a missing `{i}`, and a result over 32768 (the `LSL Runtime Error` shout and the 15 ms). These sleeps do not wait on P4.23.

### P4.F1.14 `iwStringCodec`

Depends on P4.1c, P4.19. About a day by itself. The body is one file and the oracle covers the operations, not every alphabet.

Functions: `iwStringCodec`.

Source: `LSLSystemAPI.cs` 15768–15781, which delegates to `Source/Phlox.ScriptEngine/Codecs/HalcyonStringCodec.cs`. `Run` is line 93. The host's `LSLError` shouts `LSL Runtime Error: ` plus the text and takes the 15 ms (`LSLSystemAPI.cs` 15780, 277). `ScriptSleep` is the host sleep (15781).

Operations: `OP_DECODE` 0, `OP_ENCODE` 1, `OP_VALIDATE` 2 (95–97). Length sleeps, before the work (99–101): length at least 16000 sleeps `(len / 100) * 20`, at least 8000 sleeps `(len / 100) * 10`, at least 1000 sleeps `(len / 100) * 5`. The codec name is lowercased with the invariant culture (400–402). The comment says Halcyon ran `en-US`, where that matches.

Codecs in the switch: `ascii` (126), `gzip` (156, default output codec `base4096`), `ascii-zip` (179), the hash names `md5`, `sha1`, `sha-1`, `sha128`, `sha-128`, `sha2`, `sha-2`, `sha256`, `sha-256`, `sha384`, `sha-384`, `sha512`, `sha-512` (184–196), `aes-key` (225), `aes` (286), an empty codec (371), and the default arm (374) which encodes, decodes or validates through `CodecUtil`. The output alphabets `HasCodec` accepts are `base16`, `uuid`, `base64`, `base64-safe`, `base4096` and `base4k` (411–416). A bad input codec shouts and returns `""` (116–117). A bad gzip output codec shouts and returns `""` (171–174). An ASCII decode error shouts and then falls out of the catch with no return of its own (138–141), so the method returns `""` at 397. An empty codec shouts `Error: No codec specified for iwStringCodec!` (372). A blank AES password shouts and returns `""` (228). An unknown codec in the default arm shouts and returns `""` (390–393). Validate returns `VALID`, `INVALID`, `PASS`, `FAIL` or `INVALID CODEC` at the sites above (143–153, 384–388).

The helpers under `CodecUtil` (405 onward) were not read line by line. The row ports that file. The oracle does not need a case for every hash alias.

Oracle: one encode, one decode and one validate of `base64`. One bad codec name (the shout, `""`, script still running, 15 ms). One string of length 1000 and one of length 8000, with the sleep durations `(1000/100)*5` and `(8000/100)*10`. One gzip encode with the default output codec and one with a bad output codec. One empty codec. The sleeps do not wait on P4.23.

## 3. Lists and JSON

The 32 `ll` methods are `llListSort`, `llGetListLength`, `llList2Integer`, `llList2Float`, `llList2String`, `llList2Key`, `llList2Vector`, `llList2Rot`, `llList2List`, `llDeleteSubList`, `llGetListEntryType`, `llList2CSV`, `llCSV2List`, `llListRandomize`, `llList2ListStrided`, `llListInsertList`, `llListFindList`, `llParseString2List`, `llDumpList2String`, `llParseStringKeepNulls`, `llListReplaceList`, `llListStatistics`, `llJsonGetValue`, `llJsonValueType`, `llJsonSetValue`, `llList2Json`, `llJson2List`, `llListFindStrided`, `llList2ListSlice`, `llSortListStrided`, `llListFindListNext`, `llListSortStrided`.

The 9 `os` methods are `osOldList2ListStrided`, `osListFindListNext`, `osListSortInPlace`, `osListSortInPlaceStrided`, `osListAsFloat`, `osListAsInteger`, `osListAsString`, `osListAsVector`, `osListAsRotation`.

The 6 `iw` methods are `iwParseString2List`, `iwMatchList`, `iwListIncludesElements`, `iwReverseList`, `iwListRemoveElements`, `iwListRemoveDuplicates`.

### P4.F2.1 List accessors

Depends on P4.1c, P4.19. The `osListAs*` names also depend on P4.16.

Functions: `llGetListLength`, `llList2Integer`, `llList2Float`, `llList2String`, `llList2Key`, `llList2Vector`, `llList2Rot`, `llGetListEntryType`, `osListAsFloat`, `osListAsInteger`, `osListAsString`, `osListAsVector`, `osListAsRotation`.

Source: `LSLSystemAPI.cs` 16050–16134 and 12346–12377. `LSLList.cs` 391–463. `Encoding.cs` 45–118. `SlConst.cs` 409–415.

`llGetListLength` is 0 for a null list (16050). The six `llList2*` getters (16056–16095) add the length to a negative index once. `GetLSLIntegerItem` (414–434): an int is itself, a float goes through `FloatToInteger`, a string goes through `CastToInt` (so `"0x1F"` is 31, as the comment at 16051–16055 says), and anything else is 0. Out of range is 0. `GetLSLFloatItem` (391–411): an int widens, a string uses `CastToFloat` (`en-US`, overflow `+infinity`), and anything else is 0. `GetLSLStringItem` (437–463): a string is itself, a float, vector or rotation is six digits, and anything else is `ToString()`. `llList2Key` (16074–16082) returns a string element as it is, including a string that is not a UUID, and `""` for any other type or an out-of-range index. `llList2Vector` and `llList2Rot` use `GetVector3Item` and `GetQuaternionItem`. A bad string is the zero vector or `Quaternion.Identity`.

`llGetListEntryType` (16119–16134): out of range or empty is 0. An int is `TYPE_INTEGER` (1). A float or a double is `TYPE_FLOAT` (2). A string that `UUID.TryParse` accepts is `TYPE_KEY` (4). Any other string is `TYPE_STRING` (3). A `UUID` is `TYPE_KEY`. A `Vector3` is `TYPE_VECTOR` (5). A `Quaternion` is `TYPE_ROTATION` (6). A nested `LSLList` is `TYPE_INVALID` (0). Anything else returns 3. The comment says the default is string.

`osListAs*` (12346–12377) do not coerce and do not wrap a negative index. A negative index, an out-of-range index, or a wrong type returns the typed zero: `0`, `0.0`, `""`, the zero vector, or `Quaternion.Identity`. Ungated.

Oracle: one list that holds an int, a float, a float string, `"0x1F"`, a key string, a non-key string, a vector, a rotation, and a nested list. Each getter at 0, at -1, and past the end. `llList2String` of the float is the six-digit text. `llList2Integer` of NaN and of `1e10` is -2147483648. `llGetListEntryType` of each element. `osListAsInteger` of the float element is 0. `osListAs*` at -1 is the typed zero. One `osListAs*` call with the master switch off still returns.

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

### P4.F2.4 Sort, statistics, and randomize

Depends on P4.1c, P4.19. `osListSortInPlace` and `osListSortInPlaceStrided` also depend on P4.16.

Functions: `llListSort`, `llSortListStrided`, `llListSortStrided`, `llListStatistics`, `llListRandomize`, `osListSortInPlace`, `osListSortInPlaceStrided`.

Source: `LSLSystemAPI.cs` 16159–16208, 16323–16361, 16391–16437, 21554–21555, 12076–12108. `LSLList.Sort` is `Source/InWorldz.Phlox/Types/LSLList.cs` 534–602. Constants: `DefaultConstants.cs` 577–587 and the copies at `LSLSystemAPI.cs` 16391–16393. YEngine `llListSort` is `src.Sort(stride, ascending == 1)` (`LSL_Api.cs` 6247–6249). YEngine has `llListSortStrided` (6252–6254) and no `llSortListStrided`.

`llListSort` (16159–16187) does not call `LSLList.Sort`. It stringifies element 0 of each stride with `ToString()`, tries `double.TryParse` with the current culture, and compares the numbers when both parse. Otherwise it compares the strings ordinally. A non-zero `ascending` sorts ascending, including the value 2. A stride under 1 becomes 1. When the length is not a multiple of the stride, the tail is dropped. `LSLList.Sort` (534–602) is the feathered homogeneous bubble sort. Unequal types compare equal, so they keep their relative order, and the tail is kept. The in-place comment at 12081 says the order cannot drift from LSL's. The method it calls is `llListSort`, not `LSLList.Sort`. Proposed TB169. The default under [D124](../DECISIONS.md) is the API method. Question 1 offers `LSLList.Sort` as the override. YEngine uses `src.Sort` and treats only `ascending == 1` as ascending (6247–6249). Recorded.

`llSortListStrided` (16323–16361) returns the same list when the length is not a multiple of the stride and the stride is over 1. A `stride_index` outside `[-stride, stride)` returns a new empty list. Only `ascending == 1` sorts ascending. Any other value, including 2 and 0, sorts descending. Index 0 delegates to `llListSort`. Other indexes compare ints, floats and doubles by value, and everything else by ordinal `ToString()`. `llListSortStrided` forwards to it (21554–21555).

`osListSortInPlace` (12083–12088) and `osListSortInPlaceStrided` (12103–12108) are the master switch. They sort the caller's `LSLList` and write the result back with `SortInPlace` (12096–12099), which copies `src.Data.Length` elements. `Array.Copy` throws when the sorted list is shorter. `llListSort` is shorter when the stride does not divide the length. `llSortListStrided` is empty when the stride index is outside the range (16332–16333), and `osListSortInPlaceStrided` calls `llListSortStrided`, which forwards there. The script dies. Proposed TB168. When the stride divides the length and the index is in range, both stores (`Data` and `Members`) take the new order, so a later `llList2String` sees it.

`llListStatistics` (16401–16437): `LIST_STAT_RANGE` 0 through `LIST_STAT_GEOMETRIC_MEAN` 9, and `LIST_STAT_HARMONIC_MEAN` 100 (`DefaultConstants.cs` 577–587). Non-numeric elements are skipped because `Convert.ToDouble` throws and the catch drops them. The culture is the current culture. An empty list or a list with no numbers returns 0. `STD_DEV` of fewer than two numbers is 0. The geometric mean is `exp(log(product) / n)` and is NaN when the product is negative. The harmonic mean is `n / sum(1/x)`. An unknown operation returns 0. The oracle uses the constant name, which compiles to 100, not the number 10.

`llListRandomize` (16189–16208) is a Fisher-Yates of stride chunks on `new Random()`. A stride of 0 or less becomes 1. When the length equals the stride, or the stride does not divide the length, the result is a copy in the same order. Not bit-exact.

Oracle: `llListSort` of a mixed int and string, stride 1, ascending 1 and ascending 2. A stride that does not divide the length, so the tail is gone. The same inputs named as the case that would differ if the port called `LSLList.Sort` (question 1). The pass is the API method's recording. `llSortListStrided` with `stride_index` 0, with `stride_index` 1, with `ascending` 2, with a bad index (empty list), and with a length that is not a multiple (the same list). `llListStatistics` of each constant from `RANGE` through `GEOMETRIC_MEAN`, of `HARMONIC_MEAN`, of an unknown operation, of a one-element `STD_DEV`, and of a negative product. `llListRandomize` with a stride that does not divide (order unchanged) and with a dividing stride (a permutation of the chunks, type and length exact). `osListSortInPlace` on a dividing stride, then `llList2String` of element 0, so the in-place write is visible. The same call with a stride that does not divide: the shout and the stop. `osListSortInPlaceStrided` with a bad stride index: the shout and the stop. One allowed call with the master switch on, and one denial with it off.

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

Source: `LSLSystemAPI.cs` 17766–17839, 17875–17883, 18204–18243. `JSON_APPEND` is -1 (`SlConst.cs` 117).

The constants (17875–17883) are U+FDD0 through U+FDD8 in the order invalid, object, array, number, string, null, true, false, delete. `TryJsonFind` (17802–17821) uses `System.Text.Json`, strips a leading BOM (17839), indexes an array with an integer specifier, and names an object member with a string specifier. Anything else, or a miss, is `JSON_INVALID`. `llJsonGetValue` (17770–17882) returns a string unquoted, `JSON_TRUE`, `JSON_FALSE` or `JSON_NULL` for those kinds, and the raw JSON text for a number, an object or an array. `llJsonValueType` (17785–17798) returns the matching constant.

`llJson2List` (18210–18229): an empty string is an empty list. Text that is not JSON is a one-item list of the BOM-stripped source. An array becomes its elements. An object becomes key, value, key, value. A nested object or array is that element's raw JSON text. A number that fits in int32 stays an int. Any other number is `GetSingle()` (18236–18237). true, false and null become the JSON constants. A string is unquoted.

Oracle: `llJsonGetValue("true", [])` is `JSON_TRUE`. A string value comes back unquoted. An integer specifier on an array, a string specifier on an object, a specifier of the wrong kind (`JSON_INVALID`), a BOM-prefixed document, and a missing member. `llJsonValueType` of each kind. `llJson2List` of `""`, of `not json`, of an array of a small integer and of a number that does not fit in int32, of an object, and of a nested array (the nested text is one string element). Types and order are exact. The constants compare as the exact code points.

### P4.F2.7 JSON setters

Depends on P4.1c, P4.19.

Functions: `llJsonSetValue`, `llList2Json`.

Source: `LSLSystemAPI.cs` 17885–18201. `JSON_APPEND` is -1 (`SlConst.cs` 117).

`llJsonSetValue` (17885–17904): an empty specifier list returns `JSON_INVALID`. A BOM is stripped. An empty document is built from the specifiers. An integer specifier starts an array, so `llJsonSetValue("", [0], "x")` is `["x"]`. A string specifier starts an object. A parse failure or a failed set returns `JSON_INVALID`. The mutable tree keeps object member order (17912, 17956–17969). `JSON_APPEND` (-1) appends to an array (18034–18038). An index equal to the length appends. An index under 0 or past the length returns `JSON_INVALID`, except the append value. `JSON_DELETE` removes an array element or an object member (18023, 18049, 18073). A child of the wrong shape is replaced by the rest of the path (18058–18061, 18081–18085).

`JsonValueToNode` (17992–18011): null becomes JSON null. `JSON_TRUE`, `JSON_FALSE`, `JSON_NULL` and the words `true`, `false` and `null` become literals. A framed object or array that parses is kept. A JSON number (`JsonNumber`, 17871–17872) is written bare. Anything else is quoted by `JsonQuote` (17845–17867), which escapes quotes, backslashes and controls and writes other characters as themselves.

`llList2Json` (18133–18166): `JSON_ARRAY` or `JSON_OBJECT`, and any other type is `JSON_INVALID`. An object of odd length, or a key that is not a string, is `JSON_INVALID`. `JsonValueFromObject` (18170–18192): an int is invariant digits. A float or double NaN is `"NaN"`. Infinities are `"Inf"` and `"-Inf"`. A finite float uses invariant `ToString`. A string is trimmed. true, false, null and the three JSON constants become literals. A framed valid object, array or quoted string is kept (`IsFramedJson`, 18196–18201). Any other string is quoted. A vector or rotation is quoted from its six-digit list string.

Oracle: set into an empty string with index 0 and with a key. Set a nested path. Append with `JSON_APPEND`. Delete an element and delete a missing key. A bad document returns `JSON_INVALID` and the script keeps running. `llList2Json` of an array, of an odd-length object, of a non-string key, of true, false and null, of a framed object, of a JSON number string (quoted, because a number string is quoted), of NaN, and of both infinities. The text is exact.

### P4.F2.8 `iwParseString2List`

Depends on P4.1c, P4.19. About a day by itself.

Functions: `iwParseString2List`.

Source: `LSLSystemAPI.cs` 16453–16611.

An empty source returns `[]` (16456). Both separator and spacer lists empty returns `[]` even when the source is not empty (16511–16513). That is the behaviour this row reproduces. Question 10. A spacer that equals a separator is dropped (16496–16505). The walk uses `IndexOf` from the start of the remaining source. Equal indexes keep the first delimiter in the combined array. A leading or doubled separator always keeps an empty field, even when `keepnulls` is false (16552–16556). The comment cites Halcyon.

The args walk pairs (16467–16493). Names are lowercased. `keepnulls` is true only when the integer is 1. `trimstrings` outside `0 .. 3` becomes 0. `maxsplits` under 0 becomes 0. `capitalize` outside `0 .. 2` becomes 0. `autocast` outside `1 .. 2` becomes 0. `ParseString2ListApply` (16588–16594) trims with `llStringTrim` and then `ToUpper` or `ToLower` (current culture). `AutoCastString` (16598–16611): one dot tries a float, zero dots try an int, angle brackets with two commas try a vector and with three try a rotation, and length 36 tries a UUID which is stored back as a string. `TryParse` uses the current culture. `autocast` 1 casts fields. `autocast` 2 also casts the spacer text (16569). A field that fails the parse stays a string.

Oracle: an empty source. A non-empty source with both delimiter lists empty (the empty list). A leading separator with `keepnulls` 0 (the empty field is kept). One spacer that equals a separator (the spacer is not emitted). `maxsplits` of 1. `trimstrings` 3 and `capitalize` 1. `autocast` 1 on an int, a float, a vector, a rotation and a 36-character UUID. `autocast` 2 on a spacer. A value that does not parse stays a string. Types and order are exact.

### P4.F2.9 List match and edit

Depends on P4.1c, P4.19.

Functions: `iwMatchList`, `iwListIncludesElements`, `iwReverseList`, `iwListRemoveElements`, `iwListRemoveDuplicates`.

Source: `LSLSystemAPI.cs` 16617–16758.

`listCompare` (16617–16632) requires equal length. A type of 0 on either side, or unequal types, returns 0. Integers compare by value. Floats compare with `!=`, so a NaN fails the compare. Strings and keys compare the six-digit `GetLSLStringItem`. Vectors and quaternions compare with `!=`.

`iwMatchList` (16635–16665): both empty and type at most 1 returns 1. One side empty returns 0 for those types, and 0 for every other type. Type -1 is equal. Type 0 is a head. Type 1 is a tail. Types 2, 3 and 4 shout `IW_MATCH_REGEX not implemented for iwMatchList.` (and the count wordings at 16659 and 16662) through `ScriptShoutError`, so the 15 ms applies, and then return 0. The script keeps running.

`iwListIncludesElements` (16667–16690) uses `object.Equals`. An int 1 does not match a float 1.0. Empty on either side returns 0. `any == 1` returns 1 on the first hit and 0 when none hit. Any other `any` returns 1 only when every element is found.

`iwReverseList` (16692–16706): length 0 or 1 returns the same list. A stride under 1 reverses elements. A length that is not divisible by the stride shouts `Error: stride argument is {n}, but source list length is not divisible by {n}` and returns `[]`. Otherwise each stride is reversed as a block, and the blocks themselves run last to first.

`iwListRemoveElements` (16708–16753): empty on either side returns the source. A count of 0 becomes -1, which means all. Mode 0 looks for a contiguous run equal to `elements`, using `Equals`, and removes up to `count` runs. The scan stops at `src.Length - elements.Length`, so a non-matching tail shorter than the pattern is left out of the result. Any other mode keeps an element that is not in `elements`, and only while the kept count is under `count` (or while `count` is -1). A matching element is dropped. Once `count` non-matches have been kept, the rest of the list is dropped too. This plan describes that control flow and reproduces it. Question 11. It is not a ledger row.

`iwListRemoveDuplicates` (16755–16758) returns the source when the length is 0 or 1, and otherwise `Distinct()`.

Oracle: `iwMatchList` of two empty lists, of a head, of a tail, of unequal types, of a float NaN, and of type 2 (the shout, 0, script still running, 15 ms). `iwListIncludesElements` of an int against a float, of `any` 1, and of `any` 0. `iwReverseList` of a short list with stride 1, with stride 0, and with a stride that does not divide (the shout and `[]`). `iwListRemoveElements` mode 0 of one run, mode 0 where the pattern is longer than the unmatched tail, mode 1 with count 1 (the tail drop), and count 0 (every match removed). `iwListRemoveDuplicates` of a list with two equal strings and of a one-element list (the same list). Order and types are exact.

## 4. Proposed ledger rows

The ledger's highest id on this branch is TB164. These six are the next ids, starting at TB167, which was the free id for this pass. They are not filed here. The maintainer adds the ledger rows. Ferroan reproduces Phlox on each of them until a decision says otherwise. None of them is a viewer protocol.

| # | C# | What goes wrong | Ferroan does |
| --- | --- | --- | --- |
| TB167 | `LSLSystemAPI.cs` `osStringLastIndexOf` (12521–12527) at `c401bceecd` | The five-argument form passes `offset` and a forward `count` to `String.LastIndexOf`, which searches backward and throws `ArgumentOutOfRangeException` when `offset - count + 1 < 0`. Offset 0 and count 0 set the count to the whole length and throw on a string longer than one character. The three-argument form does not. The script dies with the shout in section 1. Read from the call. Not executed. | Reproduce the throw and the stop (P4.F1.12). |
| TB168 | `LSLSystemAPI.cs` `SortInPlace` (12096–12099), `osListSortInPlace` (12083–12088), `osListSortInPlaceStrided` (12103–12108) at `c401bceecd` | `Array.Copy` asks for `src.Data.Length` elements. `llListSort` drops the tail when the stride does not divide the length (16159–16187). `llSortListStrided` returns an empty list when the stride index is outside `[-stride, stride)` (16332–16333), and the strided in-place call goes through `llListSortStrided` (21554). The copy throws. The script dies. | Reproduce the throw and the stop (P4.F2.4). |
| TB169 | `LSLSystemAPI.cs` `llListSort` (16159–16187) and `LSLList.Sort` (`LSLList.cs` 534–602) at `c401bceecd` | The API method stringifies, parses with the current culture, drops a tail that is not a whole stride, and treats any non-zero `ascending` as ascending. It does not call `LSLList.Sort`, the feathered sort that keeps unequal types in place and keeps the tail. The comment at 12081 says the in-place order cannot drift from LSL's. YEngine calls `src.Sort` (`LSL_Api.cs` 6247–6249). | Reproduce the API method (P4.F2.4). Question 1. |
| TB170 | `LSLSystemAPI.cs` `llComputeHash` (15943–15984) and `llHMAC` (15990–16022) at `c401bceecd` | The comment at 15945 lists `sha224`. Neither switch has a `sha224` case. An unknown name shouts and returns `""`. YEngine implements `sha224` (`LSL_Api.cs` 21692–21697 and 22330–22331), is case-sensitive, and returns `""` with no shout. | Reproduce the shout and the empty string (P4.F1.9). Question 2. |
| TB171 | `LSLSystemAPI.cs` `llComputeHash` `md5_sha1` (15970–15978) and `llHMAC` (15990–16022). Signatures `ISystemAPI.cs` 970 and `ILSLSystemAPI.cs` 801. YEngine `ILSL_Api.cs` 549 and `LSL_Api.cs` 21673–21684, 22314–22359. At `c401bceecd` | Phlox `md5_sha1` is SHA-1 of the lowercase hex MD5. YEngine concatenates the raw MD5 bytes and the raw SHA-1 bytes of the input. Phlox HMAC takes `(msg, privateKey, algorithm)` and returns base64. YEngine takes `(private_key, message, algo)` and returns lowercase hex. | Reproduce Phlox (P4.F1.9). Question 3. |
| TB172 | `LSLSystemAPI.cs` `osMax` (12406–12409) at `c401bceecd` | The gate key is `osGetRezzingObject`, threat `None`. `osMin` is ungated (12412–12415). With the shipped file the call is allowed, because `None` passes the threat test and `Allow_osGetRezzingObject` is commented out (`osslDefaultEnable.ini` 85). `Allow_osMax` is not consulted. Uncommenting `Allow_osGetRezzingObject = false` denies `osMax`. | Reproduce the key (P4.F1.11). Question 4. |

`llXorBase64Strings` is not a ledger row. The padding and the 300 ms sleep are the documented behaviour (`LSLSystemAPI.cs` 15856–15860).

## 5. Open questions

The earlier questions in [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 6 stay as they are. These are new.

1. List sort. P4.F2.4 reproduces `llListSort` as written (TB169). The other choice is `LSLList.Sort`. YEngine already calls that sort. This plan does not switch.
2. `sha224` and case. P4.F1.9 reproduces Phlox: lowercase the name, no `sha224`, shout and `""` (TB170). YEngine implements `sha224`, is case-sensitive, and is silent.
3. HMAC order and `md5_sha1`. P4.F1.9 reproduces Phlox's `(msg, privateKey, algorithm)`, base64, and SHA-1 of the hex MD5 (TB171). A Linden script passes the key first, which is YEngine's order and hex. Whether the SLua `ll` table should take the key first is the question. This plan does not pick it.
4. `osMax` gate key. P4.F1.11 reproduces the key `osGetRezzingObject` (TB172). The other choice is the key `osMax`.
5. Culture. `llToUpper`, `llToLower`, `double.TryParse` in `llListSort`, `Convert.ToDouble` in `llListStatistics`, `string.Format` in `osFormatString`, `osStringStartsWith` and `osStringEndsWith`, and `TryParse` in `iwVerifyType` and `iwParseString2List` use the current culture. `CastToFloat` is `en-US`. The six-digit printer is invariant. The harness records the culture it ran under. The question is whether Ferroan uses the process culture or the invariant culture.
6. Float pass. Section 1 says the six-digit chat text is the pass, and a 1 ulp that changes the sixth digit fails. A wider ulp than that text is not proposed. Confirm that this is how section 4 of the stage-1 plan applies to these families.
7. Uncaught throws. Section 1 kills the script and shouts. That covers `llModPow` with modulus 0, `llUnescapeURL` of a bad `%` sequence, `llEscapeURL` of a very long string, `osFormatString` of a bad format, `osReplaceString` of a pattern that does not compile, `llSubStringIndex` of a null pattern, and the two copy and `LastIndexOf` throws (TB167, TB168). The other choice is to catch and return a value. This plan reproduces the kill.
8. `iwParseString2List` with both delimiter lists empty returns `[]` for a non-empty source (16511–16513). P4.F2.8 reproduces that.
9. `iwListRemoveElements` mode other than 0 drops matches and also drops the tail once `count` non-matches have been kept. Mode 0 leaves out a non-matching tail shorter than the pattern (16708–16753). P4.F2.9 reproduces that control flow.

`osPerlinNoise2D` is a pure function of its arguments (`TerrainUtil.cs` 110–121). It stays in P4.F1.11. The 24 `iw` names that leave P4.S27 are the 18 in section 2 and the 6 in section 3. The other 61 stay on P4.S27, including `iwFormatTime`.
