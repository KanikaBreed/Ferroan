// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// One plugin call against a database Tranquillity already migrated.
// stdout is one JSON object. The connection string stays in the environment.

using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Framework;

namespace Ferroan.AssetOracle;

static class DbMode
{
    public static int Run(string[] args)
    {
        if (args.Length != 2)
        {
            Console.Error.WriteLine("usage: AssetOracle db put|get|exists|delete|metadata");
            return 2;
        }

        string backend = Environment.GetEnvironmentVariable("FERROAN_ASSET_BACKEND") ?? "";
        string ado = Environment.GetEnvironmentVariable("FERROAN_ASSET_ADO") ?? "";
        if (backend.Length == 0 || ado.Length == 0)
        {
            Console.Error.WriteLine("FERROAN_ASSET_BACKEND and FERROAN_ASSET_ADO are required");
            return 2;
        }

        JsonNode body = JsonNode.Parse(Console.In.ReadToEnd())
            ?? throw new InvalidOperationException("empty json");
        AssetDataBase plugin = Open(backend, ado);
        try
        {
            switch (args[1])
            {
                case "put":
                    Put(plugin, body);
                    break;
                case "get":
                    Get(plugin, body);
                    break;
                case "exists":
                    Exists(plugin, body);
                    break;
                case "delete":
                    Delete(plugin, body);
                    break;
                case "metadata":
                    Metadata(plugin, body);
                    break;
                default:
                    Console.Error.WriteLine("unknown db operation");
                    return 2;
            }
        }
        finally
        {
            plugin.Dispose();
        }

        return 0;
    }

    public static string Scrub(string text)
    {
        return Regex.Replace(text, "(?i)(password\\s*=\\s*)([^;\\s'\"]+)", "$1redacted");
    }

    static AssetDataBase Open(string backend, string ado)
    {
        AssetDataBase plugin = backend switch
        {
            "sqlite" => new OpenSim.Data.SQLite.SQLiteAssetData(),
            "mysql" => new OpenSim.Data.MySQL.MySQLAssetData(),
            "postgres" => new OpenSim.Data.PGSQL.PGSQLAssetData(),
            _ => throw new InvalidOperationException("unknown backend")
        };
        plugin.Initialise(ado);
        return plugin;
    }

    static void Put(AssetDataBase plugin, JsonNode body)
    {
        var asset = new AssetBase(
            UUID.Parse(Text(body, "id")),
            Text(body, "name"),
            (sbyte)body["type"].GetValue<int>(),
            Text(body, "creator"));
        asset.Description = Text(body, "description");
        asset.Local = body["local"].GetValue<bool>();
        asset.Temporary = body["temporary"].GetValue<bool>();
        asset.Flags = (AssetFlags)body["flags"].GetValue<int>();
        asset.Data = Bytes(body["data"]);
        bool stored = plugin.StoreAsset(asset);
        Write(new JsonObject { ["stored"] = stored });
    }

    static void Get(AssetDataBase plugin, JsonNode body)
    {
        AssetBase asset = plugin.GetAsset(UUID.Parse(Text(body, "id")));
        if (asset == null)
        {
            Write(new JsonObject { ["found"] = false });
            return;
        }

        Write(new JsonObject
        {
            ["found"] = true,
            ["id"] = asset.FullID.ToString(),
            ["name"] = asset.Name ?? "",
            ["description"] = asset.Description ?? "",
            ["type"] = asset.Type,
            ["local"] = asset.Local,
            ["temporary"] = asset.Temporary,
            ["flags"] = (int)asset.Flags,
            ["creator"] = asset.CreatorID ?? "",
            ["data"] = ByteArray(asset.Data)
        });
    }

    static void Exists(AssetDataBase plugin, JsonNode body)
    {
        JsonArray ids = body["ids"].AsArray();
        var uuids = new UUID[ids.Count];
        for (int i = 0; i < ids.Count; i++)
            uuids[i] = UUID.Parse(ids[i].GetValue<string>());
        bool[] found = plugin.AssetsExist(uuids);
        var array = new JsonArray();
        foreach (bool item in found)
            array.Add(item);
        Write(new JsonObject { ["exists"] = array });
    }

    static void Delete(AssetDataBase plugin, JsonNode body)
    {
        Write(new JsonObject { ["deleted"] = plugin.Delete(Text(body, "id")) });
    }

    static void Metadata(AssetDataBase plugin, JsonNode body)
    {
        List<AssetMetadata> rows = plugin.FetchAssetMetadataSet(
            body["start"].GetValue<int>(),
            body["count"].GetValue<int>());
        var array = new JsonArray();
        foreach (AssetMetadata row in rows)
        {
            array.Add(new JsonObject
            {
                ["id"] = row.FullID.ToString(),
                ["name"] = row.Name ?? "",
                ["description"] = row.Description ?? "",
                ["type"] = row.Type,
                ["temporary"] = row.Temporary,
                ["flags"] = (int)row.Flags,
                ["creator"] = row.CreatorID ?? ""
            });
        }

        Write(new JsonObject { ["rows"] = array });
    }

    static byte[] Bytes(JsonNode node)
    {
        JsonArray array = node.AsArray();
        var data = new byte[array.Count];
        for (int i = 0; i < array.Count; i++)
            data[i] = (byte)array[i].GetValue<int>();
        return data;
    }

    static JsonArray ByteArray(byte[] data)
    {
        var array = new JsonArray();
        if (data != null)
        {
            foreach (byte value in data)
                array.Add((int)value);
        }
        return array;
    }

    static string Text(JsonNode body, string name)
    {
        return body[name].GetValue<string>();
    }

    static void Write(JsonNode node)
    {
        Console.WriteLine(node.ToJsonString());
    }
}
