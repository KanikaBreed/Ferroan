// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Net;
using System.Text;
using System.Text.Json;
using Nini.Config;
using Npgsql;
using OpenMetaverse;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Server.Handlers.Presence;
using OpenSim.Server.Base;
using OpenSim.Services.PresenceService;

static class ServiceCases
{
    // Each invocation starts a fresh process (including the two static caches).
    public static int Run(string action, string pin, uint port, string path, bool postgres)
    {
        var config = new IniConfigSource();
        var section = config.AddConfig("PresenceService");
        section.Set("StorageProvider", postgres ? typeof(OpenSim.Data.PGSQL.PGSQLPresenceData).Assembly.Location
            : typeof(OpenSim.Data.Null.NullPresenceData).Assembly.Location);
        if (!postgres) section.Set("AllowDuplicatePresences", true);
        string realm = "presenceprobe" + Guid.NewGuid().ToString("N");
        string connection = Environment.GetEnvironmentVariable("FERROAN_PRESENCE_ADO_POSTGRES");
        using var db = postgres ? new NpgsqlConnection(connection) : null;
        if (postgres)
        {
            _ = new OpenSim.Data.PGSQL.PGSQLPresenceData(connection, "Presence");
            db.Open();
            using var create = new NpgsqlCommand($"CREATE TABLE {realm} (LIKE presence INCLUDING ALL)", db);
            create.ExecuteNonQuery();
            section.Set("ConnectionString", connection);
            section.Set("Realm", realm);
        }
        try
        {
            var steps = new List<object>();
            string Id(int n) => $"00000000-0000-4000-8000-{n:x12}";
            var user = Id(1); var s1 = Id(2); var s2 = Id(3); var region = Id(4);
            var secure = Id(5); var other = "visitor & guest"; var s3 = Id(7); var zero = UUID.Zero.ToString();
            var server = new BaseHttpServer(port) { ListenIPAddress = IPAddress.Loopback };
            var service = new PresenceService(config);
            server.AddStreamHandler(new PresenceServerPostHandler(service, ServiceAuth.Create(config, "PresenceService")));
            server.Start(false, false);
            try
            {
                using var client = new HttpClient(new SocketsHttpHandler { UseProxy = false }) { Timeout = TimeSpan.FromSeconds(10) };
                void Post(string label, string body, bool shard = false)
                {
                    using var request = new HttpRequestMessage(HttpMethod.Post, $"http://127.0.0.1:{port}/presence");
                    request.Content = new StringContent(body, Encoding.UTF8, "application/x-www-form-urlencoded");
                    if (shard) request.Headers.Add("X-SecondLife-Shard", "fixture");
                    using var response = client.Send(request);
                    byte[] reply = response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult();
                    steps.Add(new { label, body, shard, status = (int)response.StatusCode,
                        contentType = response.Content.Headers.ContentType?.ToString(), reply = Convert.ToHexStringLower(reply) });
                }
                void Login(string id, string session)
                {
                    bool result = service.LoginAgent(id, UUID.Parse(session), UUID.Parse(secure));
                    steps.Add(new { login = id, session, secure, result });
                }
                string Form(string method, string args) => $"VERSIONMIN={ProtocolVersions.ClientProtocolVersionMin}&VERSIONMAX={ProtocolVersions.ClientProtocolVersionMax}&METHOD={method}&{args}";
                void Get(string label, string session) => Post(label, Form("getagent", "SessionID=" + session));
                void Many(string label, params string[] ids) => Post(label, Form("getagents", string.Join("&", ids.Select(i => "uuids[]=" + Uri.EscapeDataString(i)))));
                Post("HTTP login refused", Form("login", $"UserID={user}&SessionID={s1}&SecureSessionID={secure}"));
                Get("absent session", s1);
                Post("missing method", $"SessionID={s1}"); Post("method list", "METHOD[]=getagent");
                Post("unknown method", "METHOD=LOGIN"); Post("getagents missing list", "METHOD=getagents");
                Post("getagents scalar", Form("getagents", $"uuids={user}"));
                Many("no known users", user);
                Post("malformed session", Form("logout", "SessionID=bad"));
                Post("report missing region", Form("report", $"SessionID={s1}"));
                Post("malformed region", Form("logoutregion", "RegionID=bad"));
                Post("script header", Form("getagent", $"SessionID={s1}"), true);
                Login(user, s1);
                Get("local login", s1);
                Post("zero region report", Form("report", $"SessionID={s1}&RegionID={zero}"));
                Post("move", Form("report", $"SessionID={s1}&RegionID={region}"));
                Many("online member after 1024 misses", Enumerable.Repeat("x", 1024).Append(user).ToArray());
                Get("moved", s1);
                Login(user, s2);
                Many("cached session for duplicate user", user);
                Get("previous session after second login", s1); Get("new session", s2);
                Login(other, s3);
                Many("requested order and repeats", other, Id(20), user, other);
                Post("move replacement", Form("report", $"SessionID={s2}&RegionID={region}"));
                Post("logout region", Form("logoutregion", $"RegionID={region}"));
                Many("region cleared", user, other);
                Post("empty region", Form("logoutregion", $"RegionID={region}"));
                Post("logout", Form("logout", $"SessionID={s3}"));
                Post("repeat logout", Form("logout", $"SessionID={s3}"));
                Get("after logout", s3);
            }
            finally { server.Stop(); }
            // Compare the local service result separately from the HTTP observations.
            bool? refusedStoreLogin = null;
            if (postgres)
            {
                using var rule = new NpgsqlCommand($"CREATE RULE refuse_insert AS ON INSERT TO {realm} DO INSTEAD NOTHING", db);
                rule.ExecuteNonQuery();
                refusedStoreLogin = service.LoginAgent(Id(20), UUID.Parse(Id(21)), UUID.Parse(secure));
                using var count = new NpgsqlCommand($"SELECT count(*) FROM {realm} WHERE \"SessionID\"=@id", db);
                count.Parameters.AddWithValue("id", Guid.Parse(Id(21)));
                if ((long)count.ExecuteScalar() != 0) throw new InvalidOperationException("expected a refused insert");
            }
            var json = JsonSerializer.Serialize(new { pin, allowDuplicates = !postgres, refusedStoreLogin, steps }, new JsonSerializerOptions { WriteIndented = true }).ReplaceLineEndings("\n") + "\n";
            if (action == "generate") File.WriteAllText(path, json);
            else if (action != "verify" || File.ReadAllText(path).ReplaceLineEndings("\n") != json)
                throw new InvalidOperationException("Presence HTTP observations differ");
            Console.WriteLine($"presence: {steps.Count} steps match on {(postgres ? "PostgreSQL" : "Null")}");
            return 0;
        }
        finally
        {
            if (postgres)
            {
                using var drop = new NpgsqlCommand($"DROP TABLE {realm}", db);
                drop.ExecuteNonQuery();
            }
        }
    }
}
