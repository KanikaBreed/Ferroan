/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */
// Fixture setup adapted from Tests/OpenSim.Region.Framework.Tests/Scenes/Tests/LoginArrivalTests.cs
// and Tests/OpenSim.Region.ClientStack.LindenUDP.Tests/ConnectionCloseTests.cs.
// Executes Source/OpenSim.Region.Framework/Scenes/Scene.LoginArrivals.cs,
// Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs,
// Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/GridUser/ActivityDetector.cs,
// and Source/OpenSim.Services.Connectors/GridUser/GridUserServicesConnector.cs unchanged.
// KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json.Nodes;
using OpenMetaverse;
using OpenMetaverse.Packets;
using OpenSim.Framework;
using OpenSim.Region.ClientStack.LindenUDP;
using OpenSim.Region.CoreModules.ServiceConnectorsOut.GridUser;
using OpenSim.Region.CoreModules.ServiceConnectorsOut.Presence;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Connectors;
using OpenSim.Services.Interfaces;
using OpenSim.Tests.Common;

namespace Ferroan.Pin3Oracle;

static class LifecycleCases
{
    static readonly UUID LastRegion = new("00000000-0000-4000-8000-000000000900");
    static UUID Id(int n) => new($"00000000-0000-4000-8000-{n:x12}");

    sealed class GridUsers : IGridUserService
    {
        readonly object gate = new();
        public GridUserInfo User;
        public readonly List<UUID> StartedRegions = [];
        public bool StartupResult = true;
        public readonly List<string> Calls = [];
        public void Online(UUID agent)
        {
            lock (gate) User = new GridUserInfo { UserID = agent.ToString(), Online = true,
                LastRegionID = LastRegion, LastPosition = new Vector3(1, 2, 3), LastLookAt = Vector3.UnitY };
        }
        public bool IsOnline { get { lock (gate) return User?.Online == true; } }
        public GridUserInfo LoggedIn(string user) => throw new NotSupportedException();
        public bool LoggedOut(string user, UUID session, UUID region, Vector3 position, Vector3 look)
        {
            lock (gate)
            {
                Calls.Add("loggedout");
                if (User is null || User.UserID != user) return false;
                User.Online = false; User.LastRegionID = region; User.LastPosition = position; User.LastLookAt = look;
                return true;
            }
        }
        public bool LoggedOutRegion(UUID region)
        {
            StartedRegions.Add(region); return StartupResult;
        }
        public bool SetHome(string user, UUID region, Vector3 position, Vector3 look) => true;
        public bool SetLastPosition(string user, UUID session, UUID region, Vector3 position, Vector3 look) => true;
        public GridUserInfo GetGridUserInfo(string user)
        {
            lock (gate) { Calls.Add("getgriduserinfo"); return User?.UserID == user ? User : null; }
        }
        public GridUserInfo[] GetGridUserInfo(string[] users) => users.Select(GetGridUserInfo).ToArray();
    }

    sealed class Region
    {
        public readonly Scene Scene;
        public readonly GridUsers Grid = new();
        public readonly UUID Agent;
        public readonly AgentCircuitData Circuit;
        public Region(int n)
        {
            Agent = Id(n);
            Scene = new SceneHelpers().SetupScene("Pin3 fixture", Id(0x1000 + n), 1000, 1000);
            Scene.RegisterModuleInterface<IGridUserService>(Grid);
            Scene.LoginConnectWait = TimeSpan.FromMilliseconds(10);
            Circuit = SceneHelpers.GenerateAgentData(Agent);
            Circuit.SessionID = Id(0x2000 + n); Circuit.SecureSessionID = Id(0x3000 + n);
            Circuit.circuitcode = (uint)(0x4000 + n);
        }
        public IPresenceService Presence => ((LocalPresenceServicesConnector)Scene.PresenceService).m_PresenceService;
        public void Login()
        {
            Presence.LoginAgent(Agent.ToString(), Circuit.SessionID, Circuit.SecureSessionID);
            if (!Scene.NewUserConnection(Circuit, (uint)TeleportFlags.ViaLogin, null, out _))
                throw new InvalidOperationException("reference scene refused test login");
            Grid.Online(Agent);
        }
        public JsonObject State() => new() {
            ["circuit"] = Scene.AuthenticateHandler.GetAgentCircuitData(Agent) is not null,
            ["watched"] = Scene.LoginsAwaitingViewer.Contains(Agent),
            ["presence_session"] = Presence.GetAgent(Circuit.SessionID) is not null,
            ["scene_presence"] = Scene.GetScenePresence(Agent) is not null,
            ["online"] = Grid.IsOnline,
            ["last_region"] = Grid.User?.LastRegionID.ToString(),
            ["last_position"] = Vector(Grid.User?.LastPosition ?? Vector3.Zero),
            ["last_look"] = Vector(Grid.User?.LastLookAt ?? Vector3.Zero)
        };
        public void Expire() { Thread.Sleep(30); Scene.ExpireLoginArrivals(); }
    }

    static JsonArray Vector(Vector3 value) => new(value.X, value.Y, value.Z);
    static void Await(Func<bool> ready)
    {
        Stopwatch elapsed = Stopwatch.StartNew();
        while (!ready())
        {
            if (elapsed.Elapsed > TimeSpan.FromSeconds(10)) throw new TimeoutException("reference cleanup did not finish");
            Thread.Sleep(10);
        }
    }

    public static JsonObject Capture()
    {
        TestHelpers.DisableLogging();
        FireAndForgetMethod previous = Util.FireAndForgetMethod;
        Util.FireAndForgetMethod = FireAndForgetMethod.RegressionTest;
        try
        {
            JsonObject result = new();
            Region missing = new(1); missing.Login();
            JsonObject before = missing.State(); missing.Expire();
            result["never_connected"] = new JsonObject { ["before"] = before, ["after"] = missing.State() };

            Region connected = new(2); connected.Login();
            connected.Scene.AddNewAgent(new TestClient(connected.Circuit, connected.Scene), PresenceType.User);
            connected.Expire();
            result["connected_past_deadline"] = connected.State();
            connected.Scene.CloseAgent(connected.Agent, false);
            Await(() => !connected.Grid.IsOnline);
            result["removed_before_root"] = connected.State();

            Region arrived = new(3); arrived.Grid.Online(arrived.Agent);
            SceneHelpers.AddScenePresence(arrived.Scene, arrived.Circuit);
            arrived.Expire(); result["root_arrived"] = arrived.State();

            Region later = new(4); later.Login();
            UUID newSession = Id(0x5004);
            later.Presence.LoginAgent(later.Agent.ToString(), newSession, Id(0x6004));
            later.Expire();
            JsonObject laterState = later.State();
            laterState["replacement_presence_session"] = later.Presence.GetAgent(newSession) is not null;
            result["later_session"] = laterState;

            Region disabled = new(5); disabled.Scene.LoginConnectWait = TimeSpan.Zero;
            disabled.Login(); disabled.Expire(); result["zero_wait"] = disabled.State();

            result["close_handlers"] = CloseHandlers();
            result["startup_success"] = Startup(true);
            result["startup_old_robust_failure"] = Startup(false);
            result["pre_root_endpoint_reconnect"] = PreRootReconnect();
            return result;
        }
        finally { Util.FireAndForgetMethod = previous; }
    }

    static JsonObject PreRootReconnect()
    {
        Region region = new(0x90);
        region.Scene.LoginConnectWait = TimeSpan.FromMinutes(2);
        TestLLUDPServer udp = ClientStackHelpers.AddUdpServer(region.Scene);
        region.Login();
        void Open(int port)
        {
            UseCircuitCodePacket packet = new();
            packet.CircuitCode.Code = region.Circuit.circuitcode;
            packet.CircuitCode.ID = region.Agent;
            packet.CircuitCode.SessionID = region.Circuit.SessionID;
            byte[] wire = packet.ToBytes();
            UDPPacketBuffer buffer = new(new IPEndPoint(IPAddress.Loopback, port));
            buffer.DataLength = wire.Length;
            Buffer.BlockCopy(wire, 0, buffer.Data, 0, wire.Length);
            udp.PacketReceived(buffer);
        }
        Open(999);
        ScenePresence first = region.Scene.GetScenePresence(region.Agent);
        if (first is null) throw new InvalidOperationException("reference first UDP connection absent");
        JsonObject before = region.State();
        Open(999);
        JsonObject sameEndpoint = region.State();
        bool samePresence = ReferenceEquals(first, region.Scene.GetScenePresence(region.Agent));
        Open(1000);
        // #437 takes the login watcher out before CloseAgent and puts it back,
        // so a child reconnect does not schedule LogOutLogin. A root still
        // logs out. Wait a short bound and stop early if the user goes offline.
        Stopwatch settle = Stopwatch.StartNew();
        while (settle.Elapsed < TimeSpan.FromSeconds(2))
        {
            if (!region.Grid.IsOnline) break;
            Thread.Sleep(10);
        }
        ScenePresence second = region.Scene.GetScenePresence(region.Agent);
        JsonObject after = region.State();
        return new JsonObject {
            ["before"] = before, ["after"] = after,
            ["same_endpoint"] = sameEndpoint, ["same_endpoint_preserves_presence"] = samePresence,
            ["presence_replaced"] = second is not null && second != first,
            ["second_endpoint"] = second?.ControllingClient.RemoteEndPoint?.Port,
            ["same_circuit_restored"] = ReferenceEquals(region.Circuit, region.Scene.AuthenticateHandler.GetAgentCircuitData(region.Agent))
        };
    }

    static JsonObject CloseHandlers()
    {
        Scene scene = new SceneHelpers().SetupScene("Pin3 close", Id(0x1070), 1000, 1000);
        TestLLUDPServer udp = ClientStackHelpers.AddUdpServer(scene);
        ScenePresence presence = ClientStackHelpers.AddChildClient(scene, udp, Id(0x70), Id(0x2070), 123456);
        LLClientView client = (LLClientView)presence.ControllingClient;
        client.IsActive = true;
        client.SendLogoutPacket();
        int reply = Enumerable.Range(0, udp.PacketsSent.Count).Single(i => udp.PacketsSent[i].Type == PacketType.LogoutReply);
        JsonArray callbacks = [];
        client.OnConnectionClosed += _ => { callbacks.Add("throwing"); throw new InvalidOperationException("injected close handler failure"); };
        client.OnConnectionClosed += _ => callbacks.Add("later-one");
        client.OnConnectionClosed += _ => callbacks.Add("later-two");
        bool threw = false;
        try { scene.CloseAgent(client.AgentId, false); } catch { threw = true; }
        return new JsonObject {
            ["callbacks"] = callbacks, ["close_threw"] = threw,
            ["scene_presence"] = scene.GetScenePresence(client.AgentId) is not null,
            ["circuit"] = scene.AuthenticateHandler.GetAgentCircuitData(client.AgentId) is not null,
            ["clients"] = scene.GetNumberOfClients(), ["logout_category"] = udp.CategoriesSent[reply].ToString()
        };
    }

    static JsonObject Startup(bool success)
    {
        Region region = new(success ? 0x80 : 0x81);
        region.Grid.StartupResult = success;
        ActivityDetector observer = new(region.Grid);
        observer.AddRegion(region.Scene);
        observer.RemoveRegion(region.Scene);
        JsonObject wire = StartupWire(region.Scene.RegionInfo.RegionID, success);
        wire["detector_region_calls"] = new JsonArray(region.Grid.StartedRegions.Select(id => (JsonNode)JsonValue.Create(id.ToString())).ToArray());
        return wire;
    }

    static JsonObject StartupWire(UUID region, bool success)
    {
        using TcpListener listener = new(IPAddress.Loopback, 0);
        listener.Start(1);
        string uri = "http://127.0.0.1:" + ((IPEndPoint)listener.LocalEndpoint).Port;
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(10));
        string response = "<?xml version=\"1.0\"?><ServerResponse><result>" + (success ? "Success" : "Failure") + "</result></ServerResponse>";
        Task<JsonObject> exchange = ReadRequest(listener, response, timeout.Token);
        bool accepted = new GridUserServicesConnector(uri).LoggedOutRegion(region);
        JsonObject captured = exchange.WaitAsync(timeout.Token).GetAwaiter().GetResult();
        captured["response"] = response; captured["status"] = 200; captured["accepted"] = accepted;
        return captured;
    }

    static async Task<JsonObject> ReadRequest(TcpListener listener, string response, CancellationToken token)
    {
        using TcpClient socket = await listener.AcceptTcpClientAsync(token);
        using NetworkStream stream = socket.GetStream();
        List<byte> header = []; byte[] one = new byte[1];
        while (header.Count < 16384)
        {
            if (await stream.ReadAsync(one, token) != 1) throw new IOException("request header EOF");
            header.Add(one[0]);
            if (header.Count >= 4 && header.TakeLast(4).SequenceEqual(new byte[] { 13, 10, 13, 10 })) break;
        }
        if (header.Count >= 16384) throw new IOException("request header limit");
        string[] lines = Encoding.ASCII.GetString(header.ToArray()).Split("\r\n");
        Dictionary<string, string> headers = new(StringComparer.OrdinalIgnoreCase);
        foreach (string line in lines.Skip(1).Where(line => line.Contains(':')))
        { int split = line.IndexOf(':'); headers[line[..split]] = line[(split + 1)..].Trim(); }
        int length = int.Parse(headers["Content-Length"]);
        if (length < 0 || length > 65536) throw new IOException("request body limit");
        byte[] body = new byte[length]; await stream.ReadExactlyAsync(body, token);
        byte[] reply = Encoding.UTF8.GetBytes(response);
        byte[] replyHeader = Encoding.ASCII.GetBytes($"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {reply.Length}\r\nConnection: close\r\n\r\n");
        await stream.WriteAsync(replyHeader, token); await stream.WriteAsync(reply, token);
        return new JsonObject { ["method"] = lines[0].Split(' ')[0], ["path"] = lines[0].Split(' ')[1],
            ["content_type"] = headers["Content-Type"], ["request"] = Encoding.UTF8.GetString(body) };
    }
}
