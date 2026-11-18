# ferroan-types

Packet bytes for the OpenMetaverse value types, checked against `fixtures/oracle`.

`src/wire.rs`, `src/texture.rs`, `src/extra.rs` and `src/shape.rs` are ported from libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. `src/permissions.rs` is ported from Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`, and the shared permission bits match OpenMetaverse:

- `OpenMetaverse.Types/UUID.cs`
- `Vector2.cs`, `Vector3.cs`, `Vector3d.cs`, `Vector4.cs`
- `Quaternion.cs`
- `Color4.cs`
- `UtilsConversions.cs` (`FloatToUInt16`, `FloatToUnitUInt16`, `UInt16ToFloat`, `FloatZeroOneToByte`, `UInt16ToBytes`)
- `OpenMetaverse/Primitives/TextureEntry.cs` and `OpenMetaverse/Helpers.cs` (`TEOffsetShort`, `TEOffsetFloat`, `TERotationShort`, `TERotationFloat`, `TEGlowByte`, `TEGlowFloat`)
- `OpenMetaverse/Primitives/Primitive.cs` (`FlexibleData`, `LightData`, `LightImage`, `SculptData`, `GetExtraParamsBytes`, `SetExtraParamsFromBytes`, `PackBeginCut` and the other `Pack*` / `Unpack*` methods, the profile-curve nibbles) and `OpenMetaverse.Types/EnumsPrimitive.cs`
- `OpenMetaverse.Types/UtilsConversions.cs` (`SwapNibbles`) and `OpenMetaverse/Permissions.cs`
- Tranquillity `Source/OpenSim.Framework/Util.cs` (`PermissionMask`) and `PermissionsUtil.cs` (`ApplyFoldedPermissions`, `ApplyNoModFoldedPermissions`, `FixAndFoldPermissions`)

Vectors and quaternions are `glam` 0.34 values. Each field is written on its own, little-endian. A `glam` struct is not the wire layout. `Color4` is a plain struct.

`REFERENCE_TRANQUILLITY_REVISION` stays `713857f10b8e91a639a967d4354a4682b080793c`.

## What the bytes are

| Call | OpenMetaverse |
| --- | --- |
| `write_uuid` | `UUID.GetBytes`, network order |
| `write_vec2`, `write_vec3`, `write_dvec3`, `write_vec4` | `GetBytes`, little-endian fields, including negative zero |
| `write_vec3_clamped` | `Vector3.ClampedToShortsBytes` |
| `write_quaternion_get_bytes` | `Quaternion.GetBytes` (12 bytes, X Y Z) |
| `write_quaternion_to_bytes` | `Quaternion.ToBytes` |
| `write_quaternion_shorts` | `Quaternion.ToShortsBytes` |
| `write_quaternion_full` | four little-endian floats |
| `write_color4` | `Color4.ToBytes`, optionally inverted |
| `write_color4_floats` | `Color4.GetFloatBytes` |
| `float_to_u16_range` | `Utils.FloatToUInt16(value, range)` |
| `float_to_u16_bounds` | `Utils.FloatToUInt16(value, lower, upper)` |
| `float_to_unit_u16` | `Utils.FloatToUnitUInt16` |
| `TextureEntry::to_bytes` | `TextureEntry.GetBytes` |
| `TextureEntry::from_bytes` | `new TextureEntry(bytes, 0, length)` |
| `ExtraParams::to_bytes` | `Primitive.GetExtraParamsBytes` |
| `ExtraParams::from_bytes` | `Primitive.SetExtraParamsFromBytes` |
| `ShapeParams::to_bytes` | `Primitive.Pack*` in `ObjectAdd` field order |
| `ShapeParams::from_bytes` | `Primitive.Unpack*` |
| `swap_nibbles` | `Utils.SwapNibbles`, which `AttachmentPoint` uses for `State` |
| `Permissions::to_bytes` | five little-endian masks: base, owner, group, everyone, next owner |
| `apply_folded_permissions` | `PermissionsUtil.ApplyFoldedPermissions` |

A buffer shorter than a fixed layout returns `WireError::Short`. A texture entry shorter than 16 bytes has no default face, and `to_bytes` of that value is empty. A texture entry or extra-parameter blob over 64 KiB returns `WireError::Limit`. OpenMetaverse does not apply that cap. A face bitfield stops after eight 7-bit groups; a 45-face mask fits in seven.

## Differences kept to match OpenMetaverse

- `GetBytes` substitutes zeros when `norm <= 1e-6` and `W >= 0.9999`. `ToBytes` substitutes zeros when `norm <= 1e-6` and `norm >= 0.9999`. Those two tests are not the same. A negative `W` flips the scale of X Y Z, so the component left out of the 12-byte form comes back positive.
- `FloatToUInt16(value, range)` and `FloatToUnitUInt16` multiply by the double `32767.5` and then truncate toward zero. `FloatToUInt16(value, lower, upper)` multiplies in single precision by `0xffff`.
- `FloatToUnitUInt16` compares the lower bound with a double `-1.0` and the upper bound with a float `1.0`.
- `UInt16ToFloat` snaps a result closer to zero than one step to `0`.
- `vector3-clamped-outside` does not round-trip. The bytes are `ClampedToShortsBytes` of `(2, -3, 0.5)` with range `1`. Decoding them and encoding the reconstructed floats does not restore the file. The test encodes that source vector.

## Texture entry

`TextureEntry.GetBytes` writes a face field only when that face's attribute bit is set and the stored value differs from the default face. Null faces are omitted. Colour bytes are inverted. Offset, rotation and glow stay as the short or byte from the wire: `TERotationShort(TERotationFloat(s))` is not the identity, so encoding the public float again would not restore the file.

Aurora's `aurora-prim` `te.rs` is the viewer packer. It treats the last face as the default and emits every face. Those bytes do not match OpenMetaverse, so it is not copied here. The 7-bit face groups are the same idea; the writer here follows `WriteFaceBitfieldBytes`.

`GetBakesBytes` is a different encoder and is not implemented. Object updates use `GetBytes`.

## Extra parameters

`GetExtraParamsBytes` writes flexible (`0x10`), light (`0x20`), light image (`0x40`), and sculpt (`0x30`) or mesh (`0x60`) when the sculpt type's low three bits are mesh. Extended mesh (`0x70`), render materials (`0x80`) and reflection probes (`0x90`) are not written. Decode skips an unknown type by its declared length, and encoding drops it. Aurora's `extra.rs` parses those viewer blocks; this crate does not treat them as OpenMetaverse output.

A light stores intensity in the colour alpha and sets `Color.A` to 1 on decode. Flexible tension, drag, gravity and wind are written with `* 10.01` and read back with `/ 10`. Sculpt invert (`64`) and mirror (`128`) live in the same byte as the type. OpenMetaverse's `Type` setter replaces that whole byte; the raw byte is what `GetBytes` writes. A sculpt whose low three bits are mesh uses the mesh parameter type even when invert or mirror is set.

`SculptData` and `FlexibleData` in C# decide whether the payload is present by looking at the length of the whole buffer, not the bytes remaining at the read position. A payload that runs past the buffer is `WireError::Short` here.

## Shape parameters

`ObjectAdd` writes `PathCurve` through `ProfileHollow` as 23 bytes. `MathF.Round` sends a midpoint to the even integer, and the cast keeps the low bits of that integer (`(byte)260f` is `4`). Aurora's `pack.rs` rounds midpoints away from zero and then clamps, so it is not copied.

The profile byte keeps the hole in the high nibble and the curve in the low nibble. `AttachmentPoint` is stored in `State` with the nibbles swapped. Unpacking a packed field is not always the float that was packed. Encoding the source float is what restores the file. Packing the unpacked float is what `again_hex` records.

## Permission masks

Tranquillity's `PermissionMask` includes OpenMetaverse's bits at the same values. The oracle refuses to generate if `None`, `Transfer`, `Modify`, `Copy`, `Export`, `Move`, `Damage` or `All` differ. `All` does not include `Export`. The folded low bits, `Slam` and the named combinations are Tranquillity's. `ApplyFoldedPermissions` leaves the target alone when the folded source is `0` or every folded bit.

The direct byte comparison uses the committed oracle fixtures in Rust tests on all three CI platforms. The [Linux live workflow](../../.github/workflows/live.yml) additionally runs `cargo xtask oracle verify` against the pinned C# implementation. Fixture success alone is not a viewer check.
