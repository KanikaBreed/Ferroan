// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Net;
using System.Reflection;
using System.Text.Json;
using Nini.Config;
using UUID = OpenMetaverse.UUID;
using TokenBucket = OpenSim.Region.ClientStack.LindenUDP.TokenBucket;
using OpenSim.Region.ClientStack.LindenUDP;

/// <summary>
/// Steady rates from the pinned <c>LLUDPClient</c>: its constructor builds the
/// client and category buckets and <c>SetThrottles</c> applies each blob.
/// </summary>
static class Program
{
    const float SceneRate = 6_250_000f;
    static readonly FieldInfo Categories = typeof(LLUDPClient).GetField(
        "m_throttleCategories", BindingFlags.NonPublic | BindingFlags.Instance)
        ?? throw new InvalidOperationException("LLUDPClient.m_throttleCategories is gone");

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
        var nan = (byte[])bot.Clone();
        Buffer.BlockCopy(BitConverter.GetBytes(float.NaN), 0, nan, 16, 4);
        var infinities = new byte[28];
        for (var i = 0; i < 7; i++)
        {
            var value = i % 2 == 0 ? float.PositiveInfinity : float.NegativeInfinity;
            Buffer.BlockCopy(BitConverter.GetBytes(value), 0, infinities, i * 4, 4);
        }
        var short8 = new byte[8];
        Buffer.BlockCopy(bot, 0, short8, 0, 8);
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
                Snap("infinities", 1, true, 1f, infinities, rates),
                // TB83. Ferroan leaves the rates unchanged (D68).
                Snap("nan_task", 2, true, 1f, nan, rates),
                // SetThrottles reads past a short array (TB84). The bytes after
                // a freshly allocated array are normally zero, so these record
                // what that read usually gives. Ferroan ignores the blob (D68).
                Snap("short_0", 1, true, 1f, Array.Empty<byte>(), rates),
                Snap("short_8", 1, true, 1f, short8, rates),
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
        // LLUDPServer builds the scene bucket the same way.
        var scene = new TokenBucket(null, SceneRate, SceneRate * 10e-3f);
        var opened = new List<LLUDPClient>();
        for (var n = 0; n < clients; n++)
        {
            opened.Add(new LLUDPClient(
                null, rates, scene, 0, UUID.Zero, new IPEndPoint(IPAddress.Loopback, 1), 0, 0));
        }
        if (apply)
        {
            foreach (var client in opened)
            {
                // A fresh copy is the newest allocation, as a decoded packet is.
                var data = (byte[])blob.Clone();
                client.SetThrottles(data, factor);
            }
        }
        var first = opened[0];
        var categories = (TokenBucket[])Categories.GetValue(first);
        var shots = new List<BucketSnap>();
        for (var i = 0; i < categories.Length; i++)
            shots.Add(Shot(categories[i]));
        return new CaseSnap
        {
            name = name,
            clients = clients,
            apply = apply,
            factor = factor,
            blob = Convert.ToHexString(blob).ToLowerInvariant(),
            scene = Shot(scene),
            client = Shot(first.FlowThrottle),
            categories = shots,
        };
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
