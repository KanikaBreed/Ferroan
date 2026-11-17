# SLua engine in Tranquillity

Decision [D1](DECISIONS.md): Linden Lab's SLua engine runs in today's C# Tranquillity first, replacing Phlox's VM and YEngine, while Ferroan is built. Ferroan's Phase 4 later reuses this work's C interface and conformance suite. This work happens in **Tranquillity's repository**, not this one: branch from upstream `develop` in a fork, and send PRs to `OpenSim-NGC/OpenSim-Tranquillity`. Tranquillity's code is BSD/MPL, so nothing from Ferroan (GPL) may be copied into it.

**End state:** one engine runs LSL, SLua, OSSL (as an `ossl` table) and InWorldz bot functions (as a `bot` table). Phlox's compiler and VM (`Source/InWorldz.Phlox`, about 77,000 lines) and YEngine (about 45,700 lines) are removed after a transition. Phlox's host layer (`Source/Phlox.ScriptEngine`, about 37,400 lines) stays and is bound to the new engine. Scripts restart once, because Phlox and YEngine state does not carry over.

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

### Later packages

| Package | Scope | Done when |
| --- | --- | --- |
| S2 Events and scheduling | Posted events and detected data through `callEventHandler`; LSL state changes; SLua `LLEvents` and `LLTimers`; Executor driven from `PhloxExecutionScheduler`, one environment per scheduler thread; memory limits and faults; listens, sensors, HTTP and dataserver | Phlox's event-order and scheduler tests pass |
| S3 State and lifecycle | Ares blobs in `StateManager`'s store, tagged with format version; crossings and teleports; OAR save and load; old Phlox/YEngine state resets the script with a log line; bytecode cache by source hash and engine version; resets and recompiles | State survives crossings, restarts, take and rez, and OAR round trips |
| S4 Conformance and performance | Phlox's tests (5,703) against both engines; Linden's golden fixtures; a real-script corpus on both engines; benchmarks; fuzzing the C interface | Parity report with no unexpected regressions; frame time no worse than Phlox |
| S5 Rollout | `DefaultScriptEngine = "SLua"` with per-region fallback to Phlox; a scanner for YEngine extensions and compile failures; migration guide | Two beta releases with no blocking issue |
| S6 Removal | Delete Phlox's compiler, VM and serialization, YEngine and its parts of `ScriptEngine.Shared` | Done |

## Open questions to settle early

- SL's memory limit per script for each language, and whether to match it or keep Executor's 128 KB default.
- How SL viewers mark a script as SLua when they upload it, so Tranquillity can honour it; fall back to Phlox's `--!slua` rule.
- Which platforms beyond Linux x64 and Windows x64 need the native library.
