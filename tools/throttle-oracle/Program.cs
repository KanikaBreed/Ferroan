// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Text.Json;
using Nini.Config;
using OpenSim.Framework;
using OpenSim.Region.ClientStack.LindenUDP;

/// <summary>
/// Steady rates from the pinned token buckets. The clamps follow
/// <c>LLUDPClient.SetThrottles</c>. The buckets themselves are the pinned types.
/// </summary>
static class Program
{
    const float SceneRate = 6_250_000f;
    static readonly float[] Floors = { 1000f, 1000f, 50f, 50f, 1000f, 1000f, 1000f };
    static readonly int[] AssignOrder = { 0, 1, 2, 3, 6, 4, 5 };

    static int Main(string[] args)
    {
        var writing = false;
        var revision = "";
        var fixtures = "";
        for (var i = 0; i < args.Length; i++)
        {
            if (args[i] == "generate")
                writing = true;
            else if (args[i] == "verify")
                writing = false;
            else if (args[i] == "--revision" && i + 1 < args.Length)
                revision = args[++i];
            else if (!args[i].StartsWith("--"))
                fixtures = args[i];
        }
        if (fixtures.Length == 0 || revision.Length == 0)
        {
            Console.Error.WriteLine("usage: ThrottleOracle generate|verify <dir> --revision <sha>");
            return 2;
        }
        if (!BitConverter.IsLittleEndian)
        {
            Console.Error.WriteLine("throttle oracle requires a little-endian host");
            return 1;
        }

        var json = JsonSerializer.Serialize(Build(revision), Options()) + "\n";
        var path = Path.Combine(fixtures, "rates.json");
        if (writing)
        {
            Directory.CreateDirectory(fixtures);
            File.WriteAllText(path, json);
            Console.WriteLine($"throttle: wrote {path}");
            return 0;
        }
        if (!File.Exists(path))
        {
            Console.Error.WriteLine($"throttle: missing {path}");
            return 1;
        }
        var existing = File.ReadAllText(path);
        if (existing != json)
        {
            Console.Error.WriteLine("throttle: fixture does not match the pinned buckets");
            return 1;
        }
        Console.WriteLine("throttle: fixture matches");
        return 0;
    }

    static JsonSerializerOptions Options() => new() { WriteIndented = true };

    static Document Build(string revision)
    {
        var rates = new ThrottleRates(new IniConfigSource());
        if (rates.AdaptiveThrottlesEnabled || rates.ClientMaxRate != 640000 || rates.Resend != 6625)
            throw new InvalidOperationException("stock throttle defaults changed");
        var bot = ChannelBlob(new[] { 600f, 400f, 80f, 80f, 2050f, 2050f, 740f });
        var ones = RepeatedFloat(1f);
        return new Document
        {
            revision = revision,
            cases = new List<CaseSnap>
            {
                Snap("defaults", 1, false, 1f, Array.Empty<byte>(), rates),
                Snap("bot_6000", 1, true, 1f, bot, rates),
                Snap("floors", 1, true, 1f, ones, rates),
                Snap("half", 1, true, 0.5f, bot, rates),
                Snap("ten_clients", 10, true, 1f, bot, rates),
            },
        };
    }

    static byte[] RepeatedFloat(float value)
    {
        var data = new byte[28];
        var bits = BitConverter.GetBytes(value);
        for (var i = 0; i < 7; i++)
            Buffer.BlockCopy(bits, 0, data, i * 4, 4);
        return data;
    }

    static byte[] ChannelBlob(float[] kbps)
    {
        var data = new byte[28];
        for (var i = 0; i < 7; i++)
        {
            var bits = BitConverter.GetBytes(kbps[i] * 1024f);
            Buffer.BlockCopy(bits, 0, data, i * 4, 4);
        }
        return data;
    }

    static CaseSnap Snap(string name, int clients, bool apply, float factor, byte[] blob, ThrottleRates rates)
    {
        var scene = new TokenBucket(null, SceneRate, SceneRate * 10e-3f);
        var opened = new List<(AdaptiveTokenBucket Client, TokenBucket[] Categories)>();
        for (var n = 0; n < clients; n++)
            opened.Add(Open(scene, rates));
        if (apply)
        {
            foreach (var (_, categories) in opened)
                Apply(categories, rates.ClientMaxRate, rates.BurstTime, blob, factor);
        }
        var first = opened[0];
        var shots = new List<BucketSnap>();
        for (var i = 0; i < first.Categories.Length; i++)
            shots.Add(Shot(first.Categories[i]));
        return new CaseSnap
        {
            name = name,
            clients = clients,
            apply = apply,
            factor = factor,
            blob = Convert.ToHexString(blob).ToLowerInvariant(),
            scene = Shot(scene),
            client = Shot(first.Client),
            categories = shots,
        };
    }

    static (AdaptiveTokenBucket Client, TokenBucket[] Categories) Open(TokenBucket scene, ThrottleRates rates)
    {
        var client = new AdaptiveTokenBucket(
            scene,
            rates.ClientMaxRate,
            rates.ClientMaxRate * rates.BurstTime,
            rates.AdaptiveThrottlesEnabled);
        var categories = new TokenBucket[8];
        for (var i = 0; i < categories.Length; i++)
        {
            var rate = rates.GetRate((ThrottleOutPacketType)i);
            categories[i] = new TokenBucket(client, rate, rate * rates.BurstTime);
        }
        return (client, categories);
    }

    static void Apply(TokenBucket[] categories, float maxRate, float burstTime, byte[] data, float factor)
    {
        var scale = 0.125f * factor;
        var values = new float[7];
        for (var i = 0; i < values.Length; i++)
            values[i] = Math.Clamp(BitConverter.ToSingle(data, i * 4) * scale, Floors[i], maxRate);
        var total = 0f;
        for (var i = 0; i < values.Length; i++)
            total += values[i];
        if (total > maxRate)
        {
            var down = maxRate / total;
            for (var i = 0; i < values.Length; i++)
                values[i] *= down;
        }
        foreach (var index in AssignOrder)
        {
            categories[index].RequestedDripRate = values[index];
            categories[index].RequestedBurst = values[index] * burstTime;
        }
    }

    static BucketSnap Shot(TokenBucket bucket) => new()
    {
        requested_drip = unchecked((uint)BitConverter.SingleToInt32Bits(bucket.RequestedDripRate)),
        drip = unchecked((uint)BitConverter.SingleToInt32Bits(bucket.DripRate)),
        requested_burst = unchecked((uint)BitConverter.SingleToInt32Bits(bucket.RequestedBurst)),
        burst = unchecked((uint)BitConverter.SingleToInt32Bits(bucket.Burst)),
        total = unchecked((uint)BitConverter.SingleToInt32Bits(bucket.TotalDripRequest)),
    };

    sealed class Document
    {
        public string revision { get; set; }
        public List<CaseSnap> cases { get; set; }
    }

    sealed class CaseSnap
    {
        public string name { get; set; }
        public int clients { get; set; }
        public bool apply { get; set; }
        public float factor { get; set; }
        public string blob { get; set; }
        public BucketSnap scene { get; set; }
        public BucketSnap client { get; set; }
        public List<BucketSnap> categories { get; set; }
    }

    sealed class BucketSnap
    {
        public uint requested_drip { get; set; }
        public uint drip { get; set; }
        public uint requested_burst { get; set; }
        public uint burst { get; set; }
        public uint total { get; set; }
    }
}
