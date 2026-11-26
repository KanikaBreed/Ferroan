using System.Globalization;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Xml;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Framework.Serialization;
using OpenSim.Region.CoreModules.World.Archiver;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Interfaces;

/// <summary>
/// Records Pin8 <c>load oar</c> for the control file and the region path.
/// The archives are written with <c>TarArchiveWriter</c> and, where noted,
/// <c>CreateControlFile</c>. Each case then runs <c>FindAndLoadControlFile</c>
/// and the <c>GetRegionFromPath</c> gate, and <c>DearchiveRegion(false)</c>.
/// </summary>
internal static class Program
{
    const string Stamp = "1700000000";
    const string SaveHome = "11111111-1111-1111-1111-111111111111";
    const string SaveWest = "11111111-1111-1111-1111-111111111111";
    const string SaveEast = "22222222-2222-2222-2222-222222222222";
    const string SaveNorth = "33333333-3333-3333-3333-333333333333";
    const string LoadHome = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const string LoadWest = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const string LoadEast = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    const string LoadKept = "dddddddd-dddd-dddd-dddd-dddddddddddd";
    const string LoadRootSwap = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    const string Decoy = "99999999-9999-9999-9999-999999999999";

    static readonly FieldInfo IncomingSize = typeof(ArchiveReadRequest).GetField(
        "m_incomingRegionSize",
        BindingFlags.Instance | BindingFlags.NonPublic);
    static readonly MethodInfo FindControl = typeof(ArchiveReadRequest).GetMethod(
        "FindAndLoadControlFile",
        BindingFlags.Instance | BindingFlags.NonPublic);

    internal static int[] IncomingSizeBits(ArchiveReadRequest request)
    {
        if (request == null || IncomingSize == null)
            return new[] { 0, 0, 0 };
        if (IncomingSize.GetValue(request) is not Vector3 size)
            return new[] { 0, 0, 0 };
        return new[]
        {
            BitConverter.SingleToInt32Bits(size.X),
            BitConverter.SingleToInt32Bits(size.Y),
            BitConverter.SingleToInt32Bits(size.Z)
        };
    }

    static int Main(string[] args)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        if (IncomingSize == null || FindControl == null)
        {
            Console.Error.WriteLine("ArchiveReadRequest is missing the control-file members");
            return 1;
        }
        if (args.Length != 3 || args[0] != "generate")
        {
            Console.Error.WriteLine("usage: OarOracle generate <directory> <pin>");
            return 2;
        }

        string dir = args[1];
        string pin = args[2];
        Directory.CreateDirectory(dir);
        var cases = new List<Case>();
        cases.Add(SingleFirst(dir));
        cases.Add(SingleDirFirst(dir));
        cases.Add(SingleLate(dir));
        cases.Add(SingleUnregistered(dir));
        cases.Add(MultiOffset(dir));
        cases.Add(MissingControl(dir));
        cases.Add(MajorTwo(dir));
        cases.Add(CompactSkip(dir));
        cases.Add(DirBeforeId(dir));
        cases.Add(BadFields(dir));
        cases.Add(StreamSeek(dir));
        cases.Add(StreamNoSeek(dir));

        string jsonPath = Path.Combine(dir, "load.json");
        using var stream = File.Create(jsonPath);
        using var writer = new Utf8JsonWriter(stream, new JsonWriterOptions { Indented = true });
        writer.WriteStartObject();
        writer.WriteString("pin", pin);
        writer.WriteStartArray("cases");
        foreach (Case item in cases)
            item.Write(writer);
        writer.WriteEndArray();
        writer.WriteEndObject();
        writer.Flush();
        stream.WriteByte((byte)'\n');
        Console.WriteLine("wrote " + cases.Count + " cases");
        return 0;
    }

    static Case SingleFirst(string dir)
    {
        string path = Path.Combine(dir, "single-first.oar");
        string xml = SingleControl(HomeSave());
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, xml);
            tar.WriteDir("notes");
            tar.WriteFile("notes/short.txt", "pin8");
            tar.WriteFile("notes/" + new string('n', 120) + ".txt", "long");
            tar.WriteFile("shared/foo.bin", new byte[] { 1, 2, 3 });
            tar.WriteFile("Regions/Nope.txt", "case");
        });
        return Run("single-first", "single-first.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case SingleDirFirst(string dir)
    {
        string path = Path.Combine(dir, "single-dir-first.oar");
        WriteGzip(path, tar =>
        {
            tar.WriteDir("notes");
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, SingleControl(HomeSave()));
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("single-dir-first", "single-dir-first.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case SingleLate(string dir)
    {
        string path = Path.Combine(dir, "single-late.oar");
        WriteGzip(path, tar =>
        {
            tar.WriteFile("early.txt", "early");
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, SingleControl(HomeSave()));
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("single-late", "single-late.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case SingleUnregistered(string dir)
    {
        string archive = Path.Combine(dir, "single-first.oar");
        Placed decoy = Place(Decoy, "Decoy", 9, 9, 256);
        return Run("single-unregistered", "single-first.oar", "gzip-path", archive, HomeRoot(), new List<Placed> { decoy });
    }

    static Case MultiOffset(string dir)
    {
        string path = Path.Combine(dir, "multi-offset.oar");
        string xml = MultiControl();
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, xml);
            tar.WriteDir("regions/1_1_West/extra");
            tar.WriteFile("regions/1_1_West/notes.txt", "west");
            tar.WriteFile("regions/3_1_East_Isle/notes.txt", "east");
            tar.WriteFile("regions/1_3_North/notes.txt", "north");
            tar.WriteFile("regions/9_9_Gone/notes.txt", "gone");
            tar.WriteFile("regions/1_1_West", "two-parts");
            tar.WriteFile("regions/1_1_West/" + new string('n', 120) + ".txt", "long");
            tar.WriteFile("shared/foo.bin", new byte[] { 4 });
            tar.WriteFile("Regions/Nope.txt", "case");
        });
        Placed west = Place(LoadWest, "Load West", 50, 60, 256);
        Placed east = Place(LoadEast, "Load East", 52, 60, 256);
        Placed decoy = Place(Decoy, "Decoy", 1, 1, 256);
        return Run("multi-offset", "multi-offset.oar", "gzip-path", path, west, new List<Placed> { west, east, decoy });
    }

    static Case MissingControl(string dir)
    {
        string path = Path.Combine(dir, "missing-control.oar");
        WriteGzip(path, tar => tar.WriteFile("early.txt", "early"));
        return Run("missing-control", "missing-control.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case MajorTwo(string dir)
    {
        string path = Path.Combine(dir, "major-two.oar");
        string xml = SingleControl(HomeSave()).Replace("major_version=\"0\"", "major_version=\"2\"", StringComparison.Ordinal);
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, xml);
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("major-two", "major-two.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case CompactSkip(string dir)
    {
        string path = Path.Combine(dir, "compact-skip.oar");
        string xml = Compact(SingleControl(HomeSave()));
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, xml);
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("compact-skip", "compact-skip.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case DirBeforeId(string dir)
    {
        string path = Path.Combine(dir, "dir-before-id.oar");
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, SwappedControl());
            tar.WriteFile("regions/1_1_Swapped/notes.txt", "swapped");
            tar.WriteFile("regions/2_1_Kept/notes.txt", "kept");
        });
        Placed root = Place(LoadRootSwap, "Swap Root", 7, 8, 256);
        Placed kept = Place(LoadKept, "Kept", 8, 8, 256);
        return Run("dir-before-id", "dir-before-id.oar", "gzip-path", path, root, new List<Placed> { root, kept });
    }

    static Case BadFields(string dir)
    {
        string path = Path.Combine(dir, "bad-fields.oar");
        string xml = SingleControl(HomeSave())
            .Replace("<datetime>" + Stamp + "</datetime>", "<datetime>nope</datetime>", StringComparison.Ordinal)
            .Replace("<size_in_meters>256,256</size_in_meters>", "<size_in_meters>nope</size_in_meters>", StringComparison.Ordinal);
        WriteGzip(path, tar =>
        {
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, xml);
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("bad-fields", "bad-fields.oar", "gzip-path", path, HomeRoot(), HomeScenes());
    }

    static Case StreamSeek(string dir)
    {
        string path = Path.Combine(dir, "late.tar");
        WriteTar(path, tar =>
        {
            tar.WriteFile("early.txt", "early");
            tar.WriteFile(ArchiveConstants.CONTROL_FILE_PATH, SingleControl(HomeSave()));
            tar.WriteFile("notes/after.txt", "after");
        });
        return Run("stream-seek", "late.tar", "tar-seek", path, HomeRoot(), HomeScenes());
    }

    static Case StreamNoSeek(string dir)
    {
        string path = Path.Combine(dir, "late.tar");
        return Run("stream-noseek", "late.tar", "tar-noseek", path, HomeRoot(), HomeScenes());
    }

    static Placed HomeSave()
    {
        return Place(SaveHome, "Home", 4, 5, 256);
    }

    static Placed HomeRoot()
    {
        return Place(LoadHome, "Load Home", 4, 5, 256);
    }

    static List<Placed> HomeScenes()
    {
        return new List<Placed> { HomeRoot(), Place(Decoy, "Decoy", 9, 9, 256) };
    }

    static string SingleControl(Placed save)
    {
        var group = new ArchiveScenesGroup();
        group.AddScene(save.Scene);
        group.CalcSceneLocations();
        var request = new ArchiveWriteRequest(save.Scene, new MemoryStream(), Guid.Empty);
        request.MultiRegionFormat = false;
        return StampTime(request.CreateControlFile(group));
    }

    static string MultiControl()
    {
        Placed west = Place(SaveWest, "West", 10, 20, 256);
        Placed east = Place(SaveEast, "East Isle", 12, 20, 512);
        Placed north = Place(SaveNorth, "North", 10, 22, 256);
        var group = new ArchiveScenesGroup();
        group.AddScene(west.Scene);
        group.AddScene(east.Scene);
        group.AddScene(north.Scene);
        group.CalcSceneLocations();
        var request = new ArchiveWriteRequest(west.Scene, new MemoryStream(), Guid.Empty);
        request.MultiRegionFormat = true;
        return StampTime(request.CreateControlFile(group));
    }

    static string SwappedControl()
    {
        using var sw = new StringWriter();
        using (var xtw = new XmlTextWriter(sw))
        {
            xtw.Formatting = Formatting.Indented;
            xtw.WriteStartDocument();
            xtw.WriteStartElement("archive");
            xtw.WriteAttributeString("major_version", "1");
            xtw.WriteAttributeString("minor_version", "0");
            xtw.WriteStartElement("creation_info");
            xtw.WriteElementString("datetime", Stamp);
            xtw.WriteEndElement();
            xtw.WriteElementString("assets_included", "True");
            xtw.WriteStartElement("regions");
            xtw.WriteStartElement("row");
            xtw.WriteStartElement("region");
            xtw.WriteElementString("dir", "1_1_Swapped");
            xtw.WriteElementString("id", "44444444-4444-4444-4444-444444444444");
            xtw.WriteElementString("size_in_meters", "256,256");
            xtw.WriteEndElement();
            xtw.WriteStartElement("region");
            xtw.WriteElementString("id", "55555555-5555-5555-5555-555555555555");
            xtw.WriteElementString("dir", "2_1_Kept");
            xtw.WriteElementString("size_in_meters", "256,256");
            xtw.WriteEndElement();
            xtw.WriteEndElement();
            xtw.WriteEndElement();
            xtw.WriteEndElement();
            xtw.Flush();
        }
        return sw.ToString();
    }

    static string StampTime(string xml)
    {
        return Regex.Replace(xml, "<datetime>\\d+</datetime>", "<datetime>" + Stamp + "</datetime>");
    }

    static string Compact(string xml)
    {
        return Regex.Replace(xml, ">\\s+<", "><");
    }

    static Case Run(string name, string archive, string source, string path, Placed root, List<Placed> registered)
    {
        _ = new SceneManager();
        foreach (Placed scene in registered)
        {
            scene.Scene.RegisterModuleInterface<IAssetService>(new UnusedAssets());
            SceneManager.Instance.Add(scene.Scene);
        }
        root.Scene.RegisterModuleInterface<IAssetService>(new UnusedAssets());

        var probe = Probe(root.Scene, source, path);
        var full = Dearchive(root.Scene, source, path);
        return new Case(name, archive, source, root, registered, probe, full);
    }

    static ProbeResult Probe(Scene root, string source, string path)
    {
        ArchiveReadRequest request = Open(root, source, path);
        var args = new object[] { null, null };
        string error = null;
        try
        {
            FindControl.Invoke(request, args);
        }
        catch (TargetInvocationException ex)
        {
            error = ex.InnerException == null ? ex.Message : ex.InnerException.Message;
        }

        var archive = args[0] as TarArchiveReader;
        var scenes = args[1] as DearchiveScenesInfo;
        if (error != null && scenes == null && request.ControlFileLoaded)
        {
            // MethodInfo.Invoke leaves out arguments unchanged when the method
            // throws. The seek failure is thrown after the control file is
            // loaded and the scenes are matched. A seekable read of the same
            // bytes is that state. Entries stay empty: the walk never starts.
            string againSource = source == "gzip-path" ? source : "tar-seek";
            ArchiveReadRequest again = Open(root, againSource, path);
            var againArgs = new object[] { null, null };
            try
            {
                FindControl.Invoke(again, againArgs);
            }
            catch (TargetInvocationException)
            {
                againArgs[1] = null;
            }
            scenes = againArgs[1] as DearchiveScenesInfo;
            if (againArgs[0] is TarArchiveReader extra)
                extra.Close();
        }
        var entries = new List<EntryRecord>();
        if (error == null && archive != null)
        {
            byte[] data;
            string fullPath;
            TarArchiveReader.TarEntryType entryType;
            while ((data = archive.ReadEntry(out fullPath, out entryType)) != null)
            {
                if (entryType == TarArchiveReader.TarEntryType.TYPE_DIRECTORY)
                {
                    entries.Add(new EntryRecord(fullPath, entryType.ToString(), false, null, fullPath));
                    continue;
                }
                bool use = scenes.GetRegionFromPath(fullPath, out Scene scene, out string relative);
                string sceneId = scene == null ? null : scene.RegionInfo.RegionID.ToString();
                entries.Add(new EntryRecord(fullPath, entryType.ToString(), use, sceneId, relative));
            }
        }
        if (archive != null)
            archive.Close();
        return new ProbeResult(error, request, scenes, entries);
    }

    static DearchiveResult Dearchive(Scene root, string source, string path)
    {
        ArchiveReadRequest request = Open(root, source, path);
        List<string> scenes = null;
        string message = null;
        bool fired = false;
        EventManager.OarFileLoaded handler = (Guid id, List<UUID> loaded, string text) =>
        {
            fired = true;
            scenes = new List<string>();
            if (loaded != null)
            {
                foreach (UUID uuid in loaded)
                    scenes.Add(uuid.ToString());
            }
            message = text;
        };
        root.EventManager.OnOarFileLoaded += handler;
        string thrown = null;
        try
        {
            request.DearchiveRegion(false);
        }
        catch (Exception ex)
        {
            thrown = ex.GetType().Name + ": " + ex.Message;
        }
        finally
        {
            root.EventManager.OnOarFileLoaded -= handler;
        }
        return new DearchiveResult(fired, scenes, message, thrown);
    }

    static ArchiveReadRequest Open(Scene root, string source, string path)
    {
        if (source == "gzip-path")
            return new ArchiveReadRequest(root, path, Guid.Empty, ConsoleOptions(root));
        Stream stream = File.OpenRead(path);
        if (source == "tar-noseek")
            stream = new NoSeekStream(stream);
        return new ArchiveReadRequest(root, stream, Guid.Empty, new Dictionary<string, object>());
    }

    static Dictionary<string, object> ConsoleOptions(Scene scene)
    {
        var options = new Dictionary<string, object>();
        options["displacement"] = Vector3.Zero;
        options["rotation"] = 0f;
        options["rotation-center"] = new Vector3(scene.RegionInfo.RegionSizeX / 2f, scene.RegionInfo.RegionSizeY / 2f, 0f);
        options["bounding-origin"] = new Vector3(0f, 0f, Constants.MinSimulationHeight);
        options["bounding-size"] = new Vector3(
            scene.RegionInfo.RegionSizeX,
            scene.RegionInfo.RegionSizeY,
            Constants.MaxSimulationHeight - Constants.MinSimulationHeight);
        return options;
    }

    static void WriteGzip(string path, Action<TarArchiveWriter> fill)
    {
        using var file = File.Create(path);
        var gzip = new System.IO.Compression.GZipStream(file, System.IO.Compression.CompressionMode.Compress, leaveOpen: true);
        var tar = new TarArchiveWriter(gzip);
        fill(tar);
        tar.Close();
    }

    static void WriteTar(string path, Action<TarArchiveWriter> fill)
    {
        using var file = File.Create(path);
        var tar = new TarArchiveWriter(file);
        fill(tar);
        tar.Close();
    }

    static Placed Place(string id, string name, uint x, uint y, uint size)
    {
        var info = new OpenSim.Framework.RegionInfo
        {
            RegionID = new UUID(id),
            RegionName = name,
            RegionLocX = x,
            RegionLocY = y,
            RegionSizeX = size,
            RegionSizeY = size
        };
        return new Placed(id, x, y, new Scene(info));
    }

}

internal sealed class Placed
{
    public string Id;
    public uint X;
    public uint Y;
    public Scene Scene;

    public Placed(string id, uint x, uint y, Scene scene)
    {
        Id = id;
        X = x;
        Y = y;
        Scene = scene;
    }
}

internal sealed class EntryRecord
{
    public string Path;
    public string Type;
    public bool Use;
    public string SceneId;
    public string Relative;

    public EntryRecord(string path, string type, bool use, string sceneId, string relative)
    {
        Path = path;
        Type = type;
        Use = use;
        SceneId = sceneId;
        Relative = relative;
    }
}

internal sealed class ProbeResult
{
    public string Error;
    public ArchiveReadRequest Request;
    public DearchiveScenesInfo Scenes;
    public List<EntryRecord> Entries;

    public ProbeResult(string error, ArchiveReadRequest request, DearchiveScenesInfo scenes, List<EntryRecord> entries)
    {
        Error = error;
        Request = request;
        Scenes = scenes;
        Entries = entries;
    }
}

internal sealed class DearchiveResult
{
    public bool Fired;
    public List<string> Scenes;
    public string Message;
    public string Thrown;

    public DearchiveResult(bool fired, List<string> scenes, string message, string thrown)
    {
        Fired = fired;
        Scenes = scenes;
        Message = message;
        Thrown = thrown;
    }
}

internal sealed class Case
{
    public string Name;
    public string Archive;
    public string Source;
    public Placed Root;
    public List<Placed> Scenes;
    public ProbeResult Probe;
    public DearchiveResult Full;

    public Case(string name, string archive, string source, Placed root, List<Placed> scenes, ProbeResult probe, DearchiveResult full)
    {
        Name = name;
        Archive = archive;
        Source = source;
        Root = root;
        Scenes = scenes;
        Probe = probe;
        Full = full;
    }

    public void Write(Utf8JsonWriter writer)
    {
        writer.WriteStartObject();
        writer.WriteString("name", Name);
        writer.WriteString("archive", Archive);
        writer.WriteString("source", Source);
        writer.WriteStartObject("root");
        writer.WriteString("id", Root.Id);
        writer.WriteNumber("loc_x", Root.X);
        writer.WriteNumber("loc_y", Root.Y);
        writer.WriteEndObject();
        writer.WriteStartArray("scenes");
        foreach (Placed scene in Scenes)
        {
            writer.WriteStartObject();
            writer.WriteString("id", scene.Id);
            writer.WriteNumber("loc_x", scene.X);
            writer.WriteNumber("loc_y", scene.Y);
            writer.WriteEndObject();
        }
        writer.WriteEndArray();
        WriteProbe(writer);
        WriteFull(writer);
        writer.WriteEndObject();
    }

    void WriteProbe(Utf8JsonWriter writer)
    {
        if (Probe.Error == null)
            writer.WriteNull("error");
        else
            writer.WriteString("error", Probe.Error);

        DearchiveScenesInfo scenes = Probe.Scenes;
        writer.WriteNumber("creation_time", scenes == null ? 0 : scenes.LoadedCreationDateTime);
        writer.WriteString("default_original_id", scenes == null || scenes.DefaultOriginalID == null ? "" : scenes.DefaultOriginalID);
        writer.WriteBoolean("multi_region", scenes != null && scenes.MultiRegionFormat);
        writer.WriteBoolean("control_loaded", Probe.Request != null && Probe.Request.ControlFileLoaded);
        int[] incoming = IncomingBits();
        writer.WriteStartArray("incoming_size_bits");
        foreach (int bit in incoming)
            writer.WriteNumberValue(bit);
        writer.WriteEndArray();
        writer.WriteNumber("scene_count", scenes == null ? 0 : scenes.GetScenesCount());
        writer.WriteStartArray("loaded_scenes");
        if (scenes != null)
        {
            foreach (UUID id in scenes.GetLoadedScenes())
                writer.WriteStringValue(id.ToString());
        }
        writer.WriteEndArray();
        writer.WriteStartArray("regions");
        WriteRegions(writer, scenes);
        writer.WriteEndArray();
        writer.WriteStartArray("entries");
        foreach (EntryRecord entry in Probe.Entries)
        {
            writer.WriteStartObject();
            writer.WriteString("path", entry.Path);
            writer.WriteString("type", entry.Type);
            writer.WriteBoolean("use", entry.Use);
            if (entry.SceneId == null)
                writer.WriteNull("scene_id");
            else
                writer.WriteString("scene_id", entry.SceneId);
            writer.WriteString("relative", entry.Relative);
            writer.WriteEndObject();
        }
        writer.WriteEndArray();
    }

    int[] IncomingBits()
    {
        return Program.IncomingSizeBits(Probe.Request);
    }

    void WriteRegions(Utf8JsonWriter writer, DearchiveScenesInfo scenes)
    {
        if (scenes == null)
            return;
        FieldInfo field = typeof(DearchiveScenesInfo).GetField("m_directory2region", BindingFlags.Instance | BindingFlags.NonPublic);
        var map = (Dictionary<string, DearchiveScenesInfo.RegionInfo>)field.GetValue(scenes);
        foreach (KeyValuePair<string, DearchiveScenesInfo.RegionInfo> pair in map)
        {
            DearchiveScenesInfo.RegionInfo region = pair.Value;
            writer.WriteStartObject();
            writer.WriteString("directory", pair.Key);
            writer.WriteString("original_id", region.OriginalID ?? "");
            writer.WriteNumber("loc_x", (int)region.Location.X);
            writer.WriteNumber("loc_y", (int)region.Location.Y);
            writer.WriteStartArray("size_bits");
            foreach (int bit in SizeBitsOf(region.RegionSize))
                writer.WriteNumberValue(bit);
            writer.WriteEndArray();
            if (region.Scene == null)
                writer.WriteNull("scene_id");
            else
                writer.WriteString("scene_id", region.Scene.RegionInfo.RegionID.ToString());
            writer.WriteEndObject();
        }
    }

    static int[] SizeBitsOf(Vector3 size)
    {
        return new[]
        {
            BitConverter.SingleToInt32Bits(size.X),
            BitConverter.SingleToInt32Bits(size.Y),
            BitConverter.SingleToInt32Bits(size.Z)
        };
    }

    void WriteFull(Utf8JsonWriter writer)
    {
        writer.WriteBoolean("dearchive_fired", Full.Fired);
        if (!Full.Fired || Full.Scenes == null)
            writer.WriteNull("dearchive_scenes");
        else
        {
            writer.WriteStartArray("dearchive_scenes");
            foreach (string id in Full.Scenes)
                writer.WriteStringValue(id);
            writer.WriteEndArray();
        }
        if (Full.Message == null)
            writer.WriteNull("dearchive_error");
        else
            writer.WriteString("dearchive_error", Full.Message);
        if (Full.Thrown == null)
            writer.WriteNull("dearchive_thrown");
        else
            writer.WriteString("dearchive_thrown", Full.Thrown);
    }
}

internal sealed class NoSeekStream : Stream
{
    readonly Stream inner;

    public NoSeekStream(Stream inner)
    {
        this.inner = inner;
    }

    public override bool CanRead => true;
    public override bool CanSeek => false;
    public override bool CanWrite => false;
    public override long Length => throw new NotSupportedException();
    public override long Position
    {
        get => throw new NotSupportedException();
        set => throw new NotSupportedException();
    }

    public override void Flush()
    {
    }

    public override int Read(byte[] buffer, int offset, int count)
    {
        return inner.Read(buffer, offset, count);
    }

    public override int Read(Span<byte> buffer)
    {
        return inner.Read(buffer);
    }

    public override long Seek(long offset, SeekOrigin origin)
    {
        throw new NotSupportedException();
    }

    public override void SetLength(long value)
    {
        throw new NotSupportedException();
    }

    public override void Write(byte[] buffer, int offset, int count)
    {
        throw new NotSupportedException();
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
            inner.Dispose();
        base.Dispose(disposing);
    }
}

internal sealed class UnusedAssets : IAssetService
{
    public AssetBase Get(string id) => null;
    public AssetBase Get(string id, string foreign, bool store) => null;
    public AssetMetadata GetMetadata(string id) => null;
    public byte[] GetData(string id) => null;
    public AssetBase GetCached(string id) => null;
    public bool Get(string id, object sender, AssetRetrieved handler) => false;
    public void Get(string id, string foreign, bool store, SimpleAssetRetrieved callback)
    {
    }
    public bool[] AssetsExist(string[] ids) => new bool[ids.Length];
    public string Store(AssetBase asset) => string.Empty;
    public bool UpdateContent(string id, byte[] data) => false;
    public bool Delete(string id) => false;
}
