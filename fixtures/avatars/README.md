# Avatars capture

`avatars.cap` is the home region's UDP from one `cargo xtask live run avatars grid` at pin `c401bceecd`, on a scratch build with the capture patch. The run printed `avatars: pass (each bot saw the other's local id and appearance; KillObject on leave)`. The raw file passed `ferroan-capture check` (131 records) and was then `ferroan-capture redact`ed: session and circuit ids are the placeholders, names are `Redacted`, and the sound `OwnerID` of each avatar block is the agent placeholder ([CAPTURES.md](../../docs/CAPTURES.md)). The avatars' full ids and the appearance sender ids are the harness accounts' fixed ids (`ESTATE_OWNER` and `CHAT_USER` in `xtask/src/live.rs`). The raw file is not stored.

It holds, in order: the owner's arrival, its own full avatar `ObjectUpdate` (local id 258129971), `AvatarAppearance` and `AvatarAnimation`; the second bot's arrival and its full update (local id 258129972) and appearance on both circuits (`CompleteMovement`); the second bot's full update and appearance again on the owner's circuit, when the owner's `SendInitialData` ran after the second bot had arrived; the owner's full update and appearance on the second bot's circuit at its `SendInitialData`; the two logouts; and `KillObject` for 258129972 on both circuits after the second bot's `LogoutReply` (TB87). Full updates and appearances are reliable and zero-coded. The kills are reliable and not zero-coded. The second bot arrives at Z 27.96, two metres above the owner: `MakeRootAgent`'s landing ray cast (`ScenePresence.cs` lines 1543–1602, with `PrimsNonPhantomAgents` on ubODE) hits the owner's capsule and stacks the newcomer on it. Ferroan's walking stand-in has no ray cast, so the compare masks Z: relative Z is that missing avatar ray cast (a physics gap), and absolute Z is the missing terrain row in the Ferroan home store. Once physics has landed an avatar, C# writes its collision plane into later full updates.

`cargo xtask live run avatars grid --region ferroan` compares the Ferroan run with this file (`xtask/src/avatars_capture.rs`), and `the_csharp_capture_has_both_avatars_and_the_kill` checks the file itself.

To record it again, under the live lock with a free port base:

```
FERROAN_LIVE_PORT_BASE=24800 cargo xtask live run avatars grid
cargo run -p ferroan-capture -- check .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap
cargo run -p ferroan-capture -- redact --in .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap --out fixtures/avatars/avatars.cap
```

Read the redacted file before committing it.
