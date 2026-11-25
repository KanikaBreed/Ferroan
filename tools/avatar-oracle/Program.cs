// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// One IAvatarData call against a database Tranquillity already migrated.
// stdout is one JSON object. The connection string stays in the environment.

using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using OpenMetaverse;
using OpenSim.Data;

const int MaxPayload = 1024 * 1024;

if (args.Length == 5 && args[0] is ("service" or "service-mysql") && ushort.TryParse(args[3], out var port) && port != 0)
{
    try { return ServiceCases.Run(args[1], args[2], port, args[4], args[0] == "service-mysql"); }
    catch (Exception error)
    {
        Console.Error.WriteLine(Scrub(error.Message));
        return 1;
    }
}

if (args.Length != 1 || !IsOperation(args[0]))
{
    Console.Error.WriteLine("usage: AvatarOracle get|upsert|delete|delete-principal, or service generate|verify <pin> <port> <fixture>");
    return 2;
}

string backend = Environment.GetEnvironmentVariable("FERROAN_AVATAR_BACKEND") ?? "";
string ado = Environment.GetEnvironmentVariable("FERROAN_AVATAR_ADO") ?? "";
string realm = Environment.GetEnvironmentVariable("FERROAN_AVATAR_REALM");
if (string.IsNullOrEmpty(realm))
    realm = "Avatars";
if (backend.Length == 0 || ado.Length == 0)
{
    Console.Error.WriteLine("FERROAN_AVATAR_BACKEND and FERROAN_AVATAR_ADO are required");
    return 2;
}

try
{
    string text = Console.In.ReadToEnd();
    if (text.Length > MaxPayload)
        throw new InvalidOperationException("json is larger than 1 MiB");
    JsonNode body = JsonNode.Parse(text) ?? throw new InvalidOperationException("empty json");
    IAvatarData plugin = Open(backend, ado, realm);
    switch (args[0])
    {
        case "get":
            Write(Rows(plugin.Get("PrincipalID", Text(body, "principal_id"))));
            break;
        case "upsert":
            Write(new JsonObject { ["stored"] = plugin.Store(ToData(body)) });
            break;
        case "delete":
            Write(new JsonObject
            {
                ["deleted"] = plugin.Delete(UUID.Parse(Text(body, "principal_id")), Text(body, "name"))
            });
            break;
        case "delete-principal":
            Write(new JsonObject
            {
                ["deleted"] = plugin.Delete("PrincipalID", Text(body, "principal_id"))
            });
            break;
        default:
            Console.Error.WriteLine("unknown avatar operation");
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
    return name == "get" || name == "upsert" || name == "delete" || name == "delete-principal";
}

static IAvatarData Open(string backend, string ado, string realm)
{
    return backend switch
    {
        "sqlite" => new OpenSim.Data.SQLite.SQLiteAvatarData(ado, realm),
        "mysql" => new OpenSim.Data.MySQL.MySQLAvatarData(ado, realm),
        "postgres" => new OpenSim.Data.PGSQL.PGSQLAvatarData(ado, realm),
        _ => throw new InvalidOperationException("unknown backend")
    };
}

static JsonObject Rows(AvatarBaseData[] rows)
{
    var array = new JsonArray();
    if (rows != null)
    {
        foreach (AvatarBaseData row in rows)
            array.Add(Row(row));
    }
    return new JsonObject { ["rows"] = array };
}

static JsonObject Row(AvatarBaseData row)
{
    string name = "";
    string value = "";
    if (row.Data != null)
    {
        if (row.Data.TryGetValue("Name", out string storedName) && storedName != null)
            name = storedName;
        if (row.Data.TryGetValue("Value", out string storedValue) && storedValue != null)
            value = storedValue;
    }
    return new JsonObject
    {
        ["principal_id"] = row.PrincipalID.ToString(),
        ["name"] = name,
        ["value"] = value
    };
}

static AvatarBaseData ToData(JsonNode body)
{
    var data = new AvatarBaseData();
    data.PrincipalID = UUID.Parse(Text(body, "principal_id"));
    data.Data = new Dictionary<string, string>
    {
        ["Name"] = Text(body, "name"),
        ["Value"] = Text(body, "value")
    };
    return data;
}

static string Text(JsonNode body, string name)
{
    return body[name] == null ? "" : body[name].GetValue<string>();
}

static void Write(JsonNode node)
{
    Console.WriteLine(node.ToJsonString());
}

static string Scrub(string text)
{
    return Regex.Replace(text, "(?i)(password\\s*=\\s*)([^;\\s'\"]+)", "$1***");
}
