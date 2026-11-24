// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Checks fixtures/walk/basicphysics-steps.json against BasicScene.Simulate.
// Number tokens are parsed as float. The file is not rewritten.

using System.Globalization;
using System.Reflection;
using OpenMetaverse;
using OpenSim.Region.PhysicsModules.BasicPhysics;

namespace Ferroan.WalkOracle;

static class Program
{
    const float Dt = 0.0909f;
    const float SizeZ = 2.1f;
    const float Flat = 21f;

    static int Main(string[] args)
    {
        if (args.Length != 2 || args[0] != "verify")
        {
            Console.Error.WriteLine("usage: WalkOracle verify <basicphysics-steps.json>");
            return 2;
        }
        try
        {
            Verify(File.ReadAllText(args[1]));
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception.Message);
            return 1;
        }
        Console.WriteLine("walk oracle: match");
        return 0;
    }

    static void Verify(string json)
    {
        Same(Scalar(json, "dt"), Dt, "dt");
        Same(Scalar(json, "size_z"), SizeZ, "size_z");
        CheckSteps(json, "flat_walk", 256f, false);
        CheckSteps(json, "fall", 256f, false);
        CheckSteps(json, "fly_hold", 256f, false);
        CheckSteps(json, "fly_snap", 256f, false);
        CheckSteps(json, "border", 16f, false);
        CheckSteps(json, "outside", 16f, false);
        CheckSteps(json, "slope", 16f, true);
        CheckNoMap(json);
    }

    static void CheckSteps(string json, string key, float extent, bool slope)
    {
        List<float[]> rows = Rows(json, key);
        if (rows.Count < 2)
            throw new InvalidOperationException(key + " needs a start and a step");
        bool flying = Scalar(json, key + "_flying") != 0f;
        BasicScene scene = Scene(extent, slope ? SlopeMap() : FlatMap(extent));
        float[] start = rows[0];
        var actor = scene.AddAvatar(
            key,
            new Vector3(start[0], start[1], start[2]),
            new Vector3(start[3], start[4], start[5]),
            new Vector3(1f, 1f, SizeZ),
            flying);
        for (int index = 1; index < rows.Count; index++)
        {
            scene.Simulate(Dt);
            float[] sample = rows[index];
            if (sample.Length < 7)
                throw new InvalidOperationException(key + " row " + index + " is short");
            string where = key + " row " + index;
            Same(actor.Position.X, sample[0], where + " x");
            Same(actor.Position.Y, sample[1], where + " y");
            Same(actor.Position.Z, sample[2], where + " z");
            Same(actor.Velocity.X, sample[3], where + " vx");
            Same(actor.Velocity.Y, sample[4], where + " vy");
            Same(actor.Velocity.Z, sample[5], where + " vz");
            if (actor.IsColliding != (sample[6] != 0f))
                throw new InvalidOperationException(where + " colliding");
        }
    }

    static void CheckNoMap(string json)
    {
        List<float[]> rows = Rows(json, "no_map");
        if (rows.Count != 1 || rows[0].Length < 4)
            throw new InvalidOperationException("no_map row is short");
        BasicScene scene = Scene(256f, null);
        var actor = scene.AddAvatar(
            "no_map",
            new Vector3(4f, 5f, 26f),
            Vector3.Zero,
            new Vector3(1f, 1f, SizeZ),
            false);
        scene.Simulate(Dt);
        float[] sample = rows[0];
        Same(actor.Position.X, sample[0], "no_map x");
        Same(actor.Position.Y, sample[1], "no_map y");
        Same(actor.Position.Z, sample[2], "no_map z");
        Same(actor.Velocity.Z, sample[3], "no_map vz");
    }

    static BasicScene Scene(float extent, float[] terrain)
    {
        var scene = new BasicScene();
        FieldInfo field = typeof(BasicScene).GetField(
            "m_regionExtent",
            BindingFlags.Instance | BindingFlags.NonPublic);
        if (field == null)
            throw new InvalidOperationException("m_regionExtent is missing");
        field.SetValue(scene, new Vector3(extent, extent, 4096f));
        if (terrain != null)
            scene.SetTerrain(terrain);
        return scene;
    }

    static float[] FlatMap(float extent)
    {
        int side = (int)extent;
        var map = new float[side * side];
        for (int index = 0; index < map.Length; index++)
            map[index] = Flat;
        return map;
    }

    static float[] SlopeMap()
    {
        float[] map = FlatMap(16f);
        map[3 * 16 + 2] = 30f;
        return map;
    }

    static void Same(float got, float want, string where)
    {
        if (BitConverter.SingleToInt32Bits(got) != BitConverter.SingleToInt32Bits(want))
            throw new InvalidOperationException(where + ": " + got + " != " + want);
    }

    static float Scalar(string json, string key)
    {
        string marker = "\"" + key + "\"";
        int at = json.IndexOf(marker, StringComparison.Ordinal);
        if (at < 0)
            throw new InvalidOperationException("missing " + key);
        string rest = json.Substring(at + marker.Length).TrimStart();
        if (rest.StartsWith(':'))
            rest = rest.Substring(1).TrimStart();
        int end = rest.IndexOfAny(new[] { ',', '\n', '}', ' ' });
        if (end < 0)
            throw new InvalidOperationException("missing value for " + key);
        return Parse(rest.Substring(0, end).Trim(), key);
    }

    static List<float[]> Rows(string json, string key)
    {
        string marker = "\"" + key + "\"";
        int at = json.IndexOf(marker, StringComparison.Ordinal);
        if (at < 0)
            throw new InvalidOperationException("missing " + key);
        string rest = json.Substring(at + marker.Length);
        int open = rest.IndexOf('[');
        if (open < 0)
            throw new InvalidOperationException("missing array " + key);
        int depth = 0;
        int end = -1;
        for (int index = open; index < rest.Length; index++)
        {
            if (rest[index] == '[')
                depth++;
            else if (rest[index] == ']')
            {
                depth--;
                if (depth == 0)
                {
                    end = index + 1;
                    break;
                }
            }
        }
        if (end < 0)
            throw new InvalidOperationException("unclosed array " + key);
        string body = rest.Substring(open, end - open);
        var rows = new List<float[]>();
        var row = new List<float>();
        var token = new System.Text.StringBuilder();
        depth = 0;
        foreach (char ch in body)
        {
            if (ch == '[')
            {
                depth++;
                if (depth == 2)
                    row.Clear();
            }
            else if (ch == ']')
            {
                if (depth == 2)
                {
                    Push(token, row);
                    rows.Add(row.ToArray());
                    row = new List<float>();
                }
                depth--;
            }
            else if (ch == ',' && depth == 2)
                Push(token, row);
            else if (depth == 2 && (char.IsDigit(ch) || ch == '.' || ch == '-' || ch == '+' || ch == 'e' || ch == 'E'))
                token.Append(ch);
        }
        return rows;
    }

    static void Push(System.Text.StringBuilder token, List<float> row)
    {
        if (token.Length == 0)
            return;
        row.Add(Parse(token.ToString(), "row"));
        token.Clear();
    }

    static float Parse(string token, string where)
    {
        if (!float.TryParse(token, NumberStyles.Float, CultureInfo.InvariantCulture, out float value))
            throw new InvalidOperationException("bad " + where + " " + token);
        return value;
    }
}
