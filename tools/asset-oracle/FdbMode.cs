// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// One FSAssets plugin call against a database Tranquillity already migrated.
// stdout is one JSON object. The connection string stays in the environment.

using System.Text.Json.Nodes;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Framework;

namespace Ferroan.AssetOracle;

static class FdbMode
{
    public static int Run(string[] args)
    {
        if (args.Length != 2)
        {
            Console.Error.WriteLine("usage: AssetOracle fdb put|get|exists|delete|count|import");
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
        MainConsole.Instance = new QuietConsole();
        IFSAssetDataPlugin plugin = Open(backend, ado);
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
                case "count":
                    Write(new JsonObject { ["count"] = plugin.Count() });
                    break;
                case "import":
                    Import(plugin, ado, body);
                    break;
                default:
                    Console.Error.WriteLine("unknown fdb operation");
                    return 2;
            }
        }
        finally
        {
            plugin.Dispose();
        }

        return 0;
    }

    static IFSAssetDataPlugin Open(string backend, string ado)
    {
        IFSAssetDataPlugin plugin = backend switch
        {
            "mysql" => new OpenSim.Data.MySQL.MySQLFSAssetData(),
            "postgres" => new OpenSim.Data.PGSQL.PGSQLFSAssetData(),
            _ => throw new InvalidOperationException("unknown backend")
        };
        plugin.Initialise(ado, "fsassets", 0);
        return plugin;
    }

    static void Put(IFSAssetDataPlugin plugin, JsonNode body)
    {
        string id = Text(body, "id");
        var meta = new AssetMetadata
        {
            ID = id,
            FullID = UUID.Parse(id),
            Name = Text(body, "name"),
            Description = Text(body, "description"),
            Type = (sbyte)body["type"].GetValue<int>(),
            Flags = (AssetFlags)body["flags"].GetValue<int>()
        };
        // "force" is the force import's update (Tranquillity #524, TB45).
        bool force = body["force"]?.GetValue<bool>() ?? false;
        bool stored = plugin.Store(meta, Text(body, "hash"), force);
        Write(new JsonObject { ["stored"] = stored });
    }

    static void Get(IFSAssetDataPlugin plugin, JsonNode body)
    {
        string hash;
        AssetMetadata meta = plugin.Get(Text(body, "id"), out hash);
        if (meta == null)
        {
            Write(new JsonObject { ["found"] = false });
            return;
        }

        Write(new JsonObject
        {
            ["found"] = true,
            ["id"] = meta.ID ?? "",
            ["name"] = meta.Name ?? "",
            ["description"] = meta.Description ?? "",
            ["type"] = meta.Type,
            ["flags"] = (int)meta.Flags,
            ["hash"] = hash ?? ""
        });
    }

    static void Exists(IFSAssetDataPlugin plugin, JsonNode body)
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

    static void Delete(IFSAssetDataPlugin plugin, JsonNode body)
    {
        Write(new JsonObject { ["deleted"] = plugin.Delete(Text(body, "id")) });
    }

    static void Import(IFSAssetDataPlugin plugin, string ado, JsonNode body)
    {
        var rows = new JsonArray();
        plugin.Import(
            ado,
            Text(body, "table"),
            body["start"].GetValue<int>(),
            body["count"].GetValue<int>(),
            false,
            (asset, force) =>
            {
                rows.Add(new JsonObject
                {
                    ["id"] = asset.FullID.ToString(),
                    ["name"] = asset.Name ?? "",
                    ["description"] = asset.Description ?? "",
                    ["type"] = asset.Type,
                    ["data"] = ByteArray(asset.Data)
                });
                return asset.ID;
            });
        Write(new JsonObject { ["rows"] = rows });
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

    sealed class QuietConsole : ICommandConsole
    {
        public event OnOutputDelegate OnOutput;

        public IScene ConsoleScene { get; set; }

        public ICommands Commands { get { return null; } }

        public string DefaultPrompt { get; set; }

        public void Output(string format)
        {
            OnOutput?.Invoke(format);
        }

        public void Output(string format, params object[] components)
        {
            OnOutput?.Invoke(format);
        }

        public string Prompt(string p)
        {
            return "";
        }

        public string Prompt(string p, string def)
        {
            return def ?? "";
        }

        public string Prompt(string p, List<char> excludedCharacters)
        {
            return "";
        }

        public string Prompt(string p, string def, List<char> excludedCharacters, bool echo = true)
        {
            return def ?? "";
        }

        public string Prompt(string prompt, string defaultresponse, List<string> options)
        {
            return defaultresponse ?? "";
        }

        public void Prompt()
        {
        }

        public void RunCommand(string cmd)
        {
        }

        public string ReadLine(string p, bool isCommand, bool e)
        {
            return "";
        }

        public void WriteLine(string s)
        {
        }

        public void ReadConfig(IConfigSource configSource)
        {
        }

        public void SetCntrCHandler(OnCntrCCelegate handler)
        {
        }
    }
}
