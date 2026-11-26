// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Compiles JoltCharacter's capsule, mass, jump latch, movement intent and
// marker descriptor. The character is constructed against RecordingBackend, so
// CreateCharacter records the descriptor and the user data the query marker
// would copy. It does not build a native body. JoltScene is not initialised.
// A ray that would see the query marker, and the body-cap miss, need that
// native body; this fixture does not record them.

using System.Globalization;
using System.Numerics;
using System.Reflection;
using System.Text;
using OpenSim.Region.PhysicsModules.Jolt;
using OpenSim.Region.PhysicsModules.Jolt.Backend;

namespace Ferroan.JoltConfigOracle;

static class CharacterOracle
{
    const string FormatLine = "format 1";

    internal readonly record struct Texts(string Calls, string Result);

    internal static Texts Capture()
    {
        var run = new Run();
        run.StaticCapsules();
        run.CreateCharacter();
        run.Intent();
        run.LatchAndSupport();
        run.SizeAndBuoyancy();
        run.Subscription();
        run.RejectTarget();
        run.KeepCases();
        return new Texts(run.Calls.ToString(), run.Result.ToString());
    }

    sealed class Run
    {
        internal readonly StringBuilder Calls = new();
        internal readonly StringBuilder Result = new();
        readonly Type _type;
        readonly JoltScene _scene;
        readonly RecordingBackend _backend = new();
        readonly MethodInfo _capsule;
        readonly MethodInfo _mass;
        readonly MethodInfo _stand;
        readonly MethodInfo _desired;
        readonly MethodInfo _apply;
        readonly MethodInfo _jump;
        readonly ConstructorInfo _ctor;
        object _character;
        object _defaultSize;

        internal Run()
        {
            Calls.Append(FormatLine).Append('\n');
            Result.Append(FormatLine).Append('\n');
            Result.Append("method JoltCharacter\n");
            Result.Append("source JoltCharacter.cs 115-145 183-211 221-234 236-265 364-407 499-518\n");
            Result.Append("access internal\n");
            Result.Append("native not_started\n");
            Result.Append("marker_query not_recorded\n");
            Result.Append("body_cap not_recorded\n");
            Result.Append("matched_native waits\n");

            _type = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltCharacter", true);
            _capsule = Require(_type, "CapsuleFromSize", BindingFlags.Static | BindingFlags.NonPublic);
            _mass = Require(_type, "MassFor", BindingFlags.Static | BindingFlags.NonPublic);
            _stand = Require(_type, "StandHalfFor", BindingFlags.Static | BindingFlags.NonPublic);
            _desired = Require(_type, "DesiredVelocity", BindingFlags.Instance | BindingFlags.NonPublic);
            _apply = Require(_type, "ApplyCharacterState", BindingFlags.Instance | BindingFlags.NonPublic);
            _jump = Require(_type, "AvatarJump", BindingFlags.Instance | BindingFlags.Public);
            _ctor = _type.GetConstructors(BindingFlags.Instance | BindingFlags.NonPublic)
                .Single(ctor => ctor.GetParameters().Length == 9);
            _scene = new JoltScene();
            SetRegion(256, 256);
            _defaultSize = Field(null, "DefaultAvatarSize");
            float mass = (float)_type.GetField("DefaultAvatarMass", BindingFlags.Static | BindingFlags.NonPublic)
                .GetRawConstantValue();
            float edge = (float)_type.GetField("EdgeMargin", BindingFlags.Static | BindingFlags.NonPublic)
                .GetRawConstantValue();
            Result.Append("region 256 256\n");
            Result.Append("edge ").Append(Hex(edge)).Append('\n');
            Result.Append("default_mass ").Append(Hex(mass)).Append('\n');
            Result.Append("default_size ").Append(Vec(Om(_defaultSize))).Append('\n');
            AppendFactors("factors");
        }

        internal void StaticCapsules()
        {
            Capsule(_defaultSize, true);
            Capsule(Om(0.1f, 0.1f, 2f), false);
            Capsule(Om(0.5f, 0.5f, 0.2f), false);
            Capsule(Om(1.2f, 0.4f, 1.6f), false);
            Capsule(Om(-1f, 0.8f, 1f), false);
            Capsule(Om(0f, 0f, 0f), false);
            Capsule(Om(0.45f, 0.45f, 3f), false);
            // Constructor path. The Size setter rejects these before CapsuleFromSize.
            Capsule(Om(float.NaN, 1f, 1f), false);
            Capsule(Om(1f, float.NaN, 1f), false);
            Capsule(Om(1f, 1f, float.NaN), false);
            Capsule(Om(float.PositiveInfinity, float.PositiveInfinity, 1f), false);
            // r + 0.1f rounds back to r, so the 0.05 m half-height floor is visible.
            Capsule(Om(5_000_000f, 5_000_000f, 1f), false);
        }

        internal void CreateCharacter()
        {
            object[] args = _ctor.GetParameters().Select(parameter => parameter.ParameterType).ToArray()
                .Select(_ => (object)null).ToArray();
            // module, backend, local id, name, position, size, feet, flying, allow outside
            args[0] = _scene;
            args[1] = _backend;
            args[2] = 1000u;
            args[3] = "Test User";
            args[4] = Om(128f, 128f, 25f);
            args[5] = _defaultSize;
            args[6] = 0f;
            args[7] = false;
            args[8] = false;
            _character = _ctor.Invoke(args);
            if (!_backend.CreatedOnce)
                throw new InvalidOperationException("CreateCharacter was not called");
            CharacterDesc desc = _backend.Created;
            int subscribed = (int)Field(_character, "_subscribedMs");
            Calls.Append("create ").Append(desc.UserData.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Calls.Append(Vec(desc.Position)).Append(' ').Append(Vec(Om(Get("Size")))).Append(' ');
            Calls.Append(Hex(desc.Orientation.X)).Append(' ').Append(Hex(desc.Orientation.Y)).Append(' ');
            Calls.Append(Hex(desc.Orientation.Z)).Append(' ').Append(Hex(desc.Orientation.W)).Append(' ');
            Calls.Append(subscribed.ToString(CultureInfo.InvariantCulture)).Append('\n');
            Result.Append("desc");
            Result.Append(' ').Append(desc.UserData.ToString(CultureInfo.InvariantCulture));
            Result.Append(' ').Append(Vec(desc.Position));
            Result.Append(' ').Append(Hex(desc.Orientation.X)).Append(' ').Append(Hex(desc.Orientation.Y));
            Result.Append(' ').Append(Hex(desc.Orientation.Z)).Append(' ').Append(Hex(desc.Orientation.W));
            Result.Append(' ').Append(Hex(desc.CapsuleHalfHeight)).Append(' ').Append(Hex(desc.CapsuleRadius));
            Result.Append(' ').Append(Hex(desc.Mass)).Append(' ').Append(Hex(desc.JumpSpeed));
            Result.Append(' ').Append(desc.WantsContactEvents ? "1" : "0");
            Result.Append(' ').Append(Hex(desc.Friction)).Append(' ').Append(Hex(desc.MaxSlopeAngle));
            Result.Append(' ').Append(Hex(desc.StepHeight)).Append(' ').Append(Hex(desc.PushStrength));
            Result.Append(' ').Append(_backend.MarkerUserData.ToString(CultureInfo.InvariantCulture));
            Result.Append('\n');
            if (_backend.Moves.Count != 1)
                throw new InvalidOperationException("construction did not push one movement");
            AppendMove("move");
        }

        internal void Intent()
        {
            IntentCase(false, false, 4.096f, 0f, 0f);
            Set("SetAlwaysRun", true);
            IntentCase(false, true, 4.096f, 0f, 0f);
            Set("SetAlwaysRun", false);
            IntentCase(false, false, 1f, -2f, 3f);
            Set("Flying", true);
            IntentCase(true, false, 4.096f * 4f, 0f, 1f);
            IntentCase(true, false, 0f, 0f, 4f);
            IntentCase(true, false, 1f, 2f, -4f);
            IntentCase(true, false, 1f, 0f, 0f);
            // Flying ignores always-run. Both signs of Z are recorded with it on.
            Set("SetAlwaysRun", true);
            IntentCase(true, true, 4.096f * 4f, 0f, 1f);
            IntentCase(true, true, 1f, 2f, -4f);
            Set("SetAlwaysRun", false);
            Set("Flying", false);
            object config = Config();
            config.GetType().GetField("AvatarWalkSpeedFactor").SetValue(config, 2f);
            IntentCase(false, false, 4.096f, 0f, 1f);
        }

        internal void LatchAndSupport()
        {
            int moves = _backend.Moves.Count;
            Invoke(_jump, _character, new object[] { 10f });
            Result.Append("jump ").Append(Hex(10f)).Append(' ').Append(Bit(Latched())).Append(' ');
            Result.Append(Bit(_backend.Moves[^1].Jump)).Append('\n');
            if (_backend.Moves.Count != moves + 1)
                throw new InvalidOperationException("AvatarJump did not push movement");
            Set("TargetVelocity", Om(1f, 0f, 0f));
            Result.Append("hold ").Append(Bit(Latched())).Append(' ').Append(Bit(_backend.Moves[^1].Jump)).Append('\n');

            Drain(true, 0f, false, false, 128f, 128f, 25f);
            Drain(true, 0.5f, false, false, 128f, 128f, 25f);
            float above = BitConverter.Int32BitsToSingle(BitConverter.SingleToInt32Bits(0.5f) + 1);
            Drain(true, above, false, false, 128f, 128f, 25f);
            // The following drain's before bit is this latch. It is not a second jump line.
            Invoke(_jump, _character, new object[] { 10f });
            Drain(true, -1f, false, false, 128f, 128f, 25f);
            Drain(false, 0f, true, true, 128f, 128f, 25f);
            Drain(true, 0f, true, true, 128f, 128f, 25f);
            Drain(true, 0f, true, false, 128f, 128f, 25f);
            Drain(false, 0f, false, false, -1f, 300f, 10f);
        }

        internal void SizeAndBuoyancy()
        {
            Size(0.1f, 0.1f, 2f);
            Size(0.1f, 0.1f, 2f);
            Size(float.NaN, 1f, 1f);
            Size(0.4f, float.PositiveInfinity, 1f);
            Buoyancy(0f);
            Buoyancy(0.5f);
            Buoyancy(1f);
            Buoyancy(1.5f);
            Buoyancy(-0.25f);
            Buoyancy(float.NaN);
            Buoyancy(float.PositiveInfinity);
        }

        internal void Subscription()
        {
            Subscribe("SubscribeEvents", 100);
            Subscribe("SubscribeEvents", 0);
            int before = _backend.Contacts.Count;
            Invoke(_type.GetMethod("UnSubscribeEvents"), _character, null);
            Calls.Append("subscribe -1\n");
            if (_backend.Contacts.Count != before + 1)
                throw new InvalidOperationException("UnSubscribeEvents did not reach the backend");
            Result.Append("subscribe -1 ").Append(Bit(_backend.Contacts[^1])).Append('\n');
        }

        internal void RejectTarget()
        {
            Set("TargetVelocity", Om(1f, 2f, 3f));
            int before = _backend.Moves.Count;
            Set("TargetVelocity", Om(float.NaN, 2f, 3f));
            object kept = Get("TargetVelocity");
            Result.Append("reject_target ").Append(Vec(Om(kept)));
            Result.Append(' ').Append(before.ToString(CultureInfo.InvariantCulture));
            Result.Append(' ').Append(_backend.Moves.Count.ToString(CultureInfo.InvariantCulture));
            Result.Append('\n');
        }

        internal void KeepCases()
        {
            MethodInfo keep = Require(_type, "KeepInRegion", BindingFlags.Instance | BindingFlags.NonPublic);
            // A 256 by 128 region. Swapping the axes clamps a different coordinate.
            Keep(keep, false, true, 256, 128, 200f, 10f, 25f);
            Keep(keep, false, true, 256, 128, 10f, 200f, 25f);
            Keep(keep, false, true, 256, 128, 300f, -1f, 25f);
            // AllowOutsideRegion returns before the clamp and writes no transform.
            Keep(keep, true, true, 256, 256, -1f, 300f, 10f);
            // An invalid character still clamps, and writes no transform.
            Keep(keep, false, false, 256, 256, -1f, 300f, 10f);
            SetCharacter(true);
            SetRegion(256, 256);
            Set("AllowOutsideRegion", false);
        }

        void Keep(MethodInfo keep, bool allow, bool valid, int regionX, int regionY, float x, float y, float z)
        {
            SetRegion(regionX, regionY);
            Set("AllowOutsideRegion", allow);
            SetCharacter(valid);
            SetPosition(x, y, z);
            int transforms = _backend.Transforms.Count;
            Invoke(keep, _character, null);
            object position = Field(_character, "_position");
            Calls.Append("keep ").Append(Bit(allow)).Append(' ').Append(Bit(valid)).Append(' ');
            Calls.Append(regionX.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Calls.Append(regionY.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Calls.Append(Vec(new Vector3(x, y, z))).Append('\n');
            Result.Append("keep ").Append(Bit(allow)).Append(' ').Append(Bit(valid)).Append(' ');
            Result.Append(regionX.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Result.Append(regionY.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Result.Append(Vec(new Vector3(x, y, z))).Append(' ');
            Result.Append(Vec(Om(position))).Append(' ');
            Result.Append((_backend.Transforms.Count - transforms).ToString(CultureInfo.InvariantCulture));
            Result.Append('\n');
        }

        void SetCharacter(bool valid)
        {
            var id = new CharacterId(valid ? 1u : 0u);
            _type.GetField("_character", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(_character, id);
        }

        void SetPosition(float x, float y, float z)
        {
            _type.GetField("_position", BindingFlags.Instance | BindingFlags.NonPublic)
                .SetValue(_character, Om(x, y, z));
        }

        void Capsule(object size, bool isDefault)
        {
            object[] args = new object[] { size, 0f, 0f };
            Invoke(_capsule, null, args);
            float half = (float)args[1];
            float radius = (float)args[2];
            float mass = (float)Invoke(_mass, null, new object[] { size });
            float stand = (float)Invoke(_stand, null, new object[] { size });
            Calls.Append("capsule ").Append(Vec(Om(size))).Append('\n');
            Result.Append("capsule ").Append(isDefault ? "1" : "0").Append(' ').Append(Vec(Om(size)));
            Result.Append(' ').Append(Hex(radius)).Append(' ').Append(Hex(half));
            Result.Append(' ').Append(Hex(stand)).Append(' ').Append(Hex(mass)).Append('\n');
        }

        void IntentCase(bool flying, bool run, float x, float y, float z)
        {
            Set("TargetVelocity", Om(x, y, z));
            object velocity = Invoke(_desired, _character, null);
            RecordingBackend.Movement move = _backend.Moves[^1];
            Calls.Append("intent ").Append(Bit(flying)).Append(' ').Append(Bit(run)).Append(' ');
            Calls.Append(Vec(new Vector3(x, y, z))).Append('\n');
            Result.Append("intent ").Append(Bit(flying)).Append(' ').Append(Bit(run)).Append(' ');
            Result.Append(Vec(new Vector3(x, y, z))).Append(' ');
            Result.Append(Hex(Factor("AvatarWalkSpeedFactor"))).Append(' ');
            Result.Append(Hex(Factor("AvatarRunSpeedFactor"))).Append(' ');
            Result.Append(Hex(Factor("AvatarFlySpeedFactor"))).Append(' ');
            Result.Append(Hex(Factor("AvatarFlyDownSpeedFactor"))).Append(' ');
            Result.Append(Vec(move.Velocity)).Append(' ').Append(Vec(Om(velocity)));
            Result.Append(' ').Append(Bit(move.Jump)).Append(' ').Append(Bit(move.Flying)).Append('\n');
        }

        void Drain(bool supported, float vz, bool ground, bool terrain, float x, float y, float z)
        {
            int transforms = _backend.Transforms.Count;
            bool before = Latched();
            var state = new CharacterState
            {
                Position = new Vector3(x, y, z),
                LinearVelocity = new Vector3(0f, 0f, vz),
                IsSupported = supported,
                GroundIsTerrain = terrain,
                GroundBody = ground ? new BodyId(7) : BodyId.Invalid,
            };
            Invoke(_apply, _character, new object[] { state });
            object position = Field(_character, "_position");
            Calls.Append("drain ").Append(Bit(before)).Append(' ').Append(Hex(vz)).Append(' ');
            Calls.Append(Bit(supported)).Append(' ').Append(Bit(ground)).Append(' ').Append(Bit(terrain)).Append(' ');
            Calls.Append(Vec(new Vector3(x, y, z))).Append('\n');
            Result.Append("drain ").Append(Bit(before)).Append(' ').Append(Hex(vz)).Append(' ');
            Result.Append(Bit(supported)).Append(' ').Append(Bit(ground)).Append(' ').Append(Bit(terrain)).Append(' ');
            Result.Append(Vec(new Vector3(x, y, z))).Append(' ');
            Result.Append(Bit(Latched())).Append(' ');
            Result.Append(Bit((bool)Get("IsColliding"))).Append(' ');
            Result.Append(Bit((bool)Get("CollidingGround"))).Append(' ');
            Result.Append(Bit((bool)Get("CollidingObj"))).Append(' ');
            Result.Append(Vec(Om(position))).Append(' ');
            Result.Append((_backend.Transforms.Count - transforms).ToString(CultureInfo.InvariantCulture));
            Result.Append('\n');
        }

        void Size(float x, float y, float z)
        {
            int shapes = _backend.Shapes.Count;
            int masses = _backend.Masses.Count;
            Set("Size", Om(x, y, z));
            bool called = _backend.Shapes.Count == shapes + 1 && _backend.Masses.Count == masses + 1;
            if (_backend.Shapes.Count - shapes != _backend.Masses.Count - masses)
                throw new InvalidOperationException("size updated mass and shape unevenly");
            object kept = Get("Size");
            float half = (float)Field(_character, "_capsuleHalfHeight");
            float radius = (float)Field(_character, "_capsuleRadius");
            float mass = (float)Field(_character, "_mass");
            Calls.Append("size ").Append(Vec(new Vector3(x, y, z))).Append('\n');
            Result.Append("size ").Append(Vec(new Vector3(x, y, z))).Append(' ');
            Result.Append(called ? "1" : "0").Append(' ');
            Result.Append(Hex(mass)).Append(' ').Append(Hex(half)).Append(' ').Append(Hex(radius)).Append(' ');
            Result.Append(Vec(Om(kept))).Append('\n');
            if (called)
            {
                RecordingBackend.ShapeCall check = _backend.Shapes[^1];
                if (check.HalfHeight != half || check.Radius != radius || _backend.Masses[^1] != mass)
                    throw new InvalidOperationException("size backend call does not match the stored capsule");
            }
        }

        void Buoyancy(float value)
        {
            int before = _backend.Buoyancies.Count;
            Set("Buoyancy", value);
            bool forwarded = _backend.Buoyancies.Count == before + 1;
            float stored = (float)Field(_character, "_buoyancy");
            Calls.Append("buoyancy ").Append(Hex(value)).Append('\n');
            Result.Append("buoyancy ").Append(Hex(value)).Append(' ').Append(forwarded ? "1" : "0");
            Result.Append(' ').Append(Hex(stored)).Append('\n');
            if (forwarded && _backend.Buoyancies[^1] != stored)
                throw new InvalidOperationException("buoyancy backend call does not match the stored value");
        }

        void Subscribe(string method, int ms)
        {
            int before = _backend.Contacts.Count;
            Invoke(_type.GetMethod(method), _character, new object[] { ms });
            if (_backend.Contacts.Count != before + 1)
                throw new InvalidOperationException(method + " did not reach the backend");
            Calls.Append("subscribe ").Append(ms.ToString(CultureInfo.InvariantCulture)).Append('\n');
            Result.Append("subscribe ").Append(ms.ToString(CultureInfo.InvariantCulture)).Append(' ');
            Result.Append(Bit(_backend.Contacts[^1])).Append('\n');
        }

        void AppendMove(string label)
        {
            RecordingBackend.Movement move = _backend.Moves[^1];
            Result.Append(label).Append(' ').Append(Vec(move.Velocity)).Append(' ');
            Result.Append(Bit(move.Jump)).Append(' ').Append(Bit(move.Flying)).Append('\n');
        }

        void AppendFactors(string label)
        {
            Result.Append(label);
            Result.Append(' ').Append(Hex(Factor("AvatarWalkSpeedFactor")));
            Result.Append(' ').Append(Hex(Factor("AvatarRunSpeedFactor")));
            Result.Append(' ').Append(Hex(Factor("AvatarFlySpeedFactor")));
            Result.Append(' ').Append(Hex(Factor("AvatarFlyDownSpeedFactor")));
            Result.Append(' ').Append(Hex(Factor("AvatarJumpSpeed")));
            Result.Append('\n');
        }

        float Factor(string name)
        {
            PropertyInfo property = typeof(JoltScene).GetProperty(name, BindingFlags.Instance | BindingFlags.NonPublic);
            return (float)property.GetValue(_scene);
        }

        object Config()
        {
            return typeof(JoltScene).GetField("_joltConfig", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(_scene);
        }

        void SetRegion(int x, int y)
        {
            typeof(JoltScene).GetField("_regionSizeX", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(_scene, x);
            typeof(JoltScene).GetField("_regionSizeY", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(_scene, y);
        }

        bool Latched() => (bool)Field(_character, "_jumpLatched");

        object Field(object host, string name)
        {
            return _type.GetField(name, BindingFlags.Static | BindingFlags.Instance | BindingFlags.NonPublic).GetValue(host);
        }

        void Set(string property, object value)
        {
            PropertyInfo info = _type.GetProperty(property, BindingFlags.Instance | BindingFlags.Public);
            info.SetValue(_character, value);
        }

        object Get(string property)
        {
            return _type.GetProperty(property, BindingFlags.Instance | BindingFlags.Public).GetValue(_character);
        }

        static MethodInfo Require(Type type, string name, BindingFlags flags)
        {
            return type.GetMethod(name, flags) ?? throw new InvalidOperationException(name + " was not found");
        }

        static object Invoke(MethodInfo method, object host, object[] args)
        {
            try
            {
                return method.Invoke(host, args);
            }
            catch (TargetInvocationException exception)
            {
                throw exception.InnerException ?? exception;
            }
        }

        object Om(float x, float y, float z)
        {
            return Activator.CreateInstance(_capsule.GetParameters()[0].ParameterType, x, y, z);
        }

        static string Hex(float value)
            => BitConverter.SingleToInt32Bits(value).ToString("X8", CultureInfo.InvariantCulture);

        static string Bit(bool value) => value ? "1" : "0";

        static string Vec(Vector3 value) => Hex(value.X) + " " + Hex(value.Y) + " " + Hex(value.Z);

        Vector3 Om(object value)
        {
            Type type = value.GetType();
            return new Vector3(
                (float)type.GetField("X").GetValue(value),
                (float)type.GetField("Y").GetValue(value),
                (float)type.GetField("Z").GetValue(value));
        }
    }
}
