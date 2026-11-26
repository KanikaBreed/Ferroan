// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// In-process Harness.Run for the stock Pin8 `drop` scenario.
// Samples keep the harness floats. ToCsv is not the trace.
// The pinned checkout is not modified.

using System.Globalization;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using OpenMetaverse;
using OpenSim.Region.PhysicsModules.Jolt;
using OpenSim.Region.PhysicsModules.Jolt.Backend;
using OpenSim.Region.PhysicsModules.Jolt.Harness;
using OpenSim.Region.PhysicsModules.SharedBase;

namespace Ferroan.JoltConfigOracle;

static class Program
{
    static int Main(string[] args)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        Environment.SetEnvironmentVariable(HarnessOptions.PhysicsRateVariable, null);
        if (args.Length != 6
            || args[2] != "--revision" || args[4] != "--reference"
            || (args[0] != "generate" && args[0] != "verify"))
        {
            Console.Error.WriteLine("usage: JoltRecordOracle generate|verify <directory> --revision <40 hex> --reference <path>");
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
                RecordOracle.Generate(directory, revision, reference);
            else
                RecordOracle.Verify(directory, revision, reference);
        }
        catch (NativeSkip skip)
        {
            Console.Error.WriteLine("SKIPPED: " + skip.Message);
            return 3;
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception);
            return 1;
        }
        return 0;
    }
}

sealed class NativeSkip : Exception
{
    internal NativeSkip(string message) : base(message) { }
}

static class RecordOracle
{
    const string FormatLine = "format 1";
    const string ScenarioName = "drop";
    const long AnchorBytes = 2046568;
    const string AnchorSha = "5FC051708BDD05031A816796612A2F87E17ED32CC198F195A43B2305B3D990FD";
    const string AnchorFile = "libjoltc.so";
    const string AnchorRid = "linux-x64";
    const string Baseline = "pin8-linux-x86_64";
    const int SubscribeMs = 100;

    const BindingFlags Inst = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
    const BindingFlags Stat = BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic;

    internal static void Generate(string directory, string revision, string reference)
    {
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        try
        {
            WriteTree(scratch, revision, reference);
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
        Console.WriteLine("generate: " + ScenarioName + " stock recording");
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
            RequireRejected(scratch, directory, ScenarioName + "/clock.txt", ScenarioName + "/negative-clock.txt");
            RequireRejected(scratch, directory, ScenarioName + "/events.txt", ScenarioName + "/negative-collider.txt");
        }
        finally
        {
            if (Directory.Exists(scratch))
                Directory.Delete(scratch, true);
        }
        Console.WriteLine("verify: " + ScenarioName + " matches Harness.Run");
    }

    static void RequireRejected(string scratch, string directory, string channel, string negative)
    {
        string channelPath = Path.Combine(scratch, channel);
        string original = File.ReadAllText(channelPath);
        File.WriteAllText(channelPath, File.ReadAllText(Path.Combine(scratch, negative)));
        try
        {
            try
            {
                CompareTrees(scratch, directory);
            }
            catch (InvalidOperationException rejected)
            {
                if (rejected.Message.Contains(channel, StringComparison.Ordinal))
                    return;
                throw;
            }
            throw new InvalidOperationException(negative + " was accepted as " + channel);
        }
        finally
        {
            File.WriteAllText(channelPath, original);
        }
    }

    static void WriteTree(string directory, string revision, string reference)
    {
        Captured captured = Capture(revision);
        Dictionary<string, string> first = Encode(captured);
        Dictionary<string, string> second = Encode(captured);
        if (first.Count != second.Count)
            throw new InvalidOperationException("recording encode was not stable");
        foreach (KeyValuePair<string, string> pair in first)
        {
            if (!second.TryGetValue(pair.Key, out string again) || again != pair.Value)
                throw new InvalidOperationException(pair.Key + " differed between two encodes");
            RejectMachinePaths(pair.Value, reference);
        }
        var manifest = new StringBuilder();
        manifest.Append(FormatLine).Append('\n');
        manifest.Append("revision ").Append(revision).Append('\n');
        manifest.Append("baseline ").Append(Baseline).Append('\n');
        manifest.Append("scenario ").Append(ScenarioName).Append('\n');
        manifest.Append('\n');
        manifest.Append(ScenarioName).Append('\n');
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), manifest.ToString());
        string caseDir = Path.Combine(directory, ScenarioName);
        Directory.CreateDirectory(caseDir);
        foreach (KeyValuePair<string, string> pair in first)
            File.WriteAllText(Path.Combine(caseDir, pair.Key), pair.Value);
    }

    static Captured Capture(string revision)
    {
        NativeIdentity native = RequireAnchor();
        string cacheDir = Path.Combine(Path.GetTempPath(), "harness-sculpt-cache");
        if (Directory.Exists(cacheDir))
            Directory.Delete(cacheDir, true);
        var host = new Host();
        Scenario source = Harness.Find(ScenarioName);
        var scenario = new Scenario
        {
            Name = source.Name,
            Description = source.Description,
            Slopes = source.Slopes,
            UsesSlope = source.UsesSlope,
            DefaultDuration = source.DefaultDuration,
            DefaultHold = source.DefaultHold,
            World = source.World,
            Setup = host.Setup(source.Setup),
            Input = host.Input(source.Input),
            Steady = source.Steady,
            SteadyFromPath = source.SteadyFromPath,
            Gap = source.Gap,
            PassedThrough = source.PassedThrough,
            UsesShot = source.UsesShot,
            AllowsTerrainFile = source.AllowsTerrainFile,
        };
        RunResult result = Harness.Run(scenario, new HarnessOptions());
        host.Finish(result);
        ConfirmLoaded(native);
        int cacheAfter = CacheFiles(cacheDir).Count;
        string cacheHash = HashBytes(CacheBytes(cacheDir));
        if (Directory.Exists(cacheDir))
            Directory.Delete(cacheDir, true);
        return host.ToCaptured(result, revision, native, cacheAfter, cacheHash);
    }

    static NativeIdentity RequireAnchor()
    {
        string rid = JoltNative.CurrentRid();
        if (!string.Equals(rid, AnchorRid, StringComparison.Ordinal))
            throw new NativeSkip("this process is " + rid + "; the anchor is " + AnchorRid);
        JoltNativeInfo info;
        try
        {
            info = JoltNative.Check(JoltNative.DefaultBaseDirectory(), rid, false);
        }
        catch (JoltNativeException exception)
        {
            throw new NativeSkip(exception.Message);
        }
        var file = new FileInfo(info.Path);
        if (!string.Equals(file.Name, AnchorFile, StringComparison.Ordinal)
            || file.Length != AnchorBytes
            || !string.Equals(info.Sha256, AnchorSha, StringComparison.OrdinalIgnoreCase))
        {
            throw new NativeSkip(
                file.Name + " is " + file.Length + " bytes sha256 " + info.Sha256
                + "; the anchor is " + AnchorFile + " " + AnchorBytes + " bytes sha256 " + AnchorSha);
        }
        return new NativeIdentity(file.Name, file.Length, info.Sha256, info.Rid);
    }

    static void ConfirmLoaded(NativeIdentity native)
    {
        object loaded = typeof(JoltNative).GetField("s_loaded", Stat).GetValue(null);
        if (loaded == null)
            throw new InvalidOperationException("Harness.Run did not load joltc");
        string sha = (string)loaded.GetType().GetProperty("Sha256").GetValue(loaded);
        string rid = (string)loaded.GetType().GetProperty("Rid").GetValue(loaded);
        string path = (string)loaded.GetType().GetProperty("Path").GetValue(loaded);
        if (!string.Equals(sha, native.Sha256, StringComparison.OrdinalIgnoreCase)
            || !string.Equals(rid, native.Rid, StringComparison.Ordinal)
            || !string.Equals(Path.GetFileName(path), native.File, StringComparison.Ordinal))
            throw new NativeSkip("the loaded native is not the anchor that was checked");
    }

    static Dictionary<string, string> Encode(Captured captured)
    {
        string samples = SamplesText(captured);
        string events = EventsText(captured);
        string providers = ProvidersText(captured);
        string clock = ClockText(captured);
        string config = captured.Config;
        string materials = captured.Materials;
        string assets = AssetsText();
        string cache = CacheText(captured);
        var files = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["samples.txt"] = samples,
            ["events.txt"] = events,
            ["providers.txt"] = providers,
            ["clock.txt"] = clock,
            ["config.txt"] = config,
            ["materials.txt"] = materials,
            ["assets.txt"] = assets,
            ["cache.txt"] = cache,
        };
        files["provenance.txt"] = ProvenanceText(captured, files);
        files["negative-clock.txt"] = MutateFirstNumber(clock, "input_ticks=", 1);
        files["negative-collider.txt"] = MutateFirstNumber(events, "collider=", 1);
        if (files["negative-clock.txt"] == clock)
            throw new InvalidOperationException("wrong-clock negative did not change the clock");
        if (files["negative-collider.txt"] == events)
            throw new InvalidOperationException("wrong-collider negative did not change a collider");
        return files;
    }

    static string SamplesText(Captured captured)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        for (int i = 0; i < captured.Samples.Count; i++)
        {
            Sample sample = captured.Samples[i];
            text.Append("i=").Append(i.ToString(CultureInfo.InvariantCulture));
            text.Append(" steps=").Append(captured.Steps[i].ToString(CultureInfo.InvariantCulture));
            text.Append(" t=").Append(D(sample.T));
            text.Append(" px=").Append(F(sample.Position.X));
            text.Append(" py=").Append(F(sample.Position.Y));
            text.Append(" pz=").Append(F(sample.Position.Z));
            text.Append(" vx=").Append(F(sample.Velocity.X));
            text.Append(" vy=").Append(F(sample.Velocity.Y));
            text.Append(" vz=").Append(F(sample.Velocity.Z));
            text.Append(" wx=").Append(F(sample.AngularVelocity.X));
            text.Append(" wy=").Append(F(sample.AngularVelocity.Y));
            text.Append(" wz=").Append(F(sample.AngularVelocity.Z));
            text.Append(" qx=").Append(F(sample.Rotation.X));
            text.Append(" qy=").Append(F(sample.Rotation.Y));
            text.Append(" qz=").Append(F(sample.Rotation.Z));
            text.Append(" qw=").Append(F(sample.Rotation.W));
            text.Append(" touching=").Append(sample.Touching ? "1" : "0");
            text.Append(" active=").Append(sample.Active.ToString(CultureInfo.InvariantCulture));
            text.Append(" ox=").Append(F(sample.Other.X));
            text.Append(" oy=").Append(F(sample.Other.Y));
            text.Append(" oz=").Append(F(sample.Other.Z));
            text.Append(" ovx=").Append(F(sample.OtherVelocity.X));
            text.Append(" ovy=").Append(F(sample.OtherVelocity.Y));
            text.Append(" ovz=").Append(F(sample.OtherVelocity.Z));
            text.Append(" oqx=").Append(F(sample.OtherRotation.X));
            text.Append(" oqy=").Append(F(sample.OtherRotation.Y));
            text.Append(" oqz=").Append(F(sample.OtherRotation.Z));
            text.Append(" oqw=").Append(F(sample.OtherRotation.W));
            text.Append(" tilt=").Append(F(sample.Tilt));
            text.Append(" height=").Append(F(sample.Height));
            text.Append('\n');
        }
        return text.ToString();
    }

    static string EventsText(Captured captured)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        foreach (string line in captured.Events)
            text.Append(line).Append('\n');
        foreach (string line in captured.Phases)
            text.Append(line).Append('\n');
        return text.ToString();
    }

    static string ProvidersText(Captured captured)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("seam RecordingStepForceProvider\n");
        text.Append("stock_attached ").Append(captured.StockProviders.ToString(CultureInfo.InvariantCulture)).Append('\n');
        foreach (string name in captured.ProviderNames)
            text.Append("provider ").Append(name).Append('\n');
        return text.ToString();
    }

    static string ClockText(Captured captured)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        for (int i = 0; i < captured.InputTicks.Count; i++)
        {
            text.Append("i=").Append(i.ToString(CultureInfo.InvariantCulture));
            text.Append(" input_ticks=").Append(captured.InputTicks[i].ToString(CultureInfo.InvariantCulture));
            text.Append(" step_ticks=");
            text.Append(captured.StepTicks[i] is long ticks
                ? ticks.ToString(CultureInfo.InvariantCulture)
                : "unobserved");
            text.Append('\n');
        }
        return text.ToString();
    }

    static string AssetsText()
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("bytes 0\n");
        text.Append("sha256 ").Append(HashBytes(Array.Empty<byte>())).Append('\n');
        text.Append("kind analytic-box\n");
        return text.ToString();
    }

    static string CacheText(Captured captured)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("cold true\n");
        text.Append("files_before 0\n");
        text.Append("files_after ").Append(captured.CacheFiles.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("content_sha256 ").Append(captured.CacheHash).Append('\n');
        return text.ToString();
    }

    static string ProvenanceText(Captured captured, Dictionary<string, string> files)
    {
        var text = new StringBuilder();
        text.Append(FormatLine).Append('\n');
        text.Append("scenario ").Append(ScenarioName).Append('\n');
        text.Append("revision ").Append(captured.Revision).Append('\n');
        text.Append("baseline ").Append(Baseline).Append('\n');
        text.Append("heartbeat 1/11\n");
        text.Append("rate_hz ").Append(captured.RateHz.ToString("R", CultureInfo.InvariantCulture)).Append('\n');
        text.Append("physics_rate_hz ").Append(captured.PhysicsRateHz.ToString("R", CultureInfo.InvariantCulture)).Append('\n');
        text.Append("substepping ").Append(captured.Substepping ? "true" : "false").Append('\n');
        text.Append("step_source ").Append(captured.Substepping ? "substeps" : "one_step").Append('\n');
        text.Append("warmup_steps ").Append(captured.WarmupSteps.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("subscribe_ms ").Append(SubscribeMs.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("native_file ").Append(captured.Native.File).Append('\n');
        text.Append("native_size ").Append(captured.Native.Bytes.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("native_sha256 ").Append(captured.Native.Sha256).Append('\n');
        text.Append("native_rid ").Append(captured.Native.Rid).Append('\n');
        text.Append("package ").Append(JoltNative.PackageId).Append('\n');
        text.Append("package_version ").Append(JoltNative.PackageVersion).Append('\n');
        text.Append("harness_commit ").Append(captured.Revision).Append('\n');
        text.Append("first_heartbeat 0\n");
        text.Append("first_t ").Append(D(captured.Samples[0].T)).Append('\n');
        text.Append("samples ").Append(captured.Samples.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("events ").Append(captured.Events.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("phases ").Append(captured.Phases.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("provider_calls 0\n");
        text.Append("stock_providers ").Append(captured.StockProviders.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("seam_calls ").Append(captured.SeamCalls.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("seam_forwarded ").Append(captured.SeamForwarded.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("dt_f64 ").Append(D(captured.Dt)).Append('\n');
        text.Append("simulate_dt_f32 ").Append(F(captured.SimulateDt)).Append('\n');
        text.Append("effective_step_rate ").Append(F(captured.EffectiveRate)).Append('\n');
        text.Append("effective_warning ").Append(captured.EffectiveWarning ?? "none").Append('\n');
        text.Append("effective_note ").Append(captured.EffectiveNote ?? "none").Append('\n');
        text.Append("body_local ").Append(captured.BodyLocal.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("body_handle ").Append(captured.BodyHandle.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("material_pins false\n");
        text.Append("use_defaults false\n");
        text.Append("shipped_config true\n");
        text.Append("warm_up true\n");
        text.Append("simulator_prims true\n");
        text.Append("region_mesher true\n");
        text.Append("presets_before_setup empty_scene\n");
        foreach (string name in new[] { "samples", "events", "providers", "clock", "config", "materials", "assets", "cache" })
        {
            text.Append("channel ").Append(name).Append(' ');
            text.Append(HashText(files[name + ".txt"])).Append('\n');
        }
        text.Append("negative clock\n");
        text.Append("negative collider\n");
        return text.ToString();
    }

    static string MutateFirstNumber(string text, string key, long delta)
    {
        int at = text.IndexOf(key, StringComparison.Ordinal);
        if (at < 0)
            throw new InvalidOperationException("no " + key + " to mutate");
        int start = at + key.Length;
        int end = start;
        while (end < text.Length && text[end] >= '0' && text[end] <= '9')
            end++;
        if (end == start)
            throw new InvalidOperationException(key + " is not a number");
        long value = long.Parse(text[start..end], CultureInfo.InvariantCulture) + delta;
        return text[..start] + value.ToString(CultureInfo.InvariantCulture) + text[end..];
    }

    static void RejectMachinePaths(string text, string reference)
    {
        if (text.Contains(reference, StringComparison.Ordinal) || text.Contains("/home/", StringComparison.Ordinal)
            || text.Contains("\\Users\\", StringComparison.Ordinal) || text.Contains("/mnt/", StringComparison.Ordinal)
            || text.Contains("/tmp/", StringComparison.Ordinal))
            throw new InvalidOperationException("recording contains a machine path");
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

    static string F(float value) => BitConverter.SingleToInt32Bits(value).ToString("X8");
    static string D(double value) => BitConverter.DoubleToInt64Bits(value).ToString("X16");

    static string HashText(string text) => HashBytes(Encoding.UTF8.GetBytes(text));

    static string HashBytes(byte[] bytes)
    {
        return Convert.ToHexString(SHA256.HashData(bytes));
    }

    static List<string> CacheFiles(string cacheDir)
    {
        if (!Directory.Exists(cacheDir))
            return new List<string>();
        return Directory.EnumerateFiles(cacheDir, "*", SearchOption.AllDirectories)
            .Select(path => Path.GetRelativePath(cacheDir, path))
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToList();
    }

    static byte[] CacheBytes(string cacheDir)
    {
        var buffer = new MemoryStream();
        foreach (string relative in CacheFiles(cacheDir))
        {
            byte[] name = Encoding.UTF8.GetBytes(relative + "\n");
            buffer.Write(name);
            byte[] body = File.ReadAllBytes(Path.Combine(cacheDir, relative));
            buffer.Write(body);
        }
        return buffer.ToArray();
    }

    sealed class Host
    {
        readonly List<long> _marks = new();
        readonly List<long?> _substepMarks = new();
        readonly List<long> _inputTicks = new();
        readonly List<long?> _stepTicks = new();
        readonly List<string> _events = new();
        readonly HashSet<uint> _touching = new();
        bool _land;
        int _heartbeat = -1;
        long _warmupSteps;
        bool _substepping;
        int _stockProviders;
        readonly List<string> _providerNames = new();
        uint _bodyLocal;
        uint _bodyHandle;
        string _materials = "";
        string _config = "";
        float _effectiveRate;
        string _effectiveWarning;
        string _effectiveNote;
        double _dt;
        float _simulateDt;
        int _seamCalls;
        int _seamForwarded;
        Func<DateTime> _clock;
        readonly List<string> _phases = new();
        CollisionWatch _watch;
        bool _subscribed;
        PhysicsActor _actor;

        internal Action<Run> Setup(Action<Run> source) => run =>
        {
            JoltScene scene = Scene(run);
            _warmupSteps = StepCount(scene);
            _substepping = IsSubstepping(scene);
            source(run);
            _actor = run.Actor ?? throw new InvalidOperationException("drop created no actor");
            _bodyLocal = _actor.LocalID;
            _bodyHandle = BodyHandle(_actor);
            _clock = (Func<DateTime>)typeof(JoltScene).GetField("VehicleClock", Inst).GetValue(scene);
            if (_clock == null)
                throw new InvalidOperationException("the harness set no vehicle clock");
            _watch = new CollisionWatch(ScenarioName);
            _watch.Attach(_actor, run);
            _actor.SubscribeEvents(SubscribeMs);
            _actor.OnCollisionUpdate += OnUpdate;
            _subscribed = true;
            run.Watches.Add(_watch);
            _stockProviders = ProviderCount(Backend(_actor));
            _providerNames.AddRange(ProviderNames(Backend(_actor)));
            _materials = Materials(_actor);
            _config = ConfigText(scene);
            (_effectiveRate, _effectiveWarning, _effectiveNote) = Effective(scene, (float)run.Dt);
            _dt = run.Dt;
            run.Cleanups.Add(() => OnCleanup(run));
        };

        internal Action<Run> Input(Action<Run> source) => run =>
        {
            source?.Invoke(run);
            _heartbeat++;
            JoltScene scene = Scene(run);
            _marks.Add(StepCount(scene));
            _substepMarks.Add(SubstepTotal(scene));
            _inputTicks.Add(_clock().Ticks);
            _stepTicks.Add(null);
        };

        internal void Finish(RunResult result)
        {
            if (result.Samples.Count == 0)
                throw new InvalidOperationException("drop produced no samples");
            if (result.Samples.Count != _inputTicks.Count || _marks.Count != result.Samples.Count + 1)
                throw new InvalidOperationException("samples and heartbeats differ");
            if (_events.Count == 0)
                throw new InvalidOperationException("drop produced no collision event");
            if (!_events.Any(line => line.Contains("collider=0 ", StringComparison.Ordinal)))
                throw new InvalidOperationException("drop produced no terrain collider");
            if (Math.Abs(result.RateHz - 11.0) > 0.0)
                throw new InvalidOperationException("drop heartbeat was not 11 Hz");
            if (result.PhysicsRateHz != 0.0)
                throw new InvalidOperationException("drop physics rate was not 0");
        }

        void OnCleanup(Run run)
        {
            foreach ((double t, string ev, uint id) in _watch.Log)
                _phases.Add("phase t=" + D(t) + " event=" + ev.Replace(' ', '-') + " id=" + id.ToString(CultureInfo.InvariantCulture));
            JoltScene scene = Scene(run);
            long end = StepCount(scene);
            long? substepEnd = SubstepTotal(scene);
            _simulateDt = (float)typeof(JoltScene).GetField("LastTimeStep", Inst).GetValue(scene);
            _marks.Add(end);
            _substepMarks.Add(substepEnd);
            if (_subscribed)
                _actor.OnCollisionUpdate -= OnUpdate;
            ProveSeam(scene, _actor);
        }

        void OnUpdate(EventArgs args)
        {
            var update = (CollisionEventUpdate)args;
            long ticks = _clock().Ticks;
            if (_stepTicks[_heartbeat] is long seen && seen != ticks)
                throw new InvalidOperationException("the step clock changed during one heartbeat");
            _stepTicks[_heartbeat] = ticks;
            var ids = new List<uint>(update.m_objCollisionList.Keys);
            ids.Sort();
            var now = new HashSet<uint>(ids);
            bool land = now.Remove(0);
            foreach (uint id in now)
            {
                if (_touching.Add(id))
                    _events.Add(EventLine("start", id, update));
            }
            foreach (uint id in _touching.Where(id => id != 0 && !now.Contains(id)).ToArray())
            {
                _touching.Remove(id);
                _events.Add(EventLine("end", id, update));
            }
            if (land)
            {
                _events.Add(EventLine(_land ? "land" : "land-start", 0, update));
                _land = true;
            }
            else if (_land)
            {
                _events.Add(PhaseOnly("land-end", 0));
                _land = false;
            }
        }

        string EventLine(string phase, uint collider, CollisionEventUpdate update)
        {
            ContactPoint contact = default;
            bool have = update.m_objCollisionList != null && update.m_objCollisionList.TryGetValue(collider, out contact);
            var text = new StringBuilder();
            text.Append("raw i=").Append(_heartbeat.ToString(CultureInfo.InvariantCulture));
            text.Append(" actor=").Append(_bodyLocal.ToString(CultureInfo.InvariantCulture));
            text.Append(" throttle=").Append(_actor.ThrottleUpdates ? "1" : "0");
            text.Append(" previous=").Append(update.CollisionsOnPreviousFrame ? "1" : "0");
            text.Append(" phase=").Append(phase);
            text.Append(" collider=").Append(collider.ToString(CultureInfo.InvariantCulture));
            if (!have)
                contact = default;
            text.Append(" px=").Append(F(contact.Position.X));
            text.Append(" py=").Append(F(contact.Position.Y));
            text.Append(" pz=").Append(F(contact.Position.Z));
            text.Append(" nx=").Append(F(contact.SurfaceNormal.X));
            text.Append(" ny=").Append(F(contact.SurfaceNormal.Y));
            text.Append(" nz=").Append(F(contact.SurfaceNormal.Z));
            text.Append(" pen=").Append(F(contact.PenetrationDepth));
            text.Append(" speed=").Append(F(contact.RelativeSpeed));
            text.Append(" feet=").Append(contact.CharacterFeet ? "1" : "0");
            return text.ToString();
        }

        string PhaseOnly(string phase, uint collider)
        {
            return "raw i=" + _heartbeat.ToString(CultureInfo.InvariantCulture)
                + " actor=" + _bodyLocal.ToString(CultureInfo.InvariantCulture)
                + " throttle=" + (_actor.ThrottleUpdates ? "1" : "0")
                + " previous=0 phase=" + phase
                + " collider=" + collider.ToString(CultureInfo.InvariantCulture)
                + " px=" + F(0f) + " py=" + F(0f) + " pz=" + F(0f)
                + " nx=" + F(0f) + " ny=" + F(0f) + " nz=" + F(0f)
                + " pen=" + F(0f) + " speed=" + F(0f) + " feet=0";
        }

        internal Captured ToCaptured(RunResult result, string revision, NativeIdentity native, int cacheFiles, string cacheHash)
        {
            var steps = new List<long>();
            for (int i = 0; i < result.Samples.Count; i++)
            {
                long count = _substepping
                    ? _substepMarks[i + 1].Value - _substepMarks[i].Value
                    : _marks[i + 1] - _marks[i];
                if (count <= 0)
                    throw new InvalidOperationException("heartbeat " + i + " took no step");
                steps.Add(count);
            }
            return new Captured
            {
                Revision = revision,
                Native = native,
                Samples = result.Samples,
                Steps = steps,
                Events = _events,
                Phases = _phases,
                InputTicks = _inputTicks,
                StepTicks = _stepTicks,
                StockProviders = _stockProviders,
                ProviderNames = _providerNames,
                RateHz = result.RateHz,
                PhysicsRateHz = result.PhysicsRateHz,
                Substepping = _substepping,
                WarmupSteps = _substepping ? _substepMarks[0].Value : _warmupSteps,
                BodyLocal = _bodyLocal,
                BodyHandle = _bodyHandle,
                Materials = _materials,
                Config = _config,
                EffectiveRate = _effectiveRate,
                EffectiveWarning = _effectiveWarning,
                EffectiveNote = _effectiveNote,
                Dt = _dt,
                SimulateDt = _simulateDt,
                SeamCalls = _seamCalls,
                SeamForwarded = _seamForwarded,
                CacheFiles = cacheFiles,
                CacheHash = cacheHash,
            };
        }

        void ProveSeam(JoltScene scene, PhysicsActor actor)
        {
            // AttachStepForceProvider's disposable removes its own entry by reference
            // (JoltPhysicsBackend.StepProviders.cs). Replacing the entry leaves that
            // disposable holding an object the list no longer contains. The proof
            // therefore keeps the entry and swaps its Provider field.
            object backend = Backend(actor);
            BodyId body = (BodyId)actor.GetType().GetField("_body", Inst).GetValue(actor);
            object stock = CreateStockProvider(actor, backend);
            if (!(bool)stock.GetType().GetMethod("Arm", Inst).Invoke(stock, new object[]
            {
                body,
                new System.Numerics.Vector3(1f, 0f, 0f),
                System.Numerics.Vector3.Zero,
                1e6f,
                0f,
            }))
                throw new InvalidOperationException("the stock provider did not attach");
            if (ProviderCount(backend) != 1)
                throw new InvalidOperationException("the stock provider did not attach one entry");
            object entry = LastEntry(backend);
            FieldInfo providerField = entry.GetType().GetField("Provider");
            var original = (IStepForceProvider)providerField.GetValue(entry);
            if (!ReferenceEquals(original, stock))
                throw new InvalidOperationException("the attached provider was not the stock provider");
            var wrapper = new RecordingStepForceProvider { Inner = original };
            providerField.SetValue(entry, wrapper);
            // OnStep reads the provider list with Volatile.Read. The fence pairs with that acquire.
            Thread.MemoryBarrier();
            if (!ReferenceEquals(LastEntry(backend), entry))
                throw new InvalidOperationException("the seam replaced the provider entry");
            try
            {
                backend.GetType().GetMethod("ActivateBody").Invoke(backend, new object[] { body });
                scene.Simulate((float)_dt);
                if ((bool)stock.GetType().GetProperty("Armed", Inst).GetValue(stock))
                    throw new InvalidOperationException("the stock provider was not stepped");
                if (wrapper.Calls <= 0 || wrapper.Forwarded != wrapper.Calls)
                    throw new InvalidOperationException("the recording seam did not forward a provider call");
                long faults = (long)backend.GetType().GetProperty("StepForceProviderFaults").GetValue(backend);
                if (faults != 0)
                    throw new InvalidOperationException("the stock provider threw");
                _seamCalls = wrapper.Calls;
                _seamForwarded = wrapper.Forwarded;
            }
            finally
            {
                providerField.SetValue(entry, original);
                Thread.MemoryBarrier();
                stock.GetType().GetMethod("Detach", Inst).Invoke(stock, null);
            }
            if (ProviderCount(backend) != 0)
                throw new InvalidOperationException("detach left a step provider");
            if ((bool)backend.GetType().GetProperty("HasStepListener", Inst).GetValue(backend))
                throw new InvalidOperationException("detach left the step listener");
        }

        static object CreateStockProvider(PhysicsActor actor, object backend)
        {
            Type stockType = typeof(JoltScene).Assembly.GetType(
                "OpenSim.Region.PhysicsModules.Jolt.ScriptForceStepProvider", true);
            object stock = Activator.CreateInstance(
                stockType,
                BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic,
                null,
                new object[] { actor, backend },
                null);
            if (stock == null)
                throw new InvalidOperationException("the stock provider was not constructed");
            return stock;
        }

        static object LastEntry(object backend)
        {
            Array list = (Array)backend.GetType().GetField("_stepProviders", Inst).GetValue(backend);
            if (list == null || list.Length == 0)
                throw new InvalidOperationException("the seam proof attached no provider");
            return list.GetValue(list.Length - 1);
        }

        static JoltScene Scene(Run run) => (JoltScene)typeof(Run).GetField("Scene", Inst).GetValue(run);

        static long StepCount(JoltScene scene) => (long)typeof(JoltScene).GetProperty("StepCount", Inst).GetValue(scene);

        static bool IsSubstepping(JoltScene scene) => (bool)typeof(JoltScene).GetProperty("Substepping", Inst).GetValue(scene);

        static long? SubstepTotal(JoltScene scene)
        {
            object substeps = typeof(JoltScene).GetProperty("Substeps", Inst).GetValue(scene);
            if (substeps == null)
                return null;
            return (long)substeps.GetType().GetProperty("Steps", Inst).GetValue(substeps);
        }

        static object Backend(PhysicsActor actor) => actor.GetType().GetField("_backend", Inst).GetValue(actor);

        static uint BodyHandle(PhysicsActor actor)
        {
            object handle = actor.GetType().GetProperty("BodyHandle", Inst).GetValue(actor);
            return (uint)handle.GetType().GetField("Value").GetValue(handle);
        }

        static int ProviderCount(object backend)
        {
            Array list = (Array)backend.GetType().GetField("_stepProviders", Inst).GetValue(backend);
            return list.Length;
        }

        static IEnumerable<string> ProviderNames(object backend)
        {
            Array list = (Array)backend.GetType().GetField("_stepProviders", Inst).GetValue(backend);
            for (int i = 0; i < list.Length; i++)
            {
                object entry = list.GetValue(i);
                object provider = entry.GetType().GetField("Provider").GetValue(entry);
                yield return provider.GetType().Name;
            }
        }

        static string Materials(PhysicsActor actor)
        {
            var text = new StringBuilder();
            text.Append(FormatLine).Append('\n');
            text.Append("friction ").Append(F(actor.Friction)).Append('\n');
            text.Append("restitution ").Append(F(actor.Restitution)).Append('\n');
            text.Append("density ").Append(F(actor.Density)).Append('\n');
            text.Append("gravity ").Append(F(actor.GravModifier)).Append('\n');
            text.Append("buoyancy ").Append(F(actor.Buoyancy)).Append('\n');
            return text.ToString();
        }

        static string ConfigText(JoltScene scene)
        {
            object config = typeof(JoltScene).GetField("_joltConfig", Inst).GetValue(scene);
            var text = new StringBuilder();
            text.Append(FormatLine).Append('\n');
            foreach (FieldInfo field in config.GetType().GetFields(Inst).OrderBy(item => item.Name, StringComparer.Ordinal))
            {
                if (field.IsStatic || !field.IsPublic)
                    continue;
                text.Append(field.Name).Append(' ').Append(FieldText(field.GetValue(config))).Append('\n');
            }
            return text.ToString();
        }

        static (float rate, string warning, string note) Effective(JoltScene scene, float heartbeat)
        {
            object config = typeof(JoltScene).GetField("_joltConfig", Inst).GetValue(scene);
            MethodInfo method = config.GetType().GetMethod("EffectivePhysicsStepRate", Inst);
            object[] args = { heartbeat, null, null };
            object rate = method.Invoke(config, args);
            return ((float)rate, (string)args[1], (string)args[2]);
        }

        static string FieldText(object value)
        {
            if (value == null)
                return "null";
            return value switch
            {
                bool bit => bit ? "true" : "false",
                float number => F(number),
                double number => D(number),
                int number => number.ToString(CultureInfo.InvariantCulture),
                long number => number.ToString(CultureInfo.InvariantCulture),
                Enum name => name.ToString(),
                string text => text,
                _ => throw new InvalidOperationException("unexpected config field " + value.GetType().FullName),
            };
        }
    }
}

/// <summary>
/// Records one provider call and forwards it. OnStep does not allocate.
/// The drop attaches none. The proof attaches <c>ScriptForceStepProvider</c> after the
/// samples are stored and forwards one simulate to that stock provider.
/// </summary>
sealed class RecordingStepForceProvider : IStepForceProvider
{
    internal IStepForceProvider Inner;
    internal int Calls;
    internal int Forwarded;

    public void OnStep(in StepBody body, in StepInfo step)
    {
        Interlocked.Increment(ref Calls);
        if (Inner == null)
            return;
        Inner.OnStep(in body, in step);
        Interlocked.Increment(ref Forwarded);
    }
}

sealed class Captured
{
    internal string Revision;
    internal NativeIdentity Native;
    internal List<Sample> Samples;
    internal List<long> Steps;
    internal List<string> Events;
    internal List<string> Phases;
    internal List<long> InputTicks;
    internal List<long?> StepTicks;
    internal int StockProviders;
    internal List<string> ProviderNames;
    internal double RateHz;
    internal double PhysicsRateHz;
    internal bool Substepping;
    internal long WarmupSteps;
    internal uint BodyLocal;
    internal uint BodyHandle;
    internal string Materials;
    internal string Config;
    internal float EffectiveRate;
    internal string EffectiveWarning;
    internal string EffectiveNote;
    internal double Dt;
    internal float SimulateDt;
    internal int SeamCalls;
    internal int SeamForwarded;
    internal int CacheFiles;
    internal string CacheHash;
}

readonly record struct NativeIdentity(string File, long Bytes, string Sha256, string Rid);
