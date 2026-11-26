// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region side of one LLUDP circuit, the seed capability and an event queue.
//!
//! Category token buckets gate resends, land and task sends. Empty event polls
//! wait for the C# hold deadline.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod appearance;
mod arrival;
mod caps;
mod chat;
mod circuit;
mod datagram_log;
mod entity_feed;
mod material;
mod task_batch;
pub use material::{Face, packet as material_packet};
pub use task_batch::{TaskBatch, TaskPackets};
mod event_queue;
mod handshake;
mod names;
mod return_hop;
mod source_hop;
mod teleport;
mod throttle;

pub use appearance::{
    AvatarWire, DEFAULT_TEXTURE as DEFAULT_AVATAR_TEXTURE, LiveAvatar, StoredAppearance,
};
pub use arrival::{
    ArrivalConfig, LandLayer, ParcelOverlayChunk, VIA_LOGIN, land_at_complete, land_at_initial,
};
pub use caps::{
    CapAnswer, CapExchange, CapGrantSet, CapService, CapTarget, CapsError, CapsRegistry,
    CapsServer, UploaderRoutes,
};
pub use circuit::{
    AgentMotionUpdate, CircuitError, CircuitExpect, CircuitServer, CircuitStop, CircuitTiming,
    LoginPose, LoginReservation, LoginScene, MAX_NAME_REPLIES, PENDING_LOGINS, ParcelQuery,
    RootArrival, SceneAnswer, SceneCall, SceneDeparture, SceneMove, SceneRefused, SceneReply,
    SceneResult, TeleportRequest, UdpCounters,
};
pub use entity_feed::{
    AVATAR_TERSE_LEN, AvatarPacket, EntityFeed, EntityUpdates, coarse_update, kill_packets,
};
pub use event_queue::EventQueueError;
pub use handshake::{HandshakeConfig, StoredHandshake};
pub use names::{
    MAX_CIRCUIT_LOOKUPS, MAX_NAME_CACHE, MAX_NAME_LOOKUPS, NameFuture, NameSource, RegionNames,
    UserName,
};
pub use return_hop::{ROOT_WAIT, ReturnPose, ReturnSlot, UPDATE_WAIT};
pub use source_hop::{
    CONNECT_INNER, FinishEvent, HopOut, HopPose, HopRequest, INTERNAL, NOT_FOUND, RESOLVE_FAILED,
    SourceHop, caps_seed, connect_reason, offline_map_block, teleport_finish_event,
};
pub use teleport::{IS_FLYING, TeleportRegion, VIA_LOCATION};
pub use throttle::{CLIENT_MAX_BPS, CLIENT_MAX_BPS_RANGE, CircuitThrottle, SCENE_MAX_BPS};
