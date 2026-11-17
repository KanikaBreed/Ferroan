# ferroan-bot

A headless viewer for the live harness. It speaks the viewer's side of login and the region circuit. It does not start a region and it does not draw anything.

The code is adapted from Aurora Viewer `crates/aurora-net` at `48cb76e830a13965ac080128bb874afedda86d69`: `xmlrpc.rs`, `login.rs`, `circuit.rs`, the throttle group's channel shares, and the seed POST. The handshake order follows that tree's `session.rs`. Inventory-offer receipts, packet-loss statistics and the dynamic throttle are not included. The region's own circuit is still ported from Tranquillity, not from this crate.

## What it does

`login` POSTs XML-RPC `login_to_simulator`. The password on the wire is `$1$` plus the md5 hex. Linden Lab grid URIs truncate the password to 16 characters, as the viewer does. Tranquillity does not. `mac` and `id0` are fixed labels for this program, not a hash of the machine.

The circuit binds `127.0.0.1`. Reliable packets are resent until they are acked. `UseCircuitCode` uses the viewer's login policy (3 resends, 5 seconds). After that ack the bot sends `CompleteAgentMovement`, one `AgentThrottle` (the 6_000 kb/s group, which is 1.5 times Aurora's 4_000 kb/s setting) and `AgentHeightWidth` (1080 by 1920). `RegionHandshake` is answered with flags `0x7`: self appearance, send all cacheable objects, and an empty object cache. `StartPingCheck` is answered. `LogoutRequest` waits for `LogoutReply`.

The seed POST asks for `EventQueueGet`, `GetTexture`, `FetchInventoryDescendents2`, `FetchInventory2` and `NewFileAgentInventory`.

A login reply over 8 MiB, or a seed reply over 1 MiB, is refused. At most 256 reliable packets stay unacked.

## Command

```sh
ferroan-bot login --uri http://127.0.0.1:9100/ --user "Test User" --start last --circuit
```

The password is `--password` or `FERROAN_BOT_PASSWORD`. The command prints `login: true`, the agent id and the simulator address. It does not print the password, the session id, the circuit code or the seed URL. `--circuit` is the handshake and a logout.

`cargo xtask live run <scenario>|all` starts the local region, runs one or all of `login`, `walk`, `chat`, `build`, `teleport`, `texture` and `inventory`, and stops the region. The report is one line per scenario. A passing `all` run has logged into the harness region. This command does not start a region by itself. CI does not run the harness yet.

## Tests

`cargo test -p ferroan-bot` checks the password hash, the XML-RPC codec, resend and duplicate suppression, and a handshake against a fake region and a fake seed. Those tests do not start Tranquillity.
