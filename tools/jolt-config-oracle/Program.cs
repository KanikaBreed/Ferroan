// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls the pinned Jolt module's config reader and writes one result per ini.
// JoltConfig.FromConfig, EffectivePhysicsStepRate, EffectivePresets and
// JoltScene.TestCommandsEnabled at OpenSim-Tranquillity c401bceecd.
// This program does not copy those methods and it does not load joltc.

using System.Globalization;
using System.Reflection;
using System.Text;
using Nini.Config;
using OpenSim.Region.PhysicsModules.Jolt;

namespace Ferroan.JoltConfigOracle;

static class Program
{
    const string FormatLine = "format 1";

    static int Main(string[] args)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        if (args.Length != 6
            || args[2] != "--revision" || args[4] != "--reference"
            || (args[0] != "generate" && args[0] != "verify"
                && args[0] != "file-generate" && args[0] != "file-verify"))
        {
            Console.Error.WriteLine("usage: JoltConfigOracle generate|verify|file-generate|file-verify <directory> --revision <40 hex> --reference <path>");
            return 2;
        }
        string directory = Path.GetFullPath(args[1]);
        string revision = args[3];
        string reference = Path.GetFullPath(args[5]);
        if (revision.Length != 40 || revision.Any(ch => !Uri.IsHexDigit(ch)))
        {
            Console.Error.WriteLine("revision must be 40 hex characters");
            return 1;
        }
        try
        {
            if (args[0] == "generate")
                Generate(directory, revision, reference);
            else if (args[0] == "verify")
                Verify(directory, revision, reference);
            else if (args[0] == "file-generate")
                FileOracle.Generate(directory, revision, reference);
            else
                FileOracle.Verify(directory, revision, reference);
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception);
            return 1;
        }
        return 0;
    }

    static void Generate(string directory, string revision, string reference)
    {
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision, reference);
        if (Directory.Exists(directory))
            Directory.Delete(directory, true);
        Directory.Move(scratch, directory);
        Console.WriteLine("generate: " + Cases().Length + " jolt config cases");
    }

    static void Verify(string directory, string revision, string reference)
    {
        string scratch = directory + ".verify";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision, reference);
        CompareTrees(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("verify: " + Cases().Length + " jolt config cases match FromConfig");
    }

    static void WriteTree(string directory, string revision, string reference)
    {
        var manifest = new StringBuilder();
        manifest.Append(FormatLine).Append('\n');
        manifest.Append("revision ").Append(revision).Append('\n');
        manifest.Append('\n');
        foreach (Case item in Cases())
        {
            manifest.Append(item.Id).Append('\n');
            string caseDir = Path.Combine(directory, item.Id);
            Directory.CreateDirectory(caseDir);
            File.WriteAllText(Path.Combine(caseDir, "kind.txt"), item.Kind + "\n");
            if (item.Kind == "parse")
                File.WriteAllText(Path.Combine(caseDir, "source.ini"), item.Source);
            else
                File.WriteAllText(Path.Combine(caseDir, "load.txt"), item.Source);
            string first = Execute(item, reference);
            string second = Execute(item, reference);
            if (first != second)
                throw new InvalidOperationException(item.Id + " differed between two reads");
            RejectMachinePaths(item.Id, first, reference);
            File.WriteAllText(Path.Combine(caseDir, "result.txt"), first);
        }
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), manifest.ToString());
    }

    static string Execute(Case item, string reference)
    {
        IConfigSource source = item.Kind == "parse"
            ? Parse(item.Source)
            : LoadShipped(reference);
        return Dump(source);
    }

    static IConfigSource Parse(string text)
    {
        string path = Path.Combine(Path.GetTempPath(), "ferroan-jolt-config-" + Guid.NewGuid().ToString("N") + ".ini");
        File.WriteAllText(path, text);
        try
        {
            return new IniConfigSource(path);
        }
        finally
        {
            File.Delete(path);
        }
    }

    static IConfigSource LoadShipped(string reference)
    {
        string appdata = Path.Combine(reference, "OpenSim-Tranquillity", "Source", "OpenSim.Server.RegionServer", "AppData");
        string defaults = Path.Combine(appdata, "RegionServerDefaults.ini");
        if (!File.Exists(defaults))
            throw new InvalidOperationException("missing " + defaults);
        var source = new IniConfigSource(defaults);
        string example = Path.Combine(appdata, "RegionServer.ini.example");
        if (File.Exists(example))
            source.Merge(new IniConfigSource(example));
        return source;
    }

    static string Dump(IConfigSource source)
    {
        Assembly assembly = typeof(JoltModule).Assembly;
        Type configType = assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltConfig")
            ?? throw new InvalidOperationException("JoltConfig was not in the Jolt assembly");
        Type sceneType = assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltScene")
            ?? throw new InvalidOperationException("JoltScene was not in the Jolt assembly");
        MethodInfo fromConfig = configType.GetMethod("FromConfig", BindingFlags.Static | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("FromConfig was not found");
        var warnings = new List<string>();
        object config = fromConfig.Invoke(null, new object[] { source, warnings })
            ?? throw new InvalidOperationException("FromConfig returned null");

        IConfig startup = source.Configs["Startup"];
        float heartbeat = startup == null ? 0.0908f : startup.GetFloat("FrameTime", 0.0908f);
        MethodInfo step = configType.GetMethod("EffectivePhysicsStepRate", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("EffectivePhysicsStepRate was not found");
        var stepArgs = new object[] { heartbeat, null, null };
        object rate = step.Invoke(config, stepArgs)
            ?? throw new InvalidOperationException("EffectivePhysicsStepRate returned null");
        string stepWarning = stepArgs[1] as string;
        string stepNote = stepArgs[2] as string;
        PropertyInfo presets = configType.GetProperty("EffectivePresets", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("EffectivePresets was not found");
        string presetName = presets.GetValue(config).ToString();

        MethodInfo testCommands = sceneType.GetMethod("TestCommandsEnabled", BindingFlags.Static | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("TestCommandsEnabled was not found");
        string commands;
        try
        {
            bool enabled = (bool)testCommands.Invoke(null, new object[] { source });
            commands = enabled ? "true" : "false";
        }
        catch (TargetInvocationException)
        {
            commands = "threw";
        }

        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("heartbeat\t").Append(Bits(heartbeat)).Append('\n');
        text.Append("test_commands\t").Append(commands).Append('\n');
        text.Append("effective\t").Append(Bits((float)rate)).Append('\n');
        text.Append("effective_presets\t").Append(presetName).Append('\n');
        text.Append("step_warning\t").Append(stepWarning ?? "").Append('\n');
        text.Append("step_note\t").Append(stepNote ?? "").Append('\n');
        foreach (string warning in warnings)
            text.Append("warning\t").Append(warning).Append('\n');
        FieldInfo[] fields = configType.GetFields(BindingFlags.Instance | BindingFlags.Public);
        Array.Sort(fields, (left, right) => string.CompareOrdinal(left.Name, right.Name));
        foreach (FieldInfo field in fields)
            text.Append("field\t").Append(field.Name).Append('\t').Append(Payload(field.FieldType, field.GetValue(config))).Append('\n');
        return text.ToString();
    }

    static string Payload(Type type, object value)
    {
        Type underlying = Nullable.GetUnderlyingType(type);
        if (underlying == typeof(float))
            return value == null ? "f32?\tnull" : "f32?\t" + F32((float)value);
        if (underlying == typeof(bool))
            return value == null ? "bool?\tnull" : "bool?\t" + ((bool)value ? "true" : "false");
        if (type == typeof(float))
            return "f32\t" + F32((float)value);
        if (type == typeof(int))
            return "i32\t" + ((int)value).ToString(CultureInfo.InvariantCulture);
        if (type == typeof(bool))
            return "bool\t" + ((bool)value ? "true" : "false");
        return "enum\t" + value;
    }

    static string F32(float value)
    {
        return Bits(value) + "\t" + value.ToString(CultureInfo.InvariantCulture);
    }

    static string Bits(float value)
    {
        return BitConverter.SingleToInt32Bits(value).ToString("X8", CultureInfo.InvariantCulture);
    }

    static void RejectMachinePaths(string id, string text, string reference)
    {
        if (text.Contains(reference, StringComparison.Ordinal) || text.Contains("/home/", StringComparison.Ordinal)
            || text.Contains("\\Users\\", StringComparison.Ordinal))
            throw new InvalidOperationException(id + " contains a machine path");
    }

    static void CompareTrees(string scratch, string directory)
    {
        var expected = new HashSet<string>(Directory.EnumerateFiles(directory, "*", SearchOption.AllDirectories)
            .Select(path => Path.GetRelativePath(directory, path)));
        var actual = new HashSet<string>(Directory.EnumerateFiles(scratch, "*", SearchOption.AllDirectories)
            .Select(path => Path.GetRelativePath(scratch, path)));
        if (!expected.SetEquals(actual))
            throw new InvalidOperationException("fixture files differ");
        foreach (string relative in expected.OrderBy(path => path, StringComparer.Ordinal))
        {
            string left = File.ReadAllText(Path.Combine(scratch, relative));
            string right = File.ReadAllText(Path.Combine(directory, relative));
            if (left != right)
                throw new InvalidOperationException(relative + " changed");
        }
    }

    static Case[] Cases()
    {
        return new[]
        {
            Parse("absent-section", "Jolt", "0.0909", null),
            Parse("absent-keys", "Jolt", "0.0909", ""),
            Parse("frame-absent", "Jolt", null, null),
            Parse("no-startup", null, null, null),
            Parse("commented-keys", "Jolt", "0.0909",
                "; PhysicsStepRate = 1\n; VehiclePresets = legacy\nGravity = -9.80665\n"),
            Parse("wrong-case", "Jolt", "0.0909", "gravity = 1\nphysicssteprate = 10\n"),
            Parse("whitespace-values", "Jolt", "0.0909",
                "Gravity =  -9.8\nCollisionSteps =  6\nDeterministicMode =  TRUE\nVehicleBehaviour =  secondlife\n"),
            Parse("internal-space", "Jolt", "0.0909", "Gravity = no pe\n"),
            Parse("empty-values", "Jolt", "0.0909", "Gravity =\nCollisionSteps =\nDeterministicMode =\nVehicleBehaviour =\n"),
            Parse("nan-infinity", "Jolt", "0.0909",
                "Gravity = NaN\nBodyMaxLinearSpeed = Infinity\nBodyMaxAngularSpeed = -Infinity\n"),
            Parse("exponent", "Jolt", "0.0909", "Gravity = 1e1\nCollisionSteps = +8\n"),
            Parse("clamp-low", "Jolt", "0.0909", Numeric("low") + Tokens(false) + "VehicleSpikeMitigation = false\nVehicleContactFriction = 0\n"),
            Parse("clamp-high", "Jolt", "0.0909", Numeric("high") + Tokens(true) + "VehicleSpikeMitigation = true\nVehicleContactFriction = 10\n"),
            Parse("out-low", "Jolt", "0.0909", Numeric("below")),
            Parse("out-high", "Jolt", "0.0909", Numeric("above")),
            Parse("hover-below", "Jolt", "0.0909", "VehicleMinHoverHeight = -100001\n"),
            Parse("hover-above", "Jolt", "0.0909", "VehicleMaxHoverHeight = 100001\n"),
            Parse("hover-inverted", "Jolt", "0.0909", "VehicleMinHoverHeight = 10\nVehicleMaxHoverHeight = 0\n"),
            Parse("hover-equal", "Jolt", "0.0909", "VehicleMinHoverHeight = 5\nVehicleMaxHoverHeight = 5\n"),
            Parse("bad-tokens", "Jolt", "0.0909",
                "DeterministicMode = maybe\nScaleCapsWithArea = 1\nVehicleBehaviour = ode\nVehiclePresets = 3\n"
                + "BasicForcesBehaviour = \nFrictionCombine = 1\nRestitutionCombine = -Max\n"
                + "VehicleSpikeMitigation = yes\nVehicleContactFriction = eleven\nTestCommands = maybe\n"),
            Parse("contact-empty", "Jolt", "0.0909", "VehicleContactFriction =\n"),
            Parse("contact-spaces", "Jolt", "0.0909", "VehicleContactFriction =    \n"),
            Parse("contact-set", "Jolt", "0.0909", "VehicleContactFriction = 0.6\n"),
            Parse("step-zero", "Jolt", "0.0909", "PhysicsStepRate = 0\n"),
            Parse("step-eleven", "Jolt", "0.0909", "PhysicsStepRate = 11\n"),
            Parse("step-ten", "Jolt", "0.0909", "PhysicsStepRate = 10\n"),
            Parse("step-invalid", "Jolt", "0.0909", "PhysicsStepRate = nope\n"),
            Parse("step-unset-fast", "Jolt", "0.01", ""),
            Parse("presets-documented", "Jolt", "0.0909", "VehiclePresets = documented\nVehicleBehaviour = InWorldz\n"),
            Parse("behaviour-secondlife", "Jolt", "0.0909", "VehicleBehaviour = SecondLife\n"),
            Parse("decay-set", "Jolt", "0.0909", "VehicleMaxDecayTimescale = 50\n"),
            Parse("physics-ubode", "ubODE", "0.0909", ""),
            Parse("physics-bulletsim", "BulletSim", "0.0909", ""),
            Parse("physics-other", "basicphysics", "0.0909", ""),
            Parse("physics-empty", "", "0.0909", ""),
            Parse("physics-jolt", "Jolt", "0.0909", ""),
            Parse("physics-lower", "jolt", "0.0909", ""),
            Parse("enum-plus-one", "Jolt", "0.0909",
                "FrictionCombine = +1\nRestitutionCombine = +1\nVehicleBehaviour = +1\n"
                + "VehiclePresets = +1\nBasicForcesBehaviour = +1\n"),
            Parse("enum-plus-zero", "Jolt", "0.0909",
                "FrictionCombine = +0\nRestitutionCombine = +0\nVehicleBehaviour = +0\n"
                + "VehiclePresets = +0\nBasicForcesBehaviour = +0\n"),
            Parse("enum-or-names", "Jolt", "0.0909",
                "FrictionCombine = Average, Min\nRestitutionCombine = Average, Min\n"
                + "VehicleBehaviour = SecondLife,InWorldz\nVehiclePresets = Documented, Legacy\n"
                + "BasicForcesBehaviour = InWorldz, SecondLife\n"),
            Parse("enum-undefined", "Jolt", "0.0909",
                "FrictionCombine = Min,Max\nRestitutionCombine = Min,Max\nVehicleBehaviour = +2\n"
                + "VehiclePresets = +2\nBasicForcesBehaviour = +2\n"),
            Parse("step-tiny", "Jolt", "0.0909", "PhysicsStepRate = 0.00001\n"),
            Parse("frame-0.005025", "Jolt", "0.005025", ""),
            new Case("shipped", "region-ref",
                "reference_appdata OpenSim.Server.RegionServer/AppData\nini_master RegionServerDefaults.ini\nini_file RegionServer.ini.example\n"),
        };
    }

    static Case Parse(string id, string physics, string frame, string jolt)
    {
        var text = new StringBuilder();
        if (physics != null || frame != null)
        {
            text.Append("[Startup]\n");
            if (physics != null)
                text.Append("physics = ").Append(physics).Append('\n');
            if (frame != null)
                text.Append("FrameTime = ").Append(frame).Append('\n');
        }
        if (jolt != null)
        {
            text.Append("[Jolt]\n");
            text.Append(jolt);
        }
        return new Case(id, "parse", text.ToString());
    }

    static string Numeric(string which)
    {
        var text = new StringBuilder();
        foreach (string[] row in Numbers)
        {
            string value = which switch
            {
                "low" => row[1],
                "high" => row[2],
                "below" => row[3],
                "above" => row[4],
                _ => throw new InvalidOperationException(which),
            };
            text.Append(row[0]).Append(" = ").Append(value).Append('\n');
        }
        return text.ToString();
    }

    static string Tokens(bool high)
    {
        string boolean = high ? "true" : "false";
        string behaviour = high ? "InWorldz" : "SecondLife";
        string presets = high ? "Legacy" : "Documented";
        string forces = high ? "SecondLife" : "InWorldz";
        string combine = high ? "Max" : "Min";
        return "DeterministicMode = " + boolean + "\n"
            + "ScaleCapsWithArea = " + boolean + "\n"
            + "AllowUnrecordedNative = " + boolean + "\n"
            + "HonourConvexMesh = " + boolean + "\n"
            + "BodyEnhancedEdgeRemoval = " + boolean + "\n"
            + "AvatarEnhancedEdgeRemoval = " + boolean + "\n"
            + "TwoSidedMeshCollision = " + boolean + "\n"
            + "KeyframedMotionCarriesRiders = " + boolean + "\n"
            + "MovedPrimsCarryRiders = " + boolean + "\n"
            + "PhysicsSit = " + boolean + "\n"
            + "JobPoolFairHandoff = " + boolean + "\n"
            + "VehicleMotorTargetOnUndrivenAxes = " + boolean + "\n"
            + "TestCommands = " + boolean + "\n"
            + "VehicleBehaviour = " + behaviour + "\n"
            + "VehiclePresets = " + presets + "\n"
            + "BasicForcesBehaviour = " + forces + "\n"
            + "FrictionCombine = " + combine + "\n"
            + "RestitutionCombine = " + combine + "\n";
    }

    // key, inclusive low, inclusive high, one step outside below, one step outside above.
    // Hover heights are separate: setting both outside the range also trips the pair reset.
    static readonly string[][] Numbers =
    {
        new[] { "Gravity", "-1000", "1000", "-1001", "1001" },
        new[] { "CollisionSteps", "1", "64", "0", "65" },
        new[] { "PositionIterations", "1", "255", "0", "256" },
        new[] { "VelocityIterations", "1", "255", "0", "256" },
        new[] { "ThreadCount", "0", "256", "-1", "257" },
        new[] { "MaxBodies", "0", "8388607", "-1", "8388608" },
        new[] { "MaxBodyPairs", "1", "8388607", "0", "8388608" },
        new[] { "MaxContactConstraints", "1", "8388607", "0", "8388608" },
        new[] { "SpeculativeContactDistance", "0", "0.5", "-0.1", "0.6" },
        new[] { "SleepVelocityThreshold", "0.001", "1", "0", "1.1" },
        new[] { "BodyUpdateBufferMax", "1", "8388607", "0", "8388608" },
        new[] { "CharacterUpdateBufferMax", "1", "65536", "0", "65537" },
        new[] { "ContactBufferMax", "0", "16777216", "-1", "16777217" },
        new[] { "AvatarJumpSpeed", "0", "100", "-1", "101" },
        new[] { "AvatarWalkSpeedFactor", "0.1", "10", "0.09", "10.1" },
        new[] { "AvatarRunSpeedFactor", "0.1", "10", "0.09", "10.1" },
        new[] { "AvatarFlySpeedFactor", "0.1", "10", "0.09", "10.1" },
        new[] { "AvatarFlyDownSpeedFactor", "0.1", "10", "0.09", "10.1" },
        new[] { "CapacityLogIntervalSeconds", "0.1", "86400", "0.09", "86401" },
        new[] { "MetricsLogIntervalSeconds", "0", "86400", "-1", "86401" },
        new[] { "JobPools", "1", "64", "0", "65" },
        new[] { "PhysicsStepRate", "0", "1000", "-1", "1001" },
        new[] { "PhysicsStepCollisionSteps", "1", "64", "0", "65" },
        new[] { "VehicleGroundGravityFactor", "0", "1", "-0.1", "1.1" },
        new[] { "VehicleMaxLinearSpeed", "1", "10000", "0.5", "10001" },
        new[] { "VehicleReferenceSpeed", "0.1", "10000", "0.09", "10001" },
        new[] { "VehicleMaxAngularSpeed", "0.1", "1000", "0.09", "1001" },
        new[] { "VehicleAngularLimiterFloor", "0", "1", "-0.1", "1.1" },
        new[] { "VehicleMinTimescale", "0.001", "1", "0", "1.1" },
        new[] { "VehicleMaxTimescale", "1", "1000", "0.5", "1001" },
        new[] { "VehicleMaxDecayTimescale", "1", "1000", "0", "1001" },
        new[] { "VehicleMaxHoverTimescale", "1", "1000", "0", "1001" },
        new[] { "VehicleMaxAttractTimescale", "1", "1000", "0", "1001" },
        new[] { "VehicleMaxMotorOffset", "0", "10000", "-1", "10001" },
        new[] { "VehicleSledAssist", "0", "10", "-0.1", "10.1" },
        new[] { "SledContactFriction", "0", "10", "-1", "11" },
        new[] { "TerrainFriction", "0", "255", "-1", "256" },
        new[] { "TerrainRestitution", "0", "1", "-0.1", "1.1" },
        new[] { "VehicleRestSpeed", "0", "1", "-0.1", "1.1" },
        new[] { "BodyMaxLinearSpeed", "1", "100000", "0.5", "100001" },
        new[] { "BodyMaxAngularSpeed", "0.1", "10000", "0.09", "10001" },
        new[] { "MaxPhysicalComplexity", "0", "1000000", "-1", "1000001" },
        new[] { "PrimLinearDamping", "0", "100", "-1", "101" },
        new[] { "PrimAngularDamping", "0", "100", "-1", "101" },
        new[] { "RayCastBudgetMs", "0.1", "1000", "0.09", "1001" },
        new[] { "RayCastSimulatorBudgetMs", "0.1", "1000", "0.09", "1001" },
        new[] { "RayCastMaxTestedHits", "1", "1000000", "0", "1000001" },
        new[] { "RayCastMaxHits", "1", "256", "0", "257" },
        new[] { "CameraRayBudgetMs", "0", "1000", "-1", "1001" },
        new[] { "AvatarPushMaxSpeed", "0", "1000", "-1", "1001" },
        new[] { "AvatarPushRecovery", "0", "1000", "-1", "1001" },
        new[] { "AvatarObjectPushPerMomentum", "0", "100000", "-1", "100001" },
        new[] { "AvatarObjectPushMaxForce", "0", "1000000", "-1", "1000001" },
    };

    sealed record Case(string Id, string Kind, string Source);
}
