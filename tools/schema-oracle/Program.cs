// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Creates SQLite databases by running Tranquillity's Migration.Update.
// The SQL stays in the reference checkout. This program only calls it.

using System.Data.SQLite;
using System.Text;
using System.Text.RegularExpressions;
using OpenSim.Data;

const int MaxPayload = 16 * 1024 * 1024;

if (args.Length != 4 || args[2] != "--revision" || !IsRevision(args[3])
    || (args[0] != "generate" && args[0] != "verify"))
{
    Console.Error.WriteLine("usage: SchemaOracle generate|verify <directory> --revision <40 hex chars>");
    return 2;
}

string command = args[0];
string directory = Path.GetFullPath(args[1]);
string revision = args[3].ToLowerInvariant();
var assembly = typeof(OpenSim.Data.SQLite.SQLiteAssetData).Assembly;
var stores = Discover(assembly);
if (stores.Count == 0)
{
    Console.Error.WriteLine("schema: the SQLite assembly has no migration resources");
    return 1;
}

try
{
    if (command == "generate")
        return Generate(directory, revision, assembly, stores);
    return Verify(directory, revision, assembly, stores);
}
catch (Exception error)
{
    Console.Error.WriteLine("schema: " + error.Message);
    return 1;
}

static int Generate(string directory, string revision, System.Reflection.Assembly assembly, List<string> stores)
{
    string left = FreshTemp();
    string right = FreshTemp();
    try
    {
        string manifest = WriteStores(left, revision, assembly, stores);
        string again = WriteStores(right, revision, assembly, stores);
        if (manifest != again)
            throw new InvalidOperationException("two generates produced different manifests");
        CompareTrees(left, right);
        Publish(directory, left, stores);
    }
    finally
    {
        DeleteDirectory(left);
        DeleteDirectory(right);
    }
    Console.WriteLine("generate: " + stores.Count + " SQLite stores");
    return 0;
}

static int Verify(string directory, string revision, System.Reflection.Assembly assembly, List<string> stores)
{
    string manifestPath = Path.Combine(directory, "manifest.txt");
    string expected = File.ReadAllText(manifestPath).Replace("\r\n", "\n");
    string freshDir = FreshTemp();
    try
    {
        string fresh = WriteStores(freshDir, revision, assembly, stores);
        if (expected != fresh)
            throw new InvalidOperationException("manifest.txt does not match a fresh Tranquillity migration");
        CompareTrees(directory, freshDir);
    }
    finally
    {
        DeleteDirectory(freshDir);
    }
    Console.WriteLine("verify: " + stores.Count + " SQLite stores match Tranquillity");
    return 0;
}

static string WriteStores(string directory, string revision, System.Reflection.Assembly assembly, List<string> stores)
{
    Directory.CreateDirectory(directory);
    var manifest = new StringBuilder();
    manifest.Append("format 1\n");
    manifest.Append("revision ").Append(revision).Append('\n');
    foreach (string store in stores)
    {
        string fileName = store + ".db";
        string path = Path.Combine(directory, fileName);
        int version = Migrate(path, assembly, store);
        manifest.Append(store).Append(' ').Append(version).Append(' ').Append(fileName).Append('\n');
    }
    string text = manifest.ToString();
    File.WriteAllText(Path.Combine(directory, "manifest.txt"), text, new UTF8Encoding(encoderShouldEmitUTF8Identifier: false));
    return text;
}

static int Migrate(string path, System.Reflection.Assembly assembly, string store)
{
    if (File.Exists(path))
        File.Delete(path);
    var builder = new SQLiteConnectionStringBuilder
    {
        DataSource = path,
        Version = 3,
        Pooling = false
    };
    using var connection = new SQLiteConnection(builder.ConnectionString);
    connection.Open();
    var migration = new Migration(connection, assembly, store);
    migration.Update();
    using var command = connection.CreateCommand();
    command.CommandText = "select name, version from migrations order by name";
    using var reader = command.ExecuteReader();
    int found = -1;
    int meta = -1;
    int rows = 0;
    while (reader.Read())
    {
        rows++;
        if (rows > 2)
            throw new InvalidOperationException(store + " migrations table has too many rows");
        string name = reader.GetString(0);
        int version = Convert.ToInt32(reader.GetValue(1));
        if (name == store)
            found = version;
        else if (name == "migrations")
            meta = version;
        else
            throw new InvalidOperationException(store + " database has an unexpected migrations row " + name);
    }
    if (meta != 1)
        throw new InvalidOperationException(store + " migrations meta version is " + meta);
    if (found < 1)
        throw new InvalidOperationException(store + " has no version row");
    return found;
}

static List<string> Discover(System.Reflection.Assembly assembly)
{
    var stores = new SortedSet<string>(StringComparer.Ordinal);
    var oldStyle = new Regex(@"\.(\d{3})_(?<store>[A-Za-z0-9_]+)\.sql$", RegexOptions.CultureInvariant);
    var newStyle = new Regex(@"\.(?<store>[A-Za-z0-9_]+)\.migrations(?:\.\d+)?$", RegexOptions.CultureInvariant);
    foreach (string name in assembly.GetManifestResourceNames())
    {
        Match oldMatch = oldStyle.Match(name);
        if (oldMatch.Success)
            stores.Add(oldMatch.Groups["store"].Value);
        Match newMatch = newStyle.Match(name);
        if (newMatch.Success)
            stores.Add(newMatch.Groups["store"].Value);
    }
    return stores.ToList();
}

static void Publish(string directory, string source, List<string> stores)
{
    if (Directory.Exists(directory))
    {
        foreach (string path in Directory.GetFiles(directory))
        {
            string name = Path.GetFileName(path);
            if (name == "README.md" || name == ".gitattributes" || name == "manifest.txt" || name.EndsWith(".db", StringComparison.Ordinal))
                continue;
            throw new InvalidOperationException("refusing to write into a directory that contains " + name);
        }
    }
    else
    {
        Directory.CreateDirectory(directory);
    }
    var keep = new HashSet<string>(StringComparer.Ordinal) { "manifest.txt" };
    foreach (string store in stores)
        keep.Add(store + ".db");
    if (Directory.Exists(directory))
    {
        foreach (string path in Directory.GetFiles(directory))
        {
            string name = Path.GetFileName(path);
            if (name.EndsWith(".db", StringComparison.Ordinal) && !keep.Contains(name))
                File.Delete(path);
        }
    }
    foreach (string name in keep)
    {
        string from = Path.Combine(source, name);
        string to = Path.Combine(directory, name);
        File.Copy(from, to, overwrite: true);
    }
}

static void CompareTrees(string left, string right)
{
    foreach (string from in Directory.GetFiles(left))
    {
        string name = Path.GetFileName(from);
        if (name == "README.md" || name == ".gitattributes")
            continue;
        string other = Path.Combine(right, name);
        if (!File.Exists(other))
            throw new InvalidOperationException(name + " exists only in " + left);
        byte[] leftBytes = ReadBounded(from);
        byte[] rightBytes = ReadBounded(other);
        if (!leftBytes.AsSpan().SequenceEqual(rightBytes))
        {
            int at = 0;
            int length = Math.Min(leftBytes.Length, rightBytes.Length);
            while (at < length && leftBytes[at] == rightBytes[at])
                at++;
            throw new InvalidOperationException(
                name + " differs (" + leftBytes.Length + " vs " + rightBytes.Length + " bytes, first mismatch at " + at + ")");
        }
    }
}

static byte[] ReadBounded(string path)
{
    var info = new FileInfo(path);
    if (info.Length > MaxPayload)
        throw new InvalidOperationException(path + " is larger than 16 MiB");
    return File.ReadAllBytes(path);
}

static string FreshTemp()
{
    string path = Path.Combine(Path.GetTempPath(), "ferroan-schema-" + Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(path);
    return path;
}

static void DeleteDirectory(string path)
{
    if (Directory.Exists(path))
        Directory.Delete(path, recursive: true);
}

static bool IsRevision(string value)
{
    if (value.Length != 40)
        return false;
    foreach (char ch in value)
    {
        if (!Uri.IsHexDigit(ch))
            return false;
    }
    return true;
}
