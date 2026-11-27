// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Records the character descriptor and the calls JoltCharacter makes while the
// file oracle compiles it. Analytic box, sphere and cylinder constructors are
// recorded only when RecordAnalytic is set. SetTerrain's heightfield cook is
// recorded only when RecordTerrain is set. Gravity-factor writes, and the
// vehicle body's friction, damping, sleep, CCD and step-provider calls that
// surround them, are recorded only when RecordGravity is set. Every other
// backend method is unexpected: this process must not reach
// JoltPhysicsBackend, and it must not load joltc. RecordHeartbeat lets
// Simulate return without a native step. It stays off for the character
// compile.
using System;
using System.Collections.Generic;
using System.Numerics;
using OpenSim.Region.PhysicsModules.Jolt.Backend;

namespace Ferroan.JoltConfigOracle;

sealed class RecordingBackend : IPhysicsBackend
{
    internal bool CreatedOnce;
    internal CharacterDesc Created;
    // JoltPhysicsBackend.cs 3070 and 3090 copy desc.UserData onto the query
    // marker. Creating that body needs native. This is the assignment.
    internal uint MarkerUserData;
    internal readonly List<Movement> Moves = new();
    internal readonly List<ShapeCall> Shapes = new();
    internal readonly List<float> Masses = new();
    internal readonly List<float> Buoyancies = new();
    internal readonly List<bool> Contacts = new();
    internal readonly List<Vector3> Transforms = new();
    // Simulate's budget, step and capacity reads. Off unless the dilation
    // capture is proving that a successful Simulate returns 1.
    internal bool RecordHeartbeat;
    internal int HeartbeatSteps;

    // Off for the character compile. An analytic case sets this and records the
    // dimensions CookPrimShape passes. A vertex array is still unexpected.
    internal bool RecordAnalytic;
    internal int BoxCalls;
    internal int SphereCalls;
    internal int CylinderCalls;
    internal Vector3 BoxHalf;
    internal float SphereRadius;
    internal float CylinderHalfHeight;
    internal float CylinderRadius;

    // Off for the character compile. A heightfield case sets this, then
    // SetTerrain copies the span it was given and returns without cooking.
    internal bool RecordTerrain;
    internal int HeightFieldCalls;
    internal float[] LastHeights = Array.Empty<float>();
    internal int LastSampleCountX;
    internal int LastSampleCountY;
    internal Vector3 LastScale;
    internal int TerrainSets;
    internal ShapeId LastTerrainShape;
    internal Vector3 LastTerrainPosition;

    // Off for the character compile. A gravity case sets this and records
    // SetBodyGravityFactor. The vehicle path's other body calls are ignored
    // only while this is set, so a character compile still fails closed.
    internal bool RecordGravity;
    internal int Activates;
    internal readonly List<GravityWrite> GravityWrites = new();

    internal readonly record struct GravityWrite(uint Body, float Factor);
    internal readonly record struct Movement(Vector3 Velocity, bool Jump, bool Flying);
    internal readonly record struct ShapeCall(float HalfHeight, float Radius);

    public string Name => "recording";
    public string Version => "0";
    public void Dispose() { }

    public CharacterId CreateCharacter(in CharacterDesc desc)
    {
        if (CreatedOnce)
            throw new InvalidOperationException("the character compile created a second character");
        CreatedOnce = true;
        Created = desc;
        MarkerUserData = desc.UserData;
        return new CharacterId(1);
    }

    public void SetCharacterMovement(CharacterId character, Vector3 desiredVelocity, bool jump, bool flying)
    {
        Moves.Add(new Movement(desiredVelocity, jump, flying));
    }

    public void SetCharacterShape(CharacterId character, float capsuleHalfHeight, float capsuleRadius)
    {
        Shapes.Add(new ShapeCall(capsuleHalfHeight, capsuleRadius));
    }

    public void SetCharacterMass(CharacterId character, float mass)
    {
        Masses.Add(mass);
    }

    public void SetCharacterBuoyancy(CharacterId character, float buoyancy)
    {
        Buoyancies.Add(buoyancy);
    }

    public void SetCharacterWantsContactEvents(CharacterId character, bool wants)
    {
        Contacts.Add(wants);
    }

    public void SetCharacterTransform(CharacterId character, Vector3 position, Quaternion orientation)
    {
        Transforms.Add(position);
    }

    public void Initialize(in PhysicsBackendSettings settings)
    {
        Unexpected();
    }
    public ShapeId CreateBoxShape(Vector3 halfExtents)
    {
        if (!RecordAnalytic)
            return Unexpected<ShapeId>();
        BoxCalls++;
        BoxHalf = halfExtents;
        return new ShapeId((uint)BoxCalls);
    }
    public ShapeId CreateSphereShape(float radius)
    {
        if (!RecordAnalytic)
            return Unexpected<ShapeId>();
        SphereCalls++;
        SphereRadius = radius;
        return new ShapeId((uint)SphereCalls);
    }
    public ShapeId CreateCapsuleShape(float halfHeight, float radius)
    {
        return Unexpected<ShapeId>();
    }
    public ShapeId CreateCylinderShape(float halfHeight, float radius)
    {
        if (!RecordAnalytic)
            return Unexpected<ShapeId>();
        CylinderCalls++;
        CylinderHalfHeight = halfHeight;
        CylinderRadius = radius;
        return new ShapeId((uint)CylinderCalls);
    }
    public ShapeId CreateConvexHullShape(ReadOnlySpan<Vector3> points)
    {
        return Unexpected<ShapeId>();
    }
    public ShapeId CreateMeshShape(ReadOnlySpan<Vector3> vertices, ReadOnlySpan<int> indices)
    {
        return Unexpected<ShapeId>();
    }
    public ShapeId CreateCompoundShape(ReadOnlySpan<CompoundChild> children)
    {
        return Unexpected<ShapeId>();
    }
    public ShapeId CreateHullCompoundShape(ReadOnlySpan<ShapeId> hulls)
    {
        return Unexpected<ShapeId>();
    }
    public ShapeId CreateHeightFieldShape(ReadOnlySpan<float> heights, int sampleCountX, int sampleCountY, Vector3 scale)
    {
        if (!RecordTerrain)
            return Unexpected<ShapeId>();
        HeightFieldCalls++;
        LastHeights = heights.ToArray();
        LastSampleCountX = sampleCountX;
        LastSampleCountY = sampleCountY;
        LastScale = scale;
        LastTerrainShape = new ShapeId((uint)HeightFieldCalls);
        return LastTerrainShape;
    }
    public ShapeId CreateScaledShape(ShapeId baseShape, Vector3 scale)
    {
        return Unexpected<ShapeId>();
    }
    public bool IsScaleValid(ShapeId shape, Vector3 scale)
    {
        return Unexpected<bool>();
    }
    public void AddShapeRef(ShapeId shape)
    {
        Unexpected();
    }
    public void ReleaseShape(ShapeId shape)
    {
        Unexpected();
    }
    public BodyId CreateBody(in BodyDesc desc)
    {
        return Unexpected<BodyId>();
    }
    public void RemoveBody(BodyId body)
    {
        Unexpected();
    }
    public bool IsBodyValid(BodyId body)
    {
        return Unexpected<bool>();
    }
    public IDisposable AttachStepForceProvider(BodyId body, IStepForceProvider provider)
    {
        if (!RecordGravity)
            return Unexpected<IDisposable>();
        return new GravityAttachment();
    }
    public void SetBodyShape(BodyId body, ShapeId shape, bool recomputeMass)
    {
        Unexpected();
    }
    public void SetBodyMotionType(BodyId body, BodyMotionType motionType, bool activate)
    {
        Unexpected();
    }
    public void SetBodyLayer(BodyId body, PhysicsLayer layer)
    {
        Unexpected();
    }
    public void SetBodySensor(BodyId body, bool isSensor)
    {
        Unexpected();
    }
    public void SetBodyTransform(BodyId body, Vector3 position, Quaternion orientation, bool activate)
    {
        Unexpected();
    }
    public void MoveKinematicBody(BodyId body, Vector3 position, Quaternion orientation, float seconds)
    {
        Unexpected();
    }
    public void SetBodyPoseAndVelocity(BodyId body, Vector3 position, Quaternion orientation, Vector3 linearVelocity, Vector3 angularVelocity)
    {
        Unexpected();
    }
    public void SetBodyLinearVelocity(BodyId body, Vector3 velocity)
    {
        Unexpected();
    }
    public void SetBodyAngularVelocity(BodyId body, Vector3 velocity)
    {
        Unexpected();
    }
    public void SetBodyMass(BodyId body, float mass)
    {
        Unexpected();
    }
    public float GetBodyMass(BodyId body)
    {
        return Unexpected<float>();
    }
    public bool GetBodyEnhancedEdgeRemoval(BodyId body)
    {
        return Unexpected<bool>();
    }
    public bool GetCharacterEnhancedEdgeRemoval(CharacterId character)
    {
        return Unexpected<bool>();
    }
    public Vector3 GetBodyInertiaDiagonal(BodyId body)
    {
        return Unexpected<Vector3>();
    }
    public float GetBodyLinearDamping(BodyId body)
    {
        return Unexpected<float>();
    }
    public void SetBodyDensity(BodyId body, float physicalDensity)
    {
        Unexpected();
    }
    public void SetBodyFriction(BodyId body, float friction)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public void SetBodyRestitution(BodyId body, float restitution)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public void SetBodyPartMaterial(BodyId body, uint partUserData, float friction, float restitution)
    {
        Unexpected();
    }
    public void SetBodyFrictionMin(BodyId body, bool min)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public float GetShapeVolume(ShapeId shape)
    {
        return Unexpected<float>();
    }
    public Vector3 GetShapeCenterOfMass(ShapeId shape)
    {
        return Unexpected<Vector3>();
    }
    public void SetShapeDensity(ShapeId shape, float density)
    {
        Unexpected();
    }
    public float GetShapeDensity(ShapeId shape)
    {
        return Unexpected<float>();
    }
    public void SetBodyDamping(BodyId body, float linear, float angular)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public void SetBodyGravityFactor(BodyId body, float factor)
    {
        if (!RecordGravity)
            Unexpected();
        GravityWrites.Add(new GravityWrite(body.Value, factor));
    }
    public void SetBodyRotationLocks(BodyId body, bool lockX, bool lockY, bool lockZ)
    {
        Unexpected();
    }
    public void ApplyForce(BodyId body, Vector3 force)
    {
        Unexpected();
    }
    public void ApplyTorque(BodyId body, Vector3 torque)
    {
        Unexpected();
    }
    public void ApplyImpulse(BodyId body, Vector3 impulse)
    {
        Unexpected();
    }
    public void ApplyImpulseAtPoint(BodyId body, Vector3 impulse, Vector3 worldPoint)
    {
        Unexpected();
    }
    public void ApplyAngularImpulse(BodyId body, Vector3 angularImpulse)
    {
        Unexpected();
    }
    public void ApplyBuoyancy(BodyId body, float waterHeight, float buoyancy, float linearDrag, float angularDrag)
    {
        Unexpected();
    }
    public void ActivateBody(BodyId body)
    {
        if (!RecordGravity)
            Unexpected();
        Activates++;
    }
    public void DeactivateBody(BodyId body)
    {
        Unexpected();
    }
    public void SetBodyAllowSleeping(BodyId body, bool allow)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public void SetBodyContinuousCollision(BodyId body, ContinuousCollision mode)
    {
        if (!RecordGravity)
            Unexpected();
    }
    public void SetBodyWantsContactEvents(BodyId body, bool wants)
    {
        Unexpected();
    }
    public bool BodyHadContact(BodyId body)
    {
        return Unexpected<bool>();
    }
    public bool IsBodyAwake(BodyId body)
    {
        return Unexpected<bool>();
    }
    public bool TryGetBodyUserData(BodyId body, out uint userData)
    {
        userData = default;
        return Unexpected<bool>();
    }
    public bool TryGetBodyState(BodyId body, out BodyState state)
    {
        state = default;
        return Unexpected<bool>();
    }
    public void RemoveCharacter(CharacterId character)
    {
        Unexpected();
    }
    public void ReGroundCharacter(CharacterId character, Vector3 position)
    {
        Unexpected();
    }
    public void AddCharacterImpulse(CharacterId character, Vector3 velocityChange)
    {
        Unexpected();
    }
    public void SetCharacterCarry(CharacterId character, Vector3 velocity)
    {
        Unexpected();
    }
    public void SetCharacterHover(CharacterId character, Func<float, float, float> heightAt, float tau)
    {
        Unexpected();
    }
    public void SetCharacterConstantForce(CharacterId character, Vector3 acceleration)
    {
        Unexpected();
    }
    public bool TryGetCharacterState(CharacterId character, out CharacterState state)
    {
        state = default;
        return Unexpected<bool>();
    }
    public void SetGravity(Vector3 gravity)
    {
        Unexpected();
    }
    public void SetTerrain(ShapeId heightFieldShape, Vector3 position)
    {
        if (!RecordTerrain)
            Unexpected();
        TerrainSets++;
        LastTerrainShape = heightFieldShape;
        LastTerrainPosition = position;
    }
    public void SetWaterHeight(float height)
    {
        Unexpected();
    }
    public bool RayCast(Vector3 origin, Vector3 direction, float maxDistance, QueryFilter filter, out RayHit hit)
    {
        hit = default;
        return Unexpected<bool>();
    }
    public int RayCastAll(Vector3 origin, Vector3 direction, float maxDistance, QueryFilter filter, Span<RayHit> hits)
    {
        return Unexpected<int>();
    }
    public int RayCastLimited(Vector3 origin, Vector3 direction, float maxDistance, QueryFilter filter, RayCastBudget budget, Span<RayHit> hits, out RayCastStatus status)
    {
        status = default;
        return Unexpected<int>();
    }
    public int RayCastBody(BodyId body, uint partUserData, Vector3 origin, Vector3 direction, float maxDistance, bool backFaces, RayCastBudget budget, Span<RayHit> hits, out RayCastStatus status)
    {
        status = default;
        return Unexpected<int>();
    }
    public void BeginRayCastBudget()
    {
        if (RecordHeartbeat)
            return;
        Unexpected();
    }
    public long RayCastsRefused(RayCastBudget budget)
    {
        if (RecordHeartbeat)
            return 0;
        return Unexpected<long>();
    }
    public int OverlapSphere(Vector3 center, float radius, QueryFilter filter, Span<BodyId> results)
    {
        return Unexpected<int>();
    }
    public int OverlapBox(Vector3 center, Vector3 halfExtents, Quaternion orientation, QueryFilter filter, Span<BodyId> results)
    {
        return Unexpected<int>();
    }
    public bool ShapeCast(ShapeId shape, Vector3 origin, Quaternion orientation, Vector3 direction, float maxDistance, QueryFilter filter, out RayHit hit)
    {
        hit = default;
        return Unexpected<bool>();
    }
    public bool BodyPenetratesOthers(BodyId body, float tolerance)
    {
        return Unexpected<bool>();
    }
    public PhysicsCapacityStats GetCapacityStats()
    {
        if (RecordHeartbeat || RecordTerrain)
            return default;
        return Unexpected<PhysicsCapacityStats>();
    }
    public StepResult Step(float deltaTime, Span<BodyState> bodyUpdates, Span<CharacterState> characterUpdates, Span<ContactReport> contacts)
    {
        if (RecordHeartbeat)
        {
            HeartbeatSteps++;
            return default;
        }
        return Unexpected<StepResult>();
    }

    sealed class GravityAttachment : IDisposable
    {
        public void Dispose() { }
    }

    static void Unexpected()
        => throw new InvalidOperationException("recording backend: the character compile called a method it does not record");

    static T Unexpected<T>()
        => throw new InvalidOperationException("recording backend: the character compile called a method it does not record");
}
