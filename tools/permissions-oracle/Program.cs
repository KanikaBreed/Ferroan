// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Reflection;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.CoreModules.World.Permissions;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Interfaces;
using Mask = OpenSim.Framework.PermissionMask;

namespace Ferroan.PermissionsOracle;

static class Program
{
    static readonly UUID Agent = new("00000000-0000-4000-8000-000000000001");
    static readonly UUID Owner = new("00000000-0000-4000-8000-000000000002");
    static readonly UUID Group = new("00000000-0000-4000-8000-000000000003");
    static readonly UUID LandOwner = new("00000000-0000-4000-8000-000000000004");
    static readonly Vector3 Position = new(32, 40, 22);

    record Case(string Name, bool OwnsObject = false, bool Deeded = false, bool Member = false,
        GroupPowers Powers = 0, Mask OwnerMask = Mask.AllEffective, Mask GroupMask = 0,
        Mask EveryoneMask = 0, bool Attachment = false, bool Manager = false, bool ManagerIsGod = false,
        int UserLevel = 0, bool GodMode = false, bool OwnsLand = false, bool GroupLand = false,
        ParcelFlags LandFlags = 0, bool HasLand = true, bool Bypass = false);

    static readonly Case[] Cases = [
        new("owner", OwnsObject: true),
        new("owner-locked", OwnsObject: true, OwnerMask: Mask.Copy | Mask.Transfer),
        new("other"), new("other-everyone-move", EveryoneMask: Mask.Move),
        new("other-everyone-copy-transfer", EveryoneMask: Mask.Copy | Mask.Transfer),
        new("other-copy-no-transfer", OwnerMask: Mask.Move | Mask.Copy, EveryoneMask: Mask.Copy),
        new("group-shared-member", Member: true, GroupMask: Mask.Modify | Mask.Move | Mask.Copy),
        new("group-shared-nonmember", GroupMask: Mask.Modify | Mask.Move | Mask.Copy),
        new("group-owned-member", Deeded: true, Member: true),
        new("group-owned-manipulate", Deeded: true, Member: true, Powers: GroupPowers.ObjectManipulate),
        new("estate-manager", Manager: true),
        new("estate-manager-as-admin", Manager: true, ManagerIsGod: true),
        new("grid-god-mode-off", UserLevel: 200),
        new("grid-god-mode-on", UserLevel: 200, GodMode: true),
        new("grid-god-locked-owner", UserLevel: 200, GodMode: true, OwnsObject: true, OwnerMask: Mask.Copy | Mask.Transfer),
        new("attachment-owner", OwnsObject: true, Attachment: true),
        new("attachment-other", Attachment: true, EveryoneMask: Mask.AllEffective),
        new("parcel-owned", OwnsLand: true),
        new("parcel-public-build", LandFlags: ParcelFlags.CreateObjects),
        new("parcel-group-build-member", Member: true, LandFlags: ParcelFlags.CreateGroupObjects),
        new("parcel-group-build-nonmember", LandFlags: ParcelFlags.CreateGroupObjects),
        new("parcel-deeded-member", Member: true, GroupLand: true),
        new("parcel-deeded-allow-rez", Member: true, GroupLand: true, Powers: GroupPowers.AllowRez),
        new("copy-and-duplicate", EveryoneMask: Mask.Copy | Mask.Transfer, LandFlags: ParcelFlags.CreateObjects),
        new("no-parcel", OwnsObject: true, HasLand: false),
        new("permissions-bypassed", Bypass: true),
    ];

    static int Main(string[] args)
    {
        if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision"
            || args[3].Length != 40 || !args[3].All(Uri.IsHexDigit))
        {
            Console.Error.WriteLine("usage: PermissionsOracle generate|verify <directory> --revision <40 hex>");
            return 2;
        }
        string path = Path.GetFullPath(Path.Combine(args[1], "objects.json"));
        Directory.SetCurrentDirectory(AppContext.BaseDirectory); // ScenePresenceAnimator loads data/avataranimations.xml.
        var rows = Cases.Select(input => new { input, expected = Observe(input) }).ToArray();
        var options = new JsonSerializerOptions { WriteIndented = true, PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
        string json = JsonSerializer.Serialize(new { revision = args[3], cases = rows }, options).ReplaceLineEndings("\n") + "\n";
        if (args[0] == "generate")
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, json);
        }
        else if (!File.Exists(path) || File.ReadAllText(path).ReplaceLineEndings("\n") != json)
        {
            Console.Error.WriteLine("permissions: fixture differs from pinned observations");
            return 1;
        }
        Console.WriteLine($"permissions: {rows.Length} cases {args[0]} passed");
        return 0;
    }

    // The lightweight scene constructor provides a real graph and permission event dispatcher.
    sealed class FixtureScene(RegionInfo info, IConfigSource config) : Scene(info)
    {
        protected override IConfigSource GetConfig() => config;
    }

    static object Observe(Case c)
    {
        var config = new IniConfigSource();
        var ini = config.AddConfig("Startup");
        ini.Set("permissionmodules", "DefaultPermissionsModule");
        ini.Set("serverside_object_permissions", !c.Bypass);
        ini.Set("allow_grid_gods", true);
        ini.Set("region_manager_is_god", c.ManagerIsGod);
        ini.Set("automatic_gods", c.GodMode);

        var info = new RegionInfo { RegionName = "Fixture" };
        info.EstateSettings.EstateOwner = LandOwner;
        if (c.Manager) info.EstateSettings.AddEstateManager(Agent);
        var scene = new FixtureScene(info, config);
        scene.RegisterModuleInterface<IUserAccountService>(InputProxy.For<IUserAccountService>((m, a) =>
            m == "GetUserAccount" && (UUID)a[1] == Agent ? new UserAccount { UserLevel = c.UserLevel } : throw Unexpected(m)));
        scene.RegisterModuleInterface<IGroupsModule>(InputProxy.For<IGroupsModule>((m, a) =>
            m == "GetMembershipData" ? ((UUID)a[0] == Group && (UUID)a[1] == Agent && c.Member
                ? new GroupMembershipData { GroupPowers = (ulong)c.Powers } : null) : throw Unexpected(m)));
        var land = new LandData { OwnerID = c.OwnsLand ? Agent : c.GroupLand ? Group : LandOwner,
            GroupID = Group, IsGroupOwned = c.GroupLand, Flags = (uint)c.LandFlags };
        var parcel = InputProxy.For<ILandObject>((m, _) => m == "get_LandData" ? land : throw Unexpected(m));
        scene.LandChannel = InputProxy.For<ILandChannel>((m, _) => m is "GetLandObject" or "GetLandObjectClippedXY" ? (c.HasLand ? parcel : null) : throw Unexpected(m));

        // Record the caller's input before the real handler runs. Returning it leaves every bit intact.
        uint? flagInput = null;
        scene.Permissions.OnGenerateClientFlags += (_, _, flags) => { flagInput = flags; return flags; };
        var module = new DefaultPermissionsModule();
        module.Initialise(config);
        module.AddRegion(scene);
        if (scene.RequestModuleInterface<IPermissionsModule>() != module)
            throw new InvalidOperationException("DefaultPermissionsModule did not register");

        ScenePresence presence = null;
        var client = InputProxy.For<IClientAPI>((m, a) => m switch
        {
            "get_AgentId" => Agent, "get_SceneAgent" => presence,
            "IsGroupMember" => (UUID)a[0] == Group && c.Member,
            "GetGroupPowers" => (UUID)a[0] == Group && c.Member ? (ulong)c.Powers : 0ul,
            "get_FirstName" => "Fixture", "get_LastName" => "User",
            "get_StartPos" => Position, "get_StartFar" => 128f,
            "RefreshGroupMembership" => null,
            _ when m.StartsWith("add_", StringComparison.Ordinal) => null,
            _ => throw Unexpected(m)
        });
        // The graph's normal presence registration is protected internal in the pin.
        var add = typeof(SceneGraph).GetMethod("CreateAndAddChildScenePresence", BindingFlags.Instance | BindingFlags.NonPublic);
        presence = (ScenePresence)add.Invoke(scene.SceneGraph, [client, new AvatarAppearance(), PresenceType.User]);

        var group = new SceneObjectGroup(c.OwnsObject ? Agent : c.Deeded ? Group : Owner, Position, PrimitiveBaseShape.CreateBox());
        var part = group.RootPart;
        part.GroupID = Group;
        part.BaseMask = (uint)Mask.AllEffective;
        part.OwnerMask = (uint)c.OwnerMask;
        part.GroupMask = (uint)c.GroupMask;
        part.EveryoneMask = (uint)c.EveryoneMask;
        part.Flags = PrimFlags.Phantom;
        group.InvalidateEffectivePerms();
        if (!scene.AddNewSceneObject(group, false, false))
            throw new InvalidOperationException("fixture object did not enter the graph");
        group.IsAttachment = c.Attachment;

        var p = scene.Permissions;
        uint viewerFlags = p.GenerateClientFlags(part, presence);
        uint initialFlags = flagInput ?? throw new InvalidOperationException("flag handler was not called");
        return new {
            clientFlagInput = initialFlags,
            moduleClientFlags = module.GenerateClientFlags(part, presence, initialFlags),
            generateClientFlags = viewerFlags,
            canRez = p.CanRezObject(1, Agent, Position),
            canEdit = p.CanEditObject(group, client), canMove = p.CanMoveObject(group, client),
            canDelete = p.CanDeleteObject(group, client), canTake = p.CanTakeObject(group, presence),
            canLink = p.CanLinkObject(Agent, part.UUID), canTakeCopy = p.CanTakeCopyObject(group, presence),
            canDuplicate = p.CanDuplicateObject(group, Agent)
        };
    }

    static Exception Unexpected(string method) => new InvalidOperationException("unexpected fixture input: " + method);
}

// Only service and viewer inputs are supplied. Unknown calls fail generation.
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
