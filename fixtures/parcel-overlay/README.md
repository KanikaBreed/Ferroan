# ParcelOverlay from Pin8

These are the ParcelData bodies received by two viewer circuits on Tranquillity `dev-build` at `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. Each record has a four-byte little-endian signed parcel sequence ID, a two-byte little-endian length (1,024), and the captured cell bytes. The sequence starts at zero for each viewer. Bodies carry no names, addresses or session identifiers.

Each run opens a disposable copy of [the SQLite store](../regionstore/objects.db). The estate owner and a second viewer log in separately. Only the second viewer belongs to East's group. The [membership seed](membership.sql) gives that viewer the Everyone role without selecting an active group. `local_memberships_paint_the_captured_overlay` opens that seed and compares the multi-store member and owner payloads with these captured bodies.

| Store | What the capture exercises | Bodies per viewer |
| --- | --- | --- |
| Empty | The whole region belongs to the estate owner. | 4 |
| Multi | The [Q60a seed](../regionstore/parcels.sql) has a buyer-restricted West parcel and a group-owned East parcel for sale to anyone. Both have local sound and hide avatars. The estate owns the intervening cells. | 4 |
| Lone | West is the only stored parcel. The whole region expands to West, which is for sale at zero cost to the second viewer alone. | 4 |
| Legacy | A 512 m square scene has a 512-byte public parcel bitmap. The second run of set bits lies on the next legacy row (64 cells), while the overlay row spans 128 cells. West sets local sound alone and East hides avatars alone. | 16 |

For seeded stores, `home-region.sql` assigns the seed's region to the live home and retains one access entry per parcel/avatar pair. Repeated pairs cause the startup save to fail on Pin8 (TB128). The Rust replay also loads the original Q60a seed with all access rows and compares both viewers with the multi capture.

## Reproduce

Build the pinned server with `cargo xtask live build`. On Linux run these commands under the shared live lock with a free port base and `UseSharedCompilation=false`:

```sh
cargo xtask live run parcel-empty grid --physics basicphysics
cargo xtask live run parcel-multi grid --physics basicphysics
cargo xtask live run parcel-lone grid --physics basicphysics
cargo xtask live run parcel-legacy grid --physics basicphysics
```

For each completed run, copy `parcel-<seed>-owner.bin` and `parcel-<seed>-member.bin` from its `.scratch/live/runs/<run>/grid/` to this directory. The receiver verifies that each decoded message body re-encodes to the original bytes, checks the sequence and size, and saves the original bodies in sequence order. Replay all eight files with `cargo test -p ferroan-scene parcels::overlay`.
