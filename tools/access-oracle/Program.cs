// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Text;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.CoreModules.World.Permissions;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Interfaces;

namespace Ferroan.AccessOracle;

static class Program
{
    static readonly UUID Owner = new UUID("00000000-0000-4000-8000-000000000001");
    static readonly UUID Manager = new UUID("00000000-0000-4000-8000-000000000002");
    static readonly UUID Ordinary = new UUID("00000000-0000-4000-8000-000000000003");
    static readonly UUID God = new UUID("00000000-0000-4000-8000-000000000004");

    sealed class Case
    {
        public string Name;
        public UUID Agent;
        public int Level;
        public bool Allow;
        public bool OwnerGod;
        public bool ManagerGod;
        public bool Force;
        public string Modules = "DefaultPermissionsModule";
        public bool? ServerPermissions = true;
        public string PermissionsModulesOverride;
        public bool? ServerPermissionsOverride;
    }

    static int Main(string[] args)
    {
        if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision"
            || args[3].Length != 40 || !args[3].All(Uri.IsHexDigit))
        {
            Console.Error.WriteLine("usage: AccessOracle generate|verify <directory> --revision <40 hex>");
            return 2;
        }
        string directory = Path.GetFullPath(args[1]);
        TextWriter stdout = Console.Out;
        TextWriter stderr = Console.Error;
        try
        {
            Console.SetOut(TextWriter.Null);
            Console.SetError(TextWriter.Null);
            string json = Capture(args[3]);
            Console.SetOut(stdout);
            Console.SetError(stderr);
            Directory.CreateDirectory(directory);
            Publish(Path.Combine(directory, "full-region.json"), json, args[0]);
            return 0;
        }
        catch (Exception error)
        {
            Console.SetOut(stdout);
            Console.SetError(stderr);
            Console.Error.WriteLine(error);
            return 1;
        }
    }

    static string Capture(string revision)
    {
        Case[] cases =
        {
            One("ordinary", Ordinary, 0, true, false, false, false),
            new Case { Name = "settings-unset", Agent = Ordinary, Modules = null, ServerPermissions = null },
            new Case { Name = "permissions-bypassed", Agent = Ordinary, Allow = true, ServerPermissions = false },
            new Case { Name = "permissions-overrides-startup", Agent = Ordinary, Modules = "Other", ServerPermissions = false,
                PermissionsModulesOverride = "DefaultPermissionsModule", ServerPermissionsOverride = true },
            new Case { Name = "permissions-bypassed-nil", Agent = UUID.Zero, ServerPermissions = false },
            new Case { Name = "default-module-absent", Agent = Ordinary, Modules = "Other" },
            new Case { Name = "default-module-empty", Agent = Ordinary, Modules = "" },
            new Case { Name = "default-module-case-sensitive", Agent = Ordinary, Modules = "defaultpermissionsmodule" },
            new Case { Name = "default-module-in-list", Agent = Ordinary, Modules = "First, DefaultPermissionsModule , Last" },
            One("estate-owner", Owner, 0, false, false, false, false),
            One("estate-manager", Manager, 0, false, false, false, false),
            One("grid-god", God, 200, true, false, false, false),
            One("level-199", God, 199, true, false, false, false),
            One("gods-disabled", God, 240, false, false, false, false),
            One("nil-user", UUID.Zero, 200, true, false, false, false),
            One("force-grid-gods-only", God, 200, false, true, true, true),
            One("force-keeps-estate-owner", Owner, 0, false, true, true, true),
            One("region-owner-is-god-is-not-enough", Ordinary, 0, false, true, false, false),
            One("region-manager-is-god-keeps-the-manager", Manager, 0, false, false, true, false),
            One("region-manager-is-god-is-not-enough", Ordinary, 0, false, false, true, false),
        };
        var body = new StringBuilder();
        body.Append("{\n");
        body.Append("  \"pin\": ").Append(JsonString(revision)).Append(",\n");
        body.Append("  \"method\": \"Scene.CheckLandPositionAccess\",\n");
        body.Append("  \"agent_limit\": 0,\n");
        body.Append("  \"not_crossing\": false,\n");
        body.Append("  \"owner\": ").Append(JsonString(Owner.ToString())).Append(",\n");
        body.Append("  \"manager\": ").Append(JsonString(Manager.ToString())).Append(",\n");
        body.Append("  \"cases\": [\n");
        for (int index = 0; index < cases.Length; index++)
        {
            Case item = cases[index];
            (bool success, string reason) = Observe(item);
            body.Append("    {\n");
            body.Append("      \"name\": ").Append(JsonString(item.Name)).Append(",\n");
            body.Append("      \"agent\": ").Append(JsonString(item.Agent.ToString())).Append(",\n");
            body.Append("      \"user_level\": ").Append(item.Level.ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(",\n");
            body.Append("      \"allow_grid_gods\": ").Append(item.Allow ? "true" : "false").Append(",\n");
            body.Append("      \"region_owner_is_god\": ").Append(item.OwnerGod ? "true" : "false").Append(",\n");
            body.Append("      \"region_manager_is_god\": ").Append(item.ManagerGod ? "true" : "false").Append(",\n");
            body.Append("      \"force_grid_gods_only\": ").Append(item.Force ? "true" : "false").Append(",\n");
            if (item.Modules != null)
                body.Append("      \"permissionmodules\": ").Append(JsonString(item.Modules)).Append(",\n");
            if (item.ServerPermissions is bool serverPermissions)
                body.Append("      \"serverside_object_permissions\": ").Append(serverPermissions ? "true" : "false").Append(",\n");
            if (item.PermissionsModulesOverride != null)
                body.Append("      \"permissions_modules_override\": ").Append(JsonString(item.PermissionsModulesOverride)).Append(",\n");
            if (item.ServerPermissionsOverride is bool serverOverride)
                body.Append("      \"permissions_serverside_override\": ").Append(serverOverride ? "true" : "false").Append(",\n");
            body.Append("      \"success\": ").Append(success ? "true" : "false").Append(",\n");
            body.Append("      \"reason\": ").Append(JsonString(reason)).Append("\n");
            body.Append(index + 1 == cases.Length ? "    }\n" : "    },\n");
        }
        body.Append("  ]\n}\n");
        return body.ToString();
    }

    static Case One(string name, UUID agent, int level, bool allow, bool ownerGod, bool managerGod, bool force)
    {
        return new Case
        {
            Name = name,
            Agent = agent,
            Level = level,
            Allow = allow,
            OwnerGod = ownerGod,
            ManagerGod = managerGod,
            Force = force,
        };
    }

    static (bool Success, string Reason) Observe(Case item)
    {
        RegionInfo info = new RegionInfo();
        info.RegionName = "Fixture";
        info.RegionSettings.AgentLimit = 0;
        info.EstateSettings.EstateOwner = Owner;
        info.EstateSettings.AddEstateManager(Manager);
        Scene scene = new Scene(info);
        scene.RegisterModuleInterface<IUserAccountService>(new LevelAccount(item.Agent, item.Level));
        DefaultPermissionsModule module = new DefaultPermissionsModule();
        module.Initialise(Permissions(item));
        module.AddRegion(scene);
        bool success = scene.CheckLandPositionAccess(
            item.Agent, false, false, true, Vector3.Zero, out string reason);
        reason ??= "";
        if (success && reason.Length != 0)
            throw new InvalidOperationException(item.Name + " succeeded with reason " + reason);
        if (!success && reason != "The region is full")
            throw new InvalidOperationException(item.Name + " refused with reason " + reason);
        return (success, reason);
    }

    static IniConfigSource Permissions(Case item)
    {
        IniConfigSource source = new IniConfigSource();
        IConfig startup = source.AddConfig("Startup");
        IConfig permissions = source.AddConfig("Permissions");
        if (item.Modules != null)
            startup.Set("permissionmodules", item.Modules);
        if (item.ServerPermissions is bool serverPermissions)
            startup.Set("serverside_object_permissions", serverPermissions ? "true" : "false");
        if (item.PermissionsModulesOverride != null)
            permissions.Set("permissionmodules", item.PermissionsModulesOverride);
        if (item.ServerPermissionsOverride is bool serverOverride)
            permissions.Set("serverside_object_permissions", serverOverride ? "true" : "false");
        permissions.Set("allow_grid_gods", item.Allow ? "true" : "false");
        permissions.Set("region_owner_is_god", item.OwnerGod ? "true" : "false");
        permissions.Set("region_manager_is_god", item.ManagerGod ? "true" : "false");
        permissions.Set("force_grid_gods_only", item.Force ? "true" : "false");
        return source;
    }

    static string JsonString(string value)
    {
        return "\"" + value.Replace("\\", "\\\\", StringComparison.Ordinal).Replace("\"", "\\\"", StringComparison.Ordinal) + "\"";
    }

    static void Publish(string path, string json, string action)
    {
        if (action == "generate")
        {
            File.WriteAllText(path, json);
            return;
        }
        if (!File.Exists(path))
            throw new InvalidOperationException("missing " + path);
        string found = File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
        if (!string.Equals(found, json, StringComparison.Ordinal))
            throw new InvalidOperationException(path + " does not match this pin");
    }

    sealed class LevelAccount : IUserAccountService
    {
        readonly UUID agent;
        readonly int level;

        public LevelAccount(UUID agent, int level)
        {
            this.agent = agent;
            this.level = level;
        }

        public UserAccount GetUserAccount(UUID scopeID, UUID userID)
        {
            if (userID != agent)
                return null;
            UserAccount account = new UserAccount(scopeID, userID, "Fixture", "User", "fixture@example.invalid");
            account.UserLevel = level;
            return account;
        }

        public UserAccount GetUserAccount(UUID scopeID, string FirstName, string LastName)
        {
            return null;
        }

        public UserAccount GetUserAccount(UUID scopeID, string Email)
        {
            return null;
        }

        public bool SetDisplayName(UUID agentID, string displayName)
        {
            return false;
        }

        public List<UserAccount> GetUserAccounts(UUID scopeID, string query)
        {
            return new List<UserAccount>();
        }

        public List<UserAccount> GetUserAccountsWhere(UUID scopeID, string where)
        {
            return new List<UserAccount>();
        }

        public List<UserAccount> GetUserAccounts(UUID scopeID, List<string> IDs)
        {
            return new List<UserAccount>();
        }

        public bool StoreUserAccount(UserAccount data)
        {
            return false;
        }

        public void InvalidateCache(UUID userID)
        {
        }
    }
}
