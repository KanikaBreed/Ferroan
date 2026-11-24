using System.Collections;
using System.Collections.Specialized;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework.Servers.HttpServer;
using OpenSim.Framework.ServiceAuth;
using OpenSim.Server.Handlers.Authentication;
using OpenSim.Server.Handlers.GridUser;
using OpenSim.Services.Interfaces;

// In-process capture of BaseStreamHandler.Handle. Content-Type starts as the
// stream-handler default, which BaseHttpServer sets before Handle. Status
// starts at 200, which HttpListenerResponse uses. No Robust process is started.

if (args.Length != 2)
{
    Console.Error.WriteLine("usage: ShardOracle <pin> <output.json>");
    return 2;
}

var cases = new List<CapturedCase>();
foreach (var plan in Plans())
    cases.Add(Run(plan));

var captured = new Captured
{
    Pin = args[0],
    Capture = "Handle on AuthenticationServerPostHandler and GridUserServerPostHandler after ServiceAuth.Create. Content-Type is application/xml before Handle. Status starts at 200.",
    Cases = cases,
};
var options = new JsonSerializerOptions
{
    WriteIndented = true,
    PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
    DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
};
File.WriteAllText(args[1], JsonSerializer.Serialize(captured, options) + "\n");
return 0;

static IEnumerable<Plan> Plans()
{
    foreach (var service in new[] { "auth", "griduser" })
    {
        var plain = service == "auth" ? "/auth/plain" : "/griduser";
        yield return new Plan(service, plain, "absent", "absent", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "default-shard");
        yield return new Plan(service, plain, "absent", "true", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "section-true");
        yield return new Plan(service, plain, "false", "true", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "network-false-section-true");
        yield return new Plan(service, plain, "true", "false", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "network-true-section-false");
        yield return new Plan(service, plain, "true", "absent", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "network-true");
        yield return new Plan(service, plain, "false", "absent", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "network-false");
        yield return new Plan(service, plain, "absent", "absent", false, "X-SecondLife-Shard", "", "METHOD=nope", null, "no-header");
        yield return new Plan(service, plain, "absent", "absent", true, "X-SecondLife-Shard", "", "METHOD=nope", null, "empty-header");
        yield return new Plan(service, plain, "absent", "absent", true, "x-secondlife-shard", "OpenSim", "METHOD=nope", null, "lower-header");
        yield return new Plan(service, plain, "absent", "absent", true, "X-SecondLife-Shard", "OpenSim", "", 1048577, "huge-shard");
    }

    yield return new Plan("auth", "/auth/crypt", "absent", "absent", true, "X-SecondLife-Shard", "OpenSim", "abc", null, "crypt-shard");
    yield return new Plan("auth", "/auth/crypt", "absent", "absent", false, "X-SecondLife-Shard", "", "abc", null, "crypt-no-header");
    yield return new Plan("auth", "/auth", "absent", "absent", true, "X-SecondLife-Shard", "OpenSim", "abc", null, "bare-shard");
    yield return new Plan("auth", "/auth", "absent", "absent", false, "X-SecondLife-Shard", "", "abc", null, "bare-no-header");
    yield return new Plan("auth", "/auth/", "absent", "absent", true, "X-SecondLife-Shard", "OpenSim", "METHOD=nope", null, "slash-shard");
}

static CapturedCase Run(Plan plan)
{
    var ini = Ini(plan);
    var path = Path.Combine(Path.GetTempPath(), "ferroan-shard-" + Guid.NewGuid().ToString("n") + ".ini");
    File.WriteAllText(path, ini);
    try
    {
        IConfigSource config = new IniConfigSource(path);
        var section = plan.Service == "auth" ? "AuthenticationService" : "GridUserService";
        IServiceAuth auth = ServiceAuth.Create(config, section);
        var body = plan.BodyLen is int length ? new string('x', length) : plan.Body;
        var stream = new CountingStream(Encoding.UTF8.GetBytes(body));
        var request = new FakeRequest(plan.HeaderPresent ? plan.HeaderName : null, plan.HeaderValue, stream);
        var response = new FakeResponse();
        byte[] bytes;
        int handled;
        if (plan.Service == "auth")
        {
            var handler = new AuthenticationServerPostHandler(new ThrowingAuth(), config.Configs[section], auth);
            bytes = handler.Handle(plan.Path, stream, request, response);
            handled = handler.RequestsHandled;
        }
        else
        {
            var handler = new GridUserServerPostHandler(new ThrowingGridUser(), auth);
            bytes = handler.Handle(plan.Path, stream, request, response);
            handled = handler.RequestsHandled;
        }

        return new CapturedCase
        {
            Id = plan.Service + "-" + plan.Name,
            Service = plan.Service,
            Path = plan.Path,
            Network = plan.Network,
            Section = plan.Section,
            HeaderPresent = plan.HeaderPresent,
            HeaderName = plan.HeaderName,
            HeaderValue = plan.HeaderValue,
            Body = plan.BodyLen is null ? plan.Body : "",
            BodyLen = plan.BodyLen,
            Status = response.StatusCode,
            ContentType = response.ContentType ?? "",
            ResponseBody = Encoding.UTF8.GetString(bytes ?? Array.Empty<byte>()),
            RequestsHandled = handled,
            BytesRead = stream.BytesRead,
        };
    }
    finally
    {
        File.Delete(path);
    }
}

static string Ini(Plan plan)
{
    var builder = new StringBuilder();
    if (plan.Network != "absent")
    {
        builder.Append("[Network]\n");
        builder.Append(Allow(plan.Network));
    }

    var section = plan.Service == "auth" ? "AuthenticationService" : "GridUserService";
    builder.Append('[').Append(section).Append("]\n");
    if (plan.Section != "absent")
        builder.Append(Allow(plan.Section));
    return builder.ToString();
}

static string Allow(string value)
{
    return value switch
    {
        "true" => "AllowllHTTPRequestIn = true\n",
        "false" => "AllowllHTTPRequestIn = false\n",
        "empty" => "AllowllHTTPRequestIn =\n",
        _ => throw new InvalidOperationException(value),
    };
}

sealed record Plan(
    string Service,
    string Path,
    string Network,
    string Section,
    bool HeaderPresent,
    string HeaderName,
    string HeaderValue,
    string Body,
    int? BodyLen,
    string Name);

sealed class Captured
{
    public string Pin { get; set; }
    public string Capture { get; set; }
    public List<CapturedCase> Cases { get; set; }
}

sealed class CapturedCase
{
    public string Id { get; set; }
    public string Service { get; set; }
    public string Path { get; set; }
    public string Network { get; set; }
    public string Section { get; set; }
    public bool HeaderPresent { get; set; }
    public string HeaderName { get; set; }
    public string HeaderValue { get; set; }
    public string Body { get; set; }
    public int? BodyLen { get; set; }
    public int Status { get; set; }
    public string ContentType { get; set; }
    public string ResponseBody { get; set; }
    public int RequestsHandled { get; set; }
    public int BytesRead { get; set; }
}

sealed class CountingStream : MemoryStream
{
    public int BytesRead;

    public CountingStream(byte[] data) : base(data) {}

    public override int Read(byte[] buffer, int offset, int count)
    {
        var read = base.Read(buffer, offset, count);
        BytesRead += read;
        return read;
    }
}

sealed class FakeRequest : IOSHttpRequest
{
    readonly NameValueCollection headers = new();
    readonly CountingStream stream;

    public FakeRequest(string name, string value, CountingStream stream)
    {
        this.stream = stream;
        if (name != null)
            headers.Add(name, value);
    }

    public string[] AcceptTypes => throw Unused();
    public Encoding ContentEncoding => Encoding.UTF8;
    public long ContentLength => stream.Length;
    public long ContentLength64 => stream.Length;
    public string ContentType => "";
    public bool HasEntityBody => stream.Length > 0;
    public NameValueCollection Headers => headers;
    public string HttpMethod => "POST";
    public Stream InputStream => stream;
    public bool IsSecured => false;
    public bool KeepAlive => false;
    public NameValueCollection QueryString => throw Unused();
    public Hashtable Query => throw Unused();
    public HashSet<string> QueryFlags => throw Unused();
    public Dictionary<string, string> QueryAsDictionary => throw Unused();
    public string RawUrl => throw Unused();
    public IPEndPoint RemoteIPEndPoint => throw Unused();
    public IPEndPoint LocalIPEndPoint => throw Unused();
    public Uri Url => throw Unused();
    public string UriPath => throw Unused();
    public string UserAgent => "";
    public double ArrivalTS => 0;

    static Exception Unused() => new NotSupportedException("shard oracle request field");
}

sealed class FakeResponse : IOSHttpResponse
{
    public string ContentType { get; set; } = "application/xml";
    public long ContentLength { get => throw Unused(); set => throw Unused(); }
    public long ContentLength64 { get => throw Unused(); set => throw Unused(); }
    public Encoding ContentEncoding { get => throw Unused(); set => throw Unused(); }
    public bool KeepAlive { get => throw Unused(); set => throw Unused(); }
    public int KeepAliveTimeout { get => throw Unused(); set => throw Unused(); }
    public Stream OutputStream => throw Unused();
    public string ProtocolVersion { get => throw Unused(); set => throw Unused(); }
    public int Priority { get => throw Unused(); set => throw Unused(); }
    public byte[] RawBuffer { get => throw Unused(); set => throw Unused(); }
    public int RawBufferStart { get => throw Unused(); set => throw Unused(); }
    public int RawBufferLen { get => throw Unused(); set => throw Unused(); }
    public Stream Body => throw Unused();
    public bool SendChunked { get => throw Unused(); set => throw Unused(); }
    public int StatusCode { get; set; } = 200;
    public string StatusDescription { get => throw Unused(); set => throw Unused(); }
    public double RequestTS => 0;
    public void Redirect(string url, HttpStatusCode redirStatusCode = HttpStatusCode.Redirect) => throw Unused();
    public void AddHeader(string key, string value) {}

    static Exception Unused() => new NotSupportedException("shard oracle response field");
}

sealed class ThrowingAuth : IAuthenticationService
{
    public string Authenticate(UUID principalID, string password, int lifetime) => throw Unused();
    public string Authenticate(UUID principalID, string password, int lifetime, out UUID realID) => throw Unused();
    public bool Verify(UUID principalID, string token, int lifetime) => throw Unused();
    public bool Release(UUID principalID, string token) => throw Unused();
    public bool SetPassword(UUID principalID, string passwd) => throw Unused();
    public AuthInfo GetAuthInfo(UUID principalID) => throw Unused();
    public bool SetAuthInfo(AuthInfo info) => throw Unused();

    static Exception Unused() => new NotSupportedException("shard oracle auth service");
}

sealed class ThrowingGridUser : IGridUserService
{
    public GridUserInfo LoggedIn(string userID) => throw Unused();
    public bool LoggedOut(string userID, UUID sessionID, UUID regionID, Vector3 lastPosition, Vector3 lastLookAt) => throw Unused();
    public bool LoggedOutRegion(UUID regionID) => throw Unused();
    public bool SetHome(string userID, UUID homeID, Vector3 homePosition, Vector3 homeLookAt) => throw Unused();
    public bool SetLastPosition(string userID, UUID sessionID, UUID regionID, Vector3 lastPosition, Vector3 lastLookAt) => throw Unused();
    public GridUserInfo GetGridUserInfo(string userID) => throw Unused();
    public GridUserInfo[] GetGridUserInfo(string[] userID) => throw Unused();

    static Exception Unused() => new NotSupportedException("shard oracle grid user service");
}
