// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Ferroan.ReferenceOracle;

/// <summary>
/// Generates and checks fixtures encoded by the pinned libopenmetaverse checkout.
/// </summary>
internal static class Program
{
    const int FormatVersion = 1;
    const int MaxManifest = 32 * 1024 * 1024;

    static readonly JsonSerializerOptions Indented = new()
    {
        WriteIndented = true,
        Encoder = JavaScriptEncoder.Default,
    };

    static readonly JsonSerializerOptions Compact = new()
    {
        Encoder = JavaScriptEncoder.Default,
    };

    static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false);

    static int Main(string[] args)
    {
        try
        {
            Codec.UseOpenMetaverseCulture();
            if (!BitConverter.IsLittleEndian)
            {
                Console.Error.WriteLine("reference-oracle: OpenMetaverse packed types are little-endian.");
                return 1;
            }

            if (!TryParse(args, out string mode, out string directory, out string revision, out bool semantic))
            {
                Console.Error.WriteLine("usage: ReferenceOracle generate|verify <directory> --revision <40 hex> [--semantic]");
                return 2;
            }

            if (mode == "generate")
                return Generate(directory, revision);
            if (mode == "verify")
                return Verify(directory, revision, semantic);

            Console.Error.WriteLine("reference-oracle: unknown mode " + mode);
            return 2;
        }
        catch (Exception ex) when (ex is InvalidOperationException or IOException or ArgumentException or JsonException or UnauthorizedAccessException)
        {
            Console.Error.WriteLine("reference-oracle: " + ex.Message);
            return 1;
        }
    }

    static bool TryParse(string[] args, out string mode, out string directory, out string revision, out bool semantic)
    {
        mode = null;
        directory = null;
        revision = null;
        semantic = false;
        for (int i = 0; i < args.Length; i++)
        {
            string arg = args[i];
            if (arg == "--semantic")
            {
                semantic = true;
                continue;
            }

            if (arg == "--revision")
            {
                if (i + 1 >= args.Length)
                    return false;
                revision = args[++i];
                continue;
            }

            if (arg.StartsWith('-'))
                return false;
            if (mode == null)
                mode = arg;
            else if (directory == null)
                directory = arg;
            else
                return false;
        }

        if (mode != "generate" && mode != "verify")
            return false;
        if (directory == null || revision == null)
            return false;
        if (semantic && mode != "verify")
            return false;
        if (revision.Length != 40 || revision.Any(c => !"0123456789abcdef".Contains(c)))
            throw new InvalidOperationException("--revision must be 40 lowercase hex characters.");
        return true;
    }

    static int Generate(string directory, string revision)
    {
        List<Fixture> fixtures = Cases.Build();
        List<Fixture> again = Cases.Build();
        if (fixtures.Count != again.Count)
            throw new InvalidOperationException("fixture count changed between two builds.");
        for (int i = 0; i < fixtures.Count; i++)
        {
            if (fixtures[i].Id != again[i].Id || !fixtures[i].Payload.AsSpan().SequenceEqual(again[i].Payload))
                throw new InvalidOperationException("fixture bytes changed between two builds: " + fixtures[i].Id);
        }

        string root = Path.GetFullPath(directory);
        EnsureSafeOutput(root);
        Directory.CreateDirectory(root);

        var written = new HashSet<string>(StringComparer.Ordinal);
        var cases = new JsonArray();
        int decodeErrors = 0;
        int notRoundTrip = 0;
        foreach (Fixture fixture in fixtures)
        {
            string relative = fixture.File.Replace('\\', '/');
            string path = SafeChild(root, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            File.WriteAllBytes(path, fixture.Payload);
            written.Add(relative);

            var entry = new JsonObject();
            entry["id"] = fixture.Id;
            entry["family"] = fixture.Family;
            entry["format"] = fixture.Format;
            entry["file"] = relative;
            if (fixture.Params is not null)
                entry["params"] = fixture.Params;
            entry["round_trip"] = fixture.RoundTrip;
            entry["decoded"] = fixture.Decoded;
            cases.Add(entry);

            if (Codec.IsError(fixture.Decoded))
                decodeErrors++;
            if (!fixture.RoundTrip)
                notRoundTrip++;
        }

        var manifest = new JsonObject();
        manifest["format"] = FormatVersion;
        manifest["libopenmetaverse"] = revision;
        manifest["cases"] = cases;
        File.WriteAllText(Path.Combine(root, "manifest.json"), manifest.ToJsonString(Indented) + "\n", Utf8);

        foreach (string stale in Directory.EnumerateFiles(root, "*.bin", SearchOption.AllDirectories))
        {
            string relative = Path.GetRelativePath(root, stale).Replace('\\', '/');
            if (!written.Contains(relative))
                File.Delete(stale);
        }

        Console.WriteLine(
            "generate: " + fixtures.Count + " fixtures, " + notRoundTrip + " do not round-trip, "
            + decodeErrors + " decode as errors (" + root + ").");
        foreach (Fixture fixture in fixtures)
        {
            if (Codec.IsError(fixture.Decoded))
                Console.WriteLine("  decode-error " + fixture.Id);
            else if (!fixture.RoundTrip)
                Console.WriteLine("  no-round-trip " + fixture.Id);
        }

        return 0;
    }

    static int Verify(string directory, string revision, bool semantic)
    {
        string root = Path.GetFullPath(directory);
        string manifestPath = Path.Combine(root, "manifest.json");
        var info = new FileInfo(manifestPath);
        if (!info.Exists)
            throw new InvalidOperationException("missing " + manifestPath);
        if (info.Length > MaxManifest)
            throw new InvalidOperationException("manifest is larger than " + MaxManifest + " bytes.");

        JsonNode rootNode = JsonNode.Parse(File.ReadAllText(manifestPath, Utf8));
        if (rootNode is not JsonObject manifest)
            throw new InvalidOperationException("manifest.json is not an object.");
        if (manifest["format"]?.GetValue<int>() != FormatVersion)
            throw new InvalidOperationException("manifest format is not " + FormatVersion + ".");
        if (manifest["libopenmetaverse"]?.GetValue<string>() != revision)
        {
            throw new InvalidOperationException(
                "manifest revision " + manifest["libopenmetaverse"] + " does not match " + revision + ".");
        }

        if (manifest["cases"] is not JsonArray cases)
            throw new InvalidOperationException("manifest cases is not an array.");

        List<Fixture> fresh = semantic ? null : Cases.Build();
        var freshById = fresh?.ToDictionary(item => item.Id, StringComparer.Ordinal);
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var problems = new List<string>();

        foreach (JsonNode item in cases)
        {
            if (item is not JsonObject entry)
            {
                problems.Add("a manifest case is not an object.");
                continue;
            }

            string id = entry["id"]?.GetValue<string>();
            string family = entry["family"]?.GetValue<string>();
            string format = entry["format"]?.GetValue<string>();
            string file = entry["file"]?.GetValue<string>();
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(family) || string.IsNullOrEmpty(format) || string.IsNullOrEmpty(file))
            {
                problems.Add("a manifest case is missing id, family, format or file.");
                continue;
            }

            if (!seen.Add(id))
                problems.Add(id + ": duplicate id.");

            string path = SafeChild(root, file);
            if (!File.Exists(path))
            {
                problems.Add(id + ": missing " + file);
                continue;
            }

            var payloadInfo = new FileInfo(path);
            if (payloadInfo.Length > Codec.MaxPayload)
            {
                problems.Add(id + ": payload is larger than " + Codec.MaxPayload + " bytes.");
                continue;
            }

            byte[] payload = File.ReadAllBytes(path);
            JsonObject parms = entry["params"] as JsonObject;
            JsonObject decoded = Codec.Decode(family, format, payload, parms);
            string actual = decoded.ToJsonString(Compact);
            string expected = entry["decoded"]?.ToJsonString(Compact);
            if (actual != expected)
            {
                problems.Add(id + ": field mismatch\n  manifest " + expected + "\n  decoded  " + actual);
            }

            bool roundTrip = entry["round_trip"]?.GetValue<bool>() ?? false;
            bool actualRoundTrip = Codec.RoundTrips(family, format, payload, parms);
            if (roundTrip != actualRoundTrip)
                problems.Add(id + ": round_trip is " + roundTrip + " but decoding and encoding again " + (actualRoundTrip ? "matches" : "differs") + ".");

            if (!semantic && freshById is not null)
            {
                if (!freshById.TryGetValue(id, out Fixture rebuilt))
                {
                    problems.Add(id + ": not produced by this oracle.");
                }
                else if (!rebuilt.Payload.AsSpan().SequenceEqual(payload))
                {
                    problems.Add(id + ": bytes differ from a fresh OpenMetaverse encode (file "
                        + payload.Length + " bytes, oracle " + rebuilt.Payload.Length + " bytes, first mismatch at "
                        + FirstMismatch(payload, rebuilt.Payload) + ").");
                }
            }
        }

        if (!semantic && fresh is not null)
        {
            foreach (Fixture fixture in fresh)
            {
                if (!seen.Contains(fixture.Id))
                    problems.Add(fixture.Id + ": missing from the manifest.");
            }
        }

        if (problems.Count > 0)
        {
            int shown = Math.Min(problems.Count, 40);
            for (int i = 0; i < shown; i++)
                Console.Error.WriteLine(problems[i]);
            if (problems.Count > shown)
                Console.Error.WriteLine("... " + (problems.Count - shown) + " more");
            Console.Error.WriteLine("verify: " + problems.Count + " problem(s) in " + cases.Count + " fixtures.");
            return 1;
        }

        Console.WriteLine("verify: " + cases.Count + " fixtures match OpenMetaverse" + (semantic ? " (fields only)" : "") + ".");
        return 0;
    }

    static void EnsureSafeOutput(string root)
    {
        if (!Directory.Exists(root))
            return;
        foreach (string path in Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories))
        {
            string name = Path.GetFileName(path);
            bool allowed = name.Equals("README.md", StringComparison.OrdinalIgnoreCase)
                || name.Equals(".gitattributes", StringComparison.OrdinalIgnoreCase)
                || name.Equals("manifest.json", StringComparison.OrdinalIgnoreCase)
                || name.EndsWith(".bin", StringComparison.OrdinalIgnoreCase);
            if (!allowed)
                throw new InvalidOperationException("refusing to generate into " + root + " because it contains " + path);
        }
    }

    static string SafeChild(string root, string relative)
    {
        if (relative.Length == 0 || Path.IsPathRooted(relative) || relative.Contains('\\') || relative.Split('/').Contains(".."))
            throw new InvalidOperationException("fixture path escapes the output directory: " + relative);
        string full = Path.GetFullPath(Path.Combine(root, relative));
        string prefix = root.EndsWith(Path.DirectorySeparatorChar) ? root : root + Path.DirectorySeparatorChar;
        if (!full.StartsWith(prefix, StringComparison.Ordinal))
            throw new InvalidOperationException("fixture path escapes the output directory: " + relative);
        return full;
    }

    static int FirstMismatch(byte[] left, byte[] right)
    {
        int length = Math.Min(left.Length, right.Length);
        for (int i = 0; i < length; i++)
        {
            if (left[i] != right[i])
                return i;
        }

        return length;
    }
}
