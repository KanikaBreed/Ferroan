// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Builds one datagram per message in libopenmetaverse's message_template.msg.
// Messages that OpenMetaverse generated a class for are also filled and encoded
// with that class; the bytes must match. The other messages use the same layout
// rules, checked by that comparison.
//
//   libopenmetaverse/data/message_template.msg
//   OpenMetaverse/_Packets_.cs Header.ToBytes and Packet.BuildPacket
//   OpenMetaverse/Helpers.cs ZeroDecode
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs ZeroEncode
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c

using System.Globalization;
using System.Reflection;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenMetaverse.Packets;

namespace Ferroan.ReferenceOracle;

internal sealed class TemplateMessage
{
    public string Header;
    public string Name;
    public string Frequency;
    public uint Number;
    public bool Zerocoded;
    public List<TemplateBlock> Blocks = new();
}

internal sealed class TemplateBlock
{
    public string Name;
    public string Kind;
    public int Count;
    public List<TemplateField> Fields = new();
}

internal sealed class TemplateField
{
    public string Name;
    public string Type;
    public int Size;
}

/// <summary>Deterministic LLUDP datagrams for every template message.</summary>
internal static partial class PacketCases
{
    const byte Reliable = 0x40;
    const byte Zerocoded = 0x80;

    static readonly UUID SampleId = UUID.Parse("00112233-4455-6677-8899-aabbccddeeff");
    static readonly BindingFlags Fields = BindingFlags.Instance | BindingFlags.Public;

    static List<TemplateMessage> template;

    internal static void Add(List<Fixture> list)
    {
        Load();
        int matched = 0;
        int decoded = 0;
        int withoutClass = 0;
        foreach (TemplateMessage message in template)
        {
            byte[] raw = EncodeRaw(message);
            byte[] wire = MaybeZero(raw);
            byte[] fromClass = TryClass(message);
            if (fromClass == null)
            {
                withoutClass++;
            }
            else
            {
                if (!fromClass.AsSpan().SequenceEqual(wire))
                {
                    throw new InvalidOperationException(Diff(message.Name, wire, fromClass));
                }

                try
                {
                    int length = wire.Length;
                    byte[] zero = new byte[256 * 1024];
                    Packet parsed = Packet.BuildPacket(wire, ref length, zero);
                    if (parsed != null && parsed.Type.ToString() == message.Name)
                    {
                        decoded++;
                    }
                }
                catch (Exception ex) when (ex is NullReferenceException or MalformedDataException or IndexOutOfRangeException or ArgumentException)
                {
                    // Some generated FromBytes methods dereference a fixed-count
                    // block the constructor allocated as null. ToBytes still matches.
                }

                matched++;
            }

            var parms = new JsonObject
            {
                ["name"] = message.Name,
                ["frequency"] = message.Frequency,
                ["number"] = (int)message.Number,
                ["has_class"] = fromClass != null,
            };
            JsonObject decodedNode = Codec.Decode("packet", "datagram", wire, parms);
            if (Codec.IsError(decodedNode))
            {
                throw new InvalidOperationException(
                    "packet fixture did not decode: " + message.Name + " " + decodedNode.ToJsonString());
            }

            list.Add(new Fixture
            {
                Id = "packet/" + message.Name,
                Family = "packet",
                Format = "datagram",
                File = "packet/" + message.Name + ".bin",
                Payload = wire,
                Decoded = decodedNode,
                Params = parms,
                RoundTrip = Codec.RoundTrips("packet", "datagram", wire, parms),
            });
        }

        if (matched < 300)
        {
            throw new InvalidOperationException("only " + matched + " packets matched a generated class");
        }

        Console.WriteLine(
            "packets: " + template.Count + " messages, " + matched + " match OpenMetaverse classes, "
            + decoded + " of those decode with BuildPacket, " + withoutClass + " have no class");
        AddExtensions(list);
    }

    internal static byte[] EncodeNamed(string name, bool extended = false)
    {
        Load();
        TemplateMessage message = (extended ? extendedTemplate : template).FirstOrDefault(item => item.Name == name);
        if (message == null)
        {
            throw new InvalidOperationException("unknown message " + name);
        }

        return MaybeZero(EncodeRaw(message));
    }

    internal static bool MatchesClass(string name, byte[] payload)
    {
        Load();
        TemplateMessage message = template.First(item => item.Name == name);
        byte[] fromClass = TryClass(message);
        if (fromClass == null)
        {
            return true;
        }

        return fromClass.AsSpan().SequenceEqual(payload);
    }

    static void Load()
    {
        if (template != null)
        {
            return;
        }

        string root = Directory.GetCurrentDirectory();
        string committed = Path.Combine(root, "crates", "ferroan-protocol", "message_template.msg");
        string referenceRoot = Environment.GetEnvironmentVariable("FERROAN_REF");
        if (string.IsNullOrEmpty(referenceRoot))
        {
            throw new InvalidOperationException("FERROAN_REF is not set");
        }

        string reference = Path.Combine(referenceRoot, "libopenmetaverse", "data", "message_template.msg");
        byte[] committedBytes = File.ReadAllBytes(committed);
        byte[] referenceBytes = File.ReadAllBytes(reference);
        template = Parse(System.Text.Encoding.UTF8.GetString(referenceBytes));
        extendedTemplate = Parse(System.Text.Encoding.UTF8.GetString(committedBytes));
        CheckExtensionValidation();
        ValidateExtensions(template, extendedTemplate);
        if (template.Count != 478)
        {
            throw new InvalidOperationException("expected 478 messages, parsed " + template.Count);
        }
    }

    static List<TemplateMessage> Parse(string src)
    {
        var tokens = new List<string>();
        foreach (string raw in src.Split('\n'))
        {
            string line = raw;
            int comment = line.IndexOf("//", StringComparison.Ordinal);
            if (comment >= 0)
            {
                line = line.Substring(0, comment);
            }

            line = line.Replace("{", " { ").Replace("}", " } ");
            foreach (string token in line.Split((char[])null, StringSplitOptions.RemoveEmptyEntries))
            {
                tokens.Add(token);
            }
        }

        int i = 0;
        while (i < tokens.Count && tokens[i] != "{")
        {
            i++;
        }
        if (string.Join(" ", tokens.Take(i)) != "version 2.0")
            throw new InvalidOperationException("expected message template version 2.0");

        var messages = new List<TemplateMessage>();
        while (i < tokens.Count)
        {
            if (tokens[i] != "{")
            {
                throw new InvalidOperationException("expected message at " + tokens[i]);
            }

            i++;
            int headerStart = i;
            var message = new TemplateMessage
            {
                Name = tokens[i],
                Frequency = tokens[i + 1],
                Number = ParseNum(tokens[i + 2]),
                Zerocoded = tokens[i + 4] == "Zerocoded",
            };
            i += 5;
            while (tokens[i] != "{" && tokens[i] != "}")
            {
                i++;
            }
            message.Header = string.Join(" ", tokens.Skip(headerStart).Take(i - headerStart));

            while (tokens[i] == "{")
            {
                i++;
                var block = new TemplateBlock { Name = tokens[i], Kind = tokens[i + 1] };
                if (block.Kind == "Multiple")
                {
                    block.Count = (int)ParseNum(tokens[i + 2]);
                    i += 3;
                }
                else
                {
                    i += 2;
                }

                while (tokens[i] == "{")
                {
                    var field = new TemplateField { Name = tokens[i + 1], Type = tokens[i + 2] };
                    if (field.Type == "Fixed" || field.Type == "Variable")
                    {
                        field.Size = (int)ParseNum(tokens[i + 3]);
                        i += 5;
                    }
                    else
                    {
                        i += 4;
                    }

                    if (tokens[i - 1] != "}")
                    {
                        throw new InvalidOperationException("expected field end in " + message.Name);
                    }

                    block.Fields.Add(field);
                }

                if (tokens[i] != "}")
                {
                    throw new InvalidOperationException("expected block end in " + message.Name);
                }

                i++;
                message.Blocks.Add(block);
            }

            if (tokens[i] != "}")
            {
                throw new InvalidOperationException("expected message end for " + message.Name);
            }

            i++;
            messages.Add(message);
        }

        return messages;
    }

    static uint ParseNum(string text)
    {
        if (text.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
        {
            return uint.Parse(text.Substring(2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        }

        return uint.Parse(text, CultureInfo.InvariantCulture);
    }

    static byte[] EncodeRaw(TemplateMessage message)
    {
        var body = new List<byte>();
        WriteBody(body, message);
        var raw = new byte[6 + body.Count];
        raw[0] = Reliable;
        if (message.Zerocoded)
        {
            raw[0] |= Zerocoded;
        }

        body.CopyTo(raw, 6);
        return raw;
    }

    static void WriteBody(List<byte> body, TemplateMessage message)
    {
        ushort id = (ushort)(message.Number & 0xFFFF);
        if (message.Frequency == "High")
        {
            body.Add((byte)id);
        }
        else if (message.Frequency == "Medium")
        {
            body.Add(0xFF);
            body.Add((byte)id);
        }
        else
        {
            body.Add(0xFF);
            body.Add(0xFF);
            body.Add((byte)(id >> 8));
            body.Add((byte)id);
        }

        foreach (TemplateBlock block in message.Blocks)
        {
            int count = block.Kind == "Single" ? 1 : block.Kind == "Multiple" ? block.Count : 1;
            if (block.Kind == "Variable")
            {
                body.Add(1);
            }

            for (int n = 0; n < count; n++)
            {
                WriteBlock(body, block);
            }
        }
    }

    static void WriteBlock(List<byte> body, TemplateBlock block)
    {
        foreach (TemplateField field in block.Fields)
        {
            switch (field.Type)
            {
                case "U8":
                case "S8":
                    body.Add(0x2A);
                    break;
                case "BOOL":
                    body.Add(1);
                    break;
                case "U16":
                case "S16":
                    AddU16(body, 0x1234);
                    break;
                case "IPPORT":
                    body.Add(0x12);
                    body.Add(0x34);
                    break;
                case "U32":
                case "S32":
                case "IPADDR":
                    AddU32(body, 0x01020304);
                    break;
                case "U64":
                case "S64":
                    AddU64(body, 0x0102030405060708);
                    break;
                case "F32":
                    AddF32(body, 1.5f);
                    break;
                case "F64":
                    AddF64(body, 1.5d);
                    break;
                case "LLVector3":
                    AddVector3(body, new Vector3(1f, 2f, 3f));
                    break;
                case "LLVector3d":
                    AddVector3d(body, new Vector3d(1d, 2d, 3d));
                    break;
                case "LLVector4":
                    AddVector4(body, new Vector4(1f, 2f, 3f, 4f));
                    break;
                case "LLQuaternion":
                    AddQuaternion(body, new Quaternion(0f, 0f, 0f, 1f));
                    break;
                case "LLUUID":
                    AddUuid(body, SampleId);
                    break;
                case "Fixed":
                    for (int i = 0; i < field.Size; i++)
                    {
                        body.Add(0xA5);
                    }

                    break;
                case "Variable":
                    byte[] data = { 0x11, 0x22, 0x33 };
                    if (field.Size == 1)
                    {
                        body.Add((byte)data.Length);
                    }
                    else
                    {
                        AddU16(body, (ushort)data.Length);
                    }

                    body.AddRange(data);
                    break;
                default:
                    throw new InvalidOperationException("unknown field type " + field.Type);
            }
        }
    }

    static byte[] TryClass(TemplateMessage message)
    {
        Type type = typeof(Packet).Assembly.GetType("OpenMetaverse.Packets." + message.Name + "Packet");
        if (type == null)
        {
            return null;
        }

        var packet = (Packet)Activator.CreateInstance(type);
        foreach (TemplateBlock block in message.Blocks)
        {
            FieldInfo field = type.GetField(block.Name, Fields | BindingFlags.DeclaredOnly);
            if (field == null)
            {
                // Header collides with Packet.Header, so the generator names that block _Header.
                field = type.GetField("_" + block.Name, Fields | BindingFlags.DeclaredOnly);
            }

            if (field == null)
            {
                throw new InvalidOperationException(message.Name + " has no block field " + block.Name);
            }

            if (block.Kind == "Single")
            {
                object value = Activator.CreateInstance(field.FieldType);
                FillBlock(message.Name, value, block);
                field.SetValue(packet, value);
            }
            else
            {
                int count = block.Kind == "Multiple" ? block.Count : 1;
                Type element = field.FieldType.GetElementType();
                Array items = Array.CreateInstance(element, count);
                for (int i = 0; i < count; i++)
                {
                    object value = Activator.CreateInstance(element);
                    FillBlock(message.Name, value, block);
                    items.SetValue(value, i);
                }

                field.SetValue(packet, items);
            }
        }

        return MaybeZero(packet.ToBytes());
    }

    static void FillBlock(string message, object block, TemplateBlock spec)
    {
        Type type = block.GetType();
        foreach (TemplateField field in spec.Fields)
        {
            FieldInfo info = type.GetField(field.Name, Fields);
            if (info == null)
            {
                throw new InvalidOperationException(message + "." + spec.Name + " has no field " + field.Name);
            }

            info.SetValue(block, ValueFor(info.FieldType, field));
        }
    }

    static object ValueFor(Type type, TemplateField field)
    {
        if (type == typeof(byte))
        {
            return (byte)0x2A;
        }

        if (type == typeof(sbyte))
        {
            return (sbyte)0x2A;
        }

        if (type == typeof(ushort))
        {
            return (ushort)0x1234;
        }

        if (type == typeof(short))
        {
            return (short)0x1234;
        }

        if (type == typeof(uint))
        {
            return 0x01020304u;
        }

        if (type == typeof(int))
        {
            return 0x01020304;
        }

        if (type == typeof(ulong))
        {
            return 0x0102030405060708ul;
        }

        if (type == typeof(long))
        {
            return 0x0102030405060708L;
        }

        if (type == typeof(float))
        {
            return 1.5f;
        }

        if (type == typeof(double))
        {
            return 1.5d;
        }

        if (type == typeof(bool))
        {
            return true;
        }

        if (type == typeof(UUID))
        {
            return SampleId;
        }

        if (type == typeof(Vector3))
        {
            return new Vector3(1f, 2f, 3f);
        }

        if (type == typeof(Vector3d))
        {
            return new Vector3d(1d, 2d, 3d);
        }

        if (type == typeof(Vector4))
        {
            return new Vector4(1f, 2f, 3f, 4f);
        }

        if (type == typeof(Quaternion))
        {
            return new Quaternion(0f, 0f, 0f, 1f);
        }

        if (type == typeof(byte[]))
        {
            if (field.Type == "Fixed")
            {
                var bytes = new byte[field.Size];
                for (int i = 0; i < bytes.Length; i++)
                {
                    bytes[i] = 0xA5;
                }

                return bytes;
            }

            return new byte[] { 0x11, 0x22, 0x33 };
        }

        throw new InvalidOperationException("unsupported packet field type " + type.FullName);
    }

    static byte[] MaybeZero(byte[] raw)
    {
        if ((raw[0] & Zerocoded) == 0)
        {
            return raw;
        }

        byte[] encoded = ZeroEncode(raw);
        if (encoded.Length <= raw.Length)
        {
            return encoded;
        }

        var plain = (byte[])raw.Clone();
        plain[0] = (byte)(plain[0] & ~Zerocoded);
        return plain;
    }

    /// <summary><c>LLUDPServer.ZeroEncode</c>: the first six bytes are copied, then the rest is coded.</summary>
    static byte[] ZeroEncode(byte[] src)
    {
        var dest = new byte[src.Length * 2 + 16];
        Buffer.BlockCopy(src, 0, dest, 0, 6);
        int zerolen = 6;
        byte zerocount = 0;
        for (int i = 6; i < src.Length; i++)
        {
            if (src[i] == 0x00)
            {
                unchecked
                {
                    zerocount++;
                }

                if (zerocount == 0)
                {
                    dest[zerolen++] = 0x00;
                    dest[zerolen++] = 0xff;
                    unchecked
                    {
                        zerocount++;
                    }
                }
            }
            else
            {
                if (zerocount != 0)
                {
                    dest[zerolen++] = 0x00;
                    dest[zerolen++] = zerocount;
                    zerocount = 0;
                }

                dest[zerolen++] = src[i];
            }
        }

        if (zerocount != 0)
        {
            dest[zerolen++] = 0x00;
            dest[zerolen++] = zerocount;
        }

        var trimmed = new byte[zerolen];
        Buffer.BlockCopy(dest, 0, trimmed, 0, zerolen);
        return trimmed;
    }

    static void AddU16(List<byte> body, ushort value)
    {
        var bytes = new byte[2];
        Utils.UInt16ToBytes(value, bytes, 0);
        body.AddRange(bytes);
    }

    static void AddU32(List<byte> body, uint value)
    {
        var bytes = new byte[4];
        Utils.UIntToBytesSafepos(value, bytes, 0);
        body.AddRange(bytes);
    }

    static void AddU64(List<byte> body, ulong value)
    {
        var bytes = new byte[8];
        Utils.UInt64ToBytes(value, bytes, 0);
        body.AddRange(bytes);
    }

    static void AddF32(List<byte> body, float value)
    {
        var bytes = new byte[4];
        Utils.FloatToBytesSafepos(value, bytes, 0);
        body.AddRange(bytes);
    }

    static void AddF64(List<byte> body, double value)
    {
        var bytes = new byte[8];
        Utils.DoubleToBytesSafepos(value, bytes, 0);
        body.AddRange(bytes);
    }

    static void AddVector3(List<byte> body, Vector3 value)
    {
        var bytes = new byte[12];
        value.ToBytes(bytes, 0);
        body.AddRange(bytes);
    }

    static void AddVector3d(List<byte> body, Vector3d value)
    {
        var bytes = new byte[24];
        value.ToBytes(bytes, 0);
        body.AddRange(bytes);
    }

    static void AddVector4(List<byte> body, Vector4 value)
    {
        var bytes = new byte[16];
        value.ToBytes(bytes, 0);
        body.AddRange(bytes);
    }

    static void AddQuaternion(List<byte> body, Quaternion value)
    {
        var bytes = new byte[12];
        value.ToBytes(bytes, 0);
        body.AddRange(bytes);
    }

    static void AddUuid(List<byte> body, UUID value)
    {
        var bytes = new byte[16];
        value.ToBytes(bytes, 0);
        body.AddRange(bytes);
    }

    static string Diff(string name, byte[] generic, byte[] fromClass)
    {
        int n = Math.Min(generic.Length, fromClass.Length);
        int at = 0;
        while (at < n && generic[at] == fromClass[at])
        {
            at++;
        }

        return name + ": generic " + generic.Length + " bytes, class " + fromClass.Length
            + " bytes, first difference at " + at
            + " generic " + Hex(generic, at)
            + " class " + Hex(fromClass, at);
    }

    static string Hex(byte[] bytes, int at)
    {
        int end = Math.Min(bytes.Length, at + 8);
        var text = new System.Text.StringBuilder();
        for (int i = at; i < end; i++)
        {
            text.Append(bytes[i].ToString("x2", CultureInfo.InvariantCulture));
        }

        return text.ToString();
    }
}
