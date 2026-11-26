# Phase 4 communication functions

Planning only. This file splits the communication functions from [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 2 into day-sized rows. It does not add a `TASKS.md` row or a `WORK_QUEUE.md` row, and it does not edit the ledger. The pin is `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. [D124](../DECISIONS.md) is settled: Phlox is the reference for the queue, the sleeps, and the `iw*` functions. A YEngine difference is recorded here and is not the pass rule.

The placeholders this file takes from are all of P4.S3 (20), `llMessageLinked` from P4.S4, the HTTP eight from P4.S5, the notecard reads from P4.S6, the three notecard reads from P4.S19, all of P4.S24 (3), all of P4.S25 (2), and four notecard reads from P4.S27. Stage 2 may split a family. It does not move a method into a second family and add it again. An id is a label. Each row below is about a day. A row that overruns a day is split before the next id starts.

C# paths are under `Source/` in the pin. `LSLSystemAPI.cs`, `PhloxEngine.cs`, `PhloxListenManager.cs`, `PhloxExecutionScheduler.cs`, `OsslGate.cs` and `SlConst.cs` are in `Source/Phlox.ScriptEngine/`. The HTTP plugin is `Source/Phlox.ScriptEngine/AsyncCommand/Plugins/HttpRequest.cs`. `Encoding.cs` is `Source/InWorldz.Phlox/Util/Encoding.cs`. `SyscallContext.cs` and `SyscallShim.cs` are in `Source/InWorldz.Phlox/Glue/`. `ChatModule.cs` is `Source/OpenSim.Region.CoreModules/Avatar/Chat/ChatModule.cs`. `LLClientView.cs` is `Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs`. `ChatTypeEnum.cs` is `Source/OpenSim.Framework/ChatTypeEnum.cs`. `LSL_Api.cs` is `Source/OpenSim.Region.ScriptEngine.Shared/Api/LSL_Api.cs` and `ILSL_Api.cs` is `Source/OpenSim.Region.ScriptEngine.Shared/Api/Interfaces/ILSL_Api.cs`. `HttpRequestModule.cs` is `Source/OpenSim.Region.CoreModules/Scripting/HttpRequest/HttpRequestModule.cs`. `UrlModule.cs` is `Source/OpenSim.Region.CoreModules/Scripting/LSLHttp/UrlModule.cs`. `LSL_Constants.cs` is `Source/OpenSim.Region.ScriptEngine.Shared/Api/ScriptBase/LSL_Constants.cs`. The shipped OSSL file is `Source/OpenSim.Server.RegionServer/AppData/config-include/osslDefaultEnable.ini`. The shipped function caps are `Source/OpenSim.Server.RegionServer/AppData/RegionServerDefaults.ini`.

## Counts

| Family | Methods | Rows |
| --- | --- | --- |
| `ll` chat, listen, dialog, map, IM and email | 20 | P4.F3.1–P4.F3.6 |
| `ll` link message | 1 | P4.F3.7 |
| `ll` HTTP out and HTTP in | 8 | P4.F3.8–P4.F3.10 |
| `ll` notecard reads | 6 | P4.F3.11, P4.F3.12 |
| `os` chat and object message | 2 | P4.F3.3, P4.F3.7 |
| `os` HTTP | 3 | P4.F3.9, P4.F3.10 |
| `os` notecard reads | 3 | P4.F3.12 |
| `iw` notecard reads | 4 | P4.F3.11 |

20 + 1 + 8 + 6 + 2 + 3 + 3 + 4 = 47. The overloads counted above are `llMapBeacon` (2), `llTargetedEmail` (2) and `llFindNotecardTextSync` (2). No `iw*` name is overloaded. P4.F3.4a is an extra day for that same 20. It adds no method.

P4.S4 keeps the 13 `llLinksetData*` methods. P4.S5 keeps the five XML-RPC methods (`llOpenRemoteDataChannel`, `llSendRemoteData`, `llRemoteDataReply`, `llCloseRemoteDataChannel`, `llRemoteDataSetRegion`) and the 16 experience and experience key-value methods. P4.S6 keeps give, rez, derez and transfer. P4.S19 keeps `osMakeNotecard` (both) and the other inventory names. P4.S27 keeps `iwMakeNotecard`. The pure-function rows already take 24 `iw*` names and leave 61; these four are four of that 61. `osRegionNotice` (both), `osNpcSay` (both) and every `bot*` name stay in their own placeholders. Other `dataserver` producers stay in their own families: `llRequestAgentData`, `llRequestInventoryData`, `llRequestSimulatorData`, `llRequestUsername`, `llRequestDisplayName`, `llRequestUserKey`.

## 1. How a row is proven

P4.19 builds `tools/script-oracle` on `Tests/InWorldz.Phlox.Tests/SchedulerHarness.cs`: a real Phlox engine, a script rezzed the way `OnRezScript` does, and `DoWork` pumped by hand. These rows do not add P4.19p or any later letter. The cases below are what that harness runs for the row. Expected text, keys, integers, event order and sleep durations are the recording. They are not numbers copied out of the cited lines. Ordinary CI does not start Tranquillity.

The compare, for every row in this file:

- An integer, a string, a key, a list element's type, list order, error text, a channel, a link number, a handle, and a sleep duration match the recording exactly.
- A chat line the script hears is the text the API already cut. This file's returns are integers, strings and keys. Text a script sees compares exactly. A float said with `(string)` is six digits (`Encoding.FloatToStringWith6FractionalDigits`, `Encoding.cs` 162–165). A vector or rotation said with `(string)` is five digits (`Vector3ToStringWith5FractionalDigits` and `QuaternionToStringWith5FractionalDigits`, `Encoding.cs` 140–148).
- A viewer `ChatFromSimulator` (packet 139, `LLClientView.cs` 1108–1152) matches the recording field for field: source type, chat type, audible, source id, owner id, name, message bytes, position. Channel-0 object say of `Script running` stays the P4.1d compare. This file does not reopen it. The 15 ms `ChatSleep` and YEngine's 2000 ms 12th-call sleep are not part of that packet.
- A script listen treats a speaker at exactly the configured range as outside (`PhloxListenManager.cs` 557, `>=` on the squared distance). `TrySendChatMessage` uses a strict less-than (`ChatModule.cs` 445–451), so a viewer still hears that distance. Both stay. Shout reach of 100 m is the same test on both engines and is not a tolerance ([TB159](../TRANQUILLITY_BUGS.md)).
- A denied `os*` call compares the thrown gate error, which is the recording P4.16 owns. `OsslGate.Check` throws `"OSSL Permission Error: " + why` (`OsslGate.cs` 126–127). The master switch, and a missing `[OSSL]` enable, throws `OSSL Permission Error: All unsafe OSSL funtions disabled` (124), with the `(OWNER)` prefix when `m_errorToOwner` is set (130). A parseable `Allow_<name> = false` leaves `AllowedControl` at 0 (146–148) and `Test` returns `{function} disabled in region configuration` (213). The threat test is not reached. An empty `Allow_` and `Creators_` pair falls through to the threat test, and the text is then `{function} permission denied.  Allowed threat level is {max} but function threat level is {level}` (211), still under the prefix, and only when the function's level is above the configured maximum. An estate-form `Allow_` line that matches none of the owner flags returns `{function} permission denied` (240–241), under the same prefix. The script is killed (`PhloxExecutionScheduler.cs` 1442–1449, `TerminateWithError` 3452–3476).

An `ll` row depends on P4.1c and P4.19. Chat that leaves a viewer packet also depends on P4.1d's object-chat path. Listens also depend on P4.7 and P4.19c. Link messages also depend on P4.2, P4.5, P4.9 and P4.19e. An `os` row also depends on P4.16. An `iw` row depends on P4.1c and P4.19. A call that parks on the service lane also depends on P4.21 and P4.19n. Sleep numbers are recorded on the harness (P4.19o). Implementing those numbers is P4.23. The recording does not wait on P4.23.

The harness runs with the shipped `[OSSL]` (`config-include/osslDefaultEnable.ini`, `OSFunctionThreatLevel = VeryLow` at line 33) and the shipped `[InWorldz.Phlox]` defaults: `ChatThrottle`, `LinkMessageThrottle`, `NotecardThrottle` and `HttpInFlightThrottle` true, `NotecardPause` `sl` (`PhloxEngine.cs` 304–323). A denied call is a second case. A Halcyon notecard pause is a second case with `NotecardPause = halcyon`.

`Vector3` in the Phlox file is an OpenMetaverse type (`using OpenMetaverse`, `LSLSystemAPI.cs` 56). It is not `System.Numerics`.

### What the script sees on a throw

`OsslCheck` throws `VMException` (`OsslGate.cs` 114–127). The tick catch stops the script. The shout is `Script error: ` plus `Script {asset} encountered a problem and was stopped: {message}` (`LSLSystemAPI.cs` 658–662, scheduler 3475). There is no second return value. `ShoutError` (`LSLSystemAPI.cs` 658–662) speaks `Script error: ` plus the text on `DEBUG_CHANNEL` and does not sleep. `ScriptShoutError` (269–272) does that and then `ChatSleep`. `LSLError` prefixes `LSL Runtime Error: ` and goes through `ScriptShoutError` (277). `YEngineError` (15212–15218) speaks `command: message`, cut to 1023 characters, with no `Script error: ` prefix and no sleep of its own.

A shout that returns a key or `""` leaves the script running. A gate denial and an uncaught exception do not.

### Where a sleep is stored

`ScriptSleep` (`LSLSystemAPI.cs` 241–249) adds to `SyscallContext.DelayMs` when the current context belongs to this script (`SyscallContext.cs` 65). Otherwise it writes `NextWakeup` and sets `RunState` to `Sleeping`. A second write on the script thread replaces the first. It does not add. Two adds inside one context do add.

`RunAsync` (`SyscallShim.cs` 1086–1105) always enters a context. `Defer` (1125–1149) enters one only when `DeferServiceCall` is set and `NeedsService` returns true. `NeedsService` (`LSLSystemAPI.cs` 91–173) defaults to true. The shipped mode is `ServiceCallDeferral` `auto` (`PhloxEngine.cs` 325–328). The email cases in this file run with that lane installed. A host with no lane runs the `Defer` body inline, and a second `ScriptSleep` replaces `NextWakeup`.

`llSay`, `llShout`, `llWhisper`, `llOwnerSay`, `llRegionSay`, `llRegionSayTo`, `llMessageLinked`, `llHTTPRequest`, the async notecard readers, `llTextBox`, `llLoadURL`, `llMapDestination`, both `llMapBeacon` forms, and `llFindNotecardTextCount` are inline shims. `llInstantMessage`, `llEmail` and `llGetNextEmail` are `RunAsync`. Both `llTargetedEmail` forms, `llDialog`, the synchronous notecard reads and the three `osGetNotecard*` reads are `Defer`.

### YEngine, once

YEngine stores no `iw*`. It declares the list form of `llMapBeacon` and the three-argument `llTargetedEmail` (`ILSL_Api.cs` 285, 449). It does not declare the agent form of `llMapBeacon` or the four-argument `llTargetedEmail`. Chat, IM, email, dialog, text box, owner say and link message differences are on the rows below. The Phlox harness is the proof.

### The gate on these `os*` calls

`OsslCheck(level, name)` looks up `Allow_` under `name` (`OsslGate.cs` 132–210). An empty pair falls through to the threat test. The shipped file:

| Call | Threat | Shipped `Allow_` |
| --- | --- | --- |
| `osListenRegex` | `Low` | `true` (118) |
| `osMessageObject` | `Low` | estate form, `osslParcelOG`, line 122 |
| `osRequestURL`, `osRequestSecureURL` | `Moderate` | the same estate form with `osslParcelO` (148, 149) |
| `osGetNotecard`, `osGetNotecardLine`, `osGetNumberOfNotecardLines` | `VeryHigh` | the same estate form (213–215) |
| `osSetContentType` | `Severe` | `false` (230) |

`osListenRegex` is allowed by the shipped `true`. Setting that line to `false` is `osListenRegex disabled in region configuration`. `osSetContentType` is shipped `false`, so the shipped file throws `osSetContentType disabled in region configuration`. The threat test is not reached for either. The other six use an estate-form `Allow_` line and no `Creators_` line. An owner the flags accept is allowed. An owner they do not accept gets `{function} permission denied` (`OsslGate.cs` 240–241). The refused case is the second harness run.

### Scene and grid

Viewer chat today is avatar-only. `crates/ferroan-client/src/chat.rs` drops a channel other than 0 and `DEBUG_CHANNEL` (228–229), sends source type agent (66, 317), and sets the owner id to the speaker (316). The file header says object chat and instant messages are not ported (35–37). P4.1d adds channel-0 object say. Object whisper, shout, region say, owner say, direct say, and a non-zero channel that still reaches a script listen, are this file. The 1100-to-1000 character cut (289–304) is the viewer-line cut. A script's say, shout, whisper or region say is cut in the API before that (`LSLSystemAPI.cs` 1532–1591). `llOwnerSay` is not cut there.

Dialogs, a text box, load-URL, the map beacon and the map destination are viewer UI. The host call and the sleep stay on P4.F3.4. The viewer packets are P4.F3.4a. Map tiles in [PLAN.md](../PLAN.md) are a different surface. Instant messages need the region's `IMessageTransferModule` sender, P4.30. Email needs `IEmailModule`, P4.31. HTTP out needs `IHttpRequestModule`, P4.32. HTTP in needs `IUrlModule`, P4.33. With the module absent, Phlox still returns as the bodies below say. Notecard bytes need the asset service. Task inventory is already on the part from Phases 1 and 2. A worn attachment, for the parcel-hide case, is P4.1f. This file does not invent a Phase 5 id.

`ChatTypeEnum` (`ChatTypeEnum.cs` 30–42) is Whisper 0, Say 1, Shout 2, DebugChannel 6, Region 7, Owner 8, Direct 9. `DEBUG_CHANNEL` is 2147483647 (`SlConst.cs` 77). `ChatSourceType.Object` is 2.

## 2. Chat and listen

The 20 communication methods on P4.S3 are `llWhisper`, `llSay`, `llShout`, `llListen`, `llListenControl`, `llListenRemove`, `llInstantMessage`, `llEmail`, `llGetNextEmail`, `llDialog`, `llOwnerSay`, `llLoadURL`, `llMapDestination`, `llRegionSay`, `llTextBox`, `llRegionSayTo`, `llMapBeacon` (both), `llTargetedEmail` (both). `llMessageLinked` is the one link method in section 4. It is not one of the 20.

`OnChatFromWorld` returns unless the channel is 0 or `DEBUG_CHANNEL` (`ChatModule.cs` 189–195), so a script say on any other channel reaches listens and does not reach a viewer. `DeliverChatToAvatars` rewrites a debug channel to chat type 6 (212) before the owner test. Object chat on that type is sent only to the owner when `SenderObject` is a part (245–257, 281–301). A worn attachment sets the parcel-hide flag only in that same part branch (251–255). The hide runs when the type is below `DebugChannel`, the destination is zero, and the parcel has `SeeAVs` off (267–279, 317–320). Phlox `HostSimChat` (`LSLSystemAPI.cs` 670–683) calls `Scene.SimChat` with `fromAgent` false, so `SenderObject` is the prim. Listens hear the spoken type through `PhloxListenManager.SpokenType` (460–498), which puts a debug-channel error back to say distance.

### P4.F3.1 Whisper, say and shout

Depends on P4.1c, P4.1d, P4.7, P4.19. The 15 ms number is the P4.19o recording. Implementing it is P4.23. The worn-attachment case also depends on P4.1f.

Functions: `llWhisper`, `llSay`, `llShout`.

Source: `LSLSystemAPI.cs` 1539–1561 and `CapUtf8Bytes` 1572–1581. YEngine: `LSL_Api.cs` 1277–1348.

Each call cuts the text to 1024 UTF-8 bytes and drops a character the cut would split (1572–1581), then `HostSimChat`, then `ChatToWorldComm` so a YEngine listen hears the same cut, then `ChatSleep`. `ChatSleep` is 15 ms when `ChatThrottle` is true (257–260). The shim is inline, so that write is `NextWakeup`. Empty text is delivered (`PhloxListenManager.cs` 488–490). A prim does not hear itself (537–538). Ranges are whisper 10, say 20, shout 100. A speaker at that distance is not heard by a script. A viewer at that distance still is (`ChatModule.cs` 451).

YEngine cuts with `Utils.StringToBytesNoTerm(text, 1023)` (1279, 1323, 1342). It has no per-call sleep. A one-second window counts channel-0 says and shouts. The 12th call in the window sleeps 2000 ms, and once the count is at least 11 a say or shout on any channel sleeps (1287–1340). `llSay` on the script console channel writes the console and does not call `SimChat` (1317–1320). `llShout` passes `fromAgent` true (1342–1345). That leaves `SenderObject` null, so the owner id viewers see is zero, a debug-channel shout is not limited to the owner, and a worn-attachment shout skips parcel hide. That is [TB159](../TRANQUILLITY_BUGS.md). Channel-0 shout reach stays 100 m on both engines. Recorded. Question 1.

Oracle: whisper, say and shout on channel 0, on a non-zero channel, and on `DEBUG_CHANNEL`. A message of 1024 ASCII bytes, one more byte, and a multibyte character on the cut. An empty message. A second prim inside the range and one exactly at the range. A viewer packet for channel 0 and for `DEBUG_CHANNEL`, including the owner id and chat type 6. A non-zero channel that is not debug produces no viewer packet and does produce a listen. The wake is 15 ms. A worn attachment on a parcel with `SeeAVs` off, heard inside the parcel and not heard outside it. YEngine's zero owner, its debug-channel reach, and its 2000 ms window are recorded and are not a pass failure.

### P4.F3.2 Region say, region say to, and owner say

Depends on P4.1c, P4.1d, P4.7, P4.19. The 15 ms is P4.19o and P4.23, as on P4.F3.1.

Functions: `llRegionSay`, `llRegionSayTo`, `llOwnerSay`.

Source: `LSLSystemAPI.cs` 1594–1676. YEngine: `LSL_Api.cs` 1351–1388 and 13717–13723.

`llRegionSay` of channel 0 calls `ScriptShoutError("llRegionSay: cannot use channel 0")` and returns (1607–1612). The wake is the shout's 15 ms. Nothing is sent. Any other channel cuts to 1024 characters, dropping a split surrogate (`CapCharacters`, 1584–1591), speaks `ChatTypeEnum.Region`, and then `ChatSleep` (1614–1617). Region range is the whole region (`PhloxListenManager.cs` 506–507). On `DEBUG_CHANNEL` the chat module rewrites the type to 6 (212) and, because Phlox set `SenderObject`, only the owner is sent the packet (289–301). Listens still use the spoken region range through `SpokenType`.

YEngine `llRegionSay` cuts to 1023 bytes, returns on channel 0 with `Error`, and calls `SimChat` only on `DEBUG_CHANNEL`, with `fromAgent` true (1351–1369). That is the debug-channel reach in TB159. On any other channel YEngine listeners hear `ChatTypeEnum.Region` and viewers are not sent the chat. Recorded.

`llRegionSayTo` of `DEBUG_CHANNEL` shouts `Cannot use llRegionSayTo() on DEBUG_CHANNEL.` and returns with no further sleep (1644–1647). A non-key or `NULL_KEY` returns with no shout and no sleep (1649). Otherwise the text is the 1024-byte cut. Channel 0 to a present non-child avatar sends chat type Direct to that viewer, source id and owner id both the prim UUID (1652–1659), and does not also call `DeliverMessageTo` (1668). Listens are `DeliverChat` of type Direct, whole region, and only a prim `DestIdMatches` accepts (586–596): the destination prim, or an attachment whose owner is the destination. The sender's own prim does not hear it (538), including when the destination is that prim. Then `ChatSleep`. YEngine returns on `DEBUG_CHANNEL` with no shout, cuts to 1023 characters, and does not send when the key does not parse (1372–1387). Recorded.

`llOwnerSay` does not cut and does not call `HostSimChat`, so no listen hears it (1594–1605). When the owner presence is in the scene, one packet goes to that client: chat type Owner, source id the prim, owner id the prim, source type Object. The owner id is the prim UUID. That is proposed TB174. A missing owner, including a group-owned prim whose `OwnerID` is the group, sends nothing. `ChatSleep` runs either way. YEngine returns immediately when `OwnerID` equals `GroupID`, then `SimChatBroadcast` of type Owner on channel 0, and does not sleep (13717–13723). `OnChatBroadcast` drops type Owner unless the client is the object's `OwnerID` (`ChatModule.cs` 387–392). Recorded. Question 9.

Oracle: `llRegionSay` on channel 0 (the shout, no second packet, 15 ms). `llRegionSay` on a private channel (a listen in another prim, no viewer packet, 15 ms, text cut at 1024 characters). `llRegionSay` on `DEBUG_CHANNEL` (owner-only viewer packet, chat type 6, listens still region-wide). `llRegionSayTo` on `DEBUG_CHANNEL`, on `NULL_KEY`, on a bad key, on channel 0 to a present avatar (Direct, owner id the prim), and on a private channel to another prim and to an attachment of that avatar. `llOwnerSay` with the owner present, with the owner absent, and on a group-owned prim. No listen event for `llOwnerSay`. The packet owner id is the recording.

### P4.F3.3 Listen handles and `osListenRegex`

Depends on P4.1c, P4.7, P4.16, P4.19, P4.19c, and on the engine in [D131](../DECISIONS.md) (P4.34 through P4.34f). Filter order, the 64 and 1000 caps, the 20 per second rate, self-mute, and the exact-range miss are the P4.19c recording. This row does not add a second copy of those cases. Saved handles across a persist are P4.13c and P4.19k.

Functions: `llListen`, `llListenControl`, `llListenRemove`, `osListenRegex`.

Source: `LSLSystemAPI.cs` 5324–5360 and 14056–14064. `PhloxListenManager.cs` 224–306 and 288–289. YEngine: `LSL_Api.cs` 1390–1411.

`llListen` parses the key filter. A bad key leaves `UUID.Zero`, which is the wildcard (5327–5328). No listen manager returns -1 and logs a stub (5326). `Add` returns the same handle when the filter is already held (241–252). A new handle is the lowest integer at least 1 that this script does not hold (272–276). At the cap it returns -1 with no shout (255–269). The manager's default is 65 (`PhloxListenManager.cs` 113). The shipped `[LL-Functions] max_listens_per_script` is 64 (`RegionServerDefaults.ini` 2165), and that is the cap P4.7 and P4.19c record, so the 65th call is the refusal. The record is stored on the script state when the handle is positive (5347–5351). `llListenControl` sets active from `active != 0` (5355). `llListenRemove` drops the manager record and the saved record (5356–5359). A missing manager on control or remove is a no-op.

`osListenRegex` is threat `Low` and the shipped `Allow_` line is true, so the shipped file allows it. A bad key, including an empty string, returns -1 with no shout (14060). Bit 1 validates the name as a regex and bit 2 validates the message. An invalid pattern shouts `Name regex is invalid.` or `Message regex is invalid.` and returns -1 (14061–14062). `Add` stores a regex only when the bit is set and the string is not empty (288–289), so an empty name with bit 1 is the wildcard after the pattern check. The same handle, control and remove path is `llListen`'s.

YEngine `llListen` returns -1 when `IWorldComm` is missing, and `UUID.TryParse` of a bad key leaves the zero key (1390–1398). Its name and message filters run before distance. That order is P4.7. Recorded.

Oracle: two identical `llListen` calls (one handle). The lowest free handle after a remove. Handle 0 is not issued. A 65th listen is -1 and the script keeps running. Control off, then a chat the listen does not hear, then control on. Remove, then silence. `osListenRegex` with bit 1, bit 2, both, and neither. An empty name with bit 1. A bad pattern (the shout, -1, script running). A bad key (-1, no shout). One run with `Allow_osListenRegex = false` compares the disabled-in-configuration text and the stopped script. The filter-order cases stay on P4.19c.

## 3. Dialog, map, instant message and email

### P4.F3.4 Dialog, text box, load URL, map

Depends on P4.1c, P4.19. `llDialog` also depends on P4.21 when `NeedsService` defers it. The viewer packets are P4.F3.4a. This row records the host call and the sleep. It does not require a viewer.

Functions: `llDialog`, `llTextBox`, `llLoadURL`, `llMapDestination`, `llMapBeacon` (list), `llMapBeacon` (agent).

Source: `LSLSystemAPI.cs` 1724–1788, 10548–10550, 17500–17548, 21478–21495. YEngine dialog: `LSL_Api.cs` 8695–8755. YEngine text box: 5707–5732. YEngine declares only the list `llMapBeacon` (`ILSL_Api.cs` 285).

`llDialog` is `Defer` (`SyscallShim.cs` 3531–3538). `NeedsService` (147–153) returns false when the key is bad, the avatar is absent or a child, or the owner presence is here. Otherwise it defers when the owner account is not cached. A bad key shouts `LSL Runtime Error: First parameter to llDialog needs to be a key` and returns, no 1000 ms (1730). More than 12 buttons, a blank label, or a label longer than 24 characters shouts and returns, no sleep (1731–1740). An empty button list becomes one `OK` (1744). An absent or child avatar returns with no shout and no sleep (1746–1747). The send uses texture `UUID.Zero` (1771). The message is not length-checked. `ScriptSleep(1000)` runs only on the send (1775). The owner name is the presence name, or the account name when the owner is not in the region.

YEngine returns when the dialog module is missing (8699–8700). A bad or zero key shouts `First parameter must be a valid key` and returns (8702–8705). An empty list becomes `Ok` (8712–8715). The 24-character check is commented out (8741–8747). An empty message or a message over 512 UTF-8 bytes shouts and the dialog is still sent (8723–8730, 8751–8755). The texture is `00000000-0000-2222-3333-100000001000`. The sleep is 1000 ms after a send. Recorded.

`llTextBox` is inline. No dialog module returns with no sleep (1779–1780). A bad key shouts `LSL Runtime Error: First parameter to llDialog needs to be a key` and returns (1782). The word is `llDialog`. That is Halcyon's text, kept. Question 5. `NULL_KEY` sends nothing and sleeps 1000 ms (1784). A message longer than 1024 characters is cut (1785). Otherwise `SendTextBoxToUser` and `ScriptSleep(1000)` (1786–1787). YEngine returns on a missing module, shouts `First parameter must be a valid agent key` for a bad or zero key, shouts on an empty message or one over 512 UTF-8 bytes and still falls through, and sleeps 1000 ms only when the owner name resolves and the send runs (5707–5732). Recorded.

`llLoadURL` sleeps 100 ms on every path, including a missing module and a bad key (17539–17547). The URL is sent only when the module is present and the id parses. `llMapDestination` reads detect index 0, or the wearer when there is no detect and the prim is attached (17505–17515). `look_at` is not passed. `SendScriptTeleportRequest` is called with options 0. A `NullReferenceException` from that call is swallowed (17522–17532). `ScriptSleep(1000)` always runs (17537).

The list `llMapBeacon` sleeps 1000 ms and sends nothing (10548–10550). The comment says that is on purpose. The agent form parses the key, returns on a bad key or an absent or child agent, and logs. It sends no packet and does not sleep (21478–21495). The comment above the log talks about an IM. The body does not send one. Question 2.

Oracle: `llDialog` with a bad key, 13 buttons, a blank label, a 25-character label, an empty list (the button is `OK`), an absent avatar, and a present avatar (the host-call fields, including texture zero, and 1000 ms). `llTextBox` with a bad key (the shout names `llDialog`), with `NULL_KEY` (1000 ms, no packet), with a message of 1024 characters and one more, and a normal send. `llLoadURL` with a module and with none (100 ms either way). `llMapDestination` from a touch, from an attachment with no detect, and with no target (1000 ms each). The list beacon (1000 ms, no packet). The agent beacon for a present avatar, a child, and a bad key (no packet, no sleep). YEngine's `Ok`, its 512-byte shout that still sends, and its texture key are recorded and are not a pass failure.

### P4.F3.4a Viewer dialog, load URL, and map beacon

Depends on P4.F3.4, P4.1d and P4.7. About a day. This row adds the dialog module to `tools/script-oracle`, following `Tests/InWorldz.Phlox.Tests/RecordingDialogs.cs`. Without this path, `llDialog` and `llTextBox` do nothing for a viewer on a Ferroan region.

Out: `ScriptDialog` from `SendDialog` (`LLClientView.cs` 3259), `LoadURL` from `SendLoadURL` (3246), and `ScriptTeleportRequest` from `SendScriptTeleportRequest` (3871). In: `ScriptDialogReply` (`HandlerScriptDialogReply`, 9108–9124) becomes client chat on `OnChatFromClient`, type Region, on the dialog channel, with the button label as the message. `PhloxEngine.OnChatFromClient` (1048) delivers that chat. `DeliverSceneChat` (1058–1067) sets the speaker key from the client when `SenderUUID` is zero, so a key-filtered listen still matches.

Oracle: a present avatar receives `ScriptDialog` with texture zero and the button labels. The viewer's `ScriptDialogReply` is heard by a listen on that channel, including a listen filtered to the avatar's key. `llLoadURL` produces `LoadURL`. `llMapDestination` produces `ScriptTeleportRequest` with options 0. The list `llMapBeacon` produces no packet. The agent `llMapBeacon` produces no packet.

### P4.F3.5 Instant message

Depends on P4.1c, P4.19, P4.21, P4.19n, and on P4.30. The 2000 ms is the P4.19o recording. Implementing it is P4.23. Until P4.30 is done, this row proves the no-module return. This row adds the transfer module to `tools/script-oracle`.

Functions: `llInstantMessage`.

Source: `LSLSystemAPI.cs` 1678–1723. Shim: `SyscallShim.cs` 2318–2328 (`RunAsync`). YEngine: `LSL_Api.cs` 4318–4349, sleep field 153.

`ScriptSleep(2000)` runs first, inside the syscall context, so it is `AddDelay`. A bad or zero key then returns, with no shout. A missing transfer module returns the same way. The message is cut to 1023 UTF-8 bytes (`CapInstantMessage`, 1713–1723). An empty message stays empty and is still sent. The grid message is `MessageFromObject` (dialog 19), `offline` 1, session the prim UUID, `fromAgentID` the owner, `ParentEstateID` from `EstateSettings.ParentEstateID`, and the binary bucket `Region/x/y/z` through `StringToBytes256` (1688–1708).

YEngine returns with no sleep when the module is missing or the message is empty (4320–4321). A bad key shouts `An invalid key  was passed to llInstantMessage` (two spaces) and then sleeps 2000 ms (4323–4327). The cut is 1024 characters (4344). `offline` is 0. `ParentEstateID` is `EstateSettings.EstateID` (4341). The sleep is after the send (4349). Recorded.

Oracle: a normal send (the grid fields above, wake 2000 ms). An empty message (still sent, 2000 ms). A message of 1023 UTF-8 bytes and one more byte. A bad key and a zero key (no shout, 2000 ms, no send). No transfer module (2000 ms, no send). YEngine's empty-message skip, its shout text, its 1024-character cut, `offline` 0 and `EstateID` are recorded and are not a pass failure.

### P4.F3.6 Email

Depends on P4.1c, P4.19, P4.21, P4.19n, and on P4.31. The pause is the P4.19o recording. Implementing it is P4.23. Until P4.31 is done, this row proves the no-module return. This row adds the email module to `tools/script-oracle`.

Functions: `llEmail`, `llGetNextEmail`, `llTargetedEmail` (three arguments), `llTargetedEmail` (four arguments).

Source: `LSLSystemAPI.cs` 15432–15482 and 21574–21664. Shims: `SyscallShim.cs` 2331–2354 (`RunAsync`), 6350–6355 and 7148–7154 (`Defer`, and the names are not in the `NeedsService` switch, so the default is true). YEngine: `LSL_Api.cs` 138, 154, 605–618, 4352–4526. YEngine declares only the three-argument form (`ILSL_Api.cs` 449).

`llEmail` sends when the module is present. The `finally` is `ScriptSleep(20000)` on every path, including no module and a caught exception (15432–15448). `RunAsync` makes that an `AddDelay`. No address is rewritten. No event is posted.

`llGetNextEmail` returns when the module is missing or the next mail is null. Otherwise it posts `email` to every script in the prim, parameters time, sender, subject, message, numLeft, empty detect (15461–15477), and offers the same to other engines. There is no sleep. The syscall delay stays 0.

The three-argument `llTargetedEmail` sleeps 20000 ms in its own `finally`, including the early returns (21608–21610). Target 2 is the object owner, and a group-owned object returns before the send (21585–21588). Target 1 is the root creator only when this item's creator is that creator (21590–21594). Any other target returns. Subject plus message longer than 4096 returns before the send (21580). A missing account, an account with no email, or a missing module sends nothing. The sleep still runs. On the service lane that sleep is one `AddDelay`.

The four-argument form calls `llEmail` when the type is 2, and otherwise `Stub`, which logs and does not shout (238–239, 21649–21663). It then `ScriptSleep(20000)` on every path. On the service lane `llEmail`'s `finally` has already added 20000, so type 2 records 40000 ms. Types other than 2 record 20000 ms and send no mail. That double add is proposed TB173. Without a service lane the second `ScriptSleep` replaces `NextWakeup`, and the wake is 20000 ms for type 2 as well. The oracle uses the lane.

YEngine `llEmail` returns with no sleep when the module is missing (4354–4357). When `m_restrictEmail` is set and the address is not local, the address becomes the owner's account email, or the send is skipped (4364–4373). The field `m_sleepMsOnEmail` is initialised to 30000 (154) and the constructor then sets it to `EMAIL_PAUSE_TIME * 1000` (618). `EMAIL_PAUSE_TIME` defaults to 20 (138), so a constructed API sleeps 20000 ms unless `[SMTP] email_pause_time` changes it. The sleep is after the request is registered (4382). `llGetNextEmail` shouts and returns when the module is missing (4387–4390). The three-argument form returns with no sleep when the parent is gone, the module is missing, or the text is over 4096 characters (4478–4491), and otherwise sleeps `m_sleepMsOnEmail` (4526). Recorded.

Oracle: `llEmail` with a module and without (20000 ms either way, no event). `llGetNextEmail` with mail waiting (the five parameters, every script in the prim, delay 0) and with none (no event, delay 0). The three-argument form for target 2, target 1 with a matching creator, target 1 with a different creator, a group-owned target 2, a subject and message over 4096 characters, and a missing module. Each wake is 20000 ms. The four-argument form for type 2 (40000 ms, one send) and for types 0 and 1 (20000 ms, no send, no shout). YEngine's missing-module return and its address rewrite are recorded and are not a pass failure.

## 4. Link messages

### P4.F3.7 `llMessageLinked` and `osMessageObject`

Depends on P4.1c, P4.2, P4.5, P4.9, P4.16, P4.19, P4.19e. The 50 ms is the P4.19o recording, not a second copy of P4.19e's delivery recording. Implementing the 50 ms is P4.23.

Functions: `llMessageLinked`, `osMessageObject`.

Source: `LSLSystemAPI.cs` 3076–3127, 7050–7079, 13047–13058, and `LinkMessageBackPressure` 305–323. `PhloxEngine.PostObjectEvent` 1968–2043. YEngine: `LSL_Api.cs` 5796–5838.

`llMessageLinked` builds `link_message` with the sender's `LinkNum`, the integer, `str` or `""`, and `id` or `NULL_KEY` (3081–3082). Detect is empty. `GetLinkParts` (7050–7079): `LINK_SET` (-1) is every prim, `LINK_ALL_OTHERS` (-2) skips the sender's prim, `LINK_ALL_CHILDREN` (-3) skips the root, `LINK_THIS` (-4) is the sender's prim, `0` and `LINK_ROOT` (1) are the root, a number from 2 through `PrimCount` is that prim, and anything else is nothing. Sitters are not targets (7082–7088). Each target is `PostObjectEvent`. That call returns before the prim's lane delivers (1979–2016). Inside one prim the lane keeps call order, and the scripts walked are the cloned task-inventory dictionary (2022–2043). That walk is not sorted by name. The sender's own script hears the message. There is no self-filter. Other engines are offered each script item (3093–3126). Then one back-pressure sleep, not one per receiver: 50 ms when `LinkMessageThrottle` is on and any counted queue is at or below 20 percent free, which is 52 or more of 64 (305–323). A script this engine does not run counts as free. The shim is inline, so the 50 ms is a `NextWakeup` write.

YEngine posts per script item. An unlinked prim (`PrimCount == 1`) reports link number 0 (5815–5817, 5836). Phlox sends `m_host.LinkNum`. YEngine adds no 50 ms. Recorded.

`osMessageObject` is threat `Low`. The shipped `Allow_` line is the estate form at `osslDefaultEnable.ini` 122. A bad key shouts `osMessageObject() cannot send messages to objects with invalid UUIDs` and returns (13050). A missing object shouts the not-found line and returns (13052). `ShoutError` does not sleep. A found prim gets `dataserver` with the sender prim's key and the message, empty detect, on that prim only (13053–13057). It is not a link message.

Oracle: `LINK_THIS`, `LINK_SET`, `LINK_ALL_OTHERS`, `LINK_ALL_CHILDREN`, `0`, link 2, and a number past the last prim. A null string and a null key become `""` and `NULL_KEY`. The sender hears its own message. Two messages to one prim stay in call order. The script walk inside the prim is the recording, not a sort by name. A queue at 52 of 64 sleeps 50 ms once. A short queue does not. `osMessageObject` to a prim in the scene (dataserver, both scripts in that prim), to a bad key, and to a missing key. One allowed owner and one refused owner. The refused case is `osMessageObject permission denied` and the stopped script. YEngine's unlinked link number 0 is recorded and is not a pass failure. P4.19e remains the recording of which scripts receive a targeted message.

## 5. HTTP

### P4.F3.8 `llHTTPRequest`

Depends on P4.1c, P4.2, P4.19, P4.19o, and on P4.32. The 80 ms and the pressure sleep are that recording. Implementing them is P4.23. The worker cap of 6 on P4.21 is not this burst. Until P4.32 is done, a missing module is the case this row can prove. A completed request becomes `http_response` in the plugin (`AsyncCommand/Plugins/HttpRequest.cs` 88–110).

Functions: `llHTTPRequest`.

Source: `LSLSystemAPI.cs` 15049–15235 and 15237–15327. `HttpRequestModule.cs` 72–77, 117–130, 370–429. `HttpRequest.cs` 168–194. YEngine's throttle refusal is `LSL_Api.cs` 15110–15111, as the stage-1 table records it.

The shim is inline. With `HttpInFlightThrottle` on, `HttpQueuePressureSleep` runs first (15225–15235): when free queue space is at or below 40 percent, the delay is `(1 - free / 0.4) * 50` milliseconds, and a positive value is `ScriptSleep`. `CheckThrottle` then uses the module buckets: prim burst 3, owner burst 5, prim 1 per second, owner 25 per second (`HttpRequestModule.cs` 74–77, 127–130). A refusal shouts, when `HTTP_VERBOSE_THROTTLE` is true or omitted (15237–15250), `Script error: llHTTPRequest: request throttled: too many HTTP requests from this object or owner.` `ShoutError` does not sleep. The following `ScriptSleep(80)` replaces `NextWakeup` (15058–15065). The return is `NULL_KEY`. With the in-flight throttle off there is no pressure sleep and no 80 ms.

`OutboundAllowed` (15193–15201) checks only an absolute `http` or `https` URL. A refused URL is `YEngineError`, 1000 ms, and `""`. No request is started. An odd-length option list, a flag that is not an integer, and `HTTP_CUSTOM_HEADER` (5) without its third value are `ScriptShoutError` and `NULL_KEY` (15084–15103). The wake is the shout's 15 ms. Setting `Content-Type` through a custom header is `ShoutError` and `NULL_KEY`, with no 15 ms (15114–15118). A name that starts with `proxy-` or `sec-`, or a forbidden header marked fatal, is `YEngineError` and `""` (15129–15134). A forbidden header that is not fatal is dropped. More than 8 custom headers are ignored after a `YEngineError` that does not return (15124–15127). A name plus value longer than 253 characters is `YEngineError` with the source spelling `exceds`, and `""` (15137–15140). A bad `HTTP_MIMETYPE` is `YEngineError`, 1000 ms, and `""` (15154–15158). Headers the simulator sets are written last and win (15165–15166, 15297–15326): shard, object name, object key, region, local position, velocity, rotation, owner name, owner key. A script header whose name starts with `x-secondlife` is removed before those are written (15305–15306).

`HTTP_BODY_MAXLENGTH` is floored at 64 and capped at `HttpBodyMaxLenMAX`, default 16384 (`HttpRequestModule.cs` 72, 117, 386–393). `HTTP_METHOD`, `HTTP_VERIFY_CERT` and `HTTP_PRAGMA_NO_CACHE` are passed through. The in-flight caps, while the throttle is on, are 10 per object and 200 per region (`HttpRequest.cs` 172–189). A request past either cap does not start. The return is `NULL_KEY` and the 80 ms replaces the pressure sleep (15176–15179). No shout. A missing HTTP module returns `NULL_KEY` with no sleep (15054).

The response event carries the request id, the status, an empty metadata list, and the body (`HttpRequest.cs` 94–100). The comment says `HTTP_BODY_TRUNCATED` is not implemented. Question 8. The event goes to every script in the prim. A late response whose script was reset or removed is dropped (81–86) and `PostObjectEvent` also drops an `http_response` the plugin no longer owns (`PhloxEngine.cs` 1973–1977). Prim events share the per-prim lane, so one prim's responses stay in completion order. The inventory walk is not sorted by name.

YEngine's refused `CheckThrottle` returns `NULL_KEY` with no sleep (`LSL_Api.cs` 15110–15111). Recorded.

Oracle: a normal request (the returned key, the simulator headers, a later `http_response` with an empty metadata list). A queue past 60 percent full (the pressure delay, and the request still starts). A burst refusal (the shout, `NULL_KEY`, wake 80 ms). The same refusal with `HTTP_VERBOSE_THROTTLE` 0 (no shout, 80 ms). The in-flight throttle off (no 80 ms). An 11th request for one object and a 201st for the region (`NULL_KEY`, 80 ms, no shout). A non-http URL that the filter refuses (`""`, 1000 ms). An odd option list (the shout, `NULL_KEY`, 15 ms). `Content-Type` as a custom header (`NULL_KEY`, no 15 ms). A ninth custom header (the request continues). A name and value over 253 characters (`""`, the `exceds` text). A body-length option of 10 (the module's 64) and of a value above 16384 (the cap). A reset before the response (no event). Two responses for one prim stay in order. YEngine's no-sleep refusal is recorded and is not a pass failure.

### P4.F3.9 URL request and release

Depends on P4.1c, P4.16, P4.19, and on P4.33. The calls themselves are inline (`SyscallShim.cs` 4405–4408 and 8282–8285). The URL module posts `http_request` from its own thread. That post is not the service lane, so this row does not depend on P4.21. Until P4.33 is done, this row proves the no-module returns.

Functions: `llRequestURL`, `llRequestSecureURL`, `llReleaseURL`, `llGetFreeURLs`, `osRequestURL`, `osRequestSecureURL`.

Source: `LSLSystemAPI.cs` 13078–13105 and 15393–15422. `UrlModule.cs` 102, 129–168, 223–241, 286–310, 453–456.

No module: the `ll` requests return `NULL_KEY`, `llGetFreeURLs` returns 0, and `llReleaseURL` returns. `RequestURL` builds `http://` plus `ExternalHostNameForLSL` plus the listener port plus `/lslhttp/` plus the code (241). `RequestSecureURL` uses `https` and `/lslhttps/` (310). When `ExternalHostNameForLSL` is unset or does not resolve, the listener stays disabled (129–155) and the request still returns a key, with `http_request` parameters that key, `URL_REQUEST_DENIED`, and the module's error string (227–230). A full table posts `URL_REQUEST_DENIED` and `Too many URLs already open` (235–239). The cap is `TotalUrls`, from `[LL-Functions] max_external_urls_per_simulator`. The code default is 15000 (`UrlModule.cs` 102, 166–168). The shipped file sets 100 (`RegionServerDefaults.ini` 2168). `GetFreeUrls` is `TotalUrls` minus the map count (453–456). The integer is the recording. This plan does not copy either number into the test as an expected value.

`osRequestURL` and `osRequestSecureURL` are threat `Moderate`. The shipped lines are the estate form (148–149). The only option string copied is `allowXss` (13078–13083). Any other option is ignored. A missing module returns `NULL_KEY` after the gate has passed.

Oracle: a request with the listener enabled (the key, then `http_request` with the URL text). A request with the host name unset (`URL_REQUEST_DENIED` and the error string). A table already at the cap. `llGetFreeURLs` before and after one request, and after `llReleaseURL`. `osRequestURL` with `allowXss` and with another string. One allowed owner and one refused owner. The refused case is `osRequestURL permission denied` or `osRequestSecureURL permission denied`, and the stopped script. `llGetFreeURLs` with no module is 0.

### P4.F3.10 HTTP response and content type

Depends on P4.1c, P4.16, P4.19, and on P4.33 for a live request id. The body needs a URL the P4.F3.9 request created.

Functions: `llHTTPResponse`, `llGetHTTPHeader`, `llSetContentType`, `osSetContentType`.

Source: `LSLSystemAPI.cs` 12681–12686 and 15337–15391. `SlConst.cs` 48–55. `CONTENT_TYPE_TEXT` is 0 (`LSL_Constants.cs` 417).

`llHTTPResponse` returns when the URL module is missing or the id is not a key (15337–15344). It does not sleep. `llGetHTTPHeader` returns `""` for a missing module, a bad id, or a header the request does not have (15346–15351). `llSetContentType` returns on a missing module or a zero id. The map is XML `application/xml`, XHTML `application/xhtml+xml`, ATOM `application/atom+xml`, JSON `application/json`, LLSD `application/llsd+xml`, FORM `application/x-www-form-urlencoded`, RSS `application/rss+xml`, and every other integer including `CONTENT_TYPE_TEXT` and HTML starts as `text/plain` (15360–15371). HTML becomes `text/html` only when the owner is a present root agent, the user-agent contains `SecondLife`, and `x-remote-ip` equals the owner's circuit address (15372–15390).

`osSetContentType` is threat `Severe` and the shipped `Allow_` line is false (230). That parseable false is `osSetContentType disabled in region configuration`. The threat test is not reached. When the line allows it, a parsed id is stored as the given string (12681–12685). A bad id returns after the gate. There is no enum.

Oracle: `llHTTPResponse` for a live request id and for a bad id. `llGetHTTPHeader` for a header the request sent and for one it did not. `llSetContentType` for each named constant, for 0, and for HTML with the owner check failing and passing. `osSetContentType` on the shipped file (the disabled-in-configuration text and the stopped script) and with the line set true (the mime string stored as given).

## 6. Notecards and dataserver

A notecard read that posts `dataserver` uses `PostDataserverToPrim` (`PhloxEngine.cs` 2137–2182). Scripts in the asking prim are ordered by item name, then item id. The asker is skipped when the pending key was already removed, which is a reset, a state change, or an unload after the ask (19538–19545). Other engines are offered the key and the data as strings. The return count is how many Phlox scripts were posted. An answer meant for another script is dropped when that script's queue is at 32 (`OTHER_ANSWER_LIMIT`, stage-1 P4.2). This file does not re-specify that cap.

`NotecardAssetIn` (`LSLSystemAPI.cs` 5777–5785) takes an inventory name or a raw asset UUID. The `os` readers take an inventory item id or a name (`OsslNotecardLines`, 13960–13961). A card with embedded items yields no lines, so a line read answers EOF (`HasEmbeddedItems`, 19558–19566). A line answer is `""` for a negative line, `\n\n\n` past the end, and otherwise the line cut to `NotecardLineCap` bytes (502–507). The cap is `[InWorldz.Phlox] NotecardLineReadCharsMax`, else the same `[YEngine]` key, else 1024, and never above 65535 (464–484). A segment answer is a character slice, `""` for a negative line or a non-positive length, and `\n\n\n` past the end (513–524). It is not the byte cap.

The shipped pause is `NotecardPause` `sl`: 100 ms, through `NotecardSleep`, which also requires `NotecardThrottle` (341–386). `halcyon` is 25 ms for a cached count and 50 ms for a fetched count, and for a line 1 ms on every 16th cached line from offset 0, else 25 ms fetched. A missing count sleeps 100 ms. These async readers are inline, so a `ScriptShoutError` writes 15 ms and a following `NotecardSleep` replaces `NextWakeup`. The wake is the later write. The two numbers are not added.

A cache hit posts the event on the calling thread before the function returns, then the pause runs. A miss is queued on the thread pool (415–440). The function returns the query key without waiting for that fetch. A fetch that finds no notecard asset shouts `Notecard 'name' could not be found.` and posts no event. The pool cannot `ScriptSleep`, so that failure adds no pause (400–404).

### P4.F3.11 Async notecard reads

Depends on P4.1c, P4.2, P4.19, P4.19o. The asset bytes are the asset service from Phases 1 and 2. Implementing the pauses is P4.23. The `iw` names also use the same table dependency P4.S27 uses.

Functions: `llGetNotecardLine`, `llGetNumberOfNotecardLines`, `iwGetNotecardSegment`, `iwGetLinkNumberOfNotecardLines`, `iwGetLinkNotecardLine`, `iwGetLinkNotecardSegment`.

Source: `LSLSystemAPI.cs` 395–442, 498–524, 5792–5842, 6324–6398.

`llGetNumberOfNotecardLines` on a missing card shouts through `ScriptShoutError` and then `NotecardSleep(100)` (5796–5801). The wake is 100 ms. The return is `NULL_KEY` and there is no event. A found card returns the query key. The event data is the line count. The pause is `NotecardCountSleep` (365–370).

`llGetNotecardLine` on a missing card is `ScriptShoutError` and `NULL_KEY`, with no extra pause (5818–5822). The wake is 15 ms. A found card uses `NotecardLineSleep` (377–386).

`iwGetNotecardSegment` on an empty name returns `NULL_KEY` with no shout (5833). A missing card is the same shout as `llGetNotecardLine` and no extra pause (5835). A cache hit uses `NotecardLineCachedSleep` (389–393): 1 ms when the line is a multiple of 16 and the offset is 0, otherwise no sleep. A miss uses `NotecardSleep(25)` (5840–5841). Neither path reads `NotecardHalcyonTiming`. The shipped `sl` preset still gets that timing. The comments say the delays match `llGetNotecardLine`. They do not. That is proposed TB175.

The three link readers search every prim of the selector, then accept the name as an asset UUID (`LinkNotecardAssets`, 6387–6398). An empty name returns `NULL_KEY` with no shout. A count that finds nothing shouts and, when the selector is exactly one prim, sleeps 100 ms, which replaces the shout's 15 ms (6336–6339). A selector of no prim or of two or more keeps the 15 ms. A line or segment that finds nothing uses `ShoutError` and then `ChatSleep` when the selector has any prim (6355–6358, 6377–6378). No prim means the shout and no sleep. The link line uses `NotecardLineSleep`. The link segment uses the same cached-1 ms or fetched-25 ms path as `iwGetNotecardSegment` (6372–6373). Question 4.

Oracle: a cached line 0, a negative line (`""`), a line past the end (`\n\n\n`), a line longer than the cap, and a card with an embedded item (EOF). A missing name on each `ll` call (the shout, `NULL_KEY`, no event, 15 ms for a line and 100 ms for a count). A cache miss whose asset is not a notecard (the shout from the pool, no event, no added pause). One run with `NotecardPause = halcyon` for the two `ll` calls. `iwGetNotecardSegment` of a cached line 0 at offset 0 (1 ms), of line 1 (no sleep), and of a miss (25 ms), with `NotecardPause` left at `sl`. The same three on `iwGetLinkNotecardSegment`. A link count whose selector is one prim and whose selector is the whole linkset. A link line with no prim selected. Dataserver order is name then item id. A second script in the prim hears the answer. The asker does not hear an answer posted after reset.

### P4.F3.12 Synchronous and `os` notecard reads

Depends on P4.1c, P4.16, P4.19, P4.21, P4.19n, and on the engine in [D131](../DECISIONS.md) (P4.34 through P4.34f) for the notecard patterns. A cache miss on these reads is the service lane (`NeedsService` 157–170). A missing name or a cached asset stays inline. The asset bytes are Phases 1 and 2.

Functions: `llGetNotecardLineSync`, `llFindNotecardTextSync` (integer), `llFindNotecardTextSync` (list), `llFindNotecardTextCount`, `osGetNotecardLine`, `osGetNotecard`, `osGetNumberOfNotecardLines`.

Source: `LSLSystemAPI.cs` 5844–5878, 10333, 10369–10456, 13960–13997. Shims: `SyscallShim.cs` 6455–6468, 7471–7486, 8921–8935.

`llGetNotecardLineSync` looks up the name only (`FindInventoryItem`, 19519–19526). A missing name, a missing asset, a negative line, or a line past the end returns `\n\n\n`. A missing name shouts `Notecard 'name' not found.` through `ShoutError`, so there is no 15 ms (5849). The line is not cut to 1024. A trailing CR is stripped (5856). The failure value on the `Defer` is `\n\n\n` (6459).

The integer `llFindNotecardTextSync` is the same name lookup. A missing card shouts the same not-found line and returns -1 (5865). The search is `IndexOf` from `max(0, line)`, ordinal when `caseSensitive` is not 0, otherwise ordinal-ignore-case (5871–5876). The first matching line number is returned.

The list form reads `NotecardAssetIn`, so a raw asset UUID works (10403). A missing card shouts `Notecard 'name' could not be found.` and returns a one-element list of `NotecardNak`, which is newline, U+0015, newline (10333, 10404–10407). A bad pattern shouts `llFindNotecardTextSync: invalid regular expression: ` plus the runtime message and returns an empty list (10383–10388, 10409–10410). `count` at or below 0 means 64, and a larger count is capped at 64 (10420). The list is line, column, length, repeated, after skipping `start` matches (10369–10380). A card with embedded items yields no matches. There is no sleep.

`llFindNotecardTextCount` is inline, not `Defer`. A bad pattern or a missing card shouts and returns `NULL_KEY` (10438–10444). There is no pause. A found card returns the query key and the event data is the match count (10446–10456). A regex timeout shouts and the event data is `0`.

`osGetNotecardLine`, `osGetNotecard` and `osGetNumberOfNotecardLines` are threat `VeryHigh`. The shipped lines are the estate form (213–215). A missing card shouts and returns `ERROR!`, `ERROR!`, or -1 (13975, 13984, 13995). There is no 1024 cut and no sleep. `osGetNotecard` joins every line with a trailing newline (13985–13987). An out-of-range `osGetNotecardLine` is `\n\n\n` (13976). The `Defer` failure values are `ERROR!`, `ERROR!` and -1 (8925, 8930, 8935).

Oracle: `llGetNotecardLineSync` of a short line, of a line longer than 1024 bytes (the full line), of a missing name (the shout, `\n\n\n`, script running, no 15 ms), and of a UUID name (missing, because this form does not take an asset id). The integer find from line 0, from a later line, case-insensitive, and of a missing card. The list find of two matches, of `count` 0 (64 is the cap), of a bad pattern (empty list), and of a missing card (the NAK element). `llFindNotecardTextCount` of a card with two matches and of a missing card (`NULL_KEY`, no event). Each `os` read of a present card and of a missing card, once with an owner the shipped line accepts and once with an owner it refuses. The refused case is `{function} permission denied` and the stopped script. A cache miss runs on the service lane and returns the same text.

## 7. Proposed ledger rows

The ledger's highest id on this branch is TB164. TB167 through TB172 belong to the pure-function rows and are not reused here. These three are the next ids this file proposes, starting at TB173. They are not filed here. The maintainer adds the ledger rows. Ferroan reproduces Phlox on each of them until a decision says otherwise.

| # | C# | What goes wrong | Ferroan does |
| --- | --- | --- | --- |
| TB173 | `LSLSystemAPI.cs` `llTargetedEmail` four-argument form (21649–21664) calling `llEmail` (15432–15448), at `c401bceecd` | Type 2 calls `llEmail`, whose `finally` adds 20000 ms, and then adds 20000 ms again. On the service lane `NeedsService` defaults to true for this name (`LSLSystemAPI.cs` 172–173, `SyscallShim.cs` 7148–7154), so the delays add. The recorded sleep is 40000 ms. Types other than 2 log a stub and add 20000 ms, and no mail is sent. Read from the calls. Not executed. | Reproduce both adds for type 2 (P4.F3.6). Question 3. |
| TB174 | `LSLSystemAPI.cs` `llOwnerSay` (1594–1605) and `LLClientView.SendChatMessage` (1115–1117), at `c401bceecd` | The packet owner id is `m_host.UUID`, the prim, in the argument that `SendChatMessage` writes as the owner. The source id is the prim as well. The chat does not go through `HostSimChat`, so no script listen hears it. | Reproduce the prim UUID in that field (P4.F3.2). Question 9. |
| TB175 | `LSLSystemAPI.cs` `iwGetNotecardSegment` (5830–5842) and `iwGetLinkNotecardSegment` (6361–6374), at `c401bceecd` | Both ignore `NotecardHalcyonTiming`. A cache hit sleeps 1 ms only when the line is a multiple of 16 and the offset is 0. A miss sleeps 25 ms. The shipped `NotecardPause` is `sl`, and `llGetNotecardLine` on that preset sleeps 100 ms (`NotecardLineSleep`, 377–386). The comments say the segment delays match `llGetNotecardLine`. | Reproduce the 1 ms and 25 ms path (P4.F3.11). Question 4. |

`llTextBox`'s shout names `llDialog` because the body copies Halcyon (`LSLSystemAPI.cs` 1781–1782). That wording is question 5. It is not a ledger row. The agent `llMapBeacon` sends nothing. That is question 2. It is not a ledger row. Empty `http_response` metadata is question 8. It is not a ledger row. TB159 stays the YEngine shout row. This file does not refile it.

## 8. Open questions

The earlier questions in [PHASE4-SCRIPTING.md](PHASE4-SCRIPTING.md) section 6 stay as they are. These are new.

1. Shout owner, debug-channel reach, and worn-attachment parcel hide. P4.F3.1 and P4.F3.2 reproduce Phlox: `fromAgent` false, owner-only debug chat when the sender is a part, and parcel hide on a worn shout. YEngine's other behaviour is [TB159](../TRANQUILLITY_BUGS.md). Channel-0 shout reach stays 100 m either way.
2. `llMapBeacon(agent, text, color)`. P4.F3.4 reproduces the body: a log line, no packet, no sleep (21478–21495). The list overload sleeps 1 s and sends nothing on purpose (10548–10550). The other choice is to send a beacon or an IM.
3. Four-argument `llTargetedEmail` type 2. P4.F3.6 reproduces the service-lane recording of 40000 ms (TB173). The other choice is one 20 s sleep. Types 0 and 1 stay a stub: no mail, 20 s, no shout.
4. `iw` notecard segment timing. P4.F3.11 reproduces the 1 ms and 25 ms path while `NotecardPause` is `sl` (TB175). The other choice is `NotecardLineSleep`.
5. `llTextBox` shouts `First parameter to llDialog needs to be a key` (1782). P4.F3.4 reproduces that text.
6. Compare rule. Section 1 matches integers, strings, keys, error text, channels, handles, link numbers and sleep durations exactly. A script listen at exactly the range is silence. A viewer at that distance still hears the chat. No wider tolerance is proposed.
7. Uncaught throws. Section 1 kills the script on a gate denial and on an exception that leaves a body. An HTTP or notecard error that shouts and returns `NULL_KEY`, `""`, `-1` or NAK leaves the script running. The other choice is to catch a denial and return a value. This plan reproduces the kill.
8. `http_response` metadata. P4.F3.8 reproduces the empty list (`HttpRequest.cs` 98). `HTTP_BODY_TRUNCATED` is not implemented.
9. `llOwnerSay` owner id. P4.F3.2 reproduces the prim UUID in the packet owner field (TB174). The other choice is the avatar id. A group-owned prim sends no packet and still sleeps 15 ms. YEngine returns immediately in that case. That difference is recorded with the row.
10. Regex engine. Decided by the maintainer on 26 November 2026. [D131](../DECISIONS.md). This is the same decision as the pure-function rows, for listen regex and notecard pattern. Ferroan ports the needed part of .NET's `System.Text.RegularExpressions`. `osListenRegex` (`LSLSystemAPI.cs` 14061–14062) shouts the fixed sentences `Name regex is invalid.` and `Message regex is invalid.` Those sentences do not include the engine's message. `NotecardPattern` (10383–10388), which `llFindNotecardTextSync` and `llFindNotecardTextCount` use, shouts `invalid regular expression: ` plus the engine's message. A match that times out shouts `regex timed out`. The timeout is 250 ms and it is the engine's clock (`ScriptRegex.cs` 21–26). The rows are P4.34 through P4.34f. Each derived file keeps the original copyright line and the MIT notice.

Linkset data, XML-RPC, experience permissions, experience key-value, notecard writes, `osRegionNotice`, `osNpcSay` and `bot*` stay out of this file. Channel-0 object say of the default script stays P4.1d.
