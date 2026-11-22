// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// One GridUserService and POST /griduser sequence on a new SQLite file.
// stdout is one JSON object. The connection string is not printed.

using System.Text;
using System.Text.Json.Nodes;
using Nini.Config;
using OpenSim.Data;
using OpenSim.Framework;
using OpenSim.Framework.Console;
using OpenSim.Server.Handlers.GridUser;
using OpenSim.Services.Interfaces;
using OpenSim.Services.UserAccountService;

static class ServiceMode
{
    public static int Run()
    {
        string text = Console.In.ReadToEnd();
        if (text.Length > 1024 * 1024)
            throw new InvalidOperationException("json is larger than 1 MiB");
        JsonNode body = JsonNode.Parse(text) ?? throw new InvalidOperationException("empty json");
        if (body["steps"] is not JsonArray steps)
            throw new InvalidOperationException("steps must be an array");

        string database = Path.Combine(Path.GetTempPath(), "ferroan-griduser-service-" + Guid.NewGuid().ToString("N") + ".db");
        var console = new RecordingConsole();
        MainConsole.Instance = console;
        try
        {
            string dll = Path.Combine(AppContext.BaseDirectory, "OpenSim.Data.SQLite.dll");
            if (!File.Exists(dll))
                throw new InvalidOperationException("OpenSim.Data.SQLite.dll is not next to the oracle");
            var source = new IniConfigSource();
            IConfig section = source.AddConfig("GridUserService");
            section.Set("StorageProvider", dll + ":SQLiteGridUserData");
            section.Set("ConnectionString", "Data Source=" + database + ";Version=3;Pooling=False;");
            section.Set("Realm", "GridUser");

            var service = new ProbeService(source);
            var handler = new ProbeHandler(service);
            var output = new JsonArray();
            foreach (JsonNode step in steps)
            {
                if (step == null)
                    throw new InvalidOperationException("empty step");
                int sampled = Util.UnixTimeSinceEpoch();
                string op = step["op"].GetValue<string>();
                var row = new JsonObject { ["op"] = op };
                switch (op)
                {
                    case "post":
                        string request = step["body"].GetValue<string>();
                        row["body"] = request;
                        Copy(step, row, "time_user");
                        Copy(step, row, "time_field");
                        byte[] response = handler.Post(request);
                        row["unix"] = Written(service, step, sampled);
                        row["response"] = Encoding.UTF8.GetString(response);
                        break;
                    case "store":
                        string userId = step["user_id"].GetValue<string>();
                        row["user_id"] = userId;
                        row["data"] = step["data"].DeepClone();
                        var data = new GridUserData { UserID = userId, Data = Strings(step["data"]) };
                        row["unix"] = sampled;
                        row["stored"] = service.StoreRaw(data);
                        break;
                    case "console":
                        string[] argv = StringsOf(step["argv"]);
                        var argvNode = new JsonArray();
                        foreach (string word in argv)
                            argvNode.Add(word);
                        row["argv"] = argvNode;
                        console.Clear();
                        if (argv.Length >= 4 && argv[0] == "show" && argv[1] == "grid" && argv[2] == "user")
                            service.ShowUser(argv);
                        else if (argv.Length >= 4 && argv[0] == "show" && argv[1] == "grid" && argv[2] == "users" && argv[3] == "online")
                            service.ShowOnline(argv);
                        else
                            service.ShowUser(argv);
                        row["unix"] = sampled;
                        row["text"] = console.Text;
                        break;
                    default:
                        throw new InvalidOperationException("unknown service step");
                }
                output.Add(row);
            }
            Console.WriteLine(new JsonObject { ["steps"] = output }.ToJsonString());
            return 0;
        }
        finally
        {
            TryDelete(database);
        }
    }

    static int Written(ProbeService service, JsonNode step, int sampled)
    {
        if (step["time_user"] == null || step["time_field"] == null)
            return sampled;
        string user = step["time_user"].GetValue<string>();
        string field = step["time_field"].GetValue<string>();
        GridUserData row = service.Read(user);
        if (row == null || row.Data == null || !row.Data.TryGetValue(field, out string value))
            return sampled;
        if (int.TryParse(value, out int unix))
            return unix;
        return sampled;
    }

    static void Copy(JsonNode from, JsonObject to, string name)
    {
        if (from[name] != null)
            to[name] = from[name].DeepClone();
    }

    static Dictionary<string, string> Strings(JsonNode node)
    {
        var data = new Dictionary<string, string>();
        if (node is JsonObject fields)
        {
            foreach (KeyValuePair<string, JsonNode> pair in fields)
                data[pair.Key] = pair.Value == null ? "" : pair.Value.GetValue<string>();
        }
        return data;
    }

    static string[] StringsOf(JsonNode node)
    {
        if (node is not JsonArray array)
            return Array.Empty<string>();
        var words = new string[array.Count];
        for (int i = 0; i < array.Count; i++)
            words[i] = array[i].GetValue<string>();
        return words;
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
}

sealed class ProbeService : GridUserService
{
    public ProbeService(IConfigSource config) : base(config) {}

    public void ShowUser(string[] argv) => HandleShowGridUser("Users", argv);

    public void ShowOnline(string[] argv) => HandleShowGridUsersOnline("Users", argv);

    public bool StoreRaw(GridUserData row) => m_Database.Store(row);

    public GridUserData Read(string userId) => m_Database.Get(userId);
}

sealed class ProbeHandler : GridUserServerPostHandler
{
    public ProbeHandler(IGridUserService service) : base(service, null) {}

    public byte[] Post(string body)
    {
        using var stream = new MemoryStream(Encoding.UTF8.GetBytes(body));
        return ProcessRequest("/griduser", stream, null, null);
    }
}

sealed class RecordingConsole : ICommandConsole
{
#pragma warning disable 0067
    public event OnOutputDelegate OnOutput;
#pragma warning restore 0067

    readonly MockCommands commands = new MockCommands();
    readonly StringBuilder text = new StringBuilder();

    public string Text => text.ToString();

    public void Clear() => text.Clear();

    public ICommands Commands => commands;

    public string DefaultPrompt { get; set; }

    public IScene ConsoleScene { get; set; }

    public void Prompt() {}

    public void RunCommand(string cmd) {}

    public string ReadLine(string p, bool isCommand, bool e) => "";

    public void WriteLine(string s) => text.Append(s).Append('\n');

    public void Output(string format) => text.Append(format).Append('\n');

    public void Output(string format, params object[] components)
    {
        if (components != null && components.Length > 0 && components[0] is ConsoleLevel)
        {
            if (components.Length == 1)
                components = null;
            else
            {
                object[] rest = new object[components.Length - 1];
                Array.Copy(components, 1, rest, 0, rest.Length);
                components = rest;
            }
        }
        string line = components == null || components.Length == 0 ? format : string.Format(format, components);
        text.Append(line).Append('\n');
    }

    public string Prompt(string p) => "";
    public string Prompt(string p, string def) => "";
    public string Prompt(string p, List<char> excludedCharacters) => "";
    public string Prompt(string p, string def, List<char> excludedCharacters, bool echo) => "";
    public string Prompt(string prompt, string defaultResponse, List<string> options) => "";
    public string PasswdPrompt(string p) => "";
    public void ReadConfig(IConfigSource configSource) {}
    public void SetCntrCHandler(OnCntrCCelegate handler) {}
}
