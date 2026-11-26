# Animations capture

`animations.cap` is the home region's UDP from one `cargo xtask live run animations grid --physics basicphysics` at pin `c401bceecd`, on a scratch build with the capture patch (run `27040-2939964`). The run printed `animations: pass (the observer saw STAND 1, WALK 1920781, STAND 1920782, the started animation at 1920783 and its stop)`. The raw file passed `ferroan-capture check` (188 records, replayed) and was then `ferroan-capture redact`ed: session and circuit ids are the placeholders and names are `Redacted` ([CAPTURES.md](../../docs/CAPTURES.md)). `AvatarAnimation`'s `Sender.ID` is not an identity field the redactor changes; it is the harness accounts' fixed ids (`ESTATE_OWNER` and `CHAT_USER` in `xtask/src/live.rs`). The raw file is not stored.

The harness owner logs in and walks; the second account (`Chat Bot`) watches (`tools/bot/src/scenario/animations.rs`). The owner's `AvatarAnimation` packets, in order, each once to its own viewer and once to the watcher:

1. `STAND` sequence 1: to the owner from its `CompleteMovement`, and to the watcher at its `SendInitialData` (`SendOtherAgentsAvatarFullToMe`).
2. `WALK` 1920781 on the first forward `AgentUpdate` (`AddNewMovement`).
3. `STAND` 1920782 after the release, on a `RegionHeartbeatEnd` update.
4. `STAND` 1920782 and the started animation 1920783, 10 ms after the owner's `AgentAnimation` start (`HandleAgentAnimation`, `avnChangeAnim` with no object).
5. `STAND` 1920782 alone after its `AgentAnimation` stop.

Every packet is reliable and not zero-coded, with no extra header and no `AnimationSourceList` or `PhysicalAvatarEventList` entries: each animation names the agent or nothing, so `SendAnimations` gives the owner's own viewer no objects either. The capture also holds the watcher's own `STAND` 1 packets, which the compare does not read.

`cargo xtask live run animations grid --region ferroan` compares the Ferroan run with this file (`xtask/src/animations_capture.rs`): both captures must re-encode as they are, then the owner's packs, each run of equal packs folded with its length, must match byte for byte, headers included, once every sequence number but 1 is written relative to the run's smallest (C# starts them from a clock, `NextAnimationSequenceNumber`). `the_csharp_capture_has_the_five_steps_to_both_viewers` checks the file itself.

To record it again, under the live lock with a free port base:

```
FERROAN_LIVE_PORT_BASE=27040 cargo xtask live run animations grid --physics basicphysics
cargo run -p ferroan-capture -- check .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap
cargo run -p ferroan-capture -- redact --in .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap --out fixtures/animations/animations.cap
```

Read the redacted file before committing it.
