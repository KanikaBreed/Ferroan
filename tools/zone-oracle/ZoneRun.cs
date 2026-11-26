// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Text.RegularExpressions;
using MySqlConnector;
using Npgsql;
using OpenMetaverse;
using OpenSim.Data.MySQL;
using OpenSim.Data.PGSQL;

static class ZoneRun
{
    public static int Run(string[] args)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        try
        {
            string connection = Environment.GetEnvironmentVariable("FERROAN_ZONE_ADO")
                ?? throw new InvalidOperationException("missing connection");
            return args switch
            {
                ["zone", "mysql" or "postgres", "migrate"] => Migrate(args[1], connection),
                ["zone", "mysql" or "postgres", "auth-read", var principal, var token] =>
                    AuthRead(args[1], connection, principal, token),
                ["zone", "mysql" or "postgres", "auth-check", var principal, var token, var life] =>
                    AuthCheck(args[1], connection, principal, token, life),
                ["zone", "mysql" or "postgres", "auth-set", var principal, var token, var life] =>
                    AuthSet(args[1], connection, principal, token, life),
                ["zone", "mysql" or "postgres", "presence-seed", var user, var session, var region, var secure] =>
                    PresenceSeed(args[1], connection, user, session, region, secure),
                ["zone", "mysql" or "postgres", "presence-report", var session, var region] =>
                    PresenceReport(args[1], connection, session, region),
                ["zone", "mysql" or "postgres", "presence-get", var session] =>
                    PresenceGet(args[1], connection, session),
                _ => 2,
            };
        }
        catch (Exception error)
        {
            Console.Error.WriteLine("zone: " + error.GetType().Name + " " + Redact(error.Message));
            return 1;
        }
    }

    static int Migrate(string engine, string connection)
    {
        if (engine == "mysql")
        {
            _ = new MySqlAuthenticationData(connection, "auth");
            _ = new MySQLPresenceData(connection, "Presence");
        }
        else
        {
            _ = new PGSQLAuthenticationData(connection, "auth");
            _ = new PGSQLPresenceData(connection, "Presence");
        }
        Console.WriteLine("migrated=1");
        return 0;
    }

    static int AuthRead(string engine, string connection, string principal, string token)
    {
        string validity = engine == "mysql"
            ? MySqlScalar(connection,
                "SELECT DATE_FORMAT(validity, '%Y-%m-%d %H:%i:%s') FROM tokens WHERE UUID = ?id AND token = ?token",
                principal, token)
            : PgScalar(connection,
                "SELECT to_char(validity, 'YYYY-MM-DD HH24:MI:SS') FROM tokens WHERE uuid = @id AND token = @token",
                principal, token);
        Console.WriteLine("validity=" + validity);
        return 0;
    }

    static int AuthCheck(string engine, string connection, string principal, string token, string life)
    {
        int lifetime = int.Parse(life, CultureInfo.InvariantCulture);
        bool ok = engine == "mysql"
            ? new MySqlAuthenticationData(connection, "auth").CheckToken(UUID.Parse(principal), token, lifetime)
            : new PGSQLAuthenticationData(connection, "auth").CheckToken(UUID.Parse(principal), token, lifetime);
        Console.WriteLine("check=" + (ok ? "true" : "false"));
        return 0;
    }

    static int AuthSet(string engine, string connection, string principal, string token, string life)
    {
        int lifetime = int.Parse(life, CultureInfo.InvariantCulture);
        bool ok = engine == "mysql"
            ? new MySqlAuthenticationData(connection, "auth").SetToken(UUID.Parse(principal), token, lifetime)
            : new PGSQLAuthenticationData(connection, "auth").SetToken(UUID.Parse(principal), token, lifetime);
        Console.WriteLine("set=" + (ok ? "true" : "false"));
        return 0;
    }

    static int PresenceSeed(string engine, string connection, string user, string session, string region, string secure)
    {
        var row = new OpenSim.Data.PresenceData
        {
            UserID = user,
            SessionID = UUID.Parse(session),
            RegionID = UUID.Parse(region),
            Data = new Dictionary<string, string> { ["SecureSessionID"] = secure },
        };
        bool ok = engine == "mysql"
            ? new MySQLPresenceData(connection, "Presence").Store(row)
            : new PGSQLPresenceData(connection, "Presence").Store(row);
        Console.WriteLine("seed=" + (ok ? "true" : "false"));
        return 0;
    }

    static int PresenceReport(string engine, string connection, string session, string region)
    {
        bool ok = engine == "mysql"
            ? new MySQLPresenceData(connection, "Presence").ReportAgent(UUID.Parse(session), UUID.Parse(region))
            : new PGSQLPresenceData(connection, "Presence").ReportAgent(UUID.Parse(session), UUID.Parse(region));
        Console.WriteLine("report=" + (ok ? "true" : "false"));
        return 0;
    }

    static int PresenceGet(string engine, string connection, string session)
    {
        OpenSim.Data.PresenceData row = engine == "mysql"
            ? new MySQLPresenceData(connection, "Presence").Get(UUID.Parse(session))
            : new PGSQLPresenceData(connection, "Presence").Get(UUID.Parse(session));
        if (row == null || !row.Data.TryGetValue("LastSeen", out string plugin) || plugin.Length == 0)
        {
            Console.Error.WriteLine("zone: plugin LastSeen was empty");
            return 1;
        }
        string sql = engine == "mysql"
            ? MySqlScalar(connection,
                "SELECT DATE_FORMAT(`LastSeen`, '%Y-%m-%d %H:%i:%s') FROM `Presence` WHERE `SessionID` = ?id",
                session, null)
            : PgScalar(connection,
                "SELECT to_char(\"LastSeen\", 'YYYY-MM-DD HH24:MI:SS') FROM presence WHERE \"SessionID\" = @id",
                session, null);
        DateTime parsed = DateTime.Parse(plugin, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal);
        DateTime stored = DateTime.Parse(sql, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal);
        if (parsed.ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture) != stored.ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture))
        {
            Console.Error.WriteLine("zone: plugin LastSeen differs from the row");
            return 1;
        }
        Console.WriteLine("lastseen=" + sql);
        Console.WriteLine("plugin=1");
        return 0;
    }

    static string MySqlScalar(string connection, string sql, string id, string token)
    {
        using var conn = new MySqlConnection(connection);
        conn.Open();
        using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("?id", id);
        if (token != null)
            cmd.Parameters.AddWithValue("?token", token);
        return Convert.ToString(cmd.ExecuteScalar(), CultureInfo.InvariantCulture) ?? "";
    }

    static string PgScalar(string connection, string sql, string id, string token)
    {
        using var conn = new NpgsqlConnection(connection);
        conn.Open();
        using var cmd = new NpgsqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("id", Guid.Parse(id));
        if (token != null)
            cmd.Parameters.AddWithValue("token", token);
        return Convert.ToString(cmd.ExecuteScalar(), CultureInfo.InvariantCulture) ?? "";
    }

    static string Redact(string text)
    {
        return Regex.Replace(text, "(?i)(password\\s*=\\s*)([^;]*)", "$1***");
    }
}
