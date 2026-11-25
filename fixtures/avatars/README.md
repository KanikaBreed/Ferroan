# Avatars capture

`avatars.cap` is the home region's UDP from one `cargo xtask live run avatars grid` at pin `c401bceecd`, on a scratch build with the capture patch. The run printed `avatars: pass (each bot saw the other's local id and appearance; KillObject on leave)`. The raw file passed `ferroan-capture check` (131 records) and was then `ferroan-capture redact`ed: session and circuit ids are the placeholders, names are `Redacted`, and the sound `OwnerID` of each avatar block is the agent placeholder ([CAPTURES.md](../../docs/CAPTURES.md)). The avatars' full ids and the appearance sender ids are the harness accounts' fixed ids (`ESTATE_OWNER` and `CHAT_USER` in `xtask/src/live.rs`). The raw file is not stored.

It holds, in order: the owner's arrival, its own full avatar `ObjectUpdate` (local id 258129971), `AvatarAppearance` and `AvatarAnimation`; the second bot's arrival and its full update (local id 258129972) and appearance on both circuits (`CompleteMovement`); the second bot's full update and appearance again on the owner's circuit, when the owner's `SendInitialData` ran after the second bot had arrived; the owner's full update and appearance on the second bot's circuit at its `SendInitialData`; the two logouts; and `KillObject` for 258129972 on both circuits after the second bot's `LogoutReply` (TB87). Full updates and appearances are reliable and zero-coded. The kills are reliable and not zero-coded. The second bot arrives at Z 27.96, two metres above the owner: `MakeRootAgent`'s landing ray cast (`ScenePresence.cs` lines 1543–1602, with `PrimsNonPhantomAgents` on ubODE) hits the owner's capsule and stacks the newcomer on it. Ferroan's walking stand-in has no ray cast, so the compare masks Z: relative Z is that missing avatar ray cast (a physics gap), and absolute Z is the missing terrain row in the Ferroan home store. Once physics has landed an avatar, C# writes its collision plane into later full updates.

`cargo xtask live run avatars grid --region ferroan` compares the Ferroan run with this file (`xtask/src/avatars_capture.rs`), and `the_csharp_capture_has_both_avatars_and_the_kill` checks the file itself.

## Names capture

`names.cap` is the home region's UDP from a later `cargo xtask live run avatars grid` at the same pin and with the same capture patch, after the scenario gained its `UUIDNameRequest` step (Q84). The run printed `avatars: pass (each bot saw the other's local id and appearance and got its name; the unknown id got no reply; KillObject on leave)`. The raw file passed `ferroan-capture check` (151 records) and was then redacted the same way: the names in each `UUIDNameReply` are `Redacted`, except the reply for the unknown id, whose `Unknown` and id the redactor keeps because they name nobody (TB89). It holds what `avatars.cap` holds, then: one `UUIDNameRequest` from each bot for the other and for `UNKNOWN_NAME_ID` (`tools/bot/src/scenario.rs`); one `UUIDNameReply` naming each bot, reliable and not zero-coded, one block each; no reply for the unknown id; and, after the owner asks for the unknown id again, one reply for it (C# answers `Unknown` and the id from the miss it cached). In this recording, and in a second one made to check, the leaving viewer did not get its own `KillObject` after its `LogoutReply`: that kill (TB87) races the circuit's close. `avatars.cap` keeps it, so the avatar compare stays on that file and only the name compare reads this one. `the_csharp_capture_names_each_bot_and_the_cached_miss` checks this file. Record it the way `avatars.cap` is recorded, redacting into `fixtures/avatars/names.cap`.

To record it again, under the live lock with a free port base:

```
FERROAN_LIVE_PORT_BASE=24800 cargo xtask live run avatars grid
cargo run -p ferroan-capture -- check .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap
cargo run -p ferroan-capture -- redact --in .scratch/live/runs/<base>-<pid>/grid/capture-udp.cap --out fixtures/avatars/avatars.cap
```

Read the redacted file before committing it.
