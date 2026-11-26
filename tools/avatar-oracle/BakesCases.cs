// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Net;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Region.CoreModules.Avatar.BakedTextures;
using OpenSim.Services.Interfaces;
using OpenSim.Framework;
using OpenSim.Framework.Console;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Server.Handlers.BakedTextures;

static class BakesCases
{
    sealed class ConsoleProbe() : CommandConsole("fixture")
    {
        public string Last;
        public override void Output(string message) => Last = message;
    }

    // Capture the region module's original upload before the handler disposes the stream.
    sealed class Handler(IBakedTextureService service, IServiceAuth auth) : BakesServerHandler(service, auth)
    {
        public (string Target, byte[] Body)? Upload;
        public override void Handle(IOSHttpRequest request, IOSHttpResponse response)
        {
            if (Upload == null && request.HttpMethod == "POST")
                Upload = (request.UriPath, ((MemoryStream)request.InputStream).ToArray());
            base.Handle(request, response);
        }
    }

    public static int Run(string action, string pin, uint port, string path)
    {
        const string id = "abcdef01-2345-4678-9abc-def012345678";
        const string route = "/bakes/" + id;
        var dir = Directory.CreateTempSubdirectory("xbakes-");
        var console = new ConsoleProbe();
        MainConsole.Instance = console;
        try
        {
            var config = new IniConfigSource();
            config.AddConfig("BakedTextureService").Set("BaseDirectory", dir.FullName);
            var service = new XBakes(config);
            var server = new BaseHttpServer(port) { ListenIPAddress = IPAddress.Loopback };
            var handler = new Handler(service, ServiceAuth.Create(config, "BakedTextureService"));
            server.AddSimpleStreamHandler(handler, true);
            server.Start(false, false);
            try
            {
                using var client = new HttpClient(new SocketsHttpHandler { UseProxy = false })
                    { BaseAddress = new Uri($"http://127.0.0.1:{port}"), Timeout = TimeSpan.FromSeconds(10) };
                var observations = new List<object>();
                object Stored() => dir.EnumerateFiles("*", SearchOption.AllDirectories)
                    .Select(f => new { path = Path.GetRelativePath(dir.FullName, f.FullName).Replace('\\', '/'),
                        bytes = File.ReadAllBytes(f.FullName) }).OrderBy(f => f.path).ToArray();
                void Send(string label, HttpMethod method, string target, byte[] data = null,
                    bool shard = false, long? declaredLength = null)
                {
                    using var request = new HttpRequestMessage(method, target);
                    if (data != null) request.Content = new ByteArrayContent(data);
                    if (shard) request.Headers.Add("X-SecondLife-Shard", "fixture");
                    if (declaredLength.HasValue)
                    {
                        // The pinned HTTP parser rejects this length before requesting the body.
                        request.Headers.ExpectContinue = true;
                        request.Content.Headers.ContentLength = declaredLength;
                    }
                    using var reply = client.SendAsync(request).GetAwaiter().GetResult();
                    observations.Add(new { label, method = method.Method, target, data, shard, declaredLength,
                        status = (int)reply.StatusCode, contentType = reply.Content.Headers.NonValidated.TryGetValues("Content-Type", out var types) ? types.Single() : null,
                        reply = reply.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult(), stored = Stored() });
                }
                void Delete(string command)
                {
                    console.Last = null;
                    console.RunCommand(command);
                    observations.Add(new { command, output = console.Last, stored = Stored() });
                }

                Send("missing file", HttpMethod.Get, route);
                Send("missing id", HttpMethod.Get, "/bakes");
                Send("missing id post", HttpMethod.Post, "/bakes", [1]);
                Send("short id", HttpMethod.Get, "/bakes/short");
                WebUtil.SetupHTTPClients(false, false, null, 4);
                config.AddConfig("XBakes").Set("URL", client.BaseAddress.GetLeftPart(UriPartial.Authority));
                var module = new XBakesModule();
                module.Initialise(config);
                Util.FireAndForgetMethod = FireAndForgetMethod.RegressionTest;
                var texture = new AssetBase(UUID.Parse("12345678-9abc-4678-9123-456789abcdef"),
                    "Fixture bake", (sbyte)AssetType.Texture, UUID.Zero.ToString()) { Data = [0, 255, 13, 10] };
                module.Store(UUID.Parse(id), [new WearableCacheItem { TextureIndex = 8, CacheId = texture.FullID, TextureAsset = texture }]);
                var upload = handler.Upload ?? throw new InvalidOperationException("region did not post its bake");
                if (upload.Target != route || !File.ReadAllBytes(dir.EnumerateFiles("*", SearchOption.AllDirectories).Single().FullName)
                        .SequenceEqual(upload.Body))
                    throw new InvalidDataException("stored bake differs from the region upload");
                Send("region upload", HttpMethod.Post, upload.Target, upload.Body);
                Send("read", HttpMethod.Get, route);
                Send("replace", HttpMethod.Post, route, [0, 255, 13, 10]);
                Send("read first parameter", HttpMethod.Get, route + "/other?query=1");
                Send("refuse shard", HttpMethod.Post, route, [1], shard: true);
                Send("method", HttpMethod.Put, route);
                Send("oversize", HttpMethod.Post, route, [], declaredLength: 64L * 1024 * 1204 + 1);
                Send("after refusal", HttpMethod.Get, route);
                Delete("delete bakes");
                Delete("delete bakes " + id);
                Delete("delete bakes " + id);
                Send("after delete", HttpMethod.Get, route);
                Send("empty file", HttpMethod.Post, route, []);
                Send("read empty file", HttpMethod.Get, route);

                string json = JsonSerializer.Serialize(new { pin, observations },
                    new JsonSerializerOptions { WriteIndented = true }).ReplaceLineEndings("\n") + "\n";
                if (action == "generate") File.WriteAllText(path, json);
                else if (action != "verify" || File.ReadAllText(path).ReplaceLineEndings("\n") != json)
                    throw new InvalidDataException("XBakes HTTP, command or stored file differs");
                System.Console.WriteLine($"XBakes: {observations.Count} Pin8 observations match");
                return 0;
            }
            finally { server.Stop(true); }
        }
        finally { dir.Delete(true); }
    }
}
