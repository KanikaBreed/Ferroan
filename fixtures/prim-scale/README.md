# Admitted scale at the pin

[pin8-scales.txt](pin8-scales.txt) records `Shape.Scale.ToBytes` after the pin admits a stored part. Pin8 (`c401bceecd1684832ae8ba86a31b5ee01a4ded7b`) recorded it; Pin9 (`41f18f983bb2bfc82e033fed410d361ba57d5aa4`) regenerated it with the same bytes. The file keeps its Pin8 name.

`cargo xtask regionstore scales` builds [tools/regionstore-oracle](../../tools/regionstore-oracle/README.md) against a task-owned clone of the pin and writes this file. `cargo xtask regionstore scales-verify` rebuilds the oracle and checks the committed file. The header records the first ten characters of the pin.

Each row is one part. `AddRestoredSceneObject(group, false, true, false)` runs on `new Scene(new RegionInfo())`. That constructor leaves `ClampPrimSize` false and `CollidablePrims` true, and the size fields at `Scene`'s defaults (nonphysical 0.001 to 256, physical 0.01 to 10), unless the row sets them. The group is built from `PrimitiveBaseShape.CreateBox` with the row's flags, path curve, state, pcode, physics shape and scale. A root whose stored physics shape is `none` gets `PhysicsShapeType` before the part joins its group, as the SQLite loader does, and the row records that stored value. `CreatePrimUpdateBlock` writes the same twelve bytes with `AddVector3` at offset 28 of `ObjectUpdate.ObjectData`.

Float columns are little-endian IEEE-754 hex, in the order `NonPhysicalPrimMin`, `NonPhysicalPrimMax`, `PhysicalPrimMin`, `PhysicalPrimMax`, then the stored scale. The last column is `Shape.Scale.ToBytes`. `a_stored_zero_scale_is_sent_as_pin8_bytes` admits a stored part with those inputs and compares the admitted scale and the encoded block.
