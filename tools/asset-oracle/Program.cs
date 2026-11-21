// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Serializes AssetBase, AssetMetadata and the small XML values the asset
// handlers return, using Tranquillity's ServerUtils.SerializeResult.
// The C# types are not copied into this repository.

using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Xml.Serialization;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Server.Base;

namespace Ferroan.AssetOracle;

static class Program
{
    static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        NewLine = "\n"
    };

    static int Main(string[] args)
    {
        if (args.Length > 0 && (args[0] == "db" || args[0] == "xdb" || args[0] == "fdb"))
        {
            try
            {
                if (args[0] == "db")
                    return DbMode.Run(args);
                if (args[0] == "xdb")
                    return XdbMode.Run(args);
                return FdbMode.Run(args);
            }
            catch (Exception exception)
            {
                Console.Error.WriteLine(DbMode.Scrub(exception.ToString()));
                return 1;
            }
        }

        if (args.Length != 4 || (args[0] != "generate" && args[0] != "verify") || args[2] != "--revision")
        {
            Console.Error.WriteLine("usage: AssetOracle generate|verify <directory> --revision <40 hex>");
            Console.Error.WriteLine("       AssetOracle db put|get|exists|delete|metadata");
            Console.Error.WriteLine("       AssetOracle xdb put|get|exists|delete|metadata");
            Console.Error.WriteLine("       AssetOracle fdb put|get|exists|delete|count|import");
            return 2;
        }

        string directory = Path.GetFullPath(args[1]);
        string revision = args[3];
        if (revision.Length != 40 || revision.Any(ch => !Uri.IsHexDigit(ch)))
        {
            Console.Error.WriteLine("revision must be 40 hex characters");
            return 1;
        }

        try
        {
            if (args[0] == "generate")
                Generate(directory, revision);
            else
                Verify(directory, revision);
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception);
            return 1;
        }

        return 0;
    }

    static void Generate(string directory, string revision)
    {
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        int count = WriteTree(scratch, revision);
        if (Directory.Exists(directory))
        {
            // README.md and .gitattributes are not generated. The attributes
            // file keeps the XML as binary so a raw CR in element text survives
            // checkout.
            foreach (string name in new[] { "README.md", ".gitattributes" })
            {
                string source = Path.Combine(directory, name);
                if (File.Exists(source))
                    File.Copy(source, Path.Combine(scratch, name));
            }
            Directory.Delete(directory, true);
        }
        Directory.Move(scratch, directory);
        Console.WriteLine("generate: " + count + " asset xml documents");
    }

    static void Verify(string directory, string revision)
    {
        string scratch = directory + ".verify";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        int count = WriteTree(scratch, revision);
        CompareTrees(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("verify: " + count + " asset xml documents match ServerUtils");
    }

    static int WriteTree(string directory, string revision)
    {
        JsonArray cases = new();
        foreach (Sample sample in Samples())
        {
            AssetBase asset = sample.Build();
            byte[] assetXml = Serialize(new XmlSerializer(typeof(AssetBase)), asset);
            byte[] metaXml = Serialize(new XmlSerializer(typeof(AssetMetadata)), asset.Metadata);
            CheckRoundTrip(asset, assetXml, metaXml);
            Write(directory, "asset/" + sample.Id + ".xml", assetXml);
            Write(directory, "metadata/" + sample.Id + ".xml", metaXml);
            cases.Add(sample.Node("asset", "asset/" + sample.Id + ".xml"));
            cases.Add(sample.Node("metadata", "metadata/" + sample.Id + ".xml"));
        }

        Write(directory, "scalar/bool-true.xml", Serialize(new XmlSerializer(typeof(bool)), true));
        Write(directory, "scalar/bool-false.xml", Serialize(new XmlSerializer(typeof(bool)), false));
        string id = "00112233-4455-6677-8899-aabbccddeeff";
        Write(directory, "scalar/string-id.xml", Serialize(new XmlSerializer(typeof(string)), id));
        bool[] flags = { true, false, true };
        Write(directory, "scalar/bools.xml", Serialize(new XmlSerializer(typeof(bool[])), flags));
        string[] ids = { id, UUID.Zero.ToString() };
        Write(directory, "scalar/ids.xml", Serialize(new XmlSerializer(typeof(string[])), ids));
        Write(directory, "scalar/bools-empty.xml", Serialize(new XmlSerializer(typeof(bool[])), Array.Empty<bool>()));
        Write(directory, "scalar/ids-empty.xml", Serialize(new XmlSerializer(typeof(string[])), Array.Empty<string>()));
        cases.Add(Scalar("bool-true", "scalar/bool-true.xml"));
        cases.Add(Scalar("bool-false", "scalar/bool-false.xml"));
        cases.Add(Scalar("string-id", "scalar/string-id.xml"));
        cases.Add(Scalar("bools", "scalar/bools.xml"));
        cases.Add(Scalar("ids", "scalar/ids.xml"));
        cases.Add(Scalar("bools-empty", "scalar/bools-empty.xml"));
        cases.Add(Scalar("ids-empty", "scalar/ids-empty.xml"));

        JsonObject types = new();
        foreach (sbyte type in ContentTypes())
            types[type.ToString()] = SLUtil.SLAssetTypeToContentType(type);
        File.WriteAllText(
            Path.Combine(directory, "content-types.json"),
            types.ToJsonString(JsonOptions) + "\n");

        JsonObject manifest = new()
        {
            ["format"] = 1,
            ["tranquillity"] = revision,
            ["cases"] = cases
        };
        File.WriteAllText(Path.Combine(directory, "manifest.json"), manifest.ToJsonString(JsonOptions) + "\n");
        // TB1: flags 8 is not a named AssetFlags bit, and 9 mixes Maptile with
        // that bit. Each record is FromStored then the AssetServerGetHandler
        // document, not a manifest case.
        WriteFlagProbe(
            directory,
            revision,
            "unnamed-flags",
            "33333333-4444-5555-6666-777777777777",
            8);
        WriteFlagProbe(
            directory,
            revision,
            "mixed-flags",
            "33333333-4444-5555-6666-777777777778",
            9);
        PilotMode.Write(directory, revision);
        return cases.Count;
    }

    /// <summary>
    /// What <c>AssetServerGetHandler</c> returns for a stored row: the store
    /// masks with <c>StoredAssetFlags.FromStored</c>, then
    /// <c>ServerUtils.SerializeResult</c> writes the asset document.
    /// </summary>
    static void WriteFlagProbe(string directory, string revision, string id, string uuid, int flags)
    {
        AssetBase asset = new AssetBase(new UUID(uuid), id, 0, "creator")
        {
            Description = "",
            Local = false,
            Temporary = false,
            Data = Array.Empty<byte>()
        };
        asset.Flags = StoredAssetFlags.FromStored(flags, asset.ID.ToString());
        bool threw = false;
        string exceptionType = "";
        try
        {
            byte[] xml = Serialize(new XmlSerializer(typeof(AssetBase)), asset);
            Write(directory, "asset/" + id + ".xml", xml);
        }
        catch (Exception exception)
        {
            threw = true;
            // FullName only. The message can name a generated assembly path.
            exceptionType = exception.GetType().FullName ?? "";
        }

        JsonObject record = new()
        {
            ["id"] = id,
            ["tranquillity"] = revision,
            ["flags"] = flags,
            ["known_flags"] = (int)asset.Flags,
            ["threw"] = threw,
            ["exception_type"] = exceptionType
        };
        File.WriteAllText(
            Path.Combine(directory, id + ".json"),
            record.ToJsonString(JsonOptions) + "\n");
    }

    static void CheckRoundTrip(AssetBase asset, byte[] assetXml, byte[] metaXml)
    {
        AssetBase back = (AssetBase)new XmlSerializer(typeof(AssetBase)).Deserialize(new MemoryStream(assetXml));
        if (back == null || back.ID != asset.ID || back.Name != asset.Name || back.Type != asset.Type)
            throw new InvalidOperationException("asset xml did not round-trip " + asset.ID);
        if (back.Data == null || asset.Data == null || !back.Data.AsSpan().SequenceEqual(asset.Data))
            throw new InvalidOperationException("asset data did not round-trip " + asset.ID);
        AssetMetadata meta = (AssetMetadata)new XmlSerializer(typeof(AssetMetadata)).Deserialize(new MemoryStream(metaXml));
        if (meta == null || meta.ID != asset.Metadata.ID || meta.Type != asset.Metadata.Type)
            throw new InvalidOperationException("metadata xml did not round-trip " + asset.ID);
    }

    static byte[] Serialize(XmlSerializer serializer, object value)
    {
        return ServerUtils.SerializeResult(serializer, value);
    }

    static void Write(string directory, string relative, byte[] bytes)
    {
        string path = Path.Combine(directory, relative.Replace('/', Path.DirectorySeparatorChar));
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        File.WriteAllBytes(path, bytes);
    }

    static JsonObject Scalar(string id, string file)
    {
        return new JsonObject
        {
            ["id"] = id,
            ["kind"] = "scalar",
            ["file"] = file
        };
    }

    static void CompareTrees(string left, string right)
    {
        foreach (string path in Directory.GetFiles(left, "*", SearchOption.AllDirectories))
        {
            string relative = Path.GetRelativePath(left, path);
            string other = Path.Combine(right, relative);
            if (!File.Exists(other))
                throw new InvalidOperationException("missing " + relative);
            byte[] a = File.ReadAllBytes(path);
            byte[] b = File.ReadAllBytes(other);
            if (!a.AsSpan().SequenceEqual(b))
                throw new InvalidOperationException("bytes differ for " + relative);
        }
    }

    static IEnumerable<sbyte> ContentTypes()
    {
        sbyte[] values =
        {
            -2, -1, 0, 1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13, 17, 18, 19, 20, 21, 22, 24, 25, 49, 57, 99
        };
        return values;
    }

    static IEnumerable<Sample> Samples()
    {
        yield return new Sample(
            "texture",
            "00112233-4455-6677-8899-aabbccddeeff",
            "Wood",
            "a plank",
            0,
            false,
            false,
            "creator",
            0,
            Encoding.ASCII.GetBytes("hello"));
        yield return new Sample(
            "escaped",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "a & b <c>",
            "line\ntwo",
            7,
            true,
            true,
            "00000000-0000-0000-0000-000000000000",
            1,
            new byte[] { 0, 1, 255 });
        byte[] longData = new byte[200];
        for (int i = 0; i < longData.Length; i++)
            longData[i] = (byte)i;
        yield return new Sample(
            "long-data",
            "11111111-2222-3333-4444-555555555555",
            new string('n', 80),
            new string('d', 80),
            6,
            false,
            false,
            "maker",
            3,
            longData);
        yield return new Sample(
            "all-flags",
            "22222222-3333-4444-5555-666666666666",
            "all",
            "d",
            0,
            false,
            false,
            "creator",
            7,
            Encoding.ASCII.GetBytes("x"));
        yield return new Sample(
            "empty",
            "44444444-5555-6666-7777-888888888888",
            "empty",
            "",
            -1,
            false,
            false,
            "",
            0,
            Array.Empty<byte>());
        yield return new Sample(
            "controls",
            "55555555-6666-7777-8888-999999999999",
            "say \"hi\"",
            "a\r\nb\t<",
            7,
            false,
            false,
            "creator",
            4,
            new byte[] { 0x20 });
    }

    sealed class Sample
    {
        public string Id;
        public string Uuid;
        public string Name;
        public string Description;
        public sbyte Type;
        public bool Local;
        public bool Temporary;
        public string CreatorId;
        public int Flags;
        public byte[] Data;

        public Sample(
            string id, string uuid, string name, string description, sbyte type,
            bool local, bool temporary, string creatorId, int flags, byte[] data)
        {
            Id = id;
            Uuid = uuid;
            Name = name;
            Description = description;
            Type = type;
            Local = local;
            Temporary = temporary;
            CreatorId = creatorId;
            Flags = flags;
            Data = data;
        }

        public AssetBase Build()
        {
            AssetBase asset = new AssetBase(new UUID(Uuid), Name, Type, CreatorId)
            {
                Description = Description,
                Local = Local,
                Temporary = Temporary,
                Flags = (AssetFlags)Flags,
                Data = Data
            };
            return asset;
        }

        public JsonObject Node(string kind, string file)
        {
            return new JsonObject
            {
                ["id"] = kind + "/" + Id,
                ["kind"] = kind,
                ["file"] = file,
                ["uuid"] = Uuid,
                ["name"] = Name,
                ["description"] = Description,
                ["type"] = Type,
                ["local"] = Local,
                ["temporary"] = Temporary,
                ["creator_id"] = CreatorId,
                ["flags"] = Flags,
                ["data_hex"] = Convert.ToHexString(Data).ToLowerInvariant()
            };
        }
    }
}
