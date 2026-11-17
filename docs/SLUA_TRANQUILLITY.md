# SLua engine in Tranquillity

Decision [D1](DECISIONS.md): Linden Lab's SLua engine runs in today's C# Tranquillity first, replacing Phlox's VM and YEngine, while Ferroan is built. Ferroan's Phase 4 later reuses this work's C interface and conformance suite. This work happens in **Tranquillity's repository**, not this one: branch from upstream `develop` in a fork, and send PRs to `OpenSim-NGC/OpenSim-Tranquillity`. Tranquillity's code is BSD/MPL, so nothing from Ferroan (GPL) may be copied into it.

**End state:** one engine runs LSL, SLua, OSSL (as an `ossl` table) and InWorldz bot functions (as a `bot` table). Phlox's compiler and VM (`Source/InWorldz.Phlox`, about 77,000 lines) and YEngine (about 45,700 lines) are removed after a transition. Phlox's host layer (`Source/Phlox.ScriptEngine`, about 37,400 lines) stays and is bound to the new engine. **Running scripts move with their state** (globals, current state, queued events, timers, listens, permissions), so a no-mod script with configuration in its globals keeps working with no action from its owner. A script whose state cannot be moved exactly stays on the engine it runs on, and is reported; it is never reset to make it fit.

## The seam

Phlox's VM reaches the server only through two pieces, and Linden's engine plugs in at the same point:

| Piece | File | Role |
| --- | --- | --- |
| `ISystemAPI` | `Source/InWorldz.Phlox/Glue/ISystemAPI.cs` | Signatures of all 831 `ll`, `os` and `bot` functions (536, 262 and 59) |
| `SyscallShim` | `Source/InWorldz.Phlox/Glue/SyscallShim.cs` | Moves arguments between Phlox's VM stack and `ISystemAPI` |
| `SyscallContext` | `Source/InWorldz.Phlox/Glue/SyscallContext.cs` | Function delays (`AddDelay`) and deferred service calls, matched by sequence number |
| `LSLSystemAPI` | `Source/Phlox.ScriptEngine/LSLSystemAPI.cs` | The 22,000-line implementation; unchanged |
| Scheduler | `Source/Phlox.ScriptEngine/PhloxExecutionScheduler.cs` | Time slices, script status, syscall returns |
| State store | `Source/Phlox.ScriptEngine/StateManager.cs` | Script state in a per-region SQLite store (protobuf today) |
| OSSL permissions | `Source/Phlox.ScriptEngine/OsslGate.cs` | Threat-level checks per OSSL call |
| Existing SLua work | `Source/InWorldz.Phlox/SLua/`, `Source/Phlox.ScriptEngine/SluaBackHalfProof.cs`, `Tests/SluaProofRunner` | Phlox's current SLua subset compiler; read it, then replace it |

On Linden's side (`~/Git/reference/slua` at the revision in [REFERENCES.md](REFERENCES.md)):

| Piece | File | Role |
| --- | --- | --- |
| Executor | `Executor/include/Luau/Executor.h`, `Script.h` | Environments, images, scripts; `HostCallbacks.populateEnvironment` registers host functions; `Script::callEventHandler`, `resumeEventHandler`, `serializeState`, `restoreState`; default memory limit 128 KB; one thread per environment at a time |
| LSL compiler | `Compiler/src/LSLCompiler.cpp` | LSL to Luau bytecode through Tailslide; every built-in call imports from the `ll` table and asserts an `ll` prefix (near lines 1694 and 2069) |
| Built-ins | `builtins.txt`, `LSLBuiltins/` | SL's 528 `ll` functions, 43 events and constants, generated from Linden's lsl-definitions |
| SL types | `VM/include/llsl.h` | vector (native), rotation and key as tagged userdata, lists as tables |
| Yielding host functions | `VM/include/lyieldable.h` | How a C function yields and resumes |
| VM-side `ll` functions | `VM/src/lll.cpp` | 39 functions implemented inside the VM |

## Three engines during the transition

Tranquillity already runs Phlox and YEngine side by side ([PhloxSetup.md](https://github.com/OpenSim-NGC/OpenSim-Tranquillity/blob/develop/Docs/PhloxSetup.md)): `DefaultScriptEngine` picks the engine for new scripts, a script with saved state stays on the engine that holds it, a `//YEngine:` header always picks YEngine, and chat, link messages, `dataserver` and other events cross between the engines. The SLua engine joins as a third engine under the same rules:

| Engine | Runs during the transition |
| --- | --- |
| SLua (new default) | New scripts, every script that is reset or recompiled (it has no state to lose), and stateful scripts the migrator has moved |
| Phlox | Scripts it holds state for, until they are moved |
| YEngine | The same, and anything that cannot move: YEngine's language extensions, scripts with an explicit `//YEngine:` header (unless the grid opts in), and scripts whose state does not convert |

- The SLua engine registers under its own engine name (`SLua`), so the header and routing rules work unchanged. Inside, it is Phlox's host layer (`Phlox.ScriptEngine`: state store, scheduler, `ISystemAPI`) with Linden's VM in place of Phlox's.
- **Fallback chain.** Today a script the default engine cannot compile is an error. With SLua as the default, a reset YEngine-extension script would break, so add an ordered `[Startup] FallbackScriptEngines` (for example `"InWorldz.Phlox, YEngine"`): try the default, then each fallback, and log which engine took the script.
- **One owner per script.** A move is stop on the old engine (after the current event ends), export, import, start on the new engine, confirm, then release the old engine's copy. Two engines never run one script.
- **Events between engines.** Phlox already delivers its events to YEngine scripts. The SLua engine does the same in both directions; Phlox and SLua share the host, and YEngine's delivery is checked to reach SLua scripts.
- **Phlox's own SLua.** Scripts on `//InWorldz.Phlox:slua` (Phlox's subset compiler) move to Linden's engine, which is SL's reference, through the same state conversion.
- Phlox and then YEngine are disabled per region when the migration audit shows nothing left on them that should move.

## Moving running scripts

### Portable script state

One documented, versioned, engine-independent format, which every engine exports to and imports from, instead of a converter per pair of engines. Ferroan reads and writes it too (the cut-over in its Phase 7 depends on it), so it is defined as LLSD with a format version, in a spec both projects follow.

A script's portable state is: engine of origin, script item and asset IDs, source hash; current state name; running flag; globals by name and LSL type (integer, float, string, key, vector, rotation, list with typed elements); queued events with their parameters and detected data; the timer's interval and time left; listens with their original handles, filters, on/off state and regex flags; repeating sensors; permissions (granter and mask); experience key; minimum event delay; start parameter. Only a script **between events** has portable state; a script asleep, looping or blocked in a call is exported once its event ends.

Mapping, per engine:

- **Globals by name.** The server holds every script's source, no-modify included, so it can always recompile to recover names. YEngine stores globals in typed arrays by index and keeps the names in `ScriptObjCode.globalVarNames`; Phlox stores them by index and its compiler's symbol table has the names; Linden's LSL compiler turns LSL globals into Luau globals set by name (`LOP_SETGLOBAL` on the name's hash in `LSLCompiler.cpp`), and state changes go through the host (`lsl.change_state`), so the host can set both on import.
- **Precedent.** YEngine already imports another engine's state this way: `processXstate` (`XMRInstCtor.cs`) reads XEngine's `<ScriptState>` with named `<Variables>`, the state name, running flag, queue, plugins (listens and timers) and permissions.
- **Exactness.** Listen handles keep their numbers (scripts store them in globals). YEngine holds floats as doubles where SL holds 32-bit floats: convert as SL rounds, and record it. Keys stay keys, distinct from strings. List element types are kept.

### Seamless migration

For each stateful script on Phlox or YEngine, the migrator:

1. **Compile check:** compiles the source with the patched LSL compiler (or as SLua). A script that does not compile stays.
2. **Function check:** scans the compiled script for functions on a known-risk list (behaviour that differs between engines, functions not yet bound, engine quirks a script may rely on, such as YEngine's `ESTATE_ACCESS_*` values). A script that uses one stays until the list says otherwise.
3. **Round trip, before anything changes in-world:** exports portable state, loads it into the SLua engine without running an event, exports it again, and requires the two to match; checks memory use leaves headroom under the limit.
4. **Switch at a quiet point:** between events, preferably at region start, when scripts load from saved state anyway. The script carries on; it gets no `state_entry` and does not see the move.
5. **Rollback window:** keeps the old engine's state for a set time. A script that raises a run-time error on SLua that it never raised before is moved back, with its current state converted back, and reported. The format therefore converts both ways from the start.

Operators run it per region: `script migrate audit` (a dry run listing every script as moves cleanly, or stays and why), `script migrate run`, `script migrate rollback`. The audit comes first, on real grid data, before any engine switch.

## Hypergrid

The script asset is source text; the engine is chosen by the region that runs it. Saved state travels inside the serialized object (`<ScriptState Engine="...">`), with attachments on teleports and crossings.

**Leaving for a grid without the SLua engine**

| Script | What happens there | What we do |
| --- | --- | --- |
| LSL running on SLua at home | The source is LSL, so YEngine or XEngine there compiles and runs it | Write the state for LSL scripts in the named-`<Variables>` form `processXstate` reads. Both core OpenSim's YEngine and Tranquillity's import any foreign `<ScriptState>` that has `<Variables>`, by global name, and ignore elements they do not know, so the SLua engine's own data can ride in the same envelope. The script carries on there, from its state, instead of restarting |
| SLua source | No engine there runs it: it fails to compile and the object travels without a running script (a Tranquillity grid with Phlox may run it with Phlox's subset compiler). Content written in SLua is grid-specific, as content that uses OSSL functions is | Keep a travel copy of its portable state at home, in a grid-level store keyed by item, written when the object leaves the grid. If it comes back with no state we can use, it resumes from the travel copy as at departure, instead of restarting |
| Uses `ossl` or `bot` functions | The same as today: depends on the destination's OSSL settings | Nothing new |

**Arriving from other grids**

- While YEngine is loaded, a visitor's YEngine scripts run on it, as today (the saved-state routing rule).
- The importer reads YEngine's own snapshot and XEngine's `<Variables>` form, so a visitor's scripts, and our own objects coming home from a trip, move onto the SLua engine with their state once YEngine is retired.
- **YEngine's state reader and the `<Variables>` writer stay after YEngine is removed**, because visitors keep arriving with YEngine state. Visitors' scripts that use YEngine's language extensions cannot run once YEngine is gone; decide before removing it whether that is acceptable or whether YEngine stays loaded for visitors only.
- Ferroan follows the same rules (its Phase 4).

All of this is checked on the live harness: an attachment with an LSL script on SLua teleports to a core OpenSim region and back, and keeps its globals both ways.

## Work packages

Each package is one branch and one PR in the Tranquillity fork. Run Tranquillity's test suites before and after, and compare the failing lists against `develop`, since `develop` has known failures.

### S0.1 Build Linden's engine

- Fork `secondlife/slua`, pin the revision in REFERENCES.md, and add it as a submodule.
- Build the VM, compilers, LSL built-ins and Executor as one shared library for Linux x64 and Windows x64 with CMake, in CI. Ship it as a native NuGet package, as `JoltPhysics.Native` is shipped.
- Run its own harness (`slua-harness`) on a sample LSL script and a sample SLua script, and record the output.
- **Done when:** CI builds the library on both platforms, and the harness runs both samples.

### S0.2 C interface

- Executor is a C++ API (classes, `shared_ptr`), so .NET needs a C layer: create and free environments, images and scripts; run, resume and abort handlers; save and restore state; register host functions; read and push the Lua values the bindings need.
- Write the header first and get it reviewed before implementing it.
- **Done when:** the header is reviewed, and a C test program drives a script through it.

### S0.3 .NET wrapper and first call

- P/Invoke, safe handles, `UnmanagedCallersOnly` callbacks for host functions, in a new project beside `Phlox.ScriptEngine`.
- **Done when:** an LSL script calls `llOwnerSay` from C# through Linden's engine, in tests on Linux and Windows. Also time an empty host call, to size the call-overhead risk.

### S1.1 Generated bindings

- A generator reads `ISystemAPI` and emits a `LuauSyscallShim` (read arguments off the Lua stack, call `ISystemAPI`, push the result), the way `SyscallShim` does for Phlox's VM.
- It also emits `builtins.txt` entries for the 262 `os`, 59 `bot` and about 19 OpenSim-only `ll` functions.
- Marshal integer, float, string, key, vector, rotation and list, including mixed lists.
- **Done when:** all 831 functions can be called from LSL and from SLua, with a round-trip test per argument type.

### S1.2 Compiler patch and tables

- Patch the LSL compiler to send `os*` calls to an `ossl` table and `bot*` calls to a `bot` table. Keep it to a few lines, and propose an upstream hook for extra built-in tables to Linden.
- Register `ll`, `ossl` and `bot` in `populateEnvironment`, with `OsslGate` checking each OSSL call. Do not use `os`, which is Luau's standard library.
- **Done when:** LSL and SLua scripts call an OSSL and a bot function, and a denied OSSL call fails as it does under Phlox.

### S1.3 Delays and deferred calls

- Turn `SyscallContext` delays (for example `llSetPos`'s 0.2 seconds) into yields.
- Turn long-running and deferred service calls into yieldable host functions, resumed with their result only when the call's sequence number still matches.
- **Done when:** Phlox's syscall and sleep tests pass on the new engine.

### Migration track (can start now)

These need no part of Linden's engine until M3, and each one is safe to merge on its own: M1 and M2 change no behaviour.

| Package | Scope | Done when |
| --- | --- | --- |
| M1 Portable state and audit | The format spec (LLSD, versioned); a YEngine exporter (snapshot plus `globalVarNames`, recompiling the source when needed); `script migrate audit` reporting, for every script in a region, its engine, whether its state exports exactly, and why not | Exports of a script corpus re-import into YEngine with equal globals, queue, timer and listens; the audit runs on a copy of a real grid's state with no change to it |
| M2 Phlox both ways | Phlox exporter and importer for the portable format; YEngine importer (the inverse of M1) | YEngine to portable to Phlox and back gives equal state for the corpus; scripts resume on Phlox with no `state_entry` |
| M3 SLua both ways | After S2: the SLua engine imports and exports portable state (globals by name, state through the host) | Corpus round trips through SLua; Phlox's state tests pass on the SLua engine |
| M4 Migrator | Three engines; `FallbackScriptEngines`; one owner per script during a move; compile check, risk list, round trip, quiet-point switch and rollback window; `script migrate run` and `rollback` | A region migrates its scripts with no `state_entry` for any moved script; rollback restores the old engine with current state |
| M5 Hypergrid | `<Variables>`-form state for LSL scripts on SLua; the grid-level travel copy for SLua scripts; arrivals with YEngine or XEngine state | On the live harness, an attachment with an LSL script on SLua teleports to a core OpenSim region and back, keeping its globals both ways |

### Later packages

| Package | Scope | Done when |
| --- | --- | --- |
| S2 Events and scheduling | Posted events and detected data through `callEventHandler`; LSL state changes; SLua `LLEvents` and `LLTimers`; Executor driven from `PhloxExecutionScheduler`, one environment per scheduler thread; memory limits and faults; listens, sensors, HTTP and dataserver; registration as the `SLua` engine beside Phlox and YEngine | Phlox's event-order and scheduler tests pass. New scripts can run on SLua with the other two engines loaded |
| S3 State and lifecycle | Ares blobs in `StateManager`'s store, tagged with format version, plus portable state (M3) for moves between engines; crossings and teleports; OAR save and load; bytecode cache by source hash and engine version; resets and recompiles. No script is reset because its state came from another engine | State survives crossings, restarts, take and rez, and OAR round trips |
| S4 Conformance and performance | Phlox's tests (5,703) against both engines; Linden's golden fixtures; a real-script corpus on both engines; benchmarks; fuzzing the C interface | Parity report with no unexpected regressions; frame time no worse than Phlox |
| S5 Rollout | `DefaultScriptEngine = "SLua"` with the fallback chain; migration in waves per region (M4), starting from the audit; operator and migration guide | Two beta releases with no blocking issue, and audits on beta grids showing what stays behind and why |
| S6 Removal | Delete Phlox's compiler, VM and serialization, then YEngine and its parts of `ScriptEngine.Shared`, per region only when the audit shows nothing left that should move. Keep YEngine's state reader and the `<Variables>` writer for hypergrid (see Hypergrid) | Done |

## Open questions to settle early

- SL's memory limit per script for each language, and whether to match it or keep Executor's 128 KB default.
- How SL viewers mark a script as SLua when they upload it, so Tranquillity can honour it; fall back to Phlox's `--!slua` rule.
- Which platforms beyond Linux x64 and Windows x64 need the native library.
- How SL rounds LSL floats on its Luau VM, so converted YEngine doubles match.
- Whether `//YEngine:` headers migrate (grid setting; off by default), and whether YEngine stays loaded for hypergrid visitors after the migration.
- That core OpenSim regions hand a foreign `<ScriptState>` from an arriving attachment to YEngine unchanged (checked on the live harness before M5 relies on it).
