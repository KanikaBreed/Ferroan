// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Calls pinned JoltScene.DispatchContacts (JoltScene.cs 3253-3416) and the
// collision sound gate (SceneObjectPart.cs 3036-3062). Prim actors are
// uninitialized and carry no body the backend created. IsBodyAwake is the
// only backend call. Script events are not subscribed.

using System.Collections;
using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Region.PhysicsModules.Jolt;
using OpenSim.Region.PhysicsModules.Jolt.Backend;
using OpenSim.Region.PhysicsModules.SharedBase;

namespace Ferroan.JoltConfigOracle;

static class CollisionOracle
{
    internal static string Capture()
    {
        var text = new StringBuilder();
        text.Append("format 1\n");
        text.Append("method DispatchContacts\n");
        text.Append("source JoltScene.cs 3253-3416\n");
        text.Append("tracker CollisionFrameTracker.cs 45-212\n");
        text.Append("asleep JoltScene.cs 3541-3549\n");
        text.Append("sound SceneObjectPart.cs 3036-3062\n");
        text.Append("sound_play CollisionSounds.cs 170-179\n");
        text.Append("sound_play CollisionSounds.cs 219-228\n");
        text.Append("header CollisionFrameTracker.cs 1-6\n");
        text.Append("header_sound SceneObjectPart.cs 1-26\n");
        text.Append("native not_called\n");
        text.Append("scripts not_called\n");
        text.Append("min_repeat ").Append(DoubleBits(MinRepeat())).Append('\n');
        text.Append("threshold_float ").Append(FloatBits(0.2f)).Append('\n');
        text.Append("contact_cap ").Append(ContactCap().ToString(CultureInfo.InvariantCulture)).Append('\n');

        Actor[] both = { Actor.Make(1, true, true, true), Actor.Make(2, true, true, true) };
        Run(text, "substep-merge", new[]
        {
            Frame.Make(0, true, false, both, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.5f),
                PersistPair(1, 2, 4f, 5f, 6f, 0f, 1f, 0f, 1.5f)),
        });
        Run(text, "substep-keep", new[]
        {
            Frame.Make(0, false, false, both, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.5f),
                PersistPair(1, 2, 4f, 5f, 6f, 0f, 1f, 0f, 1.5f)),
        });
        Run(text, "end-ignored", new[]
        {
            Frame.Make(0, false, false, both, Contact.Make(ContactPhase.End, 1, 1, 2, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.5f)),
        });

        Actor[] one = { Actor.Make(1, true, true, true) };
        Run(text, "held-terrain", new[]
        {
            Frame.Make(5, false, false, one, Ground(1, -0.4f)),
            Frame.Make(5.01, false, false, new[] { Actor.Make(1, true, true, false) }),
        });
        Run(text, "awake-ground-ends", new[]
        {
            Frame.Make(5, false, false, one, Ground(1, -0.4f)),
            Frame.Make(5.01, false, false, one),
        });
        Run(text, "static-ground-ends", new[]
        {
            Frame.Make(5, false, false, new[] { Actor.Make(1, true, false, true) }, Ground(1, -0.4f)),
            Frame.Make(5.01, false, false, new[] { Actor.Make(1, true, false, false) }),
        });

        Actor[] pairAwake = { Actor.Make(4, true, true, true), Actor.Make(5, true, true, true) };
        Run(text, "sleeping-pair", new[]
        {
            Frame.Make(8, false, false, pairAwake, PersistPair(4, 5, 1f, 2f, 3f, 0f, 0f, 1f, 0.25f)),
            Frame.Make(8.01, false, false, new[] { Actor.Make(4, true, true, false), Actor.Make(5, true, true, false) }),
        });
        Run(text, "waking-pair", new[]
        {
            Frame.Make(8, false, false, pairAwake, PersistPair(4, 5, 1f, 2f, 3f, 0f, 0f, 1f, 0.25f)),
            Frame.Make(8.01, false, false, new[] { Actor.Make(4, true, true, true), Actor.Make(5, true, true, false) }),
        });

        Actor[] five =
        {
            Actor.Make(1, true, true, true), Actor.Make(2, true, true, true), Actor.Make(3, true, true, true),
            Actor.Make(4, true, true, true), Actor.Make(5, true, true, true),
        };
        Run(text, "overflow-partial", new[]
        {
            Frame.Make(30, false, false, five,
                PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.3f),
                PersistPair(1, 3, 2f, 0f, 0f, 0f, 0f, 1f, 0.4f),
                PersistPair(4, 5, 3f, 0f, 0f, 0f, 0f, 1f, 0.6f)),
            Frame.Make(30.01, false, true, five, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.3f)),
        });

        double origin = 100;
        double inside = origin + (0.13 - 1e-9) - 1e-12;
        double edge = origin + (0.13 - 1e-9);
        double changed = edge + 1e-6;
        Actor[] trio = { Actor.Make(1, true, true, true), Actor.Make(2, true, true, true), Actor.Make(3, true, true, true) };
        Run(text, "repeat-window", new[]
        {
            Frame.Make(origin, false, false, trio, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.7f)),
            Frame.Make(inside, false, false, trio, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.7f)),
            Frame.Make(edge, false, false, trio, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.7f)),
            Frame.Make(changed, false, false, trio, PersistPair(1, 3, 1f, 0f, 0f, 0f, 0f, 1f, 0.7f)),
        });

        Run(text, "score-unsubscribed", new[]
        {
            Frame.Make(0, false, false, new[] { Actor.Make(1, true, true, true), Actor.Make(2, false, true, true) },
                PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.4f)),
        });

        Actor[] phase =
        {
            Actor.Make(1, true, true, true), Actor.Make(2, true, true, true), Actor.Make(3, true, true, true),
        };
        Run(text, "phase-order", new[]
        {
            Frame.Make(0, false, false, phase, PersistPair(1, 2, 1f, 0f, 0f, 0f, 0f, 1f, 0.5f)),
            Frame.Make(1, false, false, phase, PersistPair(2, 3, 2f, 0f, 0f, 0f, 0f, 1f, 0.6f)),
        });

        text.Append(SoundText());
        return text.ToString();
    }

    // JoltScene.cs 308 initializes _contactBufMax to 2048. AddRegion replaces it
    // from config or the backend ring. This records the value before that call.
    static int ContactCap()
    {
        var scene = new JoltScene();
        FieldInfo field = typeof(JoltScene).GetField("_contactBufMax", BindingFlags.Instance | BindingFlags.NonPublic);
        if (field == null)
            throw new InvalidOperationException("missing _contactBufMax");
        return (int)field.GetValue(scene);
    }

    static Contact Ground(uint prim, float speed)
        => Contact.Make(ContactPhase.Persist, prim, prim, 0, 0, 0f, 0f, 1f, 0f, 0f, 1f, speed);

    static Contact PersistPair(uint a, uint b, float px, float py, float pz, float nx, float ny, float nz, float speed)
        => Contact.Make(ContactPhase.Persist, a, a, b, b, px, py, pz, nx, ny, nz, speed);

    static void Run(StringBuilder text, string name, Frame[] frames)
    {
        text.Append('\n').Append("case ").Append(name).Append('\n');
        var backend = new RecordingBackend();
        var scene = new JoltScene();
        IDictionary prims = Prims(scene);
        var actors = new SortedDictionary<uint, object>();
        foreach (Frame frame in frames)
        {
            foreach (Actor actor in frame.Actors)
            {
                if (actors.ContainsKey(actor.Id))
                    continue;
                object prim = NewPrim(backend, actor.Id);
                actors.Add(actor.Id, prim);
                prims[actor.Id] = prim;
            }
        }

        var sends = new List<Sent>();
        foreach (KeyValuePair<uint, object> kv in actors)
        {
            uint id = kv.Key;
            EventInfo collision = kv.Value.GetType().GetEvent("OnCollisionUpdate");
            if (collision == null)
                throw new InvalidOperationException("OnCollisionUpdate is missing");
            collision.AddEventHandler(kv.Value, new PhysicsActor.CollisionUpdate(args =>
            {
                sends.Add(Sent.Copy(id, (CollisionEventUpdate)args));
            }));
        }

        for (int i = 0; i < frames.Length; i++)
        {
            Frame frame = frames[i];
            foreach (Actor actor in frame.Actors)
            {
                object prim = actors[actor.Id];
                Set(prim, "_isPhysical", actor.Physical);
                Set(prim, "_subscribedMs", actor.Subscribed ? 50 : 0);
                backend.AwakeBodies[actor.Id] = actor.Awake;
            }
            Set(scene, "SimTime", frame.Time);
            sends.Clear();
            ContactReport[] reports = new ContactReport[frame.Contacts.Length];
            for (int c = 0; c < reports.Length; c++)
                reports[c] = frame.Contacts[c].Report();
            InvokeDispatch(scene, reports, frame.Overflow, frame.Merge);
            WriteFrame(text, i + 1, frame, sends, actors, scene);
        }
    }

    static void WriteFrame(StringBuilder text, int index, Frame frame, List<Sent> sends, SortedDictionary<uint, object> actors, JoltScene scene)
    {
        text.Append("frame ").Append(index.ToString(CultureInfo.InvariantCulture)).Append('\n');
        text.Append("time ").Append(DoubleBits(frame.Time)).Append('\n');
        text.Append("merge ").Append(frame.Merge ? "1" : "0").Append('\n');
        text.Append("overflow ").Append(frame.Overflow ? "1" : "0").Append('\n');
        Actor[] ordered = frame.Actors.OrderBy(actor => actor.Id).ToArray();
        foreach (Actor actor in ordered)
        {
            text.Append("actor ").Append(actor.Id.ToString(CultureInfo.InvariantCulture));
            text.Append(actor.Subscribed ? " 1" : " 0");
            text.Append(actor.Physical ? " 1" : " 0");
            text.Append(actor.Awake ? " 1\n" : " 0\n");
        }
        foreach (Contact contact in frame.Contacts)
            contact.Write(text);

        text.Append("phase current\n");
        WriteSends(text, sends.Where(sent => sent.Colliders.Count > 0));
        text.Append("phase end\n");
        WriteSends(text, sends.Where(sent => sent.Colliders.Count == 0));

        foreach (Actor actor in ordered)
        {
            object prim = actors[actor.Id];
            float score = (float)prim.GetType().GetProperty("CollisionScore").GetValue(prim);
            if (score != (int)score)
                throw new InvalidOperationException("collision score is not an integer");
            text.Append("score ").Append(actor.Id.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(((int)score).ToString(CultureInfo.InvariantCulture)).Append('\n');
        }

        HashSet<uint> tracked = Tracked(scene);
        text.Append("tracked");
        foreach (uint id in tracked.OrderBy(id => id))
            text.Append(' ').Append(id.ToString(CultureInfo.InvariantCulture));
        text.Append('\n');

        IDictionary current = Current(scene);
        var ids = new List<uint>();
        foreach (DictionaryEntry entry in current)
            ids.Add((uint)entry.Key);
        ids.Sort();
        foreach (uint id in ids)
        {
            var update = (CollisionEventUpdate)current[id];
            text.Append("current ").Append(id.ToString(CultureInfo.InvariantCulture)).Append('\n');
            var colliders = new List<Collider>();
            foreach (KeyValuePair<uint, ContactPoint> kv in update.m_objCollisionList)
                colliders.Add(Collider.From(kv.Key, kv.Value));
            foreach (Collider collider in colliders.OrderBy(collider => collider.Id))
                collider.Write(text, "held");
        }
        text.Append("endframe\n");
    }

    static void WriteSends(StringBuilder text, IEnumerable<Sent> sends)
    {
        foreach (Sent sent in sends.OrderBy(sent => sent.Prim))
        {
            text.Append("send ").Append(sent.Prim.ToString(CultureInfo.InvariantCulture));
            if (sent.Colliders.Count == 0)
            {
                text.Append(" empty\n");
                continue;
            }
            text.Append('\n');
            foreach (Collider collider in sent.Colliders.OrderBy(collider => collider.Id))
            {
                if (collider.Penetration != 0f || !collider.Feet)
                    throw new InvalidOperationException("prim contact was not the dispatcher shape");
                collider.Write(text, "sent");
            }
        }
    }

    static string SoundText()
    {
        var text = new StringBuilder();
        text.Append("\ncase sound\n");
        float boundary = 0.2f;
        float below = BitConverter.Int32BitsToSingle(BitConverter.SingleToInt32Bits(boundary) - 1);
        float above = BitConverter.Int32BitsToSingle(BitConverter.SingleToInt32Bits(boundary) + 1);
        float[] speeds = { boundary, below, above, -boundary, -below, -above, 0f, 1f, -1f };
        foreach (float speed in speeds)
        {
            Probe(text, "land", "fresh", speed, false);
            Probe(text, "object", "fresh", speed, false);
        }
        Probe(text, "land", "again", 1f, true);
        Probe(text, "object", "again", 1f, true);
        return text.ToString();
    }

    static void Probe(StringBuilder text, string kind, string when, float speed, bool second)
    {
        if (kind != "land" && kind != "object")
            throw new InvalidOperationException("unknown sound kind");
        var scene = new Scene(new OpenSim.Framework.RegionInfo());
        var heard = new RecordingSound();
        scene.RegisterModuleInterface<ISoundModule>(heard);
        uint other = kind == "land" ? 0u : 2u;
        var part = new SceneObjectPart();
        part.LocalId = 1;
        part.Flags = PrimFlags.Physics;
        var group = new SceneObjectGroup(part);
        Set(group, "m_scene", scene);
        if (kind == "object")
        {
            var partner = new SceneObjectPart();
            partner.LocalId = other;
            partner.Flags = PrimFlags.Physics;
            var partnerGroup = new SceneObjectGroup(partner);
            Set(partnerGroup, "m_scene", scene);
            Set(partnerGroup, "m_lastCollisionSoundMS", 0d);
            IDictionary parts = Parts(scene);
            parts[other] = partner;
        }
        int result = Strike(heard, group, part, other, speed);
        if (second)
            result = Strike(heard, group, part, other, speed);
        text.Append("probe ").Append(kind).Append(' ').Append(when).Append(' ');
        text.Append(FloatBits(speed)).Append(' ');
        text.Append(result.ToString(CultureInfo.InvariantCulture)).Append('\n');
    }

    static int Strike(RecordingSound heard, SceneObjectGroup group, SceneObjectPart part, uint other, float speed)
    {
        Set(group, "m_lastCollisionSoundMS", 0d);
        Set(part, "LastColSoundSentTime", Util.EnvironmentTickCount() - 10000);
        heard.Plays = 0;
        var update = new CollisionEventUpdate();
        update.AddCollider(other, new ContactPoint(
            new OpenMetaverse.Vector3(1f, 2f, 3f),
            new OpenMetaverse.Vector3(0f, 0f, 1f),
            0f)
        { RelativeSpeed = speed });
        part.PhysicsCollision(update);
        if (heard.Plays > 1)
            throw new InvalidOperationException("one contact played more than one sound");
        return heard.Plays;
    }

    static object NewPrim(RecordingBackend backend, uint id)
    {
        Type primType = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.JoltPrim");
        if (primType == null)
            throw new InvalidOperationException("JoltPrim is missing");
        object prim = RuntimeHelpers.GetUninitializedObject(primType);
        Set(prim, "_backend", backend);
        Set(prim, "_body", new BodyId(id));
        Set(prim, "_isPhysical", false);
        Set(prim, "_subscribedMs", 0);
        Set(prim, "_isVolumeDetect", false);
        primType.GetProperty("LocalID").SetValue(prim, id);
        return prim;
    }

    static void InvokeDispatch(JoltScene scene, ContactReport[] reports, bool overflow, bool merge)
    {
        MethodInfo method = typeof(JoltScene).GetMethod("DispatchContacts", BindingFlags.Instance | BindingFlags.NonPublic);
        if (method == null)
            throw new InvalidOperationException("DispatchContacts is missing");
        try
        {
            method.Invoke(scene, new object[] { reports, reports.Length, overflow, merge });
        }
        catch (TargetInvocationException exception)
        {
            throw exception.InnerException ?? exception;
        }
    }

    static IDictionary Parts(Scene scene)
    {
        object graph = typeof(Scene).GetProperty("SceneGraph").GetValue(scene);
        FieldInfo field = graph.GetType().GetField("m_scenePartsByLocalID", BindingFlags.Instance | BindingFlags.NonPublic);
        if (field == null)
            throw new InvalidOperationException("m_scenePartsByLocalID is missing");
        return (IDictionary)field.GetValue(graph);
    }

    static IDictionary Prims(JoltScene scene)
    {
        PropertyInfo property = typeof(JoltScene).GetProperty("Prims", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
        return (IDictionary)property.GetValue(scene);
    }

    static HashSet<uint> Tracked(JoltScene scene)
    {
        object tracker = typeof(JoltScene).GetField("_collisions", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(scene);
        PropertyInfo property = tracker.GetType().GetProperty("CollidedLastFrame", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
        return (HashSet<uint>)property.GetValue(tracker);
    }

    static IDictionary Current(JoltScene scene)
    {
        object tracker = typeof(JoltScene).GetField("_collisions", BindingFlags.Instance | BindingFlags.NonPublic).GetValue(scene);
        PropertyInfo property = tracker.GetType().GetProperty("Current", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
        return (IDictionary)property.GetValue(tracker);
    }

    static double MinRepeat()
    {
        Type tracker = typeof(JoltScene).Assembly.GetType("OpenSim.Region.PhysicsModules.Jolt.CollisionFrameTracker");
        FieldInfo field = tracker.GetField("MinRepeatSeconds", BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public);
        return (double)field.GetValue(null);
    }

    static void Set(object target, string name, object value)
    {
        for (Type type = target.GetType(); type != null; type = type.BaseType)
        {
            FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly);
            if (field == null)
                continue;
            field.SetValue(target, value);
            return;
        }
        throw new InvalidOperationException(name + " is missing");
    }

    static string FloatBits(float value)
        => unchecked((uint)BitConverter.SingleToInt32Bits(value)).ToString("X8", CultureInfo.InvariantCulture);

    static string DoubleBits(double value)
        => unchecked((ulong)BitConverter.DoubleToInt64Bits(value)).ToString("X16", CultureInfo.InvariantCulture);

    sealed class Actor
    {
        internal uint Id;
        internal bool Subscribed;
        internal bool Physical;
        internal bool Awake;

        internal static Actor Make(uint id, bool subscribed, bool physical, bool awake)
            => new Actor { Id = id, Subscribed = subscribed, Physical = physical, Awake = awake };
    }

    sealed class Contact
    {
        internal ContactPhase Phase;
        internal uint ChildA;
        internal uint UserA;
        internal uint ChildB;
        internal uint UserB;
        internal float Px, Py, Pz, Nx, Ny, Nz, Speed;

        internal static Contact Make(ContactPhase phase, uint childA, uint userA, uint childB, uint userB,
            float px, float py, float pz, float nx, float ny, float nz, float speed)
            => new Contact
            {
                Phase = phase,
                ChildA = childA,
                UserA = userA,
                ChildB = childB,
                UserB = userB,
                Px = px,
                Py = py,
                Pz = pz,
                Nx = nx,
                Ny = ny,
                Nz = nz,
                Speed = speed,
            };

        internal ContactReport Report()
            => new ContactReport
            {
                BodyA = new BodyId(1),
                BodyB = new BodyId(1),
                ChildUserDataA = ChildA,
                UserDataA = UserA,
                ChildUserDataB = ChildB,
                UserDataB = UserB,
                Point = new System.Numerics.Vector3(Px, Py, Pz),
                Normal = new System.Numerics.Vector3(Nx, Ny, Nz),
                RelativeSpeed = Speed,
                Phase = Phase,
            };

        internal void Write(StringBuilder text)
        {
            text.Append("contact ").Append(PhaseName()).Append(' ');
            text.Append(ChildA.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(UserA.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(ChildB.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(UserB.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(FloatBits(Px)).Append(' ').Append(FloatBits(Py)).Append(' ').Append(FloatBits(Pz)).Append(' ');
            text.Append(FloatBits(Nx)).Append(' ').Append(FloatBits(Ny)).Append(' ').Append(FloatBits(Nz)).Append(' ');
            text.Append(FloatBits(Speed)).Append('\n');
        }

        string PhaseName()
        {
            if (Phase == ContactPhase.Begin)
                return "begin";
            if (Phase == ContactPhase.Persist)
                return "persist";
            if (Phase == ContactPhase.End)
                return "end";
            throw new InvalidOperationException("unknown contact phase");
        }
    }

    sealed class Frame
    {
        internal double Time;
        internal bool Merge;
        internal bool Overflow;
        internal Actor[] Actors;
        internal Contact[] Contacts;

        internal static Frame Make(double time, bool merge, bool overflow, Actor[] actors, params Contact[] contacts)
            => new Frame { Time = time, Merge = merge, Overflow = overflow, Actors = actors, Contacts = contacts ?? Array.Empty<Contact>() };
    }

    sealed class Collider
    {
        internal uint Id;
        internal float Px, Py, Pz, Nx, Ny, Nz, Speed, Penetration;
        internal bool Feet;

        internal static Collider From(uint id, ContactPoint point)
            => new Collider
            {
                Id = id,
                Px = point.Position.X,
                Py = point.Position.Y,
                Pz = point.Position.Z,
                Nx = point.SurfaceNormal.X,
                Ny = point.SurfaceNormal.Y,
                Nz = point.SurfaceNormal.Z,
                Speed = point.RelativeSpeed,
                Penetration = point.PenetrationDepth,
                Feet = point.CharacterFeet,
            };

        internal void Write(StringBuilder text, string label)
        {
            text.Append(label).Append(' ').Append(Id.ToString(CultureInfo.InvariantCulture)).Append(' ');
            text.Append(FloatBits(Px)).Append(' ').Append(FloatBits(Py)).Append(' ').Append(FloatBits(Pz)).Append(' ');
            text.Append(FloatBits(Nx)).Append(' ').Append(FloatBits(Ny)).Append(' ').Append(FloatBits(Nz)).Append(' ');
            text.Append(FloatBits(Speed)).Append('\n');
        }
    }

    sealed class Sent
    {
        internal uint Prim;
        internal List<Collider> Colliders;

        internal static Sent Copy(uint prim, CollisionEventUpdate update)
        {
            var sent = new Sent { Prim = prim, Colliders = new List<Collider>() };
            foreach (KeyValuePair<uint, ContactPoint> kv in update.m_objCollisionList)
                sent.Colliders.Add(Collider.From(kv.Key, kv.Value));
            return sent;
        }
    }

    sealed class RecordingSound : ISoundModule
    {
        internal int Plays;
        public float MaxDistance => 0f;
        public void PlayAttachedSound(UUID soundID, UUID ownerID, UUID objectID, double gain, OpenMetaverse.Vector3 position, byte flags) { }
        public void TriggerSound(UUID soundId, UUID ownerID, UUID objectID, UUID parentID, double gain, OpenMetaverse.Vector3 position, ulong handle) { }
        public void TriggerCollisionSound(UUID soundId, UUID ownerID, UUID objectID, UUID parentID, double gain, OpenMetaverse.Vector3 position, ulong handle)
        {
            Plays++;
        }
        public void StopSound(UUID objectID) { }
        public void StopSound(SceneObjectPart sop) { }
        public void PreloadSound(UUID objectID, UUID soundID) { }
        public void PreloadSound(SceneObjectPart sop, UUID soundID) { }
        public void LoopSound(UUID objectID, UUID soundID, double gain, bool isMaster, bool isSlave) { }
        public void LoopSound(SceneObjectPart sop, UUID soundID, double gain, bool isMaster, bool isSlave) { }
        public void SendSound(UUID objectID, UUID sound, double volume, bool triggered, byte flags, bool useMaster, bool isMaster) { }
        public void SendSound(SceneObjectPart sop, UUID sound, double volume, bool triggered, byte flags, bool useMaster, bool isMaster) { }
        public void TriggerSoundLimited(UUID objectID, UUID sound, double volume, OpenMetaverse.Vector3 min, OpenMetaverse.Vector3 max) { }
        public void SetSoundQueueing(UUID objectID, bool shouldQueue) { }
    }
}
