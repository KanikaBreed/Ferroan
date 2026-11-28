// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// New Ferroan code. It calls the pinned YEngine compiler. It does not copy
// those sources.

using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Reflection.Emit;
using System.Text;
using System.Text.Json;
using OpenSim.Region.ScriptEngine.Yengine;

CultureInfo.CurrentCulture = new CultureInfo("en-US");
string runtime = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;
if (!runtime.Contains("10.0.14", StringComparison.Ordinal))
{
    Console.Error.WriteLine("runtime is " + runtime + ", want 10.0.14");
    return 1;
}

bool slots = false;
string[] argv = args;
if (argv.Length == 3 && argv[0] == "--slots")
{
    slots = true;
    argv = new[] { argv[1], argv[2] };
}
if (argv.Length != 2)
{
    Console.Error.WriteLine("usage: YEngineTokensOracle cases.json out-dir");
    Console.Error.WriteLine("       YEngineTokensOracle --slots cases.json out-dir");
    return 2;
}

if (RequirePin(out string pin) != 0)
    return 1;

if (!EnsureInstanceType())
{
    Console.Error.WriteLine("instance type was not created");
    return 1;
}

JsonDocument doc = JsonDocument.Parse(File.ReadAllText(argv[0]));
string outDir = argv[1];
Directory.CreateDirectory(outDir);
string objDir = Path.Combine(outDir, "objects");
Directory.CreateDirectory(objDir);

var cases = new List<Dictionary<string, object>>();
var errors = new List<Dictionary<string, object>>();
var canonical = new StringBuilder();
bool failed = false;

foreach (JsonElement item in doc.RootElement.GetProperty("compile").EnumerateArray())
{
    string name = item.GetProperty("name").GetString();
    string source = item.GetProperty("source").GetString();
    var messages = new List<string>();
    TokenErrorMessage emsg = (token, message) => messages.Add(message);
    TokenBegin begin = TokenBegin.Construct(name, null, emsg, source, out string hash);
    var tokenizeMessages = new List<string>(messages);
    if (begin == null)
    {
        Console.Error.WriteLine("tokenize failed " + name + ": " + string.Join(" | ", messages));
        failed = true;
        continue;
    }
    var tokens = new List<string>();
    for (Token t = begin.nextToken; t != null && t is not TokenEnd; t = t.nextToken)
        tokens.Add(t.ToString());

    messages.Clear();
    TokenScript script = ScriptReduce.Reduce(begin);
    if (script == null)
    {
        Console.Error.WriteLine("reduce failed " + name + ": " + string.Join(" | ", messages));
        failed = true;
        continue;
    }
    byte[] bytes;
    using (var mem = new MemoryStream())
    {
        var utf8 = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);
        using (var writer = new BinaryWriter(mem, utf8, leaveOpen: true))
        {
            bool ok = ScriptCodeGen.CodeGen(script, writer, hash);
            if (!ok)
            {
                Console.Error.WriteLine("codegen failed " + name + ": " + string.Join(" | ", messages));
                failed = true;
                continue;
            }
        }
        bytes = mem.ToArray();
    }
    Header header;
    try
    {
        header = ReadHeader(bytes);
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine("header failed " + name + ": " + ex.Message);
        failed = true;
        continue;
    }
    if (!string.Equals(header.Hash, hash, StringComparison.Ordinal))
    {
        Console.Error.WriteLine("hash mismatch " + name);
        failed = true;
        continue;
    }
    if (header.Version != ScriptCodeGen.COMPILED_VERSION_VALUE)
    {
        Console.Error.WriteLine("version " + header.Version + " on " + name);
        failed = true;
        continue;
    }
    File.WriteAllBytes(Path.Combine(objDir, name + ".yobj"), bytes);
    if (slots)
        AppendSlots(canonical, name, header);
    else
        AppendCase(canonical, name, header);
    var globals = new List<Dictionary<string, object>>();
    foreach (Global g in header.Globals)
    {
        globals.Add(new Dictionary<string, object>
        {
            ["name"] = g.Name,
            ["type"] = g.Type,
            ["index"] = g.Index,
        });
    }
    cases.Add(new Dictionary<string, object>
    {
        ["name"] = name,
        ["source"] = source,
        ["tokens"] = tokens,
        ["hash"] = hash,
        ["version"] = header.Version,
        ["sizes"] = header.Sizes,
        ["states"] = header.States,
        ["globals"] = globals,
        ["tokenize_messages"] = tokenizeMessages,
        ["bytes"] = bytes.Length,
    });
    Console.Error.WriteLine("ok " + name + " tokens " + tokens.Count + " bytes " + bytes.Length + " globals " + globals.Count);
}

if (slots && doc.RootElement.GetProperty("errors").GetArrayLength() != 0)
{
    Console.Error.WriteLine("slots mode does not record error cases");
    return 1;
}

if (!slots)
{
foreach (JsonElement item in doc.RootElement.GetProperty("errors").EnumerateArray())
{
    string name = item.GetProperty("name").GetString();
    string source = item.GetProperty("source").GetString();
    var messages = new List<string>();
    TokenErrorMessage emsg = (token, message) => messages.Add(message);
    TokenBegin begin = TokenBegin.Construct(name, null, emsg, source, out string hash);
    if (begin != null)
    {
        Console.Error.WriteLine("expected tokenize failure " + name);
        failed = true;
        continue;
    }
    errors.Add(new Dictionary<string, object>
    {
        ["name"] = name,
        ["source"] = source,
        ["messages"] = messages,
    });
}
}

if (failed)
    return 1;

string canon = canonical.ToString();
string digest = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(Encoding.UTF8.GetBytes(canon))).ToLowerInvariant();
var opts = new JsonSerializerOptions { WriteIndented = true };
if (slots)
{
    File.WriteAllText(Path.Combine(outDir, "digest.json"), JsonSerializer.Serialize(new Dictionary<string, string>
    {
        ["runtime"] = "10.0.14",
        ["pin"] = pin,
        ["digest"] = digest,
    }, opts) + "\n");
    Console.Error.WriteLine("slots " + digest);
    return 0;
}
var manifest = new Dictionary<string, object>
{
    ["runtime"] = "10.0.14",
    ["pin"] = pin,
    ["digest"] = digest,
    ["cases"] = cases,
    ["errors"] = errors,
};
File.WriteAllText(Path.Combine(outDir, "manifest.json"), JsonSerializer.Serialize(manifest, opts) + "\n");
File.WriteAllText(Path.Combine(outDir, "digest.json"), JsonSerializer.Serialize(new Dictionary<string, string>
{
    ["runtime"] = "10.0.14",
    ["pin"] = pin,
    ["digest"] = digest,
}, opts) + "\n");
File.WriteAllText(Path.Combine(outDir, "canonical.txt"), canon);
Console.Error.WriteLine("digest " + digest);
return 0;

static bool EnsureInstanceType()
{
    if (ScriptCodeGen.xmrInstSuperType != null)
        return true;
    // XMREngine.OneTimeLateInitialization builds this type before any compile.
    // That method needs a scene and the API table. These scripts do not call
    // API methods, so the subclass has no API fields. CodeGen still requires
    // the type and the short name "xmrsuper".
    AssemblyName assemblyName = new()
    {
        Name = "XMRInstanceSuperAssembly"
    };
    AssemblyBuilder assemblyBuilder = AssemblyBuilder.DefineDynamicAssembly(assemblyName, AssemblyBuilderAccess.RunAndCollect);
    ModuleBuilder moduleBuilder = assemblyBuilder.DefineDynamicModule("XMRInstanceSuperModule");
    TypeBuilder typeBuilder = moduleBuilder.DefineType("XMRInstanceSuperType", TypeAttributes.Public | TypeAttributes.Class);
    typeBuilder.SetParent(typeof(XMRInstance));
    Type created = typeBuilder.CreateType();
    if (created == null)
        return false;
    ScriptCodeGen.xmrInstSuperType = created;
    ScriptObjWriter.DefineInternalType("xmrsuper", created);
    return true;
}

static void AppendSlots(StringBuilder sb, string name, Header header)
{
    sb.Append("C ").Append(name).Append('\n');
    foreach (Global global in header.Globals)
    {
        sb.Append("G ").Append(global.Name).Append('\n');
        sb.Append(global.Type).Append('\n');
        sb.Append(global.Index.ToString(CultureInfo.InvariantCulture)).Append('\n');
    }
    sb.Append(".\n");
}

static void AppendCase(StringBuilder sb, string name, Header header)
{
    sb.Append("C ").Append(name).Append('\n');
    sb.Append("H ").Append(header.Hash).Append('\n');
    sb.Append("V ").Append(header.Version).Append('\n');
    sb.Append("Z");
    foreach (int size in header.Sizes)
        sb.Append(' ').Append(size);
    sb.Append('\n');
    foreach (string state in header.States)
        sb.Append("S ").Append(state).Append('\n');
    foreach (Global global in header.Globals)
    {
        sb.Append("G ").Append(global.Name).Append('\n');
        sb.Append(global.Type).Append('\n');
        sb.Append(global.Index).Append('\n');
    }
    sb.Append(".\n");
}

static Header ReadHeader(byte[] bytes)
{
    using var mem = new MemoryStream(bytes, writable: false);
    var utf8 = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);
    using var reader = new BinaryReader(mem, utf8, leaveOpen: true);
    char[] magicChars = reader.ReadChars(ScriptCodeGen.OBJECT_CODE_MAGIC.Length);
    if (new string(magicChars) != ScriptCodeGen.OBJECT_CODE_MAGIC)
        throw new InvalidDataException("bad magic");
    int version = reader.ReadInt32();
    string hash = reader.ReadString();
    int[] sizes = new int[11];
    for (int i = 0; i < sizes.Length; i++)
        sizes[i] = reader.ReadInt32();
    int nStates = reader.ReadInt32();
    if (nStates < 0)
        throw new InvalidDataException("state count");
    var states = new string[nStates];
    for (int i = 0; i < nStates; i++)
        states[i] = reader.ReadString();
    var globals = new List<Global>();
    while (true)
    {
        string name = reader.ReadString();
        if (name.Length == 0)
            break;
        string type = reader.ReadString();
        int index = reader.ReadInt32();
        globals.Add(new Global(name, type, index));
    }
    return new Header(version, hash, sizes, states, globals);
}

static int RequirePin(out string sha)
{
    const string WantPin = "41f18f983bb2bfc82e033fed410d361ba57d5aa4";
    sha = "";
    string root = Environment.GetEnvironmentVariable("FERROAN_REF");
    if (string.IsNullOrEmpty(root))
    {
        string home = Environment.GetEnvironmentVariable("HOME");
        if (string.IsNullOrEmpty(home))
            home = Environment.GetEnvironmentVariable("USERPROFILE");
        root = home + "/Git/reference";
    }
    string repo = Path.Combine(root, "OpenSim-Tranquillity");
    if (!Git(repo, "rev-parse HEAD", out sha))
    {
        Console.Error.WriteLine("pin is , want " + WantPin);
        return 1;
    }
    sha = sha.Trim();
    if (!Git(repo, "status --porcelain", out string dirty))
    {
        Console.Error.WriteLine("pin status failed");
        return 1;
    }
    if (!string.IsNullOrWhiteSpace(dirty))
    {
        Console.Error.WriteLine("pin checkout is dirty:");
        Console.Error.Write(dirty.EndsWith("\n") ? dirty : dirty + "\n");
        return 1;
    }
    if (!string.Equals(sha, WantPin, StringComparison.Ordinal))
    {
        Console.Error.WriteLine("pin is " + sha + ", want " + WantPin);
        return 1;
    }
    return 0;
}

static bool Git(string repo, string arguments, out string stdout)
{
    stdout = "";
    var start = new ProcessStartInfo("git", arguments)
    {
        WorkingDirectory = repo,
        RedirectStandardOutput = true,
        RedirectStandardError = true,
        UseShellExecute = false,
    };
    using Process proc = Process.Start(start);
    if (proc == null)
        return false;
    stdout = proc.StandardOutput.ReadToEnd();
    proc.StandardError.ReadToEnd();
    proc.WaitForExit();
    return proc.ExitCode == 0;
}

sealed record Header(int Version, string Hash, int[] Sizes, string[] States, List<Global> Globals);
sealed record Global(string Name, string Type, int Index);
