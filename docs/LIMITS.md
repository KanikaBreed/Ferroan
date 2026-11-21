# Limits

Every bound that refuses, truncates, drops or delays outside input reports through `ferroan_diag`. The event is a warning on target `ferroan::limit`. The plain log line carries the limit id, because that sink does not print the target. The counter is `ferroan_limit_hits_total{limit,action}`. It counts the id on the hit, so a new limit does not need a second hand-written list. Soft limits also warn once at 80 percent, and again only after they fall below that line. The first report for an id is logged in full. Later reports in the same 60 seconds increment the counter and are summarised as `suppressed=N`. A high-water warning does not consume that minute, so a refusal just after it is still logged.

`cargo xtask check` fails when a limit id in `crates/ferroan-diag/src/catalog.rs` is missing from this file, or when that const is not named from another crate. How to trip three of them on one machine is [Trying a limit hit](TESTING.md).

The recent-events ring keeps 1000 warning, error and limit lines per process. A malformed packet or body adds a debug line with the first 256 bytes in hex. `diag dump` on the remote console, `GET /diag` on the metrics listener, and `cargo xtask live diag` write the same snapshot to `LOGDIR` or `.scratch/`. A panic hook writes `panic-{server}-{millis}.txt` there before the process exits. Passwords, session ids, circuit codes and capability URLs are redacted in all of it.

| id | value | where | soft or hard | configurable | C# at the pin |
| --- | --- | --- | --- | --- | --- |
| `http.post-body` | 1 MiB | `crates/ferroan-region/src/simulation.rs` | hard | no | No matching cap. The region refuses the body and answers 413. |
| `http.asset-body` | 50,000,000 bytes | `crates/ferroan-services/src/asset/http.rs` | hard | no | `ServerUtils.cs` sets `XmlReaderSettings.MaxCharactersInDocument` to 50,000,000. |
| `http.xml-chars` | 50,000,000 characters | `crates/ferroan-http/src/xml.rs`, asset parser | hard | no | Same `MaxCharactersInDocument`. The asset parser measures bytes. |
| `http.xml-depth` | 64 | `crates/ferroan-http/src/xml.rs` | hard | no | Tranquillity's reader has no depth cap. |
| `http.query-bytes` | 1 MiB | `crates/ferroan-http/src/query.rs` | hard | no | Query strings are not capped. |
| `http.fields` | 4096 | `crates/ferroan-http/src/query.rs` | hard | no | Query fields are not capped. |
| `http.list-items` | 4096 | `crates/ferroan-http/src/query.rs` | hard | no | A `[]` list is not capped. |
| `http.response-bytes` | caller limit, default 50,000,000 | `crates/ferroan-http/src/client.rs` | hard | the caller passes it | Region grid replies pass 1 MiB. C# does not cap the read. |
| `inventory.fetch-folders` | 64 | `crates/ferroan-region/src/fetch_caps.rs` | hard | no | `FetchInvDescHandler.cs` loops every requested folder, with no cap. |
| `inventory.fetch-items` | 256 | `crates/ferroan-region/src/fetch_caps.rs` | hard | no | `FetchInventory2Handler.cs` has no item-count cap. |
| `inventory.fetch-llsd` | 2 MiB | `crates/ferroan-region/src/fetch_caps.rs` | hard | no | The handlers do not cap the LLSD they write. |
| `inventory.reply` | 8 MiB | `crates/ferroan-region/src/inventory.rs` | hard | no | The inventory client does not cap the reply. |
| `udp.datagram` | 4096 bytes | `crates/ferroan-protocol/src/packet.rs`, `crates/ferroan-client/src/circuit.rs` | hard | no | `UDPPacketBuffer.BUFFER_SIZE` in libopenmetaverse `OpenMetaverse/ObjectPool.cs` is 4096. A larger datagram is truncated by that buffer. Ferroan refuses it. The receive buffer is 4097 bytes so the extra byte is visible. |
| `udp.zero-expansion` | 256 KiB | `crates/ferroan-protocol/src/packet.rs` | hard | no | OpenMetaverse does not cap zero-expansion. See [PROTOCOL.md](PROTOCOL.md). |
| `udp.unacked` | 256 packets | `crates/ferroan-protocol/src/reliability.rs` | soft | no | C# keeps unacked packets until the client times out. Ferroan refuses a new reliable send at 256. High-water is 205 packets. |
| `udp.retries` | 4 by default | `crates/ferroan-protocol/src/reliability.rs` | soft | no | C# retries until inactivity, with no count. This drop is the bot policy. The server keeps `retry_limit` unset. The observed value is that packet's retry count, the total that crossed the limit. |
| `event.bytes` | 64 KiB | `crates/ferroan-client/src/event_queue.rs` | hard | no | One event has no matching cap. |
| `event.queue` | 256 events | `crates/ferroan-client/src/event_queue.rs` | soft | no | The queue is not capped. |
| `event.queue-bytes` | 1 MiB | `crates/ferroan-client/src/event_queue.rs` | soft | no | The queue is not capped. |
| `control.hosts` | 256 names | `crates/ferroan-region/src/control_plane.rs` | hard | no | [D20](DECISIONS.md) keeps Ferroan's list bound. C# does not. |
| `control.host-bytes` | 2048 bytes | `crates/ferroan-region/src/control_plane.rs` | hard | no | [D20](DECISIONS.md). |
| `control.addresses` | 4096 addresses | `crates/ferroan-region/src/control_plane.rs` | hard | no | [D20](DECISIONS.md). |
| `timer.milliseconds` | `i32::MAX` | `crates/ferroan-region/src/config.rs` | hard | no | [D19](DECISIONS.md). C# accepts a timer that overflows signed milliseconds. |
| `timer.seconds` | `i32::MAX / 1000` | `crates/ferroan-region/src/config.rs` | hard | no | [D19](DECISIONS.md). Zero is also refused. |
| `library.files` | 64 | `crates/ferroan-region/src/library.rs` | hard | no | The library loader does not cap the file count. |
| `library.file-bytes` | 2 MiB | `crates/ferroan-region/src/library.rs` | hard | no | One library file is not capped. |
| `library.total-bytes` | 8 MiB | `crates/ferroan-region/src/library.rs` | hard | no | The archive total is not capped. |
| `asset.remote-body` | 16 MiB | `crates/ferroan-region/src/remote_asset.rs` | hard | no | The region connector does not cap the asset body. |
| `asset.xml-depth` | 32 | `crates/ferroan-services/src/asset/xml.rs` | hard | no | `XmlSerializer` has no matching depth cap. |
| `asset.xml-elements` | 100,000 | `crates/ferroan-services/src/asset/xml.rs` | hard | no | No matching element cap. |
| `asset.xml-name` | 128 characters | `crates/ferroan-services/src/asset/xml.rs` | hard | no | No matching name-length cap. |
| `asset.metadata-rows` | count 0..=10,000 and start 0..=1,000,000 | `crates/ferroan-services/src/asset/store.rs` | hard | no | `FetchAssetMetadataSet` does not cap `start` or `count`. |
| `asset.loader-files` | 10,000 assets | `crates/ferroan-services/src/asset/loader.rs` | hard | no | `AssetLoaderFileSystem` does not cap the asset count. The library set at the pin is 455 files. |
| `asset.loader-file-bytes` | 2 MiB | `crates/ferroan-services/src/asset/loader.rs` | hard | no | One library file is not capped. The largest file at the pin is under 400 KiB. |
| `asset.loader-total-bytes` | 32 MiB | `crates/ferroan-services/src/asset/loader.rs` | hard | no | The library total is not capped. The set at the pin is about 13 MiB. |
| `otlp.queue` | 256 records | `crates/ferroan-log/src/otlp.rs` | soft | no | There is no OTLP export in C#. A full queue drops the record. |
| `llsd.parsed-bytes` | 128 MiB | `crates/ferroan-llsd/src/lib.rs` | hard | no | OpenMetaverse does not cap the parsed tree. The observed value is the allocation that did not fit. |
| `llsd.binary-depth` | 128 | `crates/ferroan-llsd/src/binary.rs` | hard | no | No matching depth cap. |
| `llsd.xml-depth` | 256 | `crates/ferroan-llsd/src/xml.rs` | hard | no | No matching depth cap. |
| `llsd.notation-depth` | 128 | `crates/ferroan-llsd/src/notation.rs` | hard | no | No matching depth cap. |
| `db.pool` | 8 connections | `crates/ferroan-db/src/limits.rs` | soft | no | Tranquillity's `Maximum Pool Size` is not honoured. A full pool waits up to 10 seconds. High-water is 7 open connections. |
| `texture.bytes` | 64 KiB | `crates/ferroan-types/src/texture.rs` | hard | no | OpenMetaverse does not cap a texture entry. |
| `texture.faces` | 45 | `crates/ferroan-types/src/texture.rs` | hard | no | `maxfaces` is 45 in `GetBytes`. A higher index is refused. |
| `texture.bitfield` | 8 groups | `crates/ferroan-types/src/texture.rs` | hard | no | The face bitfield is not capped at 8 groups. |
| `extra.bytes` | 64 KiB | `crates/ferroan-types/src/extra.rs` | hard | no | Extra parameters are not capped. |
| `log.redact-secrets` | 64 secrets | `crates/ferroan-log/src/redact.rs` | hard | no | No matching cap. Each login holds its session id, secure session id, circuit code and seed path until that session's lease drops. A distinct secret past 64 is not stored. |
| `console.sessions` | 32 sessions | `crates/ferroan-log/src/console.rs` | hard | no | `RemoteConsole.cs` `DoExpire` (lines 360–383 at the pin) removes a connection whose `last` is more than 500000 ms behind `Environment.TickCount`. The timer period is 5 seconds (line 129). `last` is set when the session starts (line 411) and when a poll checks or reads events (lines 621 and 641), not when a command arrives. C# does not cap the count. Ferroan compares a monotonic clock, so the 500 s idle check does not wrap. A new session past 32 is refused. |
| `terrain.edge` | 4096 metres | `crates/ferroan-scene/src/height.rs` | hard | no | `Constants.MaximumRegionSize` is 4096 and `TerrainPatchSize` is 16. A side of 0, not a multiple of 16, or past 4096 is refused. |
| `terrain.blob` | 67,108,872 bytes | `crates/ferroan-scene/src/height.rs` | hard | no | No matching cap. The bound is 8 bytes plus a 4096 by 4096 map of floats, for both the stored blob and the decoded map. |
