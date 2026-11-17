// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

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
