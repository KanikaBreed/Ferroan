// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls Tranquillity's ServerUtils and writes the strings it returns.
// tools/http-oracle does not copy that C# source.

using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenSim.Server.Base;

namespace Ferroan.HttpOracle;

static class Program
{
    const string FormatLine = "format 1";

    static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        NewLine = "\n"
    };

    static int Main(string[] args)
    {
        if (args.Length != 4
            || args[2] != "--revision"
            || (args[0] != "generate" && args[0] != "verify" && args[0] != "status-generate" && args[0] != "status-verify"))
        {
            Console.Error.WriteLine("usage: HttpOracle generate|verify|status-generate|status-verify <directory> --revision <40 hex>");
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
            if (args[0].StartsWith("status-", StringComparison.Ordinal))
            {
                StatusCases.Run(directory, revision, args[0] == "status-generate");
                return 0;
            }
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
        WriteTree(scratch, revision);
        Publish(scratch, directory);
        Console.WriteLine("generate: " + Cases.Build().Count + " http cases");
    }

    static void Verify(string directory, string revision)
    {
        string scratch = directory + ".verify";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision);
        CompareTrees(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("verify: " + Cases.Build().Count + " http cases match ServerUtils");
    }

    static void WriteTree(string directory, string revision)
    {
        List<string> manifest = [FormatLine, "revision " + revision, "source ServerUtils.cs", ""];
        foreach (Case item in Cases.Build())
        {
            manifest.Add(item.Id);
            string first = Execute(item);
            string second = Execute(item);
            if (first != second)
                throw new InvalidOperationException(item.Id + " differed between two calls");
            File.WriteAllText(Path.Combine(directory, item.Id + ".json"), first, Utf8());
        }
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), string.Join("\n", manifest) + "\n", Utf8());
    }

    static string Execute(Case item)
    {
        JsonObject doc = new()
        {
            ["kind"] = item.Kind
        };
        try
        {
            if (item.Kind is "query-parse" or "xml-parse")
            {
                doc["input"] = item.Raw;
                if (item.Kind == "query-parse")
                {
                    Dictionary<string, object> parsed = ServerUtils.ParseQueryString(item.Raw);
                    doc["parsed"] = DescribeMap(parsed);
                    doc["built"] = ServerUtils.BuildQueryString(parsed);
                }
                else
                {
                    doc["parsed"] = DescribeMap(ServerUtils.ParseXmlResponse(item.Raw));
                }
            }
            else if (item.Kind == "query-build")
            {
                doc["input"] = DescribeMap(item.Value);
                doc["built"] = ServerUtils.BuildQueryString(item.Value);
            }
            else if (item.Kind == "xml-build")
            {
                doc["input"] = DescribeMap(item.Value);
                string xml = ServerUtils.BuildXmlResponse(item.Value);
                doc["built"] = xml;
                doc["parsed"] = DescribeMap(ServerUtils.ParseXmlResponse(xml));
            }
            else
            {
                throw new InvalidOperationException("unknown kind " + item.Kind);
            }
        }
        catch (Exception exception)
        {
            doc["error"] = exception.GetType().Name;
        }
        return JsonSerializer.Serialize(doc, JsonOptions) + "\n";
    }

    static JsonObject DescribeMap(Dictionary<string, object> map)
    {
        JsonObject obj = [];
        foreach (KeyValuePair<string, object> pair in map)
            obj[pair.Key] = Describe(pair.Value);
        return obj;
    }

    static JsonObject Describe(object value)
    {
        switch (value)
        {
            case null:
                return new JsonObject { ["t"] = "null" };
            case string text:
                return new JsonObject { ["t"] = "str", ["v"] = text };
            case bool flag:
                return new JsonObject { ["t"] = "bool", ["v"] = flag };
            case int number:
                return new JsonObject { ["t"] = "int", ["v"] = number };
            case UUID uuid:
                return new JsonObject { ["t"] = "uuid", ["v"] = uuid.ToString() };
            case List<string> list:
                JsonArray items = [];
                foreach (string item in list)
                    items.Add(item);
                return new JsonObject { ["t"] = "list", ["v"] = items };
            case Dictionary<string, object> map:
                return new JsonObject { ["t"] = "map", ["v"] = DescribeMap(map) };
            default:
                throw new InvalidOperationException("unexpected " + value.GetType().FullName);
        }
    }

    static void Publish(string scratch, string directory)
    {
        if (Directory.Exists(directory))
            Directory.Delete(directory, true);
        Directory.Move(scratch, directory);
    }

    static void CompareTrees(string scratch, string directory)
    {
        foreach (string produced in Directory.GetFiles(scratch))
        {
            string name = Path.GetFileName(produced);
            string committed = Path.Combine(directory, name);
            if (!File.Exists(committed))
                throw new InvalidOperationException("missing committed file " + name);
            byte[] left = File.ReadAllBytes(produced);
            byte[] right = File.ReadAllBytes(committed);
            if (!left.AsSpan().SequenceEqual(right))
                throw new InvalidOperationException(name + " does not match ServerUtils");
        }
        foreach (string committed in Directory.GetFiles(directory))
        {
            string name = Path.GetFileName(committed);
            if (!File.Exists(Path.Combine(scratch, name)))
                throw new InvalidOperationException("unexpected committed file " + name);
        }
    }

    static UTF8Encoding Utf8() => new(encoderShouldEmitUTF8Identifier: false);
}
