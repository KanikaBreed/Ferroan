// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Data.SQLite;
using System.Text.Json;
using OpenMetaverse;
using OpenSim.Data.SQLite;
using OpenSim.Framework;
using OpenSim.Region.Framework.Scenes;

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

if (args.Length == 0)
{
    Console.Error.WriteLine("usage: regionstore-oracle seed <db> | load <db> <region-uuid> <pin>");
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
        default:
            Console.Error.WriteLine("usage: regionstore-oracle seed <db> | load <db> <region-uuid> <pin>");
            return 2;
    }
}
catch (Exception error)
{
    Console.Error.WriteLine(error.GetType().Name + ": " + error.Message);
    return 1;
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
    InsertPrims(db);
    InsertShapes(db);
    tx.Commit();
}

static void InsertPrims(SQLiteConnection db)
{
    BasePrim(PrimA, Region, PrimA, "showcase")
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
        .Set("CollisionSoundVolume", 0.25)
        .Set("VolumeDetect", 1)
        .Set("MediaURL", "http://example.test/media")
        .Set("AttachedPosX", 0.5)
        .Set("DynAttrs", "<llsd><map></map></llsd>")
        .Set("PhysicsShapeType", 1)
        .Set("Density", 2.0)
        .Set("GravityModifier", 1.5)
        .Set("Friction", 0.25)
        .Set("Restitution", 0.125)
        .Set("KeyframeMotion", new byte[] { 0x01, 0x02, 0x03, 0x04 })
        .Set("PassCollisions", 1)
        .Set("PassTouches", 1)
        .Set("RotationAxisLocks", 5)
        .Set("Vehicle", "<Vehicle></Vehicle>")
        .Set("RezzerID", Rezzer)
        .Set("PhysInertia", "<PhysicsInertia></PhysicsInertia>")
        .Set("pseudocrc", 42)
        .Set("sopanims", new byte[] { 0x00 })
        .Set("linksetdata", "not-json")
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
        .Insert(db, "prims");

    BasePrim(RootC, Region, RootC, "link-table").Insert(db, "prims");
    BasePrim(ChildC2, Region, RootC, "child-first").Insert(db, "prims");
    BasePrim(ChildC3, Region, RootC, "child-second").Set("LinkNumber", 5).Insert(db, "prims");

    BasePrim(PrimD, Region, PrimD, "no-shape")
        .Set("SitTargetOffsetX", 2.0)
        .Set("pseudocrc", 0)
        .Insert(db, "prims");

    BasePrim("not-a-uuid", Region, "not-a-uuid", "unbuildable").Insert(db, "prims");
    BasePrim(PrimF, OtherRegion, PrimF, "other-region").Insert(db, "prims");
}

static void InsertShapes(SQLiteConnection db)
{
    ShowcaseShape(PrimA).Insert(db, "primshapes");
    PlainShape(RootB).Insert(db, "primshapes");
    PlainShape(ChildB3).Insert(db, "primshapes");
    PlainShape(ChildB2).Insert(db, "primshapes");
    PlainShape(RootC).Insert(db, "primshapes");
    PlainShape(ChildC2).Insert(db, "primshapes");
    PlainShape(ChildC3).Insert(db, "primshapes");
    PlainShape(PrimF).Insert(db, "primshapes");
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
        .Set("Media", "<OSMedia type=\"none\"></OSMedia>")
        .Set("LastAttachPoint", 3)
        .Set("MatOvrd", new byte[] { 0x00 });
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
                extra_params = Hex(defaultBox ? shape.ExtraParams : storedShape.Extra)
            },
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

    static string Hex(byte[] bytes)
    {
        if (bytes == null)
            return null;
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }

    static float[] Vec(Vector3 value)
    {
        return new[] { value.X, value.Y, value.Z };
    }

    static float[] Quat(Quaternion value)
    {
        return new[] { value.X, value.Y, value.Z, value.W };
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

    public void Insert(SQLiteConnection db, string table)
    {
        string names = string.Join(", ", cells.ConvertAll(cell => cell.Name));
        string marks = string.Join(", ", cells.ConvertAll(cell => "@" + cell.Name));
        using SQLiteCommand command = db.CreateCommand();
        command.CommandText = "INSERT INTO " + table + " (" + names + ") VALUES (" + marks + ")";
        foreach ((string name, object value) in cells)
        {
            command.Parameters.AddWithValue("@" + name, value ?? DBNull.Value);
        }
        command.ExecuteNonQuery();
    }
}
