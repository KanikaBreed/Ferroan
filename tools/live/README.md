# Live harness patches

`cargo xtask live` builds Tranquillity in `.scratch/live/` and starts it. The scratch tree is never a reference checkout.

Two patches are applied only there, at the revision in [docs/REFERENCES.md](../../docs/REFERENCES.md):

- [../capture/tranquillity-capture.patch](../capture/tranquillity-capture.patch) records raw UDP when `[ClientStack.LindenUDP] CaptureFile` is set. `live build` applies it. `live up` does not set `CaptureFile` yet (P0.10d).
- [tranquillity-listen.patch](tranquillity-listen.patch) binds the HTTP and HTTPS listeners to `127.0.0.1`. At the current pin there is no listen-address setting (`BaseHttpServer` defaults to every interface, and the HTTPS listener hardcodes that). This patch goes away when OpenSim-NGC/OpenSim-Tranquillity#412 is merged and the harness can set `[Network] bind_ip_address = "127.0.0.1"`.

`live up` also adds the SQLite `UserAccounts.DisplayName` and `NameChanged` columns after the first start, without changing the `migrations` version. That workaround goes away when OpenSim-NGC/OpenSim-Tranquillity#411 is merged. The manual recipe and both gotchas are in [docs/LIVE.md](../../docs/LIVE.md).

`cargo xtask live run <scenario>|all` starts that region in the foreground with `--console basic` (the default `local` console reads keys and rejects a pipe, and the option is applied after the ini). After logins are enabled it creates the second account `Chat Bot` on that console, runs the bot scenarios, and stops the region it started. It refuses to run when a region from `live up` is already up. The chat password stays in memory. `live up` does not create the second account.
