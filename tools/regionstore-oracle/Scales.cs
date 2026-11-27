// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Reflection;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.Framework.Scenes;

static class ScaleProbe
{
    const uint Physics = 0x1;
    const uint Phantom = 0x400;
    const uint Volume = 0x4000;

    sealed class PartSpec
    {
        public uint Flags;
        public byte Path = 16;
        public byte State;
        public byte PCode = 9;
        public byte Shape;
        public bool Volume;
        public float X, Y, Z;
    }

    sealed class Case
    {
        public string Name;
        public bool Clamp;
        public bool Collidable = true;
        public float NpMin = 0.001f, NpMax = 256f, PMin = 0.01f, PMax = 10f;
        public PartSpec[] Parts;
    }

    public static void Run(string pin)
    {
        string shortPin = ShortPin(pin);
        var scene = new Scene(new OpenSim.Framework.RegionInfo());
        var lines = new List<string>
        {
            "# Pin " + shortPin + " Shape.Scale.ToBytes after AddRestoredSceneObject(group, false, true, false).",
            "# Scene(RegionInfo): ClampPrimSize false and CollidablePrims true unless the row says otherwise.",
            "# Size fields start at Scene's defaults unless the row names them. CreatePrimUpdateBlock writes these",
            "# twelve bytes with AddVector3 at offset 28 of ObjectUpdate.ObjectData.",
            "# name index flags path state pcode shape vd clamp collidable npmin npmax pmin pmax inx iny inz out",
            "# float columns are little-endian IEEE-754 hex; out is Shape.Scale.ToBytes.",
        };
        foreach (var item in Cases())
        {
            scene.m_clampPrimSize = item.Clamp;
            scene.m_minNonphys = item.NpMin;
            scene.m_maxNonphys = item.NpMax;
            scene.m_minPhys = item.PMin;
            scene.m_maxPhys = item.PMax;
            SetCollidable(scene, item.Collidable);

            var rootSpec = item.Parts[0];
            var root = Make(rootSpec);
            // SQLiteSimulationData.buildPrim sets PhysicsShapeType before the
            // part joins its group. The setter replaces none on a root that
            // already has a parent, so the stored value is applied first.
            if (rootSpec.Shape != 0)
                root.PhysicsShapeType = rootSpec.Shape;
            var group = new SceneObjectGroup(root);
            var parts = new List<SceneObjectPart> { root };
            for (int i = 1; i < item.Parts.Length; i++)
            {
                var child = Make(item.Parts[i]);
                group.AddPart(child);
                if (item.Parts[i].Shape != 0)
                    child.PhysicsShapeType = item.Parts[i].Shape;
                parts.Add(child);
            }
            var storedFlags = new uint[parts.Count];
            var storedShape = new byte[parts.Count];
            var storedVolume = new bool[parts.Count];
            for (int i = 0; i < parts.Count; i++)
            {
                // ApplyPhysics rewrites a root whose stored shape is none. The row keeps the stored inputs.
                storedFlags[i] = (uint)parts[i].Flags;
                storedShape[i] = item.Parts[i].Shape;
                storedVolume[i] = parts[i].VolumeDetectActive;
            }
            if (!scene.AddRestoredSceneObject(group, false, true, false))
                throw new InvalidOperationException("refused " + item.Name);
            for (int i = 0; i < parts.Count; i++)
            {
                var spec = item.Parts[i];
                var scale = parts[i].Shape.Scale;
                lines.Add(string.Join(' ',
                    item.Name,
                    i.ToString(CultureInfo.InvariantCulture),
                    storedFlags[i].ToString(CultureInfo.InvariantCulture),
                    spec.Path.ToString(CultureInfo.InvariantCulture),
                    spec.State.ToString(CultureInfo.InvariantCulture),
                    spec.PCode.ToString(CultureInfo.InvariantCulture),
                    storedShape[i].ToString(CultureInfo.InvariantCulture),
                    storedVolume[i] ? "1" : "0",
                    item.Clamp ? "1" : "0",
                    item.Collidable ? "1" : "0",
                    Hex(item.NpMin), Hex(item.NpMax), Hex(item.PMin), Hex(item.PMax),
                    Hex(spec.X), Hex(spec.Y), Hex(spec.Z),
                    HexBytes(ToBytes(scale))));
            }
        }
        Console.Write(string.Join('\n', lines) + "\n");
    }

    static string ShortPin(string pin)
    {
        if (pin.Length < 10)
            throw new ArgumentException("pin is shorter than 10 hex characters");
        string shortPin = pin.Substring(0, 10).ToLowerInvariant();
        foreach (char c in shortPin)
        {
            bool hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
            if (!hex)
                throw new ArgumentException("pin is not hex");
        }
        return shortPin;
    }

    static SceneObjectPart Make(PartSpec spec)
    {
        var shape = PrimitiveBaseShape.CreateBox();
        shape.PCode = spec.PCode;
        shape.State = spec.State;
        shape.PathCurve = spec.Path;
        shape.Scale = new Vector3(spec.X, spec.Y, spec.Z);
        var part = new SceneObjectPart(
            UUID.Random(), shape, new Vector3(128f, 128f, 25f), Quaternion.Identity, Vector3.Zero);
        part.Flags = (PrimFlags)spec.Flags;
        if (spec.Volume)
            part.VolumeDetectActive = true;
        return part;
    }

    static void SetCollidable(Scene scene, bool value)
    {
        typeof(Scene).GetProperty("CollidablePrims")
            .GetSetMethod(true)
            .Invoke(scene, new object[] { value });
    }

    static string Hex(float value)
    {
        return Convert.ToHexString(BitConverter.GetBytes(value)).ToLowerInvariant();
    }

    static byte[] ToBytes(Vector3 scale)
    {
        var bytes = new byte[12];
        scale.ToBytes(bytes, 0);
        return bytes;
    }

    static string HexBytes(byte[] bytes)
    {
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }

    static Case[] Cases()
    {
        PartSpec P(float x, float y, float z, uint flags = 0, byte path = 16, byte state = 0,
            byte pcode = 9, byte shape = 0, bool volume = false) => new()
        {
            X = x, Y = y, Z = z, Flags = flags, Path = path, State = state, PCode = pcode,
            Shape = shape, Volume = volume,
        };
        return new[]
        {
            new Case { Name = "zero-nonphys", Parts = new[] { P(0, 0, 0) } },
            new Case { Name = "one-axis", Parts = new[] { P(0, 1, 2) } },
            new Case { Name = "mixed-huge", Parts = new[] { P(0, 1, 1000) } },
            new Case { Name = "negative", Parts = new[] { P(-1, 0.5f, 0.001f) } },
            new Case { Name = "infinities", Parts = new[] { P(float.NegativeInfinity, 1, float.PositiveInfinity) } },
            new Case { Name = "nan", Parts = new[] { P(float.NaN, 0, 1) } },
            new Case { Name = "physical-zero", Parts = new[] { P(0, 0, 0, Physics) } },
            new Case { Name = "physical-huge", Parts = new[] { P(0, 1, 100, Physics) } },
            new Case { Name = "phantom-zero", Parts = new[] { P(0, 0, 0, Phantom) } },
            new Case { Name = "phantom-physical", Parts = new[] { P(0, 0, 0, Phantom | Physics) } },
            new Case { Name = "phantom-volume", Parts = new[] { P(0, 0, 0, Phantom, volume: true) } },
            new Case { Name = "flexible", Parts = new[] { P(0, 0, 0, path: 128) } },
            new Case { Name = "attachment", Parts = new[] { P(0, 0, 0, state: 1) } },
            new Case { Name = "vegetation-state", Parts = new[] { P(0, 0, 0, state: 4, pcode: 95) } },
            new Case { Name = "collidable-off", Collidable = false, Parts = new[] { P(0, 0, 0) } },
            new Case { Name = "clamp-physical-zero", Clamp = true, Parts = new[] { P(0, 0, 0, Physics) } },
            new Case { Name = "clamp-physical-huge", Clamp = true, Parts = new[] { P(1000, 1, 0, Physics) } },
            new Case
            {
                Name = "clamp-reversed",
                Clamp = true,
                NpMin = 10, NpMax = 1,
                Parts = new[] { P(0, 5, 100) },
            },
            new Case
            {
                Name = "reversed-only",
                NpMin = 10, NpMax = 1,
                Parts = new[] { P(0, 5, 100) },
            },
            new Case
            {
                Name = "child-none",
                Parts = new[] { P(0, 1, 2), P(0, 0, 0, shape: 1) },
            },
            new Case
            {
                Name = "root-none",
                Parts = new[] { P(0, 0, 0, shape: 1) },
            },
            new Case
            {
                Name = "physical-link",
                Parts = new[] { P(0, 0, 0, Physics), P(0, 1, 100) },
            },
            new Case
            {
                Name = "phantom-root-volume-child",
                Parts = new[] { P(0, 0, 0, Phantom), P(0, 0, 0, volume: true) },
            },
            new Case
            {
                Name = "flexible-child",
                Parts = new[] { P(0, 0, 0), P(0, 0, 0, path: 128) },
            },
            new Case
            {
                Name = "flexible-root",
                Parts = new[] { P(0, 0, 0, path: 128), P(0, 1, 1000) },
            },
            new Case { Name = "in-range", Parts = new[] { P(0.5f, 1, 2) } },
            new Case
            {
                Name = "custom-min",
                NpMin = 0.05f, NpMax = 2,
                Parts = new[] { P(0, 1, 10) },
            },
            new Case
            {
                Name = "clamp-then-physical",
                Clamp = true,
                NpMin = 0.5f, NpMax = 20, PMin = 0.2f, PMax = 3,
                Parts = new[] { P(0, 1, 15, Physics) },
            },
            new Case
            {
                Name = "child-phantom-ignored",
                Parts = new[] { P(0, 0, 0), P(0, 0, 0, Phantom) },
            },
        };
    }
}
