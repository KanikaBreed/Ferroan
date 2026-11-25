// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Net;
using System.Text;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Server.Handlers.Avatar;
using OpenSim.Services.AvatarService;

static class ServiceCases
{
    // Only the selected Store returns false. All other reads, writes and deletes
    // go through Pin8's SQLite plugin, including failed-appearance cleanup.
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

    public static int Run(string action, string pin, uint port, string path)
    {
        string dir = Directory.CreateTempSubdirectory("avatar-http-").FullName;
        try
        {
            using var db = new Database($"Data Source={Path.Combine(dir, "Avatar.db")};Pooling=False;");
            var probe = new StoreProbe(db);
            var config = new IniConfigSource();
            config.AddConfig("AvatarService");
            var http = new BaseHttpServer(port) { ListenIPAddress = IPAddress.Loopback };
            http.AddStreamHandler(new AvatarServerPostHandler(new Service(config, probe), ServiceAuth.Create(config, "AvatarService")));
            http.Start(false, false);
            try
            {
                using var client = new HttpClient(new SocketsHttpHandler { UseProxy = false }) { Timeout = TimeSpan.FromSeconds(10) };
                var observations = new List<Observation>();
                const string user = "11111111-1111-4111-8111-111111111111";
                string Form(string method, string more = "") => $"VERSIONMIN=0&VERSIONMAX=0&METHOD={method}&UserID={user}{more}";
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
                Post("absent", Form("getavatar"));
                Post("missing-user", "METHOD=getavatar");
                Post("bad-user", "METHOD=setavatar&UserID=bad");
                Post("missing-method", $"UserID={user}");
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
                Console.WriteLine($"avatar: {observations.Count} pinned HTTP replies {action} passed");
                return 0;
            }
            finally { http.Stop(true); }
        }
        finally { Directory.Delete(dir, true); }
    }
}
