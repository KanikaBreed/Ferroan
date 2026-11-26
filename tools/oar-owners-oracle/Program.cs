// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Reflection;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.CoreModules.World.Archiver;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Region.Framework.Scenes.Serialization;
using OpenSim.Services.Interfaces;

namespace Ferroan.OarOwnersOracle;

static class Program
{
    static UUID Id(string tail) => new("00000000-0000-4000-8000-" + tail.PadLeft(12, '0'));

    static readonly UUID Scope = Id("5c0be");
    static readonly UUID EstateOwner = Id("e1");
    static readonly UUID DefaultUser = Id("d1");
    static readonly UUID Local1 = Id("a1");
    static readonly UUID Local2 = Id("a2");
    static readonly UUID Stranger1 = Id("b1");
    static readonly UUID Stranger2 = Id("b2");
    static readonly UUID Stranger3 = Id("b3");
    static readonly UUID AliasToLocal = Id("c1");
    static readonly UUID AliasToStranger = Id("c2");
    static readonly UUID AliasToGroup = Id("c3");
    static readonly UUID AliasToAlias = Id("c4");
    static readonly UUID Group1 = Id("f1");
    static readonly UUID UnknownGroup = Id("f2");

    // Accounts the user account service holds in Scope.
    static readonly UUID[] Users = [EstateOwner, DefaultUser, Local1, Local2];
    // Group records the groups module returns.
    static readonly UUID[] Groups = [Group1];
    // UserAlias rows: alias id to user id. Local1's row is never consulted, because Local1 is an account.
    static readonly (UUID Alias, UUID User)[] Aliases = [
        (AliasToLocal, Local2), (AliasToStranger, Stranger3), (AliasToGroup, Group1),
        (AliasToAlias, AliasToLocal), (Local1, Local2),
    ];

    const string HomeData = "http://hg.example:8002/;Hyper Traveller";
    const string BareHomeData = "http://hg.example:8002;No Slash";

    record Case(string Name, bool DefaultUserOption = false, bool NoDefaultUser = false,
        bool LookupAliases = false, bool GroupsModule = true, bool AliasService = true,
        bool UserManagement = true, bool ZeroEstateOwner = false);

    static readonly Case[] Cases = [
        new("estate-owner-default"),
        new("default-user", DefaultUserOption: true),
        new("no-default-user", NoDefaultUser: true),
        new("lookup-aliases", LookupAliases: true),
        new("lookup-aliases-no-default-user", LookupAliases: true, NoDefaultUser: true),
        new("lookup-aliases-default-user", LookupAliases: true, DefaultUserOption: true),
        new("no-groups-module", GroupsModule: false),
        new("no-groups-module-lookup-aliases", GroupsModule: false, LookupAliases: true),
        new("lookup-aliases-no-alias-service", LookupAliases: true, AliasService: false),
        new("no-user-management", UserManagement: false),
        new("zero-estate-owner", ZeroEstateOwner: true),
        new("zero-estate-owner-no-default-user", ZeroEstateOwner: true, NoDefaultUser: true),
    ];

    record Spec(UUID Creator, string CreatorData, UUID Owner, UUID LastOwner, UUID Group);

    static SceneObjectPart Part(UUID uuid, string name, Spec spec, Vector3 offset)
    {
        var part = new SceneObjectPart(spec.Owner, PrimitiveBaseShape.CreateBox(), new Vector3(64, 64, 22),
            Quaternion.Identity, offset)
        {
            UUID = uuid, Name = name, CreatorID = spec.Creator, CreatorData = spec.CreatorData ?? string.Empty,
            OwnerID = spec.Owner, LastOwnerID = spec.LastOwner, GroupID = spec.Group,
            // The constructor reads the clock. A fixed date keeps the fixture repeatable.
            CreationDate = 1700000000,
        };
        return part;
    }

    static void Item(SceneObjectPart part, UUID itemId, string name, Spec spec)
    {
        part.TaskInventory.Add(itemId, new TaskInventoryItem
        {
            ItemID = itemId, Name = name, AssetID = Id("ab" + itemId.ToString()[^4..]), Type = 7, InvType = 7,
            CreatorID = spec.Creator, CreatorData = spec.CreatorData ?? string.Empty, OwnerID = spec.Owner,
            LastOwnerID = spec.LastOwner, GroupID = spec.Group, ParentPartID = part.UUID, ParentID = part.UUID,
            CreationDate = 1700000000,
        });
    }

    // The objects every case loads, in order, through one request, so the second shares the first's lookups.
    static string[] ObjectXml()
    {
        var root = Part(Id("1001"), "root", new(Local1, null, AliasToLocal, Stranger1, Group1), Vector3.Zero);
        Item(root, Id("2001"), "hg-creator", new(Stranger1, HomeData, Group1, Stranger2, UnknownGroup));
        Item(root, Id("2002"), "zero-owner", new(AliasToStranger, null, UUID.Zero, UUID.Zero, UUID.Zero));
        var group = new SceneObjectGroup(root);
        var child1 = Part(Id("1002"), "group-creator", new(Group1, null, UnknownGroup, UUID.Zero, UUID.Zero), Vector3.UnitX);
        Item(child1, Id("2003"), "alias-to-group", new(UUID.Zero, null, AliasToGroup, Local1, Group1));
        group.AddPart(child1);
        var child2 = Part(Id("1003"), "alias-chain", new(Stranger2, BareHomeData, AliasToAlias, Local1, UnknownGroup), Vector3.UnitY);
        Item(child2, Id("2004"), "alias-creator", new(AliasToLocal, null, Stranger1, AliasToLocal, Group1));
        group.AddPart(child2);

        var single = Part(Id("1004"), "second-object", new(Stranger3, null, Local1, Stranger1, Group1), Vector3.Zero);
        Item(single, Id("2005"), "estate-owner-item", new(DefaultUser, null, EstateOwner, Stranger3, UnknownGroup));
        var second = new SceneObjectGroup(single);
        return [SceneObjectSerializer.ToXml2Format(group), SceneObjectSerializer.ToXml2Format(second)];
    }

    sealed class FixtureScene(RegionInfo info) : Scene(info)
    {
        protected override IConfigSource GetConfig() => new IniConfigSource();
    }

    static object Observe(Case c, string[] xml, string emptyArchive)
    {
        var calls = new List<string>();
        var info = new RegionInfo { RegionName = "Fixture", ScopeID = Scope };
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
        if (c.AliasService)
            scene.RegisterModuleInterface<IUserAliasService>(InputProxy.For<IUserAliasService>((m, a) =>
            {
                if (m != "GetUserForAlias") throw Unexpected(m);
                var id = (UUID)a[0];
                calls.Add($"GetUserForAlias {id}");
                foreach (var (alias, user) in Aliases)
                    if (alias == id) return new UserAlias(alias, user, "fixture");
                return null;
            }));
        if (c.UserManagement)
            scene.RegisterModuleInterface<IUserManagement>(InputProxy.For<IUserManagement>((m, a) =>
            {
                if (m != "AddCreatorUser") throw Unexpected(m);
                calls.Add($"AddCreatorUser {a[0]} {a[1]}");
                return null;
            }));

        var options = new Dictionary<string, object>();
        if (c.DefaultUserOption) options["default-user"] = DefaultUser;
        if (c.NoDefaultUser) options["no-defaultuser"] = null;
        if (c.LookupAliases) options["lookup-aliases"] = null;
        // The console path: ArchiverModule.DearchiveRegion(string, Guid, options).
        var request = new ArchiveReadRequest(scene, emptyArchive, Guid.Empty, options);
        var modify = typeof(ArchiveReadRequest).GetMethod("ModifySceneObject", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("ModifySceneObject not found");

        var objects = new List<object>();
        foreach (string text in xml)
        {
            var group = SceneObjectSerializer.FromXml2Format(text) ?? throw new InvalidOperationException("fixture XML did not load");
            modify.Invoke(request, [scene, group]);
            objects.Add(new
            {
                parts = group.Parts.Select(part => new
                {
                    uuid = part.UUID.ToString(),
                    creatorId = part.CreatorID.ToString(),
                    creatorData = part.CreatorData,
                    ownerId = part.OwnerID.ToString(),
                    lastOwnerId = part.LastOwnerID.ToString(),
                    groupId = part.GroupID.ToString(),
                    items = part.TaskInventory.Values.Select(item => new
                    {
                        itemId = item.ItemID.ToString(),
                        creatorId = item.CreatorID.ToString(),
                        creatorData = item.CreatorData,
                        ownerId = item.OwnerID.ToString(),
                        lastOwnerId = item.LastOwnerID.ToString(),
                        groupId = item.GroupID.ToString(),
                    }).ToArray(),
                }).ToArray(),
            });
        }
        return new { objects, calls };
    }

    static Exception Unexpected(string method) => new InvalidOperationException($"unexpected service call {method}");

    static int Main(string[] args)
    {
        if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision"
            || args[3].Length != 40 || !args[3].All(Uri.IsHexDigit))
        {
            Console.Error.WriteLine("usage: OarOwnersOracle generate|verify <directory> --revision <40 hex>");
            return 2;
        }
        string path = Path.GetFullPath(Path.Combine(args[1], "objects.json"));
        string emptyArchive = Path.Combine(Path.GetTempPath(), $"ferroan-oar-owners-{Environment.ProcessId}.oar");
        File.WriteAllBytes(emptyArchive, []);
        string json;
        int count;
        try
        {
            string[] xml = ObjectXml();
            var rows = Cases.Select(input => new { input, expected = Observe(input, xml, emptyArchive) }).ToArray();
            count = rows.Length;
            var fixture = new
            {
                revision = args[3],
                scope = Scope.ToString(),
                estateOwner = EstateOwner.ToString(),
                defaultUser = DefaultUser.ToString(),
                users = Users.Select(id => id.ToString()).ToArray(),
                groups = Groups.Select(id => id.ToString()).ToArray(),
                aliases = Aliases.Select(row => new { alias = row.Alias.ToString(), user = row.User.ToString() }).ToArray(),
                objects = xml,
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
                Console.Error.WriteLine($"oar-owners: missing fixture {path}");
                return 1;
            }
            string stored = File.ReadAllText(path).ReplaceLineEndings("\n");
            if (stored != json)
            {
                Console.Error.WriteLine("oar-owners: fixture differs from the Pin8 observation");
                return 1;
            }
        }
        Console.WriteLine($"oar-owners: {count} cases {args[0]} passed");
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
