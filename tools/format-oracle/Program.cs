// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

if (args.Length < 3)
{
    Console.Error.WriteLine("usage: FormatOracle cases.json digest.json literals.json [full.json]");
    return 1;
}

var description = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;
if (!description.Contains("10.0.14", StringComparison.Ordinal))
{
    Console.Error.WriteLine("runtime " + description);
    return 1;
}

CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("en-US");

JsonDocument document;
try
{
    document = JsonDocument.Parse(File.ReadAllText(args[0]));
}
catch (Exception ex)
{
    Console.Error.WriteLine(ex.Message);
    return 1;
}

if (document.RootElement.ValueKind != JsonValueKind.Array || document.RootElement.GetArrayLength() == 0)
{
    Console.Error.WriteLine("no cases");
    return 1;
}

const int AgreeMax = 9_999_999;
const int RenderCap = 4096;
var canonical = new StringBuilder();
var full = new List<object>();
var literals = new List<object>();
var seenErrors = new HashSet<string>();

foreach (var item in document.RootElement.EnumerateArray())
{
    string format = null;
    if (item.TryGetProperty("format", out var formatElement) && formatElement.ValueKind != JsonValueKind.Null)
    {
        format = formatElement.GetString();
    }
    if (!item.TryGetProperty("args", out var argsElement) || argsElement.ValueKind != JsonValueKind.Array)
    {
        Console.Error.WriteLine("case missing args");
        return 1;
    }
    var callArgs = new List<object>();
    foreach (var arg in argsElement.EnumerateArray())
    {
        if (arg.ValueKind != JsonValueKind.String)
        {
            Console.Error.WriteLine("argument is not a string");
            return 1;
        }
        callArgs.Add(arg.GetString());
    }

    string rendered = null;
    Exception failure = null;
    try
    {
        rendered = string.Format(format, callArgs.ToArray());
        if (rendered.Length > RenderCap)
        {
            Console.Error.WriteLine("rendered length " + rendered.Length.ToString(CultureInfo.InvariantCulture));
            return 1;
        }
    }
    catch (Exception ex)
    {
        failure = ex;
    }

    string text;
    if (failure == null)
    {
        Segment[] segments;
        try
        {
            segments = ReadSegments(format);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("segments " + ex.GetType().Name + " " + Show(format));
            return 1;
        }
        if (!Agrees(segments, AgreeMax))
        {
            Console.Error.WriteLine("segment disagreement " + Show(format));
            return 1;
        }
        text = OkText(rendered, segments);
        if (rendered.Length <= 64)
        {
            literals.Add(new { format, args = callArgs.Select(arg => (string)arg).ToArray(), ok = true, rendered });
        }
        full.Add(new { format, canonical = text });
    }
    else
    {
        var typeName = failure.GetType().Name;
        var message = failure.Message ?? "";
        text = "ERR\n" + typeName + "\n" + message + "\n";
        var parseFailure = message.Contains("Failure to parse", StringComparison.Ordinal);
        if (!parseFailure && format != null)
        {
            try
            {
                var segments = ReadSegments(format);
                if (Agrees(segments, AgreeMax))
                {
                    text += SegmentsText(segments);
                }
            }
            catch (FormatException)
            {
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine("segments " + ex.GetType().Name + " " + Show(format));
                return 1;
            }
        }
        if (seenErrors.Add(typeName + "\n" + message))
        {
            literals.Add(new
            {
                format,
                args = callArgs.Select(arg => (string)arg).ToArray(),
                ok = false,
                type = typeName,
                message,
            });
        }
        full.Add(new { format, canonical = text });
    }
    canonical.Append(text);
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

Console.WriteLine(document.RootElement.GetArrayLength().ToString(CultureInfo.InvariantCulture) + " cases " + digest);
return 0;

static string OkText(string rendered, Segment[] segments)
{
    var builder = new StringBuilder();
    builder.Append("OK\nR ").Append(rendered.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
    builder.Append(rendered).Append('\n');
    builder.Append(SegmentsText(segments));
    return builder.ToString();
}

static string SegmentsText(Segment[] segments)
{
    var builder = new StringBuilder();
    foreach (var segment in segments)
    {
        if (segment.Literal != null)
        {
            builder.Append("L ").Append(segment.Literal.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
            builder.Append(segment.Literal).Append('\n');
            continue;
        }
        if (segment.Format == null)
        {
            builder.Append("H ").Append(segment.Index.ToString(CultureInfo.InvariantCulture));
            builder.Append(' ').Append(segment.Alignment.ToString(CultureInfo.InvariantCulture));
            builder.Append(" -\n");
            continue;
        }
        builder.Append("H ").Append(segment.Index.ToString(CultureInfo.InvariantCulture));
        builder.Append(' ').Append(segment.Alignment.ToString(CultureInfo.InvariantCulture));
        builder.Append(' ').Append(segment.Format.Length.ToString(CultureInfo.InvariantCulture)).Append('\n');
        builder.Append(segment.Format).Append('\n');
    }
    return builder.ToString();
}

static bool Agrees(Segment[] segments, int max)
{
    foreach (var segment in segments)
    {
        if (segment.Literal != null)
        {
            continue;
        }
        if (segment.Index < 0 || segment.Index > max)
        {
            return false;
        }
        if (segment.Alignment < -max || segment.Alignment > max)
        {
            return false;
        }
    }
    return true;
}

static Segment[] ReadSegments(string format)
{
    var parsed = CompositeFormat.Parse(format);
    var field = typeof(CompositeFormat).GetField("_segments", BindingFlags.Instance | BindingFlags.NonPublic);
    if (field == null)
    {
        throw new InvalidOperationException("CompositeFormat has no _segments");
    }
    var array = field.GetValue(parsed) as Array;
    if (array == null)
    {
        throw new InvalidOperationException("segments missing");
    }
    var segments = new Segment[array.Length];
    for (var i = 0; i < array.Length; i++)
    {
        var tuple = array.GetValue(i) as ITuple;
        if (tuple == null || tuple.Length != 4)
        {
            throw new InvalidOperationException("segment shape");
        }
        segments[i] = new Segment
        {
            Literal = tuple[0] as string,
            Index = (int)tuple[1],
            Alignment = (int)tuple[2],
            Format = tuple[3] as string,
        };
    }
    return segments;
}

static string Show(string format)
{
    if (format == null)
    {
        return "null";
    }
    return format.Replace("\n", "\\n", StringComparison.Ordinal).Replace("\t", "\\t", StringComparison.Ordinal);
}

sealed class Segment
{
    public string Literal;
    public int Index;
    public int Alignment;
    public string Format;
}
