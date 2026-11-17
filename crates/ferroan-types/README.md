# ferroan-types

Packet bytes for the OpenMetaverse value types, checked against `fixtures/oracle`.

`src/wire.rs` is ported from libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`:

- `OpenMetaverse.Types/UUID.cs`
- `Vector2.cs`, `Vector3.cs`, `Vector3d.cs`, `Vector4.cs`
- `Quaternion.cs`
- `Color4.cs`
- `UtilsConversions.cs` (`FloatToUInt16`, `FloatToUnitUInt16`, `UInt16ToFloat`, `FloatZeroOneToByte`, `UInt16ToBytes`)

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

A buffer shorter than the layout returns `WireError::Short`.

## Differences kept to match OpenMetaverse

- `GetBytes` substitutes zeros when `norm <= 1e-6` and `W >= 0.9999`. `ToBytes` substitutes zeros when `norm <= 1e-6` and `norm >= 0.9999`. Those two tests are not the same. A negative `W` flips the scale of X Y Z, so the component left out of the 12-byte form comes back positive.
- `FloatToUInt16(value, range)` and `FloatToUnitUInt16` multiply by the double `32767.5` and then truncate toward zero. `FloatToUInt16(value, lower, upper)` multiplies in single precision by `0xffff`.
- `FloatToUnitUInt16` compares the lower bound with a double `-1.0` and the upper bound with a float `1.0`.
- `UInt16ToFloat` snaps a result closer to zero than one step to `0`.
- `vector3-clamped-outside` does not round-trip. The bytes are `ClampedToShortsBytes` of `(2, -3, 0.5)` with range `1`. Decoding them and encoding the reconstructed floats does not restore the file. The test encodes that source vector.

## Not in this crate yet

TextureEntry, extra parameters, prim shape parameters and permission masks are P0.3b and P0.3c. They need oracle fixtures of their own.

A viewer has not been shown these bytes. The comparison is the committed oracle fixtures.
