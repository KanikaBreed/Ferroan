// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// In-process Harness.Run for the pinned `drop` scenario, on the deterministic linux-x64 joltc
// the pin records (origin Rebuilt, joltc 886e0886, Jolt e77f1755).
// Samples keep the harness floats. ToCsv is not the trace.
// The same run writes analytic-envelope.txt: mass, one ray, and the dynamic
// free-flight position and velocity after each of the first two heartbeats.
// The pinned checkout is not modified.

using System.Globalization;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.PhysicsModules.Jolt;
using OpenSim.Region.PhysicsModules.Jolt.Backend;
using OpenSim.Region.PhysicsModules.Jolt.Harness;
using OpenSim.Region.PhysicsModules.SharedBase;

namespace Ferroan.JoltConfigOracle;

sealed class NativeSkip : Exception
{
    internal NativeSkip(string message) : base(message) { }
}

static partial class RecordOracle
{
    const string FormatLine = "format 1";
    const string ScenarioName = "drop";
    const long AnchorBytes = 2701768;
    const string AnchorSha = "27EFC48C813E75294DA11EAF22618BC1E0A5426A50EDDE7870EF4A60C8E4288B";
    const string AnchorFile = "libjoltc.so";
    const string AnchorRid = "linux-x64";
    const JoltNativeOrigin AnchorOrigin = JoltNativeOrigin.Rebuilt;
    // The harness baseline folder for the anchor (BaselineCheck.KeyFor of its record).
    const string Baseline = "det-886e0886-linux-x64";
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
        Console.WriteLine("generate: " + ScenarioName + " recording on " + Baseline);
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
        manifest.Append("analytic-envelope").Append('\n');
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), manifest.ToString());
        string caseDir = Path.Combine(directory, ScenarioName);
        Directory.CreateDirectory(caseDir);
        foreach (KeyValuePair<string, string> pair in first)
            File.WriteAllText(Path.Combine(caseDir, pair.Key), pair.Value);
        string envelope = EnvelopeText();
        RejectMachinePaths(envelope, reference);
        File.WriteAllText(Path.Combine(directory, "analytic-envelope.txt"), envelope);
    }

    // Mass and one horizontal ray for each analytic shape, on the stock rate-0 harness.
    // Mass is read in Setup, as MassReadoutTests.cs 78-82. The ray is the second input,
    // after one Simulate, through JoltScene.RaycastWorld (JoltScene.cs 2274-2280).
    // A third heartbeat exists so the second post-step pose can be read on the next
    // input, before that heartbeat's Simulate. The compared flight is the first two.
    // Two reads must match. The drop files are written before this and are not changed.
    const float EnvelopeDuration = 2.5f / 11f;
    const int FlightSteps = 2;

    static string EnvelopeText()
    {
        string first = CaptureEnvelope();
        string second = CaptureEnvelope();
        if (!string.Equals(first, second, StringComparison.Ordinal))
            throw new InvalidOperationException("analytic envelope differed between two reads");
        return first;
    }

    static string CaptureEnvelope()
    {
        var specs = new (string Name, bool Physical, uint Local, PrimitiveBaseShape Shape, Vector3 Size, Vector3 Position)[]
        {
            ("box-static", false, 4101, PrimitiveBaseShape.CreateBox(), new Vector3(1f, 1f, 1f), new Vector3(64f, 64f, 45f)),
            ("box-dynamic", true, 4102, PrimitiveBaseShape.CreateBox(), new Vector3(1f, 1f, 1f), new Vector3(80f, 80f, 45f)),
            ("sphere-static", false, 4103, PrimitiveBaseShape.CreateSphere(), new Vector3(2f, 2f, 2f), new Vector3(96f, 96f, 45f)),
            ("sphere-dynamic", true, 4104, PrimitiveBaseShape.CreateSphere(), new Vector3(2f, 2f, 2f), new Vector3(112f, 112f, 45f)),
            ("cylinder-static", false, 4105, CircleStraight(), new Vector3(1f, 1f, 2f), new Vector3(128f, 48f, 45f)),
            ("cylinder-dynamic", true, 4106, CircleStraight(), new Vector3(1f, 1f, 2f), new Vector3(144f, 48f, 45f)),
        };
        var shapes = new List<string>();
        var placed = new List<(string Name, bool Physical, uint Local, PhysicsActor Actor)>();
        var rays = new List<string>();
        var flights = new List<string>();
        float gravityX = float.NaN, gravityY = float.NaN, gravityZ = float.NaN;
        float linearDamping = float.NaN, heartbeat = float.NaN;
        int collisionSteps = -1;
        var scenario = new Scenario
        {
            Name = "analytic-envelope",
            DefaultDuration = _ => EnvelopeDuration,
            Setup = run =>
            {
                object scene = run.PhysicsScene;
                object gravity = Member(scene, "DefaultGravity");
                gravityX = Component(gravity, "X");
                gravityY = Component(gravity, "Y");
                gravityZ = Component(gravity, "Z");
                linearDamping = (float)Member(scene, "PrimLinearDamping");
                collisionSteps = (int)Member(scene, "CollisionSteps");
                heartbeat = (float)run.Dt;
                if (collisionSteps < 1 || !float.IsFinite(gravityZ) || !float.IsFinite(linearDamping) || heartbeat <= 0f)
                    throw new InvalidOperationException("analytic envelope is missing gravity, damping or the solver step");
                foreach (var spec in specs)
                {
                    PhysicsActor actor = run.AddPart(spec.Shape, spec.Size, spec.Position, Quaternion.Identity, spec.Physical, spec.Local);
                    if (spec.Name == "box-static")
                        run.Actor = actor;
                    placed.Add((spec.Name, spec.Physical, spec.Local, actor));
                    shapes.Add(string.Join(' ',
                        "shape", spec.Name,
                        "physical", spec.Physical ? "1" : "0",
                        "local", spec.Local.ToString(CultureInfo.InvariantCulture),
                        "pos", F(actor.Position.X), F(actor.Position.Y), F(actor.Position.Z),
                        "size", F(actor.Size.X), F(actor.Size.Y), F(actor.Size.Z),
                        "mass", F(actor.Mass)));
                }
            },
            Input = run =>
            {
                if (run.Heartbeats >= 1 && run.Heartbeats <= FlightSteps)
                {
                    foreach (var spec in placed)
                    {
                        if (!spec.Physical)
                            continue;
                        Vector3 position = spec.Actor.Position;
                        Vector3 velocity = spec.Actor.Velocity;
                        flights.Add(string.Join(' ',
                            "flight", spec.Name,
                            "step", run.Heartbeats.ToString(CultureInfo.InvariantCulture),
                            "pos", F(position.X), F(position.Y), F(position.Z),
                            "vel", F(velocity.X), F(velocity.Y), F(velocity.Z)));
                    }
                }
                if (run.Heartbeats != 1)
                    return;
                foreach (var spec in placed)
                {
                    if (spec.Physical)
                        continue;
                    Vector3 start = spec.Actor.Position - new Vector3(5f, 0f, 0f);
                    List<ContactResult> hits = run.PhysicsScene.RaycastWorld(start, Vector3.UnitX, 10f, 1);
                    if (hits.Count != 1 || hits[0].ConsumerID != spec.Local)
                        throw new InvalidOperationException(
                            spec.Name + " ray hits " + hits.Count.ToString(CultureInfo.InvariantCulture)
                            + (hits.Count == 0 ? "" : " consumer " + hits[0].ConsumerID.ToString(CultureInfo.InvariantCulture)));
                    ContactResult hit = hits[0];
                    rays.Add(string.Join(' ',
                        "ray", spec.Name,
                        "heartbeats", run.Heartbeats.ToString(CultureInfo.InvariantCulture),
                        "start", F(start.X), F(start.Y), F(start.Z),
                        "dir", F(Vector3.UnitX.X), F(Vector3.UnitX.Y), F(Vector3.UnitX.Z),
                        "length", F(10f),
                        "hits", "1",
                        "consumer", hit.ConsumerID.ToString(CultureInfo.InvariantCulture),
                        "pos", F(hit.Pos.X), F(hit.Pos.Y), F(hit.Pos.Z),
                        "normal", F(hit.Normal.X), F(hit.Normal.Y), F(hit.Normal.Z),
                        "depth", F(hit.Depth)));
                }
            },
        };
        var options = new HarnessOptions { PhysicsRateHz = 0, Duration = EnvelopeDuration };
        RunResult result = Harness.Run(scenario, options);
        if (result.PhysicsRateHz != 0)
            throw new InvalidOperationException("analytic envelope physics rate was " + result.PhysicsRateHz.ToString("R", CultureInfo.InvariantCulture));
        if (result.Samples.Count != 3)
            throw new InvalidOperationException("analytic envelope samples were " + result.Samples.Count.ToString(CultureInfo.InvariantCulture));
        if (shapes.Count != 6 || rays.Count != 3 || flights.Count != FlightSteps * 3)
            throw new InvalidOperationException("analytic envelope recorded " + shapes.Count + " shapes, " + rays.Count + " rays and " + flights.Count + " flights");
        var text = new StringBuilder();
        text.Append("format 1\n");
        text.Append("channel harness\n");
        text.Append("scenario analytic-envelope\n");
        text.Append("baseline ").Append(Baseline).Append('\n');
        text.Append("mass JoltPrim.Mass\n");
        text.Append("mass_source JoltPrim.cs 1228-1236\n");
        text.Append("mass_read MassReadoutTests.cs 78-82\n");
        text.Append("ray JoltScene.RaycastWorld\n");
        text.Append("ray_source JoltScene.cs 2274-2280\n");
        text.Append("extent PhysicsActor.Size\n");
        text.Append("rate_hz 11\n");
        text.Append("physics_rate_hz 0\n");
        text.Append("samples ").Append(result.Samples.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("reads 2\n");
        text.Append("flight_steps ").Append(FlightSteps.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("collision_steps ").Append(collisionSteps.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("gravity ").Append(F(gravityX)).Append(' ').Append(F(gravityY)).Append(' ').Append(F(gravityZ)).Append('\n');
        text.Append("linear_damping ").Append(F(linearDamping)).Append('\n');
        text.Append("heartbeat ").Append(F(heartbeat)).Append('\n');
        text.Append("position_tolerance_m 0.001\n");
        text.Append("velocity_tolerance_m_s 0.01\n");
        text.Append("mass_decimals 3\n");
        text.Append('\n');
        foreach (string line in shapes)
            text.Append(line).Append('\n');
        foreach (string line in rays)
            text.Append(line).Append('\n');
        foreach (string line in flights)
            text.Append(line).Append('\n');
        return text.ToString();
    }

    static object Member(object target, string name)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        Type type = target.GetType();
        PropertyInfo property = type.GetProperty(name, flags);
        if (property != null)
            return property.GetValue(target) ?? throw new InvalidOperationException(name + " is null");
        FieldInfo field = type.GetField(name, flags) ?? throw new InvalidOperationException(name + " is missing");
        return field.GetValue(target) ?? throw new InvalidOperationException(name + " is null");
    }

    static float Component(object vector, string name)
    {
        object value = Member(vector, name);
        return value is float number ? number : throw new InvalidOperationException(name + " is not a float");
    }

    static PrimitiveBaseShape CircleStraight()
    {
        PrimitiveBaseShape shape = PrimitiveBaseShape.CreateBox();
        shape.ProfileShape = ProfileShape.Circle;
        shape.PathCurve = (byte)Extrusion.Straight;
        return shape;
    }

    static Captured Capture(string revision) => Capture(revision, 0);

    static Captured Capture(string revision, double physicsRateHz)
    {
        NativeIdentity native = RequireAnchor();
        string cacheDir = Path.Combine(Path.GetTempPath(), "harness-sculpt-cache");
        if (Directory.Exists(cacheDir))
            Directory.Delete(cacheDir, true);
        var host = new Host();
        host.NoteFilesBefore(CacheFiles(cacheDir).Count);
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
        var options = new HarnessOptions();
        if (physicsRateHz > 0)
            options.PhysicsRateHz = physicsRateHz;
        RunResult result = Harness.Run(scenario, options);
        host.Finish(result, physicsRateHz);
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
        if (info.Build == null || info.Build.Origin != AnchorOrigin)
            throw new NativeSkip("the anchor's record is not the " + AnchorOrigin + " build");
        string key = BaselineCheck.KeyFor(info.Build);
        if (!string.Equals(key, Baseline, StringComparison.Ordinal))
            throw new NativeSkip("the anchor's baseline is " + (key ?? "none") + ", not " + Baseline);
        return new NativeIdentity(file.Name, file.Length, info.Sha256, info.Rid, info.Build.Origin.ToString());
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
            text.Append(" awake_bodies=").Append(sample.Active.ToString(CultureInfo.InvariantCulture));
            text.Append(" awake=").Append(captured.Awake[i] ? "1" : "0");
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
        text.Append("cold ").Append(captured.FilesBefore == 0 ? "true" : "false").Append('\n');
        text.Append("files_before ").Append(captured.FilesBefore.ToString(CultureInfo.InvariantCulture)).Append('\n');
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
        text.Append("native_origin ").Append(captured.Native.Origin).Append('\n');
        text.Append("package ").Append(JoltNative.PackageId).Append('\n');
        text.Append("package_version ").Append(JoltNative.PackageVersion).Append('\n');
        text.Append("harness_commit ").Append(captured.Revision).Append('\n');
        text.Append("first_heartbeat ").Append(captured.FirstHeartbeat.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("first_t ").Append(D(captured.Samples[0].T)).Append('\n');
        text.Append("samples ").Append(captured.Samples.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("events ").Append(captured.Events.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("phases ").Append(captured.Phases.Count.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("provider_calls ").Append(captured.ProviderCalls.ToString(CultureInfo.InvariantCulture)).Append('\n');
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
        text.Append("material_pins ").Append(captured.MaterialPins ? "true" : "false").Append('\n');
        text.Append("use_defaults ").Append(captured.UseDefaults ? "true" : "false").Append('\n');
        text.Append("shipped_config ").Append(captured.ShippedConfig ? "true" : "false").Append('\n');
        text.Append("warm_up ").Append(captured.WarmUp ? "true" : "false").Append('\n');
        text.Append("simulator_prims ").Append(captured.SimulatorPrims ? "true" : "false").Append('\n');
        text.Append("region_mesher ").Append(captured.RegionMesher ? "true" : "false").Append('\n');
        text.Append("presets_before_setup ").Append(captured.PresetsBeforeSetup).Append('\n');
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
        int _providerCalls;
        int _filesBefore;
        int _firstHeartbeat;
        bool _sawHeartbeat;
        bool _materialPins;
        bool _useDefaults;
        bool _shippedConfig;
        bool _warmUp;
        bool _simulatorPrims;
        bool _regionMesher;
        string _mesherType = "";
        string _presetsBeforeSetup = "";
        readonly List<bool> _awake = new();

        internal void NoteFilesBefore(int count) => _filesBefore = count;
        Func<DateTime> _clock;
        readonly List<string> _phases = new();
        CollisionWatch _watch;
        bool _subscribed;
        PhysicsActor _actor;

        internal Action<Run> Setup(Action<Run> source) => run =>
        {
            HarnessOptions options = run.Options;
            _presetsBeforeSetup = options.VehicleRoute == VehicleRoute.None
                && options.VehicleParams.Count == 0
                && options.VehicleFlags.Count == 0
                ? "empty_scene"
                : options.VehicleRoute.ToString();
            _useDefaults = options.UseDefaults;
            _shippedConfig = options.ShippedConfig;
            _warmUp = options.WarmUpStep;
            _simulatorPrims = options.SimulatorPrims;
            _materialPins = MaterialPins.Enabled;
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
            _regionMesher = run.Mesher != null;
            _mesherType = run.Mesher == null ? "" : run.Mesher.GetType().FullName ?? "";
            (_effectiveRate, _effectiveWarning, _effectiveNote) = Effective(scene, (float)run.Dt);
            _dt = run.Dt;
            run.Cleanups.Add(() => OnCleanup(run));
        };

        internal Action<Run> Input(Action<Run> source) => run =>
        {
            // The previous heartbeat's step has already been read. The next
            // input has not moved the body yet, so this is that sample's awake flag.
            if (_sawHeartbeat)
                _awake.Add(BodyAwake());
            source?.Invoke(run);
            _heartbeat++;
            if (!_sawHeartbeat)
            {
                _firstHeartbeat = _heartbeat;
                _sawHeartbeat = true;
            }
            JoltScene scene = Scene(run);
            _marks.Add(StepCount(scene));
            _substepMarks.Add(SubstepTotal(scene));
            _inputTicks.Add(_clock().Ticks);
            _stepTicks.Add(null);
        };

        internal void Finish(RunResult result, double expectedPhysicsRate)
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
            if (result.PhysicsRateHz != expectedPhysicsRate)
            {
                if (expectedPhysicsRate == 0.0)
                    throw new InvalidOperationException("drop physics rate was not 0");
                throw new InvalidOperationException(
                    "drop physics rate was " + result.PhysicsRateHz.ToString("R", CultureInfo.InvariantCulture)
                    + ", not " + expectedPhysicsRate.ToString("R", CultureInfo.InvariantCulture));
            }
            if (_awake.Count != result.Samples.Count)
                throw new InvalidOperationException("awake flags and samples differ");
        }

        void OnCleanup(Run run)
        {
            _awake.Add(BodyAwake());
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
                ProviderCalls = _providerCalls,
                Awake = _awake,
                FilesBefore = _filesBefore,
                FirstHeartbeat = _firstHeartbeat,
                MaterialPins = _materialPins,
                UseDefaults = _useDefaults,
                ShippedConfig = _shippedConfig,
                WarmUp = _warmUp,
                SimulatorPrims = _simulatorPrims,
                RegionMesher = _regionMesher,
                MesherType = _mesherType,
                PresetsBeforeSetup = _presetsBeforeSetup,
                CacheFiles = cacheFiles,
                CacheHash = cacheHash,
            };
        }

        bool BodyAwake()
        {
            object backend = Backend(_actor);
            object body = _actor.GetType().GetField("_body", Inst).GetValue(_actor);
            return (bool)backend.GetType().GetMethod("IsBodyAwake").Invoke(backend, new object[] { body });
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
            var wrapper = new RecordingStepForceProvider
            {
                Inner = original,
                OnStock = () => Interlocked.Increment(ref _providerCalls),
            };
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
    internal Action OnStock;

    public void OnStep(in StepBody body, in StepInfo step)
    {
        Interlocked.Increment(ref Calls);
        if (Inner == null)
            return;
        Inner.OnStep(in body, in step);
        Interlocked.Increment(ref Forwarded);
        if (Inner is not RecordingStepForceProvider)
            OnStock?.Invoke();
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
    internal int ProviderCalls;
    internal List<bool> Awake;
    internal int FilesBefore;
    internal int FirstHeartbeat;
    internal bool MaterialPins;
    internal bool UseDefaults;
    internal bool ShippedConfig;
    internal bool WarmUp;
    internal bool SimulatorPrims;
    internal bool RegionMesher;
    internal string MesherType;
    internal string PresetsBeforeSetup;
    internal int CacheFiles;
    internal string CacheHash;
}

readonly record struct NativeIdentity(string File, long Bytes, string Sha256, string Rid, string Origin);
