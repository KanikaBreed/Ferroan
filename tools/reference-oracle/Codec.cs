// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Encodes and decodes through the pinned libopenmetaverse checkout. This file is
// not a port; the C# library remains the encoder under test.
//
//   OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs
//   OpenMetaverse.StructuredData/LLSD/BinaryLLSD.cs
//   OpenMetaverse.StructuredData/LLSD/NotationLLSD.cs
//   OpenMetaverse.Types/UUID.cs
//   OpenMetaverse.Types/Vector2.cs
//   OpenMetaverse.Types/Vector3.cs
//   OpenMetaverse.Types/Vector3d.cs
//   OpenMetaverse.Types/Vector4.cs
//   OpenMetaverse.Types/Quaternion.cs
//   OpenMetaverse.Types/Color4.cs
//   OpenMetaverse.Types/UtilsConversions.cs

using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenMetaverse.StructuredData;

namespace Ferroan.ReferenceOracle;

/// <summary>
/// One call into OpenMetaverse, plus the value C# produces when it reads those bytes back.
/// </summary>
internal readonly struct Encoded
{
    public byte[] Payload { get; init; }
    public JsonObject Decoded { get; init; }
    public JsonObject Params { get; init; }
    public bool RoundTrip { get; init; }
}

/// <summary>OpenMetaverse encode and decode used by both generate and verify.</summary>
internal static class Codec
{
    internal const int MaxPayload = 16 * 1024 * 1024;

    internal static void UseOpenMetaverseCulture()
    {
        // OSDInteger.AsString and OSDDate.AsString follow the current culture
        // (the date format's ':' and '.' are culture-sensitive). Reals already
        // go through Utils.EnUsCulture. Force that same culture so fixture bytes
        // do not depend on the machine locale.
        CultureInfo culture = Utils.EnUsCulture;
        CultureInfo.DefaultThreadCurrentCulture = culture;
        CultureInfo.DefaultThreadCurrentUICulture = culture;
        Thread.CurrentThread.CurrentCulture = culture;
        Thread.CurrentThread.CurrentUICulture = culture;
    }

    internal static Encoded Llsd(string format, OSD value)
    {
        byte[] payload = EncodeLlsd(format, value);
        JsonObject decoded = Decode("llsd", format, payload, null);
        bool roundTrip = false;
        if (!IsError(decoded))
        {
            try
            {
                OSD again = DecodeLlsd(format, payload);
                roundTrip = EncodeLlsd(format, again).AsSpan().SequenceEqual(payload);
            }
            catch (Exception ex) when (IsDecodeFailure(ex))
            {
                roundTrip = false;
            }
        }

        return new Encoded
        {
            Payload = payload,
            Decoded = decoded,
            Params = null,
            RoundTrip = roundTrip,
        };
    }

    internal static JsonObject Decode(string family, string format, byte[] payload, JsonObject parms)
    {
        try
        {
            if (family == "llsd")
                return Canon(DecodeLlsd(format, payload));
            if (family == "bytes")
                return DecodeBytes(format, payload, parms ?? new JsonObject());
            throw new InvalidOperationException("unknown family: " + family);
        }
        catch (Exception ex) when (IsDecodeFailure(ex))
        {
            return Error(ex);
        }
    }

    internal static bool IsError(JsonObject node)
    {
        JsonNode type = node["type"];
        return type is not null && type.GetValue<string>() == "decode-error";
    }

    internal static string F32(float value)
    {
        return BitConverter.SingleToUInt32Bits(value).ToString("x8", CultureInfo.InvariantCulture);
    }

    internal static string F64(double value)
    {
        return BitConverter.DoubleToUInt64Bits(value).ToString("x16", CultureInfo.InvariantCulture);
    }

    internal static float ParseF32(string hex)
    {
        return BitConverter.UInt32BitsToSingle(uint.Parse(hex, NumberStyles.HexNumber, CultureInfo.InvariantCulture));
    }

    internal static byte[] EncodeLlsd(string format, OSD value)
    {
        switch (format)
        {
            case "xml":
                return OSDParser.SerializeLLSDXmlToBytes(value, false);
            case "xml-formal":
                return OSDParser.SerializeLLSDXmlToBytes(value, true);
            case "binary":
                return OSDParser.SerializeLLSDBinary(value, true);
            case "binary-raw":
                return OSDParser.SerializeLLSDBinary(value, false);
            case "notation":
                return OSDParser.SerializeLLSDNotationToBytes(value, false);
            case "notation-header":
                return OSDParser.SerializeLLSDNotationToBytes(value, true);
            default:
                throw new InvalidOperationException("unknown llsd format: " + format);
        }
    }

    static OSD DecodeLlsd(string format, byte[] payload)
    {
        switch (format)
        {
            case "xml":
            case "xml-formal":
                return OSDParser.DeserializeLLSDXml(payload);
            case "binary":
            case "binary-raw":
                return OSDParser.DeserializeLLSDBinary(payload);
            case "notation":
            case "notation-header":
                return OSDParser.DeserializeLLSDNotation(payload);
            default:
                throw new InvalidOperationException("unknown llsd format: " + format);
        }
    }

    static JsonObject Canon(OSD osd)
    {
        switch (osd.Type)
        {
            case OSDType.Unknown:
                return Type("undef");
            case OSDType.Boolean:
            {
                JsonObject node = Type("boolean");
                node["value"] = osd.AsBoolean();
                return node;
            }
            case OSDType.Integer:
            {
                JsonObject node = Type("integer");
                node["value"] = osd.AsInteger();
                return node;
            }
            case OSDType.Real:
            {
                JsonObject node = Type("real");
                node["bits"] = F64(osd.AsReal());
                return node;
            }
            case OSDType.String:
            {
                JsonObject node = Type("string");
                node["value"] = osd.AsString();
                return node;
            }
            case OSDType.UUID:
            {
                JsonObject node = Type("uuid");
                node["value"] = osd.AsString();
                return node;
            }
            case OSDType.Date:
            {
                // AsString converts through UTC, so the result does not follow
                // the machine time zone. Binary dates are stored as a local
                // DateTime by OpenMetaverse and converted back here.
                JsonObject node = Type("date");
                node["value"] = osd.AsString();
                return node;
            }
            case OSDType.URI:
            {
                JsonObject node = Type("uri");
                node["value"] = osd.AsString();
                return node;
            }
            case OSDType.Binary:
            {
                JsonObject node = Type("binary");
                node["base64"] = Convert.ToBase64String(osd.AsBinary());
                return node;
            }
            case OSDType.Array:
            {
                JsonObject node = Type("array");
                var items = new JsonArray();
                foreach (OSD item in (OSDArray)osd)
                    items.Add(Canon(item));
                node["value"] = items;
                return node;
            }
            case OSDType.Map:
            {
                JsonObject node = Type("map");
                var pairs = new JsonArray();
                foreach (KeyValuePair<string, OSD> pair in (OSDMap)osd)
                {
                    var entry = new JsonArray { pair.Key, Canon(pair.Value) };
                    pairs.Add(entry);
                }
                node["pairs"] = pairs;
                return node;
            }
            default:
                throw new InvalidOperationException("unsupported OSD type: " + osd.Type);
        }
    }

    static JsonObject DecodeBytes(string format, byte[] payload, JsonObject parms)
    {
        switch (format)
        {
            case "uuid":
                return Need(payload, 16, () =>
                {
                    var id = new UUID();
                    id.FromBytes(payload, 0);
                    JsonObject node = Type("uuid");
                    node["value"] = id.ToString();
                    return node;
                });
            case "vector2":
                return Floats(payload, 2);
            case "vector3":
                return Floats(payload, 3);
            case "vector3d":
                return Doubles(payload, 3);
            case "vector4":
                return Floats(payload, 4);
            case "vector3-clamped":
                return Clamped(payload, parms);
            case "quaternion-getbytes":
            case "quaternion-tobytes":
                return PackedQuaternion(payload, format);
            case "quaternion-shorts":
                return Shorts4(payload);
            case "quaternion-full":
                return Floats(payload, 4);
            case "color4":
                return Color(payload, false);
            case "color4-inverted":
                return Color(payload, true);
            case "color4-floats":
                return Floats(payload, 4);
            case "float-u16":
                return Quantized(payload, parms);
            default:
                throw new InvalidOperationException("unknown byte format: " + format);
        }
    }

    static JsonObject Floats(byte[] payload, int count)
    {
        int size = count * 4;
        if (payload.Length != size)
            throw new ArgumentException("expected " + size + " bytes, got " + payload.Length);
        JsonObject node = Type("f32");
        var bits = new JsonArray();
        for (int i = 0; i < count; i++)
            bits.Add(F32(Utils.BytesToFloat(payload, i * 4)));
        node["bits"] = bits;
        return node;
    }

    static JsonObject Doubles(byte[] payload, int count)
    {
        int size = count * 8;
        if (payload.Length != size)
            throw new ArgumentException("expected " + size + " bytes, got " + payload.Length);
        JsonObject node = Type("f64");
        var bits = new JsonArray();
        for (int i = 0; i < count; i++)
            bits.Add(F64(Utils.BytesToDouble(payload, i * 8)));
        node["bits"] = bits;
        return node;
    }

    static JsonObject Clamped(byte[] payload, JsonObject parms)
    {
        if (payload.Length != 6)
            throw new ArgumentException("expected 6 bytes, got " + payload.Length);
        float range = ParseF32(Required(parms, "range_bits"));
        ushort x = Utils.BytesToUInt16(payload, 0);
        ushort y = Utils.BytesToUInt16(payload, 2);
        ushort z = Utils.BytesToUInt16(payload, 4);
        JsonObject node = Type("vector3-clamped");
        node["x"] = (int)x;
        node["y"] = (int)y;
        node["z"] = (int)z;
        node["x_bits"] = F32(Utils.UInt16ToFloat(x, -range, range));
        node["y_bits"] = F32(Utils.UInt16ToFloat(y, -range, range));
        node["z_bits"] = F32(Utils.UInt16ToFloat(z, -range, range));
        return node;
    }

    static JsonObject PackedQuaternion(byte[] payload, string format)
    {
        if (payload.Length != 12)
            throw new ArgumentException("expected 12 bytes, got " + payload.Length);
        var value = new Quaternion();
        value.FromBytes(payload, 0, true);
        JsonObject node = Type("quaternion");
        node["x_bits"] = F32(value.X);
        node["y_bits"] = F32(value.Y);
        node["z_bits"] = F32(value.Z);
        node["w_bits"] = F32(value.W);
        node["encoder"] = format;
        return node;
    }

    static JsonObject Shorts4(byte[] payload)
    {
        if (payload.Length != 8)
            throw new ArgumentException("expected 8 bytes, got " + payload.Length);
        JsonObject node = Type("u16x4");
        node["x"] = (int)Utils.BytesToUInt16(payload, 0);
        node["y"] = (int)Utils.BytesToUInt16(payload, 2);
        node["z"] = (int)Utils.BytesToUInt16(payload, 4);
        node["w"] = (int)Utils.BytesToUInt16(payload, 6);
        return node;
    }

    static JsonObject Color(byte[] payload, bool inverted)
    {
        if (payload.Length != 4)
            throw new ArgumentException("expected 4 bytes, got " + payload.Length);
        var color = new Color4(0f, 0f, 0f, 0f);
        color.FromBytes(payload, 0, inverted);
        JsonObject node = Type("color4");
        node["inverted"] = inverted;
        var raw = new JsonArray();
        for (int i = 0; i < 4; i++)
            raw.Add((int)payload[i]);
        node["raw"] = raw;
        node["r_bits"] = F32(color.R);
        node["g_bits"] = F32(color.G);
        node["b_bits"] = F32(color.B);
        node["a_bits"] = F32(color.A);
        return node;
    }

    static JsonObject Quantized(byte[] payload, JsonObject parms)
    {
        if (payload.Length != 2)
            throw new ArgumentException("expected 2 bytes, got " + payload.Length);
        ushort bits = Utils.BytesToUInt16(payload, 0);
        string mode = Required(parms, "mode");
        JsonObject node = Type("u16");
        node["value"] = (int)bits;
        if (mode == "range")
        {
            float range = ParseF32(Required(parms, "range_bits"));
            node["float_bits"] = F32(Utils.UInt16ToFloat(bits, -range, range));
        }
        else if (mode == "bounds")
        {
            float lower = ParseF32(Required(parms, "lower_bits"));
            float upper = ParseF32(Required(parms, "upper_bits"));
            node["float_bits"] = F32(Utils.UInt16ToFloat(bits, lower, upper));
        }
        else if (mode != "unit")
        {
            throw new InvalidOperationException("unknown float-u16 mode: " + mode);
        }

        return node;
    }

    /// <summary>True when decoding these bytes and encoding again yields the same bytes.</summary>
    internal static bool RoundTrips(string family, string format, byte[] payload, JsonObject parms)
    {
        try
        {
            if (family == "llsd")
            {
                if (IsError(Decode(family, format, payload, parms)))
                    return false;
                OSD value = DecodeLlsd(format, payload);
                return EncodeLlsd(format, value).AsSpan().SequenceEqual(payload);
            }

            return ReencodeBytes(format, payload, parms ?? new JsonObject()).AsSpan().SequenceEqual(payload);
        }
        catch (Exception ex) when (IsDecodeFailure(ex))
        {
            return false;
        }
    }

    static byte[] ReencodeBytes(string format, byte[] payload, JsonObject parms)
    {
        switch (format)
        {
            case "uuid":
            {
                var id = new UUID();
                id.FromBytes(payload, 0);
                return id.GetBytes();
            }
            case "vector2":
            {
                var value = new Vector2();
                value.FromBytes(payload, 0);
                return value.GetBytes();
            }
            case "vector3":
            {
                var value = new Vector3();
                value.FromBytes(payload, 0);
                return value.GetBytes();
            }
            case "vector3d":
            {
                var value = new Vector3d();
                value.FromBytes(payload, 0);
                return value.GetBytes();
            }
            case "vector4":
            {
                var value = new Vector4();
                value.FromBytes(payload, 0);
                return value.GetBytes();
            }
            case "vector3-clamped":
            {
                float range = ParseF32(Required(parms, "range_bits"));
                ushort x = Utils.BytesToUInt16(payload, 0);
                ushort y = Utils.BytesToUInt16(payload, 2);
                ushort z = Utils.BytesToUInt16(payload, 4);
                var value = new Vector3(
                    Utils.UInt16ToFloat(x, -range, range),
                    Utils.UInt16ToFloat(y, -range, range),
                    Utils.UInt16ToFloat(z, -range, range));
                byte[] again = new byte[6];
                value.ClampedToShortsBytes(range, again, 0);
                return again;
            }
            case "quaternion-getbytes":
            {
                var value = new Quaternion();
                value.FromBytes(payload, 0, true);
                return value.GetBytes();
            }
            case "quaternion-tobytes":
            {
                var value = new Quaternion();
                value.FromBytes(payload, 0, true);
                byte[] again = new byte[12];
                value.ToBytes(again, 0);
                return again;
            }
            case "quaternion-shorts":
                return payload.ToArray();
            case "quaternion-full":
            case "color4-floats":
            {
                int count = payload.Length / 4;
                byte[] again = new byte[payload.Length];
                for (int i = 0; i < count; i++)
                    Utils.FloatToBytes(Utils.BytesToFloat(payload, i * 4), again, i * 4);
                return again;
            }
            case "color4":
            case "color4-inverted":
            {
                bool inverted = format == "color4-inverted";
                var color = new Color4(0f, 0f, 0f, 0f);
                color.FromBytes(payload, 0, inverted);
                byte[] again = new byte[4];
                color.ToBytes(again, 0, inverted);
                return again;
            }
            case "float-u16":
            {
                ushort bits = Utils.BytesToUInt16(payload, 0);
                string mode = Required(parms, "mode");
                ushort again;
                if (mode == "range")
                {
                    float range = ParseF32(Required(parms, "range_bits"));
                    float value = Utils.UInt16ToFloat(bits, -range, range);
                    again = Utils.FloatToUInt16(value, range);
                }
                else if (mode == "bounds")
                {
                    float lower = ParseF32(Required(parms, "lower_bits"));
                    float upper = ParseF32(Required(parms, "upper_bits"));
                    float value = Utils.UInt16ToFloat(bits, lower, upper);
                    again = Utils.FloatToUInt16(value, lower, upper);
                }
                else if (mode == "unit")
                {
                    again = bits;
                }
                else
                {
                    throw new InvalidOperationException("unknown float-u16 mode: " + mode);
                }

                return Utils.UInt16ToBytes(again);
            }
            default:
                throw new InvalidOperationException("unknown byte format: " + format);
        }
    }

    static JsonObject Need(byte[] payload, int size, Func<JsonObject> build)
    {
        if (payload.Length != size)
            throw new ArgumentException("expected " + size + " bytes, got " + payload.Length);
        return build();
    }

    internal static string Required(JsonObject parms, string key)
    {
        JsonNode node = parms[key];
        if (node is null)
            throw new InvalidOperationException("missing parameter " + key);
        return node.GetValue<string>();
    }

    static JsonObject Type(string name)
    {
        var node = new JsonObject();
        node["type"] = name;
        return node;
    }

    static JsonObject Error(Exception ex)
    {
        JsonObject node = Type("decode-error");
        node["exception"] = ex.GetType().Name;
        node["message"] = ex.Message;
        return node;
    }

    static bool IsDecodeFailure(Exception ex)
    {
        return ex is OSDException
            or FormatException
            or ArgumentException
            or InvalidOperationException
            or IndexOutOfRangeException
            or EndOfStreamException
            or DecoderFallbackException
            or System.Xml.XmlException;
    }
}
