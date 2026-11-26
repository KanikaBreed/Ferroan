// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Net;
using System.Text;
using System.Text.Json;
using MySqlConnector;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Server.Handlers.Avatar;
using OpenSim.Services.AvatarService;

static class ServiceCases
{
    // A selected SQLite Store can return false. Every other call, including
    // cleanup and all MySQL writes, goes through the pinned database plugin.
    sealed class StoreProbe(IAvatarData db) : IAvatarData
    {
        public string Refuse;
        public AvatarBaseData[] Get(string field, string val) => db.Get(field, val);
        public bool Store(AvatarBaseData row) => row.Data["Name"] != Refuse && db.Store(row);
        public bool Delete(UUID id, string name) => db.Delete(id, name);
        public bool Delete(string field, string val) => db.Delete(field, val);
    }
    sealed class Database(string connection) : OpenSim.Data.SQLite.SQLiteAvatarData(connection, "Avatars"), IDisposable
    {
        // The generic handler locates migrations in the concrete type's assembly.
        protected override System.Reflection.Assembly Assembly => typeof(OpenSim.Data.SQLite.SQLiteAvatarData).Assembly;
        public void Dispose() => m_Connection.Dispose();
    }
    sealed class Service(IConfigSource source, IAvatarData store) : AvatarService(source, store);
    record Observation(string Label, string Body, string Refuse, bool Shard, int Status, string ContentType, string Reply);

    public static int Run(string action, string pin, uint port, string path, bool mysql)
    {
        string dir = Directory.CreateTempSubdirectory("avatar-http-").FullName;
        try
        {
            CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;
            using var sqlite = mysql ? null : new Database($"Data Source={Path.Combine(dir, "Avatar.db")};Pooling=False;");
            using var table = mysql ? new MysqlTable() : null;
            var probe = new StoreProbe(mysql ? table.Store : sqlite);
            var config = new IniConfigSource();
            config.AddConfig("AvatarService");
            var http = new BaseHttpServer(port) { ListenIPAddress = IPAddress.Loopback };
            http.AddStreamHandler(new AvatarServerPostHandler(new Service(config, probe), ServiceAuth.Create(config, "AvatarService")));
            http.Start(false, false);
            try
            {
                using var client = new HttpClient(new SocketsHttpHandler { UseProxy = false }) { Timeout = TimeSpan.FromSeconds(10) };
                var observations = new List<Observation>();
                void Post(string label, string body, string refuse = null, bool shard = false)
                {
                    probe.Refuse = refuse;
                    using var request = new HttpRequestMessage(HttpMethod.Post, $"http://127.0.0.1:{port}/avatar");
                    request.Content = new StringContent(body, Encoding.UTF8, "application/x-www-form-urlencoded");
                    if (shard) request.Headers.Add("X-SecondLife-Shard", "fixture");
                    using var response = client.Send(request);
                    string reply = new UTF8Encoding(false, true).GetString(response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult());
                    observations.Add(new(label, body, refuse, shard, (int)response.StatusCode, response.Content.Headers.ContentType?.ToString(), reply));
                }
                if (mysql)
                    MysqlCases(Post, table);
                else
                    SqliteCases(Post);
                var output = JsonSerializer.Serialize(new { pin, observations }, new JsonSerializerOptions { WriteIndented = true }) + "\n";
                if (action == "generate")
                {
                    Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                    File.WriteAllText(path, output);
                }
                else if (action == "verify")
                {
                    using var fixture = JsonDocument.Parse(File.ReadAllText(path));
                    var expected = fixture.RootElement;
                    if (expected.GetProperty("pin").GetString() != pin)
                        throw new InvalidDataException("avatar HTTP fixture pin differs");
                    var cases = expected.GetProperty("observations").Deserialize<List<Observation>>();
                    if (cases.Count != observations.Count)
                        throw new InvalidDataException("avatar HTTP fixture case count differs");
                    foreach (var (want, got) in cases.Zip(observations))
                    {
                        if (want == got) continue;
                        var fields = JsonSerializer.SerializeToElement(got);
                        foreach (var field in JsonSerializer.SerializeToElement(want).EnumerateObject())
                            if (!JsonElement.DeepEquals(field.Value, fields.GetProperty(field.Name)))
                                throw new InvalidDataException($"avatar HTTP {got.Label}: {field.Name} differs");
                    }
                }
                else throw new ArgumentException("expected generate or verify");
                Console.WriteLine($"avatar ({(mysql ? "MySQL" : "SQLite")}): {observations.Count} pinned HTTP replies {action} passed");
                return 0;
            }
            finally { http.Stop(true); }
        }
        finally { Directory.Delete(dir, true); }
    }

    const string User = "11111111-1111-4111-8111-111111111111";
    static string Form(string method, string more = "") => $"VERSIONMIN=0&VERSIONMAX=0&METHOD={method}&UserID={User}{more}";
    delegate void Send(string label, string body, string refuse = null, bool shard = false);

    static void SqliteCases(Send Post)
    {
        Post("absent", Form("getavatar"));
        Post("missing-user", "METHOD=getavatar");
        Post("bad-user", "METHOD=setavatar&UserID=bad");
        Post("missing-method", $"UserID={User}");
        Post("unknown-method", Form("GetAvatar"));
        Post("shard", Form("getavatar"), shard: true);
        Post("reset-absent", Form("resetavatar"));
        Post("set-bake-index", Form("setitems", "&Names[]=Bake%3AHead&Values[]=cached&Names[]=BakeHash%3AHead&Values[]=hash&Names[]=BakeCOFVersion&Values[]=3&Names[]=BakeSize&Values[]=512&Names[]=BakeUpdated&Values[]=99&Names[]=bake&Values[]=old"));
        Post("only-items", Form("getavatar"));
        Post("first-appearance", Form("setavatar", "&AvatarType=1&AvatarHeight=1%2C8&Wearable+4%3A0=shirt&_ap_1=item&Notes=A%26B%3C"));
        Post("first-read", Form("getavatar"));
        Post("replace", Form("setavatar", "&AvatarType=2&AvatarHeight=12&Bake%3AHead=new"));
        Post("read-replaced", Form("getavatar"));
        Post("mixed-lists", Form("setitems", "&Names[]=Extra&Values=text"));
        Post("different-counts", Form("setitems", "&Names[]=Extra&Names[]=Missing&Values[]=text"));
        Post("two-items", Form("setitems", "&Names[]=Extra&Names[]=Other&Values[]=value&Values[]=abc"));
        Post("scalar-remove", Form("removeitems", "&Names=Extra"));
        Post("remove", Form("removeitems", "&Names[]=Other&Names[]=Missing&Names[]=BakeHash%3AHead"));
        Post("read-items", Form("getavatar"));
        Post("fail-later-write", Form("setavatar", "&Serial=3&Reject=bad"), "Reject");
        Post("read-after-failure", Form("getavatar"));
        Post("raw-values", Form("setavatar", "&AvatarType=01&AvatarHeight=bad&List[]=a&List[]=b"));
        Post("read-raw-values", Form("getavatar"));
        Post("fail-type-write", Form("setavatar", "&Serial=4"), "AvatarType");
        Post("read-after-type-failure", Form("getavatar"));
        Post("partial-setitems", Form("setitems", "&Names[]=First&Values[]=kept&Names[]=Reject&Values[]=bad"), "Reject");
        Post("read-partial-setitems", Form("getavatar"));
        Post("reset", Form("resetavatar"));
        Post("read-reset", Form("getavatar"));
        foreach (var (label, input) in new[] {
            ("integer-group", "0,001.5"), ("repeated-group", "0,0,2.5"),
            ("fraction-group", "1.5,0"), ("out-of-range-group", "1,2.5"),
            ("ascii-space", "\t1,8\r"), ("no-break-space", "\u00a01.5"),
            ("next-line", "\u00851.5")
        })
        {
            Post($"height-{label}", Form("setavatar", "&AvatarHeight=" + Uri.EscapeDataString(input)));
            Post($"read-height-{label}", Form("getavatar"));
        }
        Post("ascii-space-type", Form("setavatar", "&AvatarType=%205"));
        Post("read-ascii-space-type", Form("getavatar"));
        Post("no-break-space-type", Form("setavatar", "&AvatarType=%C2%A05"));
        Post("read-no-break-space-type", Form("getavatar"));
    }

    // A private copy of the migrated table preserves its real varchar(32)
    // constraint. Dropping just this copy supplies a failed DELETE statement.
    sealed class MysqlTable : IDisposable
    {
        readonly MySqlConnection connection;
        readonly string realm = "AvatarHttp_" + Guid.NewGuid().ToString("N");
        public IAvatarData Store { get; }

        public MysqlTable()
        {
            var ado = Environment.GetEnvironmentVariable("FERROAN_AVATAR_ADO_MYSQL");
            Store = new OpenSim.Data.MySQL.MySQLAvatarData(ado, realm);
            connection = new MySqlConnection(ado);
            connection.Open();
            using var create = new MySqlCommand($"CREATE TABLE {realm} LIKE Avatars", connection);
            create.ExecuteNonQuery();
        }

        public void Drop()
        {
            using var drop = new MySqlCommand($"DROP TABLE IF EXISTS {realm}", connection);
            drop.ExecuteNonQuery();
        }

        public void Dispose()
        {
            try { Drop(); } finally { connection.Dispose(); }
        }
    }

    static void MysqlCases(Send Post, MysqlTable table)
    {
        Post("cache-bake", Form("setitems", "&Names[]=Bake%3AHead&Values[]=cached"));
        Post("first-appearance", Form("setavatar", "&AvatarType=1&Wearable+4%3A0=shirt&Serial=1"));
        Post("oversize-name", Form("setavatar", "&AvatarType=1&Serial=2&" + new string('x', 33) + "=bad"));
        Post("read-after-failure", Form("getavatar"));
        table.Drop();
        Post("failed-delete", Form("removeitems", "&Names[]=Bake%3AHead"));
    }
}
