// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
// Calls unmodified pinned Tranquillity connector assemblies. The loopback
// responder injects statuses; these are fault tests, not production traffic.

using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenMetaverse.StructuredData;
using OpenSim.Framework;
using OpenSim.Services.Connectors;
using OpenSim.Services.Interfaces;
using GridRegion = OpenSim.Services.Interfaces.GridRegion;

namespace Ferroan.HttpOracle;

static class StatusCases
{
    static readonly UUID Agent = new("00000000-0000-4000-8000-000000000001");
    static readonly UUID Region = new("00000000-0000-4000-8000-000000000002");
    static readonly UUID Session = new("00000000-0000-4000-8000-000000000004");
    static readonly string[] Callers = ["grid-register", "grid-deregister", "presence-getagent", "presence-getagents", "presence-logout", "presence-report", "griduser-loggedout", "griduser-setposition", "griduser-getgriduserinfo"];
    static readonly int[] Statuses = [200, 201, 302, 500];
    static readonly JsonSerializerOptions Options = new() { WriteIndented = true, NewLine = "\n" };

    public static void Run(string directory, string revision, bool generate)
    {
        OpenSim.Framework.WebUtil.SetupHTTPClients(false, false, null, 16);
        Directory.CreateDirectory(directory);
        List<string> manifest = ["format 1", "revision " + revision, "source real Tranquillity connectors; controlled loopback HTTP status injection", ""];
        foreach (string caller in Callers)
        foreach (int status in Statuses)
        {
            string name = caller + "-" + status;
            string first = Execute(caller, status).ToJsonString(Options) + "\n";
            string second = Execute(caller, status).ToJsonString(Options) + "\n";
            if (first != second) throw new InvalidOperationException(name + " is not deterministic");
            CheckOrWrite(Path.Combine(directory, name + ".json"), first, generate);
            manifest.Add(name);
        }
        string simulation = SimulationTransfer().ToJsonString(Options);
        if (simulation != SimulationTransfer().ToJsonString(Options))
            throw new InvalidOperationException("simulation-transfer is not deterministic");
        CheckOrWrite(Path.Combine(directory, "simulation-transfer.json"), simulation + "\n", generate);
        manifest.Add("simulation-transfer");
        CheckOrWrite(Path.Combine(directory, "manifest.txt"), string.Join("\n", manifest) + "\n", generate);
        Console.WriteLine($"http-status: {(generate ? "generated" : "verified")} {Callers.Length * Statuses.Length} form cases and the simulation-transfer fixture");
    }

    static void CheckOrWrite(string path, string value, bool generate)
    {
        if (generate) File.WriteAllText(path, value, new UTF8Encoding(false));
        else if (!File.Exists(path) || File.ReadAllText(path) != value)
            throw new InvalidOperationException(Path.GetFileName(path) + " differs from the pinned connector");
    }

    static JsonObject Execute(string caller, int status)
    {
        string body = Response(caller);
        using TcpListener listener = new(IPAddress.Loopback, 0);
        listener.Start(1);
        string uri = "http://127.0.0.1:" + ((IPEndPoint)listener.LocalEndpoint).Port;
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(15));
        Task<JsonObject> capture = Capture(listener, status, body, timeout.Token);
        Task<JsonObject> connector = Task.Run(() => Invoke(caller, uri));
        JsonObject outcome = connector.WaitAsync(timeout.Token).GetAwaiter().GetResult();
        JsonObject request = capture.WaitAsync(timeout.Token).GetAwaiter().GetResult();
        return new JsonObject {
            ["caller"] = caller,
            ["request"] = request,
            ["injected_response"] = new JsonObject { ["status"] = status, ["content_type"] = "application/xml", ["body"] = body, ["location"] = null },
            ["outcome"] = outcome
        };
    }

    static JsonObject Invoke(string caller, string uri)
    {
        object value;
        string kind;
        bool accepted;
        switch (caller)
        {
            case "grid-register":
                GridRegion region = new() { RegionID = Region, RegionName = "Fixture Region", RegionLocX = 256000, RegionLocY = 256000, InternalEndPoint = new IPEndPoint(IPAddress.Loopback, 9000), ExternalHostName = "192.0.2.1", HttpPort = 9000, ServerURI = "http://192.0.2.1:9000/" };
                string answer = new GridServicesConnector(uri).RegisterRegion(UUID.Zero, region);
                value = answer.Replace(uri, "http://192.0.2.1:8003", StringComparison.Ordinal); kind = "string"; accepted = answer.Length == 0; break;
            case "grid-deregister": value = new GridServicesConnector(uri).DeregisterRegion(Region); kind = "bool"; accepted = (bool)value; break;
            case "presence-logout": value = new PresenceServicesConnector(uri).LogoutAgent(Session); kind = "bool"; accepted = (bool)value; break;
            case "presence-report": value = new PresenceServicesConnector(uri).ReportAgent(Session, Region); kind = "bool"; accepted = (bool)value; break;
            case "presence-getagent":
                PresenceInfo presence = new PresenceServicesConnector(uri).GetAgent(Session);
                value = presence == null ? null : new { user_id = presence.UserID, region_id = presence.RegionID.ToString() }; kind = "presence"; accepted = presence != null; break;
            case "presence-getagents":
                PresenceInfo[] people = new PresenceServicesConnector(uri).GetAgents([Agent.ToString()]);
                value = people?.Select(p => new { user_id = p.UserID, region_id = p.RegionID.ToString() }).ToArray(); kind = "presence-array"; accepted = people?.Length > 0; break;
            case "griduser-loggedout": value = new GridUserServicesConnector(uri).LoggedOut(Agent.ToString(), Session, Region, new Vector3(128, 128, 25), Vector3.Zero); kind = "bool"; accepted = (bool)value; break;
            case "griduser-setposition": value = new GridUserServicesConnector(uri).SetLastPosition(Agent.ToString(), Session, Region, new Vector3(128, 128, 25), Vector3.Zero); kind = "bool"; accepted = (bool)value; break;
            case "griduser-getgriduserinfo":
                GridUserInfo user = new GridUserServicesConnector(uri).GetGridUserInfo(Agent.ToString());
                value = user == null ? null : new { user_id = user.UserID, online = user.Online, last_region_id = user.LastRegionID.ToString(), last_position = user.LastPosition.ToString() }; kind = "griduser"; accepted = user != null; break;
            default: throw new InvalidOperationException("unknown connector");
        }
        return new JsonObject { ["kind"] = kind, ["value"] = JsonSerializer.SerializeToNode(value), ["accepted"] = accepted };
    }

    // Pin8 ChildAgentDataUpdate.cs (AgentData.Pack), WebUtil.cs (ServiceOSDRequest)
    // and the JSON writer that the simulation connector uses for the request.
    static JsonObject SimulationTransfer()
    {
        AgentData agent = new() {
            AlwaysRun = true, ActiveGroupID = Region, ActiveGroupName = "Fixture group",
            GodData = new OSDMap { ["ViewerUiIsGod"] = true },
            Anims = [new OpenSim.Framework.Animation(Agent, 7, Region), new OpenSim.Framework.Animation(Session, 0, UUID.Zero)]
        };
        OSDMap packed = agent.Pack(new EntityTransferContext());
        OSDMap fields = new();
        foreach (string key in new[] { "always_run", "active_group_id", "active_group_name", "god_data", "god_level", "animations" })
            fields[key] = packed[key];
        fields["empty_binary"] = OSD.FromBinary([]);
        JsonObject result = new() { ["packed"] = JsonNode.Parse(OSDParser.SerializeJsonToBytes(fields)) };
        foreach ((int status, string phrase) in new[] { (500, "Internal Server Error"), (405, "Neighbour refused"), (499, "") })
        {
            using TcpListener listener = new(IPAddress.Loopback, 0);
            listener.Start(1);
            string uri = "http://127.0.0.1:" + ((IPEndPoint)listener.LocalEndpoint).Port;
            using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(10));
            var capture = Capture(listener, status, "", timeout.Token, phrase);
            var reply = WebUtil.ServiceOSDRequest(uri, fields, "QUERYACCESS", 5000, false, false);
            capture.WaitAsync(timeout.Token).GetAwaiter().GetResult();
            result[status.ToString()] = reply["Message"].AsString();
        }
        // Reserve the ephemeral port without listening. Record whether C# discloses it.
        using Socket reserved = new(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp);
        reserved.Bind(new IPEndPoint(IPAddress.Loopback, 0));
        string endpoint = reserved.LocalEndPoint.ToString();
        var refused = WebUtil.ServiceOSDRequest("http://" + endpoint, fields, "QUERYACCESS", 5000, false, false);
        result["refused_mentions_endpoint"] = refused["Message"].AsString().Contains(endpoint, StringComparison.Ordinal);
        using TcpListener silent = new(IPAddress.Loopback, 0);
        silent.Start(1);
        var timedOut = WebUtil.ServiceOSDRequest("http://" + silent.LocalEndpoint, fields, "QUERYACCESS", 150, false, false);
        result["timeout_reason"] = timedOut["Message"].AsString();
        return result;
    }

    static string Response(string caller)
    {
        string contents = caller switch {
            "grid-register" or "grid-deregister" => "<Result>Success</Result>",
            "presence-getagent" => $"<result type=\"List\"><UserID>{Agent}</UserID><RegionID>{Region}</RegionID></result>",
            "presence-getagents" => $"<presence0 type=\"List\"><UserID>{Agent}</UserID><RegionID>{Region}</RegionID></presence0>",
            "griduser-getgriduserinfo" => $"<result type=\"List\"><UserID>{Agent}</UserID><Online>True</Online><LastRegionID>{Region}</LastRegionID><LastPosition>&lt;128, 128, 25&gt;</LastPosition><LastLookAt>&lt;0, 0, 0&gt;</LastLookAt></result>",
            _ => "<result>Success</result>"
        };
        return "<?xml version=\"1.0\"?><ServerResponse>" + contents + "</ServerResponse>";
    }

    static async Task<JsonObject> Capture(TcpListener listener, int status, string body, CancellationToken token, string customReason = null)
    {
        using TcpClient socket = await listener.AcceptTcpClientAsync(token);
        using NetworkStream stream = socket.GetStream();
        List<byte> header = [];
        byte[] one = new byte[1];
        while (header.Count < 16384)
        {
            if (await stream.ReadAsync(one, token) != 1) throw new IOException("EOF before request headers");
            header.Add(one[0]);
            if (header.Count >= 4 && header.TakeLast(4).SequenceEqual(new byte[] {13, 10, 13, 10})) break;
        }
        if (header.Count >= 16384) throw new IOException("request headers exceed limit");
        string[] lines = Encoding.ASCII.GetString(header.ToArray()).Split("\r\n");
        string[] start = lines[0].Split(' ');
        Dictionary<string,string> headers = new(StringComparer.OrdinalIgnoreCase);
        foreach (string line in lines.Skip(1).Where(line => line.Contains(':'))) { int colon = line.IndexOf(':'); headers[line[..colon]] = line[(colon+1)..].Trim(); }
        int length = int.Parse(headers["Content-Length"]);
        if (length < 0 || length > 131072) throw new IOException("request body exceeds limit");
        byte[] request = new byte[length];
        await stream.ReadExactlyAsync(request, token);
        byte[] reply = Encoding.UTF8.GetBytes(body);
        string reason = customReason ?? (status switch { 200 => "OK", 201 => "Created", 302 => "Found", _ => "Internal Server Error" });
        byte[] responseHeader = Encoding.ASCII.GetBytes($"HTTP/1.1 {status} {reason}\r\nContent-Type: application/xml\r\nContent-Length: {reply.Length}\r\nConnection: close\r\n\r\n");
        await stream.WriteAsync(responseHeader, token);
        await stream.WriteAsync(reply, token);
        return new JsonObject { ["method"] = start[0], ["path"] = start[1], ["content_type"] = headers["Content-Type"], ["body"] = Encoding.UTF8.GetString(request) };
    }
}
