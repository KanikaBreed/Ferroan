// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Drives Nini 0.9.2.16 and OpenSim.Framework.Util the way Tranquillity does.
// Region sequence: Source/OpenSim.Server.RegionServer/ConfigurationLoader.cs
// Grid sequence: Source/OpenSim.Server.GridServer/GridServerConfigSource.cs
// OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// This file is the call sequence, not a copy of those sources.

using Nini.Config;
using Nini.Ini;
using OpenSim.Framework;

namespace Ferroan.ConfigOracle;

static class Loader
{
    public const int MaxSources = 10000;

    public static Outcome LoadRegion(string configDir, IConfigSource argv, IDictionary<string, string> environment)
    {
        IConfig startup = argv.Configs["Startup"];
        string master = startup == null
            ? "RegionServerDefaults.ini"
            : startup.GetString("inimaster", "RegionServerDefaults.ini");
        if (master == "none")
            master = "";

        var iniNames = new List<string>
        {
            startup == null ? "OpenSim.ini" : startup.GetString("inifile", "OpenSim.ini")
        };
        for (int number = 2; number < 1000; number++)
        {
            string key = "inifile" + number;
            if (startup == null || !startup.Contains(key))
                break;
            iniNames.Add(startup.GetString(key));
        }

        var sources = new List<string>();
        if (IsHttp(master))
            return Outcome.Fail("remote-include\t" + Escape(master));
        if (master.Length != 0)
        {
            string path = FullPath(configDir, master);
            if (!File.Exists(path))
                return Outcome.Fail("missing-master\t" + Escape(master));
            AddSource(sources, path);
        }

        foreach (string name in iniNames)
        {
            if (IsHttp(name))
                return Outcome.Fail("remote-include\t" + Escape(name));
            string path = FullPath(configDir, name);
            if (!File.Exists(path))
            {
                string xml = FullPath(configDir, "OpenSim.xml");
                if (File.Exists(xml))
                    return Outcome.Fail("xml-config");
                continue;
            }
            AddSource(sources, path);
        }

        var warnings = new List<string>();
        IConfigSource config = new IniConfigSource();
        for (int index = 0; index < sources.Count; index++)
        {
            Outcome parsed = MergeFile(config, sources[index]);
            if (parsed != null)
                return parsed;
            Outcome includes = AddIncludes(config, sources, configDir, warnings);
            if (includes != null)
                return includes;
        }

        string iniDirectory = startup == null ? "config" : startup.GetString("inidirectory", "config");
        string directory = Locate(configDir, iniDirectory);
        if (Directory.Exists(directory))
        {
            var overrideSources = new List<string>();
            foreach (string file in Directory.GetFiles(directory))
            {
                if (Path.GetExtension(file).ToLowerInvariant() != ".ini")
                    continue;
                string full = Path.GetFullPath(file);
                if (!sources.Contains(full))
                {
                    AddSource(overrideSources, full);
                    AddSource(sources, full);
                }
            }

            IConfigSource overrideConfig = new IniConfigSource();
            for (int index = 0; index < overrideSources.Count; index++)
            {
                Outcome parsed = MergeFile(overrideConfig, overrideSources[index]);
                if (parsed != null)
                    return parsed;
                Outcome includes = AddIncludes(overrideConfig, overrideSources, configDir, warnings);
                if (includes != null)
                    return includes;
            }
            config.Merge(overrideConfig);
        }

        if (sources.Count == 0)
            return Outcome.Fail("no-config");

        return Finish(config, environment, argv, warnings);
    }

    public static Outcome LoadGrid(
        string configDir, string iniMaster, IList<string> iniFiles, string iniDirectory, IDictionary<string, string> environment)
    {
        if (string.IsNullOrEmpty(iniMaster))
            iniMaster = "OpenSim.Server.GridServer.ini";

        var sources = new List<string>();
        IConfigSource config = new IniConfigSource();
        var warnings = new List<string>();
        Outcome failed = LoadGridSource(iniMaster, configDir, config, sources);
        if (failed != null)
            return failed;
        if (iniFiles != null)
        {
            foreach (string file in iniFiles)
            {
                failed = LoadGridSource(file, configDir, config, sources);
                if (failed != null)
                    return failed;
            }
        }
        if (!string.IsNullOrEmpty(iniDirectory))
        {
            string directory = Locate(configDir, iniDirectory);
            if (Directory.Exists(directory))
            {
                foreach (string file in Directory.GetFiles(directory, "*.ini"))
                {
                    failed = LoadGridSource(file, configDir, config, sources);
                    if (failed != null)
                        return failed;
                }
            }
        }

        int index = sources.Count;
        while (true)
        {
            int before = sources.Count;
            failed = AddIncludes(config, sources, configDir, warnings);
            if (failed != null)
                return failed;
            if (sources.Count == before)
                break;
            for (; index < sources.Count; index++)
            {
                failed = MergeFile(config, sources[index]);
                if (failed != null)
                    return failed;
            }
        }

        return Finish(config, environment, null, warnings);
    }

    public static Outcome ParseText(string text, bool expand)
    {
        try
        {
            IConfigSource source = new IniConfigSource(new StringReader(text));
            if (expand)
                source.ReplaceKeyValues();
            return Outcome.Ok(Rows(source), new List<string>());
        }
        catch (Exception exception)
        {
            string mapped = MapException(exception);
            if (mapped == null)
                throw;
            return Outcome.Fail(mapped);
        }
    }

    static Outcome Finish(
        IConfigSource config, IDictionary<string, string> environment, IConfigSource argv, List<string> warnings)
    {
        try
        {
            IConfig section = config.Configs["Environment"];
            if (section != null)
            {
                foreach (string key in section.GetKeys())
                {
                    string value = "";
                    if (environment != null && environment.ContainsKey(key))
                        value = environment[key] ?? "";
                    section.Set(key, value);
                }
                config.ExpandKeyValues();
            }
            if (argv != null)
                config.Merge(argv);
            config.ReplaceKeyValues();
            return Outcome.Ok(Rows(config), warnings);
        }
        catch (Exception exception)
        {
            string mapped = MapException(exception);
            if (mapped == null)
                throw;
            return Outcome.Fail(mapped);
        }
    }

    static Outcome LoadGridSource(string stored, string configDir, IConfigSource config, List<string> sources)
    {
        if (string.IsNullOrEmpty(stored) || sources.Contains(stored))
            return null;
        if (IsHttp(stored))
            return Outcome.Fail("remote-include\t" + Escape(stored));
        string path = Locate(configDir, stored);
        if (!File.Exists(path))
            return null;
        Outcome failed = MergeFile(config, path);
        if (failed != null)
            return failed;
        AddSource(sources, stored);
        return null;
    }

    static Outcome MergeFile(IConfigSource config, string path)
    {
        try
        {
            config.Merge(new IniConfigSource(path));
            return null;
        }
        catch (Exception exception)
        {
            string mapped = MapException(exception);
            if (mapped == null)
                throw;
            return Outcome.Fail(mapped);
        }
    }

    static Outcome AddIncludes(IConfigSource config, List<string> sources, string configDir, List<string> warnings)
    {
        var includes = new List<string>();
        foreach (IConfig section in config.Configs)
        {
            foreach (string key in section.GetKeys())
            {
                if (key.StartsWith("Include-", StringComparison.Ordinal))
                    includes.Add(section.GetString(key));
            }
        }
        foreach (string file in includes)
        {
            if (IsHttp(file))
                return Outcome.Fail("remote-include\t" + Escape(file));
            bool wildcard = file.IndexOfAny(new char[] { '*', '?' }) >= 0;
            string[] paths = ResolveInclude(configDir, file);
            if (!wildcard && paths.Length == 0)
            {
                if (!warnings.Contains(file))
                    warnings.Add(file);
                continue;
            }
            foreach (string path in paths)
                AddSource(sources, path);
        }
        return null;
    }

    static string[] ResolveInclude(string configDir, string file)
    {
        int wildcard = file.IndexOfAny(new char[] { '*', '?' });
        string prefix = wildcard < 0 ? file : file.Substring(0, wildcard);
        string suffix = wildcard < 0 ? "" : file.Substring(wildcard);
        string combined = Path.Combine(configDir, prefix);
        string full = Path.GetFullPath(combined) + suffix;
        string[] found = Util.Glob(full);
        Array.Sort(found, StringComparer.Ordinal);
        return found;
    }

    static List<string> Rows(IConfigSource source)
    {
        var rows = new List<string>();
        foreach (IConfig section in source.Configs)
        {
            foreach (string key in section.GetKeys())
                rows.Add(Escape(section.Name) + "\t" + Escape(key) + "\t" + Escape(section.Get(key) ?? ""));
        }
        rows.Sort(StringComparer.Ordinal);
        return rows;
    }

    public static string Escape(string text)
    {
        if (text == null)
            return "";
        var builder = new System.Text.StringBuilder(text.Length);
        foreach (char ch in text)
        {
            switch (ch)
            {
                case '\\': builder.Append("\\\\"); break;
                case '\n': builder.Append("\\n"); break;
                case '\r': builder.Append("\\r"); break;
                case '\t': builder.Append("\\t"); break;
                default: builder.Append(ch); break;
            }
        }
        return builder.ToString();
    }

    static string MapException(Exception exception)
    {
        if (exception is IniException)
            return "parse";
        string message = exception.Message ?? "";
        const string selfPrefix = "Key cannot have a expand value of itself: ";
        const string sectionPrefix = "Expand config not found: ";
        const string keyPrefix = "Key not found: ";
        const string otherKeyPrefix = "Expand key not found: ";
        if (message.StartsWith(selfPrefix, StringComparison.Ordinal))
            return "expand-self\t" + Escape(message.Substring(selfPrefix.Length));
        if (message.StartsWith(sectionPrefix, StringComparison.Ordinal))
            return "expand-section\t" + Escape(message.Substring(sectionPrefix.Length));
        if (message.StartsWith(keyPrefix, StringComparison.Ordinal))
            return "expand-key\t" + Escape(message.Substring(keyPrefix.Length));
        if (message.StartsWith(otherKeyPrefix, StringComparison.Ordinal))
            return "expand-key\t" + Escape(message.Substring(otherKeyPrefix.Length));
        return null;
    }

    static void AddSource(List<string> sources, string path)
    {
        if (sources.Count >= MaxSources)
            throw new InvalidOperationException("include file limit");
        if (!sources.Contains(path))
            sources.Add(path);
    }

    static string Locate(string configDir, string name)
    {
        if (Path.IsPathRooted(name))
            return name;
        return Path.Combine(configDir, name);
    }

    static string FullPath(string configDir, string name)
    {
        return Path.GetFullPath(Path.Combine(configDir, name));
    }

    static bool IsHttp(string value)
    {
        if (string.IsNullOrEmpty(value) || value.IndexOfAny(new char[] { ' ', '\t', '\r', '\n' }) >= 0)
            return false;
        return Uri.TryCreate(value, UriKind.Absolute, out Uri uri)
            && (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps);
    }
}

sealed class Outcome
{
    public string Error;
    public List<string> Rows = new();
    public List<string> Warnings = new();

    public static Outcome Fail(string error)
    {
        return new Outcome { Error = error };
    }

    public static Outcome Ok(List<string> rows, List<string> warnings)
    {
        return new Outcome { Rows = rows, Warnings = warnings };
    }
}
