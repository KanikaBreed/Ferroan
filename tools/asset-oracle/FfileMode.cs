// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Reads and writes FSAssets files with the pin's HashToPath, HashToFile and
// GetFsData. The connector is not constructed, so it does not open a database
// or start the writer thread.

using System.IO.Compression;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using OpenSim.Services.FSAssetService;

namespace Ferroan.AssetOracle;

static class FfileMode
{
    sealed class Probe : FSAssetConnector
    {
        Probe() : base(null)
        {
        }

        public static Probe Create(string baseDirectory, string spoolDirectory, bool osgrid)
        {
            Probe probe = (Probe)RuntimeHelpers.GetUninitializedObject(typeof(Probe));
            probe.m_FSBase = baseDirectory;
            probe.m_SpoolDirectory = spoolDirectory;
            probe.m_useOsgridFormat = osgrid;
            return probe;
        }
    }

    public static int Run(string[] args)
    {
        if (args.Length < 2)
            return Usage();
        if (args[1] == "path" && args.Length == 4)
        {
            Probe probe = Probe.Create(string.Empty, string.Empty, Flag(args[2]));
            Write(new JsonObject
            {
                ["path"] = probe.HashToPath(args[3]),
                ["file"] = probe.HashToFile(args[3])
            });
            return 0;
        }
        if (args[1] == "read" && args.Length == 6)
        {
            Probe probe = Probe.Create(args[2], args[3], Flag(args[4]));
            byte[] data = probe.GetFsData(args[5]) ?? Array.Empty<byte>();
            Write(new JsonObject
            {
                ["path"] = probe.HashToPath(args[5]),
                ["file"] = probe.HashToFile(args[5]),
                ["data"] = Bytes(data)
            });
            return 0;
        }
        if ((args[1] == "write" || args[1] == "plain") && args.Length == 7)
        {
            byte[] data = File.ReadAllBytes(args[6]);
            Probe probe = Probe.Create(args[2], args[3], Flag(args[4]));
            string relative = probe.HashToFile(args[5]);
            string disk = Path.Combine(args[2], relative);
            if (args[1] == "write")
                WriteGzip(disk + ".gz", data);
            else
                WritePlain(disk, data);
            Write(new JsonObject
            {
                ["path"] = probe.HashToPath(args[5]),
                ["file"] = relative,
                ["length"] = data.Length
            });
            return 0;
        }
        return Usage();
    }

    static bool Flag(string text)
    {
        if (text == "1")
            return true;
        if (text == "0")
            return false;
        throw new ArgumentException("osgrid must be 0 or 1");
    }

    static void WriteGzip(string path, byte[] data)
    {
        string directory = Path.GetDirectoryName(path);
        if (string.IsNullOrEmpty(directory))
            throw new InvalidOperationException("the asset path has no directory");
        Directory.CreateDirectory(directory);
        using FileStream stream = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None);
        using (GZipStream gzip = new GZipStream(stream, CompressionMode.Compress, true))
            gzip.Write(data, 0, data.Length);
        stream.Flush(true);
    }

    static void WritePlain(string path, byte[] data)
    {
        string directory = Path.GetDirectoryName(path);
        if (string.IsNullOrEmpty(directory))
            throw new InvalidOperationException("the asset path has no directory");
        Directory.CreateDirectory(directory);
        File.WriteAllBytes(path, data);
    }

    static JsonArray Bytes(byte[] data)
    {
        JsonArray array = new JsonArray();
        foreach (byte value in data)
            array.Add(value);
        return array;
    }

    static void Write(JsonObject document)
    {
        Console.WriteLine(document.ToJsonString(new JsonSerializerOptions { NewLine = "\n" }));
    }

    static int Usage()
    {
        Console.Error.WriteLine("usage: AssetOracle ffile path <0|1> <hash>");
        Console.Error.WriteLine("       AssetOracle ffile read <base> <spool> <0|1> <hash>");
        Console.Error.WriteLine("       AssetOracle ffile write <base> <spool> <0|1> <hash> <bytes-file>");
        Console.Error.WriteLine("       AssetOracle ffile plain <base> <spool> <0|1> <hash> <bytes-file>");
        return 2;
    }
}
