// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Execute the pinned ControlPlaneAccess; no implementation is copied here.
// Source: Source/OpenSim.Server.Handlers/Base/ControlPlaneAccess.cs,
// KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5.
using System.Net;
using System.Text.Json.Nodes;
using Nini.Config;
using OpenSim.Server.Handlers.Base;

namespace Ferroan.Pin3Oracle;

static class TrustCases
{
    public static JsonObject Capture()
    {
        string[] entries = ["", "  robust:8003  ", "robust.example.invalid:8003", "fd00::1",
            "[fd00::1]:8003", "http://[fd00::2]:8003/path", "fe80::1%2", "fe80::1%fixture0",
            "[fe80::1%2]:8003", "http://[fe80::1%2]:8003/path", "::ffff:192.0.2.3",
            "http://robust.example.invalid:8003/path", "[]:8003", ":8003", "[broken",
            "192.0.2.0/24", "file:///tmp/fixture", "127.1", "0x7f000001", "0177.0.0.1",
            "[fd00::1]:bad", "[fd00::1]garbage", "[fd00::1]:-1", "[fd00::1]:65536", "[192.0.2.1]:8003",
            "fe80::1%", "fe80::1%bad%zone"];
        JsonArray forms = new();
        foreach (string entry in entries)
        {
            string address = null;
            if (IPAddress.TryParse(entry.Trim(), out IPAddress parsed))
                address = parsed.IsIPv4MappedToIPv6 ? parsed.MapToIPv4().ToString() : new IPAddress(parsed.GetAddressBytes()).ToString();
            forms.Add(new JsonObject { ["entry"] = entry, ["host"] = ControlPlaneAccess.HostOf(entry), ["address"] = address });
        }
        return new JsonObject { ["host_forms"] = forms, ["combined_lists"] = CombinedLists(), ["refresh"] = Refresh() };
    }

    static JsonObject CombinedLists()
    {
        const string ini = "[Security]\nControlPlaneTrustedHosts=\"192.0.2.1;[::ffff:192.0.2.2]:8003|fd00::1 fe80::1%2\"\nTrustedControlPlaneHosts=robust:8003\n[Network]\nControlPlaneTrustedHosts=\"192.0.2.3,robust.example.invalid:8003,[]:8003,:8003,192.0.2.0/24\"\nTrustedControlPlaneHosts=http://[fd00::2]:8003/\nhostname=http://local.example.invalid:9000/\n";
        JsonArray lookups = new();
        JsonObject answers = new() {
            ["robust"] = new JsonArray("192.0.2.4"),
            ["robust.example.invalid"] = new JsonArray("192.0.2.5"),
            ["local.example.invalid"] = new JsonArray("192.0.2.6")
        };
        IPAddress[] Resolve(string name)
        {
            lookups.Add(name);
            if (answers[name] is not JsonArray values)
                throw new InvalidOperationException("fixture DNS failure");
            return values.Select(value => IPAddress.Parse(value.GetValue<string>())).ToArray();
        }
        ControlPlaneAccess policy = new(new IniConfigSource(new StringReader(ini)), Resolve, new Clock());
        JsonArray queries = new();
        foreach (string address in new[] { "127.0.0.9", "::1", "::ffff:127.0.0.9", "192.0.2.1", "192.0.2.2", "::ffff:192.0.2.2", "192.0.2.3", "192.0.2.4", "192.0.2.5", "192.0.2.6", "fd00::1", "fd00::2", "fe80::1%7", "192.0.2.9", "fd00::9" })
            queries.Add(new JsonObject { ["address"] = address, ["allowed"] = policy.IsTrustedAddress(IPAddress.Parse(address)) });
        return new JsonObject { ["ini"] = ini, ["answers"] = answers, ["lookups"] = lookups, ["queries"] = queries };
    }

    static JsonObject Refresh()
    {
        const string ini = "[Network]\nControlPlaneTrustedHosts=moving.example.invalid,stable.example.invalid,missing.example.invalid\n";
        Dictionary<string, string[]> answers = new() {
            ["moving.example.invalid"] = ["192.0.2.10"],
            ["stable.example.invalid"] = ["192.0.2.20"],
            ["missing.example.invalid"] = null
        };
        JsonObject Answers()
        {
            JsonObject output = new();
            foreach (var (name, addresses) in answers)
                output[name] = addresses is null ? null : new JsonArray(addresses.Select(value => (JsonNode)JsonValue.Create(value)).ToArray());
            return output;
        }
        JsonArray calls = new();
        IPAddress[] Resolve(string name)
        {
            calls.Add(name);
            string[] found = answers[name];
            if (found is null) throw new InvalidOperationException("fixture DNS failure");
            return found.Select(IPAddress.Parse).ToArray();
        }
        Clock time = new();
        JsonObject startupAnswers = Answers();
        ControlPlaneAccess policy = new(new IniConfigSource(new StringReader(ini)), Resolve, time);
        JsonArray startupCalls = calls;
        JsonArray steps = new();
        void Observe(int seconds, string query)
        {
            time.Seconds = seconds;
            calls = new JsonArray();
            bool allowed = policy.IsTrustedAddress(IPAddress.Parse(query));
            steps.Add(new JsonObject { ["seconds"] = seconds, ["answers"] = Answers(), ["query"] = query, ["allowed"] = allowed, ["lookups"] = calls });
        }
        Observe(0, "192.0.2.10");
        Observe(59, "192.0.2.11");
        answers["moving.example.invalid"] = ["192.0.2.11"];
        answers["stable.example.invalid"] = [];
        answers["missing.example.invalid"] = ["192.0.2.30"];
        Observe(60, "192.0.2.11");
        Observe(60, "192.0.2.10");
        Observe(60, "192.0.2.20");
        Observe(60, "192.0.2.30");
        answers["moving.example.invalid"] = null;
        answers["stable.example.invalid"] = ["192.0.2.21"];
        answers["missing.example.invalid"] = [];
        Observe(120, "192.0.2.21");
        Observe(120, "192.0.2.11");
        Observe(120, "192.0.2.20");
        Observe(120, "192.0.2.30");
        answers["moving.example.invalid"] = ["192.0.2.12"];
        answers["stable.example.invalid"] = ["192.0.2.22"];
        answers["missing.example.invalid"] = ["192.0.2.31"];
        Observe(180, "192.0.2.11");
        Observe(180, "192.0.2.12");
        return new JsonObject { ["ini"] = ini, ["startup_answers"] = startupAnswers, ["startup_lookups"] = startupCalls, ["steps"] = steps };
    }
    sealed class Clock : TimeProvider
    {
        public int Seconds { get; set; }
        public override DateTimeOffset GetUtcNow() => new DateTimeOffset(2026, 1, 1, 0, 0, 0, TimeSpan.Zero).AddSeconds(Seconds);
    }
}
