// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Reflection;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenMetaverse.StructuredData;

namespace Ferroan.ReferenceOracle;

internal sealed class Fixture
{
    public string Id;
    public string Family;
    public string Format;
    public string File;
    public byte[] Payload;
    public JsonObject Decoded;
    public JsonObject Params;
    public bool RoundTrip;
}

/// <summary>The fixed inputs the oracle encodes with OpenMetaverse.</summary>
internal static class Cases
{
    static readonly string[] Formats = { "xml", "binary", "notation" };

    internal static List<Fixture> Build()
    {
        AssertPublishedVectors();
        var list = new List<Fixture>();
        AddLlsd(list);
        AddBytes(list);
        PacketCases.Add(list);
        var ids = new HashSet<string>(StringComparer.Ordinal);
        foreach (Fixture fixture in list)
        {
            if (!ids.Add(fixture.Id))
                throw new InvalidOperationException("duplicate fixture id: " + fixture.Id);
            if (fixture.Payload.Length == 0 || fixture.Payload.Length > Codec.MaxPayload)
                throw new InvalidOperationException("fixture length out of range: " + fixture.Id);
        }

        return list;
    }

    /// <summary>
    /// Fails if this process is not linked to the OpenMetaverse build whose
    /// own tests publish these byte vectors (OpenMetaverse.Tests/BinaryLLSDTests.cs).
    /// </summary>
    static void AssertPublishedVectors()
    {
        byte[] head = "<?llsd/binary?>\n"u8.ToArray();
        Expect(
            "BinaryLLSDTests integer 1234843",
            OSDParser.SerializeLLSDBinary(OSD.FromInteger(1234843)),
            Combine(head, new byte[] { 0x69, 0x00, 0x12, 0xd7, 0x9b }));
        Expect(
            "BinaryLLSDTests real 947835.234",
            OSDParser.SerializeLLSDBinary(OSD.FromReal(947835.234d)),
            Combine(head, new byte[] { 0x72, 0x41, 0x2c, 0xec, 0xf6, 0x77, 0xce, 0xd9, 0x17 }));
        byte[] uuidBody =
        {
            0x97, 0xf4, 0xae, 0xca, 0x88, 0xa1, 0x42, 0xa1,
            0xb3, 0x85, 0xb9, 0x7b, 0x18, 0xab, 0xb2, 0x55,
        };
        Expect(
            "BinaryLLSDTests uuid",
            OSDParser.SerializeLLSDBinary(OSD.FromUUID(UUID.Parse("97f4aeca-88a1-42a1-b385-b97b18abb255"))),
            Combine(head, Combine(new byte[] { 0x75 }, uuidBody)));
        Expect(
            "UUID.GetBytes",
            UUID.Parse("97f4aeca-88a1-42a1-b385-b97b18abb255").GetBytes(),
            uuidBody);
    }

    static void AddLlsd(List<Fixture> list)
    {
        Add(list, "undef", () => new OSD());
        Add(list, "boolean-true", () => OSD.FromBoolean(true));
        Add(list, "boolean-false", () => OSD.FromBoolean(false));
        Add(list, "integer-zero", () => OSD.FromInteger(0));
        Add(list, "integer-one", () => OSD.FromInteger(1));
        Add(list, "integer-neg", () => OSD.FromInteger(-1));
        Add(list, "integer-min", () => OSD.FromInteger(int.MinValue));
        Add(list, "integer-max", () => OSD.FromInteger(int.MaxValue));
        Add(list, "integer-1234843", () => OSD.FromInteger(1234843));
        Add(list, "real-zero", () => OSD.FromReal(0d));
        Add(list, "real-neg-zero", () => OSD.FromReal(NegZero()));
        Add(list, "real-one-half", () => OSD.FromReal(1.5d));
        Add(list, "real-tenth", () => OSD.FromReal(0.1d));
        Add(list, "real-947835-234", () => OSD.FromReal(947835.234d));
        Add(list, "real-max", () => OSD.FromReal(double.MaxValue));
        Add(list, "real-epsilon", () => OSD.FromReal(double.Epsilon));
        Add(list, "real-nan", () => OSD.FromReal(double.NaN));
        Add(list, "real-pos-inf", () => OSD.FromReal(double.PositiveInfinity));
        Add(list, "real-neg-inf", () => OSD.FromReal(double.NegativeInfinity));
        Add(list, "string-empty", () => OSD.FromString(""));
        Add(list, "string-plain", () => OSD.FromString("widget"));
        Add(list, "string-escapes", () => OSD.FromString("caf\u00e9 <&> \"' \u2603 \U0001F682"));
        Add(list, "string-nul", () => OSD.FromString("a\0b"));
        Add(list, "uuid-zero", () => OSD.FromUUID(UUID.Zero));
        Add(list, "uuid-sample", () => OSD.FromUUID(UUID.Parse("97f4aeca-88a1-42a1-b385-b97b18abb255")));
        Add(list, "uuid-ordered", () => OSD.FromUUID(UUID.Parse("00112233-4455-6677-8899-aabbccddeeff")));
        Add(list, "date-seconds", () => OSD.FromDate(Utc(2020, 1, 2, 3, 4, 5)));
        Add(list, "date-millis", () => OSD.FromDate(Utc(2020, 1, 2, 3, 4, 5, 120)));
        Add(list, "date-millis-trunc", () => OSD.FromDate(Utc(2020, 1, 2, 3, 4, 5, 123)));
        Add(list, "uri-absolute", () => OSD.FromUri(new Uri("https://example.test/path?q=1")));
        Add(list, "binary-empty", () => OSD.FromBinary(Array.Empty<byte>()));
        Add(list, "binary-bytes", () => OSD.FromBinary(new byte[] { 0x00, 0x01, 0xff, 0x7f, (byte)'A' }));
        Add(list, "array-empty", () => new OSDArray());
        Add(list, "array-mixed", MixedArray);
        Add(list, "map-empty", () => new OSDMap());
        Add(list, "map-nested", NestedMap);
        Add(list, "nest-4", () => Nest(4));

        list.Add(Llsd("xml-formal", "map-nested", NestedMap()));
        list.Add(Llsd("binary-raw", "map-nested", NestedMap()));
        list.Add(Llsd("notation-header", "map-nested", NestedMap()));
    }

    static void Add(List<Fixture> list, string name, Func<OSD> make)
    {
        foreach (string format in Formats)
            list.Add(Llsd(format, name, make()));
    }

    static Fixture Llsd(string format, string name, OSD value)
    {
        Encoded encoded = Codec.Llsd(format, value);
        return new Fixture
        {
            Id = "llsd/" + format + "/" + name,
            Family = "llsd",
            Format = format,
            File = "llsd/" + format + "/" + name + ".bin",
            Payload = encoded.Payload,
            Decoded = encoded.Decoded,
            Params = encoded.Params,
            RoundTrip = encoded.RoundTrip,
        };
    }

    static OSD MixedArray()
    {
        var array = new OSDArray();
        array.Add(new OSD());
        array.Add(OSD.FromBoolean(false));
        array.Add(OSD.FromInteger(0));
        array.Add(OSD.FromReal(NegZero()));
        array.Add(OSD.FromString("x"));
        array.Add(OSD.FromUUID(UUID.Zero));
        return array;
    }

    static OSD NestedMap()
    {
        var child = new OSDMap();
        child["n"] = OSD.FromInteger(1);
        child["label"] = OSD.FromString("inner");

        var flags = new OSDArray();
        flags.Add(OSD.FromBoolean(true));
        flags.Add(OSD.FromInteger(-1));
        flags.Add(OSD.FromReal(0.5d));
        flags.Add(new OSD());

        // Insertion order is the wire order. "zeta" is first on purpose.
        var map = new OSDMap();
        map["zeta"] = OSD.FromString("second-key-first");
        map["alpha"] = OSD.FromInteger(7);
        map["id"] = OSD.FromUUID(UUID.Parse("97f4aeca-88a1-42a1-b385-b97b18abb255"));
        map["flags"] = flags;
        map["child"] = child;
        return map;
    }

    static OSD Nest(int depth)
    {
        OSD current = OSD.FromInteger(1);
        for (int i = 0; i < depth; i++)
        {
            var array = new OSDArray();
            array.Add(current);
            current = array;
        }

        return current;
    }

    static void AddBytes(List<Fixture> list)
    {
        var sample = UUID.Parse("97f4aeca-88a1-42a1-b385-b97b18abb255");
        var ordered = UUID.Parse("00112233-4455-6677-8899-aabbccddeeff");
        list.Add(Bytes("uuid-zero", "uuid", UUID.Zero.GetBytes(), null));
        list.Add(Bytes("uuid-sample", "uuid", sample.GetBytes(), null));
        list.Add(Bytes("uuid-ordered", "uuid", ordered.GetBytes(), null));

        list.Add(Bytes("vector2", "vector2", new Vector2(1.5f, -2f).GetBytes(), null));
        list.Add(Bytes("vector3", "vector3", new Vector3(1f, 2f, 3f).GetBytes(), null));
        list.Add(Bytes("vector3-tenth", "vector3", new Vector3(0.1f, -0.1f, 0.1f).GetBytes(), null));
        float negZero = BitConverter.UInt32BitsToSingle(0x80000000);
        list.Add(Bytes("vector3-neg-zero", "vector3", new Vector3(negZero, 1f, 2f).GetBytes(), null));
        list.Add(Bytes("vector3d", "vector3d", new Vector3d(1.5d, -2.25d, 4d).GetBytes(), null));
        list.Add(Bytes("vector4", "vector4", new Vector4(1f, 2f, 3f, 4f).GetBytes(), null));

        list.Add(Clamped("vector3-clamped-inside", new Vector3(0.25f, -0.5f, 0.125f), 1f));
        list.Add(Clamped("vector3-clamped-outside", new Vector3(2f, -3f, 0.5f), 1f));

        var identity = new Quaternion(0f, 0f, 0f, 1f);
        var tilt = new Quaternion(0.1f, -0.2f, 0.3f, 0.9f);
        var negW = new Quaternion(0f, 0.70710677f, 0f, -0.70710677f);
        foreach (var pair in new[] { ("identity", identity), ("tilt", tilt), ("neg-w", negW) })
        {
            list.Add(Bytes("quaternion-getbytes-" + pair.Item1, "quaternion-getbytes", pair.Item2.GetBytes(), null));
            byte[] toBytes = new byte[12];
            pair.Item2.ToBytes(toBytes, 0);
            list.Add(Bytes("quaternion-tobytes-" + pair.Item1, "quaternion-tobytes", toBytes, null));
            byte[] shorts = new byte[8];
            pair.Item2.ToShortsBytes(shorts, 0);
            list.Add(Bytes("quaternion-shorts-" + pair.Item1, "quaternion-shorts", shorts, null));
            list.Add(Bytes("quaternion-full-" + pair.Item1, "quaternion-full", FullQuaternion(pair.Item2), null));
        }

        var quantized = new Color4((byte)255, (byte)128, (byte)0, (byte)64);
        list.Add(Bytes("color4", "color4", quantized.GetBytes(), null));
        list.Add(Bytes("color4-inverted", "color4-inverted", quantized.GetBytes(true), null));
        var wide = new Color4(0.25f, 0.5f, 0.75f, 1f);
        list.Add(Bytes("color4-floats", "color4-floats", wide.GetFloatBytes(), null));

        list.Add(U16("float-u16-range-zero", "range", 0f, 1f, 0f, 0f));
        list.Add(U16("float-u16-range-half", "range", 0.5f, 1f, 0f, 0f));
        list.Add(U16("float-u16-range-neg-half", "range", -0.5f, 1f, 0f, 0f));
        list.Add(U16("float-u16-range-below", "range", -2f, 1f, 0f, 0f));
        list.Add(U16("float-u16-range-above", "range", 2f, 1f, 0f, 0f));
        list.Add(U16("float-u16-bounds-mid", "bounds", 0.25f, 0f, 0f, 1f));
        list.Add(U16("float-u16-bounds-low", "bounds", -1f, 0f, 0f, 1f));
        list.Add(U16("float-u16-bounds-high", "bounds", 2f, 0f, 0f, 1f));
        list.Add(U16("float-u16-unit-zero", "unit", 0f, 0f, 0f, 0f));
        list.Add(U16("float-u16-unit-quarter", "unit", 0.25f, 0f, 0f, 0f));
        list.Add(U16("float-u16-unit-one", "unit", 1f, 0f, 0f, 0f));
        list.Add(U16("float-u16-unit-neg", "unit", -1f, 0f, 0f, 0f));
        AddTextures(list);
        AddExtra(list);
        AddShapes(list);
        AddPermissions(list);
    }

    static void AddTextures(List<Fixture> list)
    {
        UUID def = UUID.Parse("00112233-4455-6677-8899-aabbccddeeff");
        UUID face = UUID.Parse("11223344-5566-7788-99aa-bbccddeeff00");
        UUID other = UUID.Parse("aabbccdd-eeff-0011-2233-445566778899");
        UUID mat = UUID.Parse("01020304-0506-0708-090a-0b0c0d0e0f10");
        UUID mat2 = UUID.Parse("ffeeddcc-bbaa-9988-7766-554433221100");

        list.Add(Texture("texture-default-only", def, 45, null, null));
        list.Add(Texture("texture-one-face", def, 45, null, new[]
        {
            new FaceRecipe { Index = 0, Texture = face },
        }));

        var red = new Color4((byte)255, (byte)0, (byte)0, (byte)255);
        list.Add(Texture("texture-shared", def, 45, null, new[]
        {
            new FaceRecipe { Index = 0, Texture = face },
            new FaceRecipe { Index = 3, Texture = face },
            new FaceRecipe { Index = 1, Rgba = red },
            new FaceRecipe { Index = 4, Rgba = red },
        }));

        list.Add(Texture(
            "texture-all-fields",
            def,
            45,
            new FaceRecipe
            {
                Rgba = new Color4((byte)200, (byte)100, (byte)50, (byte)255),
                RepeatU = 2f,
                RepeatV = 0.5f,
                OffsetU = 0.25f,
                OffsetV = -0.5f,
                Rotation = 1f,
                Glow = 0.25f,
                Bump = (int)Bumpiness.Bricks,
                Shiny = (int)Shininess.Medium,
                Fullbright = true,
                Media = true,
                Mapping = (int)MappingType.Planar,
                Material = mat,
            },
            new[]
            {
                new FaceRecipe
                {
                    Index = 2,
                    Texture = other,
                    Rgba = new Color4(0.25f, 0.5f, 0.75f, 1f),
                    RepeatU = 3f,
                    RepeatV = 4f,
                    OffsetU = 0.1f,
                    OffsetV = -0.2f,
                    Rotation = 2f,
                    Glow = 0.5f,
                    Bump = (int)Bumpiness.Woodgrain,
                    Shiny = (int)Shininess.High,
                    Fullbright = false,
                    Media = false,
                    Mapping = (int)MappingType.Spherical,
                    Material = mat2,
                },
            }));

        list.Add(Texture("texture-face-40", def, 45, null, new[]
        {
            new FaceRecipe { Index = 40, Texture = face },
        }));
        list.Add(Texture("texture-max-9", def, 9, null, new[]
        {
            new FaceRecipe { Index = 8, Texture = face },
            new FaceRecipe { Index = 12, Texture = other },
        }));
        list.Add(Bytes("texture-short", "texture-entry", new byte[8], null));
    }

    static void AddExtra(List<Fixture> list)
    {
        UUID tex = UUID.Parse("11223344-5566-7788-99aa-bbccddeeff00");
        var flex = new FlexRecipe
        {
            Softness = 3,
            Tension = 2f,
            Drag = 0.5f,
            Gravity = 1f,
            Wind = 0.8f,
            Force = new Vector3(1f, -2f, 0.5f),
        };
        var light = new LightRecipe
        {
            Color = new Color4((byte)255, (byte)128, (byte)64, (byte)255),
            Intensity = 0.5f,
            Radius = 5f,
            Cutoff = 1.25f,
            Falloff = 0.25f,
        };
        var image = new LightImageRecipe
        {
            Texture = tex,
            Params = new Vector3(0.5f, 1f, -1f),
        };

        list.Add(Extra("extra-none", null, null, null, null));
        list.Add(Extra("extra-flexible", flex, null, null, null));
        list.Add(Extra("extra-light", null, light, null, null));
        list.Add(Extra("extra-light-image", null, null, image, null));
        list.Add(Extra("extra-sculpt-sphere", null, null, null, new SculptRecipe { Texture = tex, TypeByte = (byte)SculptType.Sphere }));
        list.Add(Extra("extra-sculpt-mesh", null, null, null, new SculptRecipe { Texture = tex, TypeByte = (byte)SculptType.Mesh }));
        list.Add(Extra(
            "extra-sculpt-flags",
            null,
            null,
            null,
            new SculptRecipe
            {
                Texture = tex,
                TypeByte = (byte)(SculptType.Sphere | SculptType.Invert | SculptType.Mirror),
            }));
        list.Add(Extra(
            "extra-sculpt-mesh-invert",
            null,
            null,
            null,
            new SculptRecipe { Texture = tex, TypeByte = (byte)(SculptType.Mesh | SculptType.Invert) }));
        list.Add(Extra("extra-all", flex, light, image, new SculptRecipe { Texture = tex, TypeByte = (byte)SculptType.Sphere }));

        byte[] unknown = WrapMany(
            (0x70, new byte[] { 1, 2, 3, 4 }),
            ((ushort)ExtraParamType.Flexible, flex.Build().GetBytes()));
        list.Add(Bytes("extra-skip-unknown", "extra-params", unknown, null));
    }

    static Fixture Texture(string name, UUID texture, int maxFaces, FaceRecipe defaults, FaceRecipe[] faces)
    {
        var recipe = new JsonObject
        {
            ["build"] = true,
            ["max_faces"] = maxFaces,
            ["texture_id"] = texture.ToString(),
        };
        var entry = new Primitive.TextureEntry(texture);
        if (defaults != null)
        {
            defaults.Apply(entry.DefaultTexture);
            recipe["default"] = defaults.ToJson(false);
        }

        if (faces != null && faces.Length > 0)
        {
            var array = new JsonArray();
            foreach (FaceRecipe face in faces)
            {
                face.Apply(entry.CreateFace((uint)face.Index.Value));
                array.Add(face.ToJson(true));
            }

            recipe["faces"] = array;
        }

        return Bytes(name, "texture-entry", entry.GetBytes(maxFaces), recipe);
    }

    static Fixture Extra(
        string name,
        FlexRecipe flex,
        LightRecipe light,
        LightImageRecipe image,
        SculptRecipe sculpt)
    {
        var prim = new Primitive();
        var recipe = new JsonObject { ["build"] = true };
        if (flex != null)
        {
            prim.Flexible = flex.Build();
            recipe["flexible"] = flex.ToJson();
        }

        if (light != null)
        {
            prim.Light = light.Build();
            recipe["light"] = light.ToJson();
        }

        if (image != null)
        {
            prim.LightMap = image.Build();
            recipe["light_image"] = image.ToJson();
        }

        if (sculpt != null)
        {
            prim.Sculpt = sculpt.Build();
            recipe["sculpt"] = sculpt.ToJson();
        }

        return Bytes(name, "extra-params", prim.GetExtraParamsBytes(), recipe);
    }

    static byte[] WrapMany(params (ushort type, byte[] payload)[] parts)
    {
        int size = 1;
        foreach ((ushort _, byte[] payload) in parts)
            size += 6 + payload.Length;
        byte[] buf = new byte[size];
        buf[0] = (byte)parts.Length;
        int pos = 1;
        foreach ((ushort type, byte[] payload) in parts)
        {
            Buffer.BlockCopy(Utils.UInt16ToBytes(type), 0, buf, pos, 2);
            pos += 2;
            Buffer.BlockCopy(Utils.UIntToBytes((uint)payload.Length), 0, buf, pos, 4);
            pos += 4;
            Buffer.BlockCopy(payload, 0, buf, pos, payload.Length);
            pos += payload.Length;
        }

        return buf;
    }

    sealed class FaceRecipe
    {
        public int? Index;
        public UUID? Texture;
        public Color4? Rgba;
        public float? RepeatU;
        public float? RepeatV;
        public float? OffsetU;
        public float? OffsetV;
        public float? Rotation;
        public float? Glow;
        public int? Bump;
        public int? Shiny;
        public bool? Fullbright;
        public bool? Media;
        public int? Mapping;
        public UUID? Material;

        public void Apply(Primitive.TextureEntryFace face)
        {
            if (Texture.HasValue) face.TextureID = Texture.Value;
            if (Rgba.HasValue) face.RGBA = Rgba.Value;
            if (RepeatU.HasValue) face.RepeatU = RepeatU.Value;
            if (RepeatV.HasValue) face.RepeatV = RepeatV.Value;
            if (OffsetU.HasValue) face.OffsetU = OffsetU.Value;
            if (OffsetV.HasValue) face.OffsetV = OffsetV.Value;
            if (Rotation.HasValue) face.Rotation = Rotation.Value;
            if (Glow.HasValue) face.Glow = Glow.Value;
            if (Bump.HasValue) face.Bump = (Bumpiness)Bump.Value;
            if (Shiny.HasValue) face.Shiny = (Shininess)Shiny.Value;
            if (Fullbright.HasValue) face.Fullbright = Fullbright.Value;
            if (Media.HasValue) face.MediaFlags = Media.Value;
            if (Mapping.HasValue) face.TexMapType = (MappingType)Mapping.Value;
            if (Material.HasValue) face.MaterialID = Material.Value;
        }

        public JsonObject ToJson(bool includeIndex)
        {
            var node = new JsonObject();
            if (includeIndex) node["index"] = Index.Value;
            if (Texture.HasValue) node["texture_id"] = Texture.Value.ToString();
            if (Rgba.HasValue)
            {
                node["r_bits"] = Codec.F32(Rgba.Value.R);
                node["g_bits"] = Codec.F32(Rgba.Value.G);
                node["b_bits"] = Codec.F32(Rgba.Value.B);
                node["a_bits"] = Codec.F32(Rgba.Value.A);
            }

            if (RepeatU.HasValue) node["repeat_u_bits"] = Codec.F32(RepeatU.Value);
            if (RepeatV.HasValue) node["repeat_v_bits"] = Codec.F32(RepeatV.Value);
            if (OffsetU.HasValue) node["offset_u_bits"] = Codec.F32(OffsetU.Value);
            if (OffsetV.HasValue) node["offset_v_bits"] = Codec.F32(OffsetV.Value);
            if (Rotation.HasValue) node["rotation_bits"] = Codec.F32(Rotation.Value);
            if (Glow.HasValue) node["glow_bits"] = Codec.F32(Glow.Value);
            if (Bump.HasValue) node["bump"] = Bump.Value;
            if (Shiny.HasValue) node["shiny"] = Shiny.Value;
            if (Fullbright.HasValue) node["fullbright"] = Fullbright.Value;
            if (Media.HasValue) node["media"] = Media.Value;
            if (Mapping.HasValue) node["mapping"] = Mapping.Value;
            if (Material.HasValue) node["material_id"] = Material.Value.ToString();
            return node;
        }
    }

    sealed class FlexRecipe
    {
        public int Softness;
        public float Tension;
        public float Drag;
        public float Gravity;
        public float Wind;
        public Vector3 Force;

        public Primitive.FlexibleData Build()
        {
            return new Primitive.FlexibleData
            {
                Softness = Softness,
                Tension = Tension,
                Drag = Drag,
                Gravity = Gravity,
                Wind = Wind,
                Force = Force,
            };
        }

        public JsonObject ToJson()
        {
            return new JsonObject
            {
                ["softness"] = Softness,
                ["tension_bits"] = Codec.F32(Tension),
                ["drag_bits"] = Codec.F32(Drag),
                ["gravity_bits"] = Codec.F32(Gravity),
                ["wind_bits"] = Codec.F32(Wind),
                ["force_bits"] = new JsonArray { Codec.F32(Force.X), Codec.F32(Force.Y), Codec.F32(Force.Z) },
            };
        }
    }

    sealed class LightRecipe
    {
        public Color4 Color;
        public float Intensity;
        public float Radius;
        public float Cutoff;
        public float Falloff;

        public Primitive.LightData Build()
        {
            return new Primitive.LightData
            {
                Color = Color,
                Intensity = Intensity,
                Radius = Radius,
                Cutoff = Cutoff,
                Falloff = Falloff,
            };
        }

        public JsonObject ToJson()
        {
            return new JsonObject
            {
                ["r_bits"] = Codec.F32(Color.R),
                ["g_bits"] = Codec.F32(Color.G),
                ["b_bits"] = Codec.F32(Color.B),
                ["a_bits"] = Codec.F32(Color.A),
                ["intensity_bits"] = Codec.F32(Intensity),
                ["radius_bits"] = Codec.F32(Radius),
                ["cutoff_bits"] = Codec.F32(Cutoff),
                ["falloff_bits"] = Codec.F32(Falloff),
            };
        }
    }

    sealed class LightImageRecipe
    {
        public UUID Texture;
        public Vector3 Params;

        public Primitive.LightImage Build()
        {
            return new Primitive.LightImage
            {
                LightTexture = Texture,
                Params = Params,
            };
        }

        public JsonObject ToJson()
        {
            return new JsonObject
            {
                ["texture_id"] = Texture.ToString(),
                ["params_bits"] = new JsonArray { Codec.F32(Params.X), Codec.F32(Params.Y), Codec.F32(Params.Z) },
            };
        }
    }

    sealed class SculptRecipe
    {
        public UUID Texture;
        public byte TypeByte;

        public Primitive.SculptData Build()
        {
            var sculpt = new Primitive.SculptData { SculptTexture = Texture };
            // Invert and Mirror have getters only. GetBytes writes this private byte.
            FieldInfo field = typeof(Primitive.SculptData).GetField(
                "type",
                BindingFlags.Instance | BindingFlags.NonPublic);
            if (field == null)
                throw new InvalidOperationException("SculptData.type is not available");
            field.SetValue(sculpt, TypeByte);
            return sculpt;
        }

        public JsonObject ToJson()
        {
            return new JsonObject
            {
                ["texture_id"] = Texture.ToString(),
                ["type_byte"] = (int)TypeByte,
            };
        }
    }

    static Fixture Clamped(string name, Vector3 value, float range)
    {
        byte[] payload = new byte[6];
        value.ClampedToShortsBytes(range, payload, 0);
        var parms = new JsonObject();
        parms["range_bits"] = Codec.F32(range);
        return Bytes(name, "vector3-clamped", payload, parms);
    }

    static byte[] FullQuaternion(Quaternion value)
    {
        byte[] payload = new byte[16];
        Utils.FloatToBytes(value.X, payload, 0);
        Utils.FloatToBytes(value.Y, payload, 4);
        Utils.FloatToBytes(value.Z, payload, 8);
        Utils.FloatToBytes(value.W, payload, 12);
        return payload;
    }

    static Fixture U16(string name, string mode, float input, float range, float lower, float upper)
    {
        ushort bits;
        var parms = new JsonObject();
        parms["mode"] = mode;
        parms["input_bits"] = Codec.F32(input);
        if (mode == "range")
        {
            bits = Utils.FloatToUInt16(input, range);
            parms["range_bits"] = Codec.F32(range);
        }
        else if (mode == "bounds")
        {
            bits = Utils.FloatToUInt16(input, lower, upper);
            parms["lower_bits"] = Codec.F32(lower);
            parms["upper_bits"] = Codec.F32(upper);
        }
        else if (mode == "unit")
        {
            bits = Utils.FloatToUnitUInt16(input);
        }
        else
        {
            throw new InvalidOperationException("unknown float-u16 mode: " + mode);
        }

        return Bytes(name, "float-u16", Utils.UInt16ToBytes(bits), parms);
    }

    static void AddShapes(List<Fixture> list)
    {
        list.Add(Shape(
            "shape-box", PathCurve.Line, ProfileCurve.Square, HoleType.Same,
            0f, 1f, 1f, 1f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 1f, 0f, 0f, 1f, 0f));
        list.Add(Shape(
            "shape-mixed", PathCurve.Circle, ProfileCurve.Circle, HoleType.Square,
            0.25f, 0.75f, 0.5f, 1.5f, 0.1f, -0.2f, 0.3f, -0.4f, 0.15f, 0.25f, -0.5f, 1.5f, -0.05f,
            0.2f, 0.8f, 0.5f));
        list.Add(Shape(
            "shape-midpoint", PathCurve.Line, ProfileCurve.Square, HoleType.Same,
            0.00005f, 0.00007f, 0.025f, 0.035f, 0.025f, 0.035f, 0.025f, 0.035f, 0.025f, 0.025f, 0.035f,
            1.0375f, 0.025f, 0.00005f, 0.00007f, 0.00005f));
        list.Add(Shape(
            "shape-wrap", PathCurve.Flexible, ProfileCurve.HalfCircle, HoleType.Triangle,
            0f, 1f, 2.6f, -0.2f, 2f, -2f, 2f, -2f, 2f, 2f, -2f, 4f, 2f, 0f, 1f, 1.2f));
        list.Add(Attachment("attachment-chest", AttachmentPoint.Chest));
        list.Add(Attachment("attachment-nibbles", (AttachmentPoint)0x2A));
    }

    static Fixture Shape(
        string name,
        PathCurve path,
        ProfileCurve profile,
        HoleType hole,
        float pathBegin,
        float pathEnd,
        float scaleX,
        float scaleY,
        float shearX,
        float shearY,
        float twist,
        float twistBegin,
        float radius,
        float taperX,
        float taperY,
        float revolutions,
        float skew,
        float profileBegin,
        float profileEnd,
        float hollow)
    {
        var data = new Primitive.ConstructionData();
        data.PathCurve = path;
        data.ProfileCurve = profile;
        data.ProfileHole = hole;
        byte[] bytes = new byte[23];
        bytes[0] = (byte)data.PathCurve;
        bytes[1] = data.profileCurve;
        Utils.UInt16ToBytes(Primitive.PackBeginCut(pathBegin)).CopyTo(bytes, 2);
        Utils.UInt16ToBytes(Primitive.PackEndCut(pathEnd)).CopyTo(bytes, 4);
        bytes[6] = Primitive.PackPathScale(scaleX);
        bytes[7] = Primitive.PackPathScale(scaleY);
        bytes[8] = (byte)Primitive.PackPathShear(shearX);
        bytes[9] = (byte)Primitive.PackPathShear(shearY);
        bytes[10] = (byte)Primitive.PackPathTwist(twist);
        bytes[11] = (byte)Primitive.PackPathTwist(twistBegin);
        bytes[12] = (byte)Primitive.PackPathTwist(radius);
        bytes[13] = (byte)Primitive.PackPathTaper(taperX);
        bytes[14] = (byte)Primitive.PackPathTaper(taperY);
        bytes[15] = Primitive.PackPathRevolutions(revolutions);
        bytes[16] = (byte)Primitive.PackPathTwist(skew);
        Utils.UInt16ToBytes(Primitive.PackBeginCut(profileBegin)).CopyTo(bytes, 17);
        Utils.UInt16ToBytes(Primitive.PackEndCut(profileEnd)).CopyTo(bytes, 19);
        Utils.UInt16ToBytes(Primitive.PackProfileHollow(hollow)).CopyTo(bytes, 21);
        var parms = new JsonObject
        {
            ["build"] = true,
            ["path_curve"] = bytes[0],
            ["profile_curve"] = bytes[1],
            ["path_begin_bits"] = Codec.F32(pathBegin),
            ["path_end_bits"] = Codec.F32(pathEnd),
            ["path_scale_x_bits"] = Codec.F32(scaleX),
            ["path_scale_y_bits"] = Codec.F32(scaleY),
            ["path_shear_x_bits"] = Codec.F32(shearX),
            ["path_shear_y_bits"] = Codec.F32(shearY),
            ["path_twist_bits"] = Codec.F32(twist),
            ["path_twist_begin_bits"] = Codec.F32(twistBegin),
            ["path_radius_offset_bits"] = Codec.F32(radius),
            ["path_taper_x_bits"] = Codec.F32(taperX),
            ["path_taper_y_bits"] = Codec.F32(taperY),
            ["path_revolutions_bits"] = Codec.F32(revolutions),
            ["path_skew_bits"] = Codec.F32(skew),
            ["profile_begin_bits"] = Codec.F32(profileBegin),
            ["profile_end_bits"] = Codec.F32(profileEnd),
            ["profile_hollow_bits"] = Codec.F32(hollow),
        };
        return Bytes(name, "shape", bytes, parms);
    }

    static Fixture Attachment(string name, AttachmentPoint point)
    {
        var data = new Primitive.ConstructionData();
        data.AttachmentPoint = point;
        var parms = new JsonObject
        {
            ["build"] = true,
            ["attachment"] = (int)point,
        };
        return Bytes(name, "attachment-state", new[] { data.State }, parms);
    }

    static void AddPermissions(List<Fixture> list)
    {
        AssertSharedMasks();
        uint all = (uint)OpenSim.Framework.PermissionMask.All;
        uint export = (uint)OpenSim.Framework.PermissionMask.Export;
        uint modify = (uint)OpenSim.Framework.PermissionMask.Modify;
        uint copy = (uint)OpenSim.Framework.PermissionMask.Copy;
        uint transfer = (uint)OpenSim.Framework.PermissionMask.Transfer;
        uint noMod = (uint)OpenSim.Framework.PermissionMask.AllAndExportNoMod;
        uint[] masks =
        {
            all,
            all | export,
            modify | copy,
            transfer,
            noMod,
        };
        byte[] blob = new byte[20];
        for (int i = 0; i < masks.Length; i++)
            Utils.UIntToBytes(masks[i]).CopyTo(blob, i * 4);
        var parms = new JsonObject
        {
            ["build"] = true,
            ["base"] = masks[0],
            ["owner"] = masks[1],
            ["group"] = masks[2],
            ["everyone"] = masks[3],
            ["next_owner"] = masks[4],
        };
        list.Add(Bytes("permissions-object", "permissions", blob, parms));
        list.Add(MaskConstants());
        uint foldedCopyTransfer = (uint)(OpenSim.Framework.PermissionMask.FoldedCopy | OpenSim.Framework.PermissionMask.FoldedTransfer);
        uint foldedCopy = (uint)OpenSim.Framework.PermissionMask.FoldedCopy;
        list.Add(Fold("permission-fold-apply", "apply", foldedCopyTransfer, all));
        list.Add(Fold("permission-fold-ignore-empty", "apply", 0, all));
        list.Add(Fold("permission-fold-ignore-full", "apply", (uint)OpenSim.Framework.PermissionMask.FoldedMask, all));
        list.Add(Fold("permission-fold-nomod", "nomod", foldedCopy, all));
        list.Add(Fold("permission-fold-fix", "fix", 0, modify));
        list.Add(Fold("permission-fold-fix-neither", "fix", 0, (uint)OpenSim.Framework.PermissionMask.Slam));
    }

    static void AssertSharedMasks()
    {
        Same(PermissionMask.None, OpenSim.Framework.PermissionMask.None, "None");
        Same(PermissionMask.Transfer, OpenSim.Framework.PermissionMask.Transfer, "Transfer");
        Same(PermissionMask.Modify, OpenSim.Framework.PermissionMask.Modify, "Modify");
        Same(PermissionMask.Copy, OpenSim.Framework.PermissionMask.Copy, "Copy");
        Same(PermissionMask.Export, OpenSim.Framework.PermissionMask.Export, "Export");
        Same(PermissionMask.Move, OpenSim.Framework.PermissionMask.Move, "Move");
        Same(PermissionMask.Damage, OpenSim.Framework.PermissionMask.Damage, "Damage");
        Same(PermissionMask.All, OpenSim.Framework.PermissionMask.All, "All");
    }

    static void Same(PermissionMask omv, OpenSim.Framework.PermissionMask tranq, string name)
    {
        if ((uint)omv != (uint)tranq)
            throw new InvalidOperationException("permission bit " + name + " differs between OpenMetaverse and Tranquillity");
    }

    static Fixture MaskConstants()
    {
        OpenSim.Framework.PermissionMask[] names =
        {
            OpenSim.Framework.PermissionMask.None,
            OpenSim.Framework.PermissionMask.FoldedTransfer,
            OpenSim.Framework.PermissionMask.FoldedModify,
            OpenSim.Framework.PermissionMask.FoldedCopy,
            OpenSim.Framework.PermissionMask.FoldedExport,
            OpenSim.Framework.PermissionMask.Slam,
            OpenSim.Framework.PermissionMask.FoldedMask,
            (OpenSim.Framework.PermissionMask)OpenSim.Framework.PermissionMask.FoldingShift,
            OpenSim.Framework.PermissionMask.Transfer,
            OpenSim.Framework.PermissionMask.Modify,
            OpenSim.Framework.PermissionMask.Copy,
            OpenSim.Framework.PermissionMask.Export,
            OpenSim.Framework.PermissionMask.Move,
            OpenSim.Framework.PermissionMask.Damage,
            OpenSim.Framework.PermissionMask.All,
            OpenSim.Framework.PermissionMask.AllAndExport,
            OpenSim.Framework.PermissionMask.AllAndExportNoMod,
            OpenSim.Framework.PermissionMask.AllEffective,
            OpenSim.Framework.PermissionMask.UnfoldedMask,
        };
        byte[] blob = new byte[names.Length * 4];
        for (int i = 0; i < names.Length; i++)
            Utils.UIntToBytes((uint)names[i]).CopyTo(blob, i * 4);
        return Bytes("permission-masks", "permission-masks", blob, null);
    }

    static Fixture Fold(string name, string op, uint source, uint target)
    {
        uint result = target;
        if (op == "apply")
            OpenSim.Framework.PermissionsUtil.ApplyFoldedPermissions(source, ref result);
        else if (op == "nomod")
            OpenSim.Framework.PermissionsUtil.ApplyNoModFoldedPermissions(source, ref result);
        else if (op == "fix")
            result = OpenSim.Framework.PermissionsUtil.FixAndFoldPermissions(target);
        else
            throw new InvalidOperationException("unknown fold op: " + op);
        var parms = new JsonObject
        {
            ["build"] = true,
            ["op"] = op,
            ["source"] = source,
            ["target"] = target,
        };
        return Bytes(name, "permission-fold", Utils.UIntToBytes(result), parms);
    }

    static Fixture Bytes(string name, string format, byte[] payload, JsonObject parms)
    {
        JsonObject decoded = Codec.Decode("bytes", format, payload, parms);
        if (Codec.IsError(decoded))
        {
            throw new InvalidOperationException(
                "byte fixture did not decode: " + name + " " + decoded.ToJsonString());
        }

        return new Fixture
        {
            Id = "bytes/" + name,
            Family = "bytes",
            Format = format,
            File = "bytes/" + name + ".bin",
            Payload = payload,
            Decoded = decoded,
            Params = parms,
            RoundTrip = Codec.RoundTrips("bytes", format, payload, parms),
        };
    }

    static DateTime Utc(int year, int month, int day, int hour, int minute, int second, int millisecond = 0)
    {
        return new DateTime(year, month, day, hour, minute, second, millisecond, DateTimeKind.Utc);
    }

    static double NegZero()
    {
        return BitConverter.UInt64BitsToDouble(0x8000000000000000);
    }

    static byte[] Combine(byte[] left, byte[] right)
    {
        byte[] both = new byte[left.Length + right.Length];
        Buffer.BlockCopy(left, 0, both, 0, left.Length);
        Buffer.BlockCopy(right, 0, both, left.Length, right.Length);
        return both;
    }

    static void Expect(string name, byte[] actual, byte[] expected)
    {
        if (!actual.AsSpan().SequenceEqual(expected))
        {
            throw new InvalidOperationException(
                name + " mismatch\n  actual   " + Convert.ToHexString(actual).ToLowerInvariant()
                + "\n  expected " + Convert.ToHexString(expected).ToLowerInvariant());
        }
    }
}
