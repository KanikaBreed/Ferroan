# Parcel overlay arrival on the Land throttle

Two fresh viewer circuits logged in to the same Pin8 region with the [Q60b multi-parcel store](../parcel-overlay/README.md). Only the requested Land rate varied between the two throttle groups. The owner logged in first. All times below are milliseconds since the first of the viewer's UseCircuitCode and seed request reached the simulator. The owner's UseCircuitCode and seed request were recorded in the same millisecond on two different clocks, so their order is unresolved.

| Viewer | Land requested (bit/s) | Parcel sequences 0, 1, 2, 3 | SendInitialData | Land terrain datagrams |
| --- | --- | --- | ---: | ---: |
| Estate owner | 512,000 | 118, 133, 148, 178 | 372 | 11 |
| Group member | 8,000, restored to 512,000 at 2,019 ms | 26, 1,201, 2,031, 2,046 | 298 | 11 |

Both viewers received one pass. The member received the two outstanding chunks within 27 ms of restoring Land. Its last three chunks arrived after the initial-data gate. Their parcel sequences and LLUDP sequences identify them as the continuation of its original pass. Neither circuit had an overlay resend. On both circuits the four overlay chunks arrived ahead of the first land terrain datagram. Pin8 calls the movement-completion overlay in `ScenePresence.cs` at 2475 and only requests another at the later gate for zero teleport flags (4343). These normal grid logins carried the login flag. `LLClientView.SendLandParcelOverlay` queues the packet on Land (6749). This capture records arrival order on the socket, which can differ from queue insertion order.

The two JSON timelines retain movement, handshake, seed, throttle, terrain, overlay and logout milestones. Seed timestamps come from the home HTTP proxy. The initial-data timestamp comes from the region's per-agent debug log. Each UDP event carries its original sequence and flags. The two FERROCAP1 files contain only the original outgoing Land terrain and ParcelOverlay datagrams, with timestamps relative to their viewer. Extraction verifies the C# arrival version and checks that each published datagram re-encodes byte for byte before copying the original bytes. The replay compares every parcel body with Q60b's fixture for that viewer. A resend has the original reliable sequence and identical body with the resent flag set. A new transmission of parcel sequence zero starts another pass after the previous four chunks.

## Reproduce

Build Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` with `cargo xtask live build`. Run `cargo xtask live run parcel-timing grid --physics basicphysics` under the shared live lock with a free port base and `UseSharedCompilation=false`. The harness prints the four arrival times per viewer, the gate times, the restore time and the fixture directory. Exact wall-clock intervals vary between runs.

To re-extract from the same raw run, use `cargo xtask live parcel-timing-fixtures <grid-run-directory>` in the same time zone as the region log. To install a new recording, copy its four files here, update the table and refresh the digest in `pin8_parcel_arrival_replays_both_circuits` using the file order in that test. Run `cargo test -p xtask parcel_timing`.
