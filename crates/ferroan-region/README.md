# ferroan-region

This first region process logs a bot in through C# Robust. It reads Tranquillity's region configuration using `ferroan-config`, registers with the grid service, accepts `QUERYACCESS` and `POST /agent/...` simulation requests, and serves one LLUDP circuit and its seed capability. Robust remains the viewer's login server. SIGTERM or Ctrl-C deregisters the region before closing its HTTP listener.

Run the local compatibility check with:

```sh
cargo xtask live run login grid --region ferroan
```

The harness starts C# Robust and a C# neighbour, using the Ferroan process for the home region. It checks login and requires a real `LogoutReply`. The register, access-query and agent-creation HTTP exchanges are compared with the C# baseline. See [the harness guide](../../docs/LIVE.md).

The process accepts `--inimaster FILE`, `--inifile FILE`, `--inidirectory DIR` and `--background`. Paths and includes are resolved from its working directory as the region loader does. The harness supplies the normal `RegionServer.ini` and `Regions.ini` files and copies the reference defaults into the working directory.

Current limits: one 256 metre region, one active login, and a five-minute circuit lifetime. Simulation bodies are limited to 1 MiB before and after gzip decompression. The local login boundary admits agent creation from loopback and rejects viewer shard headers; broader trusted-host policy, estate/presence authorization and region crossings remain planned. It does not serve `login_to_simulator`, terrain, appearance, physics, or object updates. The seed advertises `EventQueueGet`; an empty poll waits 58 seconds before returning 502, as Tranquillity does. Leaving the region closes the queue.

`RegionHandshake` takes the name and `RegionType` from the region INI and the estate owner from the server configuration. Flags, access, terrain texture IDs, elevations, water height and billing fields currently use Tranquillity's fresh-region settings. The typed handshake builder accepts updated settings, but loading persisted region/estate settings remains planned. Its cache UUID is stable for this process lifetime. Server-side baking and terrain PBR are advertised only through the builder's explicit capability inputs; the current region enables neither. The region sends queued ACKs every 500 ms or after receiving over two MTUs of reliable data. Its existing bounded retry policy is four retries at 200 ms, after which the entry is removed; C# instead retries until its inactivity timeout.

The event queue permits four simultaneous polls, 256 queued events and 1 MiB of queued XML. Individual events are limited to 64 KiB, 4096 values and 32 levels of nesting. Excess polls return 503; event producers receive a structured error when a bound is reached. Seed and poll request bodies are limited to 64 KiB. Region event producers remain planned; the generic queue can already deliver batches with the captured C# encoding.

For this login comparison, both C# and Rust disable map generation (`[Map] GenerateMaptiles = false`) and use a zero static map UUID, which is a supported C# configuration. Registration then advertises zero map image IDs without fabricating map assets. TLS listeners, variable-size regions and interactive configuration are refused. This is a local test region; viewer validation remains a separate maintainer gate.
