// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

if (args.Length > 0 && args[0] == "--groups")
{
    return WriteGroups(args);
}

if (args.Length > 0 && args[0] == "--enumerate")
{
    return WriteEnumeration(args);
}

if (args.Length < 3)
{
    Console.Error.WriteLine("usage: RegexOracle patterns.json digest.json literals.json [full.json]");
    Console.Error.WriteLine("       RegexOracle --groups patterns.json digest.json literals.json");
    return 1;
}

var description = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;
if (!description.Contains("10.0.14", StringComparison.Ordinal))
{
    Console.Error.WriteLine("runtime " + description);
    return 1;
}

var patterns = JsonSerializer.Deserialize<string[]>(File.ReadAllText(args[0]));
if (patterns == null || patterns.Length == 0)
{
    Console.Error.WriteLine("no patterns");
    return 1;
}

var canonical = new StringBuilder();
var full = new List<object>();
var literals = new List<object>();
var seen = new HashSet<string>();
foreach (var pattern in patterns)
{
    try
    {
        _ = new Regex(pattern);
        canonical.Append("OK\n");
        full.Add(new { pattern, ok = true });
    }
    catch (RegexParseException ex)
    {
        canonical.Append("ERR\n").Append(ex.Message).Append('\n');
        full.Add(new { pattern, ok = false, message = ex.Message });
        var prefix = "Invalid pattern '" + pattern + "' at offset " + ex.Offset.ToString() + ". ";
        var detail = ex.Message.StartsWith(prefix, StringComparison.Ordinal)
            ? ex.Message.Substring(prefix.Length)
            : ex.Message;
        var pinned = pattern is "^?*" or "$?*" or ".?*" or "[a]?*" or "(a)?*" or "\\p{L}?*" or "\\w?*" or "?*";
        if (seen.Add(detail) || pinned)
        {
            literals.Add(new { pattern, ok = false, message = ex.Message });
        }
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine(ex.GetType().FullName + " " + pattern);
        return 1;
    }
}

foreach (var pattern in new[] { "", "a", "(?<é>a)", "😀*", "[a-z-[b]]", "\\p{Lu}" })
{
    if (Array.IndexOf(patterns, pattern) >= 0)
    {
        literals.Add(new { pattern, ok = true });
    }
}

var digest = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(canonical.ToString()))).ToLowerInvariant();
var options = new JsonSerializerOptions { WriteIndented = true };
File.WriteAllText(args[1], JsonSerializer.Serialize(new
{
    runtime = "10.0.14",
    pin = "41f18f983bb2bfc82e033fed410d361ba57d5aa4",
    digest,
}, options) + "\n");
File.WriteAllText(args[2], JsonSerializer.Serialize(new { cases = literals }, options) + "\n");
if (args.Length > 3)
{
    File.WriteAllText(args[3], JsonSerializer.Serialize(new { digest, outcomes = full }, options) + "\n");
}

Console.WriteLine(patterns.Length + " patterns " + digest);
return 0;

static int WriteGroups(string[] args)
{
    if (args.Length != 4)
    {
        Console.Error.WriteLine("usage: RegexOracle --groups patterns.json digest.json literals.json");
        return 1;
    }

    var description = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;
    if (!description.Contains("10.0.14", StringComparison.Ordinal))
    {
        Console.Error.WriteLine("runtime " + description);
        return 1;
    }

    var patterns = JsonSerializer.Deserialize<string[]>(File.ReadAllText(args[1]));
    if (patterns == null || patterns.Length == 0)
    {
        Console.Error.WriteLine("no patterns");
        return 1;
    }

    var canonical = new StringBuilder();
    var literals = new List<object>();
    foreach (var pattern in patterns)
    {
        Regex regex;
        try
        {
            regex = new Regex(pattern);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(ex.GetType().FullName + " " + pattern + " " + ex.Message);
            return 1;
        }

        var numbers = string.Join(",", regex.GetGroupNumbers());
        var names = string.Join("\u001f", regex.GetGroupNames());
        canonical.Append(pattern).Append('\n').Append(numbers).Append('\n').Append(names).Append('\n');
        var named = false;
        var groupNames = regex.GetGroupNames();
        var groupNumbers = regex.GetGroupNumbers();
        for (var i = 0; i < groupNames.Length; i++)
        {
            if (groupNames[i] != groupNumbers[i].ToString())
            {
                named = true;
                break;
            }
        }

        if (named || pattern is "" or "a|b" or "(a)(b)" or "(a)*" or "(a){0}" or "()" or "(?(a)b|c)" or "(?n)(a)b" or "(?<2>a)" or "(?<10>a)(b)")
        {
            literals.Add(new { pattern, numbers = groupNumbers, names = groupNames });
        }
    }

    var digest = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(canonical.ToString()))).ToLowerInvariant();
    var options = new JsonSerializerOptions { WriteIndented = true };
    File.WriteAllText(args[2], JsonSerializer.Serialize(new
    {
        runtime = "10.0.14",
        pin = "41f18f983bb2bfc82e033fed410d361ba57d5aa4",
        digest,
    }, options) + "\n");
    File.WriteAllText(args[3], JsonSerializer.Serialize(new { cases = literals }, options) + "\n");
    Console.WriteLine(patterns.Length + " patterns " + digest);
    return 0;
}

static int WriteEnumeration(string[] args)
{
    if (args.Length != 4)
    {
        Console.Error.WriteLine("usage: RegexOracle --enumerate cases.json digest.json literals.json");
        return 1;
    }

    var description = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;
    if (!description.Contains("10.0.14", StringComparison.Ordinal))
    {
        Console.Error.WriteLine("runtime " + description);
        return 1;
    }

    var cases = JsonSerializer.Deserialize<EnumCase[]>(File.ReadAllText(args[1]));
    if (cases == null || cases.Length == 0)
    {
        Console.Error.WriteLine("no cases");
        return 1;
    }

    var canonical = new StringBuilder();
    var literals = new List<object>();
    foreach (var row in cases)
    {
        if (row.pattern == null || row.subject == null || row.pattern.Contains('\n') || row.subject.Contains('\n'))
        {
            Console.Error.WriteLine("pattern or subject is missing or contains a newline");
            return 1;
        }

        Regex regex;
        try
        {
            regex = new Regex(row.pattern);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(ex.GetType().FullName + " " + row.pattern + " " + ex.Message);
            return 1;
        }

        var match = regex.Match(row.subject);
        if (!match.Success)
        {
            Console.Error.WriteLine("subject did not match " + row.pattern);
            return 1;
        }

        var numbers = regex.GetGroupNumbers();
        var names = regex.GetGroupNames();
        if (match.Groups.Count != numbers.Length || names.Length != numbers.Length)
        {
            Console.Error.WriteLine("count " + match.Groups.Count + " numbers " + numbers.Length + " " + row.pattern);
            return 1;
        }

        var enumerated = new List<object>();
        canonical.Append(row.pattern).Append('\n').Append(row.subject).Append('\n');
        canonical.Append(string.Join(",", numbers)).Append('\n');
        canonical.Append(string.Join("\u001f", names)).Append('\n');
        canonical.Append("E\n");
        foreach (Group group in match.Groups)
        {
            if (group.Name.Contains('\n') || group.Value.Contains('\n'))
            {
                Console.Error.WriteLine("newline in a captured field " + row.pattern);
                return 1;
            }

            canonical.Append(group.Success ? '1' : '0').Append('\n');
            canonical.Append(group.Index).Append('\n');
            canonical.Append(group.Name).Append('\n');
            canonical.Append(group.Value).Append('\n');
            enumerated.Add(new
            {
                success = group.Success,
                index = group.Index,
                name = group.Name,
                value = group.Value,
            });
        }

        var declared = new List<object>();
        canonical.Append("D\n");
        foreach (var number in numbers)
        {
            Group group = match.Groups[number];
            if (group.Name.Contains('\n') || group.Value.Contains('\n'))
            {
                Console.Error.WriteLine("newline in a captured field " + row.pattern);
                return 1;
            }

            canonical.Append(number).Append('\n');
            canonical.Append(group.Success ? '1' : '0').Append('\n');
            canonical.Append(group.Index).Append('\n');
            canonical.Append(group.Name).Append('\n');
            canonical.Append(group.Value).Append('\n');
            declared.Add(new
            {
                number,
                success = group.Success,
                index = group.Index,
                name = group.Name,
                value = group.Value,
            });
        }

        canonical.Append(".\n");
        literals.Add(new
        {
            pattern = row.pattern,
            subject = row.subject,
            numbers,
            names,
            enumerated,
            declared,
        });
    }

    var digest = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(canonical.ToString()))).ToLowerInvariant();
    var options = new JsonSerializerOptions { WriteIndented = true };
    File.WriteAllText(args[2], JsonSerializer.Serialize(new
    {
        runtime = "10.0.14",
        pin = "41f18f983bb2bfc82e033fed410d361ba57d5aa4",
        digest,
    }, options) + "\n");
    File.WriteAllText(args[3], JsonSerializer.Serialize(new { cases = literals }, options) + "\n");
    Console.WriteLine(cases.Length + " matches " + digest);
    return 0;
}

sealed class EnumCase
{
    public string pattern { get; set; }
    public string subject { get; set; }
}
