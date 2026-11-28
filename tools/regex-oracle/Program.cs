// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

if (args.Length < 3)
{
    Console.Error.WriteLine("usage: RegexOracle patterns.json digest.json literals.json [full.json]");
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
