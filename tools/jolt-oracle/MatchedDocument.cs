// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// JSON for one matched-native `drop`. The heartbeat stays 1/11 s. PhysicsStepRate is 45.
// The scene, the host and the sample text come from the P3.57 recorder.

using System.Globalization;
using System.Text;

namespace Ferroan.JoltConfigOracle;

static partial class RecordOracle
{
    const double MatchedPhysicsRateHz = 45;
    const string DropJson = "drop.json";

    internal static void GenerateMatched(string directory, string revision, string reference)
    {
        string json = MatchedJson(revision, reference);
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        try
        {
            File.WriteAllText(Path.Combine(scratch, DropJson), json);
            if (Directory.Exists(directory))
                Directory.Delete(directory, true);
            Directory.Move(scratch, directory);
        }
        catch
        {
            if (Directory.Exists(scratch))
                Directory.Delete(scratch, true);
            throw;
        }
        Console.WriteLine("generate: " + ScenarioName + " matched-native JSON on " + Baseline);
    }

    internal static void VerifyMatched(string directory, string revision, string reference)
    {
        string json = MatchedJson(revision, reference);
        string committed = Path.Combine(directory, DropJson);
        if (!File.Exists(committed))
            throw new InvalidOperationException("missing " + DropJson);
        string existing = File.ReadAllText(committed);
        if (existing != json)
            throw new InvalidOperationException(DropJson + " changed");
        string wrongClock = MutateFirstNumber(json, "\"input_ticks\": ", 1);
        if (wrongClock == json)
            throw new InvalidOperationException("wrong-clock negative did not change the clock");
        if (wrongClock == existing)
            throw new InvalidOperationException("wrong clock was accepted as " + DropJson);
        string wrongCollider = MutateFirstNumber(json, "collider=", 1);
        if (wrongCollider == json)
            throw new InvalidOperationException("wrong-collider negative did not change a collider");
        if (wrongCollider == existing)
            throw new InvalidOperationException("wrong collider was accepted as " + DropJson);
        Console.WriteLine("verify: " + ScenarioName + " matches Harness.Run at physics rate 45");
    }

    static string MatchedJson(string revision, string reference)
    {
        Captured captured = Capture(revision, MatchedPhysicsRateHz);
        string first = Document(captured, reference);
        string second = Document(captured, reference);
        if (first != second)
            throw new InvalidOperationException("matched JSON differed between two encodes");
        return first;
    }

    static string Document(Captured captured, string reference)
    {
        if (captured.RateHz != 11.0)
            throw new InvalidOperationException("drop heartbeat was not 11 Hz");
        if (captured.PhysicsRateHz != MatchedPhysicsRateHz)
            throw new InvalidOperationException(
                "drop physics rate was " + captured.PhysicsRateHz.ToString("R", CultureInfo.InvariantCulture)
                + ", not " + MatchedPhysicsRateHz.ToString("R", CultureInfo.InvariantCulture));
        if (!captured.Substepping)
            throw new InvalidOperationException("PhysicsStepRate 45 did not substep");
        if (string.IsNullOrEmpty(captured.MesherType))
            throw new InvalidOperationException("drop named no mesher");
        if (captured.Steps.Count != captured.Samples.Count || captured.InputTicks.Count != captured.Samples.Count)
            throw new InvalidOperationException("steps, clock and samples differ");
        string mode = DeterministicMode(captured.Config);
        RejectMachinePaths(captured.MesherType, reference);

        Dictionary<string, string> files = Encode(captured);
        foreach (KeyValuePair<string, string> pair in files)
            RejectMachinePaths(pair.Value, reference);

        long stepTotal = 0;
        foreach (long count in captured.Steps)
            stepTotal += count;

        var sb = new StringBuilder();
        sb.Append("{\n");
        sb.Append("  \"format\": 1,\n");
        Field(sb, "pin", captured.Revision);
        Field(sb, "baseline", Baseline);
        Field(sb, "scenario", ScenarioName);
        Field(sb, "heartbeat", "1/11");
        sb.Append("  \"rate_hz\": ");
        sb.Append(captured.RateHz.ToString("R", CultureInfo.InvariantCulture));
        sb.Append(",\n");
        sb.Append("  \"physics_rate_hz\": ");
        sb.Append(captured.PhysicsRateHz.ToString("R", CultureInfo.InvariantCulture));
        sb.Append(",\n");
        sb.Append("  \"physics_step_rate_set\": ");
        sb.Append(MatchedPhysicsRateHz.ToString("R", CultureInfo.InvariantCulture));
        sb.Append(",\n");
        sb.Append("  \"substepping\": ");
        sb.Append(captured.Substepping ? "true" : "false");
        sb.Append(",\n");
        Field(sb, "step_source", "substeps");
        sb.Append("  \"step_total\": ");
        sb.Append(stepTotal.ToString(CultureInfo.InvariantCulture));
        sb.Append(",\n");
        sb.Append("  \"steps\": [");
        for (int i = 0; i < captured.Steps.Count; i++)
        {
            if (i > 0)
                sb.Append(", ");
            sb.Append(captured.Steps[i].ToString(CultureInfo.InvariantCulture));
        }
        sb.Append("],\n");
        Field(sb, "meshing", captured.MesherType);
        sb.Append("  \"deterministic_mode\": ");
        sb.Append(mode);
        sb.Append(",\n");
        Field(sb, "native_file", captured.Native.File);
        sb.Append("  \"native_size\": ");
        sb.Append(captured.Native.Bytes.ToString(CultureInfo.InvariantCulture));
        sb.Append(",\n");
        Field(sb, "native_sha256", captured.Native.Sha256);
        Field(sb, "native_rid", captured.Native.Rid);
        Field(sb, "native_origin", captured.Native.Origin);
        sb.Append("  \"samples\": ");
        sb.Append(captured.Samples.Count.ToString(CultureInfo.InvariantCulture));
        sb.Append(",\n");
        sb.Append("  \"clock\": [\n");
        for (int i = 0; i < captured.InputTicks.Count; i++)
        {
            sb.Append("    {\"i\": ");
            sb.Append(i.ToString(CultureInfo.InvariantCulture));
            sb.Append(", \"input_ticks\": ");
            sb.Append(captured.InputTicks[i].ToString(CultureInfo.InvariantCulture));
            sb.Append(", \"step_ticks\": ");
            if (captured.StepTicks[i] is long ticks)
                sb.Append(ticks.ToString(CultureInfo.InvariantCulture));
            else
                sb.Append("null");
            sb.Append(i + 1 == captured.InputTicks.Count ? "}\n" : "},\n");
        }
        sb.Append("  ],\n");
        sb.Append("  \"channels\": {\n");
        string[] names =
        {
            "samples.txt", "events.txt", "providers.txt", "clock.txt", "config.txt",
            "materials.txt", "assets.txt", "cache.txt", "provenance.txt",
            "negative-clock.txt", "negative-collider.txt",
        };
        for (int i = 0; i < names.Length; i++)
        {
            string name = names[i];
            sb.Append("    \"");
            sb.Append(name);
            sb.Append("\": ");
            JsonString(sb, files[name]);
            sb.Append(i + 1 == names.Length ? "\n" : ",\n");
        }
        sb.Append("  }\n");
        sb.Append("}\n");
        string json = sb.ToString();
        RejectMachinePaths(json, reference);
        return json;
    }

    static string DeterministicMode(string config)
    {
        const string key = "DeterministicMode ";
        foreach (string line in config.Split('\n'))
        {
            if (!line.StartsWith(key, StringComparison.Ordinal))
                continue;
            string mode = line[key.Length..];
            if (mode == "true" || mode == "false")
                return mode;
            throw new InvalidOperationException("DeterministicMode was " + mode);
        }
        throw new InvalidOperationException("config names no DeterministicMode");
    }

    static void Field(StringBuilder sb, string name, string value)
    {
        sb.Append("  \"");
        sb.Append(name);
        sb.Append("\": ");
        JsonString(sb, value);
        sb.Append(",\n");
    }

    static void JsonString(StringBuilder sb, string value)
    {
        sb.Append('"');
        foreach (char c in value)
        {
            switch (c)
            {
                case '\\':
                    sb.Append("\\\\");
                    break;
                case '"':
                    sb.Append("\\\"");
                    break;
                case '\n':
                    sb.Append("\\n");
                    break;
                case '\r':
                    sb.Append("\\r");
                    break;
                case '\t':
                    sb.Append("\\t");
                    break;
                default:
                    if (c < 0x20)
                    {
                        sb.Append("\\u");
                        sb.Append(((int)c).ToString("X4", CultureInfo.InvariantCulture));
                    }
                    else
                        sb.Append(c);
                    break;
            }
        }
        sb.Append('"');
    }
}
