# Walking step fixture

`basicphysics-steps.json` is one `BasicScene.Simulate` trace at pin `c401bceecd`. The step is `FrameTime` 0.0909. The avatar box is 2.1.

`tools/walk-oracle` checks that file. It parses each number as `float` and compares `Simulate` bit for bit. It does not rewrite the file, and `cargo xtask check` does not build it.

```
dotnet run --project tools/walk-oracle -- verify fixtures/walk/basicphysics-steps.json
```

Set `FerroanRef` to the reference checkout when it is not `~/Git/reference`. The oracle compiles against `OpenSim.Region.PhysicsModules.SharedBase.dll` in that project's `bin/Debug/net10.0` directory. Building BasicPhysics writes it.

`flat_walk`, `fall`, `fly_hold` and `fly_snap` use a 256 m region and flat ground 21. `border` and `outside` use a 16 m region and the same ground. `slope` uses a 16 m region whose cell (2, 3) is 30 and whose other cells are 21. `no_map` has no heightmap: it starts at (4, 5, 26) with no velocity and lands at the single stored row. A `*_flying` key is 1 when that case is flying. The first row of each other case is the actor before `Simulate`. Column 7 of a later row is ground contact.

A kinematic sample may finish outside the region. The presence tick's no-neighbour stop is not part of this file.

# Live walk capture

`walk.cap` is the home region's UDP from one `cargo xtask live run walk grid --physics basicphysics` at pin `c401bceecd`, on a scratch build with the capture patch. The run printed `walk: pass (128.0,128.0,26.0 to 134.0,128.3,26.9)`. The raw file passed `ferroan-capture check` (94 records) and was then `ferroan-capture redact`ed: session, agent and circuit ids are the placeholders, and names are `Redacted` ([CAPTURES.md](../../docs/CAPTURES.md)). The raw file is not stored.

It holds the walker's full avatar `ObjectUpdate` (local id 850050570 at 128, 128, 25.96), the `LayerData` burst, 24 forward `AgentUpdate` packets 100 ms apart and the release, and three `ImprovedTerseObjectUpdate` blocks to the walker: the start (tick 4 after the first forward update), the first tick more than 5 m past it on X (tick 27), and the delayed stop (tick 31).

`terrain.bin` is that region's stored heightmap: the `Heightfield` of its `terrain` row in `OpenSim.db` (revision 23, 64,634 bytes, the default `pinhead-island`). BasicPhysics reads the same cells.

`the_csharp_walk_matches_the_stand_in_and_the_terse_thresholds` (`crates/ferroan-scene/src/walk_capture.rs`) replays the inputs through the stand-in and the `ScenePresence.Update` thresholds. X, Y and Z match each block bit for bit, and velocity and rotation match after the avatar block's 16-bit packing. The send ticks match. The C# frame clock is not in the capture: tick `k` is placed at the first block's arrival plus `(k - 4) * 90.9` ms, every block falls on it within half a frame, the first forward update falls between ticks 0 and 1, and the release deadline falls between ticks 30 and 31. This heartbeat ran about 90.4 ms frames. The first frame that runs the thresholds is the one after `SendInitialData`, because `ScenePresence.Update` returns early until then. On a login `SendInitialData` sends no terrain (`ScenePresence.cs` line 4338 needs `TeleportFlags` 0): the `LayerData` burst is `CompleteMovement`'s push (line 2472), drained by the terrain module's every-fourth-frame check (`Scene.cs` 1768–1771). On this recording that check ran in the `SendInitialData` frame (region log 23:26:36,005, first `LayerData` 36,017), so the test uses the first `LayerData` as the marker for that frame. A new recording may not line up.

To record it again, under the live lock with a free port base:

```
FERROAN_LIVE_PORT_BASE=23600 cargo xtask live run walk grid --physics basicphysics
cargo run -p ferroan-capture -- check .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap
cargo run -p ferroan-capture -- redact --in .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap --out fixtures/walk/walk.cap
sqlite3 .scratch/live/runs/<base>-<pid>/grid/home/OpenSim.db "select writefile('fixtures/walk/terrain.bin', Heightfield) from terrain"
```

A new recording moves the arrival times and may move the start tick. Update the tick and count assertions in the test with it.
