# Pin8 admitted scale

[pin8-scales.txt](pin8-scales.txt) records `Shape.Scale.ToBytes` after Pin8 admits a stored part at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.

Each row is one part. `AddRestoredSceneObject(group, false, true, false)` runs on `new Scene(new RegionInfo())`. That constructor leaves `ClampPrimSize` false and `CollidablePrims` true, and the size fields at `Scene`'s defaults (nonphysical 0.001 to 256, physical 0.01 to 10), unless the row sets them. The group is built from `PrimitiveBaseShape.CreateBox` with the row's flags, path curve, state, pcode, physics shape and scale. A root whose stored physics shape is `none` is recorded as that stored value. `CreatePrimUpdateBlock` writes the same twelve bytes with `AddVector3` at offset 28 of `ObjectUpdate.ObjectData`.

Float columns are little-endian IEEE-754 hex, in the order `NonPhysicalPrimMin`, `NonPhysicalPrimMax`, `PhysicalPrimMin`, `PhysicalPrimMax`, then the stored scale. The last column is `Shape.Scale.ToBytes`. `a_stored_zero_scale_is_sent_as_pin8_bytes` admits a stored part with those inputs and compares the admitted scale and the encoded block.
