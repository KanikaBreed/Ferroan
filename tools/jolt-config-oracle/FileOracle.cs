// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls pinned JoltScene.NoteDilationSample (JoltScene.cs 2928-2942) and records
// the private dilation window. The same capture compiles JoltCharacter's capsule,
// mass, latch, intent and marker descriptor, then Simulate's return of 1 and
// SimStatsReporter's FPS transform, then calls CookPrimShape
// (JoltScene.cs 1229-1259) and records the analytic constructor it chooses,
// before the final native proof. The physics scene is constructed and never
// initialised, so JoltNative stays unloaded and Foundation.Init is not called.

using System.Diagnostics;
using System.Globalization;
using System.Numerics;
using System.Reflection;
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
            || first.Analytic != second.Analytic)
            throw new InvalidOperationException(CaseId + " differed between two reads");
        RejectMachinePaths(first.Calls, reference);
        RejectMachinePaths(first.Result, reference);
        RejectMachinePaths(first.Native, reference);
        RejectMachinePaths(first.CharacterCalls, reference);
        RejectMachinePaths(first.CharacterResult, reference);
        RejectMachinePaths(first.Stats, reference);
        RejectMachinePaths(first.Analytic, reference);

        var manifest = new StringBuilder();
        manifest.Append(FormatLine).Append('\n');
        manifest.Append("revision ").Append(revision).Append('\n');
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs\n");
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/JoltCharacter.cs\n");
        manifest.Append("compile OpenSim.Region.Framework/Scenes/SimStatsReporter.cs\n");
        manifest.Append('\n');
        manifest.Append(CaseId).Append('\n');
        manifest.Append("character\n");
        manifest.Append("stats\n");
        manifest.Append("analytic\n");

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
        native.Append(NativeProof("after_calls"));
        native.Append("witness s_foundationRefCount stays 0, so JoltPhysicsBackend.Initialize did not call Foundation.Init\n");
        native.Append("witness joltc is not mapped, so Foundation.Init did not call JPH_Init\n");
        return new Captured(calls.ToString(), result.ToString(), native.ToString(), character.Calls, character.Result, stats, analytic);
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

    readonly record struct Captured(string Calls, string Result, string Native, string CharacterCalls, string CharacterResult, string Stats, string Analytic);

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

}
