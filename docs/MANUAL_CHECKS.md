# Manual checks

Everything a program can check is checked by the live harness ([D15](DECISIONS.md), [TESTING.md](TESTING.md)). `cargo xtask live run all` already checks login, walk, chat, build, teleport, a texture upload and fetch, and inventory. `cargo xtask live assets` compares that texture and inventory HTTP with `ferroan-robust`. This file is what a person checks by hand with Firestorm at each phase gate: what the bot cannot see, mostly what the viewer draws and how it feels to use.

Record each run in the gate's row in [TASKS.md](../TASKS.md): date, Firestorm version and build (Help > About Firestorm), the Ferroan commit, the Tranquillity revision, and a pass, fail or note for every step. A failed step gets a bug report ([BUG_REPORTING.md](BUG_REPORTING.md)) with the viewer log.

## Setting up Firestorm for a local grid

1. Start the test region with `cargo xtask live up --assets ferroan` for G0, or `cargo xtask live up` for the C# asset-service comparison. It prints the login URL (`http://127.0.0.1:<port>/`). The account is `Test User`. The password is in the run configuration under `.scratch/live/` and is not printed.
2. In Firestorm: Preferences > OpenSim > Grid Manager. Add the login URL (for example `http://127.0.0.1:9100/`), press Apply, and pick the grid on the login screen.
3. Use a fresh Firestorm cache for each gate run: Preferences > Network & Files > Directories > Clear Cache, then restart. A warm cache hides texture and inventory faults.

## G0: Phase 0 gate

Log in to the local C# region while its asset service is `ferroan-robust`. Start it with:

```sh
cargo xtask live build
cargo xtask live up --assets ferroan
cargo xtask live run assets --running
```

After the build, `live up --assets ferroan` seeds a copy of the C# asset store with library assets, starts `ferroan-robust`, and leaves the region pointed at it. It prints the login URL and the region and asset ports. `live run assets --running` checks texture upload, library texture fetch and inventory with `ferroan-bot`, leaving both services running for Firestorm. Use the printed login URL and the generated password from `.scratch/live/run/password`. Record the login URL, asset port and Ferroan commit in the G0 row. The maintainer's completed G0 run recorded there used the earlier manual setup; these commands are the setup for subsequent runs.

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

After the viewer has logged out, `cargo xtask live down` stops both the region and `ferroan-robust`. To run the C# comparison, stop this setup first and start `cargo xtask live up`. `cargo xtask live assets` separately compares the two services' captured texture and inventory HTTP and stops its processes when finished.

## Firestorm on the Ferroan region (before G2)

These are informal maintainer checks. Firestorm validation is pending; this
setup does not mark G2 or any other gate passed.

```sh
cargo xtask live build
cargo xtask live up grid --region ferroan
```

The command leaves C# Robust, the C# neighbour and the Ferroan home region
running. Add the printed Robust login URL in Firestorm's Grid Manager, then
log in as the printed account using the password from the printed file path.
Keep that password and private viewer logs out of public reports. All servers
bind to loopback by default, so run Firestorm on the same machine.

| Step | Expected |
| --- | --- |
| Log in to `Ferroan Test` | Login completes and the viewer arrives on the Ferroan region |
| Stay connected for at least two minutes | The session remains connected without a timeout |
| Log out | The viewer completes logout |
| Log in again with the same account, then log out | A fresh session arrives and completes logout |

Terrain (P2.0e), avatar appearance (P2.0f), local chat (P2.0g), teleports inside
the region and to its neighbour (P2.0h/P2.0i), objects, physics and scripts are
not implemented yet. Inventory and texture fetch capabilities arrive with
P2.0d7. A blank scene, missing avatar appearance or unavailable inventory is
not evidence those later items have passed.

Record informal notes with the date, Firestorm version/build, printed Ferroan
commit, Tranquillity pin and result for each step. The maintainer supplies
these observations; automated login checks cannot establish what Firestorm
renders. After logging out, stop all three processes from this worktree:

```sh
cargo xtask live down
```

Use the same command after an interrupted launch. The harness retains process
identities for recovery. Stop this grid before running another live scenario.

## Later gates

Each phase adds a section here when its packages are written. The Phase 2 first-login milestone and gate will cover the same steps against a Ferroan region, side by side with a C# region.
