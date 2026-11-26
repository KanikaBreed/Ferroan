// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using MySqlConnector;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Data.MySQL;

static class MysqlCross
{
    public static int Run(string phase, string prefix, bool alternate)
    {
        // The generic reader formats LastSeen with the process culture (TB104).
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        string connection = Environment.GetEnvironmentVariable("FERROAN_PRESENCE_ADO_MYSQL")
            ?? throw new InvalidOperationException("Set FERROAN_PRESENCE_ADO_MYSQL");
        var realm = alternate ? "PresenceGridB" + Guid.Parse(prefix + "00").ToString("N") : "Presence";
        var store = new MySQLPresenceData(connection, realm);
        UUID Id(int suffix) => UUID.Parse(prefix + suffix.ToString("x2"));
        var user = Id(1);
        var first = Id(2);
        var second = Id(3);
        var secure = Id(4);
        var home = Id(5);
        var away = Id(6);
        var newSecure = Id(7);

        PresenceData Row(UUID session, UUID key) => new() {
            UserID = user.ToString(), SessionID = session, RegionID = home,
            Data = new() { ["SecureSessionID"] = key.ToString() }
        };
        static void Check(bool condition, string step)
        {
            if (!condition)
            {
                Console.Error.WriteLine("Presence check failed: " + step);
                throw new InvalidOperationException("Presence cross-read mismatch");
            }
        }

        switch (phase)
        {
            case "seed":
                var original = Row(first, newSecure);
                original.UserID = "visiting-user@example.invalid";
                Check(store.Store(original), "initial store");
                var replacementRow = Row(first, secure);
                replacementRow.Data["SecureSessionID"] = secure.ToString().ToUpperInvariant();
                Check(store.Store(replacementRow), "replace session");
                Check(store.Get(first).Data["SecureSessionID"] == secure.ToString(), "secure UUID spelling");
                Check(store.Store(Row(second, newSecure)), "second session");
                Check(store.Get("UserID", user.ToString()).Length == 2, "two sessions");
                var bad = Row(Id(9), secure);
                bad.UserID = "invalid-secure-session";
                bad.Data["SecureSessionID"] = "invalid";
                Check(store.Store(bad), "store text in CHAR(36)");
                bool refused = false;
                try { store.Get(bad.SessionID); }
                catch (FormatException) { refused = true; }
                Check(refused, "malformed secure UUID read");
                break;
            case "report":
                Check(!store.ReportAgent(first, UUID.Zero), "zero report");
                Check(store.ReportAgent(first, away), "region move");
                break;
            case "restore":
                var reported = store.Get(first);
                Check(reported.RegionID == away && reported.Data.ContainsKey("LastSeen"), "reported row");
                Check(!store.Store(reported), "TB104: fetched LastSeen refuses re-store");
                CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("en-US");
                Check(!store.Store(store.Get(first)), "TB104: Robust culture refuses re-store");
                CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("sv-SE");
                Check(store.Store(store.Get(first)), "Swedish culture accepts re-store");
                CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
                reported.Data.Remove("LastSeen");
                Check(store.Store(reported), "same row without LastSeen");
                break;
            case "read-rust":
                var moved = store.Get(first);
                Check(moved.UserID == user.ToString() && moved.RegionID == home, "Rust re-store");
                Check(moved.Data["SecureSessionID"] == secure.ToString(), "retained secure session");
                var seen = DateTime.Parse(moved.Data["LastSeen"], CultureInfo.InvariantCulture);
                var instant = new DateTimeOffset(2001, 2, 3, 4, 5, 6, TimeSpan.Zero).ToUnixTimeSeconds();
                using (var conn = new MySqlConnection(connection))
                {
                    conn.Open();
                    using var cmd = new MySqlCommand($"SELECT UNIX_TIMESTAMP(LastSeen), FROM_UNIXTIME(@instant) FROM {realm} WHERE SessionID=@id", conn);
                    cmd.Parameters.AddWithValue("@instant", instant);
                    cmd.Parameters.AddWithValue("@id", first.ToString());
                    using var reader = cmd.ExecuteReader();
                    Check(reader.Read() && reader.GetInt64(0) == instant && reader.GetDateTime(1) == seen, "D84: preserved instant in server zone");
                }
                var replacement = store.Get(second);
                Check(replacement.UserID == user.ToString() && replacement.RegionID == away, "Rust report");
                Check(replacement.Data["SecureSessionID"] == UUID.Zero.ToString(), "replacement default");
                Check(store.VerifyAgent(user, secure) && !store.VerifyAgent(away, secure), "verify");
                Check(store.Get("UserID", user.ToString()).Length == 2, "two Rust sessions");
                store.LogoutRegionAgents(home);
                Check(store.Get(first) == null && store.Get(second) != null, "region logout");
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
