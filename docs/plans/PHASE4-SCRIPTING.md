# Phase 4 plan: scripting, stage 1

Planning note for the manager. This file is the map. It does not add rows to [TASKS.md](../../TASKS.md) or [WORK_QUEUE.md](../../WORK_QUEUE.md). Stage 2 plans each function family. Stage 1 is the engine and the infrastructure around it.

**Gate** ([PLAN.md](../PLAN.md)): the SLua engine plan's conformance suite passes on a Rust region. The estimate there is 7 to 13 person-months. Phases 3 and 4 run side by side after Phase 2.

**Scope here:** one script engine, the scheduler, events, time slices, memory limits, state saving, and reading and writing portable script state. The 900 `ll`, `os` and `bot` methods at the pin are inventoried and left as stage-2 placeholders. [PLAN.md](../PLAN.md) calls that surface 831 functions. Section 2 says what the pin actually declares.

**Standing decisions**

- [D1](../DECISIONS.md). The SLua engine plan is done in today's C# server first. Phase 4 reuses that work's C interface and conformance suite. At Pin8 that C interface is not in the tree. Section 1 says what is there, and what [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) still has to write.
- [D6](../DECISIONS.md) and [ARCHITECTURE.md](../../ARCHITECTURE.md). tokio for networking. The region frame, Jolt and the SLua engine each run on their own thread. Linden's engine requires each VM environment to be used by one thread at a time.
- [D8](../DECISIONS.md) and [AGENTS.md](../../AGENTS.md). The only native libraries are Jolt and Linden Lab's SLua engine. `ferroan-scripting` is the only crate that calls the engine. The workspace keeps `unsafe_code = "forbid"` everywhere else. Every `unsafe` block in that crate needs a `SAFETY` comment covering ownership, lifetime, threading and bounds.
- [D10](../DECISIONS.md). One script engine. Linden's SLua runs LSL, SLua, OSSL (an `ossl` table, because `os` is Luau's stdlib) and bot functions (a `bot` table). Phlox's VM and YEngine are not ported. YEngine's language extensions are unsupported.
- [D13](../DECISIONS.md). Ferroan code is Rust. A harness that must run Tranquillity is a later edit of that decision. This plan does not invent the id. The oracle row names the shape and leaves the id to that edit.
- [PLAN.md](../PLAN.md) building blocks name native Linden Lab SLua, one engine for LSL, SLua, OSSL and bot. The saved-state row there is the portable format in [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md), written by the C# server before cut-over. A script whose state does not convert stays on a C# region. YEngine and its extensions are dropped, and YEngine and XEngine state from hypergrid visitors is still imported.

The rows below are about a day each, in the order section 5 lists. An id is a label. A row that overruns a day is split before the next one starts, as Phase 2 did. P4.1 is already split that way: the crate, the native bind, the default script, and the chat packet are four rows. A native bind and object chat do not fit in one day.

The physics plan uses the words "Phase 4" for two later pieces. P3.56, and question 9 in that plan, mean a crossing that succeeds, with the avatar crossings [D38](../DECISIONS.md) leaves for later. That work is not this plan. P3.18a, P3.26 and P3.46 mean collision events delivered to scripts, and the script calls that apply forces. Those calls are this plan's stage 2. This file does not schedule crossings.

Line numbers below are the file as it is at Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. Paths are under `Source/` in the Tranquillity checkout unless they start with `Tests/` or `Docs/`. Linden's engine is `secondlife/slua` at `a02d459aa0cb64a4c2b3dd4d841e67ba4a01d9b1`, the revision [REFERENCES.md](../REFERENCES.md) pins. Paths for that tree start at its root (`Executor/`, `VM/`).

## 1. What Tranquillity does today

Tranquillity runs two engines in one region. The shipped ini selects Phlox. The code default, when the key is absent, is YEngine. They never both run one script. Neither is Linden's VM.

### Which engine runs a script

`Scene.DefaultScriptEngine` reads `[Startup] DefaultScriptEngine` and falls back to `"YEngine"` (`Scene.cs` 1076). The shipped file sets `DefaultScriptEngine = "InWorldz.Phlox"` (`RegionServerDefaults.ini` 319–325) and `ScriptEngineChoice = true` (334–336). `[YEngine] Enabled = true` (2409–2418). `[InWorldz.Phlox] Enabled` defaults to false in code (`PhloxEngine.cs` 252–258) and the shipped ini sets it true (2495–2505). Both engines are loaded. The ini default is the one a shipped region uses.

`Scene.ResolveScriptEngine` (`Scene.Inventory.cs` 108–202) places a script in this order:

1. A viewer compile target stored on the item, when some loaded engine's `AcceptsCompileTarget` returns true and no other engine holds saved state for that item and asset (111–128). The interface default is false (`IScriptEngineSelection.cs` 41). Phlox does not override it. YEngine does not implement the interface (`XMREngine.cs` 61). A viewer target of `luau` or `lsl-luau` therefore selects no engine at this pin.
2. An engine name saved on the item. `ScriptEngineChoices` uses 1 for YEngine and 2 for `InWorldz.Phlox` (the alias `Phlox` counts as the same engine).
3. A first line `//Name:` with a newline past index 5 and a colon at index 3 or later (146–164). `//YEngine:` selects YEngine when that engine is loaded. It is not a Phlox rule. Phlox also accepts the alias `//Phlox:` (`PhloxEngine.cs` 134–137, `PhloxEngineHeader.cs` 69–75). `ClaimsScript` on Phlox returns false (`PhloxEngine.cs` 139), so a body with no header is not claimed by content.
4. Saved state, the default engine first, then any other engine that has a row for the item and asset (177–188).
5. `ClaimsScript` (191–199).
6. Otherwise the default engine (201).

`EventManager.TriggerRezScript` tells every handler and skips an engine that is not an `IScriptEngineSelection` when the item carries a choice and the engine name is not the chosen one (`EventManager.cs` 1812–1849). Phlox's own `OnRezScript` (`PhloxEngine.cs` 789–830) runs the script when the region's chosen engine is Phlox, or, when the item carries no compile or engine choice, when the header resolves to Phlox. Otherwise it disowns the item.

The text a new LSL script gets is `Constants.DefaultScriptText` (`Constants.cs` 74): `llSay(0, "Script running")` inside `state_entry`. The same string is the asset `ScriptsAssetSet/DefaultScript.lsl` line 5, and the Lua default `DefaultLuaScript.Text` (`DefaultLuaScript.cs` 22–26) under `--!slua` as `ll.Say(0, "Script running")`. `Docs/PhloxSLua.md` shows `ll.Say(0, "Hello from SLua")` as an illustration. That string is not the shipped default. The first slice matches `"Script running"`.

### Compile and load

| Piece | File | What it does |
| --- | --- | --- |
| Phlox module | `Phlox.ScriptEngine/PhloxEngine.cs` | Region module. Name `InWorldz.Phlox` (`PhloxEngineHeader.cs` 69). |
| Header | `Phlox.ScriptEngine/PhloxEngineHeader.cs` 53–95 | `//Name:language`. Language is the text after the colon, trimmed and lowercased, when at least two characters follow. |
| Loader | `Phlox.ScriptEngine/PhloxScriptLoader.cs` | One background thread `"Phlox compile"`, stack 16 MiB (126–137, 195–197, 701). Cache directory `ScriptEngines/Phlox/bytecode`, extension `.plx`, schema version 7 (63–92). A lower stamp deletes the cached files. Unloaded in-memory cache holds 128. Load order: shared in-memory, unloaded cache, disk, else compile. |
| LSL compile | `InWorldz.Phlox/Glue/CompilerFrontend.cs` 78–84, 162–328 | Sanitize, lex, parse, definition, types and analysis visitors, then assembly text. Errors return null. A null compile does not call `BeginScriptRun` (`PhloxScriptLoader.cs` 797–804, 904, 924). |
| Assemble | `InWorldz.Phlox/Glue/CompilerFrontend.cs` 338–368 | Assembly text to bytecode. |
| SLua subset | `InWorldz.Phlox/SLua/SLuaCompiler.cs` 55–91 | `IsLuaScript` is true for a leading `--!slua`, `--!lua`, or any `--`. `CompileToAssembly` emits Phlox assembly text. `CompileLua` (`CompilerFrontend.cs` 377–384) assembles that text. The file's header mentions `SLUA_SURFACE.md`. That file is not in the tree. |
| VM | `InWorldz.Phlox/VM/Interpreter.cs` | One opcode per `Tick` (209–233). |
| YEngine module | `YEngine/XMREngine.cs` 59–61, `PluginRegistration.cs` 33–49 | Region module id `YEngine`. Script class name `YEngineScript` (147–153). |
| YEngine compile | `YEngine/XMRInstCtor.cs` 65–286, `MMRScriptCompile.cs`, `MMRScriptCodeGen.cs`, `MMRScriptObjCode.cs` | Source key is a SHA-256 of the text. Empty source, or source that is only leading `//` comments, uses the asset id. Reuses a `.yobj` unless forced. Magic `YObjectCode`, version −11. The result is CIL (`DynamicMethod`), not an interpreter. |
| Shared API types | `ScriptEngine.Shared/Api/ScriptBase/` | `ScriptBaseClass`, `LSL_Constants`, `LSL_Stub`, `OSSL_Stub`, `LS_Stub`, `MOD_Stub`. YEngine compiles against these. Phlox does not. |
| YEngine LSL body | `ScriptEngine.Shared/Api/LSL_Api.cs` | About 22,566 lines. The compiled script calls `xmr*` helpers for memory, not the constant stubs near the bottom of this file. |
| YEngine OSSL body | `ScriptEngine.Shared/Api/OSSL_Api.cs` | Threat checks and the `os*` bodies YEngine runs, including `osNpc*`. |
| Phlox host | `Phlox.ScriptEngine/LSLSystemAPI.cs` | About 22,553 lines. Implements `ISystemAPI`. This is the host [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) says stays. |
| Signatures | `InWorldz.Phlox/Glue/ISystemAPI.cs` 18–1114 | The pin's script surface. Section 2 counts it. |
| Bot module | `OptionalModules/World/NPC/BotManager.cs` | Phlox's bot and NPC module. It wraps `INPCModule` (`NPCModule.cs`). YEngine has no `bot*` names. Its NPC calls are `osNpc*` on `OSSL_Api`. |

A fresh Phlox script is set to `Waiting`, then `state_entry` when the item's Running flag is on. A restored state is not replaced with a new `state_entry`. After that, the rez may post `on_rez`, then `attach` for an attached rez, then `changed` from the state source. YEngine does the same shape: no `.state` file means state code 0, global init, then `state_entry`; a snapshot resumes through `MigrateInEventHandler` (`XMRInstCtor.cs` 1374–1407). If the item is running, the instance is queued to start (`XMREngine.cs` 1522–1540).

YEngine extensions are turned on by `yoption`: arrays, advanced flow control, try/catch, objects, chars, dollar signs (`MMRScriptTokenize.cs` 183–192, 609–648, 1044–1143). `yoptions;` enables only advanced flow control and try/catch. Runtime extras include `xmrEventDequeue`, `xmrEventEnqueue`, `xmrEventSaveDets` and `xmrEventLoadDets`. [D10](../DECISIONS.md) leaves these unsupported. Phlox rejects overloaded user functions. YEngine allows them.

### Threads, and how the region frame reaches scripts

The region heartbeat does not run script opcodes.

Phlox has one scheduler thread per region, `"PhloxMasterScheduler"`, priority from `[InWorldz.Phlox] SchedulerThreadPriority`, default `Lowest` (`PhloxEngine.cs` 69–78, 337–339; `PhloxMasterScheduler.cs` 46–72). The loop calls `PhloxExecutionScheduler.DoWork`, then `PhloxScriptLoader.DoWork`, then waits until the next sleep or timer wake (`Clock.Now`) or a signal (`PhloxMasterScheduler.cs` 105–160). `Clock.Now` is `(ulong)Environment.TickCount64` (`InWorldz.Phlox/Util/Clock.cs` 45–53). `EventManager.OnFrame` is subscribed only when the physics throttle is on, and only to sample physics milliseconds (`PhloxEngine.cs` 224–229, 486).

Beside that thread, Phlox starts:

- `"Phlox compile"`, one per engine, above.
- A service lane of up to `ServiceCallThreads` (default 4), idle exit `ServiceThreadIdleMs` 30000, deadline `ServiceCallTimeoutMs` 35000 (`PhloxExecutionScheduler.cs` 189–211, 248–253). The shipped ini sets `ServiceCallDeferral = auto`.
- Long calls drained by `Task.Run`, cap `MAX_ASYNC_WORKERS = 6` (206–209, 3489–3493). Workers exit when the queue is empty.
- One process-wide `"PhloxAsyncCmdHandlerThread"` that sleeps 100 ms and polls HTTP, XML-RPC and sensor repeats (`AsyncCommand/AsyncCommandManager.cs` 61–62, 154–180, 203–226).
- `"PhloxStateManager"`, the SQLite flush thread (`StateManager.cs` around 229).

A deferred call is matched by `SyscallContext` sequence (`SyscallContext.cs` 65–75). The deadline wins or the worker wins. Both do not complete the same call.

YEngine's scripts are not OS threads. `NumThreadScriptWorkers` defaults to 2 (`XMREngine.cs` 207). Each worker is an OS thread running `RunScriptThread` (`XMRScriptThread.cs` 45–53, 100–206). One more thread, `"Yengine sleep"`, waits the sleep queue (`XMREngine.cs` 317, 1865–1916). `StartEx` and `ResumeEx` run the script on a worker (`XMRScriptUThread.cs` 42–81). `OnFrame` (`XMREngine.cs` 1665–1677, subscribed at 739) runs one-shot thunks only. Workers block on a wake lock. Each pass takes up to six start-queue scripts, then one yield-queue script. Lock order is the run lock, then the queue lock (`XMRInstMain.cs` 128–136).

Events arrive on scene subscriptions, not from the frame tick. Phlox subscribes in `PhloxEngine.cs`: chat from world, client and broadcast (1042–1072), `IWorldComm.OnMessageDelivered` (1100–1106), grab (1115–1156), `changed` (1216–1224), collider start, colliding and end (1229–1247), land collision (1292–1310), attach (1315–1322), moving start and end (1544–1555), at-target and not-at-target including rotation (1560–1585), money (1588–1613), `on_death` (1835–1850), damage (1871–1911), control (1958–1965). `PostObjectEvent` (1968–2043) queues for one prim and posts to every script in it. `PostScriptEvent` (1777–1797) maps the name through `SupportedEventList` and copies detect data. The timer wake is the scheduler, not `EventManager`: `CheckSleepingScripts` posts `timer` (`PhloxExecutionScheduler.cs` 1591–1596). `EventManager.TriggerTimerEvent` throws `NotImplementedException` (`EventManager.cs` 2924–2926). YEngine subscribes in `XMREvents.cs` 48–76.

`SupportedEventList.Events` (`InWorldz.Phlox/Types/SupportedEventList.cs` 22–71, signatures 73–405) is the Phlox event set, including touch, collision, land collision, listen, sensor, timer, link message, money, HTTP, dataserver, path update, damage and linkset data. `game_control` is compile-only (395–403). The enum value is the saved event's index, so appended names stay at the end. YEngine's names are `ScriptEventCode` (`MMRScriptEventCode.cs` 43–92), size 44, with gaps. Startup checks those names against the scene's event list and disables the engine on a mismatch (`XMREngine.cs` 224–240).

### Time slices

Phlox limits a slice by opcode count. `INSTRUCTION_FREQUENCY` is 48 `Tick` calls per `DoWork` pass. `SCRIPT_TIMESLICE` is 8 ticks per script (`PhloxExecutionScheduler.cs` 66–71, `DoTimeslices` 1406–1482). A `Stopwatch` records the slice for a decaying average and warns at `SLOW_THRESH_MS` 250. Those three constants are not config keys. The limit is the tick count. When 8 ticks finish, the cursor moves to the next run-queue node. `CheckRunstateChange` (1486–1512): `Sleeping` goes to the sleep heap, `Waiting` starts the next queued event or leaves the queue, `Syscall` and `Killed` leave the queue. Every timeslice calls `StateManager.ScriptChanged`.

YEngine limits a slice by wall time. `RunOne` stores `m_SliceStart` (`XMRInstRun.cs` 263–264). `CheckRunWork` returns at once when less than 60.0 ms has elapsed and throws `StackHibernateException` at the end of the slice (932–955). The 60 ms is a literal. There is no slice config key. The instance then goes on the tail of the yield queue. Most `ll*` calls are followed by a compiler-inserted `CheckRun` unless the name is in `noCheckRuns` or starts with `xmr` (`MMRScriptInlines.cs` 63–109, 199–204, 695–699).

Linden's executor limits a slice with a quanta clock. `QuantaClock` is monotonic seconds (`Executor/include/Luau/Executor.h` 83–87). `InterruptInstaller` (111–168) installs a safepoint interrupt by a deadline. The policy is `Resident` (the handler stays and checks the clock), `Threaded` (a watchdog thread), or `Signal` (Linux only, else `Threaded`). `HostCallbacks.interruptInstallPolicy` must be `Resident` when the quanta clock is not wall time (178–194). Eight opcodes, 60 ms, and a quanta deadline are three different clocks. Section 4 treats slice length as a tolerance.

### Memory

Phlox's script account is `MemoryInfo.MAX_MEMORY` `0x20000` (128 KiB), not a config key (`InWorldz.Phlox/VM/MemoryInfo.cs` 26). `UseMemory`, an operand push, a stored replace and `AddCall` throw `VMException("Out of memory")` past that. The scheduler's tick catch calls `TerminateWithError` and takes the script off the run queue (`PhloxExecutionScheduler.cs` 1441–1452). `MemoryCalc.CalcSizeOf` (`Util/MemoryCalc.cs` 30–64) counts an int as 4, a float as 4, a string as 4 plus 2 per character, a vector as 12, a rotation as 16, and a list as its own size. Anything else counts as 0. The base charge is the bytecode length plus the constant pool (`CompiledScript.cs` 94–100), applied in the interpreter constructor (`Interpreter.cs` 105–109). This is the account `llGetUsedMemory` reads. It is not the .NET heap. `LSLTable` and `LSLList` each throw on their own size past `0x20000` (`LSLTable.cs` 176–200, `LSLList.cs` 88–95). `MemoryCalc` does not add a table into `MemInfo`. `llSetMemoryLimit` returns 1 only for 131072, otherwise 0. `llGetMemoryLimit` always returns 131072 (`LSLSystemAPI.cs` 3136–3142). `llScriptProfiler` and `llGetSPMaxMemory` are a peak field (3143–3159).

YEngine's heap defaults to `ScriptHeapSize` 1024 shifted left by 10, so 1,048,576 bytes, and the stack to 2048 shifted the same way (`XMREngine.cs` 218–219). Both are floored at 16384 (`XMRInstCtor.cs` 68–71). The counted bytes are locals plus array use (`XMRInstAbstract.cs` 493–523). Strings count `Length * 2`. An empty array costs 64 plus its keys and values. `UpdateLocalsHeapUse` throws `OutOfHeapException` over the limit. The handler sends an error that includes `OutOfHeap:` and `"Script must be Reset to re-enable."`, then sets the sleep-until to `DateTime.MaxValue` (`XMRInstRun.cs` 537–540, 554–613). Compiled `llGetFreeMemory` and `llGetUsedMemory` are `xmrHeapLeft` and `xmrHeapUsed` (`MMRScriptInlines.cs` 152–153, 588–618). The `LSL_Api` stubs that return the constant 65536 (`LSL_Api.cs` 7842–7846, 17968–17971) are not what a compiled YEngine script calls. `llSetMemoryLimit` on that API returns false (17956–17959).

Linden's default is the same 128 KiB figure and a different counter: `kDefaultMemoryLimit = 1024 * 128` (`Executor/include/Luau/Executor.h` 23–24). `Script::setMemoryLimit` is `Script.h` 182–183. `llGetFreeMemory` cannot be bit-identical across these three accounts. [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) already leaves that open. [D10](../DECISIONS.md) runs Linden's VM, so the script-visible number is Linden's counter. Section 4 compares the other two as tolerances.

### Sleeps and delays

Phlox's `ScriptSleep` (`LSLSystemAPI.cs` 241–249) sets `NextWakeup` to `Clock.Now` plus the milliseconds and sets `RunState` to `Sleeping`, unless the call is inside a `SyscallContext`, in which case it adds the milliseconds to that context and does not change run state. `CheckSleepingScripts` (scheduler 1564–1589) moves due scripts back to the run queue. The master thread waits until the heap's earliest wake. Off-thread, `ProcessSyscallReturns` (3068–3077) sleeps `Clock.Now + Delay`, or, when the delay is 0, puts the script back on the run queue. There is no single delay table. Each method calls `ScriptSleep` itself.

| Call | Phlox | YEngine |
| --- | --- | --- |
| `llSleep` | `(int)(sec * 1000)` (`LSLSystemAPI.cs` 2824) | `inst.Sleep` (`XMRInstBackend.cs` 66–68). `ScriptDelayFactor` does not multiply it. |
| `llSetPos`, `llSetRot`, `llSetLocalRot` | 200 ms (`LSLSystemAPI.cs` 1988–2024) | 200 ms in `LSL_Api.cs` 138–152, multiplied by `ScriptDelayFactor` when the product is at least 10 ms (`XMRLSL_Api` `ScriptSleep`). |
| `llSay`, `llShout`, `llWhisper`, `llRegionSay`, `llRegionSayTo`, `llOwnerSay` | 15 ms when `ChatThrottle` is true, the default (`PhloxEngine.cs` 184–185, 304; `ChatSleep` at `LSLSystemAPI.cs` 257–260). The same 15 ms follows a script error on `DEBUG_CHANNEL`. | No per-call sleep. On channel 0, the 11th say or shout inside one 1-second window sleeps 2000 ms (`LSL_Api.cs` 1287–1340). |
| `llInstantMessage` | 2000 ms (`LSLSystemAPI.cs` 1678–1680) | The shared API's pause constants (`LSL_Api.cs` 138–152). |
| `llEmail` | 20000 ms in a `finally` (`LSLSystemAPI.cs` 15432–15448) | `EMAIL_PAUSE_TIME` 20 seconds in the same constants. |
| Link message, queue nearly full | 50 ms when `LinkMessageThrottle` is on (`LSLSystemAPI.cs` 305–323) | No extra delay. The queue cap is the event cap. |
| Physics frame slow | Pause for the average physics frame when the 10-frame average exceeds 30 ms (`LSLSystemAPI.cs` 290–302), sampled from `OnFrame`. | No equivalent on the API. |
| Notecard line | `NotecardPause`: the `sl` preset is 0.1 s; `halcyon` is 25, 50 and 1 ms (`LSLSystemAPI.cs` 341–386). | Its own pauses in `LSL_Api`. |
| Too many resets | More than 5 resets in 1 s sleeps 5 s (`LSLSystemAPI.cs` 885). | Separate. |
| `llSensor`, `llSensorRepeat` | No `ScriptSleep` (5279–5295). The repeat interval is the script's rate, polled on the async thread. | The sensor plugin polls. No `ScriptSleep` on the call itself. |
| Bot calls | 15 ms when `BotThrottle` is on. | YEngine has no `bot*` calls. |

`[YEngine] ScriptDelayFactor` is commented at 1.0 (`RegionServerDefaults.ini` 2456–2457). YEngine's `ScriptSleep` multiplies delays other than `llSleep` by that factor when the result is at least 10 ms.

[SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) says `LSLSystemAPI` stays. YEngine's table is what a script on that engine sees today, and hypergrid visitors still arrive with YEngine state. Which table the oracle uses is question 2. The hello packet in P4.1d does not include the 15 ms or the 2000 ms.

### Events and the queue

Both engines cap a queue at 64. They count differently.

Phlox: `RuntimeState.MAX_EVENT_QUEUE_SIZE` is 64 (`RuntimeState.cs` 23). `QueueEvent` drops when `Count` is at 64, except `on_rez`, `state_entry`, `state_exit` and `timer`, which may pass the cap (477–501). The scheduler uses the same 64 as `MAX_EVENT_QUEUE_DEPTH` (`PhloxExecutionScheduler.cs` 74–75) and drops at 1815–1832. No handler means the event is dropped (1811–1813). A disabled script drops events except `state_entry` and `state_exit`, which stay queued (1767–1775). Order is FIFO. A duplicate no-change `control` is not queued while one already waits (1860–1864). An answer meant for another script (`AnswersOtherScript`) is dropped when the queue is at half, 32 (`OTHER_ANSWER_LIMIT` at 1877, checks at 1787 and 1822). `http_response` is dropped unless the HTTP plugin still owns the request key (`PhloxEngine.cs` 1973–1977). `TOUCH_INTERVAL` is 100 ms (scheduler 73).

`llMinEventDelay` (`LSLSystemAPI.cs` 2825–2832) is a floor between handler starts. Events inside the window are queued, not dropped. A heap wake delivers them (scheduler 3154–3185). A state change clears the queue, pending events and listens, then posts `state_exit` and `state_entry` (3082–3113). A timer is re-armed when the new state has `timer()`.

YEngine: `MAXEVENTQUEUE` is 64 (`XMRInstMain.cs` 81), counted per event code in `m_EventCounts`. `PostEvent` (`XMRInstRun.cs` 54–223): an unknown name or a missing handler returns false. A script that is not running returns true without queueing, except a suspended script with an empty queue accepts one `state_entry` at the front (70–82). While `llMinEventDelay` is above 0.001 s (`XMRInstMisc.cs` 307–320), these are dropped until the next event time: collision, land collision, listen, not-at-target, not-at-rot-target, no_sensor, sensor, timer, touch (86–105). `changed` is delayed only when the mask is a subset of scale and position (107–120). A second `timer` is dropped (127–132). Any other code is dropped when its own count is already 64 (136–137). `state_entry` goes to the front. An `attach` of `NULL_KEY` is inserted after existing `state_entry` and `attach` events and sets `m_DetachQuantum` to 100 (157–177).

The viewer event queue (`event.queue`, 256) and the viewer chat queue (`client.chat-queue`, 64) in [LIMITS.md](../LIMITS.md) are different caps. They are not the script queue.

### Detect parameters

`DetectParams` is `ScriptEngine.Shared/Helpers.cs` 95. Type flags: `AGENT` 1, `ACTIVE` 2, `PASSIVE` 4, `SCRIPTED` 8, `OS_NPC` `0x01000000` (97–101). Fields run through the touch face. Defaults are face −1 and touch ST and UV of (−1, −1, 0). `EventParams` is the same file at 323. Phlox stores `DetectVariables[]` on the posted event (`PostedEvent.cs` 22). `DetectVariables.cs` 46–312 adds `BotID` and a non-serialized `AdjustDamage`. `GetDetect` (`LSLSystemAPI.cs` 5196–5211) requires `0 <= n < DetectVars.Length`. Out of range, touch face is −1 and touch ST and UV are (−1, −1, 0). Other fields stay at their defaults. `iwDetectedBot` reads index 0 only (5254–5262).

`ConvertDetectParams` (`PhloxEngine.cs` 2477–2510) fills detect data for touch, collision, money, sensor, `on_damage` and `final_damage`. Land collision, `changed`, attach, moving, targets, control, link message and email pass an empty array. A prim listen has null detect. A bot listen has one entry whose `BotID` is the bot (`PhloxListenManager.cs` 741–761).

YEngine sets `m_DetectParams` from the dequeued event and clears it when the handler returns (`XMRInstRun.cs` 392, 412–414). Filled: touch, one record (`XMREvents.cs` 164–180); `changed`, only when the parameter is a UUID, and then only the key (194–200); money, one record (90–93, 220–227); collision, one per collider (244–263); sensor, one per hit up to `SensorMaxResults` (`SensorRepeat.cs` 263–298). Empty: listen, land collision, timer, `no_sensor`, link message, control, email, at-target, attach, moving (`XMREvents.cs` 214–380, `LSL_Api.cs` 5824–5826, `ScriptTimer.cs` 139–141).

### Timers

Phlox `llSetTimerEvent` (`LSLSystemAPI.cs` 2786–2819) raises a positive interval to `MinTimerInterval`. Zero and negative stop the timer and are not raised. The code default is 0.1 s (`PhloxEngine.cs` 147, 160, 276). The ini key under `[InWorldz.Phlox]` is commented (`RegionServerDefaults.ini` 2516), so the shipped region uses 0.1. The scheduler stores milliseconds and arms the sleep heap (1231–1254). The next `timer()` is armed when that event is delivered.

YEngine keeps one `ScriptTimer` per script key (`ScriptTimer.cs` 75–106). `llSetTimerEvent` floors a non-zero interval at `m_MinTimerInterval`, whose code default is 0.5 s (`LSL_Api.cs` 113, 518, 4079–4080). The shipped `[YEngine] MinTimerInterval = 0.1` (`RegionServerDefaults.ini` 2469) overrides that, so a shipped region also uses 0.1. The code default of 0.5 applies only when the key is absent. `CheckTimerEvents` posts `"timer"` with an empty detect array (`ScriptTimer.cs` 122–145). The queue holds at most one timer. The async poll thread uses `[YEngine] AsyncLLCommandLoopms`, default 100, clamped to 25–250 (`AsyncCommandManager.cs` 179–182).

### Listen

Phlox `llListen` is `LSLSystemAPI.cs` 5324–5350. `PhloxListenManager` defaults are 20 events per second (0 or below means no rate limit), 65 listens per script and 1000 per region (`PhloxListenManager.cs` 69–77, 110–117). A per-script cap below 1 means unlimited (205). The shipped `[LL-Functions]` sets `max_listens_per_script = 64` and `max_listens_per_region = 1000` (`RegionServerDefaults.ini` 2159–2165). The rate key is commented, so 20 per second stands. Delivery (485–582) requires an active listener, the same channel, a prim that is not the speaker (538), then name, key and message, ordinal-exact, then distance. Default ranges are whisper 10, say 20, shout 100 (180–182). Equality is outside the range (`>=` on the squared distance, 557). `PostListenEvent` (751–778) queues `listen` with channel, name, key and message. `listen` is not an overflow event, so it counts toward the 64.

YEngine `llListen` (`LSL_Api.cs` 1390–1397) calls `IWorldComm.Listen`. `WorldCommModule` uses the same ini keys. Its code default per script is 65 and per region 1000, and the shipped ini overrides the per-script cap to 64. `TryEnqueueMessage` (687–743) requires active, same channel, not the speaking prim, an id match when the filter id is not zero, and name and message compared with `InvariantCulture` unless the regex bit is set. Empty name, empty message and `UUID.Zero` are wildcards. The queue is per event code, so listen drops when that code's count is already 64. `llMinEventDelay` can drop it as well. `WorldCommModule.AddRegion` (142–147) subscribes to client chat and broadcast. An object say reaches YEngine through `DeliverMessage` (404–436). Heard only when `maxDistanceSQ` is strictly greater than the squared distance (737), so the exact boundary is not heard. Ranges are stored already squared.

Chat ranges for viewers are the same 10, 20 and 100 (`ChatModule.cs` 47–49, 73–81; ini `[Chat]` 996–1007). `TrySendChatMessage` (418–460) drops whisper, say and shout when `maxDistSQ` is less than the squared distance, so the exact distance is still sent to the viewer. Phlox's listener and YEngine's listener and the viewer path therefore disagree at the exact boundary. That sample is a tolerance (question 10), not a byte to match.

Channel 0 say and shout also disagree on reach. Phlox `llSay` (`LSLSystemAPI.cs` 1539–1545) calls `HostSimChat` (670–677), which always uses `Scene.SimChat` with broadcast false, then `ChatToWorldComm` so the other engine hears it, then `ChatSleep`. `llShout` (1547–1553) is the same with `ChatTypeEnum.Shout` and broadcast false, so viewers use the 100 m test. YEngine `llSay` (`LSL_Api.cs` 1307–1328) calls `SimChat` with broadcast false. YEngine `llShout` (1332–1348) calls `SimChat` with broadcast true, and `OnChatBroadcast` (`ChatModule.cs` 334–398) does not apply distance, so a channel-0 shout is region-wide for viewers. `llRegionSay` on channel 0 is an error in both (`LSLSystemAPI.cs` 1607–1617, `LSL_Api.cs` 1351–1356). `ChatTypeEnum` (`OpenSim.Framework/ChatTypeEnum.cs` 30–42) is Whisper 0, Say 1, Shout 2, DebugChannel 6, Region 7, Owner 8, Direct 9, Broadcast `0xFF`. OpenMetaverse's `ChatType.RegionSay` is 255 and its comment says that value is not sent to the client (`AgentManager.cs` 199–221, the libopenmetaverse revision in [REFERENCES.md](../REFERENCES.md)). The hello compare uses Say, value 1. Region-say's packet type is a stage-2 reading of what `ChatModule` puts on the wire.

`ChatFromSimulator` is built in `LLClientView.SendChatMessage` (1108–1152): reliable, low frequency, id 139. Body: from-name, source id, owner id, source type, chat type, audible, position, message via `StringToBytes1024`. `ChatSourceType` is System 0, Agent 1, Object 2 (`AgentManager.cs` 227–234). `ChatAudibleLevel.Fully` is 1. Object chat on `DEBUG_CHANNEL` (2147483647) is rewritten to chat type 6 and sent only to the owner (`ChatModule.cs` 281–291). Messages longer than 1100 characters are cut to 1000 (260–261, 346–347). The first hello compare is the object say on channel 0. The owner-only debug path is a later row.

### Sensors

Both engines default `SensorMaxRange` to 96 and `SensorMaxResults` to 32 (`Phlox.ScriptEngine/AsyncCommand/Plugins/SensorRepeat.cs` 52–53; `ScriptEngine.Shared/Api/Plugins/SensorRepeat.cs` 80–81). Both ini keys are commented (`RegionServerDefaults.ini` 2472–2474 and 2585–2587), so the defaults stand. One repeater per script. `llSensorRepeat` replaces it. A rate of 0 removes it. The sweep posts `sensor` with one detect record per hit, or `no_sensor` when there are none. The sweep is a scene query. It is not the physics ray cast.

### Link messages

Phlox `llMessageLinked` (`LSLSystemAPI.cs` 3076–3089) posts to the targeted prims. When `LinkMessageThrottle` is on and the receiver's queue is nearly full, the sender sleeps 50 ms (305–323). YEngine `llMessageLinked` (`LSL_Api.cs` 5796–5838) posts `"link_message"` to every script in the targeted prims and to the other engine. The detect array is empty. Neither engine adds a second cap on top of its 64. An engine with no instance, or a script with no handler, drops the event.

### Script state

Three formats travel. They are not the same document.

**YEngine snapshot.** `GetExecutionState` (`XMRInstCapture.cs` 45–217) writes the `.state` file (209–215). The outer wrapper from `GetXMLState` (`XMREngine.cs` 1132–1167) is `<State Engine="YEngine" UUID Asset>`. The method's own comment (1128–1131) names Take, Take Copy and delete-to-trash as callers. The inner `<ScriptState>` carries Engine, Asset, SourceHash, `StateSchemaVersion` (constant 1, `XMRInstMain.cs` 167), `MigrationVersion` (byte 12, line 164), `GlobalsSignature` and `ApiFingerprint`. Children include the base64 snapshot, Running, DoGblInit, script time, minimum event delay, permissions, the detect array, the event queue, plugins, the experience key and heap use (capture 92–192). The binary snapshot is a version byte, an ignored byte 16, then `MigrateOut` (225–231). Detach waits up to 5 seconds (`m_DetachReady.WaitOne(5000)`). `LoadScriptState` (`XMRInstCtor.cs` 591–751) then `MigrateInEventHandler` (1374–1407) reads it back. An asset or source-hash mismatch throws and the script resets (633–643). `StrictStateMigrationVersion` defaults to false (`XMREngine.cs` 216). When it is false a version mismatch is best-effort. `SetXMLState` accepts `Engine="YEngine"` or `Engine="XEngine"` (`XMREngine.cs` 1172–1229).

On region remove, `DoMaintenance` writes a `.state` for every instance whose object is not an attachment (`XMREngine.cs` 700–702, 1970–1986). `OnShutdown` is empty (1695–1698). The maintenance timer defaults to 10 minutes (265–271). Removing a script deletes its `.state` file (1589–1592).

**Foreign `<Variables>` form.** When Engine is not YEngine and a `<Variables>` child exists, `processXstate` (`XMRInstCtor.cs` 617–626, 800–1147) reads State, Running, ExperienceKey, Variables (name and type, matched to the compiler's global names), Queue, Plugins, Permissions and MinEventDelay. Keys are stored as strings. It restores globals by name, the state, the running flag, the queue, plugins, permissions, the experience and the minimum delay. It forces global-init off, the current event to none, detect data null, and the extension arrays empty (1103–1124). There is no stack snapshot. A missing `<Variables>` when the engine is wrong throws. Elements the switch does not know are skipped. This is the hypergrid and IAR/OAR form [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) and [PLAN.md](../PLAN.md) already require Ferroan to read and write. A legacy `<ScriptState>` with no engine is rewritten as `<State Engine="XEngine">` in `SceneObjectPartInventory.RestoreSavedScriptState` (594–625).

**Phlox state.** On disk this is protobuf-net `SerializedRuntimeState` in SQLite (`StateManager.cs` around 397–399 and 585). The table is `script_state` under `ScriptEngines/Phlox/state/`. The directory is not per region. `SerializedRuntimeState.cs` 21–80 holds the instruction pointer, LSL state, globals, operands, calls, the top frame, memory info, the event queue, run state, enabled, next wake, capture time, timer interval and last schedule, the running event, permissions, active listens and the start parameter. Halcyon databases are not imported. `StateRowMaxAgeDays` is 0.

The carried form is `GetXMLState` (`PhloxEngine.cs` 1640–1714): `<State Engine="InWorldz.Phlox" UUID="item" Asset="asset" Version="1">` and one base64 child of the protobuf. `CarriedStateVersion` is `"1"` (1640). The capture timeout is 10 seconds (1646). `MaxCarriedStateBytes` is `(MAX_EVENT_QUEUE_SIZE + 1) * MAX_MEMORY` (1653), checked before decode. Capture runs on the scheduler thread, between timeslices, for all of the object's scripts at once. A caller that is the scheduler thread captures directly. Any other caller waits at most 10 seconds. On timeout the object travels without state. `GetXMLState` returns empty when the script is not a Phlox script loaded here, the row could not be read, or the state is over the byte cap.

**Where the object XML writes it.** `SceneObjectSerializer.ToOriginalXmlFormat` (174–180, 262–263) calls `SceneObjectGroup.SaveScriptedState` when script states are requested (`SceneObjectGroup.cs` 2716–2746). The XML2 writer (`SOGToXml2`, 1611–1637) does not write that block. `FromXml2Format` still calls `LoadScriptState` (336–338), so a stray tag would load, and the XML2 writer never emits one. Take uses the original form (`Scene.Inventory.cs` 3049) and includes state. OAR save uses `SerializeGroupToXml2` (`ArchiveWriteRequest.cs` 624) and does not write script state. OAR load starts scripts with state source 0 (`ArchiveReadRequest.cs` 579–591). IAR object assets are inventory XML. `InventoryArchiveReadRequest.cs` 531–536 rewrites through the original form, which does carry state. [TESTING.md](../TESTING.md) records that a loaded group has no script state, so Ferroan's original-XML writer does not add one today (the region-store serialize paragraph).

[SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) defines the portable state Ferroan reads and writes: versioned LLSD, globals by name and LSL type, state name, running, the queue with detect data, the timer interval and time left, listens with their original handles, sensors, permissions, the experience key, the minimum event delay and the start parameter. Only between events. That document is the spec. This plan does not replace it. Question 3 is whether the YEngine snapshot and the Phlox protobuf are also loaded, beyond that form.

### Script-visible stats

`SimStats` indexes `ScriptMS` 10, `ActiveScripts` 15 and `ScriptEps` 31 (`OpenSim.Framework/SimStats.cs` 52, 57, 73). The array positions sent to the viewer are `ActiveScripts` 19, `ScriptEps` 28 and `ScriptMS` 37 (126, 135, 144). `SimStatsReporter` fills active scripts, `ScriptMS`, `ScriptEps`, `LSLScriptLinesPerSecond` and `ScriptEngineThreadCount` (365–400) and stores `lastReportedSimFPS` for `llGetRegionFPS` (340–341). `llGetRegionFPS` returns that value (`LSL_Api.cs` 7866–7871). `llGetRegionTimeDilation` returns `World.TimeDilation` (7861–7863), which is the physics scene's dilation (`Scene.cs` 532–534). Script time is accumulated in `Scene.AddScriptExecutionTime` (2022–2026) and folded into the frame stats (1999–2002). Phlox `llGetSimStats` indexes `LastReportedSimStats` (`LSLSystemAPI.cs` 16955–16968): 0 is time dilation through 21 `SimSpareMs`, and 19 is active scripts. `llGetRegionTimeDilation` on Phlox is `World.TimeDilation` or 1 (1852). The monitor names are `"Active Scripts"` and `"Script Events"` (`MonitorModule.cs` 193–207). YEngine's `yeng` console (`XMREngine.cs` 274–277, help 803–812) prints per-script CPU milliseconds, event count and status (`XMREngXmrTestLs.cs` 47–178). `GetAverageScriptTime` on `ISystemAPI` is a host callback, not a script function.

### What the C# SLua work already replaces

Nothing of the VM. `Docs/PhloxSLua.md` describes SLua as Luau-flavoured source compiled onto the Phlox VM. LSL and SLua emit the same bytecode and share the scheduler and the state store. A script is SLua when, after leading whitespace, it starts with `--`, or when the first line is `//InWorldz.Phlox:slua`.

`Tests/SluaProofRunner/Program.cs` is an offline harness. `RunBody` (67–85) wraps a snippet as `function run()` under `--!slua`, compiles with `SLuaCompiler.CompileToAssembly`, assembles with `CompilerFrontend`, and runs the Phlox interpreter. A recording shim keeps the string argument of `llSay`, `llShout`, `llWhisper` and `llOwnerSay`. Results are bucketed PASS, DIVERGENCE or GAP (87–95). The file's comment that some `math` calls are expected gaps is stale relative to `InWorldz.Phlox/SLua/LuaLib.cs` 77 and 138–139, which declares `MathLog10`, `MathSinh`, `MathCosh`, `MathTanh` and `MathNoise`. This plan did not run the harness. The runner does not call Luau, and it does not describe a C API.

`Phlox.ScriptEngine/SluaBackHalfProof.cs` `Run` (102–197) assembles a hand-written Phlox program (the text at 75–96), ticks it part way, serialises `SerializedRuntimeState` with protobuf-net, restores into a second interpreter and ticks to the end. The script makes no syscall. The report is `RESULT: PASS` or `RESULT: FAIL`. It is a Phlox-assembly round trip. It does not involve a Luau parser.

No file under `InWorldz.Phlox/SLua` or `Tests/SluaProofRunner` imports a lua, luau or slua native library. The C interface [D1](../DECISIONS.md) tells Phase 4 to reuse is the one [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) still has to write on the C# side (create and free environments, run, resume, abort, save, restore, register host functions). It is not in Pin8.

What Ferroan can bind today is Linden's C++ API at the pinned `slua` revision:

| Piece | File | What it does |
| --- | --- | --- |
| Executor | `Executor/include/Luau/Executor.h` | `kDefaultMemoryLimit` 128 KiB (24). `HostCallbacks` (178–194): clock, random, `setTimerEvent`, event-handler registration, quanta clock, interrupt policy, `populateEnvironment`. `IEnvironment` is exclusive to one thread (196–202). `ImageConfig` is a bytecode asset (36–45). `ScriptConfig` carries the memory limit and a host context (49–57). |
| Script | `Executor/include/Luau/Script.h` | `callEventHandler` (132), `serializeState` (175), `restoreState` (179), `setMemoryLimit` (182). A payload header leads every serialised blob (54–58). |
| LSL values | `VM/include/llsl.h` | vector, rotation, key, list. |
| Yield | `VM/include/lyieldable.h` | How a host function yields and resumes. |

[SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) also names `Compiler/src/LSLCompiler.cpp`, `builtins.txt` and `VM/src/lll.cpp` (39 functions implemented inside the VM). The host surface that document keeps is Phlox's `ISystemAPI`, `LSLSystemAPI` and `OsslGate`, not YEngine's `LSL_Api`. The proof runner's PASS, DIVERGENCE and GAP buckets are evidence about Phlox's subset compiler. They are not the conformance suite the Phase 4 gate names.

## 2. The function surface

Counts are methods on `ISystemAPI`, each overload counted. Unique names are in parentheses. The inventory is disjoint: a name is in one family. Stage 2 may split a family. It should not move a method into a second family and add it again.

| Prefix | Methods | Unique names |
| --- | --- | --- |
| `ll` | 551 | 541 |
| `os` | 291 | 260 |
| `bot` | 58 | 58 |
| `iw` | 85 | 85 |
| Host callbacks, no script prefix | 10 | 10 |
| Total | 995 | 954 |

`ll` + `os` + `bot` is 900 methods. [PLAN.md](../PLAN.md) and [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) call the surface 831 functions and split it 536, 262 and 59. Those three numbers sum to 857, and none of them matches this interface. `bot` is 58, not 59. There is no 59th `bot*` declaration. This plan uses the pin count. 831 stays the figure in those two documents until a later edit. Thirty-three names are overloads (41 extra signatures). The ten `ll` overloads are pairs: `llDerezObject`, `llFindNotecardTextSync`, `llGetAttachedListFiltered`, `llGiveAgentInventory`, `llLinkPlaySound`, `llMapBeacon`, `llSHA256String`, `llTargetedEmail`, `llTransferOwnership`, `llUpdateKeyValue`. The other twenty-three overloaded names are `os*`. `osApproxEquals` has six signatures. `osTeleportAgent`, `osTeleportOwner`, `osSetPenColor` and `osSetProjectionParams` have three. No `bot*` or `iw*` name is overloaded. Both `llSRGB2Linear` and `llsRGB2Linear` are declared. They are two methods.

The ten host callbacks are `SetScriptEventFlags`, `CompleteSyscall`, `ShoutError`, `OnScriptReset`, `OnStateChange`, `OnScriptUnloaded`, `AddExecutionTime`, `OnScriptInjected`, `OnGroupCrossedAvatarReady` and `GetAverageScriptTime` (`ISystemAPI.cs` 20–36). They are not script functions.

### `ll` families (551)

| Family | Methods | What it holds | Depends on |
| --- | --- | --- | --- |
| Math, strings and crypto | 64 | `llSin` through the hashes, base64, `llSignRSA`, `llComputeHash`, both sRGB conversions, `llGenerateKey` | None for the pure calls. |
| Lists and JSON | 32 | `llList*`, `llJson*`, `llParseString2List`, `llCSV2List`, `llSortListStrided` | None. |
| Communication | 20 | Say, whisper, shout, region say, listen, dialog, text box, email, IM, `llLoadURL`, `llMapDestination`, `llMapBeacon` | Channel-0 object say is P4.1d. The rest is stage 2. Map UI is viewer-visible. |
| Link messages and linkset data | 14 | `llMessageLinked` and the 13 `llLinksetData*` methods | Link messages are P4.9. Linkset data is Phase 5. |
| HTTP, XML-RPC and experience | 29 | `llHTTPRequest`, URL request and release, the remote-data channel, experience permissions, experience key-value | HTTP needs the HTTP client. Experiences and key-value are Phase 5. |
| Inventory, rez and notecards | 33 | Notecard reads, give, rez, derez, `llRemoteLoadScript`, `llTransferOwnership`, `llGodLikeRezObject` | Inventory items exist from Phases 1 and 2. Give and rez dialogs are viewer-visible. The full transfer path is Phase 5. |
| Land, estate, region and environment | 64 | Parcels, ban and pass lists, estate access, return objects, ground height and ground textures, region name and flags, sun and moon, `llGetEnvironment`, `llSameGroup`, `llWind`, `llCloud`, `llWater` | Land, estates, groups and environment are Phase 5. Ground height can read the scene height the frame already serves. |
| Physics-backed | 66 | Impulse, force, torque, velocity, mass, buoyancy, hover, vehicles, volume detect, cast ray, collision filter, targets, pathfinding, `llSetStatus`, `llSetKeyframedMotion`, `llTargetOmega`, damage and health | Phase 3. P3.18a leaves script delivery to this phase. P3.26 and P3.46 leave the calls that apply forces to this phase. |
| Avatars, animation, camera, permissions, money | 61 | `llRequestPermissions`, animations, attach, sit, camera, teleport, names, `llGiveMoney`, `llTransferLindenDollars`, pay price, `llGetAgentRot` | Animations, sit and attachments are viewer-visible and need the scene. Money and groups are Phase 5. Permission storage is P4.12. |
| Sound and media | 34 | `llPlaySound` and the link sound calls, collision sound, prim media, `llSetPrimURL`, the particle bursts `llMakeExplosion`, `llMakeFire`, `llMakeFountain`, `llMakeSmoke` | Sound, media and particles are viewer-visible. PLAN puts media and sound in Phase 5. |
| Sensors and detect | 21 | `llSensor`, `llSensorRepeat`, `llSensorRemove`, every `llDetected*` | The sweep is P4.8. The detect getters are stage 2. |
| Object, prim and script control | 113 | Prim params, textures, sit target, particles, scale, text, link, GLTF, `llSleep`, `llSetTimerEvent`, `llMinEventDelay`, memory getters, `llResetScript`, `llGetFreeMemory`, `llDie`, `llSetPos`, `llSetRot` | Poses and prim params are the scene. The sleep, timer, memory and reset mechanisms are section 5. `llSetPos` of a physical prim waits on Phase 3. |

64 + 32 + 20 + 14 + 29 + 33 + 64 + 66 + 61 + 34 + 21 + 113 = 551.

Permissions are a cross-cut. `llRequestPermissions` is one of the 61 avatar methods. Functions in other families that refuse without a grant stay in those families. They are not added again. `llGetAndResetTime`, `llGetTime`, `llGetTimestamp`, `llGetUnixTime`, `llGetWallclock` and `llGetGMTclock` sit in object and script control with the other script-lifetime calls. Stage 2 can split that family. The count stays 113 until it does.

### `os` families (291)

Every `os*` call goes through `OsslGate` (section 1, and P4.16). The threat check is not a second copy of these counts.

| Family | Methods | Examples |
| --- | --- | --- |
| Math, strings, lists, crypto | 47 | `osApproxEquals`, `osStringSubString`, `osListSortInPlace`, `osSHA256`, `osAESEncrypt`, `osRegexIsMatch`, `osSlerp` |
| Avatars, teleport, attach, groups | 40 | `osTeleportAgent`, `osForceAttachToAvatar`, `osForceOtherSit`, `osKickAvatar`, `osInviteToGroup`, `osAgentSaveAppearance` |
| Land, estate, region, sun and terrain | 35 | `osSetTerrainHeight`, `osGetParcelDetails`, `osSetEstateSunSettings`, `osRegionNotice` (2), `osRegionRestart` (2), `osSetRegionWaterHeight` |
| Drawing | 31 | `osDrawLine`, `osDrawText`, `osSetDynamicTextureData`, `osSetPenColor`, `osMovePen` |
| NPC | 26 | `osNpcCreate`, `osNpcSay`, `osNpcMoveTo`, `osIsNpc`, `osGetNPCList` |
| Physics, wind, health | 22 | `osSetInertia`, `osVolumeDetect`, `osSetSpeed`, `osGetPhysicsEngineName`, `osSetWindParam`, `osCauseDamage`, `osSetHealth` |
| Inventory and notecards | 21 | `osMakeNotecard`, `osGetNotecard`, `osGetInventoryItemKey`, the link-inventory getters |
| Sound | 13 | `osPlaySound`, `osLoopSound`, `osTriggerSound`, `osPreloadSound`, `osSetSoundRadius` |
| Object, prim, sit, particles, script engine | 33 | `osSetPrimitiveParams`, `osSetProjectionParams`, `osForceCreateLink`, `osParticleSystem`, `osSetSitActiveRange`, `osSetScriptEngine`, `osDie` |
| Grid and simulator queries | 10 | `osGetGridName`, `osGetSimulatorVersion`, `osGetSimulatorMemory`, `osConsoleCommand` |
| Time and creation stamps | 8 | `osGetApparentTime`, `osGetPSTWallclock`, `osLoadedCreationDate`, `osLoadedCreationID`, `osLoadedCreationTime` |
| HTTP | 3 | `osRequestURL`, `osRequestSecureURL`, `osSetContentType` |
| Communication | 2 | `osMessageObject`, `osListenRegex` |

47 + 40 + 35 + 31 + 26 + 22 + 21 + 13 + 33 + 10 + 8 + 3 + 2 = 291. `osRegionNotice` is viewer-visible and sits in the land and region row because that row holds the region-admin calls. Stage 2 can move it. NPC, drawing, land, money-adjacent teleport and grid admin are the threat-sensitive calls. Physics `os*` waits on Phase 3. Land, estate, NPC and drawing wait on Phase 5 even though the signatures exist at the pin. Dynamic textures are the drawing row, and PLAN puts them in Phase 5.

### `bot` (58)

One family. `botCreateBot` starts at `ISystemAPI.cs` 827. The set covers create and remove, movement, chat, animation, appearance and the persistence calls near 1109–1113. YEngine does not declare these names. A bot that chats, animates or moves is an NPC avatar on the wire, so viewers and other regions see an agent. PLAN puts NPCs and bots in Phase 5. The engine can register the `bot` table before those functions have bodies.

### Not in the 900

`iw*` is 85 methods on the same interface. Phlox ships them. [D10](../DECISIONS.md) names LSL, SLua, OSSL and bot, and does not name `iw*`. They are grid-specific. Other servers do not have them. Question 6 asks whether stage 2 includes them. They are not a placeholder row until that is answered.

YEngine's shared API also has `lsGetWindlightScene`, `lsSetWindlightScene`, `lsSetWindlightSceneTargeted` and `lsClearWindlightScene` (`Interfaces/ILS_Api.cs` 36–39) and nine `mod*` methods (`Interfaces/IMOD_Api.cs` 41–51). [D10](../DECISIONS.md) does not run YEngine. These 13 are not scheduled.

Nine `os*` methods are on `IOSSL_Api` and not on `ISystemAPI`: `osGiveLinkInventory`, `osGiveLinkInventoryList`, `osMessageAttachments`, `osNpcLookAt`, `osNpcSayTo`, `osReplaceAgentEnvironment`, `osReplaceParcelEnvironment`, `osReplaceRegionEnvironment`, `osResetEnvironment`. The YEngine spelling `osTemperature2sRBG` (`OSSL_Stub.cs` 1851) is `osTemperature2sRGB` on Phlox, which is inside the 47. A script compiled for YEngine that calls one of the nine does not compile on the Phlox host. That is part of question 2.

### What viewers and other servers already depend on

- Chat packets: source type, chat type, audible, ranges, the owner-only debug channel, the 1100-to-1000 cut. Other regions hear region-say and hypergrid crossings as the C# region sends them.
- Object updates from prim params, text, texture, particles, sit and scale.
- Avatar animation, sit, attachments and the permission dialog.
- Sound, prim media, parcel media and ground textures.
- `llLoadURL`, inventory offers, map destination.
- Bots, once they exist, as ordinary agents on the circuit.
- Physics motion and collision events, which Phase 3 owns until the call or the event crosses into a script.
- Linkset data, experience key-value, money and group powers, which other regions persist. Those services are Phase 5.

A function can be registered, and return a typed default, before the scene service it needs exists. The row that makes it match C# waits on that service. Section 5's placeholders say which.

## 3. What Ferroan has today, and the boundary

There is no `ferroan-scripting` crate. The workspace members stop at the crates that exist, and `unsafe_code = "forbid"` is the workspace lint (`Cargo.toml`). [ARCHITECTURE.md](../../ARCHITECTURE.md) already names the crate as the only caller of Linden's engine, in phase 4. [NOTICE.md](../../NOTICE.md) already names that engine.

The scene has no script state. [TESTING.md](../TESTING.md) says a loaded group has no script state, so the original XML form does not add one. No script event is posted. Collision contacts from Phase 3 stop at the scene (P3.18a).

The frame thread is `ferroan-scene`, spawned in `with_object_settings` (`crates/ferroan-scene/src/frame.rs` 354–388). `FRAME_TIME` is 0.0909 (96–98), the shipped ini value. The frame owns poses. A caller that needs an answer uses a oneshot. `chat_positions` (554–564) posts `Request::Places` and returns `Pending<HashMap<Uuid, [f32; 3]>>`. The frame copies each presence position and omits a missing agent (779–786). `ground` (444–449) is the same shape for a height. `Pending::wait` is async (162–166). `Pending::blocking_wait` is compiled for tests only (168–172). The script thread blocks on the oneshot receiver itself. It does not await on a tokio worker, and it does not call the test helper. Circuits post `AgentUpdate` on the bounded inbound queue (`SCENE_INBOUND` 256, lines 100–102) and do not wait.

Viewer chat is avatar-only. `crates/ferroan-client/src/chat.rs` says parcel hide, the god prefix, object chat and instant messages are not ported (35–37). `SOURCE_AGENT` is 1 (66). `publish` drops any channel other than 0 or `DEBUG_CHANNEL` (228). Say, whisper and shout use 10, 20 and 100 m (59–70). `DEBUG_CHANNEL` becomes chat type 6 and reaches every seated avatar with a pose, with no distance cap (45–47, 246–250). A message longer than 1100 UTF-16 code units is cut to 1000 (289–304). The packet sets source type to the agent constant, owner to the speaker, and audible to fully (the builder near 306–325). The outbound queue is 64 per viewer. A full queue drops the newest line and counts `client.chat-queue` ([LIMITS.md](../LIMITS.md)). The circuit accepts `ChatFromViewer` on channel 0 or `DEBUG_CHANNEL` and asks the frame for positions (`circuit.rs`). Nothing in that path has a prim name, an object source type, or an owner id taken from a prim.

### The boundary

`ferroan-scripting` owns the engine thread. That thread is the only caller of a given `IEnvironment` (`Executor.h` 200–202, [D6](../DECISIONS.md), [D8](../DECISIONS.md)). No other crate links the native library.

A host call that needs the scene posts a request on the frame's existing oneshot channel, the way `chat_positions` and `ground` do, and the script thread blocks for the reply. It does not await on a tokio worker. The frame reads the part pose it already stores and returns it. The script thread does not read scene fields itself. The frame does not call into the VM.

Events the scene wants a script to see (touch, collision, chat, link message) cross onto the script thread through a bounded queue. The pin's cap is 64 on both engines, counted differently (section 1). The counting rule is question 1. Until that is answered, the queue exists and the drop rule stays unshipped. The viewer caps `event.queue` and `client.chat-queue` stay what they are. Script caps get their own [LIMITS.md](../LIMITS.md) ids when P4.18 lands. This plan does not add those ids.

Object chat for the hello slice adds source type Object (2, `AgentManager.cs` 227–234), the prim's name, the owner's id, and the prim pose from the frame. Channel 0 is what viewers see. A non-zero channel stays inside the script engine for listeners and is still dropped by `publish` on the way to a viewer. Owner-only `DEBUG_CHANNEL` object chat is a later row, after P4.1d.

## 4. Proving equivalence with C#

### The harness

A C# oracle runs one script on the pin's shipped engine and records the result. The shipped ini selects Phlox, so that is the engine the harness starts. YEngine is a second recording when the question says a script that arrived on YEngine must match YEngine. Rust runs the same source on the bound engine. The compare is return values, events in order, side effects on the scene, and the chat packet.

The oracle has to run Tranquillity, so it is the kind of tool [D13](../DECISIONS.md) already allows once that decision names it. P4.19 is the row that asks for the edit. It does not invent the id. The proposed path is `tools/script-oracle`, the same shape as the oracles already under `tools/`. Ordinary CI does not start it. The proof runner's PASS, DIVERGENCE and GAP buckets are not this harness. They grade Phlox's subset compiler against itself.

The first useful compare is P4.1d. The script is `Constants.DefaultScriptText`. The packet is `ChatFromSimulator` for `llSay(0, "Script running")`.

### What compares exactly

Integers, strings, keys, list element types, and the order of events for one script. Listen handles, as numbers a script stores in a global. Permission-denial text, once that row exists.

The hello packet, field for field, once the pose is the scene's pose: source type Object (2), chat type Say (1), audible Fully (1), the message bytes of `Script running`, the prim name, the owner id, the source id, and the position. The 15 ms Phlox throttle, YEngine's 11th-say sleep of 2000 ms, and shout's broadcast-versus-100 m reach are not part of that compare.

### What needs a tolerance

- Floats. Compared as a named `libm` ulp, or as the integer a key or a handle actually is. A residual measured from Ferroan is not the tolerance. YEngine stores floats as doubles. Linden's LSL path uses 32-bit floats. [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) already says to convert as SL rounds and to record it.
- Time slices. Phlox counts 8 opcodes. YEngine yields at 60 ms. Linden uses a quanta deadline. A test that needs a slice to end asserts that the script parked and resumed, not that the same opcode fell on the same millisecond.
- Timers. The shipped floor is 0.1 s on both engines. YEngine's code default is 0.5 s when the key is absent. Wake time follows the scheduler's poll. The compare allows the poll interval of the engine under test.
- Function sleeps. The two tables in section 1. Which table is the oracle is question 2. A row that implements a number cites both and compares against the one question 2 names.
- `llGetFreeMemory`. Phlox's `MemoryCalc`, YEngine's 1 MiB heap, and Luau's 128 KiB counter. The script-visible value is Linden's. The other two are recorded, not required to match.
- Listen distance at the exact range. Phlox drops when the squared distance is greater than or equal to the range. The viewer path still sends when the squared distance equals the range. YEngine's `WorldComm` does not hear that point. Question 10. The listen row's evidence excludes that one sample until the question is answered.
- Shout reach. YEngine's channel-0 shout is a broadcast. Phlox's is 100 m. Question 2.

### Script state, both ways

The portable document is the `<Variables>` form `processXstate` already reads (`XMRInstCtor.cs` 800–1147), in the LLSD shape [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) specifies. Ferroan reads a document C# wrote. YEngine's `processXstate` reads the document Ferroan wrote. The round trip is globals by name and type, state name, running, the queue with detect data, listens with their original handles, the timer interval and time left, permissions, the experience key, the minimum event delay and the start parameter. It is taken between events. Keys stay keys. List element types stay.

A YEngine base64 snapshot and a Phlox protobuf envelope are not that document. P4.15 refuses each with the item id and the engine name, and does not reset the script. Loading either format, in either direction, waits on question 3. [PLAN.md](../PLAN.md) already says a script whose state does not convert stays on a C# region.

Object XML follows the pin. The original form can carry `<State>`. XML2 and OAR do not. Ferroan's original writer grows that block only when a script has state to write (P4.22). A group with no scripts still matches the current fixture, which has none.

### Risks

- Capture in the middle of an event. Both engines capture between slices. Phlox waits at most 10 seconds and then lets the object travel with no state. YEngine's detach waits 5 seconds. A compare of saved state uses a script that is between events.
- Two engines never both run one script at the pin. The harness must not start Phlox and YEngine on the same item and then average them.
- A script that uses a YEngine extension does not compile. The harness records that as "stays on C#", which is what [PLAN.md](../PLAN.md) already says.
- `iw*` and `bot*` are absent from YEngine. The nine `os*` names in section 2 are absent from Phlox. A single expected-output file cannot cover both engines for those names.
- Phlox SLua (`--!slua` compiled to Phlox bytecode) and Linden Luau are different compilers. The conformance suite is Linden's, once [D1](../DECISIONS.md)'s C# work has one. The Phlox proof runner is not a substitute.
- A viewer compile target of `luau` or `lsl-luau` selects no engine at the pin (section 1). The harness does not treat that flag as a Phlox script.
- Float 32 against YEngine's doubles, and key against string, change globals on the way through `<Variables>`. The round trip records the conversion. It does not silently coerce.
- Listen handles must survive a save. A script that stored the handle in a global is the test, not a fresh `llListen` after load.
- The hello packet's position is only exact when both sides read the same pose. The frame is the source on the Rust side. The oracle records the pose it passed to `SimChat`.

### Licences on the rows

[PORTING.md](../PORTING.md) is the rule. Phlox sources are Apache-2.0: the original notice, plus a statement that the file was changed. YEngine and `ScriptEngine.Shared` are BSD-3-Clause. Linden's SLua is MIT, and [NOTICE.md](../../NOTICE.md) already names it. A row that ports C# keeps that file's header. This plan does not edit NOTICE.md. A C shim, if question 5 asks for one, gets its notice on the row that adds the file. `SluaBackHalfProof.cs`, `SLuaCompiler.cs` and `Tests/SluaProofRunner` are read as evidence. They are not ported. The VM they exercise is the one [D10](../DECISIONS.md) does not port.

## 5. Queue items

The first slice, in this order. Each later row waits on the rows its dependency cell names.

1. **P4.1a** The crate and the script thread. No native library.
2. **P4.1b** Link the pinned `slua` inside that crate. One environment, one script, an integer result.
3. **P4.1c** The default LSL script runs `state_entry` and the host records `llSay`.
4. **P4.1d** That call is a `ChatFromSimulator` an avatar on channel 0 can see. This is the smallest useful slice.

P4.1 is four days on purpose. Binding the library and sending object chat are not one row.

| Id | Title | C# source | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| P4.1a | Crate and script thread | [ARCHITECTURE.md](../../ARCHITECTURE.md) names `ferroan-scripting`. [D6](../DECISIONS.md), [D8](../DECISIONS.md), [D10](../DECISIONS.md). The frame thread is the pattern (`frame.rs` 385–388). | The crate is a workspace member. One thread starts and joins. It does not link `slua`. No other crate depends on it. `unsafe_code` stays forbid outside the two native crates. No `unsafe` yet. | None. |
| P4.1b | Bind one environment | `Executor/include/Luau/Executor.h` 23–57, 178–218. `Script.h` 132–183. Pinned revision in [REFERENCES.md](../REFERENCES.md). | One `IEnvironment`, used on the script thread only. A Luau chunk returns an integer and the test sees that integer. The environment is freed on drop. Every `unsafe` block has a `SAFETY` comment. No scene call. No C or C++ file is added. Question 5 can replace this bind with a C shim before the row is taken. | P4.1a. Question 5 if the maintainer wants a shim instead of a Rust bind of the C++ API. |
| P4.1c | Default script reaches `llSay` | `Constants.DefaultScriptText` (`Constants.cs` 74). `DefaultScript.lsl` line 5. `DefaultLuaScript.cs` 22–26. `LSLCompiler.cpp` as [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) names it. `populateEnvironment` (`Executor.h` 193). | The source compiles to an image, `state_entry` runs, and the host record is channel 0 and the bytes `Script running`. No packet. The Lua default is the same string under `ll.Say` and is a second case on this row only if it fits the day. If it does not, it is split off before P4.1d. | P4.1b |
| P4.1d | Object say on channel 0 | Phlox `llSay` (`LSLSystemAPI.cs` 1539–1545, `HostSimChat` 670–677). YEngine `llSay` (`LSL_Api.cs` 1307–1328) as the second recording. `ChatModule.OnChatFromWorld` (189–194) passes `ChatSourceType.Object`. `SendChatMessage` (1108–1152). `ChatTypeEnum.Say` is 1 (`ChatTypeEnum.cs` 30–42). `ChatSourceType.Object` is 2 (`AgentManager.cs` 227–234). Ferroan today: `chat.rs` 35–37, 66, 228, 317. Frame pose: `chat_positions` (`frame.rs` 554–564). | A captured `ChatFromSimulator`: source type 2, chat type 1, audible 1, message bytes `Script running`, prim name, owner id, source id, position equal to the pose the frame returned. The 15 ms `ChatSleep` and the 2000 ms 11th-say sleep are not in the compare. Channels other than 0 are still not delivered to viewers. `client.chat-queue` is unchanged. | P4.1c. The frame thread. |
| P4.2 | Event queue of 64 | Phlox `RuntimeState.cs` 23, 477–501. Scheduler 74–75, 1767–1877. YEngine `MAXEVENTQUEUE` (`XMRInstMain.cs` 81) and `PostEvent` (`XMRInstRun.cs` 54–223). | One script queues events to the cap and the next is dropped or kept exactly as question 1's rule says. `on_rez`, `state_entry`, `state_exit` and `timer` are the Phlox overflow set. YEngine's per-code count and its single `timer` are the other rule. The row implements the rule question 1 names. A test posts one `listen` and the script receives it. The viewer ids `event.queue` and `client.chat-queue` are not reused. | P4.1c. Question 1. |
| P4.3 | Park and wake | Phlox sleep heap (`PhloxExecutionScheduler.cs` 1564–1589, `PhloxMasterScheduler.cs` 105–160). YEngine sleep thread (`XMREngine.cs` 1865–1916). `InterruptInstaller` (`Executor.h` 111–168). | A script that sleeps is off the run queue and back on it when the wake passes. The test injects the clock. Slice length is "parked and resumed", not 8 opcodes and not 60 ms. The millisecond table is P4.23. | P4.1c |
| P4.4 | Memory account | `kDefaultMemoryLimit` (`Executor.h` 24). `Script.h` 182–183. Phlox `MemoryInfo.cs` 26 and `MemoryCalc.cs` 30–64. YEngine heap (`XMREngine.cs` 219) as the other recording. | A script that crosses the limit stops with an error the test can see. `llGetMemoryLimit` is 131072. `llSetMemoryLimit(131072)` succeeds and any other value fails, matching Phlox. `llGetFreeMemory` reads Linden's counter. Phlox's byte count and YEngine's heap are recorded and not required to match. | P4.1b |
| P4.5 | Detect parameters | `Helpers.cs` 95–101 and `EventParams` at 323. Phlox `ConvertDetectParams` (`PhloxEngine.cs` 2477–2510), `GetDetect` (`LSLSystemAPI.cs` 5196–5211). YEngine `XMREvents.cs` 164–380. | A touch event carries one detect record. A link message carries an empty array. An out-of-range `llDetectedTouchFace` is −1 and the touch ST and UV are (−1, −1, 0). Keys and link numbers match exactly. Positions use the float rule in section 4. | P4.2 |
| P4.6 | Timers | Phlox `llSetTimerEvent` (`LSLSystemAPI.cs` 2786–2819), floor 0.1 (`PhloxEngine.cs` 147). YEngine code default 0.5 (`LSL_Api.cs` 113) and shipped ini 0.1 (`RegionServerDefaults.ini` 2469). `ScriptTimer.cs` 122–145. | Zero and negative stop the timer. A positive request below the floor becomes the floor. One `timer` is queued. The next arm happens when that event is delivered. The injected clock fires inside the poll tolerance. | P4.2, P4.3 |
| P4.7 | Listens | `PhloxListenManager.cs` 69–117, 205, 485–582, 751–778. `WorldCommModule` `TryEnqueueMessage` 687–743. Ini `max_listens_per_script` 64 and `max_listens_per_region` 1000 (`RegionServerDefaults.ini` 2159–2165). Rate default 20 per second. | Filters: channel, name, key, message. A prim does not hear itself. Caps: 64 per script, 1000 per region, 20 per second. A 65th listen is refused. The exact-range sample is excluded until question 10. Handles are stable across a save (P4.13). | P4.2. P4.1d for channel-0 delivery into the listener. |
| P4.8 | Sensors | Both `SensorRepeat` plugins, defaults 96 m and 32 hits. Phlox lines 52–53 of the Phlox plugin. Shared plugin lines 80–81. | One repeater per script. A rate of 0 removes it. `sensor` carries one detect record per hit. `no_sensor` carries none. The sweep reads the scene. It does not call physics. A later physics-backed cast is the physics family, not this row. | P4.5. Scene poses. |
| P4.9 | Link messages | Phlox `llMessageLinked` (`LSLSystemAPI.cs` 3076–3089) and the 50 ms back-pressure (305–323). YEngine `LSL_Api.cs` 5796–5838. | A message reaches every script in the targeted prim that has a handler, with an empty detect array, and does not reach a prim that was not targeted. The 50 ms pause is P4.23, because it is a number from question 2. | P4.2, P4.5 |
| P4.10 | Minimum event delay | Phlox queues inside the window (scheduler 3154–3185). YEngine drops a named set until the deadline (`XMRInstRun.cs` 86–120). | The row implements the rule question 1 pairs with the queue. A `touch` inside the window is either queued or dropped, and the test names which. | P4.2. Question 1. |
| P4.11 | Reset and state change | Phlox state change (scheduler 3082–3113). YEngine reset path in `XMRInstRun.cs` and `llResetScript` on the shared API. | `state_exit` then `state_entry`. The queue and the listens are cleared. A timer is re-armed only when the new state has `timer()`. A reset does not keep globals. | P4.2, P4.6, P4.7 |
| P4.12 | Permission grants | `llRequestPermissions` is on `ISystemAPI`. Both engines store granter and mask on the saved state (YEngine capture 92–192, Phlox `SerializedRuntimeState.cs` 21–80). | The mask and the granter survive a save. A call that requires a grant fails with the pin's error text when the mask is empty, and runs when the bit is set. The dialog packet can be a follow-on if it does not fit the day. | P4.11, P4.13 |
| P4.13 | Read `<Variables>` | `processXstate` (`XMRInstCtor.cs` 617–626, 800–1147). The field list in [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md). | A document C# wrote loads: globals by name and type, state name, running, queue, listens with their handles, timer, permissions, experience, minimum delay, start parameter. The script does not get a fresh `state_entry`. Keys stay keys. | P4.1c, P4.2 |
| P4.14 | Write `<Variables>` | The same reader, used in reverse. YEngine `SetXMLState` accepts a foreign engine when `<Variables>` is present. | YEngine's `processXstate` on the pin loads a document Ferroan wrote, and the globals, the queue and the listen handles match. Taken between events. | P4.13 |
| P4.15 | Refuse the other two formats | YEngine snapshot (`XMRInstCapture.cs` 45–217, `XMREngine.cs` 1132–1167). Phlox envelope (`PhloxEngine.cs` 1640–1714). | A `<State Engine="YEngine">` snapshot and a `<State Engine="InWorldz.Phlox" Version="1">` protobuf envelope are refused. The log names the item id and the engine. The script is not reset and is not started empty. Conversion waits on question 3. | P4.13 |
| P4.16 | OSSL gate, no functions | `OsslGate.cs` 80–121, from `OSSL_Api.CheckThreatLevel` (`OSSL_Api.cs` 180–215). | `[OSSL] AllowOSFunctions` defaults true. `OSFunctionThreatLevel` defaults to `VeryLow`, and an unknown word is `VeryLow`. `Allow_<function>` and `Creators_<function>` parse. A denied call throws and the script stops. No `os*` body is implemented on this row. Question 4 can change the default before the row is taken. The row ships the pin's `VeryLow` if the question is still open. | P4.1c |
| P4.17 | Script stats on the wire | `SimStats.cs` 52, 57, 73, 126, 135, 144. `SimStatsReporter.cs` 340–400. `Scene.AddScriptExecutionTime` (2022–2026). Phlox `llGetSimStats` (`LSLSystemAPI.cs` 16955–16968). | `ActiveScripts`, `ScriptMS` and `ScriptEps` move when a script is running and when it is not. `llGetRegionFPS` and `llGetRegionTimeDilation` follow the frame and the physics dilation already on the scene. Time dilation stays the Phase 3 field. This row does not invent a second one. | P4.1a. The frame stats that already exist. |
| P4.18 | Bounds and native-boundary fuzz | Phlox 128 KiB, queue 64, listen caps in section 1. `Executor.h` memory limit. [TESTING.md](../TESTING.md) asks for `SAFETY` comments and fuzzing on the native boundary. | Each new cap is a [LIMITS.md](../LIMITS.md) id added on this row, with a call site outside `ferroan-diag`. Fuzz rejects an oversize memory grow, a queue post past the cap, and a non-finite float argument before the native call. Existing ids are not renumbered. `client.chat-queue` stays the viewer chat queue. | P4.1b, P4.2, P4.4 |
| P4.19 | Script oracle | [D13](../DECISIONS.md). [TESTING.md](../TESTING.md) oracle pattern. The pin's shipped `DefaultScriptEngine`. | `tools/script-oracle` runs the default script on Phlox and writes the chat packet and the event list. Rust compares P4.1d against that file. The D13 edit that names the tool is part of the row's PR. This plan does not choose the decision id. Ordinary CI does not start Tranquillity. | P4.1d. The D13 edit. |
| P4.20 | Touch and collision posts | Phlox subscriptions (`PhloxEngine.cs` 1115–1156, 1229–1247, 1292–1310). YEngine `XMREvents.cs` 48–76. P3.18a produces the contact set and leaves script delivery to this phase. | A touch and a collision contact become events on the script queue with the detect data P4.5 defined. A script with no handler drops the event. No physics function is implemented. Land collision is the empty-detect case. | P4.5. P3.18a for collision. Touch needs the scene grab path. |
| P4.21 | Service-call lane | `SyscallContext.cs` 65–75. Scheduler service lane (`PhloxExecutionScheduler.cs` 189–211, 1131–1172, 3068–3077, 3480–3514). `lyieldable.h`. | A host call that yields parks the script, completes on a worker, and resumes on the script thread. The deadline and the worker do not both deliver. `FailureValue` is what the script sees when the deadline wins. Cap 6 in flight, matching `MAX_ASYNC_WORKERS`. | P4.3 |
| P4.22 | Save on stop, and original XML | Phlox `GetXMLState` timeout 10 s (`PhloxEngine.cs` 1646–1685). YEngine maintenance (`XMREngine.cs` 1970–1986). `ToOriginalXmlFormat` (serializer 174–180, 262–263). XML2 does not write state (1611–1637). OAR uses XML2 (`ArchiveWriteRequest.cs` 624). [TESTING.md](../TESTING.md) records that a loaded group has no script state. | Region stop writes `<Variables>` for every running script. The original XML of a group with a script contains one `<State>` block. A group with no script is byte-identical to today's fixture. XML2 and OAR stay without script state. A capture that misses the between-events window logs the item and writes no partial state. | P4.14 |
| P4.23 | Function sleep numbers | The table in section 1. Phlox `ScriptSleep` (241–249). YEngine `ScriptDelayFactor` (`LSL_Api.cs` and `RegionServerDefaults.ini` 2456–2457). | The numbers question 2 names: chat, `llSetPos` and `llSetRot`, email, IM, link back-pressure, notecard, reset storm. An injected clock. The other engine's numbers are recorded and not required to match. | P4.3. Question 2. |

### Stage-2 placeholders

One row per family when stage 2 is written. The count is the method count from section 2. The C# bodies are `LSLSystemAPI.cs` for `ll*`, `os*` and `bot*`, with `OsslGate.cs` on every `os*`. YEngine's `LSL_Api.cs` and `OSSL_Api.cs` are the second recording where question 2 says so. This table does not plan a function.

| Id | Family | Count | Waits on |
| --- | --- | --- | --- |
| P4.S1 | `ll` math, strings and crypto | 64 | P4.1c |
| P4.S2 | `ll` lists and JSON | 32 | P4.1c |
| P4.S3 | `ll` communication, beyond the channel-0 say in P4.1d | 20 | P4.1d, P4.7. Email and IM also wait on P4.21 and P4.23. |
| P4.S4 | `ll` link messages and linkset data | 14 | P4.9 for `llMessageLinked`. Linkset data waits on Phase 5. |
| P4.S5 | `ll` HTTP, XML-RPC and experience | 29 | P4.21. Experiences wait on Phase 5. |
| P4.S6 | `ll` inventory, rez and notecards | 33 | Inventory items from Phases 1 and 2. Transfer and give wait on Phase 5. |
| P4.S7 | `ll` land, estate, region and environment | 64 | Phase 5, except a ground-height read that the frame already answers. |
| P4.S8 | `ll` physics-backed | 66 | Phase 3 rows P3.18a, P3.26 and P3.46, plus P4.20 for the events those calls cause. |
| P4.S9 | `ll` avatars, animation, camera, permissions, money | 61 | P4.12. Money waits on Phase 5. |
| P4.S10 | `ll` sound and media | 34 | Phase 5 for media and sound. |
| P4.S11 | `ll` sensors and detect | 21 | P4.5, P4.8 |
| P4.S12 | `ll` object, prim and script control | 113 | The scene. Physical `llSetPos` also waits on Phase 3. Sleep and timer mechanisms are P4.3 and P4.6. |
| P4.S13 | `os` math, strings, lists, crypto | 47 | P4.16 |
| P4.S14 | `os` avatars, teleport, attach, groups | 40 | P4.16. Groups wait on Phase 5. |
| P4.S15 | `os` land, estate, region, sun, terrain | 35 | P4.16. Phase 5. Includes `osRegionNotice`. |
| P4.S16 | `os` drawing | 31 | P4.16. Phase 5 (dynamic textures). |
| P4.S17 | `os` NPC | 26 | P4.16. Phase 5. |
| P4.S18 | `os` physics, wind, health | 22 | P4.16. Phase 3. |
| P4.S19 | `os` inventory and notecards | 21 | P4.16. Phase 5 for the transfer. |
| P4.S20 | `os` sound | 13 | P4.16. Phase 5. |
| P4.S21 | `os` object, prim, sit, particles, script engine | 33 | P4.16. The scene. |
| P4.S22 | `os` grid and simulator queries | 10 | P4.16 |
| P4.S23 | `os` time and creation stamps | 8 | P4.16 |
| P4.S24 | `os` HTTP | 3 | P4.16, P4.21 |
| P4.S25 | `os` communication | 2 | P4.16, P4.7 |
| P4.S26 | `bot` | 58 | P4.1c for the table. Bodies wait on Phase 5. Viewer-visible once a bot is an agent. |

`iw*` (85) has no row. Question 6 adds one, or it stays out. `ls*` (4) and `mod*` (9) have no row. [D10](../DECISIONS.md) leaves them on YEngine.

## 6. Questions

None of these is decided. No decision id is reserved. A row that names a question waits, or ships the pin's current value and says so.

1. **Event queue accounting.** Phlox keeps one queue of 64. `on_rez`, `state_entry`, `state_exit` and `timer` may pass it. Another script's answer is dropped at 32. YEngine keeps 64 per event code, at most one `timer`, and `llMinEventDelay` drops a named set instead of queueing it. P4.2 and P4.10 implement whichever rule is chosen. The viewer caps in [LIMITS.md](../LIMITS.md) are not this queue.

2. **Which host is the oracle for numbers.** Sleeps, the chat throttle, shout reach, and the timer floor when the ini key is absent. Phlox sleeps 15 ms on every throttled say and does not broadcast shout. YEngine sleeps 2000 ms on the 11th channel-0 say or shout in a one-second window, and broadcasts shout. Shipped timer floors are 0.1 s on both engines. YEngine's code default is 0.5 s. [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md) says `LSLSystemAPI` stays. Hypergrid visitors still arrive with YEngine state. P4.23 and the shout case of P4.S3 wait on this. P4.1d does not.

3. **YEngine snapshots and Phlox protobuf, both ways.** The `<Variables>` form is already required by [PLAN.md](../PLAN.md) and by [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md). P4.13 and P4.14 do that form. P4.15 refuses the other two. Loading a YEngine base64 snapshot, or a Phlox `SerializedRuntimeState`, in either direction, is this question. A script that does not convert stays on a C# region, which is the plan's current rule.

4. **OSSL threat-level default.** The pin ships `VeryLow` (`OsslGate.cs` 96–108). `AllowOSFunctions` defaults true. P4.16 copies that. This question is whether Ferroan's default stays `VeryLow`.

5. **The C interface: source and licence.** Pin8 has no C API in front of a Luau library. The bind target is the C++ executor at `slua` `a02d459aa0cb64a4c2b3dd4d841e67ba4a01d9b1` (MIT, [REFERENCES.md](../REFERENCES.md)). [D1](../DECISIONS.md)'s C shim is future Tranquillity work described in [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md), not a file at the pin. P4.1b links that C++ API from Rust and adds no C file. This question is whether a C shim is required instead, whether that shim is compiled into the SLua library or shipped beside it, and what licence it carries. [D13](../DECISIONS.md) wants Ferroan code in Rust. A C shim would be part of the native library under [D8](../DECISIONS.md), and [PORTING.md](../PORTING.md) would have to allow its licence. [NOTICE.md](../../NOTICE.md) already names Linden's engine. A shim needs its own notice line on the row that adds it.

6. **`iw*` (85 methods).** They are on `ISystemAPI` and not in [D10](../DECISIONS.md)'s list. Stage 2 either gains a family row or leaves them out. `ls*` and `mod*` stay out under D10. That part is not a question.

7. **Viewer compile targets.** `luau` and `lsl-luau` select no engine at the pin, because `AcceptsCompileTarget` defaults to false and YEngine does not implement the interface. The question is what Ferroan does with an item that carries one of those flags before a native engine accepts it.

8. **Floats and keys.** Already open in [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md). YEngine's doubles, Linden's 32-bit floats, and key versus string on the `<Variables>` round trip. P4.13 records the conversion it applies. This question is whether that conversion is allowed to change a value a script can observe.

9. **A script only YEngine can compile.** `//YEngine:` and the language extensions. Already open in [SLUA_TRANQUILLITY.md](../SLUA_TRANQUILLITY.md). [D10](../DECISIONS.md) does not run YEngine. The question is the operator-visible result: leave the script unloaded, or report it and keep the C# region as the host. The plan's current sentence is that a script whose state does not convert stays on a C# region.

10. **Listen distance at the exact range.** Phlox treats the boundary as outside. The viewer chat path still delivers. YEngine's `WorldComm` does not hear it. P4.7 excludes that sample until this is answered.
