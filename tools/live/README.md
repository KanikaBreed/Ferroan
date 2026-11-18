# Live harness

`cargo xtask live` builds Tranquillity in `.scratch/live/` and starts it. The scratch tree is never a reference checkout.

One patch is applied only there, at the revision in [docs/REFERENCES.md](../../docs/REFERENCES.md):

- [../capture/tranquillity-capture.patch](../capture/tranquillity-capture.patch) records raw UDP when `[ClientStack.LindenUDP] CaptureFile` is set. `live build` applies it. `live run` sets `CaptureFile`, records the bot's HTTP beside it, then runs `check`, `redact` and `replay`. `live up` does not set `CaptureFile`.

The region ini sets `[Network] bind_ip_address = "127.0.0.1"`. An empty value means every interface, which is Tranquillity's default. The harness does not alter SQLite. At this pin, `UserAccount.migrations` version 4 adds `DisplayName` and `NameChanged`, and the region is started once. The manual recipe is in [docs/LIVE.md](../../docs/LIVE.md).

`live build` also publishes GridServer, self-contained, because that project references `Microsoft.AspNetCore.App` and the harness machine has only `Microsoft.NETCore.App`. `cargo xtask live assets` starts that server, and `ferroan-robust`, behind `ferroan-capture http-proxy`, and compares the texture and inventory HTTP. Its C# asset-service ini sets `[Network] bind_ip_address = "127.0.0.1"`, so the service passes the loopback socket check.

`cargo xtask live up --assets ferroan` prepares a seeded copy of the asset store, starts `ferroan-robust`, and starts the C# region with the same `Include-Asset` override used by the comparison. Both stay up, with the login URL and region and asset ports printed. `cargo xtask live run assets --running` drives texture and inventory against that existing region, reads the generated password privately, and leaves both processes running. `cargo xtask live down` stops both. This is the setup for [the G0 viewer check](../../docs/MANUAL_CHECKS.md).

`cargo xtask live run <scenario>|all` starts that region in the foreground with `--console basic` (the default `local` console reads keys and rejects a pipe, and the option is applied after the ini). After logins are enabled it creates the second account `Chat Bot` on that console, runs the bot scenarios, and stops the region it started. It refuses to run when a region from `live up` is already up; the explicit `live run assets --running` check uses that region instead and needs no second account. The chat password stays in memory. `live up` does not create the second account.

`cargo xtask live run <scenario>|all|neighbour grid` starts GridServer and two regions. Accounts are created on the Robust console before the regions start, because a grid region cannot create the estate owner itself. The regions are `--background`. Their HTTP listeners are on `127.0.0.2` and `127.0.0.3`. Proxies on `127.0.0.1` record region-to-Robust and region-to-region HTTP. The bot logs in on Robust's address, `127.0.0.4`. Home-region UDP is the only datagram capture. The guide is [docs/LIVE.md](../../docs/LIVE.md).
