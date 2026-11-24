# Region store objects

`objects.db` is a SQLite region store seeded by [tools/regionstore-oracle](../../tools/regionstore-oracle/README.md). `objects.json` is the groups that store returns for region `11111111-1111-1111-1111-111111111111`. The `pin` field is the checkout that produced the dump. Pin8 (`c401bceecd`) regenerated it: the groups are unchanged; the database also carries `RegionStore` 48, the two indexes of #507.

The file holds one prim, two linksets, a prim with no shape row, a row that cannot be built, and one prim in another region.

- `aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1` (`showcase`) is a single prim, so its link number stays 0. Its stored link number is ignored. The sit target is explicitly inactive. Stored flags keep physics and phantom; touch, money and allow-inventory-drop are cleared, and volume detect sets bit `0x4000`. Extra params stay `00aa`. The pseudo-CRC is 42.
- `bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1` (`link-restore`) has children stored as link 3 then link 2. Those numbers are restored, so the parts come back as the root (1), `child-two` (2), then `child-three` (3). `child-two` has an explicit sit target with a zero offset, so the target is active.
- `cccccccc-cccc-cccc-cccc-ccccccccccc1` (`link-table`) has a missing stored number and a number past the group. The parts stay in table order: the root (1), `child-first` (2), then `child-second` (3).
- `dddddddd-dddd-dddd-dddd-ddddddddddd1` (`no-shape`) has no shape row, so the shape is the default box, including the plywood texture bytes and a single zero extra-params byte. The sit offset is not zero and the active cell is null, so the target is active. Pay prices are 0. A stored pseudo-CRC of 0 is absent. The media URL is empty.
- A row whose id is not a UUID is absent. The other prims in the region still load.
- `ffffffff-ffff-ffff-ffff-fffffffffff1` belongs to region `22222222-2222-2222-2222-222222222222` and is absent from this dump.

Local ids and parent ids are 0. The eight structured payloads are the stored text or bytes. `cargo xtask regionstore verify` reloads the file through the pin. Ordinary tests replay `objects.json` and do not start the oracle.
