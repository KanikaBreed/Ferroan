// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Writes the resolved key/value set Nini produces for synthetic INI trees and
// for Tranquillity's shipped examples. The Rust crate reads the same trees.

using Nini.Config;

namespace Ferroan.ConfigOracle;

static class Program
{
    const string FormatLine = "format 1";
    const string NiniLine = "nini 0.9.2.16";

    static int Main(string[] args)
    {
        if (args.Length != 6 || (args[0] != "generate" && args[0] != "verify")
            || args[2] != "--revision" || args[4] != "--reference")
        {
            Console.Error.WriteLine("usage: ConfigOracle generate|verify <directory> --revision <40 hex> --reference <path>");
            return 2;
        }
        string directory = Path.GetFullPath(args[1]);
        string revision = args[3];
        string reference = Path.GetFullPath(args[5]);
        if (revision.Length != 40 || revision.Any(ch => !Uri.IsHexDigit(ch)))
        {
            Console.Error.WriteLine("revision must be 40 hex characters");
            return 1;
        }
        try
        {
            if (args[0] == "generate")
                Generate(directory, revision, reference);
            else
                Verify(directory, revision, reference);
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception.Message);
            return 1;
        }
        return 0;
    }

    static void Generate(string directory, string revision, string reference)
    {
        string scratch = directory + ".partial";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision, reference);
        Publish(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("generate: " + CaseList().Count + " configuration cases");
    }

    static void Verify(string directory, string revision, string reference)
    {
        string scratch = directory + ".verify";
        if (Directory.Exists(scratch))
            Directory.Delete(scratch, true);
        Directory.CreateDirectory(scratch);
        WriteTree(scratch, revision, reference);
        CompareTrees(scratch, directory);
        Directory.Delete(scratch, true);
        Console.WriteLine("verify: " + CaseList().Count + " configuration cases match Nini");
    }

    static void WriteTree(string directory, string revision, string reference)
    {
        var manifest = new List<string> { FormatLine, NiniLine, "revision " + revision, "" };
        foreach (Case item in CaseList())
        {
            manifest.Add(item.Id);
            string caseDir = Path.Combine(directory, item.Id);
            Directory.CreateDirectory(caseDir);
            File.WriteAllText(Path.Combine(caseDir, "kind.txt"), item.Kind + "\n");
            if (item.Source != null)
                File.WriteAllText(Path.Combine(caseDir, "source.ini"), item.Source);
            if (item.Files != null)
            {
                foreach (KeyValuePair<string, string> file in item.Files)
                {
                    string path = Path.Combine(caseDir, file.Key.Replace('/', Path.DirectorySeparatorChar));
                    Directory.CreateDirectory(Path.GetDirectoryName(path));
                    File.WriteAllText(path, file.Value);
                }
            }
            Outcome first = Execute(item, caseDir, reference);
            Outcome second = Execute(item, caseDir, reference);
            if (!Same(first, second))
                throw new InvalidOperationException(item.Id + " differed between two loads");
            RejectMachinePaths(item.Id, first, reference);
            AssertShipped(item.Id, first);
            WriteOutcome(caseDir, first);
        }
        File.WriteAllText(Path.Combine(directory, "manifest.txt"), string.Join("\n", manifest) + "\n");
    }

    static void WriteOutcome(string caseDir, Outcome outcome)
    {
        if (outcome.Error != null)
        {
            File.WriteAllText(Path.Combine(caseDir, "error.txt"), outcome.Error + "\n");
            return;
        }
        File.WriteAllText(Path.Combine(caseDir, "resolved.txt"), string.Join("\n", outcome.Rows) + (outcome.Rows.Count == 0 ? "" : "\n"));
        var warnings = new List<string>();
        foreach (string warning in outcome.Warnings)
            warnings.Add("missing-include\t" + Loader.Escape(warning));
        File.WriteAllText(Path.Combine(caseDir, "warnings.txt"), warnings.Count == 0 ? "" : string.Join("\n", warnings) + "\n");
    }

    static Outcome Execute(Case item, string caseDir, string reference)
    {
        if (item.Kind == "parse" || item.Kind == "expand")
            return Loader.ParseText(File.ReadAllText(Path.Combine(caseDir, "source.ini")), item.Kind == "expand");

        var lines = File.ReadAllLines(Path.Combine(caseDir, "load.txt"));
        string appdata = null;
        string master = null;
        var files = new List<string>();
        string iniDirectory = null;
        bool sawDirectory = false;
        var environment = new Dictionary<string, string>(StringComparer.Ordinal);
        var argvKeys = new List<string[]>();
        foreach (string raw in lines)
        {
            string line = raw.TrimEnd('\r');
            if (line.Length == 0 || line.StartsWith('#'))
                continue;
            int space = line.IndexOf(' ');
            string field = space < 0 ? line : line.Substring(0, space);
            string rest = space < 0 ? "" : line.Substring(space + 1);
            switch (field)
            {
                case "reference_appdata": appdata = rest; break;
                case "ini_master": master = rest; break;
                case "ini_file": files.Add(rest); break;
                case "ini_directory": sawDirectory = true; iniDirectory = rest; break;
                case "env":
                    int equals = rest.IndexOf('=');
                    if (equals < 1)
                        throw new InvalidOperationException("env line needs KEY=VALUE");
                    environment[rest.Substring(0, equals)] = rest.Substring(equals + 1);
                    break;
                case "argv":
                    string[] parts = rest.Split(new char[] { ' ' }, 3, StringSplitOptions.None);
                    if (parts.Length != 3)
                        throw new InvalidOperationException("argv line needs Section Key Value");
                    argvKeys.Add(parts);
                    break;
                default:
                    throw new InvalidOperationException("unknown load field " + field);
            }
        }

        string configDir = appdata == null
            ? Path.Combine(caseDir, "tree")
            : Path.Combine(reference, "OpenSim-Tranquillity", "Source", appdata.Replace('/', Path.DirectorySeparatorChar));
        if (item.Kind == "grid" || item.Kind == "grid-ref")
            return Loader.LoadGrid(configDir, master, files, sawDirectory ? iniDirectory : null, environment);

        IConfigSource argv = new IniConfigSource();
        IConfig startup = argv.AddConfig("Startup");
        if (master != null)
            startup.Set("inimaster", master);
        for (int index = 0; index < files.Count; index++)
            startup.Set(index == 0 ? "inifile" : "inifile" + (index + 1), files[index]);
        if (sawDirectory)
            startup.Set("inidirectory", iniDirectory);
        foreach (string[] parts in argvKeys)
        {
            IConfig section = argv.Configs[parts[0]] ?? argv.AddConfig(parts[0]);
            section.Set(parts[1], parts[2]);
        }
        return Loader.LoadRegion(configDir, argv, environment);
    }

    static void AssertShipped(string id, Outcome outcome)
    {
        if (id != "region-shipped" && id != "grid-shipped")
            return;
        if (outcome.Error != null)
            throw new InvalidOperationException(id + " failed: " + outcome.Error);
        if (id == "region-shipped" && !outcome.Rows.Any(row => row.Contains("\tAssetServices\t") && row.Contains("RegionAssetConnector")))
            throw new InvalidOperationException("region include Standalone.ini did not load");
        if (id == "grid-shipped" && !outcome.Rows.Any(row => row.Contains("\tBaseHostname\t") && row.Contains("127.0.0.1")))
            throw new InvalidOperationException("grid example did not load BaseHostname");
    }

    static void RejectMachinePaths(string id, Outcome outcome, string reference)
    {
        foreach (string row in outcome.Rows)
        {
            if (row.Contains(reference, StringComparison.Ordinal) || row.Contains("/home/", StringComparison.Ordinal)
                || row.Contains("\\Users\\", StringComparison.Ordinal))
                throw new InvalidOperationException(id + " contains a machine path");
        }
    }

    static bool Same(Outcome left, Outcome right)
    {
        if (left.Error != right.Error || left.Rows.Count != right.Rows.Count || left.Warnings.Count != right.Warnings.Count)
            return false;
        for (int index = 0; index < left.Rows.Count; index++)
        {
            if (left.Rows[index] != right.Rows[index])
                return false;
        }
        for (int index = 0; index < left.Warnings.Count; index++)
        {
            if (left.Warnings[index] != right.Warnings[index])
                return false;
        }
        return true;
    }

    static void Publish(string scratch, string directory)
    {
        Directory.CreateDirectory(directory);
        var allowed = new HashSet<string>(StringComparer.Ordinal) { "README.md", ".gitattributes", "manifest.txt" };
        foreach (string id in CaseList().Select(item => item.Id))
            allowed.Add(id);
        foreach (string entry in Directory.GetFileSystemEntries(directory))
        {
            string name = Path.GetFileName(entry);
            if (!allowed.Contains(name))
                throw new InvalidOperationException("unexpected file in " + directory + ": " + name);
        }
        CopyFile(Path.Combine(scratch, "manifest.txt"), Path.Combine(directory, "manifest.txt"));
        foreach (Case item in CaseList())
        {
            string from = Path.Combine(scratch, item.Id);
            string to = Path.Combine(directory, item.Id);
            if (Directory.Exists(to))
                Directory.Delete(to, true);
            CopyDirectory(from, to);
        }
    }

    static void CopyDirectory(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (string file in Directory.GetFiles(from))
            CopyFile(file, Path.Combine(to, Path.GetFileName(file)));
        foreach (string sub in Directory.GetDirectories(from))
            CopyDirectory(sub, Path.Combine(to, Path.GetFileName(sub)));
    }

    static void CopyFile(string from, string to)
    {
        File.WriteAllBytes(to, File.ReadAllBytes(from));
    }

    static void CompareTrees(string left, string right)
    {
        CompareFile(Path.Combine(left, "manifest.txt"), Path.Combine(right, "manifest.txt"));
        foreach (Case item in CaseList())
            CompareDirectory(Path.Combine(left, item.Id), Path.Combine(right, item.Id));
    }

    static void CompareDirectory(string left, string right)
    {
        var leftNames = Directory.GetFileSystemEntries(left).Select(Path.GetFileName).OrderBy(name => name, StringComparer.Ordinal).ToList();
        var rightNames = Directory.GetFileSystemEntries(right).Select(Path.GetFileName).OrderBy(name => name, StringComparer.Ordinal).ToList();
        if (!leftNames.SequenceEqual(rightNames))
            throw new InvalidOperationException("directory entries differ: " + right);
        foreach (string name in leftNames)
        {
            string leftPath = Path.Combine(left, name);
            string rightPath = Path.Combine(right, name);
            if (Directory.Exists(leftPath))
                CompareDirectory(leftPath, rightPath);
            else
                CompareFile(leftPath, rightPath);
        }
    }

    static void CompareFile(string left, string right)
    {
        byte[] leftBytes = File.ReadAllBytes(left);
        byte[] rightBytes = File.ReadAllBytes(right);
        if (!leftBytes.AsSpan().SequenceEqual(rightBytes))
            throw new InvalidOperationException("bytes differ: " + right);
    }

    static List<Case> CaseList()
    {
        return new List<Case>
        {
            new Case("parse-basic", "parse",
                "[S]\nA = 1\nA = 2\nb = 3\nB = 4\n\n  K  =  v  \nM=\nN = \n" +
                "Kq = \"hello ; world\"\nKm = bare ; comment\nKh = hello \"world\"\nKe = \"\"\nKs = \"a\\\"b\"\n"),
            new Case("parse-section-replace", "parse", "[A]\nX=1\n[B]\nY=1\n[A]\nZ=2\nA=4\n"),
            new Case("parse-before-section", "parse", "A=1\n[S]\nB=2\n"),
            new Case("parse-crlf", "parse", "[S]\r\nA = 1\r\nB = two\r\n"),
            new Case("parse-hash", "parse", "[S]\n# c\nK=1\n"),
            new Case("parse-hash-in-value", "parse", "[S]\nK=1 # still\n"),
            new Case("parse-space-section", "parse", "[ Startup ]\nA=1\n[Startup]\nB=2\n"),
            new Case("expand-subst", "expand",
                "[Const]\nBase = http://x\n[S]\nU = ${Const|Base}/app\nV = ${U}/z\nW = ${Const|Base|extra}\n"),
            new Case("expand-order", "expand", "[S]\nA = ${B}\nB = x${C}\nC = y\n"),
            new Case("expand-missing-key", "expand", "[S]\nU = ${Missing}\n"),
            new Case("expand-missing-section", "expand", "[S]\nU = ${NoSuch|K}\n"),
            new Case("expand-self", "expand", "[S]\nU = ${U}\n"),
            new Case("expand-unclosed", "expand", "[S]\nA = pre ${nope\n"),
            new Case("expand-multi", "expand", "[S]\nA=x\nB=${A}-${A}\n"),
            new Case("expand-empty", "expand", "[S]\nA=${}\n"),
            new Case("region-order", "region", null, RegionOrderFiles(), RegionOrderLoad()),
            new Case("region-directory", "region", null, DirectoryFiles(), "ini_master master.ini\nini_directory overrides\n"),
            new Case("region-missing-master", "region", null, new Dictionary<string, string>(), "ini_master missing.ini\nini_file missing-user.ini\n"),
            new Case("region-none", "region", null, new Dictionary<string, string>(), "ini_master none\nini_file missing-user.ini\n"),
            new Case("region-remote", "region", null, new Dictionary<string, string>(), "ini_master http://example.test/remote.ini\n"),
            new Case("grid-include", "grid", null, GridIncludeFiles(), "ini_master master.ini\nini_directory overrides\n"),
            new Case("grid-missing-master", "grid", null, new Dictionary<string, string>(), "ini_master missing.ini\n"),
            new Case("region-shipped", "region-ref", null, null,
                "reference_appdata OpenSim.Server.RegionServer/AppData\nini_master RegionServerDefaults.ini\nini_file RegionServer.ini.example\n"),
            new Case("grid-shipped", "grid-ref", null, null,
                "reference_appdata OpenSim.Server.GridServer/AppData\nini_master GridServer.ini.example\n"),
        };
    }

    static Dictionary<string, string> RegionOrderFiles()
    {
        return new Dictionary<string, string>
        {
            ["tree/master.ini"] = "[Startup]\nA = master\nInclude-One = one.ini\nInclude-Gone = gone.ini\nSeen = ${Environment|FERROAN_CONFIG_ORACLE_SAMPLE}\n[Environment]\nFERROAN_CONFIG_ORACLE_SAMPLE = template\n",
            ["tree/one.ini"] = "[Startup]\nB = from-one\nA = from-one\n",
            ["tree/user.ini"] = "[Startup]\nA = from-user\nInclude-Two = two.ini\n",
            ["tree/two.ini"] = "[Startup]\nC = from-two\n",
            ["tree/overrides/z.ini"] = "[Startup]\nD = from-override\n",
        };
    }

    static string RegionOrderLoad()
    {
        return "ini_master master.ini\nini_file user.ini\nini_directory overrides\n" +
            "env FERROAN_CONFIG_ORACLE_SAMPLE=oracle-sample\nargv Startup Note from-argv\n";
    }

    static Dictionary<string, string> DirectoryFiles()
    {
        return new Dictionary<string, string>
        {
            ["tree/master.ini"] = "[Startup]\nA = master\nInclude-One = one.ini\n",
            ["tree/one.ini"] = "[Startup]\nA = include\n",
            ["tree/overrides/z.ini"] = "[Startup]\nA = directory\n",
        };
    }

    static Dictionary<string, string> GridIncludeFiles()
    {
        return new Dictionary<string, string>
        {
            ["tree/master.ini"] = "[Service]\nA = master\nInclude-One = one.ini\n",
            ["tree/one.ini"] = "[Service]\nA = include\nB = extra\n",
            ["tree/overrides/z.ini"] = "[Service]\nA = directory\n",
        };
    }
}

sealed class Case
{
    public readonly string Id;
    public readonly string Kind;
    public readonly string Source;
    public readonly Dictionary<string, string> Files;

    public Case(string id, string kind, string source)
    {
        Id = id;
        Kind = kind;
        Source = source;
    }

    public Case(string id, string kind, string source, Dictionary<string, string> files, string loadText)
        : this(id, kind, source)
    {
        Files = files ?? new Dictionary<string, string>();
        Files["load.txt"] = loadText;
    }
}
