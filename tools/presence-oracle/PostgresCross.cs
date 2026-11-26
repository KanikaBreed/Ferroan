// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using Npgsql;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Data.PGSQL;

static class PostgresCross
{
    public static int Run(string phase, string prefix, string expectedStamp, bool alternate)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        string connection = Environment.GetEnvironmentVariable("FERROAN_PRESENCE_ADO_POSTGRES")
            ?? throw new InvalidOperationException("Set FERROAN_PRESENCE_ADO_POSTGRES");
        var realm = alternate ? "PresenceGridB" + Guid.Parse(prefix + "00").ToString("N") : "Presence";
        var store = new PGSQLPresenceData(connection, realm);
        UUID Id(int suffix) => UUID.Parse(prefix + suffix.ToString("x2"));
        var user = Id(1);
        var first = Id(2);
        var second = Id(3);
        var third = Id(4);
        var secure = Id(5);
        var home = Id(6);
        var away = Id(7);
        var staging = Id(8);
        var thirdSecure = Id(9);
        PresenceData Row(UUID session, UUID region) => new() {
            UserID = user.ToString(), SessionID = session, RegionID = region, Data = new()
        };

        static void Check(bool holds, string step)
        {
            if (!holds)
            {
                Console.Error.WriteLine("Presence check failed: " + step);
                throw new InvalidOperationException("Presence cross-read mismatch");
            }
        }

        // Get exposes a culture-formatted string. Read the same row's native
        // value to compare all six fractional digits as well as SQL NULL.
        void CheckStamp()
        {
            using var conn = new NpgsqlConnection(connection);
            conn.Open();
            using var cmd = new NpgsqlCommand($"SELECT \"LastSeen\" FROM {realm} WHERE \"SessionID\" = @id", conn);
            cmd.Parameters.AddWithValue("id", first.Guid);
            object stamp = cmd.ExecuteScalar();
            if (expectedStamp.Length == 0)
                Check(stamp is DBNull, "restored SQL NULL");
            else
                Check(stamp is DateTime time &&
                    time == DateTime.Parse(expectedStamp, CultureInfo.InvariantCulture),
                    "restored reported microseconds");
        }

        static bool RefusesTime(Action action)
        {
            try { action(); return false; }
            catch (InvalidCastException) { return true; }
        }

        switch (phase)
        {
            case "seed":
                var initial = Row(first, home);
                initial.Data["SecureSessionID"] = secure.ToString();
                Check(store.Store(initial) && store.Store(Row(second, home)), "fresh sessions");
                Check(store.Store(Row(first, staging)), "partial store");
                Check(store.Get(first).Data["LastSeen"] == "", "fresh timestamp");
                Check(store.VerifyAgent(user, secure), "partial store kept secure session");
                Check(store.Get("SecureSessionID", "invalid").Any(x => x.SessionID == second), "invalid UUID selects zero");
                Check(RefusesTime(() => store.Get("LastSeen", "")) &&
                      RefusesTime(() => store.Delete("LastSeen", "")), "text timestamp selectors");
                break;
            case "report":
                var rust = store.Get(third);
                Check(rust.RegionID == home && rust.UserID == user.ToString() &&
                    rust.Data["LastSeen"] == "" && store.VerifyAgent(user, thirdSecure), "Rust insert");
                Check(store.Get("UserID", user.ToString()).Length == 3, "three sessions");
                Check(!store.ReportAgent(first, UUID.Zero) && !store.ReportAgent(Id(10), home), "refused reports");
                Check(store.ReportAgent(first, away), "report");
                var reported = store.Get(first);
                Check(reported.RegionID == away && reported.Data["LastSeen"] != "", "report stamped row");
                Check(RefusesTime(() => store.Store(reported)), "TB105 fetched row re-store");
                reported.Data.Remove("LastSeen");
                Check(store.Store(reported), "same row without LastSeen");
                break;
            case "partial":
                var partial = store.Get(first);
                Check(partial.RegionID == staging && partial.Data["SecureSessionID"] == secure.ToString(), "Rust partial update");
                CheckStamp();
                Check(RefusesTime(() => store.Get("LastSeen", expectedStamp)) &&
                      RefusesTime(() => store.Delete("LastSeen", expectedStamp)), "TB114 valid timestamp selectors");
                break;
            case "restored-null":
                Check(store.Get(first).Data["LastSeen"] == "", "Pin8 reads null timestamp");
                CheckStamp();
                break;
            case "restored-in-place":
                Check(store.Get(first).RegionID == away, "re-stored region");
                CheckStamp();
                break;
            case "restored-report":
                Check(store.Get(first).RegionID == away && store.Get(first).Data["LastSeen"] != "", "Pin8 reads reported row");
                CheckStamp();
                Check(store.Get(second).RegionID == away && store.Get(second).Data["LastSeen"] != "", "Rust report");
                Check(store.VerifyAgent(user, secure) && !store.VerifyAgent(away, secure), "secure session");
                store.LogoutRegionAgents(home);
                break;
            case "after-logout":
                Check(store.Get("UserID", user.ToString()).Length == 0, "Rust region logout");
                break;
            default:
                return 2;
        }
        return 0;
    }
}
