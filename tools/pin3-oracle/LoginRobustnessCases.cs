// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Execute the pinned login, presence and agent-handler code. No implementation is copied here.
// Sources: Scene.NewUserConnection, PresenceDetector, PresenceServicesConnector,
// ServiceAuth.Create, AgentSimpleHandler and ControlPlaneAccess.Authorize.
// KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5.

using System.Collections.Specialized;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Text.Json.Nodes;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Region.CoreModules.ServiceConnectorsOut.Presence;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Server.Handlers.Base;
using OpenSim.Server.Handlers.Simulation;
using OpenSim.Services.Connectors;
using OpenSim.Services.Interfaces;
using OpenSim.Tests.Common;

namespace Ferroan.Pin3Oracle;

static class LoginRobustnessCases
{
    public static JsonObject Capture()
    {
        return new JsonObject
        {
            ["logout_region"] = LogoutRegion(),
            ["logins_disabled"] = LoginsDisabled(),
            ["auth_types"] = AuthTypes(),
            ["refused_post"] = RefusedPost()
        };
    }

    static JsonObject LogoutRegion()
    {
        using TcpListener listener = new(IPAddress.Loopback, 0);
        listener.Start(2);
        string uri = "http://127.0.0.1:" + ((IPEndPoint)listener.LocalEndpoint).Port;
        const string response = "<?xml version=\"1.0\"?><ServerResponse><result>Success</result></ServerResponse>";
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(10));
        Task<JsonArray> pending = ReadAll(listener, 2, response, timeout.Token);
        Scene scene = new SceneHelpers().SetupScene(
            "Pin3 logout region", new UUID("00000000-0000-4000-8000-0000000010a0"), 1000, 1000);
        PresenceDetector detector = new(new PresenceServicesConnector(uri));
        detector.AddRegion(scene);
        detector.RemoveRegion(scene);
        JsonArray calls = pending.WaitAsync(timeout.Token).GetAwaiter().GetResult();
        return new JsonObject
        {
            ["region"] = scene.RegionInfo.RegionID.ToString(),
            ["calls"] = calls
        };
    }

    static JsonObject LoginsDisabled()
    {
        UUID agent = new("00000000-0000-4000-8000-0000000000b1");
        Scene scene = new SceneHelpers().SetupScene(
            "Pin3 logins disabled", new UUID("00000000-0000-4000-8000-0000000010b0"), 1000, 1000);
        AgentCircuitData circuit = SceneHelpers.GenerateAgentData(agent);
        circuit.SessionID = new UUID("00000000-0000-4000-8000-0000000002b1");
        circuit.SecureSessionID = new UUID("00000000-0000-4000-8000-0000000003b1");
        IPresenceService presence = ((LocalPresenceServicesConnector)scene.PresenceService).m_PresenceService;
        if (!presence.LoginAgent(agent.ToString(), circuit.SessionID, circuit.SecureSessionID))
            throw new InvalidOperationException("reference presence login failed");
        scene.LoginsEnabled = false;
        bool accepted = scene.NewUserConnection(circuit, (uint)TeleportFlags.ViaLogin, null, out string reason);
        return new JsonObject
        {
            ["accepted"] = accepted,
            ["reason"] = reason,
            ["presence_session"] = presence.GetAgent(circuit.SessionID) is not null,
            ["circuit"] = scene.AuthenticateHandler.GetAgentCircuitData(agent) is not null
        };
    }

    static JsonObject AuthTypes()
    {
        JsonArray rows = [];
        foreach (string auth in new[] { "None", "none", "", "TrustedGridAuthentication", "BasicHttpAuthentication" })
        {
            string ini = "[Network]\nAuthType=" + auth + "\n";
            IConfigSource source = new IniConfigSource(new StringReader(ini));
            IServiceAuth created = ServiceAuth.Create(source, "PresenceService");
            JsonArray names = [];
            if (created is CompoundAuthentication compound)
            {
                foreach (IServiceAuth authenticator in compound.GetAuthentors())
                    names.Add(authenticator.Name);
            }
            NameValueCollection headers = new();
            created?.AddAuthorization(headers);
            rows.Add(new JsonObject
            {
                ["auth"] = auth,
                ["result"] = created is null ? "null" : created.GetType().Name,
                ["names"] = names,
                ["adds_authorization"] = headers["Authorization"] is not null
            });
        }
        return new JsonObject { ["network"] = rows };
    }

    static JsonArray RefusedPost()
    {
        ControlPlaneAccess access = new(new IniConfigSource(new StringReader("[Network]\n")));
        ISimulationService service = DispatchProxy.Create<ISimulationService, UnusedSimulationService>();
        AgentSimpleHandler handler = new(service, access);
        MethodInfo process = typeof(AgentSimpleHandler).GetMethod(
            "ProcessRequest", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("AgentSimpleHandler.ProcessRequest is missing");
        JsonArray rows = [];
        foreach ((string name, IPAddress address, string shard) in new (string, IPAddress, string)[]
        {
            ("untrusted", IPAddress.Parse("192.0.2.1"), null),
            ("script-marker", IPAddress.Loopback, "1")
        })
        {
            RefusedRequest request = new(
                "/agent/00000000-0000-4000-8000-000000000001/", "POST", address);
            if (shard is not null)
                request.Headers["X-SecondLife-Shard"] = shard;
            TestOSHttpResponse response = new();
            process.Invoke(handler, new object[] { request, response });
            rows.Add(new JsonObject
            {
                ["name"] = name,
                ["status"] = response.StatusCode,
                ["content_type"] = response.ContentType,
                ["keep_alive"] = response.KeepAlive,
                ["body_length"] = response.RawBuffer?.Length ?? -1
            });
        }
        return rows;
    }

    static async Task<JsonArray> ReadAll(TcpListener listener, int count, string response, CancellationToken token)
    {
        JsonArray rows = [];
        for (int i = 0; i < count; i++)
            rows.Add(await ReadRequest(listener, response, token));
        return rows;
    }

    static async Task<JsonObject> ReadRequest(TcpListener listener, string response, CancellationToken token)
    {
        using TcpClient socket = await listener.AcceptTcpClientAsync(token);
        using NetworkStream stream = socket.GetStream();
        List<byte> header = [];
        byte[] one = new byte[1];
        while (header.Count < 16384)
        {
            if (await stream.ReadAsync(one, token) != 1)
                throw new IOException("request header EOF");
            header.Add(one[0]);
            if (header.Count >= 4 && header.TakeLast(4).SequenceEqual(new byte[] { 13, 10, 13, 10 }))
                break;
        }
        if (header.Count >= 16384)
            throw new IOException("request header limit");
        string[] lines = Encoding.ASCII.GetString(header.ToArray()).Split("\r\n");
        Dictionary<string, string> headers = new(StringComparer.OrdinalIgnoreCase);
        foreach (string line in lines.Skip(1).Where(line => line.Contains(':')))
        {
            int split = line.IndexOf(':');
            headers[line[..split]] = line[(split + 1)..].Trim();
        }
        int length = int.Parse(headers["Content-Length"]);
        if (length < 0 || length > 65536)
            throw new IOException("request body limit");
        byte[] body = new byte[length];
        await stream.ReadExactlyAsync(body, token);
        byte[] reply = Encoding.UTF8.GetBytes(response);
        byte[] replyHeader = Encoding.ASCII.GetBytes(
            $"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {reply.Length}\r\nConnection: close\r\n\r\n");
        await stream.WriteAsync(replyHeader, token);
        await stream.WriteAsync(reply, token);
        return new JsonObject
        {
            ["method"] = lines[0].Split(' ')[0],
            ["path"] = lines[0].Split(' ')[1],
            ["content_type"] = headers["Content-Type"],
            ["request"] = Encoding.UTF8.GetString(body),
            ["status"] = 200,
            ["response"] = response
        };
    }
}

public class UnusedSimulationService : DispatchProxy
{
    protected override object Invoke(MethodInfo targetMethod, object[] args) =>
        throw new InvalidOperationException(targetMethod.Name);
}

sealed class RefusedRequest : IOSHttpRequest
{
    public RefusedRequest(string path, string method, IPAddress address)
    {
        UriPath = path;
        HttpMethod = method;
        RemoteIPEndPoint = new IPEndPoint(address, 9);
    }

    public string[] AcceptTypes => throw new NotImplementedException();
    public Encoding ContentEncoding => throw new NotImplementedException();
    public long ContentLength => 0;
    public long ContentLength64 => 0;
    public string ContentType => "application/json";
    public bool HasEntityBody => false;
    public NameValueCollection Headers { get; } = new();
    public string HttpMethod { get; }
    public Stream InputStream => Stream.Null;
    public bool IsSecured => false;
    public bool KeepAlive => false;
    public NameValueCollection QueryString => throw new NotImplementedException();
    public System.Collections.Hashtable Query => throw new NotImplementedException();
    public HashSet<string> QueryFlags => throw new NotImplementedException();
    public Dictionary<string, string> QueryAsDictionary => throw new NotImplementedException();
    public string RawUrl => UriPath;
    public IPEndPoint RemoteIPEndPoint { get; }
    public IPEndPoint LocalIPEndPoint => throw new NotImplementedException();
    public Uri Url => throw new NotImplementedException();
    public string UriPath { get; }
    public string UserAgent => throw new NotImplementedException();
    public double ArrivalTS => 0;
}
