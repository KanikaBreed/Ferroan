// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls pinned JoltScene.NoteDilationSample (JoltScene.cs 2928-2942) and records
// the private dilation window. The same capture compiles JoltCharacter's capsule,
// mass, latch, intent and marker descriptor, then Simulate's return of 1 and
// SimStatsReporter's FPS transform, then calls CookPrimShape
// (JoltScene.cs 1229-1259) and records the analytic constructor it chooses,
// then calls public SetTerrain (JoltScene.cs 3552-3613) through a recording
// backend and stores the padded field that method keeps, then calls
// JoltPrim.Buoyancy, VehicleType and HalcyonBasicForces.LegacyBuoyancy and
// records the gravity factor, then calls DispatchContacts and records the
// actor sets, before the final native proof. The physics scene is
// constructed and never initialised, so
// JoltNative stays unloaded and Foundation.Init is not called.

using System.Diagnostics;
using System.Globalization;
using System.Numerics;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text;
using OpenSim.Framework;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Region.PhysicsModules.Jolt;
using OpenSim.Region.PhysicsModules.Jolt.Backend;

namespace Ferroan.JoltConfigOracle;

static class FileOracle
{
    const string FormatLine = "format 1";
    const string CaseId = "note-dilation";

    internal static void Generate(string directory, string revision, string reference)
    {
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision, reference);
        if (Directory.Exists(directory))
            Directory.Delete(directory, true);
        Directory.Move(scratch, directory);
        Console.WriteLine("file-generate: " + CaseId);
    }

    internal static void Verify(string directory, string revision, string reference)
    {
        string scratch = directory + ".verify";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        try
        {
            WriteTree(scratch, revision, reference);
            CompareTrees(scratch, directory);
        }
        finally
        {
            if (Directory.Exists(scratch))
                Directory.Delete(scratch, true);
        }
        Console.WriteLine("file-verify: " + CaseId + " matches NoteDilationSample");
    }

    static void WriteTree(string directory, string revision, string reference)
    {
        RequirePortableClock();
        Captured first = Capture(directory);
        Captured second = Capture(directory);
        if (first.Calls != second.Calls || first.Result != second.Result || first.Native != second.Native
            || first.CharacterCalls != second.CharacterCalls || first.CharacterResult != second.CharacterResult
            || first.Stats != second.Stats
            || first.Analytic != second.Analytic
            || first.Heightfield != second.Heightfield
            || first.Gravity != second.Gravity
            || first.Collision != second.Collision)
            throw new InvalidOperationException(CaseId + " differed between two reads");
        RejectMachinePaths(first.Calls, reference);
        RejectMachinePaths(first.Result, reference);
        RejectMachinePaths(first.Native, reference);
        RejectMachinePaths(first.CharacterCalls, reference);
        RejectMachinePaths(first.CharacterResult, reference);
        RejectMachinePaths(first.Stats, reference);
        RejectMachinePaths(first.Analytic, reference);
        RejectMachinePaths(first.Heightfield, reference);
        RejectMachinePaths(first.Gravity, reference);
        RejectMachinePaths(first.Collision, reference);

        var manifest = new StringBuilder();
        manifest.Append(FormatLine).Append('\n');
        manifest.Append("revision ").Append(revision).Append('\n');
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs\n");
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/JoltCharacter.cs\n");
        manifest.Append("compile OpenSim.Region.Framework/Scenes/SimStatsReporter.cs\n");
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/CollisionFrameTracker.cs\n");
        manifest.Append("compile OpenSim.Region.Framework/Scenes/SceneObjectPart.cs\n");
        manifest.Append("compile OpenSim.Region.Framework/Scenes/CollisionSounds.cs\n");
        manifest.Append('\n');
        manifest.Append(CaseId).Append('\n');
        manifest.Append("character\n");
        manifest.Append("stats\n");
        manifest.Append("analytic\n");
        manifest.Append("heightfield\n");
        manifest.Append("gravity\n");
        manifest.Append("collision\n");

        string caseDir = Path.Combine(directory, CaseId);
        Directory.CreateDirectory(caseDir);
        File.WriteAllText(Path.Combine(caseDir, "calls.txt"), first.Calls);
        File.WriteAllText(Path.Combine(caseDir, "result.txt"), first.Result);
        File.WriteAllText(Path.Combine(caseDir, "native.txt"), first.Native);
        string characterDir = Path.Combine(directory, "character");
        Directory.CreateDirectory(characterDir);
        File.WriteAllText(Path.Combine(characterDir, "calls.txt"), first.CharacterCalls);
        File.WriteAllText(Path.Combine(characterDir, "result.txt"), first.CharacterResult);
        string statsDir = Path.Combine(directory, "stats");
        Directory.CreateDirectory(statsDir);
        File.WriteAllText(Path.Combine(statsDir, "transform.txt"), first.Stats);
        string analyticDir = Path.Combine(directory, "analytic");
        Directory.CreateDirectory(analyticDir);
        File.WriteAllText(Path.Combine(analyticDir, "shape.txt"), first.Analytic);
        string heightDir = Path.Combine(directory, "heightfield");
        Directory.CreateDirectory(heightDir);
        File.WriteAllText(Path.Combine(heightDir, "field.txt"), first.Heightfield);
        string gravityDir = Path.Combine(directory, "gravity");
        Directory.CreateDirectory(gravityDir);
        File.WriteAllText(Path.Combine(gravityDir, "matrix.txt"), first.Gravity);
        string collisionDir = Path.Combine(directory, "collision");
        Directory.CreateDirectory(collisionDir);
        File.WriteAllText(Path.Combine(collisionDir, "replay.txt"), first.Collision);
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), manifest.ToString());
    }

    static Captured Capture(string directory)
    {
        MethodInfo method = SampleMethod();
        var native = new StringBuilder();
        native.Append(FormatLine).Append('\n');
        native.Append(NativeProof("before_construct"));
        var sceneA = new JoltScene();
        var sceneB = new JoltScene();
        native.Append(NativeProof("after_construct"));

        Call[] sceneACalls = SceneA();
        Call[] sceneBCalls = SceneB();
        var calls = new StringBuilder();
        calls.Append(FormatLine).Append('\n');
        calls.Append("clock Stopwatch\n");
        calls.Append("units time_step_seconds gap_ticks\n");
        calls.Append("config unused\n");
        AppendCalls(calls, "A", sceneACalls);
        AppendCalls(calls, "B", sceneBCalls);

        var result = new StringBuilder();
        result.Append(FormatLine).Append('\n');
        result.Append("method NoteDilationSample\n");
        result.Append("source JoltScene.cs 2928-2942\n");
        result.Append("access private\n");
        result.Append("config unused\n");
        result.Append("initialise not_called\n");
        result.Append("clock Stopwatch\n");
        result.Append("units time_step_seconds gap_ticks dilation_ratio\n");
        int window = WindowLength(sceneA);
        if (WindowLength(sceneB) != window)
            throw new InvalidOperationException("dilation windows differ");
        result.Append("window ").Append(window.ToString(CultureInfo.InvariantCulture)).Append('\n');

        AppendScene(result, "A", sceneA, method, sceneACalls);
        AppendScene(result, "B", sceneB, method, sceneBCalls);
        result.Append("property_matches_field true\n");
        CharacterOracle.Texts character = CharacterOracle.Capture();
        string stats = StatsText();
        string analytic = AnalyticText(directory);
        string heightfield = HeightfieldText(directory);
        string gravity = GravityText();
        string collision = CollisionOracle.Capture();
        native.Append(NativeProof("after_calls"));
        native.Append("witness s_foundationRefCount stays 0, so JoltPhysicsBackend.Initialize did not call Foundation.Init\n");
        native.Append("witness joltc is not mapped, so Foundation.Init did not call JPH_Init\n");
        return new Captured(calls.ToString(), result.ToString(), native.ToString(), character.Calls, character.Result, stats, analytic, heightfield, gravity, collision);
    }

    const double HealthyFrameMs = 1000.0;
    // Inside the healthy side of SimStatsReporter.cs 312. The reporter reads the
    // clock again, so this setpoint is not the measured residue. A residue past
    // 1250 ms would take the stall branch and the capture rejects that run.
    const double HealthyUpdateMs = 500.0;
    // 2^20 seconds. That float's bin is wider than the clock read at line 276,
    // so the cast is this exact value and the stall branch is reproducible.
    const double StallUpdateMs = 1048576000.0;
    const int StallFrames = 209736;

    static string StatsText()
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("method NoteDilationSample\n");
        text.Append("source JoltScene.cs 2928-2942\n");
        text.Append("source_simulate JoltScene.cs 2786-2833\n");
        text.Append("source_substeps JoltScene.cs 3107-3156\n");
        text.Append("factor SimStatsReporter.cs 178-183\n");
        text.Append("transform SimStatsReporter.cs 319-338\n");
        text.Append("round SimStatsReporter.cs 368-370\n");
        text.Append("branch SimStatsReporter.cs 312-317\n");
        text.Append("clock SimStatsReporter.cs 274-276\n");
        text.Append("step SimStatsReporter.cs 390\n");
        text.Append("header SimStatsReporter.cs 1-26\n");
        text.Append("native not_called\n");
        text.Append("healthy_frame_ms ").Append(HealthyFrameMs.ToString("0", CultureInfo.InvariantCulture)).Append('\n');
        text.Append("healthy_update_ms ").Append(HealthyUpdateMs.ToString("0", CultureInfo.InvariantCulture)).Append('\n');
        text.Append("stall_update_ms ").Append(StallUpdateMs.ToString("0", CultureInfo.InvariantCulture)).Append('\n');
        text.Append("physics_step_ms ").Append(FloatBits(20f)).Append('\n');
        AppendSimulate(text, "null");
        AppendSimulate(text, "once");
        AppendSimulate(text, "substep_empty");
        AppendSimulate(text, "substeps");

        var scene = new Scene(new OpenSim.Framework.RegionInfo());
        scene.RegionInfo.RegionName = "fixture";
        SetScene(scene, "m_active", true);
        AppendStats(text, scene, "normalized-11", true, 0.0909f, 11, 1f, 1f, false);
        AppendStats(text, scene, "raw-11", false, 0.0909f, 11, 1f, 1f, false);
        AppendStats(text, scene, "constructor-frame", true, 0.0908f, 11, 1f, 1f, false);
        AppendStats(text, scene, "physics-off", true, 0.0909f, 11, 1f, 0f, false);
        AppendStats(text, scene, "physics-capped", true, 0.0909f, 11, 1f, 2f, false);
        AppendStats(text, scene, "no-frames", true, 0.0909f, 0, 1f, 1f, false);
        AppendStats(text, scene, "nan-dilation", true, 0.0909f, 11, BitConverter.Int32BitsToSingle(0x7FC00001), 1f, false);
        AppendStats(text, scene, "signed-zero", true, 0.0909f, 1, -0.0004f, 1f, false);
        AppendStats(text, scene, "stalled", true, 0.0909f, StallFrames, 1f, 1f, true);
        return text.ToString();
    }

    static void AppendSimulate(StringBuilder text, string name)
    {
        var scene = new JoltScene();
        typeof(JoltScene).GetProperty("RegionName").SetValue(scene, "fixture");
        var backend = new RecordingBackend();
        int steps = 0;
        float returned;
        if (name == "null")
        {
            returned = scene.Simulate(0.0909f);
        }
        else
        {
            backend.RecordHeartbeat = true;
            SetSceneField(scene, "_backend", backend);
            if (name == "substep_empty")
            {
                SetSceneField(scene, "_substeps", Substeps(45f));
                returned = scene.Simulate(0.0001f);
            }
            else if (name == "substeps")
            {
                SetSceneField(scene, "_substeps", Substeps(45f));
                returned = scene.Simulate(0.0909f);
            }
            else if (name == "once")
            {
                returned = scene.Simulate(0.0909f);
            }
            else
            {
                throw new InvalidOperationException("unknown simulate " + name);
            }
            steps = backend.HeartbeatSteps;
        }
        text.Append("simulate ").Append(name).Append(' ').Append(FloatBits(returned)).Append(' ').Append(steps.ToString(CultureInfo.InvariantCulture)).Append('\n');
    }

    static object Substeps(float rate)
    {
        Type type = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.SubstepAccumulator")
            ?? throw new InvalidOperationException("SubstepAccumulator was not found");
        return Activator.CreateInstance(type, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic, null, new object[] { rate }, null)
            ?? throw new InvalidOperationException("SubstepAccumulator was not constructed");
    }

    static void SetSceneField(JoltScene scene, string name, object value)
    {
        FieldInfo field = typeof(JoltScene).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException(name + " was not found");
        field.SetValue(scene, value);
    }

    static void AppendStats(StringBuilder text, Scene scene, string name, bool normalized, float frameTime, int frames, float dilationEach, float physicsEach, bool stall)
    {
        SetScene(scene, "Normalized55FPS", normalized);
        SetScene(scene, "FrameTime", frameTime);
        var reporter = new SimStatsReporter(scene);
        reporter.Close();
        double frameElapsedMs = HealthyFrameMs;
        double updateElapsedMs = stall ? StallUpdateMs : HealthyUpdateMs;
        float frameElapsed = (float)(frameElapsedMs / 1000.0);
        float expectedUpdate = (float)(updateElapsedMs / 1000.0);
        bool wantStall = StallBranch(frameElapsed, expectedUpdate);
        if (wantStall != stall)
            throw new InvalidOperationException("stats elapsed setpoint is on the wrong branch");
        for (int attempt = 0; attempt < 8; attempt++)
        {
            for (int i = 0; i < frames; i++)
                reporter.AddFrameStats(dilationEach, physicsEach, 0f, 0f, 0f, 0f, 0f, 0f);
            int framesRead = (int)ReporterField(reporter, "m_fps");
            float dilationSum = (float)ReporterField(reporter, "m_timeDilation");
            float physicsSum = (float)ReporterField(reporter, "m_pfps");
            float factor = (float)ReporterField(reporter, "m_statisticsFPSfactor");
            ReporterFieldSet(reporter, "m_prevFrameStatsTS", 0.0d);
            ReporterFieldSet(reporter, "m_FrameStatsTS", frameElapsedMs);
            double timeTmp = UpdateTimestamp(expectedUpdate, updateElapsedMs, stall);
            ReporterFieldSet(reporter, "m_lastUpdateTS", timeTmp);
            InvokeHeartbeat(reporter);
            double after = (double)ReporterField(reporter, "m_lastUpdateTS");
            float actualUpdate = (float)((after - timeTmp) / 1000.0);
            bool updateOk = stall
                ? FloatBitsEqual(actualUpdate, expectedUpdate)
                : StallBranch(frameElapsed, actualUpdate) == wantStall;
            if (!updateOk)
                continue;
            var values = (float[])ReporterField(reporter, "lastReportedSimStats");
            float timeDilation = values[(int)StatsIndex.TimeDilation];
            float simFps = values[(int)StatsIndex.SimFPS];
            float physicsFps = values[(int)StatsIndex.PhysicsFPS];
            float stepMs = values[(int)StatsIndex.SimPhysicsStepMs];
            if (stepMs != 20f)
                throw new InvalidOperationException("SimPhysicsStepMs was not 20");
            text.Append("\ncase ").Append(name).Append('\n');
            text.Append("normalized ").Append(normalized ? "1" : "0").Append('\n');
            text.Append("frame_time ").Append(FloatBits(frameTime)).Append('\n');
            text.Append("frames ").Append(frames.ToString(CultureInfo.InvariantCulture)).Append('\n');
            text.Append("dilation_each ").Append(FloatBits(dilationEach)).Append('\n');
            text.Append("physics_each ").Append(FloatBits(physicsEach)).Append('\n');
            text.Append("frame_ms ").Append(frameElapsedMs.ToString("0", CultureInfo.InvariantCulture)).Append('\n');
            text.Append("update_ms ").Append(updateElapsedMs.ToString("0", CultureInfo.InvariantCulture)).Append('\n');
            text.Append("frames_read ").Append(framesRead.ToString(CultureInfo.InvariantCulture)).Append('\n');
            text.Append("dilation_sum ").Append(SumText(dilationSum)).Append('\n');
            text.Append("physics_sum ").Append(FloatBits(physicsSum)).Append('\n');
            text.Append("factor ").Append(FloatBits(factor)).Append('\n');
            text.Append("time_dilation ").Append(FloatBits(timeDilation)).Append('\n');
            text.Append("sim_fps ").Append(FloatBits(simFps)).Append('\n');
            text.Append("physics_fps ").Append(FloatBits(physicsFps)).Append('\n');
            text.Append("step_ms ").Append(FloatBits(stepMs)).Append('\n');
            return;
        }
        throw new InvalidOperationException("stats elapsed left the controlled branch for " + name);
    }

    static double UpdateTimestamp(float expectedUpdate, double updateElapsedMs, bool pinUpdate)
    {
        double now = Util.GetTimeStampMS();
        if (!pinUpdate)
            return now - updateElapsedMs;
        return now - (FloatBinCenterSeconds(expectedUpdate) * 1000.0);
    }

    static bool StallBranch(float frameElapsed, float updateElapsed)
    {
        return (double)(frameElapsed / updateElapsed) < 0.8;
    }

    static bool FloatBitsEqual(float left, float right)
    {
        return BitConverter.SingleToInt32Bits(left) == BitConverter.SingleToInt32Bits(right);
    }

    // Center of the doubles that cast to `target`, away from either tie.
    static double FloatBinCenterSeconds(float target)
    {
        float down = MathF.BitDecrement(target);
        float up = MathF.BitIncrement(target);
        double low = ((double)down + target) * 0.5;
        double high = ((double)target + up) * 0.5;
        return (low + high) * 0.5;
    }

    static string SumText(float value)
    {
        if (float.IsNaN(value))
            return "nan";
        return FloatBits(value);
    }

    static void InvokeHeartbeat(SimStatsReporter reporter)
    {
        MethodInfo method = typeof(SimStatsReporter).GetMethod("statsHeartBeat", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("statsHeartBeat was not found");
        try
        {
            method.Invoke(reporter, new object[] { null, EventArgs.Empty });
        }
        catch (TargetInvocationException exception)
        {
            throw exception.InnerException ?? exception;
        }
    }

    static object ReporterField(SimStatsReporter reporter, string name)
    {
        FieldInfo field = typeof(SimStatsReporter).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException(name + " was not found");
        return field.GetValue(reporter);
    }

    static void ReporterFieldSet(SimStatsReporter reporter, string name, object value)
    {
        FieldInfo field = typeof(SimStatsReporter).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException(name + " was not found");
        field.SetValue(reporter, value);
    }

    static void SetScene(Scene scene, string name, object value)
    {
        if (name == "m_active")
        {
            FieldInfo field = typeof(Scene).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
                ?? throw new InvalidOperationException(name + " was not found");
            field.SetValue(scene, value);
            return;
        }
        PropertyInfo property = typeof(Scene).GetProperty(name)
            ?? throw new InvalidOperationException(name + " was not found");
        property.SetValue(scene, value);
    }

    static void AppendCalls(StringBuilder calls, string scene, Call[] script)
    {
        calls.Append('\n');
        calls.Append("scene ").Append(scene).Append('\n');
        foreach (Call call in script)
            calls.Append("call ").Append(FloatBits(call.TimeStep)).Append(' ').Append(call.Gap).Append('\n');
    }

    static void AppendScene(StringBuilder result, string scene, JoltScene host, MethodInfo method, Call[] script)
    {
        result.Append('\n');
        result.Append("scene ").Append(scene).Append('\n');
        AppendSample(result, host, "before", null);
        for (int i = 0; i < script.Length; i++)
        {
            Call call = script[i];
            Invoke(method, host, call);
            AppendSample(result, host, i.ToString(CultureInfo.InvariantCulture), call);
        }
    }

    static void AppendSample(StringBuilder result, JoltScene host, string label, Call call)
    {
        FieldInfo stepsField = PrivateField("_dilationStepSeconds");
        FieldInfo gapsField = PrivateField("_dilationGapSeconds");
        var steps = (double[])stepsField.GetValue(host);
        var gaps = (double[])gapsField.GetValue(host);
        int count = (int)PrivateField("_dilationCount").GetValue(host);
        int next = (int)PrivateField("_dilationNext").GetValue(host);
        float stored = (float)PrivateField("_timeDilation").GetValue(host);
        float property = host.TimeDilation;
        if (BitConverter.SingleToInt32Bits(property) != BitConverter.SingleToInt32Bits(stored))
            throw new InvalidOperationException("TimeDilation does not match _timeDilation");
        result.Append("sample ").Append(label).Append('\n');
        if (call != null)
        {
            result.Append("time_step ").Append(FloatBits(call.TimeStep)).Append('\n');
            result.Append("gap ").Append(call.Gap).Append('\n');
        }
        result.Append("count ").Append(count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        result.Append("next ").Append(next.ToString(CultureInfo.InvariantCulture)).Append('\n');
        result.Append("dilation ").Append(FloatBits(stored)).Append('\n');
        for (int i = 0; i < steps.Length; i++)
            result.Append("step ").Append(i.ToString(CultureInfo.InvariantCulture)).Append(' ').Append(StepText(steps[i])).Append('\n');
        for (int i = 0; i < gaps.Length; i++)
            result.Append("gap_slot ").Append(i.ToString(CultureInfo.InvariantCulture)).Append(' ').Append(GapText(gaps[i])).Append('\n');
    }

    static void Invoke(MethodInfo method, JoltScene host, Call call)
    {
        try
        {
            method.Invoke(host, new object[] { call.TimeStep, GapTicks(call) });
        }
        catch (TargetInvocationException exception)
        {
            throw exception.InnerException ?? exception;
        }
    }

    static long GapTicks(Call call)
    {
        if (call.Gap == "frequency")
            return Stopwatch.Frequency;
        if (call.Gap == "1")
            return 1;
        throw new InvalidOperationException("unknown gap " + call.Gap);
    }

    static MethodInfo SampleMethod()
    {
        MethodInfo method = typeof(JoltScene).GetMethod("NoteDilationSample", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("NoteDilationSample was not found");
        if (!method.IsPrivate)
            throw new InvalidOperationException("NoteDilationSample is not private");
        ParameterInfo[] parameters = method.GetParameters();
        if (parameters.Length != 2 || parameters[0].ParameterType != typeof(float) || parameters[1].ParameterType != typeof(long))
            throw new InvalidOperationException("NoteDilationSample parameters changed");
        return method;
    }

    static FieldInfo PrivateField(string name)
    {
        return typeof(JoltScene).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException(name + " was not found");
    }

    static int WindowLength(JoltScene host)
    {
        var steps = (double[])PrivateField("_dilationStepSeconds").GetValue(host);
        var gaps = (double[])PrivateField("_dilationGapSeconds").GetValue(host);
        if (steps.Length != gaps.Length || steps.Length < 1)
            throw new InvalidOperationException("dilation arrays are empty or uneven");
        return steps.Length;
    }

    static string NativeProof(string when)
    {
        object loaded = StaticField(typeof(JoltNative), "s_loaded").GetValue(null);
        object handle = StaticField(typeof(JoltNative), "s_handle").GetValue(null);
        object count = StaticField(typeof(JoltPhysicsBackend), "s_foundationRefCount").GetValue(null);
        bool mapped = JoltcMapped();
        if (loaded != null)
            throw new InvalidOperationException("JoltNative.s_loaded was set at " + when);
        if ((IntPtr)handle != IntPtr.Zero)
            throw new InvalidOperationException("JoltNative.s_handle was set at " + when);
        if ((int)count != 0)
            throw new InvalidOperationException("s_foundationRefCount was set at " + when);
        if (mapped)
            throw new InvalidOperationException("joltc was mapped at " + when);
        var text = new StringBuilder();
        text.Append("observed ").Append(when).Append('\n');
        text.Append("JoltNative.s_loaded null\n");
        text.Append("JoltNative.s_handle 0\n");
        text.Append("JoltPhysicsBackend.s_foundationRefCount 0\n");
        text.Append("joltc_mapped false\n");
        return text.ToString();
    }

    static FieldInfo StaticField(Type type, string name)
    {
        return type.GetField(name, BindingFlags.Static | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException(type.Name + "." + name + " was not found");
    }

    static bool JoltcMapped()
    {
        foreach (ProcessModule module in Process.GetCurrentProcess().Modules)
        {
            if (IsJoltcFile(module.ModuleName) || IsJoltcFile(module.FileName))
                return true;
        }
        return false;
    }

    // The oracle assembly is named JoltConfigOracle, which contains those letters.
    // Match the native library file, not a substring of every module name.
    static bool IsJoltcFile(string name)
    {
        if (string.IsNullOrEmpty(name))
            return false;
        int slash = Math.Max(name.LastIndexOf('/'), name.LastIndexOf('\\'));
        string file = slash >= 0 ? name.Substring(slash + 1) : name;
        return file.Equals("libjoltc.so", StringComparison.OrdinalIgnoreCase)
            || file.StartsWith("libjoltc.so.", StringComparison.OrdinalIgnoreCase)
            || file.Equals("joltc.dll", StringComparison.OrdinalIgnoreCase)
            || file.Equals("libjoltc.dylib", StringComparison.OrdinalIgnoreCase);
    }

    static void RequirePortableClock()
    {
        long frequency = Stopwatch.Frequency;
        if (frequency <= 1 || frequency > (1L << 53) || (long)(double)frequency != frequency)
            throw new InvalidOperationException("Stopwatch.Frequency is not an exact double");
        // The same conversion NoteDilationSample uses: gapTicks / (double)Stopwatch.Frequency.
        double oneSecond = frequency / (double)frequency;
        double oneTick = 1d / frequency;
        if (oneSecond != 1d || oneTick == 0d || !(oneTick < 1d))
            throw new InvalidOperationException("a one-frequency gap is not exactly one second");
    }

    static string StepText(double value)
    {
        if (value != 0d && value != 1d && value != 0.5d && value != 0.25d)
            throw new InvalidOperationException("step seconds are not one of the fixture inputs");
        return DoubleBits(value);
    }

    static string GapText(double value)
    {
        if (value == 0d || value == 1d)
            return DoubleBits(value);
        if (value == 1d / Stopwatch.Frequency)
            return "frequency_divisor";
        throw new InvalidOperationException("gap seconds are not a portable value");
    }

    static string FloatBits(float value)
    {
        return BitConverter.SingleToInt32Bits(value).ToString("X8", CultureInfo.InvariantCulture);
    }

    static string DoubleBits(double value)
    {
        return BitConverter.DoubleToInt64Bits(value).ToString("X16", CultureInfo.InvariantCulture);
    }

    static Call[] SceneA()
    {
        var calls = new List<Call> { Call.Frequency(1f) };
        for (int i = 0; i < 10; i++)
            calls.Add(Call.Frequency(0.5f));
        calls.Add(Call.Frequency(0.25f));
        return calls.ToArray();
    }

    static Call[] SceneB()
    {
        return new[] { Call.OneTick(1f) };
    }

    static void RejectMachinePaths(string text, string reference)
    {
        if (text.Contains(reference, StringComparison.Ordinal) || text.Contains("/home/", StringComparison.Ordinal)
            || text.Contains("\\Users\\", StringComparison.Ordinal))
            throw new InvalidOperationException("file fixture contains a machine path");
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

    sealed class Call
    {
        internal readonly float TimeStep;
        internal readonly string Gap;

        Call(float timeStep, string gap)
        {
            TimeStep = timeStep;
            Gap = gap;
        }

        internal static Call Frequency(float timeStep) => new Call(timeStep, "frequency");

        internal static Call OneTick(float timeStep) => new Call(timeStep, "1");
    }

    readonly record struct Captured(string Calls, string Result, string Native, string CharacterCalls, string CharacterResult, string Stats, string Analytic, string Heightfield, string Gravity, string Collision);

    static string AnalyticText(string directory)
    {
        string assets = File.ReadAllText(StockAssetPath(directory));
        string kind = LineValue(assets, "kind");
        if (kind != "analytic-box")
            throw new InvalidOperationException("stock drop asset kind is " + kind);
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("method CookPrimShape\n");
        text.Append("source JoltScene.cs 1229-1259\n");
        text.Append("cuts JoltScene.cs 2224-2228\n");
        text.Append("box JoltPhysicsBackend.cs 1712-1718\n");
        text.Append("sphere JoltPhysicsBackend.cs 1720-1723\n");
        text.Append("cylinder JoltPhysicsBackend.cs 1731-1741\n");
        text.Append("access internal\n");
        text.Append("initialise not_called\n");
        text.Append("native_proof after_calls\n");
        text.Append("mesher null\n");
        text.Append("stock_source jolt-record/drop\n");
        text.Append("stock_asset_kind ").Append(kind).Append('\n');
        // The drop releases a 1 m cube. CreateCylinder() is Square+Curve1, a tube,
        // so the cylinder case is Circle + Straight, which is what the fast path cooks.
        AppendShape(text, "box-static", false, PrimitiveBaseShape.CreateBox(), Om(1f, 1f, 1f), "box");
        AppendShape(text, "box-dynamic", true, PrimitiveBaseShape.CreateBox(), Om(1f, 1f, 1f), "box");
        AppendShape(text, "sphere-static", false, PrimitiveBaseShape.CreateSphere(), Om(2f, 2f, 2f), "sphere");
        AppendShape(text, "sphere-dynamic", true, PrimitiveBaseShape.CreateSphere(), Om(2f, 2f, 2f), "sphere");
        AppendShape(text, "cylinder-static", false, CircleStraight(), Om(1f, 1f, 2f), "cylinder");
        AppendShape(text, "cylinder-dynamic", true, CircleStraight(), Om(1f, 1f, 2f), "cylinder");
        AppendShape(text, "ellipsoid-static", false, PrimitiveBaseShape.CreateSphere(), Om(1f, 2f, 3f), "bbox(fallback)");
        AppendShape(text, "ellipsoid-dynamic", true, PrimitiveBaseShape.CreateSphere(), Om(1f, 2f, 3f), "bbox(fallback)");
        return text.ToString();
    }

    static PrimitiveBaseShape CircleStraight()
    {
        PrimitiveBaseShape shape = PrimitiveBaseShape.CreateBox();
        shape.ProfileShape = ProfileShape.Circle;
        shape.PathCurve = (byte)Extrusion.Straight;
        return shape;
    }

    static OpenMetaverse.Vector3 Om(float x, float y, float z) => new OpenMetaverse.Vector3(x, y, z);

    static string StockAssetPath(string directory)
    {
        DirectoryInfo parent = Directory.GetParent(directory)
            ?? throw new InvalidOperationException("fixture directory has no parent");
        return Path.Combine(parent.FullName, "jolt-record", "drop", "assets.txt");
    }

    static string LineValue(string text, string key)
    {
        string prefix = key + " ";
        foreach (string raw in text.Split('\n'))
        {
            string line = raw.TrimEnd('\r');
            if (line.StartsWith(prefix, StringComparison.Ordinal))
                return line.Substring(prefix.Length);
        }
        throw new InvalidOperationException("stock fixture missing " + key);
    }

    static void AppendShape(StringBuilder text, string name, bool physical, PrimitiveBaseShape shape,
        OpenMetaverse.Vector3 size, string expectKind)
    {
        var backend = new RecordingBackend { RecordAnalytic = true };
        var scene = new JoltScene();
        MethodInfo method = CookMethod();
        object[] args =
        {
            backend, shape, size, physical,
            default(Quaternion), null, 0, 0d,
            null, false, false,
        };
        ShapeId cooked;
        try
        {
            cooked = (ShapeId)method.Invoke(scene, args);
        }
        catch (TargetInvocationException exception)
        {
            throw new InvalidOperationException(name + " CookPrimShape threw: " + (exception.InnerException ?? exception).Message, exception);
        }
        var axis = (Quaternion)args[4];
        string kind = (string)args[5];
        int hulls = (int)args[6];
        double massVolume = (double)args[7];
        if (kind != expectKind)
            throw new InvalidOperationException(name + " kind " + kind + " != " + expectKind);
        if (!cooked.IsValid || cooked.Value != 1)
            throw new InvalidOperationException(name + " shape id is " + cooked.Value);
        int calls = backend.BoxCalls + backend.SphereCalls + backend.CylinderCalls;
        if (calls != 1)
            throw new InvalidOperationException(name + " made " + calls + " analytic constructors");
        bool analytic = kind == "box" || kind == "sphere" || kind == "cylinder";
        if (kind == "box" && backend.BoxCalls != 1)
            throw new InvalidOperationException(name + " did not construct a box");
        if (kind == "sphere" && backend.SphereCalls != 1)
            throw new InvalidOperationException(name + " did not construct a sphere");
        if (kind == "cylinder" && backend.CylinderCalls != 1)
            throw new InvalidOperationException(name + " did not construct a cylinder");
        if (!analytic && (backend.SphereCalls != 0 || backend.CylinderCalls != 0 || backend.BoxCalls != 1))
            throw new InvalidOperationException(name + " rejected shape took an analytic constructor");

        text.Append('\n');
        text.Append("case ").Append(name).Append('\n');
        text.Append("physical ").Append(physical ? "true" : "false").Append('\n');
        text.Append("profile ").Append(((byte)shape.ProfileShape).ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("path ").Append(shape.PathCurve.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("cuts ")
            .Append(shape.ProfileBegin.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.ProfileEnd.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.ProfileHollow.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathTwist.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathTwistBegin.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathBegin.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathEnd.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathTaperX.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathTaperY.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathScaleX.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathScaleY.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathShearX.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(shape.PathShearY.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("size ").Append(FloatBits(size.X)).Append(' ').Append(FloatBits(size.Y)).Append(' ')
            .Append(FloatBits(size.Z)).Append('\n');
        text.Append("kind ").Append(kind).Append('\n');
        text.Append("analytic ").Append(analytic ? "true" : "false").Append('\n');
        text.Append("box_calls ").Append(backend.BoxCalls.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("sphere_calls ").Append(backend.SphereCalls.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("cylinder_calls ").Append(backend.CylinderCalls.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("half ");
        if (backend.BoxCalls == 1)
            text.Append(FloatBits(backend.BoxHalf.X)).Append(' ').Append(FloatBits(backend.BoxHalf.Y)).Append(' ')
                .Append(FloatBits(backend.BoxHalf.Z)).Append('\n');
        else
            text.Append("none\n");
        text.Append("radius ").Append(backend.SphereCalls == 1 ? FloatBits(backend.SphereRadius) : "none").Append('\n');
        text.Append("cylinder_half ").Append(backend.CylinderCalls == 1 ? FloatBits(backend.CylinderHalfHeight) : "none").Append('\n');
        text.Append("cylinder_radius ").Append(backend.CylinderCalls == 1 ? FloatBits(backend.CylinderRadius) : "none").Append('\n');
        text.Append("axis ").Append(FloatBits(axis.X)).Append(' ').Append(FloatBits(axis.Y)).Append(' ')
            .Append(FloatBits(axis.Z)).Append(' ').Append(FloatBits(axis.W)).Append('\n');
        text.Append("hulls ").Append(hulls.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("mass_volume ").Append(DoubleBits(massVolume)).Append('\n');
        text.Append("shape ").Append(cooked.Value.ToString(CultureInfo.InvariantCulture)).Append('\n');
    }

    static MethodInfo CookMethod()
    {
        MethodInfo found = null;
        foreach (MethodInfo method in typeof(JoltScene).GetMethods(BindingFlags.Instance | BindingFlags.NonPublic))
        {
            if (method.Name != "CookPrimShape" || method.GetParameters().Length != 11)
                continue;
            if (found != null)
                throw new InvalidOperationException("more than one CookPrimShape with 11 parameters");
            found = method;
        }
        if (found == null)
            throw new InvalidOperationException("CookPrimShape was not found");
        return found;
    }

    static string HeightfieldText(string directory)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("method SetTerrain\n");
        text.Append("source JoltScene.cs 3552-3613\n");
        text.Append("access public\n");
        text.Append("initialise not_called\n");
        text.Append("layout y*m+x\n");
        text.Append("native_proof after_calls\n");
        AppendStock(text, directory);
        AppendCooked(text, "square-1", 1, 1, Map(1, 0x40490FDB));
        AppendCooked(text, "square-3", 3, 3, Sequence(3, 3, 0x40000000));
        AppendCooked(text, "rect-5x3", 5, 3, Sequence(5, 3, 0x40000000));
        AppendCooked(text, "rect-2x4", 2, 4, Sequence(2, 4, 0x40000000));
        AppendCooked(text, "square-256", 256, 256, Sequence(256, 256, 0x40000000));
        AppendCooked(text, "extra-tail", 2, 2, ExtraTail());
        AppendCooked(text, "special-bits", 2, 2, SpecialBits());
        AppendRejected(text, "reject-short", 4, 3, Sequence(4, 3, 0x40000000).Take(11).ToArray());
        AppendRejected(text, "reject-null", 4, 3, null);
        AppendRejected(text, "reject-zero-x", 0, 4, Array.Empty<float>());
        AppendRejected(text, "reject-zero-y", 5, 0, Array.Empty<float>());
        return text.ToString();
    }

    static void AppendStock(StringBuilder text, string directory)
    {
        string config = File.ReadAllText(StockPath(directory, "config.txt"));
        string provenance = File.ReadAllText(StockPath(directory, "provenance.txt"));
        string gravity = LineValue(config, "Gravity");
        string linear = LineValue(config, "PrimLinearDamping");
        string angular = LineValue(config, "PrimAngularDamping");
        string dt64 = LineValue(provenance, "dt_f64");
        string dt32 = LineValue(provenance, "simulate_dt_f32");

        Type configType = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltConfig")
            ?? throw new InvalidOperationException("JoltConfig was not found");
        object instance = Activator.CreateInstance(configType, true)
            ?? throw new InvalidOperationException("JoltConfig could not be constructed");
        string compiledGravity = FloatBits((float)ConfigField(configType, "Gravity").GetValue(instance));
        string compiledLinear = FloatBits((float)ConfigField(configType, "PrimLinearDamping").GetValue(instance));
        string compiledAngular = FloatBits((float)ConfigField(configType, "PrimAngularDamping").GetValue(instance));
        double heartbeat = 1.0 / 11.0;
        string compiledDt64 = DoubleBits(heartbeat);
        string compiledDt32 = FloatBits((float)heartbeat);
        RequireEqual("gravity", compiledGravity, gravity);
        RequireEqual("prim linear damping", compiledLinear, linear);
        RequireEqual("prim angular damping", compiledAngular, angular);
        RequireEqual("dt_f64", compiledDt64, dt64);
        RequireEqual("simulate_dt_f32", compiledDt32, dt32);

        text.Append("stock_source jolt-record/drop\n");
        text.Append("stock_gravity ").Append(gravity).Append('\n');
        text.Append("stock_prim_linear_damping ").Append(linear).Append('\n');
        text.Append("stock_prim_angular_damping ").Append(angular).Append('\n');
        text.Append("stock_dt_f64 ").Append(dt64).Append('\n');
        text.Append("stock_simulate_dt_f32 ").Append(dt32).Append('\n');
        text.Append("compiled_gravity ").Append(compiledGravity).Append('\n');
        text.Append("compiled_prim_linear_damping ").Append(compiledLinear).Append('\n');
        text.Append("compiled_prim_angular_damping ").Append(compiledAngular).Append('\n');
        text.Append("compiled_dt_f64 ").Append(compiledDt64).Append('\n');
        text.Append("compiled_simulate_dt_f32 ").Append(compiledDt32).Append('\n');
        text.Append("stock_match true\n");
    }

    static string StockPath(string directory, string name)
    {
        DirectoryInfo parent = Directory.GetParent(directory)
            ?? throw new InvalidOperationException("fixture directory has no parent");
        return Path.Combine(parent.FullName, "jolt-record", "drop", name);
    }

    static FieldInfo ConfigField(Type type, string name)
    {
        return type.GetField(name, BindingFlags.Instance | BindingFlags.Public)
            ?? throw new InvalidOperationException("JoltConfig." + name + " was not found");
    }

    static void RequireEqual(string label, string compiled, string stock)
    {
        if (compiled != stock)
            throw new InvalidOperationException(label + " " + compiled + " != stock " + stock);
    }

    static float[] Map(int count, int bits)
    {
        var map = new float[count];
        float value = BitConverter.Int32BitsToSingle(bits);
        for (int i = 0; i < count; i++)
            map[i] = value;
        return map;
    }

    static float[] Sequence(int width, int height, int origin)
    {
        var map = new float[width * height];
        for (int i = 0; i < map.Length; i++)
            map[i] = BitConverter.Int32BitsToSingle(origin + i);
        return map;
    }

    static float[] ExtraTail()
    {
        float[] map = Sequence(2, 2, 0x40000000);
        return map.Concat(new[]
        {
            BitConverter.Int32BitsToSingle(0x42F60000),
            BitConverter.Int32BitsToSingle(unchecked((int)0xC2F60000)),
        }).ToArray();
    }

    static float[] SpecialBits()
    {
        return new[]
        {
            BitConverter.Int32BitsToSingle(unchecked((int)0x80000000)),
            BitConverter.Int32BitsToSingle(0x7F800000),
            BitConverter.Int32BitsToSingle(unchecked((int)0xFF800000)),
            BitConverter.Int32BitsToSingle(0x7FC00001),
        };
    }

    static void AppendCooked(StringBuilder text, string name, int sx, int sy, float[] map)
    {
        RecordingBackend backend = Run(name, sx, sy, map);
        float[] stored = StoredField(name);
        int storedM = StoredM();
        if (backend.HeightFieldCalls != 1 || backend.TerrainSets != 1)
            throw new InvalidOperationException(name + " did not cook exactly once");
        if (stored == null || storedM < 2)
            throw new InvalidOperationException(name + " stored no field");
        if (backend.LastSampleCountX != storedM || backend.LastSampleCountY != storedM)
            throw new InvalidOperationException(name + " sample counts are not the stored square");
        if (!BitsEqual(stored, backend.LastHeights))
            throw new InvalidOperationException(name + " stored field is not the span passed in");
        if (backend.LastTerrainShape.Value != 1)
            throw new InvalidOperationException(name + " shape is not the id CreateHeightFieldShape returned");
        RequireScale(name, backend.LastScale);
        RequireZero(name, backend.LastTerrainPosition);
        if (backend.GetCapacityStats().TerrainBodyMissing)
            throw new InvalidOperationException(name + " reports a missing terrain body");

        text.Append('\n');
        text.Append("case ").Append(name).Append('\n');
        text.Append("region ").Append(sx.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(sy.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("cooked true\n");
        text.Append("heightfield_calls 1\n");
        text.Append("terrain_sets 1\n");
        text.Append("sample_count ").Append(storedM.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(storedM.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("field_m ").Append(storedM.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("shape 1\n");
        text.Append("scale ").Append(FloatBits(backend.LastScale.X)).Append(' ')
            .Append(FloatBits(backend.LastScale.Y)).Append(' ')
            .Append(FloatBits(backend.LastScale.Z)).Append('\n');
        text.Append("position ").Append(FloatBits(backend.LastTerrainPosition.X)).Append(' ')
            .Append(FloatBits(backend.LastTerrainPosition.Y)).Append(' ')
            .Append(FloatBits(backend.LastTerrainPosition.Z)).Append('\n');
        text.Append("stored_matches_span true\n");
        if (name == "square-256")
        {
            AppendDigest(text, map, stored, storedM);
            return;
        }
        text.Append("input_count ").Append(map.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("input\n");
        AppendFloats(text, map);
        text.Append("field_count ").Append(stored.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("field\n");
        AppendFloats(text, stored);
    }

    // SHA-256 of every stored float, little-endian IEEE bits, in y*m+x order.
    static void AppendDigest(StringBuilder text, float[] map, float[] stored, int storedM)
    {
        text.Append("input_formula origin-plus-index\n");
        text.Append("input_origin 40000000\n");
        text.Append("input_count ").Append(map.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("field_encoding digest\n");
        text.Append("byte_order little-endian-float32\n");
        text.Append("field_count ").Append(stored.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("field_sha256 ").Append(FieldDigest(stored)).Append('\n');
        AppendPoint(text, "corner", 0, 0, stored, storedM);
        AppendPoint(text, "edge", storedM - 1, 0, stored, storedM);
        AppendPoint(text, "edge", 0, storedM - 1, stored, storedM);
        AppendPoint(text, "corner", storedM - 1, storedM - 1, stored, storedM);
    }

    static string FieldDigest(float[] stored)
    {
        var bytes = new byte[stored.Length * 4];
        for (int i = 0; i < stored.Length; i++)
        {
            int bits = BitConverter.SingleToInt32Bits(stored[i]);
            bytes[i * 4] = (byte)bits;
            bytes[i * 4 + 1] = (byte)(bits >> 8);
            bytes[i * 4 + 2] = (byte)(bits >> 16);
            bytes[i * 4 + 3] = (byte)(bits >> 24);
        }
        return Convert.ToHexString(SHA256.HashData(bytes));
    }

    static void AppendPoint(StringBuilder text, string label, int x, int y, float[] stored, int m)
    {
        text.Append(label).Append(' ')
            .Append(x.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(y.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(FloatBits(stored[y * m + x])).Append('\n');
    }

    static void AppendRejected(StringBuilder text, string name, int sx, int sy, float[] map)
    {
        RecordingBackend backend = Run(name, sx, sy, map);
        float[] stored = StoredField(name);
        int storedM = StoredM();
        if (backend.HeightFieldCalls != 0 || backend.TerrainSets != 0 || stored != null || storedM != 0)
            throw new InvalidOperationException(name + " cooked or stored a field");
        text.Append('\n');
        text.Append("case ").Append(name).Append('\n');
        text.Append("region ").Append(sx.ToString(CultureInfo.InvariantCulture)).Append(' ')
            .Append(sy.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("cooked false\n");
        text.Append("heightfield_calls 0\n");
        text.Append("terrain_sets 0\n");
        if (map == null)
            text.Append("input null\n");
        else
        {
            text.Append("input_count ").Append(map.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
            text.Append("input\n");
            AppendFloats(text, map);
        }
        text.Append("field null\n");
    }

    static JoltScene HeightfieldScene;

    static RecordingBackend Run(string name, int sx, int sy, float[] map)
    {
        var backend = new RecordingBackend { RecordTerrain = true };
        var scene = new JoltScene();
        PrivateField("_backend").SetValue(scene, backend);
        PrivateField("_regionSizeX").SetValue(scene, sx);
        PrivateField("_regionSizeY").SetValue(scene, sy);
        try
        {
            scene.SetTerrain(map);
        }
        catch (Exception exception)
        {
            throw new InvalidOperationException(name + " SetTerrain threw: " + exception.Message, exception);
        }
        HeightfieldScene = scene;
        return backend;
    }

    static float[] StoredField(string name)
    {
        object value = PrivateField("_terrainField").GetValue(HeightfieldScene);
        if (value == null)
            return null;
        if (value is float[] field)
            return field;
        throw new InvalidOperationException(name + " stored a field that is not a float array");
    }

    static int StoredM()
    {
        return (int)PrivateField("_terrainFieldM").GetValue(HeightfieldScene);
    }

    static void RequireScale(string name, Vector3 scale)
    {
        if (FloatBits(scale.X) != "3F800000" || FloatBits(scale.Y) != "3F800000" || FloatBits(scale.Z) != "3F800000")
            throw new InvalidOperationException(name + " scale is not 1,1,1");
    }

    static void RequireZero(string name, Vector3 position)
    {
        if (FloatBits(position.X) != "00000000" || FloatBits(position.Y) != "00000000" || FloatBits(position.Z) != "00000000")
            throw new InvalidOperationException(name + " position is not zero");
    }

    static bool BitsEqual(float[] left, float[] right)
    {
        if (left == null || right == null || left.Length != right.Length)
            return false;
        for (int i = 0; i < left.Length; i++)
        {
            if (BitConverter.SingleToInt32Bits(left[i]) != BitConverter.SingleToInt32Bits(right[i]))
                return false;
        }
        return true;
    }

    static void AppendFloats(StringBuilder text, float[] values)
    {
        for (int i = 0; i < values.Length; i++)
            text.Append(FloatBits(values[i])).Append('\n');
    }

    static string GravityText()
    {
        var scene = new JoltScene();
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("method Buoyancy\n");
        text.Append("source JoltPrim.cs 1784-1819\n");
        text.Append("source JoltPrim.cs 2303-2332\n");
        text.Append("source JoltPrim.cs 2755-2767\n");
        text.Append("source JoltPrim.cs 2794-2810\n");
        text.Append("source HalcyonBasicForces.cs 103-104\n");
        text.Append("header HalcyonBasicForces.cs 1-33\n");
        text.Append("water not_used\n");
        text.Append("native not_called\n");
        text.Append("max_linear ").Append(FloatBits(SceneFloat(scene, "BodyMaxLinearSpeed"))).Append('\n');
        text.Append("step_rate ").Append(FloatBits(ConfigFloat("MaxPhysicsStepRate"))).Append('\n');
        object gravity = SceneProperty(scene, "DefaultGravity");
        text.Append("gravity_z ").Append(FloatBits(VectorZ(gravity))).Append('\n');
        text.Append("gravity_len ").Append(FloatBits(VectorLength(gravity))).Append('\n');
        float threshold = 0.977f;
        text.Append("threshold ").Append(FloatBits(threshold)).Append('\n');
        float huge = 1e10f;
        text.Append("clamp_modifier ").Append(FloatBits(huge)).Append('\n');
        text.Append("clamp_max ").Append(FloatBits(ScriptFactor(Prim(scene, new RecordingBackend { RecordGravity = true }, 1, true, huge)))).Append('\n');
        text.Append("clamp_min ").Append(FloatBits(ScriptFactor(Prim(scene, new RecordingBackend { RecordGravity = true }, 1, true, -huge)))).Append('\n');

        text.Append("function\n");
        AppendFunction(text, 0f);
        AppendFunction(text, BitConverter.Int32BitsToSingle(unchecked((int)0x80000000)));
        AppendFunction(text, -1f);
        AppendFunction(text, 0.976f);
        AppendFunction(text, Below(threshold));
        AppendFunction(text, threshold);
        AppendFunction(text, 0.98f);
        AppendFunction(text, Below(1f));
        AppendFunction(text, 1f);
        AppendFunction(text, 1.5f);
        AppendFunction(text, BitConverter.Int32BitsToSingle(unchecked((int)0x7FC00001)));
        AppendFunction(text, float.PositiveInfinity);
        AppendFunction(text, float.NegativeInfinity);

        float below = Below(threshold);
        float underOne = Below(1f);
        RunCase(text, "boundary", "InWorldz", true, 1, true, 1f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0f),
            GravityStep.Buoyancy(BitConverter.Int32BitsToSingle(unchecked((int)0x80000000))),
            GravityStep.Buoyancy(0.5f),
            GravityStep.Buoyancy(0.5f),
            GravityStep.Buoyancy(0f),
            GravityStep.Buoyancy(0.976f),
            GravityStep.Buoyancy(below),
            GravityStep.Buoyancy(threshold),
            GravityStep.Buoyancy(0.98f),
            GravityStep.Buoyancy(underOne),
            GravityStep.Buoyancy(1f),
            GravityStep.Buoyancy(1.5f),
            GravityStep.Buoyancy(BitConverter.Int32BitsToSingle(unchecked((int)0x7FC00001))),
            GravityStep.Buoyancy(float.PositiveInfinity),
            GravityStep.Buoyancy(float.NegativeInfinity),
        });
        RunCase(text, "second-life", "SecondLife", true, 1, true, 1f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.98f),
            GravityStep.Buoyancy(threshold),
            GravityStep.Buoyancy(1f),
        });
        RunCase(text, "multiplier", "InWorldz", true, 1, true, 2f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
        });
        RunCase(text, "clamp-high", "InWorldz", true, 1, true, huge, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
        });
        RunCase(text, "clamp-low", "InWorldz", true, 1, true, -huge, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
        });
        RunCase(text, "not-physical", "InWorldz", false, 1, true, 1f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
        });
        RunCase(text, "invalid-body", "InWorldz", true, 0, false, 1f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
        });
        RunCase(text, "child", "InWorldz", true, 1, true, 1f, 2, new GravityStep[]
        {
            GravityStep.Child(0.25f),
            GravityStep.Child(BitConverter.Int32BitsToSingle(unchecked((int)0x7FC00001))),
        });
        int car = VehicleCode("TYPE_CAR");
        int none = VehicleCode("TYPE_NONE");
        RunCase(text, "vehicle", "InWorldz", true, 1, true, 1f, 0, new GravityStep[]
        {
            GravityStep.Type(car),
            GravityStep.Buoyancy(0.5f),
            GravityStep.Type(none),
        });
        RunCase(text, "child-vehicle", "InWorldz", true, 1, true, 1f, 2, new GravityStep[]
        {
            GravityStep.Type(car),
            GravityStep.Child(0.25f),
            GravityStep.Type(none),
        });
        // 7 is not a Vehicle type. ProcessTypeChangeCore maps it to None.
        RunCase(text, "unknown-type", "InWorldz", true, 1, true, 1f, 0, new GravityStep[]
        {
            GravityStep.Buoyancy(0.5f),
            GravityStep.Type(car),
            GravityStep.Type(7),
        });
        return text.ToString();
    }

    static void AppendFunction(StringBuilder text, float value)
    {
        text.Append("in ").Append(FloatBits(value)).Append(" out ").Append(FloatBits(Legacy(value))).Append('\n');
    }

    static void RunCase(StringBuilder text, string name, string behaviour, bool physical, uint body, bool valid, float modifier, uint child, GravityStep[] steps)
    {
        var scene = new JoltScene();
        SetBehaviour(scene, behaviour);
        if (BehaviourName(scene) != behaviour)
            throw new InvalidOperationException(name + " behaviour did not stick");
        var backend = new RecordingBackend { RecordGravity = true };
        object root = Prim(scene, backend, body, physical, modifier);
        object linked = null;
        if (child != 0)
        {
            linked = Prim(scene, backend, child, physical, modifier);
            SetField(linked, "_linkRoot", root);
        }
        text.Append("case ").Append(name).Append('\n');
        text.Append("behaviour ").Append(behaviour).Append('\n');
        text.Append("physical ").Append(physical ? "true" : "false").Append('\n');
        text.Append("body ").Append(body.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("valid ").Append(valid ? "true" : "false").Append('\n');
        text.Append("modifier ").Append(FloatBits(modifier)).Append('\n');
        text.Append("child ").Append(child.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("initial ").Append(FloatBits(FloatField(root, "_buoyancy"))).Append('\n');
        foreach (GravityStep step in steps)
        {
            backend.GravityWrites.Clear();
            backend.Activates = 0;
            if (step.Kind == "child")
            {
                if (linked == null)
                    throw new InvalidOperationException(name + " has no child");
                SetProp(linked, "Buoyancy", step.Value);
                text.Append("action child ").Append(FloatBits(step.Value)).Append('\n');
                text.Append("root_stored ").Append(FloatBits(FloatField(root, "_buoyancy"))).Append('\n');
                text.Append("child_stored ").Append(FloatBits(FloatField(linked, "_buoyancy"))).Append('\n');
                text.Append("child_get ").Append(FloatBits((float)Prop(linked, "Buoyancy"))).Append('\n');
            }
            else if (step.Kind == "type")
            {
                SetProp(root, "VehicleType", step.Code);
                text.Append("action type ").Append(step.Code.ToString(CultureInfo.InvariantCulture)).Append('\n');
                text.Append("stored ").Append(FloatBits(FloatField(root, "_buoyancy"))).Append('\n');
                text.Append("get ").Append(FloatBits((float)Prop(root, "Buoyancy"))).Append('\n');
                text.Append("type_get ").Append(Convert.ToInt32(Prop(root, "VehicleType"), CultureInfo.InvariantCulture).ToString(CultureInfo.InvariantCulture)).Append('\n');
            }
            else
            {
                SetProp(root, "Buoyancy", step.Value);
                text.Append("action buoyancy ").Append(FloatBits(step.Value)).Append('\n');
                text.Append("stored ").Append(FloatBits(FloatField(root, "_buoyancy"))).Append('\n');
                text.Append("get ").Append(FloatBits((float)Prop(root, "Buoyancy"))).Append('\n');
            }
            bool active = Convert.ToInt32(Prop(root, "VehicleType"), CultureInfo.InvariantCulture) != 0;
            text.Append("active ").Append(active ? "true" : "false").Append('\n');
            text.Append("activates ").Append(backend.Activates.ToString(CultureInfo.InvariantCulture)).Append('\n');
            text.Append("writes ").Append(backend.GravityWrites.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
            foreach (RecordingBackend.GravityWrite write in backend.GravityWrites)
                text.Append("write ").Append(write.Body.ToString(CultureInfo.InvariantCulture)).Append(' ').Append(FloatBits(write.Factor)).Append('\n');
        }
    }

    readonly struct GravityStep
    {
        internal readonly string Kind;
        internal readonly float Value;
        internal readonly int Code;

        GravityStep(string kind, float value, int code)
        {
            Kind = kind;
            Value = value;
            Code = code;
        }

        internal static GravityStep Buoyancy(float value) => new GravityStep("buoyancy", value, 0);
        internal static GravityStep Child(float value) => new GravityStep("child", value, 0);
        internal static GravityStep Type(int code) => new GravityStep("type", 0f, code);
    }

    static object Prim(JoltScene scene, RecordingBackend backend, uint body, bool physical, float modifier)
    {
        Type type = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltPrim")
            ?? throw new InvalidOperationException("JoltPrim was not in the Jolt assembly");
        object prim = RuntimeHelpers.GetUninitializedObject(type);
        SetField(prim, "_module", scene);
        SetField(prim, "_backend", backend);
        SetField(prim, "_body", new BodyId(body));
        SetField(prim, "_isPhysical", physical);
        SetField(prim, "_gravityModifier", modifier);
        SetField(prim, "_vehicleStepGate", new object());
        return prim;
    }

    static void SetBehaviour(JoltScene scene, string name)
    {
        object config = Field(typeof(JoltScene), "_joltConfig").GetValue(scene)
            ?? throw new InvalidOperationException("JoltScene has no config");
        FieldInfo behaviour = Field(config.GetType(), "BasicForcesBehaviour");
        behaviour.SetValue(config, Enum.Parse(behaviour.FieldType, name));
    }

    static string BehaviourName(JoltScene scene)
    {
        object value = SceneProperty(scene, "BasicForcesBehaviour");
        return value.ToString() ?? throw new InvalidOperationException("behaviour has no name");
    }

    static float SceneFloat(JoltScene scene, string name)
    {
        return (float)SceneProperty(scene, name);
    }

    static object SceneProperty(JoltScene scene, string name)
    {
        return Prop(scene, name);
    }

    static float ConfigFloat(string name)
    {
        Type config = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltConfig")
            ?? throw new InvalidOperationException("JoltConfig was not in the Jolt assembly");
        FieldInfo field = config.GetField(name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static)
            ?? throw new InvalidOperationException("missing " + name);
        return (float)field.GetValue(null);
    }

    static float VectorZ(object vector)
    {
        FieldInfo field = vector.GetType().GetField("Z")
            ?? throw new InvalidOperationException("Vector3.Z was not found");
        return (float)field.GetValue(vector);
    }

    static float VectorLength(object vector)
    {
        MethodInfo method = vector.GetType().GetMethod("Length", Type.EmptyTypes)
            ?? throw new InvalidOperationException("Vector3.Length() was not found");
        return (float)method.Invoke(vector, null);
    }

    static float ScriptFactor(object prim)
    {
        return (float)Prop(prim, "ScriptGravityFactor");
    }

    static float Legacy(float value)
    {
        Type type = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.HalcyonBasicForces")
            ?? throw new InvalidOperationException("HalcyonBasicForces was not loaded");
        MethodInfo method = type.GetMethod("LegacyBuoyancy", BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("LegacyBuoyancy was not found");
        try
        {
            return (float)method.Invoke(null, new object[] { value });
        }
        catch (TargetInvocationException exception)
        {
            throw exception.InnerException ?? exception;
        }
    }

    static int VehicleCode(string name)
    {
        Type type = null;
        foreach (Assembly assembly in AppDomain.CurrentDomain.GetAssemblies())
        {
            type = assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.Vehicles.Vehicle");
            if (type != null)
                break;
        }
        if (type == null)
            throw new InvalidOperationException("Vehicle enum was not loaded");
        return (int)Enum.Parse(type, name);
    }

    static float Below(float value)
    {
        return BitConverter.Int32BitsToSingle(BitConverter.SingleToInt32Bits(value) - 1);
    }

    static float FloatField(object target, string name)
    {
        return (float)Field(target.GetType(), name).GetValue(target);
    }

    static void SetField(object target, string name, object value)
    {
        Field(target.GetType(), name).SetValue(target, value);
    }

    static FieldInfo Field(Type type, string name)
    {
        return type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static)
            ?? throw new InvalidOperationException("missing field " + name);
    }

    static object Prop(object target, string name)
    {
        PropertyInfo property = target.GetType().GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("missing property " + name);
        return property.GetValue(target);
    }

    static void SetProp(object target, string name, object value)
    {
        PropertyInfo property = target.GetType().GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("missing property " + name);
        try
        {
            property.SetValue(target, value);
        }
        catch (TargetInvocationException exception)
        {
            throw exception.InnerException ?? exception;
        }
    }

}
