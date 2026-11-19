// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Ferroan.Pin3Oracle;

static class Program
{
    static int Main(string[] args)
    {
        if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision"
            || args[3].Length != 40 || !args[3].All(Uri.IsHexDigit))
        {
            Console.Error.WriteLine("usage: Pin3Oracle generate|verify <directory> --revision <40 hex>");
            return 2;
        }
        string directory = Path.GetFullPath(args[1]);
        Directory.SetCurrentDirectory(AppContext.BaseDirectory);
        TextWriter report = Console.Out;
        try
        {
            // Reference test helpers log scene setup and transient IDs. The oracle
            // emits only its deterministic, fixed-identity observations.
            Console.SetOut(TextWriter.Null);
            Dictionary<string, JsonObject> cases = new()
            {
                ["lifecycle.json"] = LifecycleCases.Capture(),
                ["trust.json"] = TrustCases.Capture()
            };
            Console.SetOut(report);
            Directory.CreateDirectory(directory);
            foreach ((string name, JsonObject value) in cases)
            {
                value["revision"] = args[3];
                string json = value.ToJsonString(new JsonSerializerOptions { WriteIndented = true }) + "\n";
                Publish(Path.Combine(directory, name), json, args[0]);
            }
            Publish(Path.Combine(directory, "manifest.txt"),
                "format 1\nrevision " + args[3] + "\nsource actual pinned Scene, LLClientView, ActivityDetector, GridUserServicesConnector and ControlPlaneAccess\n\nlifecycle.json\ntrust.json\n", args[0]);
            Console.WriteLine("pin3 oracle: " + args[0] + " passed (lifecycle, close, startup and trust observations)");
            return 0;
        }
        catch (Exception error)
        {
            Console.SetOut(report);
            // Artifacts are under .scratch, so diagnostics from the reference
            // helpers do not enter public output.
            File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "oracle-error.log"), error.ToString());
            Console.Error.WriteLine("pin3 oracle failed: " + error.GetType().Name + "; details in scratch oracle-error.log");
            return 1;
        }
        finally { Console.SetOut(report); }
    }

    static void Publish(string path, string text, string action)
    {
        if (action == "generate") File.WriteAllText(path, text, new UTF8Encoding(false));
        else if (!File.Exists(path) || File.ReadAllText(path) != text)
            throw new InvalidOperationException(Path.GetFileName(path) + " differs from the C# observation");
    }
}
