// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// One IGridUserData call against a database Tranquillity already migrated.
// stdout is one JSON object. The connection string stays in the environment.

using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using OpenMetaverse;
using OpenSim.Data;

const int MaxPayload = 1024 * 1024;

if (args.Length != 1 || !IsOperation(args[0]))
{
    Console.Error.WriteLine("usage: GridUserOracle get|getall|getmany|online|store");
    return 2;
}

string backend = Environment.GetEnvironmentVariable("FERROAN_GRIDUSER_BACKEND") ?? "";
string ado = Environment.GetEnvironmentVariable("FERROAN_GRIDUSER_ADO") ?? "";
string realm = Environment.GetEnvironmentVariable("FERROAN_GRIDUSER_REALM");
if (string.IsNullOrEmpty(realm))
    realm = "GridUser";
if (backend.Length == 0 || ado.Length == 0)
{
    Console.Error.WriteLine("FERROAN_GRIDUSER_BACKEND and FERROAN_GRIDUSER_ADO are required");
    return 2;
}

try
{
    string text = Console.In.ReadToEnd();
    if (text.Length > MaxPayload)
        throw new InvalidOperationException("json is larger than 1 MiB");
    JsonNode body = JsonNode.Parse(text) ?? throw new InvalidOperationException("empty json");
    IGridUserData plugin = Open(backend, ado, realm);
    switch (args[0])
    {
        case "get":
            Write(GetOne(plugin, Text(body, "user_id")));
            break;
        case "getall":
            Write(Rows(plugin.GetAll(Text(body, "user_id"))));
            break;
        case "getmany":
            Write(Rows(plugin.GetMany(Ids(body))));
            break;
        case "online":
            Write(Rows(plugin.GetOnlineInRegion(UUID.Parse(Text(body, "region_id")))));
            break;
        case "store":
            Write(new JsonObject { ["stored"] = plugin.Store(ToData(body)) });
            break;
        default:
            Console.Error.WriteLine("unknown grid user operation");
            return 2;
    }
}
catch (Exception error)
{
    Console.Error.WriteLine(Scrub(error.ToString()));
    return 1;
}

return 0;

static bool IsOperation(string name)
{
    return name == "get" || name == "getall" || name == "getmany" || name == "online" || name == "store";
}

static IGridUserData Open(string backend, string ado, string realm)
{
    return backend switch
    {
        "sqlite" => new OpenSim.Data.SQLite.SQLiteGridUserData(ado, realm),
        "mysql" => new OpenSim.Data.MySQL.MySQLGridUserData(ado, realm),
        "postgres" => new OpenSim.Data.PGSQL.PGSQLGridUserData(ado, realm),
        _ => throw new InvalidOperationException("unknown backend")
    };
}

static JsonObject GetOne(IGridUserData plugin, string userId)
{
    GridUserData row = plugin.Get(userId);
    if (row == null)
        return new JsonObject { ["found"] = false };
    JsonObject found = Row(row);
    found["found"] = true;
    return found;
}

static JsonObject Rows(GridUserData[] rows)
{
    var array = new JsonArray();
    if (rows != null)
    {
        foreach (GridUserData row in rows)
            array.Add(Row(row));
    }
    return new JsonObject { ["rows"] = array };
}

static JsonObject Row(GridUserData row)
{
    var data = new JsonObject();
    if (row.Data != null)
    {
        foreach (KeyValuePair<string, string> pair in row.Data)
            data[pair.Key] = pair.Value ?? "";
    }
    return new JsonObject
    {
        ["user_id"] = row.UserID ?? "",
        ["data"] = data
    };
}

static GridUserData ToData(JsonNode body)
{
    var data = new Dictionary<string, string>();
    if (body["data"] is JsonObject fields)
    {
        foreach (KeyValuePair<string, JsonNode> pair in fields)
            data[pair.Key] = pair.Value == null ? "" : pair.Value.GetValue<string>();
    }
    return new GridUserData
    {
        UserID = Text(body, "user_id"),
        Data = data
    };
}

static string[] Ids(JsonNode body)
{
    if (body["user_ids"] is not JsonArray array)
        return Array.Empty<string>();
    var ids = new string[array.Count];
    for (int i = 0; i < array.Count; i++)
        ids[i] = array[i].GetValue<string>();
    return ids;
}

static string Text(JsonNode body, string name)
{
    return body[name].GetValue<string>();
}

static void Write(JsonNode node)
{
    Console.WriteLine(node.ToJsonString());
}

static string Scrub(string text)
{
    return Regex.Replace(text, "(?i)(password\\s*=\\s*)([^;\\s'\"]+)", "$1***");
}
