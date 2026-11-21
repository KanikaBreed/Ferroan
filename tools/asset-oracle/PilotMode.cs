// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Records the filesystem loader, the StoreAsset update the loader uses, and
// the text of show/dump/delete asset. The C# types are not copied here.

using System.Data.SQLite;
using System.Text;
using System.Text.Json.Nodes;
using System.Xml;
using OpenMetaverse;
using OpenSim.Data.SQLite;
using OpenSim.Framework;
using OpenSim.Framework.AssetLoader.Filesystem;

namespace Ferroan.AssetOracle;

static class PilotMode
{
    const string LibraryOwner = "11111111-1111-0000-0000-000100bba000";

    public static void Write(string directory, string revision)
    {
        WriteLoader(directory, revision);
        WriteSeed(directory, revision);
        WriteCommands(directory, revision);
        WriteLibrary(directory, revision);
    }

    static void WriteLoader(string directory, string revision)
    {
        string root = Path.Combine(directory, "loader-sample");
        Directory.CreateDirectory(Path.Combine(root, "ok", "One"));
        Directory.CreateDirectory(Path.Combine(root, "ok", "Two"));
        Directory.CreateDirectory(Path.Combine(root, "failures"));
        File.WriteAllBytes(Path.Combine(root, "ok", "One", "hello.dat"), Encoding.ASCII.GetBytes("hello"));
        File.WriteAllBytes(Path.Combine(root, "ok", "Two", "two.dat"), new byte[] { 1, 2, 3 });
        File.WriteAllText(Path.Combine(root, "ok", "One", "one.xml"), """
            <Nini>
              <Section Name="Hello">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="a &amp; b"/>
                <Key Name="assetType" Value=" 20 "/>
                <Key Name="fileName" Value="hello.dat"/>
              </Section>
              <Section Name="EmptyFile">
                <Key Name="assetID" Value="bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="empty"/>
                <Key Name="fileName" Value=""/>
              </Section>
              <Section Name="Wrap">
                <Key Name="assetID" Value="dddddddd-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="wrap"/>
                <Key Name="assetType" Value="128"/>
                <Key Name="fileName" Value="hello.dat"/>
              </Section>
            </Nini>
            """);
        File.WriteAllText(Path.Combine(root, "ok", "broken.xml"), "<Nini><Section");
        File.WriteAllText(Path.Combine(root, "ok", "Two", "two.xml"), """
            <Nini>
              <Section Name="Two">
                <Key Name="assetID" Value="eeeeeeee-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="second"/>
                <Key Name="assetType" Value="+1"/>
                <Key Name="fileName" Value="two.dat"/>
              </Section>
            </Nini>
            """);
        string control = Path.Combine(root, "ok", "AssetSets.xml");
        File.WriteAllText(control, """
            <Nini>
              <Section Name="One"><Key Name="file" Value="One/one.xml"/></Section>
              <Section Name="Broken"><Key Name="file" Value="broken.xml"/></Section>
              <Section Name="Missing"><Key Name="file" Value="no-such.xml"/></Section>
              <Section Name="Two"><Key Name="file" Value="Two/two.xml"/></Section>
            </Nini>
            """);

        JsonArray assets = LoadAssets(control);
        bool random = MissingIdIsRandom(Path.Combine(root, "failures"));
        JsonArray failures = new();
        failures.Add(Failure("duplicate-section", DuplicateSection(root)));
        failures.Add(Failure("missing-file", WrappedSet(root, "missing-file", """
            <Nini>
              <Section Name="Gone">
                <Key Name="assetID" Value="cccccccc-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="gone"/>
                <Key Name="assetType" Value="7"/>
                <Key Name="fileName" Value="missing.dat"/>
              </Section>
            </Nini>
            """)));
        failures.Add(Failure("duplicate-key", WrappedSet(root, "duplicate-key", """
            <Nini>
              <Section Name="Twice">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="one"/>
                <Key Name="name" Value="two"/>
                <Key Name="assetType" Value="0"/>
              </Section>
            </Nini>
            """)));
        failures.Add(Failure("bad-type", WrappedSet(root, "bad-type", """
            <Nini>
              <Section Name="Bad">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="assetType" Value="nope"/>
              </Section>
            </Nini>
            """)));
        failures.Add(Failure("bad-id", WrappedSet(root, "bad-id", """
            <Nini>
              <Section Name="Bad">
                <Key Name="assetID" Value="not-a-uuid"/>
                <Key Name="assetType" Value="0"/>
              </Section>
            </Nini>
            """)));
        failures.Add(Failure("duplicate-set", DuplicateSet(root)));
        failures.Add(Failure("bad-control", BadControl(root)));
        failures.Add(Failure("empty-type", WrappedSet(root, "empty-type", """
            <Nini>
              <Section Name="Empty">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="assetType" Value=""/>
              </Section>
            </Nini>
            """)));
        failures.Add(Failure("overflow-type", WrappedSet(root, "overflow-type", """
            <Nini>
              <Section Name="Overflow">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="assetType" Value="2147483648"/>
              </Section>
            </Nini>
            """)));

        JsonObject document = new()
        {
            ["tranquillity"] = revision,
            ["sample"] = "loader-sample/ok/AssetSets.xml",
            ["assets"] = assets,
            ["missing_asset_id_is_random"] = random,
            ["failures"] = failures
        };
        WriteJson(directory, "loader-sample.json", document);
    }

    static JsonArray LoadAssets(string control)
    {
        JsonArray assets = new();
        new AssetLoaderFileSystem().ForEachDefaultXmlAsset(control, asset => assets.Add(AssetNode(asset)));
        return assets;
    }

    static JsonObject AssetNode(AssetBase asset)
    {
        bool missing = asset.Data == null;
        return new JsonObject
        {
            ["id"] = asset.ID,
            ["name"] = asset.Name,
            ["type"] = asset.Type,
            ["creator"] = asset.CreatorID,
            ["description"] = asset.Description ?? "",
            ["local"] = asset.Local,
            ["temporary"] = asset.Temporary,
            ["flags"] = (int)asset.Flags,
            ["data_null"] = missing,
            ["length"] = missing ? -1 : asset.Data.Length,
            ["sha1"] = missing ? "" : Util.SHA1Hash(asset.Data)
        };
    }

    static bool MissingIdIsRandom(string directory)
    {
        string set = Path.Combine(directory, "no-id-set.xml");
        File.WriteAllText(set, """
            <Nini>
              <Section Name="NoId">
                <Key Name="name" Value="no-id"/>
                <Key Name="assetType" Value="0"/>
              </Section>
            </Nini>
            """);
        string control = Path.Combine(directory, "no-id.xml");
        File.WriteAllText(control, "<Nini><Section Name=\"Only\"><Key Name=\"file\" Value=\"no-id-set.xml\"/></Section></Nini>\n");
        string first = FirstId(control);
        string second = FirstId(control);
        if (first.Length == 0 || first == second)
            throw new InvalidOperationException("a missing assetID was not a fresh UUID");
        return true;
    }

    static string FirstId(string control)
    {
        string id = "";
        new AssetLoaderFileSystem().ForEachDefaultXmlAsset(control, asset =>
        {
            if (id.Length == 0)
                id = asset.ID;
        });
        return id;
    }

    static string DuplicateSection(string root)
    {
        string failures = Path.Combine(root, "failures");
        File.WriteAllText(Path.Combine(failures, "duplicate-section-first.xml"), """
            <Nini>
              <Section Name="Kept">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="kept"/>
                <Key Name="assetType" Value="0"/>
              </Section>
            </Nini>
            """);
        File.WriteAllText(Path.Combine(failures, "duplicate-section-second.xml"), """
            <Nini>
              <Section Name="Dropped">
                <Key Name="assetID" Value="bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="name" Value="dropped"/>
                <Key Name="assetType" Value="1"/>
              </Section>
            </Nini>
            """);
        string path = Path.Combine(failures, "duplicate-section.xml");
        File.WriteAllText(path, """
            <Nini>
              <Section Name="Same"><Key Name="file" Value="duplicate-section-first.xml"/></Section>
              <Section Name="Same"><Key Name="file" Value="duplicate-section-second.xml"/></Section>
            </Nini>
            """);
        return path;
    }

    static string WrappedSet(string root, string name, string body)
    {
        File.WriteAllText(Path.Combine(root, "failures", name + "-set.xml"), body);
        return ControlFor(root, name, name + "-set.xml");
    }

    static string DuplicateSet(string root)
    {
        return WrappedSet(root, "duplicate-set", """
            <Nini>
              <Section Name="Same">
                <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="assetType" Value="0"/>
              </Section>
              <Section Name="Same">
                <Key Name="assetID" Value="bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee"/>
                <Key Name="assetType" Value="0"/>
              </Section>
            </Nini>
            """);
    }

    static string ControlFor(string root, string name, string file)
    {
        string path = Path.Combine(root, "failures", name + ".xml");
        File.WriteAllText(path, "<Nini><Section Name=\"Only\"><Key Name=\"file\" Value=\"" + file + "\"/></Section></Nini>\n");
        return path;
    }

    static string BadControl(string root)
    {
        string path = Path.Combine(root, "failures", "bad-control.xml");
        File.WriteAllText(path, "<Nini>");
        return path;
    }

    static JsonObject Failure(string name, string path)
    {
        JsonArray assets = new();
        bool threw = false;
        string exceptionType = "";
        try
        {
            new AssetLoaderFileSystem().ForEachDefaultXmlAsset(path, asset => assets.Add(AssetNode(asset)));
        }
        catch (Exception exception)
        {
            threw = true;
            exceptionType = exception.GetType().FullName ?? "";
            assets = new();
        }
        return new JsonObject
        {
            ["name"] = name,
            ["file"] = Relative(path),
            ["threw"] = threw,
            ["exception_type"] = exceptionType,
            ["callback_count"] = assets.Count,
            ["assets"] = assets
        };
    }

    static string Relative(string path)
    {
        string marker = "loader-sample" + Path.DirectorySeparatorChar;
        int at = path.LastIndexOf(marker, StringComparison.Ordinal);
        if (at < 0)
            throw new InvalidOperationException("path is outside loader-sample");
        return path.Substring(at).Replace(Path.DirectorySeparatorChar, '/');
    }

    static void WriteSeed(string directory, string revision)
    {
        string fixture = Path.GetFullPath(Path.Combine(directory, "..", "db", "sqlite", "AssetStore.db"));
        if (!File.Exists(fixture))
            throw new InvalidOperationException("missing asset fixture database");
        string temp = Path.Combine(Path.GetTempPath(), "ferroan-asset-seed-" + Guid.NewGuid().ToString("N") + ".db");
        File.Copy(fixture, temp);
        JsonArray cases = new();
        try
        {
            var database = new SQLiteAssetData();
            database.Initialise("Data Source=" + temp + ";Version=3;");
            try
            {
                cases.Add(SeedCase(database, temp, "fresh", "11111111-1111-1111-1111-111111111111", null, "Fresh", "68656c6c6f"));
                cases.Add(SeedCase(database, temp, "keep", "22222222-2222-2222-2222-222222222222", "Old", "New", "68656c6c6f"));
                cases.Add(SeedCase(database, temp, "replace", "33333333-3333-3333-3333-333333333333", "Old", "New", "62626262", "61616161"));
                cases.Add(SeedMissing(database, temp, "missing-new", "44444444-4444-4444-4444-444444444444", false));
                cases.Add(SeedMissing(database, temp, "missing-existing", "55555555-5555-5555-5555-555555555555", true));
            }
            finally
            {
                database.Dispose();
            }
        }
        finally
        {
            TryDelete(temp);
            TryDelete(temp + "-journal");
            TryDelete(temp + "-wal");
            TryDelete(temp + "-shm");
        }
        WriteJson(directory, "seed.json", new JsonObject
        {
            ["tranquillity"] = revision,
            ["cases"] = cases
        });
    }

    static JsonObject SeedCase(SQLiteAssetData database, string dbPath, string name, string id, string beforeName, string inputName, string inputHex, string beforeHex = null)
    {
        JsonObject before = RowAbsent();
        if (beforeName != null)
        {
            database.StoreAsset(Built(id, beforeName, Hex(beforeHex ?? inputHex)));
            before = ReadRow(dbPath, id);
        }
        AssetBase incoming = Built(id, inputName, Hex(inputHex));
        bool threw = false;
        string exceptionType = "";
        try
        {
            Apply(database, incoming);
        }
        catch (Exception exception)
        {
            threw = true;
            exceptionType = exception.GetType().FullName ?? "";
        }
        return new JsonObject
        {
            ["name"] = name,
            ["id"] = id,
            ["before"] = before,
            ["input"] = InputNode(inputName, false, inputHex),
            ["threw"] = threw,
            ["exception_type"] = exceptionType,
            ["after"] = ReadRow(dbPath, id)
        };
    }

    static JsonObject SeedMissing(SQLiteAssetData database, string dbPath, string name, string id, bool existing)
    {
        JsonObject before = RowAbsent();
        if (existing)
        {
            database.StoreAsset(Built(id, "Kept", Hex("68656c6c6f")));
            before = ReadRow(dbPath, id);
        }
        AssetBase incoming = Built(id, "Missing", null);
        incoming.Data = null;
        bool threw = false;
        string exceptionType = "";
        try
        {
            Apply(database, incoming);
        }
        catch (Exception exception)
        {
            threw = true;
            exceptionType = exception.GetType().FullName ?? "";
        }
        return new JsonObject
        {
            ["name"] = name,
            ["id"] = id,
            ["before"] = before,
            ["input"] = InputNode("Missing", true, ""),
            ["threw"] = threw,
            ["exception_type"] = exceptionType,
            ["after"] = ReadRow(dbPath, id)
        };
    }

    static void Apply(SQLiteAssetData database, AssetBase asset)
    {
        AssetBase existing = database.GetAsset(asset.FullID);
        if (existing == null || Util.SHA1Hash(existing.Data) != Util.SHA1Hash(asset.Data))
            database.StoreAsset(asset);
    }

    static AssetBase Built(string id, string name, byte[] data)
    {
        return new AssetBase(new UUID(id), name, 0, LibraryOwner)
        {
            Data = data ?? Array.Empty<byte>(),
            Description = "",
            Local = false,
            Temporary = false
        };
    }

    static JsonObject InputNode(string name, bool dataNull, string hex)
    {
        return new JsonObject
        {
            ["name"] = name,
            ["description"] = "",
            ["type"] = 0,
            ["local"] = false,
            ["temporary"] = false,
            ["flags"] = 0,
            ["creator"] = LibraryOwner,
            ["data_null"] = dataNull,
            ["hex"] = hex
        };
    }

    static JsonObject RowAbsent()
    {
        return new JsonObject { ["present"] = false };
    }

    static JsonObject ReadRow(string dbPath, string id)
    {
        using var connection = new SQLiteConnection("Data Source=" + dbPath + ";Version=3;");
        connection.Open();
        using var command = new SQLiteCommand(
            "SELECT Name, Description, Type, Local, Temporary, asset_flags, CreatorID, Data IS NULL, hex(Data) FROM assets WHERE UUID = @id",
            connection);
        command.Parameters.Add(new SQLiteParameter("@id", id));
        using var reader = command.ExecuteReader();
        if (!reader.Read())
            return RowAbsent();
        bool dataNull = Convert.ToInt32(reader.GetValue(7)) != 0;
        string hex = dataNull || reader.IsDBNull(8) ? "" : reader.GetString(8).ToLowerInvariant();
        return new JsonObject
        {
            ["present"] = true,
            ["name"] = reader.GetString(0),
            ["description"] = reader.IsDBNull(1) ? "" : reader.GetString(1),
            ["type"] = Convert.ToInt32(reader.GetValue(2)),
            ["local"] = Convert.ToBoolean(reader.GetValue(3)),
            ["temporary"] = Convert.ToBoolean(reader.GetValue(4)),
            ["flags"] = Convert.ToInt32(reader.GetValue(5)),
            ["creator"] = reader.IsDBNull(6) ? "" : reader.GetString(6),
            ["data_null"] = dataNull,
            ["hex"] = hex
        };
    }

    static void WriteCommands(string directory, string revision)
    {
        JsonArray parsed = new();
        string[] raws =
        {
            "00112233-4455-6677-8899-aabbccddeeff",
            "00112233-4455-6677-8899-AABBCCDDEEFF",
            "  00112233-4455-6677-8899-aabbccddeeff",
            "00112233-4455-6677-8899-aabbccddeeff  ",
            "  00112233-4455-6677-8899-aabbccddeeff  ",
            "00112233445566778899aabbccddeeff",
            "{00112233-4455-6677-8899-aabbccddeeff}",
            "urn:uuid:00112233-4455-6677-8899-aabbccddeeff",
            "not-a-uuid",
            "",
            "00112233-4455-6677-8899-aabbccddeeff extra",
            "00112233-4455"
        };
        foreach (string raw in raws)
        {
            bool ok = UUID.TryParse(raw, out UUID id);
            parsed.Add(new JsonObject
            {
                ["raw"] = raw,
                ["ok"] = ok,
                ["id"] = ok ? id.ToString() : ""
            });
        }

        JsonArray flags = new();
        foreach (int value in new[] { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1 })
        {
            flags.Add(new JsonObject
            {
                ["value"] = value,
                ["text"] = ((AssetFlags)value).ToString()
            });
        }

        JsonArray types = new();
        foreach (sbyte type in new sbyte[] { -2, -1, 0, 4, 20, 26, 56, 58, 59, 99 })
        {
            AssetBase probe = new AssetBase(UUID.Zero, "n", type, "c");
            types.Add(new JsonObject
            {
                ["value"] = type,
                ["text"] = ((AssetType)type).ToString(),
                ["content_type"] = probe.Metadata.ContentType
            });
        }

        byte[] seventeen = new byte[17];
        for (int i = 0; i < seventeen.Length; i++)
            seventeen[i] = (byte)i;
        byte[] eightyOne = new byte[81];
        for (int i = 0; i < eightyOne.Length; i++)
            eightyOne[i] = (byte)(i * 3);

        AssetBase wood = new AssetBase(new UUID("00112233-4455-6677-8899-aabbccddeeff"), "Wood", 0, "creator")
        {
            Description = "",
            Temporary = true,
            Data = seventeen
        };
        wood.Flags = AssetFlags.Maptile | AssetFlags.Rewritable | AssetFlags.Collectable;
        AssetBase odd = new AssetBase(new UUID("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"), "Odd", 4, "creator")
        {
            Description = "line",
            Temporary = false,
            Data = eightyOne
        };
        odd.Flags = (AssetFlags)9;
        AssetBase empty = new AssetBase(new UUID("bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee"), "Empty", 7, "creator")
        {
            Data = Array.Empty<byte>()
        };

        JsonArray show = new()
        {
            Show(new[] { "show", "asset" }, null),
            Show(new[] { "show", "asset", "extra", "ignored" }, null),
            Show(new[] { "show", "asset", "00112233-4455-6677-8899-aabbccddeeff" }, null),
            Show(new[] { "show", "asset", "00112233-4455-6677-8899-aabbccddeeff" }, empty),
            Show(new[] { "show", "asset", "00112233-4455-6677-8899-aabbccddeeff", "extra" }, wood),
            Show(new[] { "show", "asset", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" }, odd)
        };

        JsonArray delete = new()
        {
            Delete(new[] { "delete", "asset" }, null, false),
            Delete(new[] { "delete", "asset", "raw-token" }, null, false),
            Delete(new[] { "delete", "asset", "raw-token" }, empty, false),
            Delete(new[] { "delete", "asset", "  00112233-4455-6677-8899-aabbccddeeff" }, wood, true),
            Delete(new[] { "delete", "asset", "00112233-4455-6677-8899-aabbccddeeff" }, wood, false)
        };

        JsonArray dump = new()
        {
            Dump(new[] { "dump", "asset" }, false, false, null),
            Dump(new[] { "dump", "asset", "not-a-uuid" }, false, false, null),
            Dump(new[] { "dump", "asset", "00112233445566778899aabbccddeeff" }, false, false, null),
            Dump(new[] { "dump", "asset", "  00112233-4455-6677-8899-aabbccddeeff" }, true, true, seventeen),
            Dump(new[] { "dump", "asset", "00112233-4455-6677-8899-aabbccddeeff", "extra" }, true, false, Array.Empty<byte>())
        };

        WriteJson(directory, "commands.json", new JsonObject
        {
            ["tranquillity"] = revision,
            ["try_parse"] = parsed,
            ["flags"] = flags,
            ["types"] = types,
            ["show"] = show,
            ["delete"] = delete,
            ["dump"] = dump
        });
    }

    static JsonObject Show(string[] args, AssetBase asset)
    {
        JsonArray lines = new();
        if (args.Length < 3)
            lines.Add("Syntax: show asset <ID>");
        else if (asset == null || asset.Data.Length == 0)
            lines.Add("Asset not found");
        else
        {
            lines.Add(string.Format("Name: {0}", asset.Name));
            lines.Add(string.Format("Description: {0}", asset.Description));
            lines.Add(string.Format("Type: {0} (type number = {1})", (AssetType)asset.Type, asset.Type));
            lines.Add(string.Format("Content-type: {0}", asset.Metadata.ContentType));
            lines.Add(string.Format("Size: {0} bytes", asset.Data.Length));
            lines.Add(string.Format("Temporary: {0}", asset.Temporary ? "yes" : "no"));
            lines.Add(string.Format("Flags: {0}", asset.Metadata.Flags));
            for (int i = 0; i < 5; i++)
            {
                int off = i * 16;
                if (asset.Data.Length <= off)
                    break;
                int len = 16;
                if (asset.Data.Length < off + len)
                    len = asset.Data.Length - off;
                byte[] line = new byte[len];
                Array.Copy(asset.Data, off, line, 0, len);
                lines.Add(string.Format("{0:x4}: {1}", off, BitConverter.ToString(line)));
            }
        }
        return new JsonObject
        {
            ["args"] = Args(args),
            ["asset"] = asset == null ? AbsentAsset() : Shown(asset),
            ["lines"] = lines
        };
    }

    static JsonObject Shown(AssetBase asset)
    {
        return new JsonObject
        {
            ["present"] = true,
            ["id"] = asset.ID,
            ["name"] = asset.Name,
            ["description"] = asset.Description ?? "",
            ["type"] = asset.Type,
            ["temporary"] = asset.Temporary,
            ["flags"] = (int)asset.Flags,
            ["hex"] = Convert.ToHexString(asset.Data).ToLowerInvariant()
        };
    }

    static JsonObject AbsentAsset()
    {
        return new JsonObject { ["present"] = false };
    }

    static JsonObject Delete(string[] args, AssetBase asset, bool deleted)
    {
        JsonArray lines = new();
        bool found = asset != null && asset.Data.Length != 0;
        if (args.Length < 3)
            lines.Add("Syntax: delete asset <ID>");
        else if (!found)
            lines.Add(string.Format("Could not find asset with ID {0}", args[2]));
        else if (!deleted)
            lines.Add(string.Format("ERROR: Could not delete asset {0} {1}", asset.ID, asset.Name));
        else
            lines.Add(string.Format("Deleted asset {0} {1}", asset.ID, asset.Name));
        return new JsonObject
        {
            ["args"] = Args(args),
            ["asset"] = found ? Shown(asset) : AbsentAsset(),
            ["deleted"] = deleted,
            ["lines"] = lines
        };
    }

    static JsonObject Dump(string[] args, bool found, bool fileExists, byte[] data)
    {
        JsonArray lines = new();
        string writeHex = "";
        bool write = false;
        if (args.Length < 3)
            lines.Add("Usage is dump asset <ID>");
        else if (!UUID.TryParse(args[2], out UUID assetId))
            lines.Add(string.Format("ERROR: {0} is not a valid ID format", args[2]));
        else if (!found)
            lines.Add(string.Format("ERROR: No asset found with ID {0}", assetId));
        else
        {
            string fileName = args[2];
            if (fileExists)
                lines.Add(string.Format("File {0} already exists.  Please move or remove it.", fileName));
            else
            {
                write = true;
                writeHex = Convert.ToHexString(data).ToLowerInvariant();
                lines.Add(string.Format("Asset dumped to file {0}", fileName));
            }
        }
        return new JsonObject
        {
            ["args"] = Args(args),
            ["found"] = found,
            ["file_exists"] = fileExists,
            ["hex"] = data == null ? "" : Convert.ToHexString(data).ToLowerInvariant(),
            ["write"] = write,
            ["write_hex"] = writeHex,
            ["lines"] = lines
        };
    }

    static JsonArray Args(string[] args)
    {
        JsonArray array = new();
        foreach (string arg in args)
            array.Add(arg);
        return array;
    }

    static void WriteLibrary(string directory, string revision)
    {
        string path = LibraryPath();
        if (!File.Exists(path))
            throw new InvalidOperationException("missing library asset set " + path);
        JsonArray first = LoadAssets(path);
        JsonArray second = LoadAssets(path);
        if (first.Count != second.Count)
            throw new InvalidOperationException("library asset count changed between loads");
        for (int i = 0; i < first.Count; i++)
        {
            if (first[i]["id"].ToString() != second[i]["id"].ToString())
                throw new InvalidOperationException("library asset " + first[i]["name"] + " has no stable id");
        }
        WriteJson(directory, "library-manifest.json", new JsonObject
        {
            ["tranquillity"] = revision,
            ["count"] = first.Count,
            ["assets"] = first
        });
    }

    static string LibraryPath()
    {
        string root = Environment.GetEnvironmentVariable("FERROAN_REF");
        if (string.IsNullOrEmpty(root))
        {
            string home = Environment.GetEnvironmentVariable("HOME");
            if (string.IsNullOrEmpty(home))
                home = Environment.GetEnvironmentVariable("USERPROFILE");
            if (string.IsNullOrEmpty(home))
                throw new InvalidOperationException("set FERROAN_REF to the reference checkouts root");
            root = Path.Combine(home, "Git", "reference");
        }
        return Path.Combine(
            root,
            "OpenSim-Tranquillity",
            "Source",
            "OpenSim.Services.InventoryService",
            "AppData",
            "assets",
            "AssetSets.xml");
    }

    static byte[] Hex(string hex)
    {
        return Convert.FromHexString(hex);
    }

    static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path))
                File.Delete(path);
        }
        catch (IOException)
        {
        }
    }

    static void WriteJson(string directory, string name, JsonObject document)
    {
        File.WriteAllText(Path.Combine(directory, name), document.ToJsonString(ProgramOptions.Json) + "\n");
    }
}

static class ProgramOptions
{
    public static readonly System.Text.Json.JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        NewLine = "\n"
    };
}
