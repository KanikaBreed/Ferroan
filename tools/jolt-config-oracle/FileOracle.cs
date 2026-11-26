// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls pinned JoltScene.NoteDilationSample (JoltScene.cs 2928-2942) and records
// the private dilation window. The scene is constructed and never initialised,
// so JoltNative stays unloaded and Foundation.Init is not called.

using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Text;
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
        WriteTree(scratch, revision, reference);
        CompareTrees(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("file-verify: " + CaseId + " matches NoteDilationSample");
    }

    static void WriteTree(string directory, string revision, string reference)
    {
        RequirePortableClock();
        Captured first = Capture();
        Captured second = Capture();
        if (first.Calls != second.Calls || first.Result != second.Result || first.Native != second.Native)
            throw new InvalidOperationException(CaseId + " differed between two reads");
        RejectMachinePaths(first.Calls, reference);
        RejectMachinePaths(first.Result, reference);
        RejectMachinePaths(first.Native, reference);

        var manifest = new StringBuilder();
        manifest.Append(FormatLine).Append('\n');
        manifest.Append("revision ").Append(revision).Append('\n');
        manifest.Append("compile OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs\n");
        manifest.Append('\n');
        manifest.Append(CaseId).Append('\n');

        string caseDir = Path.Combine(directory, CaseId);
        Directory.CreateDirectory(caseDir);
        File.WriteAllText(Path.Combine(caseDir, "calls.txt"), first.Calls);
        File.WriteAllText(Path.Combine(caseDir, "result.txt"), first.Result);
        File.WriteAllText(Path.Combine(caseDir, "native.txt"), first.Native);
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), manifest.ToString());
    }

    static Captured Capture()
    {
        MethodInfo method = SampleMethod();
        var native = new StringBuilder();
        native.Append(FormatLine).Append('\n');
        native.Append(NativeProof("after_type_init"));
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
        native.Append(NativeProof("after_calls"));
        native.Append("witness s_foundationRefCount stays 0, so JoltPhysicsBackend.Initialize did not call Foundation.Init\n");
        native.Append("witness joltc is not mapped, so Foundation.Init did not call JPH_Init\n");
        return new Captured(calls.ToString(), result.ToString(), native.ToString());
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

    readonly record struct Captured(string Calls, string Result, string Native);
}
