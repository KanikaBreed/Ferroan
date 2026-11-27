# RAW32 oracle

A .NET 10 console program. It writes a RAW32 member, packs it into a one-region OAR, and runs Pin8 `HandleLoadOarConsoleCommand` on a scene named `Raw32`. It also runs `RAW32.LoadStream` and the section `LoadFile`. Build it against a scratch clone of the pin, the same way `oracle_reference::prepare` does. Leave the reference checkout untouched.

The committed capture is [fixtures/oar-raw32](../../fixtures/oar-raw32/README.md). `cargo xtask raw32 verify` rebuilds this program against a task-owned clone of the pin and diffs a fresh run with that directory.

```sh
dotnet build tools/raw32-oracle /p:FerroanRef=.scratch/oracle-sources /p:UseSharedCompilation=false
```

Run the built program from `tools/raw32-oracle/bin/Debug/net10.0`, so `OpenSim.Tests.Common.dll` loads, under the shared live lock with a free port base and `UseSharedCompilation=false`. Pass an empty output directory. The run prints `load oar matches LoadStream`.
