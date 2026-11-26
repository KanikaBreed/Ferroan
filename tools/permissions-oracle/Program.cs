// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
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

    // These property names are the INI keys written to the fixture.
    record Settings(string permissionmodules = "DefaultPermissionsModule",
        bool serverside_object_permissions = true, bool allow_grid_gods = true,
        bool region_owner_is_god = false, bool region_manager_is_god = false,
        bool force_grid_gods_only = false, bool automatic_gods = false,
        bool implicit_gods = false, bool propagate_permissions = true)
    {
        public IConfigSource Config()
        {
            var config = new IniConfigSource();
            var startup = config.AddConfig("Startup");
            foreach (var setting in JsonSerializer.SerializeToElement(this).EnumerateObject())
                startup.Set(setting.Name, setting.Value.ToString());
            return config;
        }
    }

    record ItemMasks(Mask Current, Mask Group, Mask Everyone);

    record ChildPart(Mask OwnerMask, ItemMasks InventoryItem = null, bool Observe = false);

    record Case(string Name, bool OwnsObject = false, bool Deeded = false, bool Member = false,
        GroupPowers Powers = 0, Mask OwnerMask = Mask.AllEffective, Mask GroupMask = 0,
        Mask EveryoneMask = 0, bool Attachment = false, bool Manager = false,
        int UserLevel = 0, bool OwnsLand = false, bool GroupLand = false,
        ParcelFlags LandFlags = 0, bool HasLand = true, bool EstateOwner = false,
        Mask BaseMask = Mask.AllEffective, PrimFlags Flags = PrimFlags.Phantom,
        ItemMasks InventoryItem = null, bool FriendCanModify = false)
    {
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public bool NilOwner { get; init; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public bool NilGroup { get; init; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public int? CreationDate { get; init; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public ChildPart Child { get; init; }

        [JsonIgnore]
        public Settings Settings { get; init; } = new();
    }

    static readonly Case[] Cases = [
        new("owner", OwnsObject: true),
        new("owner-unlocked", OwnsObject: true, OwnerMask: Mask.Modify | Mask.Copy | Mask.Transfer | Mask.Move),
        new("owner-locked", OwnsObject: true, OwnerMask: Mask.Modify | Mask.Copy | Mask.Transfer),
        new("owner-inventory-no-copy", OwnsObject: true,
            InventoryItem: new(Mask.Modify | Mask.Move | Mask.Transfer, 0, 0)),
        new("other"), new("other-everyone-move", EveryoneMask: Mask.Move),
        new("other-everyone-copy-transfer", EveryoneMask: Mask.Copy | Mask.Transfer),
        new("other-copy-no-transfer", OwnerMask: Mask.Move | Mask.Copy, EveryoneMask: Mask.Copy),
        new("group-shared-member", Member: true, OwnerMask: Mask.Modify | Mask.Copy | Mask.Transfer | Mask.Move,
            GroupMask: Mask.Modify | Mask.Move | Mask.Copy),
        new("group-shared-member-locked", Member: true, OwnerMask: Mask.Modify | Mask.Copy | Mask.Transfer,
            GroupMask: Mask.Modify | Mask.Move | Mask.Copy),
        new("group-shared-no-base-transfer", Member: true, BaseMask: Mask.AllEffective & ~Mask.Transfer,
            GroupMask: Mask.Modify | Mask.Move | Mask.Copy, EveryoneMask: Mask.Copy),
        new("other-everyone-move-locked", OwnerMask: Mask.Modify | Mask.Copy | Mask.Transfer, EveryoneMask: Mask.Move),
        new("group-shared-nonmember", GroupMask: Mask.Modify | Mask.Move | Mask.Copy),
        new("group-owned-member", Deeded: true, Member: true),
        new("group-owned-nonmember", Deeded: true),
        new("friend-with-modify-rights", FriendCanModify: true),
        new("group-owned-manipulate", Deeded: true, Member: true, Powers: GroupPowers.ObjectManipulate),
        new("estate-owner", EstateOwner: true),
        new("estate-owner-as-admin", EstateOwner: true) { Settings = new(region_owner_is_god: true) },
        new("estate-manager", Manager: true),
        new("estate-manager-as-admin", Manager: true) { Settings = new(region_manager_is_god: true) },
        new("estate-manager-god-mode", Manager: true) { Settings = new(region_manager_is_god: true, automatic_gods: true) },
        new("grid-god-mode-off", UserLevel: 200),
        new("grid-god-mode-on", UserLevel: 200) { Settings = new(automatic_gods: true) },
        new("grid-god-implicit", UserLevel: 200) { Settings = new(implicit_gods: true) },
        new("grid-gods-disabled", UserLevel: 200) { Settings = new(allow_grid_gods: false, automatic_gods: true) },
        new("grid-god-locked-owner", UserLevel: 200, OwnsObject: true, OwnerMask: Mask.Copy | Mask.Transfer) { Settings = new(automatic_gods: true) },
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
        new("permissions-bypassed") { Settings = new(serverside_object_permissions: false) },
        new("bypass-restricted-mask", OwnerMask: Mask.Transfer) { Settings = new(serverside_object_permissions: false) },
        new("bypass-other-attachment", Attachment: true) { Settings = new(serverside_object_permissions: false) },
        new("scripted", Flags: PrimFlags.Scripted),
        new("touch", Flags: PrimFlags.Touch),
        new("money", Flags: PrimFlags.Money),
        new("inventory-drop", Flags: PrimFlags.AllowInventoryDrop),
        new("temporary", Flags: PrimFlags.Temporary),
        new("temporary-on-rez", Flags: PrimFlags.TemporaryOnRez),
        new("physics", Flags: PrimFlags.Physics),
        new("friend-on-deeded-object", Deeded: true, FriendCanModify: true),
        new("grid-god-locked-nonowner", UserLevel: 200, OwnerMask: Mask.Copy | Mask.Transfer)
            { Settings = new(automatic_gods: true) },
        new("friend-no-transfer", FriendCanModify: true, OwnerMask: Mask.Copy | Mask.Modify | Mask.Move),
        new("group-manipulate-no-transfer", Deeded: true, Member: true, Powers: GroupPowers.ObjectManipulate,
            OwnerMask: Mask.Copy | Mask.Modify | Mask.Move),
        new("attachment-friend", Attachment: true, FriendCanModify: true),
        new("attachment-group-member", Attachment: true, Member: true, GroupMask: Mask.Copy | Mask.Move),
        new("bypass-restrictive-inventory", InventoryItem: new(Mask.Transfer, 0, 0))
            { Settings = new(serverside_object_permissions: false) },
        new("bypass-child-owner-mask") { Child = new(Mask.Transfer, Observe: true),
            Settings = new(serverside_object_permissions: false) },
        new("nil-group-membership", Member: true, GroupMask: Mask.Copy | Mask.Move) { NilGroup = true },
        new("unowned-object") { NilOwner = true },
        new("child-inventory-modern", EveryoneMask: Mask.Copy, GroupMask: Mask.Copy)
            { CreationDate = 1493574995, Child = new(Mask.AllEffective, new(Mask.AllEffective, 0, 0)) },
        new("child-inventory-legacy", EveryoneMask: Mask.Copy, GroupMask: Mask.Copy)
            { CreationDate = 1493574994, Child = new(Mask.AllEffective, new(Mask.AllEffective, 0, 0)) },
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
        var rows = Cases.Select(input => new { input, settings = input.Settings, expected = Observe(input) }).ToArray();
        var options = new JsonSerializerOptions { WriteIndented = true, PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
        string json = JsonSerializer.Serialize(new { revision = args[3], cases = rows }, options).ReplaceLineEndings("\n") + "\n";
        if (args[0] == "generate")
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, json);
        }
        else
        {
            if (!File.Exists(path))
            {
                Console.Error.WriteLine($"permissions: missing fixture {path}");
                return 1;
            }
            string stored = File.ReadAllText(path).ReplaceLineEndings("\n");
            if (stored != json)
            {
                try
                {
                    using var observation = JsonDocument.Parse(json);
                    using var fixture = JsonDocument.Parse(stored);
                    string field = Difference(observation.RootElement, fixture.RootElement, "fixture");
                    Console.Error.WriteLine($"permissions: fixture differs at {field ?? "formatting or property order"}");
                }
                catch (JsonException error)
                {
                    Console.Error.WriteLine($"permissions: invalid fixture JSON at line {error.LineNumber}, byte {error.BytePositionInLine}");
                }
                return 1;
            }
        }
        Console.WriteLine($"permissions: {rows.Length} cases {args[0]} passed");
        return 0;
    }

    static string Difference(JsonElement observed, JsonElement stored, string path)
    {
        if (observed.ValueKind != stored.ValueKind) return path;
        if (observed.ValueKind == JsonValueKind.Object)
        {
            foreach (var field in observed.EnumerateObject())
            {
                string key = $"{path}.{field.Name}";
                if (!stored.TryGetProperty(field.Name, out var value)) return key;
                if (Difference(field.Value, value, key) is { } difference) return difference;
            }
            foreach (var field in stored.EnumerateObject())
                if (!observed.TryGetProperty(field.Name, out _)) return $"{path}.{field.Name} (extra)";
            return null;
        }
        if (path == "fixture.cases")
        {
            int index = 0;
            foreach (var row in observed.EnumerateArray())
            {
                string name = row.GetProperty("input").GetProperty("name").GetString();
                string key = $"case {name}";
                if (index >= stored.GetArrayLength()) return $"{key} (missing)";
                if (Difference(row, stored[index++], key) is { } difference) return difference;
            }
            return index == stored.GetArrayLength() ? null : $"fixture.cases[{index}] (extra)";
        }
        return JsonElement.DeepEquals(observed, stored) ? null : path;
    }

    // The lightweight scene constructor provides a real graph and permission event dispatcher.
    sealed class FixtureScene(RegionInfo info, IConfigSource config) : Scene(info)
    {
        protected override IConfigSource GetConfig() => config;
    }

    static object Observe(Case c)
    {
        UUID objectGroup = c.NilGroup ? UUID.Zero : Group;
        UUID objectOwner = c.NilOwner ? UUID.Zero : c.OwnsObject ? Agent : c.Deeded ? objectGroup : Owner;
        var config = c.Settings.Config();
        var info = new RegionInfo { RegionName = "Fixture" };
        info.EstateSettings.EstateOwner = c.EstateOwner ? Agent : LandOwner;
        if (c.Manager) info.EstateSettings.AddEstateManager(Agent);
        var scene = new FixtureScene(info, config);
        scene.RegisterModuleInterface<IUserAccountService>(InputProxy.For<IUserAccountService>((m, a) =>
            m == "GetUserAccount" && (UUID)a[1] == Agent ? new UserAccount { UserLevel = c.UserLevel } : throw Unexpected(m)));
        scene.RegisterModuleInterface<IGroupsModule>(InputProxy.For<IGroupsModule>((m, a) =>
            m == "GetMembershipData" ? ((UUID)a[0] == objectGroup && (UUID)a[1] == Agent && c.Member
                ? new GroupMembershipData { GroupPowers = (ulong)c.Powers } : null) : throw Unexpected(m)));
        if (c.FriendCanModify)
            scene.RegisterModuleInterface<IFriendsModule>(InputProxy.For<IFriendsModule>((m, a) =>
                m == "GetRightsGrantedByFriend" ? ((UUID)a[0] == Agent && (UUID)a[1] == objectOwner
                    ? (int)FriendRights.CanModifyObjects : 0) : throw Unexpected(m)));
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
            "IsGroupMember" => (UUID)a[0] == objectGroup && c.Member,
            "GetGroupPowers" => (UUID)a[0] == objectGroup && c.Member ? (ulong)c.Powers : 0ul,
            "get_FirstName" => "Fixture", "get_LastName" => "User",
            "get_StartPos" => Position, "get_StartFar" => 128f,
            "RefreshGroupMembership" => null,
            _ when m.StartsWith("add_", StringComparison.Ordinal) => null,
            _ => throw Unexpected(m)
        });
        // The graph's normal presence registration is protected internal in the pin.
        var add = typeof(SceneGraph).GetMethod("CreateAndAddChildScenePresence", BindingFlags.Instance | BindingFlags.NonPublic);
        presence = (ScenePresence)add.Invoke(scene.SceneGraph, [client, new AvatarAppearance(), PresenceType.User]);

        var group = new SceneObjectGroup(objectOwner, Position, PrimitiveBaseShape.CreateBox());
        var root = group.RootPart;
        root.GroupID = objectGroup;
        root.BaseMask = (uint)c.BaseMask;
        root.OwnerMask = (uint)c.OwnerMask;
        root.GroupMask = (uint)c.GroupMask;
        root.EveryoneMask = (uint)c.EveryoneMask;
        if (c.CreationDate is int date) root.CreationDate = date;
        root.Flags = PrimFlags.Phantom;
        SceneObjectPart child = null;
        if (c.Child is { } linked)
        {
            child = new SceneObjectPart(objectOwner, PrimitiveBaseShape.CreateBox(), Position,
                Quaternion.Identity, Vector3.UnitX) { GroupID = objectGroup, OwnerMask = (uint)linked.OwnerMask };
            group.AddPart(child);
        }
        group.InvalidateEffectivePerms();
        if (!scene.AddNewSceneObject(group, false, false))
            throw new InvalidOperationException("fixture object did not enter the graph");
        group.IsAttachment = c.Attachment;
        RestoreItem(root, c.InventoryItem);
        if (child is not null) RestoreItem(child, c.Child.InventoryItem);
        var part = c.Child?.Observe == true ? child : root;

        // Supply the observed prim state after attach. Runtime event flags are derived by the part.
        part.Flags = c.Flags;
        part.AllowedDrop = c.Flags.HasFlag(PrimFlags.AllowInventoryDrop);
        scriptEvents events = 0;
        if (c.Flags.HasFlag(PrimFlags.Touch)) events |= scriptEvents.touch_start;
        if (c.Flags.HasFlag(PrimFlags.Money)) events |= scriptEvents.money;
        if (events != 0 || part.AllowedDrop) part.SetScriptEvents(UUID.Random(), (ulong)events);
        var p = scene.Permissions;
        uint viewerFlags = p.GenerateClientFlags(part, presence);
        uint initialFlags = flagInput ?? throw new InvalidOperationException("flag handler was not called");
        return new {
            effectiveOwnerMask = group.EffectiveOwnerPerms, effectiveGroupMask = group.EffectiveGroupPerms,
            effectiveEveryoneMask = group.EffectiveEveryOnePerms,
            effectiveGroupOrEveryoneMask = group.EffectiveGroupOrEveryOnePerms, isGod = presence.IsGod,
            isViewerUIGod = presence.IsViewerUIGod, clientFlagInput = initialFlags,
            moduleClientFlags = module.GenerateClientFlags(part, presence, initialFlags),
            generateClientFlags = viewerFlags,
            canRez = p.CanRezObject(1, Agent, Position),
            canEdit = p.CanEditObject(group, client), canMove = p.CanMoveObject(group, client),
            canDelete = p.CanDeleteObject(group, client), canTake = p.CanTakeObject(group, presence),
            canLink = p.CanLinkObject(Agent, part.UUID), canTakeCopy = p.CanTakeCopyObject(group, presence),
            canDuplicate = p.CanDuplicateObject(group, Agent), canEditById = p.CanEditObject(part.UUID, Agent),
            canDeleteById = p.CanDeleteObject(part.UUID, Agent), canDelink = p.CanDelinkObject(Agent, part.UUID),
            canSelectObject = p.CanChangeSelectedState(part, presence)
        };
    }

    static void RestoreItem(SceneObjectPart part, ItemMasks masks)
    {
        if (masks is not null)
            part.Inventory.RestoreInventoryItems([new TaskInventoryItem {
                ItemID = UUID.Random(), OwnerID = part.OwnerID, InvType = (int)InventoryType.Notecard,
                CurrentPermissions = (uint)masks.Current, GroupPermissions = (uint)masks.Group,
                EveryonePermissions = (uint)masks.Everyone
            }]);
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
