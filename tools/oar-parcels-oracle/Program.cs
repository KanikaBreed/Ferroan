// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Reflection;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Framework.Serialization.External;
using OpenSim.Region.CoreModules.World.Archiver;
using OpenSim.Region.CoreModules.World.Land;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Interfaces;

namespace Ferroan.OarParcelsOracle;

static class Program
{
    static UUID Id(string tail) => new("00000000-0000-4000-8000-" + tail.PadLeft(12, '0'));

    static readonly UUID Scope = Id("5c0be");
    static readonly UUID EstateOwner = Id("e1");
    static readonly UUID Local1 = Id("a1");
    static readonly UUID Local2 = Id("a2");
    static readonly UUID Stranger1 = Id("b1");
    static readonly UUID Stranger2 = Id("b2");
    static readonly UUID Group1 = Id("f1");
    static readonly UUID UnknownGroup = Id("f2");

    // Accounts the user account service holds in Scope, and the group records the groups module returns.
    static readonly UUID[] Users = [EstateOwner, Local1, Local2];
    static readonly UUID[] Groups = [Group1];

    // A bitmap as ConvertLandBitmapToBytes writes it: rows from the south, LSB first.
    static byte[] Bits(int side, Func<int, int, bool> on)
    {
        var bytes = new byte[side * side / 8];
        for (int y = 0; y < side; y++)
            for (int x = 0; x < side; x++)
                if (on(x, y))
                {
                    int n = y * side + x;
                    bytes[n / 8] |= (byte)(1 << (n % 8));
                }
        return bytes;
    }

    static LandData Land(string tail, string name, UUID owner, UUID group, bool groupOwned, byte[] bitmap,
        params (UUID Agent, AccessList Flags)[] access)
    {
        var land = new LandData
        {
            GlobalID = Id(tail), LocalID = Convert.ToInt32(tail[^1..], 16), Name = name, OwnerID = owner,
            GroupID = group, IsGroupOwned = groupOwned, Bitmap = bitmap, Area = 16 * bitmap.Sum(b => BitCount(b)),
        };
        foreach (var (agent, flags) in access)
            land.ParcelAccessList.Add(new LandAccessEntry { AgentID = agent, Flags = flags, Expires = 1234 });
        return land;
    }

    static int BitCount(byte b) => System.Numerics.BitOperations.PopCount(b);

    static string Xml(LandData land) => LandDataSerializer.Serialize(land, new Dictionary<string, object>());

    const string UnreadableEnvironment =
        "<llsd><map><key>env_version</key><integer>3</integer><key>day_cycle</key><map><key>frames</key>"
        + "<map><key>bad</key><integer>7</integer></map></map></map></llsd>";

    const string EnvironmentText = "<llsd><map><key>env_version</key><integer>42</integer></map></llsd>";

    // The parcel files. Each is the text of one landdata/ entry.
    static Dictionary<string, string> Files()
    {
        var local = Land("1a", "West & field", Local1, Group1, false, Bits(64, (x, _) => x < 32),
            (Local2, AccessList.Access), (Stranger1, AccessList.Ban), (UUID.Zero, AccessList.Access));
        local.Description = "first <parcel>";
        local.AuctionID = 7;
        local.AuthBuyerID = Local2;
        local.Category = (ParcelCategory)3;
        local.ClaimDate = 1700000000;
        local.ClaimPrice = 10;
        local.Flags = 0x4000_0001;
        local.LandingType = 1;
        local.MediaAutoScale = 1;
        local.MediaID = Id("3a");
        local.MediaURL = "http://media.example/a";
        local.MusicURL = "http://music.example/a";
        local.MediaType = "text/html";
        local.MediaDescription = "media";
        local.MediaWidth = 640;
        local.MediaHeight = 480;
        local.MediaLoop = true;
        local.ObscureMedia = true;
        local.PassHours = 1.5f;
        local.PassPrice = 25;
        local.SalePrice = 100;
        local.SnapshotID = Id("3b");
        local.UserLocation = new Vector3(10.5f, 20.25f, 30f);
        local.UserLookAt = new Vector3(1f, 0f, 0f);
        local.OtherCleanTime = 5;
        local.SeeAVs = false;
        local.AnyAVSounds = false;
        local.Status = (ParcelStatus)1;
        string localXml = Xml(local);

        var withEnv = Land("19", "Sky", Local2, UUID.Zero, false, Bits(64, (_, y) => y == 20));
        withEnv.Environment = ViewerEnvironment.FromOSDString(EnvironmentText);
        withEnv.EnvironmentVersion = withEnv.Environment.version;

        // Pretty printed by hand, with values the serializer never writes.
        string bitmapG = Convert.ToBase64String(Bits(64, (_, y) => y >= 10 && y < 12));
        string wrapped = string.Join("\n    ", Enumerable.Range(0, (bitmapG.Length + 63) / 64)
            .Select(i => bitmapG.Substring(i * 64, Math.Min(64, bitmapG.Length - i * 64))));
        string hand = $"""
            <?xml version="1.0" encoding="utf-8"?>
            <!-- written by hand -->
            <LandData>
              <Area>abc</Area>
              <Unknown><Nested>1</Nested></Unknown>
              <Dwell>5</Dwell>
              <Name>Hand &amp; made</Name>
              <OwnerID>   {Local2.ToString().Replace("-", "")}</OwnerID>
              <GroupID>{Group1.ToString().ToUpperInvariant()}</GroupID>
              <IsGroupOwned>  FALSE </IsGroupOwned>
              <SnapshotID>not-a-uuid</SnapshotID>
              <Category>-1</Category>
              <Status> 1 </Status>
              <Flags>-0</Flags>
              <LocalID>  42  </LocalID>
              <LandingType>300</LandingType>
              <MediaAutoScale>+1</MediaAutoScale>
              <MediaW>0x10</MediaW>
              <MediaH>7</MediaH>
              <PassHours>1,234.5</PassHours>
              <UserLocation>&lt;1, 2, 3&gt;</UserLocation>
              <UserLookAt>garbage</UserLookAt>
              <SeeAVs>true</SeeAVs>
              <AnyAVSnds>1</AnyAVSnds>
              <Bitmap>
                {wrapped}
              </Bitmap>
              <ParcelAccessList>
                <ParcelAccessEntry>
                  <AgentID>{Local1}</AgentID>
                  <Time>123</Time>
                  <AccessList>1</AccessList>
                </ParcelAccessEntry>
                <ParcelAccessEntry />
                <ParcelAccessEntry></ParcelAccessEntry>
                <ParcelAccessEntry>
                  <AccessList>x</AccessList>
                  <AgentID>{Local2}</AgentID>
                </ParcelAccessEntry>
              </ParcelAccessList>
              <Environment>not llsd</Environment>
              <OtherCleanTime>9</OtherCleanTime>
            </LandData>
            """;

        string tenBad = string.Concat(Enumerable.Repeat("<Area>x</Area>", 10));
        string strangerEntry = $"<ParcelAccessEntry><AgentID>{Stranger2}</AgentID><Time>0</Time><AccessList>x</AccessList></ParcelAccessEntry>";
        string nameElement = "<Name>West &amp; field</Name>";
        if (!localXml.Contains(nameElement) || !localXml.Contains("<ParcelAccessList>"))
            throw new InvalidOperationException("unexpected serializer output");

        return new Dictionary<string, string>
        {
            ["local"] = localXml,
            ["stranger"] = Xml(Land("1b", "South east", Stranger1, UnknownGroup, false, Bits(64, (x, y) => x >= 32 && y < 32))),
            ["group-owned"] = Xml(Land("1c", "North east", Stranger2, Group1, true, Bits(64, (x, y) => x >= 32 && y >= 32))),
            ["unknown-group-owned"] = Xml(Land("1d", "Corner", Local1, UnknownGroup, true, Bits(64, (x, y) => x == 63 && y == 63))),
            ["zero"] = Xml(Land("1e", "Nobody", UUID.Zero, UUID.Zero, false, Bits(64, (x, y) => x == 0 && y == 63))),
            ["no-land"] = Xml(Land("1f", "Empty", Local1, UUID.Zero, false, new byte[512])),
            ["hand"] = hand,
            ["environment"] = Xml(withEnv),
            ["legacy-512"] = Xml(Land("2a", "From a 512", Local1, UUID.Zero, false, Bits(128, (x, y) => x >= 60 && x < 70 && y < 4))),
            ["tiny"] = Xml(Land("2b", "Tiny", Local1, UUID.Zero, false, [0x01])),
            ["past-the-square"] = Xml(Land("2c", "Past the square", Local1, UUID.Zero, false, [0x00, 0x00, 0xff])),
            ["not-landdata"] = "<Land><Name>x</Name></Land>",
            ["ten-errors"] = localXml.Replace("<LandData>", "<LandData>" + tenBad),
            ["ten-errors-at-end"] = localXml.Replace("</LandData>", tenBad + "</LandData>"),
            // TB203: Pin8 loses its place after these.
            ["bad-last-value"] = localXml.Replace("</LandData>", "<Area>x</Area></LandData>"),
            ["bad-last-in-entry"] = localXml.Replace("<ParcelAccessList>", "<ParcelAccessList>" + strangerEntry),
            ["nested-value"] = localXml.Replace(nameElement, nameElement + "<Name><b>x</b></Name>"),
            // TB203 as real archives meet it: the serializer writes Environment last, and this day cycle is one
            // Pin8's FromOSDString cannot read (a frame that is not a map).
            ["bad-environment"] = localXml.Replace("</LandData>",
                "<Environment>" + System.Security.SecurityElement.Escape(UnreadableEnvironment) + "</Environment></LandData>"),
        };
    }

    // The global ids the files carry. A file without one gets a random id.
    static readonly Dictionary<string, UUID> FileIds = new()
    {
        ["local"] = Id("1a"), ["stranger"] = Id("1b"), ["group-owned"] = Id("1c"),
        ["unknown-group-owned"] = Id("1d"), ["zero"] = Id("1e"), ["no-land"] = Id("1f"),
        ["environment"] = Id("19"), ["legacy-512"] = Id("2a"), ["tiny"] = Id("2b"), ["past-the-square"] = Id("2c"),
        ["ten-errors"] = Id("1a"), ["ten-errors-at-end"] = Id("1a"), ["bad-last-value"] = Id("1a"),
        ["bad-last-in-entry"] = Id("1a"), ["nested-value"] = Id("1a"), ["bad-environment"] = Id("1a"),
    };

    record Existing(string Tail, string Name, string Corner);

    record Case(string Name, string[] Files, uint RegionSize = 256, float[] Displacement = null, float Rotation = 0f,
        float[] BoundingOrigin = null, float[] BoundingSize = null, bool Merge = false, bool MergeParcels = false,
        bool GroupsModule = true, bool ZeroEstateOwner = false, Existing[] Existing = null);

    // Parcels the region already has, for the merge cases.
    static readonly Existing[] RegionParcels = [
        new("4a", "South strip", "south"), new("4b", "West column", "west"), new("4c", "North east cell", "corner"),
    ];

    static byte[] ExistingBits(string corner) => corner switch
    {
        "south" => Bits(64, (_, y) => y < 16),
        "west" => Bits(64, (x, _) => x < 8),
        _ => Bits(64, (x, y) => x >= 60 && y >= 60),
    };

    static readonly Case[] Cases = [
        new("default", ["local", "stranger", "group-owned", "unknown-group-owned", "zero", "no-land", "hand",
            "environment", "legacy-512", "tiny", "past-the-square"]),
        new("displacement", ["local", "stranger", "group-owned"], Displacement: [128f, 64f, 0f]),
        new("negative-displacement", ["local", "group-owned"], Displacement: [-130f, -6f, 0f]),
        new("rotation-90", ["local", "stranger", "group-owned", "hand"], Rotation: 90f),
        new("rotation-180", ["local", "stranger"], Rotation: 180f),
        new("rotation-minus-90", ["local", "stranger"], Rotation: -90f),
        new("rotation-45", ["local", "group-owned"], Rotation: 45f),
        new("rotation-minus-30", ["stranger", "legacy-512"], Rotation: -30f),
        new("bounding", ["local", "stranger", "group-owned"], BoundingOrigin: [64f, 64f, 0f], BoundingSize: [128f, 1000f, 100f]),
        new("bounding-displaced", ["local", "stranger"], BoundingOrigin: [100f, 0f, 0f], BoundingSize: [40f, 0f, 0f],
            Displacement: [10f, 10f, 0f]),
        new("region-512", ["local", "legacy-512"], RegionSize: 512),
        new("merge", ["local", "group-owned"], Merge: true, MergeParcels: true, Existing: RegionParcels),
        new("merge-no-land", ["no-land"], Merge: true, MergeParcels: true, Existing: RegionParcels),
        new("no-groups-module", ["local", "group-owned", "unknown-group-owned"], GroupsModule: false),
        new("zero-estate-owner", ["stranger", "zero"], ZeroEstateOwner: true),
        new("no-parcels", []),
        new("no-land", ["no-land"]),
        new("not-landdata", ["local", "not-landdata"]),
        new("ten-errors", ["ten-errors"]),
        new("ten-errors-at-end", ["ten-errors-at-end"]),
        new("bad-last-value", ["bad-last-value"]),
        new("bad-last-in-entry", ["bad-last-in-entry"]),
        new("nested-value", ["nested-value"]),
        new("bad-environment", ["bad-environment"]),
    ];

    sealed class FixtureScene(RegionInfo info) : Scene(info)
    {
        protected override IConfigSource GetConfig() => new IniConfigSource();
    }

    static object Record(LandData land, HashSet<UUID> knownIds) => new
    {
        globalId = knownIds.Contains(land.GlobalID) ? land.GlobalID.ToString() : "random",
        localId = land.LocalID,
        name = land.Name,
        description = land.Description,
        ownerId = land.OwnerID.ToString(),
        groupId = land.GroupID.ToString(),
        isGroupOwned = land.IsGroupOwned,
        area = land.Area,
        auctionId = land.AuctionID,
        authBuyerId = land.AuthBuyerID.ToString(),
        category = (int)(sbyte)land.Category,
        claimDate = land.ClaimDate,
        claimPrice = land.ClaimPrice,
        salePrice = land.SalePrice,
        status = (int)(sbyte)land.Status,
        flags = land.Flags,
        landingType = land.LandingType,
        mediaAutoScale = land.MediaAutoScale,
        mediaId = land.MediaID.ToString(),
        mediaUrl = land.MediaURL,
        musicUrl = land.MusicURL,
        passHours = land.PassHours,
        passPrice = land.PassPrice,
        snapshotId = land.SnapshotID.ToString(),
        dwell = land.Dwell,
        mediaType = land.MediaType,
        mediaDescription = land.MediaDescription,
        mediaWidth = land.MediaWidth,
        mediaHeight = land.MediaHeight,
        mediaLoop = land.MediaLoop,
        obscureMedia = land.ObscureMedia,
        obscureMusic = land.ObscureMusic,
        seeAvs = land.SeeAVs,
        anyAvSounds = land.AnyAVSounds,
        groupAvSounds = land.GroupAVSounds,
        userLocation = new[] { land.UserLocation.X, land.UserLocation.Y, land.UserLocation.Z },
        userLookAt = new[] { land.UserLookAt.X, land.UserLookAt.Y, land.UserLookAt.Z },
        otherCleanTime = land.OtherCleanTime,
        simwideArea = land.SimwideArea,
        environment = land.Environment is not null,
        environmentVersion = land.EnvironmentVersion,
        bitmap = Convert.ToBase64String(land.Bitmap),
        access = land.ParcelAccessList.Select(entry => new
        {
            agentId = entry.AgentID.ToString(),
            flags = (uint)entry.Flags,
            expires = entry.Expires,
        }).ToArray(),
    };

    static object Observe(Case c, Dictionary<string, string> files, string emptyArchive)
    {
        var calls = new List<string>();
        var info = new RegionInfo { RegionName = "Fixture", ScopeID = Scope, RegionSizeX = c.RegionSize, RegionSizeY = c.RegionSize };
        info.EstateSettings.EstateOwner = c.ZeroEstateOwner ? UUID.Zero : EstateOwner;
        var scene = new FixtureScene(info);
        scene.RegisterModuleInterface<IAssetService>(InputProxy.For<IAssetService>((m, _) => throw Unexpected(m)));
        scene.RegisterModuleInterface<IUserAccountService>(InputProxy.For<IUserAccountService>((m, a) =>
        {
            if (m != "GetUserAccount" || a.Length != 2 || a[1] is not UUID id) throw Unexpected(m);
            if ((UUID)a[0] != Scope) throw new InvalidOperationException($"GetUserAccount scope {a[0]}");
            calls.Add($"GetUserAccount {id}");
            return Users.Contains(id) ? new UserAccount { PrincipalID = id, ScopeID = Scope } : null;
        }));
        if (c.GroupsModule)
            scene.RegisterModuleInterface<IGroupsModule>(InputProxy.For<IGroupsModule>((m, a) =>
            {
                if (m != "GetGroupRecord" || a[0] is not UUID id) throw Unexpected(m);
                calls.Add($"GetGroupRecord {id}");
                return Groups.Contains(id) ? new GroupRecord { GroupID = id, GroupName = "group" } : null;
            }));

        var existing = (c.Existing ?? []).Select(e =>
        {
            var land = new LandData { GlobalID = Id(e.Tail), LocalID = Convert.ToInt32(e.Tail[^1..], 16), Name = e.Name,
                OwnerID = Local1, Bitmap = ExistingBits(e.Corner) };
            return (ILandObject)new LandObject(land, scene);
        }).ToList();
        var cleared = new List<bool>();
        scene.LandChannel = InputProxy.For<ILandChannel>((m, a) => m switch
        {
            "AllParcels" => new List<ILandObject>(existing),
            "Clear" => Do(() => cleared.Add((bool)a[0])),
            _ => throw Unexpected(m),
        });
        List<LandData> stored = null;
        scene.EventManager.OnIncomingLandDataFromStorage += data =>
        {
            if (stored is not null) throw new InvalidOperationException("second TriggerIncomingLandDataFromStorage");
            stored = new List<LandData>(data);
        };

        float[] displacement = c.Displacement ?? [0f, 0f, 0f];
        float[] origin = c.BoundingOrigin ?? [0f, 0f, Constants.MinSimulationHeight];
        float[] size = c.BoundingSize ?? [c.RegionSize, c.RegionSize, Constants.MaxSimulationHeight - Constants.MinSimulationHeight];
        // The console path always passes these four (ArchiverModule.cs 249-256).
        var options = new Dictionary<string, object>
        {
            ["displacement"] = new Vector3(displacement[0], displacement[1], displacement[2]),
            ["rotation"] = c.Rotation,
            ["rotation-center"] = new Vector3(c.RegionSize / 2f, c.RegionSize / 2f, 0f),
            ["bounding-origin"] = new Vector3(origin[0], origin[1], origin[2]),
            ["bounding-size"] = new Vector3(size[0], size[1], size[2]),
        };
        if (c.Merge) options["merge"] = null;
        if (c.MergeParcels) options["merge-parcels"] = null;
        var request = new ArchiveReadRequest(scene, emptyArchive, Guid.Empty, options);
        var load = typeof(ArchiveReadRequest).GetMethod("LoadParcels", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("LoadParcels not found");

        string error = null;
        try
        {
            load.Invoke(request, [scene, c.Files.Select(name => files[name]).ToList()]);
        }
        catch (TargetInvocationException e) when (e.InnerException is not null)
        {
            error = e.InnerException.GetType().Name;
        }

        var known = new HashSet<UUID>(FileIds.Values.Concat(RegionParcels.Select(e => Id(e.Tail))));
        return new
        {
            error,
            clear = cleared.Select(setup => new { setupDefaultParcel = setup }).ToArray(),
            parcels = stored?.Select(land => Record(land, known)).ToArray(),
            kept = stored is null ? 0 : stored.Count(land => existing.Any(e => ReferenceEquals(e.LandData, land))),
            calls,
        };
    }

    static object Do(Action action)
    {
        action();
        return null;
    }

    static Exception Unexpected(string method) => new InvalidOperationException($"unexpected service call {method}");

    static int Main(string[] args)
    {
        if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision"
            || args[3].Length != 40 || !args[3].All(Uri.IsHexDigit))
        {
            Console.Error.WriteLine("usage: OarParcelsOracle generate|verify <directory> --revision <40 hex>");
            return 2;
        }
        // The region server's culture (Culture.cs).
        Culture.SetCurrentCulture();
        Culture.SetDefaultCurrentCulture();
        string path = Path.GetFullPath(Path.Combine(args[1], "parcels.json"));
        string emptyArchive = Path.Combine(Path.GetTempPath(), $"ferroan-oar-parcels-{Environment.ProcessId}.oar");
        File.WriteAllBytes(emptyArchive, []);
        string json;
        int count;
        try
        {
            var files = Files();
            var rows = Cases.Select(input => new { input, expected = Observe(input, files, emptyArchive) }).ToArray();
            count = rows.Length;
            var fixture = new
            {
                revision = args[3],
                scope = Scope.ToString(),
                estateOwner = EstateOwner.ToString(),
                users = Users.Select(id => id.ToString()).ToArray(),
                groups = Groups.Select(id => id.ToString()).ToArray(),
                existing = RegionParcels.Select(e => new
                {
                    globalId = Id(e.Tail).ToString(),
                    localId = Convert.ToInt32(e.Tail[^1..], 16),
                    name = e.Name,
                    ownerId = Local1.ToString(),
                    bitmap = Convert.ToBase64String(ExistingBits(e.Corner)),
                }).ToArray(),
                files = files.ToDictionary(kv => kv.Key, kv => kv.Value),
                fileIds = FileIds.ToDictionary(kv => kv.Key, kv => kv.Value.ToString()),
                cases = rows,
            };
            var options = new JsonSerializerOptions { WriteIndented = true, PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
            json = JsonSerializer.Serialize(fixture, options).ReplaceLineEndings("\n") + "\n";
        }
        finally
        {
            File.Delete(emptyArchive);
        }
        if (args[0] == "generate")
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, json);
        }
        else
        {
            if (!File.Exists(path))
            {
                Console.Error.WriteLine($"oar-parcels: missing fixture {path}");
                return 1;
            }
            string stored = File.ReadAllText(path).ReplaceLineEndings("\n");
            if (stored != json)
            {
                Console.Error.WriteLine("oar-parcels: fixture differs from the Pin8 observation");
                return 1;
            }
        }
        Console.WriteLine($"oar-parcels: {count} cases {args[0]} passed");
        return 0;
    }
}

public class InputProxy : DispatchProxy
{
    Func<string, object[], object> read;
    public static T For<T>(Func<string, object[], object> read) where T : class
    {
        T proxy = Create<T, InputProxy>();
        ((InputProxy)(object)proxy).read = read;
        return proxy;
    }
    protected override object Invoke(MethodInfo method, object[] args) => read(method.Name, args);
}
