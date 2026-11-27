// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Net;
using System.Text.Json;
using System.Text.Json.Nodes;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Server.Handlers.Appearance;
using OpenSim.Services.AssetService;
using OpenSim.Services.AvatarService;
using OpenSim.Services.Interfaces;

static class ServiceCases
{
    const string Agent = "a7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    const string Bake = "b7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    const string Other = "c7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    const string Empty = "d7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    static readonly byte[] BakeBytes = [0xFF, 0x4F, 0xFF, 0x51, 0x01, 0x02, 0x03, 0x04];
    static readonly byte[] OtherBytes = [0x00, 0x11];

    sealed class Wired(IConfigSource config, IAvatarService avatars, IAssetService assets)
        : AppearanceService(config, avatars, assets);

    public static int Run(string action, string pin, ushort port, string path)
    {
        if (action != "generate" && action != "verify")
            throw new ArgumentException("expected generate or verify");
        string fixturePath = Path.GetFullPath(path);
        DirectoryInfo appearanceDir = Directory.GetParent(fixturePath) ?? throw new InvalidOperationException("fixture directory");
        DirectoryInfo fixturesDir = appearanceDir.Parent ?? throw new InvalidOperationException("fixtures directory");
        string root = fixturesDir.Parent?.FullName ?? throw new InvalidOperationException("repository root");
        string dir = Directory.CreateTempSubdirectory("appearance-http-").FullName;
        try
        {
            string avatarDb = Path.Combine(dir, "Avatar.db");
            string assetDb = Path.Combine(dir, "AssetStore.db");
            File.Copy(Path.Combine(root, "fixtures", "db", "sqlite", "Avatar.db"), avatarDb);
            File.Copy(Path.Combine(root, "fixtures", "db", "sqlite", "AssetStore.db"), assetDb);
            string sqlite = Path.Combine(AppContext.BaseDirectory, "OpenSim.Data.SQLite.dll");
            if (!File.Exists(sqlite))
                throw new InvalidOperationException("OpenSim.Data.SQLite.dll is not next to the oracle");

            var config = new IniConfigSource();
            IConfig avatarsConfig = config.AddConfig("AvatarService");
            avatarsConfig.Set("StorageProvider", sqlite + ":SQLiteAvatarData");
            avatarsConfig.Set("ConnectionString", "Data Source=" + avatarDb + ";Version=3;Pooling=False;");
            avatarsConfig.Set("Realm", "Avatars");
            IConfig assetsConfig = config.AddConfig("AssetService");
            assetsConfig.Set("StorageProvider", sqlite + ":SQLiteAssetData");
            assetsConfig.Set("ConnectionString", "Data Source=" + assetDb + ";Version=3;Pooling=False;");
            assetsConfig.Set("AssetLoaderEnabled", "false");
            config.AddConfig("AppearanceService").Set("LocalServiceModule", "OpenSim.Services.AvatarService.dll:AppearanceService");

            var avatars = new AvatarService(config);
            var assets = new AssetService(config);
            Store(assets, Bake, BakeBytes);
            Store(assets, Other, OtherBytes);
            Store(assets, Empty, []);
            var data = new AvatarData { Data = new Dictionary<string, string> { ["Bake:Head"] = Bake, ["Bake:Upper"] = Empty } };
            if (!avatars.SetAvatar(UUID.Parse(Agent), data))
                throw new InvalidOperationException("avatar store failed");
            byte[] loaded = assets.Get(Bake)?.Data ?? throw new InvalidOperationException("stored bake is missing");
            if (!loaded.AsSpan().SequenceEqual(BakeBytes))
                throw new InvalidOperationException("stored bake bytes differ");

            JsonArray seedRows = Rows(avatars);
            var server = new BaseHttpServer(port) { ListenIPAddress = IPAddress.Loopback };
            server.AddSimpleStreamHandler(
                new AppearanceServerHandler(new Wired(config, avatars, assets), ServiceAuth.Create(config, "AppearanceService")),
                true);
            server.Start(false, false);
            try
            {
                using var client = new HttpClient(new SocketsHttpHandler { UseProxy = false })
                {
                    BaseAddress = new Uri("http://127.0.0.1:" + port + "/"),
                    Timeout = TimeSpan.FromSeconds(10)
                };
                var observations = new JsonArray();
                string hit = "texture/" + Agent + "/head/" + Bake;
                Send(client, observations, "hit", HttpMethod.Get, hit, false, null);
                Send(client, observations, "channel case", HttpMethod.Get, "texture/" + Agent + "/HEAD/" + Bake, false, null);
                Send(client, observations, "unknown token", HttpMethod.Get, "texture/" + Agent + "/torso/" + Bake, false, null);
                Send(client, observations, "mismatched uuid", HttpMethod.Get, "texture/" + Agent + "/head/" + Other, false, null);
                Send(client, observations, "empty asset", HttpMethod.Get, "texture/" + Agent + "/upper/" + Empty, false, null);
                if (!assets.Delete(Bake))
                    throw new InvalidOperationException("bake delete failed");
                Send(client, observations, "missing asset", HttpMethod.Get, hit, false, Bake);
                Send(client, observations, "method", HttpMethod.Post, hit, false, null);
                Send(client, observations, "shard", HttpMethod.Get, hit, true, null);
                Send(client, observations, "shard before method", HttpMethod.Post, hit, true, null);
                Send(client, observations, "bad uuid", HttpMethod.Get, "texture/" + Agent + "/head/not-a-uuid", false, null);
                Send(client, observations, "short path", HttpMethod.Get, "texture/" + Agent + "/head", false, null);
                Send(client, observations, "bare path", HttpMethod.Get, "texture", false, null);

                JsonArray indexAfter = Rows(avatars);
                if (seedRows.ToJsonString() != indexAfter.ToJsonString())
                    throw new InvalidOperationException("appearance GET changed avatar rows");
                var document = new JsonObject
                {
                    ["pin"] = pin,
                    ["backend"] = "sqlite",
                    ["seed"] = new JsonObject
                    {
                        ["agent"] = Agent,
                        ["rows"] = seedRows,
                        ["assets"] = new JsonArray
                        {
                            AssetSeed(Bake, BakeBytes),
                            AssetSeed(Other, OtherBytes),
                            AssetSeed(Empty, [])
                        }
                    },
                    ["indexAfter"] = indexAfter,
                    ["assetsAfter"] = new JsonArray { AssetNow(assets, Bake), AssetNow(assets, Other), AssetNow(assets, Empty) },
                    ["observations"] = observations
                };
                string json = JsonSerializer.Serialize(document, new JsonSerializerOptions { WriteIndented = true }).ReplaceLineEndings("\n") + "\n";
                if (action == "generate")
                {
                    Directory.CreateDirectory(appearanceDir.FullName);
                    File.WriteAllText(fixturePath, json);
                }
                else if (File.ReadAllText(fixturePath).ReplaceLineEndings("\n") != json)
                    throw new InvalidDataException("appearance fixture differs");
                Console.WriteLine("appearance: " + observations.Count + " SQLite HTTP replies " + action + " passed");
                return 0;
            }
            finally { server.Stop(true); }
        }
        finally { Directory.Delete(dir, true); }
    }

    static void Store(AssetService assets, string id, byte[] data)
    {
        var asset = new AssetBase(UUID.Parse(id), "Fixture bake", (sbyte)AssetType.Texture, UUID.Zero.ToString()) { Data = data };
        if (assets.Store(asset) == UUID.Zero.ToString())
            throw new InvalidOperationException("asset store failed for " + id);
    }

    static void Send(HttpClient client, JsonArray observations, string label, HttpMethod method, string target, bool shard, string dropAsset)
    {
        using var request = new HttpRequestMessage(method, target);
        if (shard)
            request.Headers.TryAddWithoutValidation("X-SecondLife-Shard", "fixture");
        using HttpResponseMessage reply = client.Send(request);
        string contentType = "";
        if (reply.Content.Headers.NonValidated.TryGetValues("Content-Type", out var types))
            contentType = string.Join(", ", types);
        byte[] body = reply.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult();
        var row = new JsonObject
        {
            ["label"] = label,
            ["method"] = method.Method,
            ["target"] = "/" + target,
            ["shard"] = shard,
            ["status"] = (int)reply.StatusCode,
            ["contentType"] = contentType,
            ["reply"] = Convert.ToBase64String(body)
        };
        if (dropAsset != null)
            row["dropAsset"] = dropAsset;
        observations.Add(row);
    }

    static JsonArray Rows(IAvatarService avatars)
    {
        AvatarData data = avatars.GetAvatar(UUID.Parse(Agent));
        var rows = new JsonArray();
        foreach (KeyValuePair<string, string> pair in data.Data.OrderBy(pair => pair.Key, StringComparer.Ordinal))
            rows.Add(new JsonObject { ["name"] = pair.Key, ["value"] = pair.Value });
        return rows;
    }

    static JsonObject AssetSeed(string id, byte[] data)
    {
        return new JsonObject { ["id"] = id, ["data"] = Convert.ToBase64String(data) };
    }

    static JsonObject AssetNow(IAssetService assets, string id)
    {
        byte[] data = assets.Get(id)?.Data;
        bool present = data is { Length: > 0 };
        return new JsonObject
        {
            ["id"] = id,
            ["data"] = present ? Convert.ToBase64String(data) : ""
        };
    }
}
