// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Data;
using System.Data.Common;
using System.Data.SQLite;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using MySqlConnector;
using Npgsql;
using OpenMetaverse;
using OpenMetaverse.StructuredData;
using OpenSim.Data.MySQL;
using OpenSim.Data.PGSQL;
using OpenSim.Data.SQLite;
using OpenSim.Framework;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Region.Framework.Scenes.Serialization;
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
const string RewriteGroup = "88888888-8888-8888-8888-888888888880";
const string RewriteRoot = "88888888-8888-8888-8888-888888888881";
const string RewriteChild = "88888888-8888-8888-8888-888888888882";

if (args.Length == 0)
{
    Console.Error.WriteLine("usage: regionstore-oracle seed <db> | prim-seed <db> <region-uuid> <owner-uuid> | prim-load-seed <db> <region-uuid> <owner-uuid> | load <db> <region-uuid> <pin> | bulk <db> <region-uuid> <count> | mysql-seed <ado> | mysql-load <ado> <region-uuid> <pin> | postgres-seed <ado> | postgres-load <ado> <region-uuid> <pin> | settings-seed <db> | settings-load <db> <region-uuid> <pin> | serialize <db> <region-uuid> <pin> | read <xml-json> <pin> | coalesced <xml-dump> <pin> | probe <probe-json> <pin> | parcels <scratch-db> <seed-sql> <region-uuid> <pin> | terrain-load <db> <region-uuid> <pin> | mysql-terrain-load <ado> <region-uuid> <pin> | postgres-terrain-load <ado> <region-uuid> <pin> | mysql-terrain-seed <ado> <region-uuid> | postgres-terrain-seed <ado> <region-uuid> | environment-text <pin> | environment-load <db> <region-uuid> <pin> | environment-store <db> <region-uuid> <pin> | environment-remove <db> <region-uuid> <pin> | mysql-environment-load <ado> <region-uuid> <pin> | mysql-environment-store <ado> <region-uuid> <pin> | mysql-environment-remove <ado> <region-uuid> <pin> | postgres-environment-load <ado> <region-uuid> <pin> | postgres-environment-store <ado> <region-uuid> <pin> | postgres-environment-remove <ado> <region-uuid> <pin> | scales <pin>");
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
        case "prim-seed" or "prim-load-seed" when args.Length == 4:
            SeedPrimUpdates(args[1], new UUID(args[2]), new UUID(args[3]), args[0] == "prim-load-seed");
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
        case "postgres-seed" when args.Length == 2:
            SeedPostgres(args[1]);
            return 0;
        case "postgres-load" when args.Length == 4:
            LoadPostgres(args[1], args[2], args[3]);
            return 0;
        case "parcels" when args.Length == 5:
            LoadParcels(args[1], args[2], args[3], args[4]);
            return 0;
        case "settings-seed" when args.Length == 2:
            SeedSettings(args[1]);
            return 0;
        case "settings-load" when args.Length == 4:
            LoadSettings(args[1], args[2], args[3]);
            return 0;
        case "serialize" when args.Length == 4:
            Serialize(args[1], args[2], args[3]);
            return 0;
        case "read" when args.Length == 3:
            ReadXml(args[1], args[2]);
            return 0;
        case "coalesced" when args.Length == 3:
            CaptureCoalesced(args[1], args[2]);
            return 0;
        case "probe" when args.Length == 3:
            ProbeXml(args[1], args[2]);
            return 0;
        case "terrain-load" when args.Length == 4:
            LoadTerrainRows(args[1], args[2], args[3]);
            return 0;
        case "mysql-terrain-load" when args.Length == 4:
            DumpTerrain(new MySQLSimulationData(), args[1], args[2], args[3],
                table => RawTerrainRowDb(new MySqlConnection(args[1]), table, args[2], "?RegionUUID", "RegionUUID", "Revision", "Heightfield", new UUID(args[2]).ToString()));
            return 0;
        case "postgres-terrain-load" when args.Length == 4:
            DumpTerrain(new PGSQLSimulationData(), args[1], args[2], args[3],
                table => RawTerrainRowDb(new NpgsqlConnection(args[1]), table, args[2], ":RegionUUID", "\"RegionUUID\"", "\"Revision\"", "\"Heightfield\"", new Guid(args[2])));
            return 0;
        case "mysql-terrain-seed" when args.Length == 3:
            SeedTerrain(new MySQLSimulationData(), args[1], args[2]);
            return 0;
        case "postgres-terrain-seed" when args.Length == 3:
            SeedTerrain(new PGSQLSimulationData(), args[1], args[2]);
            return 0;
        case "environment-text" when args.Length == 2:
            WriteEnvironment(args[1], null, SampleEnvironment());
            return 0;
        case "environment-load" when args.Length == 4:
            EnvironmentLoad(new SQLiteSimulationData(), "Data Source=" + Path.GetFullPath(args[1]) + ";Version=3;", args[2], args[3]);
            return 0;
        case "environment-store" when args.Length == 4:
            EnvironmentStore(new SQLiteSimulationData(), "Data Source=" + Path.GetFullPath(args[1]) + ";Version=3;", args[2], args[3]);
            return 0;
        case "environment-remove" when args.Length == 4:
            EnvironmentRemove(new SQLiteSimulationData(), "Data Source=" + Path.GetFullPath(args[1]) + ";Version=3;", args[2], args[3]);
            return 0;
        case "mysql-environment-load" when args.Length == 4:
            EnvironmentLoad(new MySQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "mysql-environment-store" when args.Length == 4:
            EnvironmentStore(new MySQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "mysql-environment-remove" when args.Length == 4:
            EnvironmentRemove(new MySQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "postgres-environment-load" when args.Length == 4:
            EnvironmentLoad(new PGSQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "postgres-environment-store" when args.Length == 4:
            EnvironmentStore(new PGSQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "postgres-environment-remove" when args.Length == 4:
            EnvironmentRemove(new PGSQLSimulationData(), args[1], args[2], args[3]);
            return 0;
        case "scales" when args.Length == 2:
            ScaleProbe.Run(args[1]);
            return 0;
        default:
            Console.Error.WriteLine("usage: regionstore-oracle seed <db> | prim-seed <db> <region-uuid> <owner-uuid> | prim-load-seed <db> <region-uuid> <owner-uuid> | load <db> <region-uuid> <pin> | bulk <db> <region-uuid> <count> | mysql-seed <ado> | mysql-load <ado> <region-uuid> <pin> | postgres-seed <ado> | postgres-load <ado> <region-uuid> <pin> | settings-seed <db> | settings-load <db> <region-uuid> <pin> | serialize <db> <region-uuid> <pin> | read <xml-json> <pin> | coalesced <xml-dump> <pin> | probe <probe-json> <pin> | parcels <scratch-db> <seed-sql> <region-uuid> <pin> | terrain-load <db> <region-uuid> <pin> | mysql-terrain-load <ado> <region-uuid> <pin> | postgres-terrain-load <ado> <region-uuid> <pin> | mysql-terrain-seed <ado> <region-uuid> | postgres-terrain-seed <ado> <region-uuid> | environment-text <pin> | environment-load <db> <region-uuid> <pin> | environment-store <db> <region-uuid> <pin> | environment-remove <db> <region-uuid> <pin> | mysql-environment-load <ado> <region-uuid> <pin> | mysql-environment-store <ado> <region-uuid> <pin> | mysql-environment-remove <ado> <region-uuid> <pin> | postgres-environment-load <ado> <region-uuid> <pin> | postgres-environment-store <ado> <region-uuid> <pin> | postgres-environment-remove <ado> <region-uuid> <pin> | scales <pin>");
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

// Exercise SQLiteSimulationData.StoreObject at Pin8 c401bceecd. Only synthetic
// objects enter this database. The live C# region opens a disposable copy.
static void SeedPrimUpdates(string path, UUID region, UUID owner, bool loadScene)
{
    if (File.Exists(path))
        throw new IOException("prim-seed needs a fresh database");
    UUID Id(int n) => new UUID($"65000000-0000-4000-8000-{n:x12}");
    SceneObjectPart Part(int n, string name, PrimitiveBaseShape shape, Vector3 position, Vector3 offset)
    {
        return new SceneObjectPart(owner, shape, position, Quaternion.Identity, offset)
        {
            UUID = Id(n), Name = name, CreationDate = 1700000000, PseudoCRC = 6500 + n
        };
    }

    Vector3 position = new Vector3(130, 128, 40);
    PrimitiveBaseShape box = PrimitiveBaseShape.Default;
    var textures = new Primitive.TextureEntry(Id(21));
    textures.CreateFace(1).MaterialID = Id(22);
    box.Textures = textures;
    SceneObjectPart root = Part(1, "box", box, position, Vector3.Zero);
    root.Text = "Pin8 box";
    root.Color = 0xd04080ff;
    root.MediaUrl = "https://example.invalid/prim";
    root.Sound = Id(23);
    root.SoundFlags = 1;
    root.SoundGain = 0.25;
    root.SoundRadius = 12;
    root.ParticleSystem = new Primitive.ParticleSystem { BurstPartCount = 3, PartMaxAge = 2, Texture = Id(24) }.GetBytes();
    root.TextureAnimation = new byte[] { 1, 0, 2, 2, 0, 0, 0, 0, 0, 0, 128, 64, 0, 0, 128, 63 };
    SceneObjectGroup group = new SceneObjectGroup(root);

    PrimitiveBaseShape mesh = PrimitiveBaseShape.CreateMesh(2, Id(25));
    mesh.SetScale(0.75f);
    mesh.ProfileBegin = mesh.ProfileHollow = 1;
    mesh.PathScaleY = 100;
    mesh.RenderMaterials = new Primitive.RenderMaterials {
        entries = new[] { new Primitive.RenderMaterials.RenderMaterialEntry { te_index = 1, id = Id(26) } }
    };
    var child = Part(2, "mesh", mesh, position, new Vector3(1, 2, 0.5f));
    child.RotationOffset = Quaternion.CreateFromAxisAngle(Vector3.UnitZ, 0.5f);
    group.AddPart(child);

    PrimitiveBaseShape tree = PrimitiveBaseShape.Default;
    tree.PCode = (byte)PCode.Tree;
    tree.State = 4;
    tree.SetScale(2);
    var vegetation = new SceneObjectGroup(Part(3, "tree", tree, new Vector3(134, 128, 40), Vector3.Zero));
    SceneObjectGroup RotatedVegetation(int n, PCode code)
    {
        var shape = PrimitiveBaseShape.Default;
        shape.PCode = (byte)code;
        shape.State = 3;
        var part = Part(n, code.ToString(), shape, new Vector3(132 + n, 128, 40), Vector3.Zero);
        part.RotationOffset = Quaternion.CreateFromAxisAngle(Vector3.UnitX, 0.75f);
        return new SceneObjectGroup(part);
    }
    var effects = PrimitiveBaseShape.Default;
    effects.PathTwist = -19;
    effects.PathTwistBegin = 7;
    effects.PathRadiusOffset = -11;
    effects.PathTaperX = -13;
    effects.PathTaperY = -17;
    effects.PathSkew = -23;
    effects.FlexiEntry = true;
    effects.FlexiSoftness = 2;
    effects.FlexiTension = 2.5f;
    effects.FlexiDrag = 1.5f;
    effects.FlexiGravity = -3;
    effects.FlexiWind = 0.5f;
    effects.FlexiForceX = 0.75f;
    effects.LightEntry = true;
    effects.LightColorR = 0.25f;
    effects.LightColorG = 0.5f;
    effects.LightColorB = 0.75f;
    effects.LightIntensity = 0.6f;
    effects.LightRadius = 8;
    effects.LightFalloff = 0.5f;
    effects.LightCutoff = 1.25f;
    effects.ProjectionEntry = true;
    effects.ProjectionTextureUUID = Id(27);
    effects.ProjectionFOV = 1.5f;
    effects.ProjectionFocus = -0.25f;
    effects.ProjectionAmbiance = 0.5f;
    effects.ReflectionProbe = new Primitive.ReflectionProbe { Ambiance = 0.75f, ClipDistance = 3.5f, Flags = 1 };
    var emitter = Part(6, "particle emitter", effects, new Vector3(138, 128, 40), Vector3.Zero);
    emitter.RotationOffset = Quaternion.CreateFromAxisAngle(Vector3.UnitY, -0.75f);
    emitter.Material = (byte)OpenMetaverse.Material.Glass;
    emitter.ClickAction = (byte)OpenMetaverse.ClickAction.Sit;
    emitter.Text = "Lumière 雪";
    emitter.Color = 0xff4080bb;
    emitter.ParticleSystem = new Primitive.ParticleSystem { BurstPartCount = 5, PartMaxAge = 4, Texture = Id(28) }.GetBytes();
    var speaker = Part(7, "looping sound", PrimitiveBaseShape.Default, new Vector3(139, 128, 40), Vector3.Zero);
    speaker.Sound = Id(29);
    speaker.SoundFlags = 1;
    speaker.SoundGain = 0.375;

    var animatedMesh = PrimitiveBaseShape.CreateMesh(1, Id(30));
    animatedMesh.SetScale(0.5f);
    animatedMesh.AnimeshEnabled = true;
    var animated = Part(8, "extended mesh flags", animatedMesh, new Vector3(140, 128, 40), Vector3.Zero);

    string connection = "Data Source=" + Path.GetFullPath(path) + ";Version=3;";
    var store = new SQLiteSimulationData();
    try
    {
        store.Initialise(connection);
        store.StoreObject(group, region);
        store.StoreObject(vegetation, region);
        store.StoreObject(RotatedVegetation(4, PCode.Grass), region);
        store.StoreObject(RotatedVegetation(5, PCode.NewTree), region);
        store.StoreObject(new SceneObjectGroup(emitter), region);
        store.StoreObject(new SceneObjectGroup(speaker), region);
        store.StoreObject(new SceneObjectGroup(animated), region);
        if (loadScene)
        {
            // 8 capture parts plus 246 linked pairs. StoreObject persists both
            // prim and shape rows; every local ID is allocated on scene load.
            for (int n = 9; n <= 500; n += 2)
            {
                int pair = (n - 9) / 2;
                var pos = new Vector3(112 + 2 * (pair % 16), 112 + 2 * (pair / 16), 40 + pair % 5);
                var shape = pair % 2 == 0 ? PrimitiveBaseShape.CreateBox() : PrimitiveBaseShape.CreateSphere();
                shape.Textures = new Primitive.TextureEntry(Id(1000 + n));
                shape.SetScale(0.5f + (pair % 4) * 0.125f);
                var parent = Part(n, "load root", shape, pos, Vector3.Zero);
                if (pair % 2 == 1)
                    parent.RotationOffset = new Quaternion(0, 0, 1, 0);
                var pairGroup = new SceneObjectGroup(parent);
                var cylinder = PrimitiveBaseShape.CreateCylinder();
                cylinder.SetScale(0.25f + (pair % 3) * 0.125f);
                cylinder.Textures = new Primitive.TextureEntry(Id(1001 + n));
                var linked = Part(n + 1, "load child", cylinder, pos, new Vector3(0.75f, (pair % 3) * 0.25f, 0.5f));
                linked.RotationOffset = new Quaternion(1, 0, 0, 0);
                pairGroup.AddPart(linked);
                store.StoreObject(pairGroup, region);
            }
        }
    }
    finally
    {
        store.Dispose();
    }
    // Exercise the restore path before the Text and MediaUrl setters cut to
    // 254 UTF-8 bytes and Scene.LoadPrimsFromStorage clears Scripted on the root.
    using var db = new SQLiteConnection(connection);
    db.Open();
    using var update = db.CreateCommand();
    update.CommandText = "UPDATE prims SET Text=@text, MediaURL=@url, ObjectFlags=ObjectFlags | @scripted WHERE UUID=@id";
    update.Parameters.AddWithValue("@text", new string('é', 126) + "雪");
    update.Parameters.AddWithValue("@url", "https://example.invalid/".PadRight(250, 'a') + "🌐雪");
    update.Parameters.AddWithValue("@scripted", (int)PrimFlags.Scripted);
    update.Parameters.AddWithValue("@id", speaker.UUID.ToString());
    if (update.ExecuteNonQuery() != 1)
        throw new InvalidDataException("missing sound prim in capture seed");
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

static void SeedPostgres(string ado)
{
    PGSQLSimulationData store = new PGSQLSimulationData();
    try
    {
        store.Initialise(ado);
    }
    finally
    {
        store.Dispose();
    }

    using NpgsqlConnection db = new NpgsqlConnection(ado);
    db.Open();
    using (NpgsqlCommand version = db.CreateCommand())
    {
        version.CommandText = "SELECT version FROM migrations WHERE name = 'RegionStore'";
        object found = version.ExecuteScalar();
        if (found == null || Convert.ToInt32(found) != 62)
            throw new InvalidOperationException("RegionStore version is " + found);
    }
    InsertPrims(db, true);
    InsertShapes(db, true);
    InsertItems(db, true);
    InsertRewriteGroup(db);
}

static void InsertRewriteGroup(NpgsqlConnection db)
{
    BasePrim(RewriteRoot, Region, RewriteGroup, "rewrite-root").Set("LinkNumber", 0).Insert(db, "prims");
    BasePrim(RewriteChild, Region, RewriteGroup, "rewrite-child").Set("LinkNumber", 4).Insert(db, "prims");
    PlainShape(RewriteRoot).Set("Shape", 0).Insert(db, "primshapes");
    PlainShape(RewriteChild).Set("Shape", 0).Insert(db, "primshapes");
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
    if (db is MySqlConnection)
    {
        showcase.Set("AllowUnsit", 0);
        showcase.Set("ScriptedSitOnly", 1);
    }
    showcase
        .Set("CreatorID", Creator + ";https://host/agent")
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

    BasePrim(RootB, Region, RootB, "link-restore")
        .Set("LinkNumber", 9)
        .Set("CreatorID", Creator + ";https://host;Name;extra")
        .Insert(db, "prims");
    BasePrim(ChildB3, Region, RootB, "child-three").Set("LinkNumber", 3).Insert(db, "prims");
    Row childTwo = BasePrim(ChildB2, Region, RootB, "child-two")
        .Set("LinkNumber", 2)
        .Set("SitTargetActive", 1)
        .Set("DynAttrs", "<llsd><map></map></llsd>")
        .Set("KeyframeMotion", new byte[] { 0x01, 0x02, 0x03, 0x04 })
        .Set("Vehicle", "<Vehicle></Vehicle>")
        .Set("PhysInertia", "<PhysicsInertia></PhysicsInertia>")
        .Set("sopanims", new byte[] { 0x00 })
        .Set("linksetdata", "not-json");
    // A null pseudo-CRC skips this prim after link numbers are chosen. MySQL and
    // PostgreSQL keep a readable value so their load still includes the row.
    if (db is SQLiteConnection)
        childTwo.Set("pseudocrc", DBNull.Value);
    childTwo.Insert(db, "prims");

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
    Row note = TaskItem(PrimA, "c0000000-0000-0000-0000-000000000003", "00000000-0000-0000-0000-0000000000c3", "00000000-0000-0000-0000-0000000000d3", "note", 7, 7)
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
        .Set("experienceID", mysql ? "00000000-0000-0000-0000-000000000000" : "nope");
    // A null description stops the PostgreSQL load. The successful seed stores empty text.
    if (db is NpgsqlConnection)
        note.Set("description", "");
    note.Insert(db, "primitems");
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

// Both terrain rows through the pin's LoadTerrain and LoadBakedTerrain for a
// 256 m region. Each map is its cells as float bits, Y outer and X inner (the
// order GetDatabaseBlob writes), the stored revision, and whether the pin's
// own GetDatabaseBlob of that map gives the stored bytes.
static void LoadTerrainRows(string path, string regionText, string pin)
{
    string full = Path.GetFullPath(path);
    DumpTerrain(new SQLiteSimulationData(), "Data Source=" + full + ";Version=3;", regionText, pin,
        table => RawTerrainRow(full, table, regionText));
}

// One backend's LoadTerrain and LoadBakedTerrain for a 256 m region, with the
// stored row beside each map.
static void DumpTerrain(
    ISimulationDataStore store,
    string connection,
    string regionText,
    string pin,
    Func<string, (long revision, byte[] blob)?> raw)
{
    UUID region = new UUID(regionText);
    TerrainData terrain;
    TerrainData baked;
    try
    {
        store.Initialise(connection);
        terrain = store.LoadTerrain(region, 256, 256, (int)Constants.RegionHeight);
        baked = store.LoadBakedTerrain(region, 256, 256, (int)Constants.RegionHeight);
    }
    finally
    {
        store.Dispose();
    }

    JsonObject dump = new JsonObject
    {
        ["pin"] = pin,
        ["region"] = region.ToString(),
        ["terrain"] = TerrainJson(terrain, raw("terrain")),
        ["bakedterrain"] = TerrainJson(baked, raw("bakedterrain")),
    };
    Console.Out.Write(dump.ToJsonString(new JsonSerializerOptions { WriteIndented = false }));
    Console.Out.WriteLine();
}

// The pin's own StoreTerrain and StoreBakedTerrain of a new pinhead-island,
// as Scene.LoadWorldMap stores it. MySQL stores from a fire-and-forget
// callback; FireAndForgetMethod.None runs it before this returns.
static void SeedTerrain(ISimulationDataStore store, string connection, string regionText)
{
    Util.FireAndForgetMethod = FireAndForgetMethod.None;
    UUID region = new UUID(regionText);
    TerrainChannel channel = new TerrainChannel("pinhead-island", 256, 256, (int)Constants.RegionHeight);
    try
    {
        store.Initialise(connection);
        store.StoreTerrain(channel.GetTerrainData(), region);
        store.StoreBakedTerrain(channel.GetTerrainData(), region);
    }
    finally
    {
        store.Dispose();
    }
}

// The first stored row of `table` for the region, as the backend returns it.
static (long revision, byte[] blob)? RawTerrainRowDb(
    DbConnection db,
    string table,
    string regionText,
    string parameter,
    string regionColumn,
    string revisionColumn,
    string heightColumn,
    object region)
{
    using (db)
    {
        db.Open();
        using DbCommand cmd = db.CreateCommand();
        cmd.CommandText = "select " + revisionColumn + ", " + heightColumn + " from " + table +
            " where " + regionColumn + " = " + parameter;
        DbParameter value = cmd.CreateParameter();
        value.ParameterName = parameter.TrimStart('?', ':');
        value.Value = region;
        cmd.Parameters.Add(value);
        using DbDataReader row = cmd.ExecuteReader();
        if (!row.Read() || row.IsDBNull(1))
            return null;
        return (Convert.ToInt64(row[0]), (byte[])row[1]);
    }
}

static (long revision, byte[] blob)? RawTerrainRow(string path, string table, string region)
{
    using SQLiteConnection db = new SQLiteConnection("Data Source=" + path + ";Version=3;");
    db.Open();
    using SQLiteCommand cmd = new SQLiteCommand(
        "select Revision, Heightfield from " + table + " where RegionUUID = :RegionUUID", db);
    cmd.Parameters.Add(new SQLiteParameter(":RegionUUID", region));
    using SQLiteDataReader row = cmd.ExecuteReader();
    if (!row.Read())
        return null;
    return (Convert.ToInt64(row["Revision"]), (byte[])row["Heightfield"]);
}

static JsonNode TerrainJson(TerrainData map, (long revision, byte[] blob)? raw)
{
    if (map is null)
        return null;
    JsonArray cells = new JsonArray();
    for (int y = 0; y < map.SizeY; y++)
        for (int x = 0; x < map.SizeX; x++)
            cells.Add(BitConverter.SingleToUInt32Bits(map[x, y]));
    map.GetDatabaseBlob(out int revision, out Array blob);
    byte[] ours = (byte[])blob;
    return new JsonObject
    {
        ["size_x"] = map.SizeX,
        ["size_y"] = map.SizeY,
        ["stored_revision"] = raw?.revision,
        ["stored_bytes"] = raw?.blob.Length,
        ["csharp_revision"] = revision,
        ["csharp_bytes"] = ours.Length,
        ["csharp_blob_is_stored_blob"] = raw is not null && ours.AsSpan().SequenceEqual(raw.Value.blob),
        ["cells"] = cells,
    };
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

static void Serialize(string path, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    string full = Path.GetFullPath(path);
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

    SerializeDump root = new SerializeDump
    {
        pin = pin,
        region = region.ToString(),
        groups = new List<SerializeGroup>()
    };
    foreach (SceneObjectGroup group in groups)
    {
        root.groups.Add(new SerializeGroup
        {
            id = group.RootPart.UUID.ToString(),
            xml2 = SceneObjectSerializer.ToXml2Format(group),
            original = SceneObjectSerializer.ToOriginalXmlFormat(group)
        });
    }

    // Keep writer-only inputs out of the XML reader fixture. XmlTextWriter
    // emits references for characters that an XML 1.0 parser can reject.
    SceneObjectGroup probe = groups.First(g => g.RootPart.Name == "showcase");
    SceneObjectPart part = probe.RootPart;
    string special = "x\r\n\t<>&\"'+`\\\u0001\u001f\u007f\ufffe\uffffé😀";
    part.Description = special;
    part.SoundGain = (double)0.1f;
    part.AllowUnsit = false;
    part.ScriptedSitOnly = true;
    part.LinksetData = new LinksetData();
    part.LinksetData.AddOrUpdateLinksetDataKey("key", special, "");
    using var stream = new MemoryStream();
    using var bytes = new BinaryWriter(stream);
    bytes.Write((byte)8);
    void Entry(ushort kind, string hex)
    {
        byte[] data = Convert.FromHexString(hex);
        bytes.Write(kind);
        bytes.Write(data.Length);
        bytes.Write(data);
    }
    const string id = "0123456789ABCDEF0123456789ABCDEF";
    Entry(0x10, "870A64050000803F0000004000004040"); // flexible
    Entry(0x20, "10204080000080400000003F0000803F"); // light
    Entry(0x30, id + "01");                           // sculpt
    Entry(0x60, id + "05");                           // mesh sculpt type
    Entry(0x40, id + "0000803F0000003F0000803E");     // projection
    Entry(0x70, "03000000");                          // mesh flags
    Entry(0x90, "0000003F0000004003");                // reflection probe
    Entry(0x80, "0102" + id);                         // render materials
    byte[] seed = stream.ToArray();
    // SQLite builds ExtraParams first, then restores the separate MatOvrd column.
    byte[] overrides = part.Shape.RenderMaterialsOvrToRawBin();
    part.Shape.ExtraParams = seed;
    part.Shape.RenderMaterialsOvrFromRawBin(overrides);
    root.writer_probe = new {
        id = part.UUID.ToString(), description = part.Description,
        extra = seed.Select(b => (int)b).ToArray(),
        gains = new[] { (double)0.1f, 1e-4, 1e-5, 1e16, 1e17, double.Epsilon, -0d,
                        double.MaxValue, double.PositiveInfinity, double.NegativeInfinity, double.NaN }
            .Select(g => new { bits = BitConverter.DoubleToUInt64Bits(g).ToString("X16"),
                               text = g.ToString(OpenSim.Framework.Culture.FormatProvider) }).ToArray(),
        bytes_used = part.LinksetData.BytesUsed, bytes_free = part.LinksetData.BytesFree,
        xml2 = SceneObjectSerializer.ToXml2Format(probe),
        original = SceneObjectSerializer.ToOriginalXmlFormat(probe)
    };

    JsonSerializerOptions options = new JsonSerializerOptions { WriteIndented = true };
    Console.Out.Write(JsonSerializer.Serialize(root, options));
    Console.Out.Write('\n');
}

// Pin8 CoalescedSceneObjectsSerializer and SceneObjectGroup.Extra{To,From}XmlString.
static void CaptureCoalesced(string dumpPath, string pin)
{
    var rows = JsonNode.Parse(File.ReadAllText(dumpPath))["groups"].AsArray();
    var linked = System.Xml.Linq.XElement.Parse(rows[1]["original"].GetValue<string>());
    var parts = linked.Descendants("SceneObjectPart").ToArray();
    void SetVector(System.Xml.Linq.XElement part, string name, params string[] values)
    {
        foreach (var (axis, value) in part.Element(name).Elements().Zip(values))
            axis.Value = value;
    }
    parts[0].Element("LocalId").Value = "73";
    SetVector(parts[0], "GroupPosition", "-12.25", "2.75", "41.5");
    SetVector(parts[0], "RotationOffset", "0", "0", "0.6", "0.8");
    parts[1].Element("ParentID").Value = "73";
    SetVector(parts[1], "OffsetPosition", "-3", "5", "9");
    SetVector(parts[1], "RotationOffset", "0.6", "0", "0", "0.8");
    SetVector(parts[1], "Scale", "1", "3", "8");
    string[] inputs = [linked.ToString(System.Xml.Linq.SaveOptions.DisableFormatting), rows[0]["original"].GetValue<string>()];
    var groups = inputs.Select(SceneObjectSerializer.FromOriginalXmlFormat).ToArray();
    var bundle = new CoalescedSceneObjects(UUID.Zero, groups);
    string xml = CoalescedSceneObjectsSerializer.ToXml(bundle);
    string ReadBack(string source)
    {
        if (!CoalescedSceneObjectsSerializer.TryFromXml(source, out var read))
            throw new InvalidOperationException("coalesced read failed");
        return CoalescedSceneObjectsSerializer.ToXml(read);
    }
    var probe = groups[1];
    string id = probe.UUID.ToString();
    string tag = $"<ExtraFromItemID>{id}</ExtraFromItemID>";
    string[] extras = ["", "<Other />", tag, $"around{tag}suffix", $"<ExtraFromItemID>{id}",
        "<ExtraFromItemID>bad</ExtraFromItemID>", $"<ExtraFromItemID>{id.Replace("-", "")}</ExtraFromItemID>",
        $"<ExtraFromItemID> {id} trailing</ExtraFromItemID>", $"<ExtraFromItemID>bad</ExtraFromItemID>{tag}"];
    var extra = extras.Select(input =>
    {
        probe.FromItemID = probe.UUID;
        probe.ExtraFromXmlString(input);
        return new { input, id = probe.FromItemID.ToString(), xml = probe.ExtraToXmlString() };
    }).ToArray();
    string[] documents = ["<Other />", "<CoalescedObject />", "<CoalescedObject",
        "<CoalescedObject /><Extra />", "<CoalescedObject xmlns='urn:fixture' />",
        "<CoalescedObject><Unknown><SceneObjectGroup /></Unknown></CoalescedObject>"];
    var reads = documents.Select(input =>
    {
        bool accepted = CoalescedSceneObjectsSerializer.TryFromXml(input, out var result);
        return new { input, count = accepted ? result.Count : (int?)null };
    }).ToArray();
    // Preserve a valid member after a component that FromOriginalXmlFormat cannot read.
    var changed = System.Xml.Linq.XElement.Parse(xml);
    changed.SetAttributeValue("x", "invalid");
    changed.Elements().First().SetAttributeValue("offsety", "99999");
    string badMember = SceneObjectSerializer.ToOriginalXmlFormat(groups[1])
        .Replace("<KeyframeMotion>", "<KeyframeMotion>!");
    changed.AddFirst(new System.Xml.Linq.XElement("SceneObjectGroup", System.Xml.Linq.XElement.Parse(badMember)));
    bool skipsBadMember = ReadBack(changed.ToString(System.Xml.Linq.SaveOptions.DisableFormatting)) == ReadBack(xml);
    const string noShape = "<CoalescedObject><SceneObjectGroup><RootPart><SceneObjectPart /></RootPart>"
        + "<OtherParts /></SceneObjectGroup></CoalescedObject>";
    if (!CoalescedSceneObjectsSerializer.TryFromXml(noShape, out var missingShape))
        throw new InvalidOperationException("shape-less read refused");
    string noShapeWriteError = null;
    try { CoalescedSceneObjectsSerializer.ToXml(missingShape); }
    catch (NullReferenceException error) { noShapeWriteError = error.GetType().Name; }
    // ModifySerializedObject's asset-byte path. Exercise both UTF-8 replacement
    // and SanitizeXml's namespace repair inside the first member's name.
    byte[] damaged = System.Text.Encoding.UTF8.GetBytes(xml.Replace("<Name>link-restore<", "<Name>xmlns:xmlns:broken!name<"));
    int marker = damaged.AsSpan().IndexOf("!name<"u8);
    if (marker < 0) throw new InvalidOperationException("missing damaged-name marker");
    damaged[marker] = 0xc3;
    string decoded = OpenSim.Framework.Serialization.External.ExternalRepresentationUtils.SanitizeXml(Utils.BytesToString(damaged));
    if (!CoalescedSceneObjectsSerializer.TryFromXml(decoded, out var repaired))
        throw new InvalidOperationException("asset byte path refused the coalesced groups");
    var repairedNames = repaired.Objects.Select(g => g.Name).ToArray();
    byte[] bytes = System.Text.Encoding.UTF8.GetBytes(xml);
    Console.Out.Write(JsonSerializer.Serialize(new {
        pin, inputs, xml, repairedNames, readBack = ReadBack(xml), reads, skipsBadMember, noShape, noShapeWriteError,
        single = CoalescedSceneObjectsSerializer.ToXml(new CoalescedSceneObjects(UUID.Zero, groups[0])),
        empty = CoalescedSceneObjectsSerializer.ToXml(new CoalescedSceneObjects(UUID.Zero)),
        readBytes = CoalescedSceneObjectsSerializer.TryFromXmlData(bytes, out _),
        readBytesWithNul = CoalescedSceneObjectsSerializer.TryFromXmlData([..bytes, 0], out _), extra
    }, new JsonSerializerOptions { WriteIndented = true }));
}

static void ReadXml(string dumpPath, string pin)
{
    JsonNode root = JsonNode.Parse(File.ReadAllText(dumpPath))
        ?? throw new InvalidOperationException("empty xml dump");
    string region = root["region"]?.GetValue<string>()
        ?? throw new InvalidOperationException("xml dump has no region");
    JsonArray groups = root["groups"]?.AsArray()
        ?? throw new InvalidOperationException("xml dump has no groups");
    ReadDump dump = new ReadDump
    {
        pin = pin,
        region = region,
        groups = new List<SerializeGroup>()
    };
    string firstId = null;
    string firstXml2 = null;
    string firstOriginal = null;
    foreach (JsonNode node in groups)
    {
        if (node is null)
            continue;
        string id = node["id"]?.GetValue<string>()
            ?? throw new InvalidOperationException("group has no id");
        string xml2 = node["xml2"]?.GetValue<string>()
            ?? throw new InvalidOperationException("group has no xml2");
        string original = node["original"]?.GetValue<string>()
            ?? throw new InvalidOperationException("group has no original");
        SceneObjectGroup read2 = SceneObjectSerializer.FromXml2Format(xml2)
            ?? throw new InvalidOperationException("xml2 read failed for " + id);
        SceneObjectGroup readOriginal = SceneObjectSerializer.FromOriginalXmlFormat(original)
            ?? throw new InvalidOperationException("original read failed for " + id);
        dump.groups.Add(new SerializeGroup
        {
            id = id,
            xml2 = SceneObjectSerializer.ToXml2Format(read2),
            original = SceneObjectSerializer.ToOriginalXmlFormat(readOriginal)
        });
        if (firstId is null)
        {
            firstId = id;
            firstXml2 = xml2;
            firstOriginal = original;
        }
    }
    if (firstId is null || firstXml2 is null || firstOriginal is null)
        throw new InvalidOperationException("xml dump has no groups");

    string dirty2 = InsertUnknown(firstXml2);
    string dirtyOriginal = InsertUnknown(firstOriginal);
    SceneObjectGroup unknown2 = SceneObjectSerializer.FromXml2Format(dirty2)
        ?? throw new InvalidOperationException("unknown xml2 read failed");
    SceneObjectGroup unknownOriginal = SceneObjectSerializer.FromOriginalXmlFormat(dirtyOriginal)
        ?? throw new InvalidOperationException("unknown original read failed");
    string unknownXml2 = SceneObjectSerializer.ToXml2Format(unknown2);
    string unknownOriginalXml = SceneObjectSerializer.ToOriginalXmlFormat(unknownOriginal);
    if (unknownXml2 != dump.groups[0].xml2 || unknownOriginalXml != dump.groups[0].original)
        throw new InvalidOperationException("unknown element changed the fields that were read");
    if (unknownXml2.Contains("NotAField") || unknownXml2.Contains("SculptData")
        || unknownOriginalXml.Contains("NotAField") || unknownOriginalXml.Contains("SculptData"))
        throw new InvalidOperationException("unknown element was kept");
    dump.unknown = new ReadUnknown
    {
        id = firstId,
        element = "NotAField",
        shape_element = "SculptData",
        xml2 = unknownXml2,
        original = unknownOriginalXml
    };

    JsonSerializerOptions options = new JsonSerializerOptions { WriteIndented = true };
    Console.Out.Write(JsonSerializer.Serialize(dump, options));
    Console.Out.Write('\n');
}

static void ProbeXml(string dumpPath, string pin)
{
    JsonNode root = JsonNode.Parse(File.ReadAllText(dumpPath))
        ?? throw new InvalidOperationException("empty probe dump");
    JsonArray cases = root["cases"]?.AsArray()
        ?? throw new InvalidOperationException("probe dump has no cases");
    JsonArray rows = new();
    foreach (JsonNode node in cases)
    {
        if (node is null)
            continue;
        JsonObject row = new()
        {
            ["name"] = node["name"]?.GetValue<string>() ?? "case",
            ["xml2_in"] = node["xml2"]?.DeepClone(),
            ["original_in"] = node["original"]?.DeepClone()
        };
        if (node["xml2_expect"] is JsonNode xml2Expect)
            row["xml2_expect"] = xml2Expect.DeepClone();
        if (node["original_expect"] is JsonNode originalExpect)
            row["original_expect"] = originalExpect.DeepClone();
        ProbeOne(row, node["xml2"], true);
        ProbeOne(row, node["original"], false);
        rows.Add(row);
    }
    JsonObject dump = new()
    {
        ["pin"] = pin,
        ["cases"] = rows
    };
    JsonSerializerOptions options = new JsonSerializerOptions { WriteIndented = true };
    Console.Out.Write(JsonSerializer.Serialize(dump, options));
    Console.Out.Write('\n');
}

static void ProbeOne(JsonObject row, JsonNode xmlNode, bool xml2)
{
    string field = xml2 ? "xml2" : "original";
    if (xmlNode is null)
    {
        row[field] = null;
        row[field + "_error"] = null;
        return;
    }
    string xml = xmlNode.GetValue<string>();
    try
    {
        SceneObjectGroup group = xml2
            ? SceneObjectSerializer.FromXml2Format(xml)
            : SceneObjectSerializer.FromOriginalXmlFormat(xml);
        if (group is null)
        {
            row[field] = null;
            row[field + "_error"] = "null";
            return;
        }
        row[field] = xml2
            ? SceneObjectSerializer.ToXml2Format(group)
            : SceneObjectSerializer.ToOriginalXmlFormat(group);
        row[field + "_error"] = null;
    }
    catch (Exception error)
    {
        row[field] = null;
        row[field + "_error"] = error.GetType().Name + ": " + error.Message;
    }
}

static string InsertUnknown(string xml)
{
    int at = xml.IndexOf("<SceneObjectPart", StringComparison.Ordinal);
    if (at < 0)
        throw new InvalidOperationException("no SceneObjectPart");
    int end = xml.IndexOf('>', at);
    if (end < 0)
        throw new InvalidOperationException("SceneObjectPart is not closed");
    string withPart = xml.Insert(end + 1, "<NotAField>skip</NotAField>");
    int shape = withPart.IndexOf("<Shape>", StringComparison.Ordinal);
    if (shape < 0)
        throw new InvalidOperationException("no Shape");
    return withPart.Insert(shape + "<Shape>".Length, "<SculptData>nope</SculptData>");
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

static void LoadPostgres(string ado, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    Dictionary<string, RawPrim> prims = ReadPrimRawPostgres(ado);
    Dictionary<string, RawShape> shapes = ReadShapeRawPostgres(ado);
    List<SceneObjectGroup> groups;
    PGSQLSimulationData store = new PGSQLSimulationData();
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

static Dictionary<string, RawPrim> ReadPrimRawPostgres(string ado)
{
    Dictionary<string, RawPrim> rows = new Dictionary<string, RawPrim>();
    using NpgsqlConnection db = new NpgsqlConnection(ado);
    db.Open();
    using NpgsqlCommand command = db.CreateCommand();
    command.CommandText = "SELECT \"UUID\", \"DynAttrs\", \"Vehicle\", \"PhysInertia\", \"pseudocrc\", \"sopanims\", \"linksetdata\" FROM prims";
    using NpgsqlDataReader reader = command.ExecuteReader();
    while (reader.Read())
    {
        string uuid = CellText(reader, 0);
        rows[uuid] = new RawPrim
        {
            DynAttrs = reader.IsDBNull(1) ? null : reader.GetString(1),
            Keyframe = null,
            Vehicle = reader.IsDBNull(2) ? null : reader.GetString(2),
            Inertia = reader.IsDBNull(3) ? null : reader.GetString(3),
            PseudoCrc = reader.IsDBNull(4) ? 0 : reader.GetInt32(4),
            Animations = reader.IsDBNull(5) ? null : (byte[])reader.GetValue(5),
            Linkset = reader.IsDBNull(6) ? null : reader.GetString(6)
        };
    }
    return rows;
}

static Dictionary<string, RawShape> ReadShapeRawPostgres(string ado)
{
    Dictionary<string, RawShape> rows = new Dictionary<string, RawShape>();
    using NpgsqlConnection db = new NpgsqlConnection(ado);
    db.Open();
    using NpgsqlCommand command = db.CreateCommand();
    command.CommandText = "SELECT \"UUID\", \"Media\", \"MatOvrd\", \"Texture\", \"ExtraParams\" FROM primshapes";
    using NpgsqlDataReader reader = command.ExecuteReader();
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

static string CellText(DbDataReader reader, int index)
{
    object value = reader.GetValue(index);
    if (value is Guid guid)
        return guid.ToString();
    return value.ToString();
}

static void SeedSettings(string path)
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
    using SQLiteCommand command = db.CreateCommand();
    command.CommandText =
        "INSERT INTO regionsettings (" +
        "regionUUID, block_terraform, block_fly, allow_damage, restrict_pushing, " +
        "allow_land_resell, allow_land_join_divide, block_show_in_search, " +
        "agent_limit, object_bonus, maturity, disable_scripts, disable_collisions, " +
        "disable_physics, terrain_texture_1, terrain_texture_2, terrain_texture_3, " +
        "terrain_texture_4, elevation_1_nw, elevation_2_nw, elevation_1_ne, " +
        "elevation_2_ne, elevation_1_se, elevation_2_se, elevation_1_sw, " +
        "elevation_2_sw, water_height, terrain_raise_limit, terrain_lower_limit, " +
        "use_estate_sun, fixed_sun, sun_position, covenant, sandbox, " +
        "sunvectorx, sunvectory, sunvectorz, map_tile_ID, covenant_datetime, " +
        "TelehubObject, parcel_tile_ID, block_search, casino, cacheID, " +
        "TerrainPBR1, TerrainPBR2, TerrainPBR3, TerrainPBR4" +
        ") VALUES (" +
        "@region, 1, 1, 1, 1, " +
        "0, 0, 1, " +
        "15, 1.5, 21, 1, 1, " +
        "1, @zero, @tex2, @tex3, " +
        "@tex4, 11.5, 80.5, 13.5, " +
        "5000, 12.25, 90, -150, " +
        "70, 42.5, 250, -300, " +
        "0, 1, 1.25, @covenant, 1, " +
        "0.25, -0.5, 0.75, @map, 1700000000, " +
        "@telehub, @parcel, 1, 1, @cache, " +
        "@zero, @pbr2, @pbr3, @pbr4)";
    command.Parameters.AddWithValue("@region", Region);
    command.Parameters.AddWithValue("@zero", UUID.Zero.ToString());
    command.Parameters.AddWithValue("@tex2", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2");
    command.Parameters.AddWithValue("@tex3", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3");
    command.Parameters.AddWithValue("@tex4", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4");
    command.Parameters.AddWithValue("@pbr2", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2");
    command.Parameters.AddWithValue("@pbr3", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb3");
    command.Parameters.AddWithValue("@pbr4", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb4");
    command.Parameters.AddWithValue("@covenant", "c0c0c0c0-0000-4000-8000-0000000000c0");
    command.Parameters.AddWithValue("@map", "0a070a07-0000-4000-8000-0000000000a1");
    command.Parameters.AddWithValue("@telehub", "7e1e0000-0000-4000-8000-0000000000b1");
    command.Parameters.AddWithValue("@parcel", "0a0c0000-0000-4000-8000-0000000000c1");
    command.Parameters.AddWithValue("@cache", "5e77e5c0-0000-4000-8000-000000000056");
    command.ExecuteNonQuery();

    InsertSpawn(db, 0.5, 0.25, 3.0);
    InsertSpawn(db, -1.0, 0.125, 8.5);
}

static void InsertSpawn(SQLiteConnection db, double yaw, double pitch, double distance)
{
    using SQLiteCommand command = db.CreateCommand();
    command.CommandText = "INSERT INTO spawn_points (RegionID, Yaw, Pitch, Distance) VALUES (@region, @yaw, @pitch, @distance)";
    command.Parameters.AddWithValue("@region", Region);
    command.Parameters.AddWithValue("@yaw", yaw);
    command.Parameters.AddWithValue("@pitch", pitch);
    command.Parameters.AddWithValue("@distance", distance);
    command.ExecuteNonQuery();
}

// Isolate the missing-row observation from the committed fixture. Opening a
// fresh plugin twice proves the id came back from SQLite across a restart.
static bool MissingCachePersists(string source)
{
    string path = Path.Combine(AppContext.BaseDirectory, $"missing-settings-{Guid.NewGuid():N}.db");
    File.Copy(source, path);
    UUID region = UUID.Random();
    UUID Load()
    {
        var store = new SQLiteSimulationData();
        try
        {
            store.Initialise($"Data Source={path};Version=3;Pooling=False;");
            return store.LoadRegionSettings(region).CacheID;
        }
        finally
        {
            store.Dispose();
        }
    }
    try
    {
        UUID first = Load();
        return !first.IsZero() && first == Load();
    }
    finally
    {
        File.Delete(path);
    }
}

// Observe Scene's own all-zero PBR fallback at its Save boundary. Leave before
// the rest of scene startup, which would require a grid and region modules.
static string[] SceneSavedPbr(RegionSettings source)
{
    var settings = new RegionSettings {
        TerrainTexture1 = source.TerrainTexture1, TerrainTexture2 = source.TerrainTexture2,
        TerrainTexture3 = source.TerrainTexture3, TerrainTexture4 = source.TerrainTexture4
    };
    var service = DispatchProxy.Create<ISimulationDataService, TerrainProbeService>();
    ((TerrainProbeService)service).Settings = settings;
    settings.OnSave += _ => throw new TerrainSettingsSaved();
    try
    {
        _ = new Scene(new RegionInfo { RegionID = source.RegionUUID }, null, service, null, null, "");
    }
    catch (TerrainSettingsSaved)
    {
        return new[] { settings.TerrainPBR1, settings.TerrainPBR2,
                       settings.TerrainPBR3, settings.TerrainPBR4 }.Select(id => id.ToString()).ToArray();
    }
    throw new InvalidOperationException("Scene did not save the empty PBR set");
}

// Load the same SQL seed as the Rust test through Pin8's SQLite provider.
static void LoadParcels(string path, string seed, string region, string pin)
{
    string connection = "Data Source=" + Path.GetFullPath(path) + ";Version=3;";
    using (SQLiteConnection db = new(connection))
    {
        db.Open();
        using SQLiteCommand command = db.CreateCommand();
        command.CommandText = File.ReadAllText(seed);
        command.ExecuteNonQuery();
    }
    SQLiteSimulationData store = new();
    try
    {
        store.Initialise(connection);
        var parcels = store.LoadLandObjects(new UUID(region));
        var defaults = new LandData { GlobalID = UUID.Zero };
        List<string> environments = new() {
            "{}", "null", "unknown", "{\"env_version\":2.5}", "{\"env_version\":true}",
            "{\"env_version\":\"-1.6\"}", "{\"env_version\":2147483649}",
            """{"env_version":5,"day_cycle":{"frames":{"bad":7}}}""",
            """{"env_version":5,"day_cycle":{"tracks":[[],[],[],[],[7]]}}""",
            """{"env_version":5,"day_cycle":{"frames":{"sky":{"type":"sky","absorption_config":[{},7]}}}}""",
            """{"env_version":5,"day_cycle":{"frames":{"sky":{"type":"sky","mie_config":[{},7]}},"tracks":[[{}],0,[],[],[],[7]]}}""",
            """{"env_version":5,"day_cycle":{"frames":{"sky":{"type":"sky","rayleigh_config":[7]}}}}""",
            "<llsd><map><key>env_version</key><integer>5</integer><key>day_cycle</key><map><key>frames</key><map><key>sky</key><map><key>type</key><string>sky</string><key>glow</key><string/></map></map></map></map></llsd>",
            "<? llsd/notation ?>{'env_version':i5,'day_cycle':{'tracks':[[i7]]}}",
            "<? llsd/notation ?>{'env_version':i5,'day_cycle':{'frames':{'s':{'type':'sky','glow':'x'}}}}",
            "<llsd><map><key>env_version</key><integer>5</integer><key>day_cycle</key><map><key>frames</key><map><key>s</key><map><key>type</key><uri>sky</uri><key>glow</key><string>x</string></map></map></map></map></llsd>",
            "<llsd><map><key>env_version</key><integer>5</integer><key>day_cycle</key><map><key>frames</key><map><key>s</key><map><key>type</key><uri>sky</uri><key>glow</key><uri>x</uri></map></map></map></map></llsd>",
            "<llsd><map><key>env_version</key><integer>42</integer></map></llsd>"
        };
        void VectorProbe(string type, string key, object value, bool haze = false)
        {
            Dictionary<string, object> frame = new() { ["type"] = type };
            if (haze) frame["legacy_haze"] = new Dictionary<string, object> { [key] = value };
            else frame[key] = value;
            environments.Add(JsonSerializer.Serialize(new { env_version = 5, day_cycle = new { frames = new { probe = frame } } }));
        }
        foreach (string key in new[] { "glow", "cloud_color", "cloud_pos_density1", "cloud_pos_density2", "cloud_scroll_rate", "moon_rotation", "sun_rotation" })
            VectorProbe("sky", key, "x");
        foreach (string key in new[] { "ambient", "blue_density", "blue_horizon" })
            VectorProbe("sky", key, "bad", haze: true);
        foreach (string key in new[] { "normal_scale", "water_fog_color", "wave1_direction", "wave2_direction" })
            VectorProbe("water", key, "bad");
        foreach (string value in new[] { "<1,2,3>", "1,2,3 trailing", "", "1,2", "1,2,inf", "1,2,Infinity", "1,2,∞", "1,2,+NaN" })
            VectorProbe("sky", "glow", value);
        foreach (string value in new[] { "1,2", "prefix<1,2>tail", "1,2 extra" })
            VectorProbe("water", "wave1_direction", value);
        foreach (string value in new[] { "bad", "<1,2,3>", "1,2,3", "1,2,3,4", "<1,2,3,4>tail" })
            VectorProbe("sky", "sun_rotation", value);
        VectorProbe("sky", "moon_rotation", "1,2,3>");
        foreach (string value in new[] { "1,2, 3", "1,2,3\0", "1,2, ∞", "1,2,\u00a0∞", "1,2,\u00a03", "11,2," })
            VectorProbe("sky", "glow", value);
        foreach (string value in new[] { "11,2,", "12," })
            VectorProbe("sky", "cloud_scroll_rate", value);
        VectorProbe("sky", "glow", new[] { 1, 2 });
        Console.WriteLine(JsonSerializer.Serialize(new {
            pin, region, parcels = parcels.Select(ParcelFields), defaults = ParcelFields(defaults),
            environments = environments.Select(text => new {
                text, version = ViewerEnvironment.FromOSDString(text)?.version
            })
        }, new JsonSerializerOptions { WriteIndented = true }));
    }
    finally { store.Dispose(); }
}

static object ParcelFields(LandData p) => new
{
    id = p.GlobalID.ToString(), local_id = p.LocalID,
    bitmap = Convert.ToHexString(p.Bitmap).ToLowerInvariant(), name = p.Name,
    description = p.Description, owner = p.OwnerID.ToString(), group_owned = p.IsGroupOwned,
    area = p.Area, auction = p.AuctionID, category = (int)p.Category, claim_date = p.ClaimDate,
    claim_price = p.ClaimPrice, group = p.GroupID.ToString(), sale_price = p.SalePrice,
    status = (int)p.Status, flags = p.Flags, landing_type = p.LandingType,
    media_auto_scale = p.MediaAutoScale, media_id = p.MediaID.ToString(),
    media_url = p.MediaURL, music_url = p.MusicURL, pass_hours = p.PassHours,
    pass_price = p.PassPrice, snapshot = p.SnapshotID.ToString(), dwell = p.Dwell,
    media_type = p.MediaType, media_description = p.MediaDescription,
    media_size = new[] {p.MediaWidth, p.MediaHeight}, media_loop = p.MediaLoop,
    obscure_media = p.ObscureMedia, obscure_music = p.ObscureMusic, see_avs = p.SeeAVs,
    any_av_sounds = p.AnyAVSounds, group_av_sounds = p.GroupAVSounds,
    location = new[] {p.UserLocation.X, p.UserLocation.Y, p.UserLocation.Z},
    look_at = new[] {p.UserLookAt.X, p.UserLookAt.Y, p.UserLookAt.Z},
    auth_buyer = p.AuthBuyerID.ToString(), other_clean_time = p.OtherCleanTime,
    environment_version = p.EnvironmentVersion,
    access = p.ParcelAccessList.Select(e => new { agent = e.AgentID.ToString(), flags = (uint)e.Flags, expires = e.Expires })
};

static void LoadSettings(string path, string regionText, string pin)
{
    string full = Path.GetFullPath(path);
    UUID region = new UUID(regionText);
    (bool fixedColumn, double sunX, double sunY, double sunZ, double sunPosition) = ReadSunColumns(full, regionText);
    RegionSettings settings;
    SQLiteSimulationData store = new SQLiteSimulationData();
    try
    {
        store.Initialise("Data Source=" + full + ";Version=3;");
        settings = store.LoadRegionSettings(region);
    }
    finally
    {
        store.Dispose();
    }

    EstateSettings estate = new EstateSettings();
    List<SpawnPoint> points = settings.SpawnPoints();
    SettingsDump dump = new SettingsDump
    {
        pin = pin,
        region = region.ToString(),
        csharp_region_flags = RegionHandshakeFlags(settings, estate),
        water_height = (float)settings.WaterHeight,
        cache_id = settings.CacheID.ToString(),
        terrain_textures = new[]
        {
            settings.TerrainTexture1.ToString(),
            settings.TerrainTexture2.ToString(),
            settings.TerrainTexture3.ToString(),
            settings.TerrainTexture4.ToString()
        },
        terrain_pbr = new[]
        {
            settings.TerrainPBR1.ToString(),
            settings.TerrainPBR2.ToString(),
            settings.TerrainPBR3.ToString(),
            settings.TerrainPBR4.ToString()
        },
        terrain_start_heights = new[]
        {
            (float)settings.Elevation1SW,
            (float)settings.Elevation1NW,
            (float)settings.Elevation1SE,
            (float)settings.Elevation1NE
        },
        terrain_height_ranges = new[]
        {
            (float)settings.Elevation2SW,
            (float)settings.Elevation2NW,
            (float)settings.Elevation2SE,
            (float)settings.Elevation2NE
        },
        allow_damage = settings.AllowDamage,
        block_terraform = settings.BlockTerraform,
        block_fly = settings.BlockFly,
        restrict_pushing = settings.RestrictPushing,
        allow_land_resell = settings.AllowLandResell,
        allow_land_join_divide = settings.AllowLandJoinDivide,
        block_show_in_search = settings.BlockShowInSearch,
        sandbox = settings.Sandbox,
        casino = settings.Casino,
        god_block_search = settings.GodBlockSearch,
        disable_scripts = settings.DisableScripts,
        disable_collisions = settings.DisableCollisions,
        disable_physics = settings.DisablePhysics,
        fixed_sun = settings.FixedSun,
        fixed_sun_column = fixedColumn,
        sun_vector = new[] { settings.SunVector.X, settings.SunVector.Y, settings.SunVector.Z },
        sun_vector_column = new[] { sunX, sunY, sunZ },
        sun_position = settings.SunPosition,
        sun_position_column = sunPosition,
        agent_limit = settings.AgentLimit,
        object_bonus = settings.ObjectBonus,
        maturity = settings.Maturity,
        sim_access = Util.ConvertMaturityToAccessLevel((uint)settings.Maturity),
        access_by_maturity = new[] { 0, 1, 2, 3, -1 }.Select(m =>
            (int)Util.ConvertMaturityToAccessLevel((uint)m)).ToArray(),
        missing_cache_persists = MissingCachePersists(full),
        saved_pbr_from_empty = SceneSavedPbr(settings),
        terrain_raise_limit = settings.TerrainRaiseLimit,
        terrain_lower_limit = settings.TerrainLowerLimit,
        use_estate_sun = settings.UseEstateSun,
        covenant = settings.Covenant.ToString(),
        covenant_datetime = settings.CovenantChangedDateTime,
        map_tile_id = settings.TerrainImageID.ToString(),
        telehub_object = settings.TelehubObject.ToString(),
        parcel_tile_id = settings.ParcelImageID.ToString(),
        spawn_points = new SettingsSpawn[points.Count]
    };
    for (int index = 0; index < points.Count; index++)
    {
        dump.spawn_points[index] = new SettingsSpawn
        {
            yaw = points[index].Yaw,
            pitch = points[index].Pitch,
            distance = points[index].Distance
        };
    }

    JsonSerializerOptions options = new JsonSerializerOptions { WriteIndented = true };
    Console.Out.Write(JsonSerializer.Serialize(dump, options));
    Console.Out.Write('\n');
}

static (bool, double, double, double, double) ReadSunColumns(string path, string region)
{
    using SQLiteConnection db = new SQLiteConnection("Data Source=" + path + ";Version=3;");
    db.Open();
    using SQLiteCommand command = db.CreateCommand();
    command.CommandText = "SELECT fixed_sun, sunvectorx, sunvectory, sunvectorz, sun_position FROM regionsettings WHERE regionUUID = @region";
    command.Parameters.AddWithValue("@region", region);
    using SQLiteDataReader reader = command.ExecuteReader();
    if (!reader.Read())
        throw new InvalidOperationException("region settings row is missing");
    return (
        Convert.ToInt32(reader.GetValue(0), System.Globalization.CultureInfo.InvariantCulture) != 0,
        Convert.ToDouble(reader.GetValue(1), System.Globalization.CultureInfo.InvariantCulture),
        Convert.ToDouble(reader.GetValue(2), System.Globalization.CultureInfo.InvariantCulture),
        Convert.ToDouble(reader.GetValue(3), System.Globalization.CultureInfo.InvariantCulture),
        Convert.ToDouble(reader.GetValue(4), System.Globalization.CultureInfo.InvariantCulture)
    );
}

static uint RegionHandshakeFlags(RegionSettings region, EstateSettings estate)
{
    OpenMetaverse.RegionFlags flags = OpenMetaverse.RegionFlags.None;
    if (region.AllowDamage)
        flags |= OpenMetaverse.RegionFlags.AllowDamage;
    if (estate.AllowLandmark)
        flags |= OpenMetaverse.RegionFlags.AllowLandmark;
    if (estate.AllowSetHome)
        flags |= OpenMetaverse.RegionFlags.AllowSetHome;
    if (estate.ResetHomeOnTeleport)
        flags |= OpenMetaverse.RegionFlags.ResetHomeOnTeleport;
    if (region.FixedSun)
        flags |= OpenMetaverse.RegionFlags.SunFixed;
    if (!estate.TaxFree)
        flags |= OpenMetaverse.RegionFlags.AllowParcelAccessOverride;
    if (region.BlockTerraform)
        flags |= OpenMetaverse.RegionFlags.BlockTerraform;
    if (!region.AllowLandResell)
        flags |= OpenMetaverse.RegionFlags.BlockLandResell;
    if (region.Sandbox)
        flags |= OpenMetaverse.RegionFlags.Sandbox;
    if (region.Casino)
        flags |= OpenMetaverse.RegionFlags.SkipAgentAction;
    if (region.GodBlockSearch)
        flags |= OpenMetaverse.RegionFlags.SkipUpdateInterestList;
    if (region.DisableCollisions)
        flags |= OpenMetaverse.RegionFlags.SkipCollisions;
    if (region.DisableScripts)
        flags |= OpenMetaverse.RegionFlags.SkipScripts;
    if (region.DisablePhysics)
        flags |= OpenMetaverse.RegionFlags.SkipPhysics;
    if (estate.PublicAccess)
        flags |= OpenMetaverse.RegionFlags.ExternallyVisible;
    if (estate.BlockDwell)
        flags |= OpenMetaverse.RegionFlags.BlockDwell;
    if (region.BlockFly)
        flags |= OpenMetaverse.RegionFlags.NoFly;
    if (estate.AllowDirectTeleport)
        flags |= OpenMetaverse.RegionFlags.AllowDirectTeleport;
    if (estate.EstateSkipScripts)
        flags |= OpenMetaverse.RegionFlags.EstateSkipScripts;
    if (region.RestrictPushing)
        flags |= OpenMetaverse.RegionFlags.RestrictPushObject;
    if (estate.DenyAnonymous)
        flags |= OpenMetaverse.RegionFlags.DenyAnonymous;
    if (estate.DenyIdentified)
        flags |= OpenMetaverse.RegionFlags.DenyIdentified;
    if (estate.DenyTransacted)
        flags |= OpenMetaverse.RegionFlags.DenyTransacted;
    if (region.AllowLandJoinDivide)
        flags |= OpenMetaverse.RegionFlags.AllowParcelChanges;
    if (estate.AllowVoice)
        flags |= OpenMetaverse.RegionFlags.AllowVoice;
    if (region.BlockShowInSearch)
        flags |= OpenMetaverse.RegionFlags.BlockParcelSearch;
    if (estate.DenyMinors)
        flags |= OpenMetaverse.RegionFlags.DenyAgeUnverified;
    if (estate.AllowEnvironmentOverride)
        flags |= OpenMetaverse.RegionFlags.AllowEnvironmentOverride;
    return (uint)flags;
}

// The notation StoreOnRegion writes: ViewerEnvironment.ToOSD then
// SerializeLLSDNotationFull. The sample is loaded back through FromOSDString
// and written again, and that second write must be the same text.
static string SampleEnvironment()
{
    ViewerEnvironment env = new ViewerEnvironment();
    env.FromOSD(new OSDMap
    {
        ["day_cycle"] = new OSDMap
        {
            ["frames"] = new OSDMap(),
            ["name"] = "probe-day",
            ["tracks"] = new OSDArray
            {
                new OSDArray(),
                new OSDArray(),
                new OSDArray(),
                new OSDArray(),
                new OSDArray(),
            },
            ["type"] = "daycycle",
        },
        ["day_length"] = 14400,
        ["day_offset"] = 57600,
        ["flags"] = 1,
        ["env_version"] = 7,
        ["track_altitudes"] = new OSDArray { 1100f, 2200f, 3300f },
    });
    if (env.version != 7 || env.DayLength != 14400 || env.DayOffset != 57600 || env.Flags != 1)
        throw new InvalidOperationException("sample environment did not load");
    string text = ViewerEnvironment.ToOSDString(env);
    if (string.IsNullOrEmpty(text))
        throw new InvalidOperationException("sample environment did not serialize");
    ViewerEnvironment again = ViewerEnvironment.FromOSDString(text);
    if (again == null || again.version != 7 || again.DayLength != 14400 || again.Flags != 1)
        throw new InvalidOperationException("sample environment did not parse");
    if (ViewerEnvironment.ToOSDString(again) != text)
        throw new InvalidOperationException("sample environment is not stable");
    return text;
}

static void WriteEnvironment(string pin, string region, string settings)
{
    JsonObject dump = new JsonObject
    {
        ["pin"] = pin,
        ["settings"] = settings,
    };
    if (region != null)
        dump["region"] = region;
    Console.Out.Write(dump.ToJsonString(new JsonSerializerOptions { WriteIndented = false }));
    Console.Out.WriteLine();
}

static void EnvironmentLoad(ISimulationDataStore store, string connection, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    string settings;
    try
    {
        store.Initialise(connection);
        settings = store.LoadRegionEnvironmentSettings(region);
    }
    finally
    {
        store.Dispose();
    }
    WriteEnvironment(pin, region.ToString(), settings ?? "");
}

static void EnvironmentStore(ISimulationDataStore store, string connection, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    string settings = SampleEnvironment();
    try
    {
        store.Initialise(connection);
        store.StoreRegionEnvironmentSettings(region, settings);
    }
    finally
    {
        store.Dispose();
    }
    WriteEnvironment(pin, region.ToString(), settings);
}

static void EnvironmentRemove(ISimulationDataStore store, string connection, string regionText, string pin)
{
    UUID region = new UUID(regionText);
    string settings;
    try
    {
        store.Initialise(connection);
        store.RemoveRegionEnvironmentSettings(region);
        settings = store.LoadRegionEnvironmentSettings(region);
    }
    finally
    {
        store.Dispose();
    }
    WriteEnvironment(pin, region.ToString(), settings ?? "");
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

sealed class SerializeDump
{
    public object writer_probe { get; set; }
    public string pin { get; set; }
    public string region { get; set; }
    public List<SerializeGroup> groups { get; set; }
}

sealed class SerializeGroup
{
    public string id { get; set; }
    public string xml2 { get; set; }
    public string original { get; set; }
}

sealed class ReadDump
{
    public string pin { get; set; }
    public string region { get; set; }
    public List<SerializeGroup> groups { get; set; }
    public ReadUnknown unknown { get; set; }
}

sealed class ReadUnknown
{
    public string id { get; set; }
    public string element { get; set; }
    public string shape_element { get; set; }
    public string xml2 { get; set; }
    public string original { get; set; }
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
    public bool allow_unsit { get; set; }
    public bool scripted_sit_only { get; set; }
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
            allow_unsit = part.AllowUnsit,
            scripted_sit_only = part.ScriptedSitOnly,
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
        bool postgres = db is NpgsqlConnection;
        // A later Set for the same column replaces the earlier value. MySQL rejects
        // two parameters of one name; the stored row is the last value either way.
        List<string> names = new List<string>();
        Dictionary<string, object> values = new Dictionary<string, object>();
        foreach ((string name, object value) in cells)
        {
            if (postgres && PostgresSkip.Contains(name))
                continue;
            if (!values.ContainsKey(name))
                names.Add(name);
            values[name] = value;
        }
        // The boolean conversion drops the old default, so an omitted cell is rejected.
        // Sit-target numbers have no default. A null one stops the load, so the
        // successful seed stores zero unless the row already set the cell.
        if (postgres && table == "prims")
        {
            if (!values.ContainsKey("PassCollisions"))
            {
                names.Add("PassCollisions");
                values["PassCollisions"] = false;
            }
            foreach (string sit in new[]
            {
                "SitTargetOffsetX", "SitTargetOffsetY", "SitTargetOffsetZ",
                "SitTargetOrientX", "SitTargetOrientY", "SitTargetOrientZ", "SitTargetOrientW"
            })
            {
                if (!values.ContainsKey(sit))
                {
                    names.Add(sit);
                    values[sit] = 0.0;
                }
            }
        }
        string columns = postgres
            ? string.Join(", ", names.ConvertAll(name => "\"" + name.Replace("\"", "\"\"") + "\""))
            : string.Join(", ", names);
        string marks = string.Join(", ", names.ConvertAll(name => "@" + name));
        using DbCommand command = db.CreateCommand();
        command.CommandText = "INSERT INTO " + table + " (" + columns + ") VALUES (" + marks + ")";
        foreach (string name in names)
        {
            object stored = values[name] ?? DBNull.Value;
            if ((mysql || postgres) && stored is string text && (name == "TextureAnimation" || name == "ParticleSystem"))
                stored = Convert.FromBase64String(text);
            if (postgres)
                stored = PostgresValue(table, name, stored);
            DbParameter parameter = command.CreateParameter();
            parameter.ParameterName = "@" + name;
            parameter.Value = stored;
            command.Parameters.Add(parameter);
        }
        command.ExecuteNonQuery();
    }

    static readonly HashSet<string> PostgresSkip = new HashSet<string>
    {
        "VolumeDetect", "AttachedPosX", "AttachedPosY", "AttachedPosZ", "KeyframeMotion", "LastAttachPoint"
    };

    static readonly HashSet<string> UuidColumns = new HashSet<string>
    {
        "UUID", "RegionUUID", "SceneGroupID", "OwnerID", "GroupID", "LastOwnerID",
        "LoopedSound", "CollisionSound", "RezzerID",
        "itemID", "primID", "assetID", "parentFolderID", "ownerID", "lastOwnerID", "groupID", "experienceID"
    };

    static object PostgresValue(string table, string name, object stored)
    {
        if (stored is int number && (name == "PassTouches" || name == "PassCollisions"))
            return number != 0;
        if (stored is int date && table == "primitems" && name == "creationDate")
            return date.ToString(System.Globalization.CultureInfo.InvariantCulture);
        if (stored is string text && UuidColumns.Contains(name) && Guid.TryParse(text, out Guid guid))
            return guid;
        return stored;
    }
}

sealed class TerrainSettingsSaved : Exception { }

public class TerrainProbeService : DispatchProxy
{
    public RegionSettings Settings { get; set; }
    protected override object Invoke(MethodInfo method, object[] args) => method.Name switch
    {
        nameof(ISimulationDataService.LoadRegionSettings) => Settings,
        nameof(ISimulationDataService.GetExtra) => new Dictionary<string, string>(),
        _ => throw new NotSupportedException(method.Name)
    };
}

sealed class SettingsDump
{
    public string pin { get; set; }
    public string region { get; set; }
    public uint csharp_region_flags { get; set; }
    public float water_height { get; set; }
    public string cache_id { get; set; }
    public string[] terrain_textures { get; set; }
    public string[] terrain_pbr { get; set; }
    public float[] terrain_start_heights { get; set; }
    public float[] terrain_height_ranges { get; set; }
    public bool allow_damage { get; set; }
    public bool block_terraform { get; set; }
    public bool block_fly { get; set; }
    public bool restrict_pushing { get; set; }
    public bool allow_land_resell { get; set; }
    public bool allow_land_join_divide { get; set; }
    public bool block_show_in_search { get; set; }
    public bool sandbox { get; set; }
    public bool casino { get; set; }
    public bool god_block_search { get; set; }
    public bool disable_scripts { get; set; }
    public bool disable_collisions { get; set; }
    public bool disable_physics { get; set; }
    public bool fixed_sun { get; set; }
    public bool fixed_sun_column { get; set; }
    public float[] sun_vector { get; set; }
    public double[] sun_vector_column { get; set; }
    public double sun_position { get; set; }
    public double sun_position_column { get; set; }
    public int agent_limit { get; set; }
    public double object_bonus { get; set; }
    public int maturity { get; set; }
    public byte sim_access { get; set; }
    public int[] access_by_maturity { get; set; }
    public bool missing_cache_persists { get; set; }
    public string[] saved_pbr_from_empty { get; set; }
    public double terrain_raise_limit { get; set; }
    public double terrain_lower_limit { get; set; }
    public bool use_estate_sun { get; set; }
    public string covenant { get; set; }
    public int covenant_datetime { get; set; }
    public string map_tile_id { get; set; }
    public string telehub_object { get; set; }
    public string parcel_tile_id { get; set; }
    public SettingsSpawn[] spawn_points { get; set; }
}

sealed class SettingsSpawn
{
    public float yaw { get; set; }
    public float pitch { get; set; }
    public float distance { get; set; }
}
