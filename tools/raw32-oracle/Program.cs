using System.Buffers.Binary;
using System.IO.Compression;
using System.Text;
using OpenMetaverse;
using OpenSim.Region.CoreModules.World.Archiver;
using OpenSim.Region.CoreModules.World.Terrain;
using OpenSim.Region.CoreModules.World.Terrain.FileLoaders;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Tests.Common;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: Raw32Oracle <output-directory>");
    return 2;
}

string outDir = args[0];
Directory.CreateDirectory(outDir);

byte[] member = BuildMember();
byte[] trimIn = BuildTrim();
byte[] sectionIn = BuildSection();
byte[] tinyIn = new byte[4 * 4 * 4];
byte[] taintIn = new byte[4 * 4 * 4];
Write(taintIn, 0, 1f);
byte[] wordIn = new byte[17 * 16 * 4];
Write(wordIn, 16, 1f);
byte[] shortIn = new byte[100];
File.WriteAllBytes(Path.Combine(outDir, "member.r32"), member);
File.WriteAllBytes(Path.Combine(outDir, "trim-in.r32"), trimIn);
File.WriteAllBytes(Path.Combine(outDir, "section-in.r32"), sectionIn);
File.WriteAllBytes(Path.Combine(outDir, "tiny-in.r32"), tinyIn);
File.WriteAllBytes(Path.Combine(outDir, "taint-in.r32"), taintIn);
File.WriteAllBytes(Path.Combine(outDir, "word-in.r32"), wordIn);
File.WriteAllBytes(Path.Combine(outDir, "short.r32"), shortIn);

string oarPath = Path.Combine(outDir, "region.oar");
WriteOar(oarPath, member);

var helpers = new SceneHelpers();
var scene = helpers.SetupScene("Raw32", UUID.Random(), 1000, 1000);
SceneHelpers.SetupSceneModules(scene, new TerrainModule(), new ArchiverModule());

var archiver = (ArchiverModule)scene.RequestModuleInterface<IRegionArchiverModule>();
archiver.HandleLoadOarConsoleCommand("RegionArchiverModule", new[] { "load", "oar", oarPath });

var terrain = (TerrainModule)scene.RequestModuleInterface<ITerrainModule>();
byte[] loaded = Save(terrain, "loaded.r32");
File.WriteAllBytes(Path.Combine(outDir, "loaded.r32"), loaded);
int cell3 = BitConverter.SingleToInt32Bits(scene.Heightmap[3, 0]);
Console.WriteLine($"loaded {loaded.Length} cell7={(double)scene.Heightmap[7, 0]} cell3=0x{cell3:X8}");

var raw = new RAW32();
byte[] streamed;
using (var input = new MemoryStream(member, writable: false))
{
    var channel = raw.LoadStream(input);
    streamed = SaveChannel(raw, channel);
}
File.WriteAllBytes(Path.Combine(outDir, "stream.r32"), streamed);

byte[] trimmed;
using (var input = new MemoryStream(trimIn, writable: false))
{
    var channel = raw.LoadStream(input);
    trimmed = SaveChannel(raw, channel);
}
File.WriteAllBytes(Path.Combine(outDir, "trim-out.r32"), trimmed);
Console.WriteLine($"trim {channelWidth(trimmed)} bytes {trimmed.Length}");

string sectionPath = Path.Combine(outDir, "section-in.r32");
var section = raw.LoadFile(sectionPath, 1, 1, 2, 99, 16, 16);
byte[] sectionOut = SaveChannel(raw, section);
File.WriteAllBytes(Path.Combine(outDir, "section-out.r32"), sectionOut);
Console.WriteLine($"section {sectionOut.Length} cell0=0x{BitConverter.SingleToInt32Bits(section[0, 0]):X8} cell1=0x{BitConverter.SingleToInt32Bits(section[1, 0]):X8}");

var tiny = raw.LoadFile(Path.Combine(outDir, "tiny-in.r32"), 0, 0, 1, 1, 4, 4);
byte[] tinyOut = SaveChannel(raw, tiny);
File.WriteAllBytes(Path.Combine(outDir, "tiny-out.r32"), tinyOut);
Console.WriteLine($"tiny {tinyOut.Length}");

bool wordFailed = false;
try
{
    var word = raw.LoadFile(Path.Combine(outDir, "word-in.r32"), 0, 0, 1, 1, 17, 16);
    byte[] wordOut = SaveChannel(raw, word);
    File.WriteAllBytes(Path.Combine(outDir, "word-out.r32"), wordOut);
    Console.WriteLine($"word {wordOut.Length} cell16={(double)word[16, 0]}");
}
catch (Exception ex)
{
    wordFailed = true;
    File.WriteAllText(Path.Combine(outDir, "word.txt"), ex.GetType().FullName + "\n");
    Console.WriteLine($"word {ex.GetType().FullName}");
}

try
{
    raw.LoadFile(Path.Combine(outDir, "taint-in.r32"), 0, 0, 1, 1, 4, 4);
    Console.WriteLine("taint loaded");
    return 4;
}
catch (Exception ex)
{
    File.WriteAllText(Path.Combine(outDir, "taint.txt"), ex.GetType().FullName + "\n");
    Console.WriteLine($"taint {ex.GetType().FullName}");
}

try
{
    using var input = new MemoryStream(shortIn, writable: false);
    raw.LoadStream(input);
    Console.WriteLine("short loaded");
    return 1;
}
catch (Exception ex)
{
    File.WriteAllText(Path.Combine(outDir, "short.txt"), ex.GetType().FullName + "\n");
    Console.WriteLine($"short {ex.GetType().FullName}");
}

if (wordFailed)
    return 5;

if (!loaded.AsSpan().SequenceEqual(streamed))
{
    int mismatch = FirstMismatch(loaded, streamed);
    Console.WriteLine($"load oar differs from LoadStream at byte {mismatch}");
    return 3;
}

Console.WriteLine("load oar matches LoadStream");
return 0;

static int channelWidth(byte[] saved)
{
    return saved.Length / 4;
}

static int FirstMismatch(byte[] left, byte[] right)
{
    int n = Math.Min(left.Length, right.Length);
    for (int i = 0; i < n; i++)
    {
        if (left[i] != right[i])
            return i;
    }
    return n;
}

static byte[] Save(TerrainModule terrain, string name)
{
    var stream = new KeepStream();
    terrain.SaveToStream(name, stream);
    return stream.ToArray();
}

static byte[] SaveChannel(RAW32 raw, ITerrainChannel channel)
{
    var stream = new KeepStream();
    raw.SaveStream(stream, channel);
    return stream.ToArray();
}

static byte[] BuildMember()
{
    const int edge = 256;
    byte[] raw = new byte[edge * edge * 4 + 7];
    for (int y = 0; y < edge; y++)
    {
        for (int x = 0; x < edge; x++)
        {
            float value;
            if (y == 0 && x == 0)
                value = float.NaN;
            else if (y == 0 && x == 1)
                value = float.PositiveInfinity;
            else if (y == 0 && x == 2)
                value = float.NegativeInfinity;
            else if (y == 0 && x == 3)
                value = BitConverter.Int32BitsToSingle(unchecked((int)0x80000000));
            else if (y == 0 && x == 4)
                value = 1.0001f;
            else if (y == 0 && x == 5)
                value = -12.5f;
            else if (y == 0 && x == 6)
                value = BitConverter.Int32BitsToSingle(1);
            else if (y == 0 && x == 7)
                value = 100f;
            else
                value = ((x * 17 + y * 3) % 10007) * 0.01f;
            Write(raw, y * edge + x, value);
        }
    }
    for (int i = edge * edge * 4; i < raw.Length; i++)
        raw[i] = 0xA5;
    return raw;
}

static byte[] BuildTrim()
{
    const int edge = 300;
    byte[] raw = new byte[edge * edge * 4];
    for (int i = 0; i < edge * edge; i++)
        Write(raw, i, i * 0.001f);
    return raw;
}

static byte[] BuildSection()
{
    // offsetX 1, offsetY 1, fileWidth 2, section 16x16: one Y strip of
    // 16 * (2 * 16) floats, then each row skips 16 and reads 16.
    const int count = 1024;
    byte[] raw = new byte[count * 4];
    for (int i = 0; i < count; i++)
        Write(raw, i, i);
    Write(raw, 528, float.NaN);
    Write(raw, 529, BitConverter.Int32BitsToSingle(unchecked((int)0x80000000)));
    Write(raw, 530, float.PositiveInfinity);
    Write(raw, 531, BitConverter.Int32BitsToSingle(1));
    return raw;
}

static void Write(byte[] raw, int index, float value)
{
    BinaryPrimitives.WriteSingleLittleEndian(raw.AsSpan(index * 4), value);
}

static void WriteOar(string path, byte[] member)
{
    byte[] xml = Encoding.ASCII.GetBytes(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<archive major_version=\"0\" minor_version=\"3\">\n</archive>\n");
    using var tar = new MemoryStream();
    WriteTar(tar, "archive.xml", xml);
    WriteTar(tar, "terrains/Raw32.r32", member);
    tar.Write(new byte[1024]);
    using var file = File.Create(path);
    using var gzip = new GZipStream(file, CompressionLevel.SmallestSize);
    gzip.Write(tar.ToArray());
}

static void WriteTar(Stream tar, string name, byte[] body)
{
    byte[] header = new byte[512];
    byte[] nameBytes = Encoding.ASCII.GetBytes(name);
    if (nameBytes.Length > 100)
        throw new InvalidOperationException(name);
    nameBytes.CopyTo(header, 0);
    PutOctal(header, 100, 8, 0644);
    PutOctal(header, 108, 8, 0);
    PutOctal(header, 116, 8, 0);
    PutOctal(header, 124, 12, body.Length);
    PutOctal(header, 136, 12, 0);
    for (int i = 148; i < 156; i++)
        header[i] = 0x20;
    header[156] = (byte)'0';
    Encoding.ASCII.GetBytes("ustar").CopyTo(header, 257);
    header[262] = 0;
    header[263] = (byte)'0';
    header[264] = (byte)'0';
    int sum = 0;
    foreach (byte b in header)
        sum += b;
    PutOctal(header, 148, 7, sum);
    header[155] = 0x20;
    tar.Write(header);
    tar.Write(body);
    int pad = (512 - (body.Length % 512)) % 512;
    if (pad != 0)
        tar.Write(new byte[pad]);
}

static void PutOctal(byte[] header, int offset, int width, int value)
{
    string text = Convert.ToString(value, 8).PadLeft(width - 1, '0');
    Encoding.ASCII.GetBytes(text).CopyTo(header, offset);
    header[offset + width - 1] = 0;
}

sealed class KeepStream : MemoryStream
{
    protected override void Dispose(bool disposing)
    {
    }
}
