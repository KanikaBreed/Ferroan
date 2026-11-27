# Appearance oracle

`cargo xtask appearance service-generate` builds this program against the pinned Tranquillity checkout and records `GET /texture/<agent>/<channel>/<uuid>` from `AppearanceServerHandler`. `cargo xtask appearance service-verify` runs that capture again and requires the same JSON.

The listener is loopback. The stores are copies of `fixtures/db/sqlite/Avatar.db` and `fixtures/db/sqlite/AssetStore.db`. The capture is SQLite only: the queue row does not name another backend. The handler reads the avatar service and the asset service. It does not write the avatar index.

The fixture records a hit, the same hit with an upper-case channel token, an unknown token, a uuid that is not the index, an empty asset, a missing asset, a non-GET, a shard refusal before the method check, a path that is not a uuid, a short path and the bare `/texture` page. Build output stays under `.scratch/`.
