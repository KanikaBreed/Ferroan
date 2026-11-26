// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Reflection;
using System.Text;
using OpenSim.Region.PhysicsModules.Jolt;

/// <summary>
/// Step counts from the pinned <c>SubstepAccumulator</c>. The fixture stores
/// the class's own integers and IEEE bits. Nothing here is typed by hand.
/// </summary>
static class Program
{
    const string Source = "Source/OpenSim.Region.PhysicsModules.Jolt/SubstepAccumulator.cs";

    static int Main(string[] args)
    {
        var action = "";
        var revision = "";
        var fixture = "";
        for (var i = 0; i < args.Length; i++)
        {
            if (args[i] == "--revision" && i + 1 < args.Length)
                revision = args[++i];
            else if (args[i] == "generate" || args[i] == "verify")
                action = args[i];
            else if (!args[i].StartsWith("--"))
                fixture = args[i];
            else
            {
                Usage();
                return 2;
            }
        }
        if (action.Length == 0 || fixture.Length == 0 || !IsRevision(revision))
        {
            Usage();
            return 2;
        }
        if (!BitConverter.IsLittleEndian)
        {
            Console.Error.WriteLine("jolt: this oracle requires a little-endian host");
            return 1;
        }

        var json = Document(revision);
        if (action == "generate")
        {
            var directory = Path.GetDirectoryName(fixture);
            if (!string.IsNullOrEmpty(directory))
                Directory.CreateDirectory(directory);
            File.WriteAllText(fixture, json);
            Console.WriteLine("jolt: wrote " + fixture);
            return 0;
        }
        if (!File.Exists(fixture))
        {
            Console.Error.WriteLine("jolt: missing " + fixture);
            return 1;
        }
        var existing = File.ReadAllText(fixture);
        if (existing != json)
        {
            Console.Error.WriteLine("jolt: fixture does not match the pinned accumulator");
            return 1;
        }
        Console.WriteLine("jolt: fixture matches");
        return 0;
    }

    static void Usage()
    {
        Console.Error.WriteLine("usage: JoltOracle generate|verify <file> --revision <sha>");
    }

    static bool IsRevision(string revision)
    {
        if (revision.Length != 40)
            return false;
        foreach (var c in revision)
        {
            var hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
            if (!hex)
                return false;
        }
        return true;
    }

    static string Document(string revision)
    {
        Reject(0f);
        Reject(-0.0f);
        Reject(-1f);
        Reject(float.NaN);
        Reject(float.PositiveInfinity);
        Reject(float.NegativeInfinity);

        var primary = new SubstepAccumulator(45f);
        var sixty = new SubstepAccumulator(60f);
        var sequence = RunSequence();
        var capped = RunCapped();
        var exact = RunExact();
        var epsilon90 = RunEpsilon90();
        var epsilon11 = RunEpsilon11();
        var held = RunHeld();

        var sb = new StringBuilder();
        sb.Append("{\n");
        Field(sb, "pin", revision, 1, true);
        Field(sb, "source", Source, 1, true);
        sb.Append("  \"max_steps_per_frame\": ");
        sb.Append(Inv(SubstepAccumulator.MaxStepsPerFrame));
        sb.Append(",\n");
        sb.Append("  \"rate\": ");
        RateObject(sb, primary);
        sb.Append(",\n");
        sb.Append("  \"also_rates\": [\n    ");
        RateObject(sb, sixty);
        sb.Append("\n  ],\n");
        sb.Append("  \"sequence\": ");
        SequenceObject(sb, sequence);
        sb.Append(",\n");
        sb.Append("  \"capped\": ");
        CappedObject(sb, capped);
        sb.Append(",\n");
        sb.Append("  \"exact_sixteen\": ");
        ExactObject(sb, exact);
        sb.Append(",\n");
        sb.Append("  \"epsilon_90hz\": ");
        CountsObject(sb, epsilon90);
        sb.Append(",\n");
        sb.Append("  \"epsilon_11hz\": ");
        CountsObject(sb, epsilon11);
        sb.Append(",\n");
        sb.Append("  \"rejected_rate_bits\": [\n");
        Rejected(sb);
        sb.Append("  ],\n");
        sb.Append("  \"no_time\": [\n");
        NoTime(sb);
        sb.Append("  ],\n");
        sb.Append("  \"held\": ");
        HeldObject(sb, held);
        sb.Append("\n}\n");
        return sb.ToString();
    }

    static SequenceRun RunSequence()
    {
        const float Heartbeat = 0.0909f;
        var acc = new SubstepAccumulator(45f);
        var counts = new List<int>(1000);
        for (var i = 0; i < 1000; i++)
            counts.Add(acc.Advance(Heartbeat));
        var fours = 0;
        var fives = 0;
        var other = 0;
        foreach (var n in counts)
        {
            if (n == 4)
                fours++;
            else if (n == 5)
                fives++;
            else
                other++;
        }
        Console.WriteLine(
            "jolt: " + Inv(counts.Count) + " heartbeats of 0.0909f at 45 Hz from a zero carry: "
            + Inv(fours) + " fours, " + Inv(fives) + " fives, " + Inv(other)
            + " other, steps " + Inv(acc.Steps) + ", capped " + Inv(acc.CappedFrames));
        return new SequenceRun
        {
            Acc = acc,
            Heartbeat = Heartbeat,
            Counts = counts,
            Fours = fours,
            Fives = fives,
            Other = other,
        };
    }

    static CappedRun RunCapped()
    {
        var acc = new SubstepAccumulator(45f);
        var returned = acc.Advance(1.0);
        var steps = acc.Steps;
        var carry = acc.Carry;
        var capped = acc.CappedFrames;
        var next = acc.Advance(0.0909);
        Console.WriteLine(
            "jolt: cap Advance(1.0) returned " + Inv(returned) + ", capped " + Inv(capped)
            + ", carry " + Hex64(carry) + "; Advance(0.0909) returned " + Inv(next)
            + ", capped " + Inv(acc.CappedFrames) + ", steps " + Inv(acc.Steps)
            + ", carry " + Hex64(acc.Carry));
        return new CappedRun
        {
            Acc = acc,
            Returned = returned,
            Steps = steps,
            CarryBits = Hex64(carry),
            CappedFrames = capped,
            NextReturned = next,
        };
    }

    static ExactRun RunExact()
    {
        var acc = new SubstepAccumulator(45f);
        var frame = 16.0 / 45.0;
        var returned = acc.Advance(frame);
        Console.WriteLine(
            "jolt: exact Advance(16.0/45.0) at 45 Hz returned " + Inv(returned)
            + ", capped " + Inv(acc.CappedFrames)
            + ", steps " + Inv(acc.Steps)
            + ", carry " + Hex64(acc.Carry));
        return new ExactRun { Acc = acc, Frame = frame, Returned = returned };
    }

    // Four heartbeats so index 3 is present. That count is 12 only with the epsilon.
    static CountRun RunEpsilon90()
    {
        const float Heartbeat = 0.125f;
        var acc = new SubstepAccumulator(90f);
        var counts = new List<int>(4);
        for (var i = 0; i < 4; i++)
            counts.Add(acc.Advance(Heartbeat));
        Console.WriteLine(
            "jolt: 90 Hz 0.125f heartbeat 3 returned " + Inv(counts[3])
            + ", capped " + Inv(acc.CappedFrames)
            + ", steps " + Inv(acc.Steps)
            + ", carry " + Hex64(acc.Carry));
        return new CountRun
        {
            Acc = acc,
            Heartbeat = Heartbeat,
            HeartbeatF32 = Heartbeat,
            StoreF32 = true,
            Counts = counts,
        };
    }

    // Eleven heartbeats so index 10 is present. That count is 5 only with the epsilon.
    static CountRun RunEpsilon11()
    {
        var heartbeat = 1.0 / 11.0;
        var acc = new SubstepAccumulator(45f);
        var counts = new List<int>(11);
        for (var i = 0; i < 11; i++)
            counts.Add(acc.Advance(heartbeat));
        Console.WriteLine(
            "jolt: 45 Hz 1.0/11.0 heartbeat 10 returned " + Inv(counts[10])
            + ", capped " + Inv(acc.CappedFrames)
            + ", steps " + Inv(acc.Steps)
            + ", carry " + Hex64(acc.Carry));
        return new CountRun
        {
            Acc = acc,
            Heartbeat = heartbeat,
            StoreF32 = false,
            Counts = counts,
        };
    }

    static HeldRun RunHeld()
    {
        var acc = new SubstepAccumulator(45f);
        var first = acc.Advance(0.0909f);
        var frames = new[] { 0.0, -0.1, double.NaN, double.PositiveInfinity, double.NegativeInfinity };
        var shots = new List<Shot>();
        foreach (var frame in frames)
        {
            var returned = acc.Advance(frame);
            shots.Add(new Shot
            {
                Frame = frame,
                Returned = returned,
                Steps = acc.Steps,
                CarryBits = Hex64(acc.Carry),
                CappedFrames = acc.CappedFrames,
            });
        }
        return new HeldRun { Acc = acc, First = first, Shots = shots };
    }

    static void Reject(float rate)
    {
        try
        {
            var created = new SubstepAccumulator(rate);
            throw new InvalidOperationException("jolt: rate was accepted " + Hex32(created.RateHz));
        }
        catch (ArgumentOutOfRangeException)
        {
        }
    }

    static void RateObject(StringBuilder sb, SubstepAccumulator acc)
    {
        sb.Append("{\n");
        Field(sb, "rate_hz_bits", Hex32(acc.RateHz), 2, true);
        Field(sb, "step_bits", Hex64(StepOf(acc)), 2, true);
        Field(sb, "step_seconds_bits", Hex32(acc.StepSeconds), 2, false);
        sb.Append("  }");
    }

    static void SequenceObject(StringBuilder sb, SequenceRun run)
    {
        sb.Append("{\n");
        RateFields(sb, run.Acc, 2);
        Field(sb, "heartbeat_f32_bits", Hex32(run.Heartbeat), 2, true);
        Field(sb, "heartbeat_f64_bits", Hex64(run.Heartbeat), 2, true);
        sb.Append("    \"counts\": [\n");
        for (var i = 0; i < run.Counts.Count; i++)
        {
            sb.Append("      ");
            sb.Append(Inv(run.Counts[i]));
            if (i + 1 != run.Counts.Count)
                sb.Append(',');
            sb.Append('\n');
        }
        sb.Append("    ],\n");
        sb.Append("    \"steps\": ");
        sb.Append(Inv(run.Acc.Steps));
        sb.Append(",\n");
        Field(sb, "carry_bits", Hex64(run.Acc.Carry), 2, true);
        sb.Append("    \"capped_frames\": ");
        sb.Append(Inv(run.Acc.CappedFrames));
        sb.Append(",\n");
        sb.Append("    \"fours\": ");
        sb.Append(Inv(run.Fours));
        sb.Append(",\n");
        sb.Append("    \"fives\": ");
        sb.Append(Inv(run.Fives));
        sb.Append(",\n");
        sb.Append("    \"other\": ");
        sb.Append(Inv(run.Other));
        sb.Append("\n  }");
    }

    static void CappedObject(StringBuilder sb, CappedRun run)
    {
        sb.Append("{\n");
        RateFields(sb, run.Acc, 2);
        Field(sb, "frame_f64_bits", Hex64(1.0), 2, true);
        sb.Append("    \"returned\": ");
        sb.Append(Inv(run.Returned));
        sb.Append(",\n");
        sb.Append("    \"steps\": ");
        sb.Append(Inv(run.Steps));
        sb.Append(",\n");
        Field(sb, "carry_bits", run.CarryBits, 2, true);
        sb.Append("    \"capped_frames\": ");
        sb.Append(Inv(run.CappedFrames));
        sb.Append(",\n");
        Field(sb, "next_frame_f64_bits", Hex64(0.0909), 2, true);
        sb.Append("    \"next_returned\": ");
        sb.Append(Inv(run.NextReturned));
        sb.Append(",\n");
        sb.Append("    \"next_steps\": ");
        sb.Append(Inv(run.Acc.Steps));
        sb.Append(",\n");
        Field(sb, "next_carry_bits", Hex64(run.Acc.Carry), 2, true);
        sb.Append("    \"next_capped_frames\": ");
        sb.Append(Inv(run.Acc.CappedFrames));
        sb.Append("\n  }");
    }

    static void ExactObject(StringBuilder sb, ExactRun run)
    {
        sb.Append("{\n");
        RateFields(sb, run.Acc, 2);
        Field(sb, "frame_f64_bits", Hex64(run.Frame), 2, true);
        var counts = new List<int> { run.Returned };
        AppendCounts(sb, counts, 2);
        sb.Append("    \"steps\": ");
        sb.Append(Inv(run.Acc.Steps));
        sb.Append(",\n");
        Field(sb, "carry_bits", Hex64(run.Acc.Carry), 2, true);
        sb.Append("    \"capped_frames\": ");
        sb.Append(Inv(run.Acc.CappedFrames));
        sb.Append("\n  }");
    }

    static void CountsObject(StringBuilder sb, CountRun run)
    {
        sb.Append("{\n");
        RateFields(sb, run.Acc, 2);
        if (run.StoreF32)
            Field(sb, "heartbeat_f32_bits", Hex32(run.HeartbeatF32), 2, true);
        Field(sb, "heartbeat_f64_bits", Hex64(run.Heartbeat), 2, true);
        AppendCounts(sb, run.Counts, 2);
        sb.Append("    \"steps\": ");
        sb.Append(Inv(run.Acc.Steps));
        sb.Append(",\n");
        Field(sb, "carry_bits", Hex64(run.Acc.Carry), 2, true);
        sb.Append("    \"capped_frames\": ");
        sb.Append(Inv(run.Acc.CappedFrames));
        sb.Append("\n  }");
    }

    static void AppendCounts(StringBuilder sb, List<int> counts, int indent)
    {
        sb.Append(' ', indent * 2);
        sb.Append("\"counts\": [\n");
        for (var i = 0; i < counts.Count; i++)
        {
            sb.Append(' ', (indent + 1) * 2);
            sb.Append(Inv(counts[i]));
            if (i + 1 != counts.Count)
                sb.Append(',');
            sb.Append('\n');
        }
        sb.Append(' ', indent * 2);
        sb.Append("],\n");
    }

    static void Rejected(StringBuilder sb)
    {
        var rates = new[]
        {
            0f, -0.0f, -1f, float.NaN, float.PositiveInfinity, float.NegativeInfinity,
        };
        for (var i = 0; i < rates.Length; i++)
        {
            sb.Append("    \"");
            sb.Append(Hex32(rates[i]));
            sb.Append('"');
            if (i + 1 != rates.Length)
                sb.Append(',');
            sb.Append('\n');
        }
    }

    static void NoTime(StringBuilder sb)
    {
        var frames = new[] { 0.0, -0.1, double.NaN, double.PositiveInfinity, double.NegativeInfinity };
        for (var i = 0; i < frames.Length; i++)
        {
            var acc = new SubstepAccumulator(45f);
            var returned = acc.Advance(frames[i]);
            sb.Append("    {\n");
            RateFields(sb, acc, 3);
            Field(sb, "frame_bits", Hex64(frames[i]), 3, true);
            sb.Append("      \"returned\": ");
            sb.Append(Inv(returned));
            sb.Append(",\n");
            sb.Append("      \"steps\": ");
            sb.Append(Inv(acc.Steps));
            sb.Append(",\n");
            Field(sb, "carry_bits", Hex64(acc.Carry), 3, true);
            sb.Append("      \"capped_frames\": ");
            sb.Append(Inv(acc.CappedFrames));
            sb.Append("\n    }");
            if (i + 1 != frames.Length)
                sb.Append(',');
            sb.Append('\n');
        }
    }

    static void HeldObject(StringBuilder sb, HeldRun run)
    {
        sb.Append("{\n");
        RateFields(sb, run.Acc, 2);
        Field(sb, "heartbeat_f64_bits", Hex64((double)0.0909f), 2, true);
        sb.Append("    \"first_returned\": ");
        sb.Append(Inv(run.First));
        sb.Append(",\n");
        sb.Append("    \"frames\": [\n");
        for (var i = 0; i < run.Shots.Count; i++)
        {
            var shot = run.Shots[i];
            sb.Append("      {\n");
            Field(sb, "frame_bits", Hex64(shot.Frame), 4, true);
            sb.Append("        \"returned\": ");
            sb.Append(Inv(shot.Returned));
            sb.Append(",\n");
            sb.Append("        \"steps\": ");
            sb.Append(Inv(shot.Steps));
            sb.Append(",\n");
            Field(sb, "carry_bits", shot.CarryBits, 4, true);
            sb.Append("        \"capped_frames\": ");
            sb.Append(Inv(shot.CappedFrames));
            sb.Append("\n      }");
            if (i + 1 != run.Shots.Count)
                sb.Append(',');
            sb.Append('\n');
        }
        sb.Append("    ]\n  }");
    }

    static void RateFields(StringBuilder sb, SubstepAccumulator acc, int indent)
    {
        Field(sb, "rate_hz_bits", Hex32(acc.RateHz), indent, true);
        Field(sb, "step_bits", Hex64(StepOf(acc)), indent, true);
        Field(sb, "step_seconds_bits", Hex32(acc.StepSeconds), indent, true);
    }

    static void Field(StringBuilder sb, string name, string value, int indent, bool comma)
    {
        sb.Append(' ', indent * 2);
        sb.Append('"');
        sb.Append(name);
        sb.Append("\": \"");
        sb.Append(value);
        sb.Append('"');
        if (comma)
            sb.Append(',');
        sb.Append('\n');
    }

    static double StepOf(SubstepAccumulator acc)
    {
        var field = typeof(SubstepAccumulator).GetField(
            "_step", BindingFlags.NonPublic | BindingFlags.Instance);
        if (field == null)
            throw new InvalidOperationException("SubstepAccumulator._step is gone");
        if (field.GetValue(acc) is double step)
            return step;
        throw new InvalidOperationException("SubstepAccumulator._step is not a double");
    }

    static string Hex64(double value)
    {
        return unchecked((ulong)BitConverter.DoubleToInt64Bits(value)).ToString("x16");
    }

    static string Hex32(float value)
    {
        return unchecked((uint)BitConverter.SingleToInt32Bits(value)).ToString("x8");
    }

    static string Inv(long value)
    {
        return value.ToString(CultureInfo.InvariantCulture);
    }

    sealed class SequenceRun
    {
        internal SubstepAccumulator Acc;
        internal float Heartbeat;
        internal List<int> Counts;
        internal int Fours;
        internal int Fives;
        internal int Other;
    }

    sealed class CappedRun
    {
        internal SubstepAccumulator Acc;
        internal int Returned;
        internal long Steps;
        internal string CarryBits;
        internal long CappedFrames;
        internal int NextReturned;
    }

    sealed class ExactRun
    {
        internal SubstepAccumulator Acc;
        internal double Frame;
        internal int Returned;
    }

    sealed class CountRun
    {
        internal SubstepAccumulator Acc;
        internal double Heartbeat;
        internal float HeartbeatF32;
        internal bool StoreF32;
        internal List<int> Counts;
    }

    sealed class HeldRun
    {
        internal SubstepAccumulator Acc;
        internal int First;
        internal List<Shot> Shots;
    }

    sealed class Shot
    {
        internal double Frame;
        internal int Returned;
        internal long Steps;
        internal string CarryBits;
        internal long CappedFrames;
    }
}
