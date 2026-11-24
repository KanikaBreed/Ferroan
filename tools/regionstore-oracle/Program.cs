// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Data;
using System.Data.Common;
using System.Data.SQLite;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using MySqlConnector;
using OpenMetaverse;
using OpenMetaverse.StructuredData;
using OpenSim.Data.MySQL;
using OpenSim.Data.SQLite;
using OpenSim.Framework;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Region.PhysicsModules.SharedBase;

const string Region = "11111111-1111-1111-1111-111111111111";
const string OtherRegion = "22222222-2222-2222-2222-222222222222";
const string Owner = "00000000-0000-0000-0000-000000000001";
const string LastOwner = "00000000-0000-0000-0000-000000000002";
const string GroupOwner = "00000000-0000-0000-0000-000000000003";
const string Creator = "00000000-0000-0000-0000-000000000004";
const string Rezzer = "00000000-0000-0000-0000-000000000005";
const string Sound = "00000000-0000-0000-0000-0000000000aa";
const string CollisionSound = "00000000-0000-0000-0000-0000000000bb";

const string PrimA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1";
const string RootB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1";
const string ChildB3 = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb3";
const string ChildB2 = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2";
const string RootC = "cccccccc-cccc-cccc-cccc-ccccccccccc1";
const string ChildC2 = "cccccccc-cccc-cccc-cccc-ccccccccccc2";
const string ChildC3 = "cccccccc-cccc-cccc-cccc-ccccccccccc3";
const string PrimD = "dddddddd-dddd-dddd-dddd-ddddddddddd1";
const string PrimF = "ffffffff-ffff-ffff-ffff-fffffffffff1";
const string NullPrim = "99999999-9999-9999-9999-999999999991";
const string Orphan = "99999999-9999-9999-9999-999999999992";
const string MissingParent = "abababab-abab-abab-abab-abababababab";

if (args.Length == 0)
{
    Console.Error.WriteLine("usage: regionstore-oracle seed <db> | load <db> <region-uuid> <pin> | bulk <db> <region-uuid> <count> | mysql-seed <ado> | mysql-load <ado> <region-uuid> <pin>");
    return 2;
}

try
{
    switch (args[0])
    {
        case "seed" when args.Length == 2:
            Seed(args[1]);
            return 0;
        case "load" when args.Length == 4:
            Load(args[1], args[2], args[3]);
            return 0;
        case "bulk" when args.Length == 4:
            Bulk(args[1], args[2], args[3]);
            return 0;
        case "mysql-seed" when args.Length == 2:
            SeedMysql(args[1]);
            return 0;
        case "mysql-load" when args.Length == 4:
            LoadMysql(args[1], args[2], args[3]);
            return 0;
        default:
            Console.Error.WriteLine("usage: regionstore-oracle seed <db> | load <db> <region-uuid> <pin> | bulk <db> <region-uuid> <count> | mysql-seed <ado> | mysql-load <ado> <region-uuid> <pin>");
            return 2;
    }
}
catch (Exception error)
{
    Console.Error.WriteLine(error.GetType().Name + ": " + error.Message);
    return 1;
}

static void Bulk(string path, string region, string countText)
{
    if (!UUID.TryParse(region, out UUID _))
        throw new ArgumentException("region id is not a UUID");
    if (!int.TryParse(countText, out int count) || count < 1 || count > 100_000)
        throw new ArgumentException("count must be from 1 to 100000");
    string full = Path.GetFullPath(path);
    Directory.CreateDirectory(Path.GetDirectoryName(full)!);
    if (File.Exists(full))
        File.Delete(full);
    string connection = "Data Source=" + full + ";Version=3;";
    SQLiteSimulationData store = new SQLiteSimulationData();
    try
    {
        store.Initialise(connection);
    }
    finally
    {
        store.Dispose();
    }

    using SQLiteConnection db = new SQLiteConnection(connection);
    db.Open();
    using SQLiteTransaction tx = db.BeginTransaction();
    for (int index = 0; index < count; index++)
    {
        string id = index.ToString("x8") + "-0000-4000-8000-000000000000";
        BasePrim(id, region, id, "prim").Insert(db, "prims");
    }
    tx.Commit();
}

static void Seed(string path)
{
    string full = Path.GetFullPath(path);
    Directory.CreateDirectory(Path.GetDirectoryName(full)!);
    if (File.Exists(full))
        File.Delete(full);
    string connection = "Data Source=" + full + ";Version=3;";
    SQLiteSimulationData store = new SQLiteSimulationData();
    try
    {
        store.Initialise(connection);
    }
    finally
    {
        store.Dispose();
    }

    using SQLiteConnection db = new SQLiteConnection(connection);
    db.Open();
    using SQLiteTransaction tx = db.BeginTransaction();
    InsertPrims(db, false);
    InsertShapes(db, false);
    InsertItems(db, false);
    tx.Commit();
}

static void SeedMysql(string ado)
{
    MySQLSimulationData store = new MySQLSimulationData();
    try
    {
        store.Initialise(ado);
    }
    finally
    {
        store.Dispose();
    }

    using MySqlConnection db = new MySqlConnection(ado);
    db.Open();
    using (MySqlCommand version = db.CreateCommand())
    {
        version.CommandText = "SELECT version FROM migrations WHERE name = 'RegionStore'";
        object found = version.ExecuteScalar();
        if (found == null || Convert.ToInt32(found) != 71)
            throw new InvalidOperationException("RegionStore version is " + found);
    }
    InsertPrims(db, true);
    InsertShapes(db, true);
    InsertItems(db, true);
    InsertNullPrim(db);
    InsertOrphan(db);
}

static void InsertNullPrim(MySqlConnection db)
{
    using (MySqlCommand insert = db.CreateCommand())
    {
        insert.CommandText = "INSERT INTO prims (UUID, RegionUUID, SceneGroupID) VALUES (@id, @region, @id)";
        insert.Parameters.AddWithValue("@id", NullPrim);
        insert.Parameters.AddWithValue("@region", Region);
        insert.ExecuteNonQuery();
    }
    NullNullable(db, "prims", "UUID", NullPrim, new HashSet<string> { "UUID", "RegionUUID", "SceneGroupID" });
    using (MySqlCommand shape = db.CreateCommand())
    {
        shape.CommandText = "INSERT INTO primshapes (UUID, Shape) VALUES (@id, 0)";
        shape.Parameters.AddWithValue("@id", NullPrim);
        shape.ExecuteNonQuery();
    }
    NullNullable(db, "primshapes", "UUID", NullPrim, new HashSet<string> { "UUID", "Shape" });
}

static void InsertOrphan(MySqlConnection db)
{
    BasePrim(Orphan, Region, MissingParent, "orphan").Set("LinkNumber", 4).Insert(db, "prims");
    PlainShape(Orphan).Set("Shape", 0).Insert(db, "primshapes");
}

static void NullNullable(MySqlConnection db, string table, string key, string id, HashSet<string> keep)
{
    List<string> columns = new List<string>();
    using (MySqlCommand list = db.CreateCommand())
    {
        list.CommandText = "SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = @table AND IS_NULLABLE = 'YES'";
        list.Parameters.AddWithValue("@table", table);
        using MySqlDataReader reader = list.ExecuteReader();
        while (reader.Read())
        {
            string name = reader.GetString(0);
            if (!keep.Contains(name))
                columns.Add(name);
        }
    }
    if (columns.Count == 0)
        return;
    string assigns = string.Join(", ", columns.ConvertAll(name => "`" + name.Replace("`", "``") + "` = NULL"));
    using MySqlCommand update = db.CreateCommand();
    update.CommandText = "UPDATE `" + table + "` SET " + assigns + " WHERE `" + key + "` = @id";
    update.Parameters.AddWithValue("@id", id);
    update.ExecuteNonQuery();
}

static void InsertPrims(DbConnection db, bool mysql)
{
    Row showcase = BasePrim(PrimA, Region, PrimA, "showcase")
        .Set("Text", "hello")
        .Set("Description", "desc")
        .Set("SitName", "sit")
        .Set("TouchName", "touch")
        .Set("ObjectFlags", 0x10681)
        .Set("PositionX", 0.5)
        .Set("PositionY", 1.5)
        .Set("PositionZ", 2.5)
        .Set("GroupPositionX", 10.0)
        .Set("GroupPositionY", 20.0)
        .Set("GroupPositionZ", 30.0)
        .Set("VelocityZ", 1.0)
        .Set("RotationW", 1.0)
        .Set("SitTargetOffsetX", 1.0)
        .Set("SitTargetActive", 0)
        .Set("standtargetx", 0.5)
        .Set("sitactrange", 2.5)
        .Set("ClickAction", 3)
        .Set("PayPrice", 10)
        .Set("PayButton1", 20)
        .Set("PayButton2", 30)
        .Set("PayButton3", 40)
        .Set("PayButton4", 50)
        .Set("LoopedSound", Sound)
        .Set("LoopedSoundGain", 0.5)
        .Set("TextureAnimation", Convert.ToBase64String(new byte[] { 0x10, 0x20, 0x30 }))
        .Set("ParticleSystem", Convert.ToBase64String(new byte[] { 0x40 }))
        .Set("CameraEyeOffsetX", 1.0)
        .Set("CameraEyeOffsetY", 2.0)
        .Set("CameraEyeOffsetZ", 3.0)
        .Set("CameraAtOffsetX", 4.0)
        .Set("CameraAtOffsetY", 5.0)
        .Set("CameraAtOffsetZ", 6.0)
        .Set("ForceMouselook", 1)
        .Set("ScriptAccessPin", 1234)
        .Set("AllowedDrop", 1)
        .Set("DieAtEdge", 1)
        .Set("SalePrice", 15)
        .Set("SaleType", 2)
        .Set("Material", 7)
        .Set("CollisionSound", CollisionSound)
        .Set("CollisionSoundVolume", 0.25);
    if (!mysql)
        showcase.Set("VolumeDetect", 1);
    showcase
        .Set("MediaURL", "http://example.test/media")
        .Set("AttachedPosX", 0.5)
        .Set("DynAttrs", DynXml())
        .Set("PhysicsShapeType", 1)
        .Set("Density", 2.0)
        .Set("GravityModifier", 1.5)
        .Set("Friction", 0.25)
        .Set("Restitution", 0.125)
        .Set("KeyframeMotion", KeyframeBlob())
        .Set("PassCollisions", 1)
        .Set("PassTouches", 1)
        .Set("RotationAxisLocks", 5)
        .Set("Vehicle", "<Vehicle><TYPE>2</TYPE><LMTIME>1.5</LMTIME></Vehicle>")
        .Set("RezzerID", Rezzer)
        .Set("PhysInertia", "<PhysicsInertia><MASS>2.5</MASS><CM><X>1</X><Y>2</Y><Z>3</Z></CM></PhysicsInertia>")
        .Set("pseudocrc", 42)
        .Set("sopanims", AnimationBlob())
        .Set("linksetdata", "{\"a\":{\"Value\":\"w\",\"Password\":\"\"},\"b\":{\"Value\":\"v\",\"Password\":\"secret\"}}")
        .Set("StartStr", "rez-arg")
        .Set("LinkNumber", 7)
        .Set("ColorR", 10)
        .Set("ColorG", 20)
        .Set("ColorB", 30)
        .Set("ColorA", 40)
        .Insert(db, "prims");

    BasePrim(RootB, Region, RootB, "link-restore").Set("LinkNumber", 9).Insert(db, "prims");
    BasePrim(ChildB3, Region, RootB, "child-three").Set("LinkNumber", 3).Insert(db, "prims");
    BasePrim(ChildB2, Region, RootB, "child-two")
        .Set("LinkNumber", 2)
        .Set("SitTargetActive", 1)
        .Set("DynAttrs", "<llsd><map></map></llsd>")
        .Set("KeyframeMotion", new byte[] { 0x01, 0x02, 0x03, 0x04 })
        .Set("Vehicle", "<Vehicle></Vehicle>")
        .Set("PhysInertia", "<PhysicsInertia></PhysicsInertia>")
        .Set("sopanims", new byte[] { 0x00 })
        .Set("linksetdata", "not-json")
        .Insert(db, "prims");

    BasePrim(RootC, Region, RootC, "link-table").Insert(db, "prims");
    BasePrim(ChildC2, Region, RootC, "child-first").Insert(db, "prims");
    BasePrim(ChildC3, Region, RootC, "child-second").Set("LinkNumber", 5).Insert(db, "prims");
    // SQLite leaves this prim out and continues. On MySQL the same document
    // stops the region's load, so the successful MySQL seed does not include it.
    if (!mysql)
        BasePrim("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1", Region, RootC, "unparsed")
            .Set("DynAttrs", "<llsd><integer>1</integer></llsd>")
            .Insert(db, "prims");

    BasePrim(PrimD, Region, PrimD, "no-shape")
        .Set("SitTargetOffsetX", 2.0)
        .Set("pseudocrc", 0)
        .Insert(db, "prims");

    // The SQLite fixture keeps a row whose id does not parse. MySQL's load
    // reads that id as text and a bad id is not part of this store's dump.
    if (!mysql)
        BasePrim("not-a-uuid", Region, "not-a-uuid", "unbuildable").Insert(db, "prims");
    BasePrim(PrimF, OtherRegion, PrimF, "other-region").Insert(db, "prims");
}

static void InsertItems(DbConnection db, bool mysql)
{
    // Stored order is not item-id order. A null description, a creator id, a
    // creator endpoint with no name, and an unparsable experience id all load.
    TaskItem(PrimA, "c0000000-0000-0000-0000-000000000003", "00000000-0000-0000-0000-0000000000c3", "00000000-0000-0000-0000-0000000000d3", "note", 7, 7)
        .Set("creationDate", 1700000103)
        .Set("creatorID", Creator)
        .Set("nextPermissions", 11)
        .Set("currentPermissions", 22)
        .Set("basePermissions", 33)
        .Set("everyonePermissions", 44)
        .Set("groupPermissions", 55)
        .Set("flags", 66)
        // "nope" is the SQLite cell. A CHAR(36) value that is not a uuid stops
        // the MySQL load, so that seed stores the zero id the load would keep.
        .Set("experienceID", mysql ? "00000000-0000-0000-0000-000000000000" : "nope")
        .Insert(db, "primitems");
    TaskItem(PrimA, "c0000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-0000000000c1", "00000000-0000-0000-0000-0000000000d1", "script", 10, 10)
        .Set("description", "runs")
        .Set("creationDate", 1700000101)
        .Set("creatorID", Creator + ";https://profile.example/agent")
        .Set("nextPermissions", 2147483647)
        .Set("currentPermissions", 2147483647)
        .Set("basePermissions", 2147483647)
        .Set("everyonePermissions", 2147483647)
        .Set("groupPermissions", 2147483647)
        .Set("flags", 0)
        .Set("experienceID", "00000000-0000-0000-0000-000000000000")
        .Insert(db, "primitems");
    TaskItem(PrimA, "c0000000-0000-0000-0000-000000000002", "00000000-0000-0000-0000-0000000000c2", "00000000-0000-0000-0000-0000000000d2", "box", 6, 6)
        .Set("description", "worn")
        .Set("creationDate", 1700000102)
        .Set("creatorID", Creator + ";https://profile.example/agent;Ada Lovelace")
        .Set("nextPermissions", 8)
        .Set("currentPermissions", 16)
        .Set("basePermissions", 32)
        .Set("everyonePermissions", 0)
        .Set("groupPermissions", 1)
        .Set("flags", 0)
        .Set("experienceID", "abcdefab-cdef-abcd-efab-cdefabcdefab")
        .Insert(db, "primitems");
    TaskItem(ChildB2, "b2222222-2222-2222-2222-222222222222", "00000000-0000-0000-0000-0000000000b2", "00000000-0000-0000-0000-0000000000e2", "mark", 3, 3)
        .Set("description", "child")
        .Set("creationDate", 1700000104)
        .Set("creatorID", Creator + ";")
        .Set("nextPermissions", 7)
        .Set("currentPermissions", 7)
        .Set("basePermissions", 7)
        .Set("everyonePermissions", 0)
        .Set("groupPermissions", 0)
        .Set("flags", 1)
        .Set("experienceID", "00000000-0000-0000-0000-000000000000")
        .Insert(db, "primitems");
    TaskItem(PrimD, "d0000000-0000-0000-0000-000000000009", "00000000-0000-0000-0000-0000000000d9", "00000000-0000-0000-0000-0000000000e9", "blank", 7, 7)
        .Set("description", "")
        .Set("creationDate", 1700000105)
        .Set("creatorID", "")
        .Set("nextPermissions", 0)
        .Set("currentPermissions", 0)
        .Set("basePermissions", 0)
        .Set("everyonePermissions", 0)
        .Set("groupPermissions", 0)
        .Set("flags", 0)
        .Insert(db, "primitems");
    TaskItem("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1", "e0000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-0000000000e1", "00000000-0000-0000-0000-0000000000f1", "gone", 7, 7)
        .Set("description", "unbuilt prim")
        .Set("creationDate", 1700000106)
        .Set("creatorID", Creator)
        .Set("experienceID", "00000000-0000-0000-0000-000000000000")
        .Insert(db, "primitems");
    TaskItem(PrimF, "f0000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-0000000000f2", "00000000-0000-0000-0000-0000000000f3", "elsewhere", 7, 7)
        .Set("description", "other region")
        .Set("creationDate", 1700000107)
        .Set("creatorID", Creator)
        .Set("experienceID", "00000000-0000-0000-0000-000000000000")
        .Insert(db, "primitems");
}

static Row TaskItem(string prim, string item, string asset, string folder, string name, int inv, int assetType)
{
    return new Row()
        .Set("itemID", item)
        .Set("primID", prim)
        .Set("assetID", asset)
        .Set("parentFolderID", folder)
        .Set("invType", inv)
        .Set("assetType", assetType)
        .Set("name", name)
        .Set("ownerID", Owner)
        .Set("lastOwnerID", LastOwner)
        .Set("groupID", GroupOwner);
}

static void InsertShapes(DbConnection db, bool mysql)
{
    Row Mark(Row row)
    {
        if (mysql)
            row.Set("Shape", 0);
        return row;
    }
    Mark(ShowcaseShape(PrimA)).Insert(db, "primshapes");
    Mark(PlainShape(RootB)).Insert(db, "primshapes");
    Mark(PlainShape(ChildB3)).Insert(db, "primshapes");
    Mark(PlainShape(ChildB2)
        .Set("Media", "<OSMedia type=\"none\"></OSMedia>")
        .Set("MatOvrd", new byte[] { 0x00 }))
        .Insert(db, "primshapes");
    Mark(PlainShape(RootC)).Insert(db, "primshapes");
    Mark(PlainShape(ChildC2)).Insert(db, "primshapes");
    Mark(PlainShape(ChildC3)).Insert(db, "primshapes");
    Mark(PlainShape(PrimF)).Insert(db, "primshapes");
}

static Row BasePrim(string uuid, string region, string group, string name)
{
    return new Row()
        .Set("UUID", uuid)
        .Set("RegionUUID", region)
        .Set("CreationDate", 1700000000)
        .Set("Name", name)
        .Set("SceneGroupID", group)
        .Set("Text", "")
        .Set("Description", "")
        .Set("SitName", "")
        .Set("TouchName", "")
        .Set("CreatorID", Creator)
        .Set("OwnerID", Owner)
        .Set("GroupID", GroupOwner)
        .Set("LastOwnerID", LastOwner)
        .Set("OwnerMask", 0)
        .Set("NextOwnerMask", 0)
        .Set("GroupMask", 0)
        .Set("EveryoneMask", 0)
        .Set("BaseMask", 0)
        .Set("PositionX", 0.0)
        .Set("PositionY", 0.0)
        .Set("PositionZ", 0.0)
        .Set("GroupPositionX", 0.0)
        .Set("GroupPositionY", 0.0)
        .Set("GroupPositionZ", 0.0)
        .Set("VelocityX", 0.0)
        .Set("VelocityY", 0.0)
        .Set("VelocityZ", 0.0)
        .Set("AngularVelocityX", 0.0)
        .Set("AngularVelocityY", 0.0)
        .Set("AngularVelocityZ", 0.0)
        .Set("AccelerationX", 0.0)
        .Set("AccelerationY", 0.0)
        .Set("AccelerationZ", 0.0)
        .Set("RotationX", 0.0)
        .Set("RotationY", 0.0)
        .Set("RotationZ", 0.0)
        .Set("RotationW", 0.0)
        .Set("ObjectFlags", 0);
}

static Row ShowcaseShape(string uuid)
{
    return new Row()
        .Set("UUID", uuid)
        .Set("ScaleX", 1.0)
        .Set("ScaleY", 2.0)
        .Set("ScaleZ", 3.0)
        .Set("PCode", 9)
        .Set("PathBegin", 10)
        .Set("PathEnd", 20)
        .Set("PathScaleX", 50)
        .Set("PathScaleY", 80)
        .Set("PathShearX", 1)
        .Set("PathShearY", 2)
        .Set("PathSkew", -3)
        .Set("PathCurve", 32)
        .Set("PathRadiusOffset", 4)
        .Set("PathRevolutions", 2)
        .Set("PathTaperX", -5)
        .Set("PathTaperY", 6)
        .Set("PathTwist", 7)
        .Set("PathTwistBegin", -8)
        .Set("ProfileBegin", 9)
        .Set("ProfileEnd", 11)
        .Set("ProfileCurve", 17)
        .Set("ProfileHollow", 100)
        .Set("Texture", new byte[] { 0x11, 0x22, 0x33, 0x44 })
        .Set("ExtraParams", new byte[] { 0x00, 0xaa })
        .Set("State", 2)
        .Set("Media", MediaXml())
        .Set("LastAttachPoint", 3)
        .Set("MatOvrd", MaterialBlob());
}

static Row PlainShape(string uuid)
{
    return new Row()
        .Set("UUID", uuid)
        .Set("ScaleX", 1.0)
        .Set("ScaleY", 1.0)
        .Set("ScaleZ", 1.0)
        .Set("PCode", 9)
        .Set("PathBegin", 0)
        .Set("PathEnd", 0)
        .Set("PathScaleX", 100)
        .Set("PathScaleY", 100)
        .Set("PathShearX", 0)
        .Set("PathShearY", 0)
        .Set("PathSkew", 0)
        .Set("PathCurve", 16)
        .Set("PathRadiusOffset", 0)
        .Set("PathRevolutions", 0)
        .Set("PathTaperX", 0)
        .Set("PathTaperY", 0)
        .Set("PathTwist", 0)
        .Set("PathTwistBegin", 0)
        .Set("ProfileBegin", 0)
        .Set("ProfileEnd", 0)
        .Set("ProfileCurve", 1)
        .Set("ProfileHollow", 0)
        .Set("Texture", new byte[] { 0x01, 0x02, 0x03 })
        .Set("ExtraParams", new byte[] { 0x00 })
        .Set("State", 0)
        .Set("LastAttachPoint", 0);
}

static string DynXml()
{
    return "<llsd><map><key>kept</key><map><key>store</key><map><key>n</key><integer>7</integer></map>"
        + "<key>dropped</key><integer>1</integer></map><key>badns</key><string>nope</string></map></llsd>";
}

static string MediaXml()
{
    return "<OSMedia type=\"sl\" version=\"0.1\"><OSData><llsd><array><map>"
        + "<key>alt_image_enable</key><boolean>1</boolean>"
        + "<key>auto_loop</key><boolean>1</boolean>"
        + "<key>auto_play</key><boolean>0</boolean>"
        + "<key>auto_scale</key><boolean>1</boolean>"
        + "<key>auto_zoom</key><boolean>0</boolean>"
        + "<key>controls</key><integer>1</integer>"
        + "<key>current_url</key><string>http://current.test</string>"
        + "<key>first_click_interact</key><boolean>1</boolean>"
        + "<key>height_pixels</key><integer>480</integer>"
        + "<key>home_url</key><string>http://home.test</string>"
        + "<key>perms_control</key><integer>1</integer>"
        + "<key>perms_interact</key><integer>4</integer>"
        + "<key>whitelist</key><array><string>http://ok.test</string></array>"
        + "<key>whitelist_enable</key><boolean>1</boolean>"
        + "<key>width_pixels</key><integer>640</integer>"
        + "</map><integer>0</integer></array></llsd></OSData></OSMedia>";
}

static byte[] KeyframeBlob()
{
    using MemoryStream stream = new MemoryStream();
    using BinaryWriter writer = new BinaryWriter(stream);
    writer.Write((byte)'K');
    writer.Write((byte)'F');
    writer.Write((byte)'M');
    writer.Write((byte)'1');
    writer.Write((uint)1);
    WriteVec(writer, 1f, 2f, 3f);
    WriteVec(writer, 4f, 5f, 6f);
    WriteQuat(writer, 0f, 0f, 0f, 1f);
    writer.Write(true);
    WriteVec(writer, 7f, 8f, 9f);
    writer.Write(false);
    WriteQuat(writer, 0f, 0f, 0f, 1f);
    writer.Write(10);
    writer.Write(20);
    WriteVec(writer, 0f, 1f, 0f);
    WriteVec(writer, 1f, 0f, 0f);
    writer.Write(1);
    writer.Write(false);
    writer.Write(true);
    WriteQuat(writer, 0f, 0f, 0f, 1f);
    WriteQuat(writer, 0f, 0f, 0f, 1f);
    writer.Write(5);
    writer.Write(5);
    WriteVec(writer, 0f, 0f, 0f);
    WriteVec(writer, 0f, 0f, 0f);
    writer.Write(0);
    writer.Write(2);
    writer.Write(3);
    writer.Write(true);
    writer.Write(4);
    writer.Write(6);
    writer.Flush();
    return stream.ToArray();
}

static void WriteVec(BinaryWriter writer, float x, float y, float z)
{
    writer.Write(x);
    writer.Write(y);
    writer.Write(z);
}

static void WriteQuat(BinaryWriter writer, float x, float y, float z, float w)
{
    writer.Write(x);
    writer.Write(y);
    writer.Write(z);
    writer.Write(w);
}

static byte[] AnimationBlob()
{
    UUID id = new UUID("12345678-1234-1234-1234-123456789abc");
    byte[] name = System.Text.Encoding.UTF8.GetBytes("wave");
    byte[] blob = new byte[2 + 16 + 1 + name.Length + 16];
    blob[0] = 2;
    id.GetBytes().CopyTo(blob, 2);
    blob[18] = (byte)name.Length;
    name.CopyTo(blob, 19);
    return blob;
}

static byte[] MaterialBlob()
{
    byte[] text = System.Text.Encoding.UTF8.GetBytes("face-data");
    byte[] blob = new byte[4 + text.Length];
    blob[0] = 1;
    blob[1] = 1;
    blob[2] = (byte)text.Length;
    blob[3] = 0;
    text.CopyTo(blob, 4);
    return blob;
}

static void Load(string path, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    string full = Path.GetFullPath(path);
    Dictionary<string, RawPrim> prims = ReadPrimRaw(full);
    Dictionary<string, RawShape> shapes = ReadShapeRaw(full);

    List<SceneObjectGroup> groups;
    SQLiteSimulationData store = new SQLiteSimulationData();
    try
    {
        store.Initialise("Data Source=" + full + ";Version=3;");
        groups = store.LoadObjects(region);
    }
    finally
    {
        store.Dispose();
    }
    WriteDump(groups, prims, shapes, region, pin);
}

static void LoadMysql(string ado, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    Dictionary<string, RawPrim> prims = ReadPrimRawMysql(ado);
    Dictionary<string, RawShape> shapes = ReadShapeRawMysql(ado);
    List<SceneObjectGroup> groups;
    MySQLSimulationData store = new MySQLSimulationData();
    try
    {
        store.Initialise(ado);
        groups = store.LoadObjects(region);
    }
    finally
    {
        store.Dispose();
    }
    WriteDump(groups, prims, shapes, region, pin);
}

static void WriteDump(
    List<SceneObjectGroup> groups,
    Dictionary<string, RawPrim> prims,
    Dictionary<string, RawShape> shapes,
    UUID region,
    string pin)
{
    Dump root = new Dump
    {
        pin = pin,
        region = region.ToString(),
        groups = new List<DumpGroup>()
    };
    foreach (SceneObjectGroup group in groups)
    {
        DumpGroup dumped = new DumpGroup
        {
            rez_parameter = group.RezStringParameter,
            parts = new List<DumpPart>()
        };
        foreach (SceneObjectPart part in group.Parts)
        {
            string id = part.UUID.ToString();
            prims.TryGetValue(id, out RawPrim raw);
            shapes.TryGetValue(id, out RawShape shapeRaw);
            dumped.parts.Add(DumpPart.From(part, raw, shapeRaw));
        }
        root.groups.Add(dumped);
    }

    JsonSerializerOptions options = new JsonSerializerOptions { WriteIndented = true };
    Console.Out.Write(JsonSerializer.Serialize(root, options));
    Console.Out.Write('\n');
}

static Dictionary<string, RawPrim> ReadPrimRaw(string path)
{
    Dictionary<string, RawPrim> rows = new Dictionary<string, RawPrim>();
    using SQLiteConnection db = new SQLiteConnection("Data Source=" + path + ";Version=3;");
    db.Open();
    using SQLiteCommand command = db.CreateCommand();
    command.CommandText = "SELECT UUID, DynAttrs, KeyframeMotion, Vehicle, PhysInertia, pseudocrc, sopanims, linksetdata FROM prims ORDER BY rowid";
    using SQLiteDataReader reader = command.ExecuteReader();
    while (reader.Read())
    {
        string uuid = reader.GetString(0);
        rows[uuid] = new RawPrim
        {
            DynAttrs = reader.IsDBNull(1) ? null : reader.GetString(1),
            Keyframe = reader.IsDBNull(2) ? null : (byte[])reader.GetValue(2),
            Vehicle = reader.IsDBNull(3) ? null : reader.GetString(3),
            Inertia = reader.IsDBNull(4) ? null : reader.GetString(4),
            PseudoCrc = reader.IsDBNull(5) ? 0 : reader.GetInt32(5),
            Animations = reader.IsDBNull(6) ? null : (byte[])reader.GetValue(6),
            Linkset = reader.IsDBNull(7) ? null : reader.GetString(7)
        };
    }
    return rows;
}

static Dictionary<string, RawShape> ReadShapeRaw(string path)
{
    Dictionary<string, RawShape> rows = new Dictionary<string, RawShape>();
    using SQLiteConnection db = new SQLiteConnection("Data Source=" + path + ";Version=3;");
    db.Open();
    using SQLiteCommand command = db.CreateCommand();
    command.CommandText = "SELECT UUID, Media, MatOvrd, Texture, ExtraParams FROM primshapes ORDER BY rowid";
    using SQLiteDataReader reader = command.ExecuteReader();
    while (reader.Read())
    {
        string uuid = reader.GetString(0);
        rows[uuid] = new RawShape
        {
            Media = reader.IsDBNull(1) ? null : reader.GetString(1),
            MatOvrd = reader.IsDBNull(2) ? null : (byte[])reader.GetValue(2),
            Texture = reader.IsDBNull(3) ? null : (byte[])reader.GetValue(3),
            Extra = reader.IsDBNull(4) ? null : (byte[])reader.GetValue(4)
        };
    }
    return rows;
}

static Dictionary<string, RawPrim> ReadPrimRawMysql(string ado)
{
    Dictionary<string, RawPrim> rows = new Dictionary<string, RawPrim>();
    using MySqlConnection db = new MySqlConnection(ado);
    db.Open();
    using MySqlCommand command = db.CreateCommand();
    command.CommandText = "SELECT UUID, DynAttrs, KeyframeMotion, Vehicle, PhysInertia, pseudocrc, sopanims, linksetdata FROM prims";
    using MySqlDataReader reader = command.ExecuteReader();
    while (reader.Read())
    {
        string uuid = CellText(reader, 0);
        rows[uuid] = new RawPrim
        {
            DynAttrs = reader.IsDBNull(1) ? null : reader.GetString(1),
            Keyframe = reader.IsDBNull(2) ? null : (byte[])reader.GetValue(2),
            Vehicle = reader.IsDBNull(3) ? null : reader.GetString(3),
            Inertia = reader.IsDBNull(4) ? null : reader.GetString(4),
            PseudoCrc = reader.IsDBNull(5) ? 0 : reader.GetInt32(5),
            Animations = reader.IsDBNull(6) ? null : (byte[])reader.GetValue(6),
            Linkset = reader.IsDBNull(7) ? null : reader.GetString(7)
        };
    }
    return rows;
}

static Dictionary<string, RawShape> ReadShapeRawMysql(string ado)
{
    Dictionary<string, RawShape> rows = new Dictionary<string, RawShape>();
    using MySqlConnection db = new MySqlConnection(ado);
    db.Open();
    using MySqlCommand command = db.CreateCommand();
    command.CommandText = "SELECT UUID, Media, MatOvrd, Texture, ExtraParams FROM primshapes";
    using MySqlDataReader reader = command.ExecuteReader();
    while (reader.Read())
    {
        string uuid = CellText(reader, 0);
        rows[uuid] = new RawShape
        {
            Media = reader.IsDBNull(1) ? null : reader.GetString(1),
            MatOvrd = reader.IsDBNull(2) ? null : (byte[])reader.GetValue(2),
            Texture = reader.IsDBNull(3) ? null : (byte[])reader.GetValue(3),
            Extra = reader.IsDBNull(4) ? null : (byte[])reader.GetValue(4)
        };
    }
    return rows;
}

static string CellText(MySqlDataReader reader, int index)
{
    object value = reader.GetValue(index);
    if (value is Guid guid)
        return guid.ToString();
    return value.ToString();
}

sealed class RawPrim
{
    public string DynAttrs;
    public byte[] Keyframe;
    public string Vehicle;
    public string Inertia;
    public int PseudoCrc;
    public byte[] Animations;
    public string Linkset;
}

sealed class RawShape
{
    public string Media;
    public byte[] MatOvrd;
    public byte[] Texture;
    public byte[] Extra;
}

sealed class Dump
{
    public string pin { get; set; }
    public string region { get; set; }
    public List<DumpGroup> groups { get; set; }
}

sealed class DumpGroup
{
    public string rez_parameter { get; set; }
    public List<DumpPart> parts { get; set; }
}

sealed class DumpPart
{
    public string uuid { get; set; }
    public int link_num { get; set; }
    public uint local_id { get; set; }
    public uint parent_id { get; set; }
    public int creation_date { get; set; }
    public string name { get; set; }
    public string text { get; set; }
    public uint color { get; set; }
    public string description { get; set; }
    public string sit_name { get; set; }
    public string touch_name { get; set; }
    public uint flags { get; set; }
    public string creator_id { get; set; }
    public string owner_id { get; set; }
    public string group_id { get; set; }
    public string last_owner_id { get; set; }
    public string rezzer_id { get; set; }
    public uint owner_mask { get; set; }
    public uint next_owner_mask { get; set; }
    public uint group_mask { get; set; }
    public uint everyone_mask { get; set; }
    public uint base_mask { get; set; }
    public float[] offset { get; set; }
    public float[] group_position { get; set; }
    public float[] velocity { get; set; }
    public float[] angular_velocity { get; set; }
    public float[] acceleration { get; set; }
    public float[] rotation { get; set; }
    public float[] sit_target_position { get; set; }
    public float[] sit_target_rotation { get; set; }
    public bool sit_target_active { get; set; }
    public float[] stand_offset { get; set; }
    public float sit_active_range { get; set; }
    public int click_action { get; set; }
    public int[] pay_price { get; set; }
    public string sound { get; set; }
    public float sound_gain { get; set; }
    public int sound_flags { get; set; }
    public string texture_animation { get; set; }
    public string particles { get; set; }
    public float[] camera_eye { get; set; }
    public float[] camera_at { get; set; }
    public bool force_mouselook { get; set; }
    public int script_access_pin { get; set; }
    public bool allowed_drop { get; set; }
    public bool die_at_edge { get; set; }
    public int sale_price { get; set; }
    public int sale_type { get; set; }
    public int material { get; set; }
    public string collision_sound { get; set; }
    public float collision_sound_volume { get; set; }
    public bool volume_detect { get; set; }
    public string media_url { get; set; }
    public float[] attached_pos { get; set; }
    public int physics_shape_type { get; set; }
    public float density { get; set; }
    public float gravity_modifier { get; set; }
    public float friction { get; set; }
    public float restitution { get; set; }
    public bool pass_collisions { get; set; }
    public bool pass_touches { get; set; }
    public int rotation_axis_locks { get; set; }
    public int? pseudo_crc { get; set; }
    public DumpShape shape { get; set; }
    public JsonNode dyn_attrs { get; set; }
    public DumpMotion keyframe { get; set; }
    public DumpVehicle vehicle { get; set; }
    public DumpInertia phys_inertia { get; set; }
    public List<DumpAnimation> animations { get; set; }
    public DumpLinkset linkset { get; set; }
    public List<DumpItem> items { get; set; }
    public DumpRaw raw { get; set; }

    public static DumpPart From(SceneObjectPart part, RawPrim raw, RawShape storedShape)
    {
        bool defaultBox = storedShape == null;
        PrimitiveBaseShape shape = part.Shape;
        return new DumpPart
        {
            uuid = part.UUID.ToString(),
            link_num = part.LinkNum,
            local_id = part.LocalId,
            parent_id = part.ParentID,
            creation_date = part.CreationDate,
            name = part.Name,
            text = part.Text,
            color = part.Color,
            description = part.Description,
            sit_name = part.SitName,
            touch_name = part.TouchName,
            flags = (uint)part.Flags,
            creator_id = part.CreatorIdentification,
            owner_id = part.OwnerID.ToString(),
            group_id = part.GroupID.ToString(),
            last_owner_id = part.LastOwnerID.ToString(),
            rezzer_id = part.RezzerID.ToString(),
            owner_mask = part.OwnerMask,
            next_owner_mask = part.NextOwnerMask,
            group_mask = part.GroupMask,
            everyone_mask = part.EveryoneMask,
            base_mask = part.BaseMask,
            offset = Vec(part.OffsetPosition),
            group_position = Vec(part.GroupPosition),
            velocity = Vec(part.Velocity),
            angular_velocity = Vec(part.AngularVelocity),
            acceleration = Vec(part.Acceleration),
            rotation = Quat(part.RotationOffset),
            sit_target_position = Vec(part.SitTargetPositionLL),
            sit_target_rotation = Quat(part.SitTargetOrientationLL),
            sit_target_active = part.SitTargetActive,
            stand_offset = Vec(part.StandOffset),
            sit_active_range = part.SitActiveRange,
            click_action = part.ClickAction,
            pay_price = (int[])part.PayPrice.Clone(),
            sound = part.Sound.ToString(),
            sound_gain = (float)part.SoundGain,
            sound_flags = part.SoundFlags,
            texture_animation = Hex(part.TextureAnimation),
            particles = Hex(part.ParticleSystem),
            camera_eye = Vec(part.GetCameraEyeOffset()),
            camera_at = Vec(part.GetCameraAtOffset()),
            force_mouselook = part.GetForceMouselook(),
            script_access_pin = part.ScriptAccessPin,
            allowed_drop = part.AllowedDrop,
            die_at_edge = part.DIE_AT_EDGE,
            sale_price = part.SalePrice,
            sale_type = part.ObjectSaleType,
            material = part.Material,
            collision_sound = part.CollisionSound.ToString(),
            collision_sound_volume = part.CollisionSoundVolume,
            volume_detect = part.VolumeDetectActive,
            media_url = part.MediaUrl,
            attached_pos = Vec(part.AttachedPos),
            physics_shape_type = part.PhysicsShapeType,
            density = part.Density,
            gravity_modifier = part.GravityModifier,
            friction = part.Friction,
            restitution = part.Restitution,
            pass_collisions = part.PassCollisions,
            pass_touches = part.PassTouches,
            rotation_axis_locks = part.RotationAxisLocks,
            pseudo_crc = raw != null && raw.PseudoCrc != 0 ? part.PseudoCRC : null,
            shape = new DumpShape
            {
                default_box = defaultBox,
                scale = Vec(shape.Scale),
                pcode = shape.PCode,
                path_begin = shape.PathBegin,
                path_end = shape.PathEnd,
                path_scale_x = shape.PathScaleX,
                path_scale_y = shape.PathScaleY,
                path_shear_x = shape.PathShearX,
                path_shear_y = shape.PathShearY,
                path_skew = shape.PathSkew,
                path_curve = shape.PathCurve,
                path_radius_offset = shape.PathRadiusOffset,
                path_revolutions = shape.PathRevolutions,
                path_taper_x = shape.PathTaperX,
                path_taper_y = shape.PathTaperY,
                path_twist = shape.PathTwist,
                path_twist_begin = shape.PathTwistBegin,
                profile_begin = shape.ProfileBegin,
                profile_end = shape.ProfileEnd,
                profile_curve = shape.ProfileCurve,
                profile_hollow = shape.ProfileHollow,
                state = shape.State,
                last_attach_point = shape.LastAttachPoint,
                texture = Hex(defaultBox ? shape.TextureEntry : storedShape.Texture),
                extra_params = Hex(defaultBox ? shape.ExtraParams : storedShape.Extra),
                media_entries = MediaOf(shape.Media),
                materials = MaterialsOf(shape.RenderMaterials)
            },
            dyn_attrs = DynOf(part.DynAttrs),
            keyframe = MotionOf(part.KeyframeMotion),
            vehicle = VehicleOf(part.VehicleParams),
            phys_inertia = InertiaOf(part.PhysicsInertia),
            animations = AnimsOf(part),
            linkset = LinksetOf(part.LinksetData),
            items = DumpItems(part),
            raw = new DumpRaw
            {
                media = storedShape == null ? null : storedShape.Media,
                mat_ovrd = storedShape == null ? null : Hex(storedShape.MatOvrd),
                dyn_attrs = raw == null ? null : raw.DynAttrs,
                keyframe_motion = raw == null ? null : Hex(raw.Keyframe),
                vehicle = raw == null ? null : raw.Vehicle,
                phys_inertia = raw == null ? null : raw.Inertia,
                sopanims = raw == null ? null : Hex(raw.Animations),
                linkset_data = raw == null ? null : raw.Linkset
            }
        };
    }

    static List<DumpItem> DumpItems(SceneObjectPart part)
    {
        List<DumpItem> items = new List<DumpItem>();
        foreach (TaskInventoryItem item in part.TaskInventory.Values)
            items.Add(DumpItem.From(item));
        return items;
    }

    public static string Hex(byte[] bytes)
    {
        if (bytes == null)
            return null;
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }

    public static float[] Vec(Vector3 value)
    {
        return new[] { value.X, value.Y, value.Z };
    }

    public static float[] Quat(Quaternion value)
    {
        return new[] { value.X, value.Y, value.Z, value.W };
    }

    static JsonNode DynOf(DAMap map)
    {
        if (map == null || map.TopLevelMap == null)
            return null;
        return OsdJson(map.TopLevelMap);
    }

    static JsonNode OsdJson(OSD value)
    {
        if (value == null)
            return null;
        switch (value.Type)
        {
            case OSDType.Boolean:
                return JsonValue.Create(value.AsBoolean());
            case OSDType.Integer:
                return JsonValue.Create(value.AsInteger());
            case OSDType.Real:
                return JsonValue.Create(value.AsReal());
            case OSDType.String:
            case OSDType.UUID:
            case OSDType.URI:
            case OSDType.Date:
            case OSDType.Binary:
                return JsonValue.Create(value.AsString());
            case OSDType.Array:
                JsonArray array = new JsonArray();
                foreach (OSD child in (OSDArray)value)
                    array.Add(OsdJson(child));
                return array;
            case OSDType.Map:
                JsonObject obj = new JsonObject();
                foreach (KeyValuePair<string, OSD> entry in (OSDMap)value)
                    obj[entry.Key] = OsdJson(entry.Value);
                return obj;
            default:
                return null;
        }
    }

    static object Private(object target, string name)
    {
        FieldInfo field = target.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic);
        if (field == null)
            throw new InvalidOperationException("missing " + name);
        return field.GetValue(target);
    }

    static DumpMotion MotionOf(KeyframeMotion motion)
    {
        if (motion == null)
            return null;
        return new DumpMotion
        {
            serialized_position = Vec((Vector3)Private(motion, "m_serializedPosition")),
            base_position = Vec((Vector3)Private(motion, "m_basePosition")),
            base_rotation = Quat((Quaternion)Private(motion, "m_baseRotation")),
            current_frame = OneFrame((KeyframeMotion.Keyframe)Private(motion, "m_currentFrame")),
            frames = FrameList(Private(motion, "m_frames")),
            keyframes = FrameList(Private(motion, "m_keyframes")),
            mode = Convert.ToInt32(Private(motion, "m_mode")),
            data = Convert.ToInt32(Private(motion, "m_data")),
            running = (bool)Private(motion, "m_running"),
            iterations = (int)Private(motion, "m_iterations"),
            skip_loops = (int)Private(motion, "m_skipLoops")
        };
    }

    static List<DumpKeyframe> FrameList(object value)
    {
        if (value == null)
            return null;
        List<DumpKeyframe> frames = new List<DumpKeyframe>();
        if (value is KeyframeMotion.Keyframe[] array)
        {
            foreach (KeyframeMotion.Keyframe frame in array)
                frames.Add(OneFrame(frame));
            return frames;
        }
        foreach (KeyframeMotion.Keyframe frame in (List<KeyframeMotion.Keyframe>)value)
            frames.Add(OneFrame(frame));
        return frames;
    }

    static DumpKeyframe OneFrame(KeyframeMotion.Keyframe frame)
    {
        return new DumpKeyframe
        {
            position = frame.Position.HasValue ? Vec(frame.Position.Value) : null,
            rotation = frame.Rotation.HasValue ? Quat(frame.Rotation.Value) : null,
            start_rotation = Quat(frame.StartRotation),
            time_ms = frame.TimeMS,
            time_total = frame.TimeTotal,
            angular_velocity = Vec(frame.AngularVelocity),
            start_position = Vec(frame.StartPosition)
        };
    }

    static DumpVehicle VehicleOf(SOPVehicle vehicle)
    {
        if (vehicle == null)
            return null;
        VehicleData data = vehicle.vd;
        return new DumpVehicle
        {
            type = (int)data.m_type,
            flags = (int)data.m_flags,
            linear_motor_direction = Vec(data.m_linearMotorDirection),
            linear_friction_timescale = Vec(data.m_linearFrictionTimescale),
            linear_motor_decay_timescale = data.m_linearMotorDecayTimescale,
            linear_motor_timescale = data.m_linearMotorTimescale,
            linear_motor_offset = Vec(data.m_linearMotorOffset),
            angular_motor_direction = Vec(data.m_angularMotorDirection),
            angular_motor_timescale = data.m_angularMotorTimescale,
            angular_motor_decay_timescale = data.m_angularMotorDecayTimescale,
            angular_friction_timescale = Vec(data.m_angularFrictionTimescale),
            angular_deflection_efficiency = data.m_angularDeflectionEfficiency,
            angular_deflection_timescale = data.m_angularDeflectionTimescale,
            linear_deflection_efficiency = data.m_linearDeflectionEfficiency,
            linear_deflection_timescale = data.m_linearDeflectionTimescale,
            banking_efficiency = data.m_bankingEfficiency,
            banking_mix = data.m_bankingMix,
            banking_timescale = data.m_bankingTimescale,
            hover_height = data.m_VhoverHeight,
            hover_efficiency = data.m_VhoverEfficiency,
            hover_timescale = data.m_VhoverTimescale,
            buoyancy = data.m_VehicleBuoyancy,
            vertical_attraction_efficiency = data.m_verticalAttractionEfficiency,
            vertical_attraction_timescale = data.m_verticalAttractionTimescale,
            reference_frame = Quat(data.m_referenceFrame),
            linear_motor_timescale_axes = OptVec(data.m_linearMotorTimescaleAxes),
            linear_motor_decay_timescale_axes = OptVec(data.m_linearMotorDecayTimescaleAxes),
            angular_motor_timescale_axes = OptVec(data.m_angularMotorTimescaleAxes),
            angular_motor_decay_timescale_axes = OptVec(data.m_angularMotorDecayTimescaleAxes),
            mouselook_azimuth = data.m_mouselookAzimuth,
            mouselook_altitude = data.m_mouselookAltitude,
            banking_azimuth = data.m_bankingAzimuth,
            disable_motors_above = data.m_disableMotorsAbove,
            disable_motors_after = data.m_disableMotorsAfter,
            inverted_banking_modifier = data.m_invertedBankingModifier,
            linear_wind_efficiency = OptVec(data.m_linearWindEfficiency),
            angular_wind_efficiency = OptVec(data.m_angularWindEfficiency),
            linear_motor_run_age = data.m_linearMotorRunAge,
            linear_motor_run_direction = OptVec(data.m_linearMotorRunDirection),
            linear_motor_run_target = OptVec(data.m_linearMotorRunTarget),
            angular_motor_run_age = data.m_angularMotorRunAge,
            angular_motor_run_direction = OptVec(data.m_angularMotorRunDirection),
            angular_motor_run_target = OptVec(data.m_angularMotorRunTarget)
        };
    }

    static float[] OptVec(Vector3? value)
    {
        return value.HasValue ? Vec(value.Value) : null;
    }

    static DumpInertia InertiaOf(PhysicsInertiaData data)
    {
        if (data == null)
            return null;
        return new DumpInertia
        {
            mass = data.TotalMass,
            center = Vec(data.CenterOfMass),
            inertia = Vec(data.Inertia),
            rotation = new[] { data.InertiaRotation.X, data.InertiaRotation.Y, data.InertiaRotation.Z, data.InertiaRotation.W }
        };
    }

    static List<DumpAnimation> AnimsOf(SceneObjectPart part)
    {
        if (part.AnimationsNames == null)
            return null;
        List<DumpAnimation> list = new List<DumpAnimation>();
        foreach (KeyValuePair<UUID, string> entry in part.AnimationsNames)
            list.Add(new DumpAnimation { id = entry.Key.ToString(), name = entry.Value });
        return list;
    }

    static DumpLinkset LinksetOf(LinksetData data)
    {
        if (data == null)
            return null;
        List<DumpLinksetEntry> entries = null;
        if (data.Data != null)
        {
            entries = new List<DumpLinksetEntry>();
            foreach (KeyValuePair<string, LinksetDataEntry> entry in data.Data)
            {
                entries.Add(new DumpLinksetEntry
                {
                    key = entry.Key,
                    value = entry.Value == null ? null : entry.Value.Value,
                    password = entry.Value == null ? null : entry.Value.Password
                });
            }
        }
        return new DumpLinkset
        {
            entries = entries,
            bytes_used = data.BytesUsed,
            bytes_free = data.BytesFree
        };
    }

    static List<DumpMedia> MediaOf(PrimitiveBaseShape.MediaList media)
    {
        if (media == null)
            return null;
        List<DumpMedia> list = new List<DumpMedia>();
        foreach (MediaEntry entry in media)
        {
            list.Add(new DumpMedia
            {
                alt_image_enable = entry.EnableAlterntiveImage,
                auto_loop = entry.AutoLoop,
                auto_play = entry.AutoPlay,
                auto_scale = entry.AutoScale,
                auto_zoom = entry.AutoZoom,
                controls = (int)entry.Controls,
                current_url = entry.CurrentURL,
                first_click_interact = entry.InteractOnFirstClick,
                height_pixels = entry.Height,
                home_url = entry.HomeURL,
                perms_control = (int)entry.ControlPermissions,
                perms_interact = (int)entry.InteractPermissions,
                whitelist = entry.WhiteList,
                whitelist_enable = entry.EnableWhiteList,
                width_pixels = entry.Width
            });
        }
        return list;
    }

    static DumpMaterials MaterialsOf(Primitive.RenderMaterials materials)
    {
        if (materials == null)
            return null;
        List<DumpMaterialOverride> overrides = null;
        if (materials.overrides != null)
        {
            overrides = new List<DumpMaterialOverride>();
            foreach (Primitive.RenderMaterials.RenderMaterialOverrideEntry entry in materials.overrides)
                overrides.Add(new DumpMaterialOverride { te_index = entry.te_index, data = entry.data });
        }
        return new DumpMaterials { overrides = overrides };
    }
}

sealed class DumpItem
{
    public string item_id { get; set; }
    public string parent_part_id { get; set; }
    public string asset_id { get; set; }
    public string parent_id { get; set; }
    public int inv_type { get; set; }
    public int asset_type { get; set; }
    public string name { get; set; }
    public string description { get; set; }
    public uint creation_date { get; set; }
    public string creator_id { get; set; }
    public string owner_id { get; set; }
    public string last_owner_id { get; set; }
    public string group_id { get; set; }
    public uint next_permissions { get; set; }
    public uint current_permissions { get; set; }
    public uint base_permissions { get; set; }
    public uint everyone_permissions { get; set; }
    public uint group_permissions { get; set; }
    public uint flags { get; set; }
    public string experience_id { get; set; }

    public static DumpItem From(TaskInventoryItem item)
    {
        return new DumpItem
        {
            item_id = item.ItemID.ToString(),
            parent_part_id = item.ParentPartID.ToString(),
            asset_id = item.AssetID.ToString(),
            parent_id = item.ParentID.ToString(),
            inv_type = item.InvType,
            asset_type = item.Type,
            name = item.Name,
            description = item.Description,
            creation_date = item.CreationDate,
            creator_id = item.CreatorIdentification,
            owner_id = item.OwnerID.ToString(),
            last_owner_id = item.LastOwnerID.ToString(),
            group_id = item.GroupID.ToString(),
            next_permissions = item.NextPermissions,
            current_permissions = item.CurrentPermissions,
            base_permissions = item.BasePermissions,
            everyone_permissions = item.EveryonePermissions,
            group_permissions = item.GroupPermissions,
            flags = item.Flags,
            experience_id = item.ExperienceID.ToString()
        };
    }
}

sealed class DumpShape
{
    public bool default_box { get; set; }
    public float[] scale { get; set; }
    public int pcode { get; set; }
    public int path_begin { get; set; }
    public int path_end { get; set; }
    public int path_scale_x { get; set; }
    public int path_scale_y { get; set; }
    public int path_shear_x { get; set; }
    public int path_shear_y { get; set; }
    public int path_skew { get; set; }
    public int path_curve { get; set; }
    public int path_radius_offset { get; set; }
    public int path_revolutions { get; set; }
    public int path_taper_x { get; set; }
    public int path_taper_y { get; set; }
    public int path_twist { get; set; }
    public int path_twist_begin { get; set; }
    public int profile_begin { get; set; }
    public int profile_end { get; set; }
    public int profile_curve { get; set; }
    public int profile_hollow { get; set; }
    public int state { get; set; }
    public int last_attach_point { get; set; }
    public string texture { get; set; }
    public string extra_params { get; set; }
    public List<DumpMedia> media_entries { get; set; }
    public DumpMaterials materials { get; set; }
}

sealed class DumpMotion
{
    public float[] serialized_position { get; set; }
    public float[] base_position { get; set; }
    public float[] base_rotation { get; set; }
    public DumpKeyframe current_frame { get; set; }
    public List<DumpKeyframe> frames { get; set; }
    public List<DumpKeyframe> keyframes { get; set; }
    public int mode { get; set; }
    public int data { get; set; }
    public bool running { get; set; }
    public int iterations { get; set; }
    public int skip_loops { get; set; }
}

sealed class DumpKeyframe
{
    public float[] position { get; set; }
    public float[] rotation { get; set; }
    public float[] start_rotation { get; set; }
    public int time_ms { get; set; }
    public int time_total { get; set; }
    public float[] angular_velocity { get; set; }
    public float[] start_position { get; set; }
}

sealed class DumpVehicle
{
    public int type { get; set; }
    public int flags { get; set; }
    public float[] linear_motor_direction { get; set; }
    public float[] linear_friction_timescale { get; set; }
    public float linear_motor_decay_timescale { get; set; }
    public float linear_motor_timescale { get; set; }
    public float[] linear_motor_offset { get; set; }
    public float[] angular_motor_direction { get; set; }
    public float angular_motor_timescale { get; set; }
    public float angular_motor_decay_timescale { get; set; }
    public float[] angular_friction_timescale { get; set; }
    public float angular_deflection_efficiency { get; set; }
    public float angular_deflection_timescale { get; set; }
    public float linear_deflection_efficiency { get; set; }
    public float linear_deflection_timescale { get; set; }
    public float banking_efficiency { get; set; }
    public float banking_mix { get; set; }
    public float banking_timescale { get; set; }
    public float hover_height { get; set; }
    public float hover_efficiency { get; set; }
    public float hover_timescale { get; set; }
    public float buoyancy { get; set; }
    public float vertical_attraction_efficiency { get; set; }
    public float vertical_attraction_timescale { get; set; }
    public float[] reference_frame { get; set; }
    public float[] linear_motor_timescale_axes { get; set; }
    public float[] linear_motor_decay_timescale_axes { get; set; }
    public float[] angular_motor_timescale_axes { get; set; }
    public float[] angular_motor_decay_timescale_axes { get; set; }
    public float mouselook_azimuth { get; set; }
    public float mouselook_altitude { get; set; }
    public float banking_azimuth { get; set; }
    public float disable_motors_above { get; set; }
    public float disable_motors_after { get; set; }
    public float inverted_banking_modifier { get; set; }
    public float[] linear_wind_efficiency { get; set; }
    public float[] angular_wind_efficiency { get; set; }
    public float? linear_motor_run_age { get; set; }
    public float[] linear_motor_run_direction { get; set; }
    public float[] linear_motor_run_target { get; set; }
    public float? angular_motor_run_age { get; set; }
    public float[] angular_motor_run_direction { get; set; }
    public float[] angular_motor_run_target { get; set; }
}

sealed class DumpInertia
{
    public float mass { get; set; }
    public float[] center { get; set; }
    public float[] inertia { get; set; }
    public float[] rotation { get; set; }
}

sealed class DumpAnimation
{
    public string id { get; set; }
    public string name { get; set; }
}

sealed class DumpLinkset
{
    public List<DumpLinksetEntry> entries { get; set; }
    public int bytes_used { get; set; }
    public int bytes_free { get; set; }
}

sealed class DumpLinksetEntry
{
    public string key { get; set; }
    public string value { get; set; }
    public string password { get; set; }
}

sealed class DumpMedia
{
    public bool alt_image_enable { get; set; }
    public bool auto_loop { get; set; }
    public bool auto_play { get; set; }
    public bool auto_scale { get; set; }
    public bool auto_zoom { get; set; }
    public int controls { get; set; }
    public string current_url { get; set; }
    public bool first_click_interact { get; set; }
    public int height_pixels { get; set; }
    public string home_url { get; set; }
    public int perms_control { get; set; }
    public int perms_interact { get; set; }
    public string[] whitelist { get; set; }
    public bool whitelist_enable { get; set; }
    public int width_pixels { get; set; }
}

sealed class DumpMaterials
{
    public List<DumpMaterialOverride> overrides { get; set; }
}

sealed class DumpMaterialOverride
{
    public int te_index { get; set; }
    public string data { get; set; }
}

sealed class DumpRaw
{
    public string media { get; set; }
    public string mat_ovrd { get; set; }
    public string dyn_attrs { get; set; }
    public string keyframe_motion { get; set; }
    public string vehicle { get; set; }
    public string phys_inertia { get; set; }
    public string sopanims { get; set; }
    public string linkset_data { get; set; }
}

sealed class Row
{
    readonly List<(string Name, object Value)> cells = new List<(string, object)>();

    public Row Set(string name, object value)
    {
        cells.Add((name, value));
        return this;
    }

    public void Insert(DbConnection db, string table)
    {
        bool mysql = db is MySqlConnection;
        // A later Set for the same column replaces the earlier value. MySQL rejects
        // two parameters of one name; the stored row is the last value either way.
        List<string> names = new List<string>();
        Dictionary<string, object> values = new Dictionary<string, object>();
        foreach ((string name, object value) in cells)
        {
            if (!values.ContainsKey(name))
                names.Add(name);
            values[name] = value;
        }
        string columns = string.Join(", ", names);
        string marks = string.Join(", ", names.ConvertAll(name => "@" + name));
        using DbCommand command = db.CreateCommand();
        command.CommandText = "INSERT INTO " + table + " (" + columns + ") VALUES (" + marks + ")";
        foreach (string name in names)
        {
            object stored = values[name] ?? DBNull.Value;
            if (mysql && stored is string text && (name == "TextureAnimation" || name == "ParticleSystem"))
                stored = Convert.FromBase64String(text);
            DbParameter parameter = command.CreateParameter();
            parameter.ParameterName = "@" + name;
            parameter.Value = stored;
            command.Parameters.Add(parameter);
        }
        command.ExecuteNonQuery();
    }
}
