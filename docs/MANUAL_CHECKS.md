# Manual checks

Everything a program can check is checked by the live harness ([D15](DECISIONS.md), [TESTING.md](TESTING.md)). This file is what a person checks by hand with Firestorm at each phase gate: what the bot cannot see, mostly what the viewer draws and how it feels to use.

Record each run in the gate's row in [TASKS.md](../TASKS.md): date, Firestorm version and build (Help > About Firestorm), the Ferroan commit, the Tranquillity revision, and a pass, fail or note for every step. A failed step gets a bug report ([BUG_REPORTING.md](BUG_REPORTING.md)) with the viewer log.

## Setting up Firestorm for a local grid

1. Start the test region with `cargo xtask live up`. It prints the login URL (`http://127.0.0.1:<port>/`). The account is `Test User`. The password is in the run configuration under `.scratch/live/` and is not printed. This region uses its in-process asset service.
2. In Firestorm: Preferences > OpenSim > Grid Manager. Add the login URL (for example `http://127.0.0.1:9100/`), press Apply, and pick the grid on the login screen.
3. Use a fresh Firestorm cache for each gate run: Preferences > Network & Files > Directories > Clear Cache, then restart. A warm cache hides texture and inventory faults.

## G0: Phase 0 gate

Log in to the local C# region while its asset service is `ferroan-robust`. `cargo xtask live assets` runs texture and inventory against that service and against the C# service, then stops. `cargo xtask live up` leaves the region up on the in-process C# asset service. A region that stays up and points at `ferroan-robust` is not a harness command yet, so that Firestorm session is still set up by hand. Record the login URL and the Ferroan commit in the G0 row.

| # | Step | Expected |
| --- | --- | --- |
| 1 | Log in as the test account | Login completes in under a minute, no error dialog. You arrive standing on the ground |
| 2 | Look around for a minute | Terrain is textured. Your avatar is not a cloud or grey; its skin, clothes and shape appear |
| 3 | Open inventory, then the Library folder, then Textures | Folders load. Open three textures: each preview loads sharply, not stuck blurry |
| 4 | Build > rez a cube. Edit it: Texture tab, pick a Library texture | The texture shows on the cube within a few seconds |
| 5 | Upload a texture (Build > Upload > Image) from a small PNG of your own | Upload succeeds. The new item appears in Inventory > Textures and its preview loads |
| 6 | Apply the uploaded texture to the cube | It shows on the cube |
| 7 | Log out, clear the cache, log in again | The cube and both textures load again from the asset service, not from the cache |
| 8 | Walk, fly, say something in local chat, teleport within the region (map, double click) | All work as they do on your usual Tranquillity region |
| 9 | Log out | Clean logout, no crash dialog |

Notes worth recording even when the step passes: slow texture loads, textures that never sharpen, anything that looks different from the same steps on `cargo xtask live up` (in-process C# asset service).

## Later gates

Each phase adds a section here when its packages are written. The Phase 2 first-login milestone and gate will cover the same steps against a Ferroan region, side by side with a C# region.
