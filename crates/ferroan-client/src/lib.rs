// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region side of one LLUDP circuit, the seed capability and an event queue.
//!
//! Throttles are still planned. Empty event polls wait for the C# hold deadline.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod appearance;
mod arrival;
mod caps;
mod chat;
mod circuit;
mod datagram_log;
mod event_queue;
mod handshake;
mod return_hop;
mod source_hop;
mod teleport;

pub use appearance::{AvatarWire, StoredAppearance};
pub use arrival::{ArrivalConfig, LandLayer};
pub use caps::{
    CapAnswer, CapExchange, CapGrantSet, CapService, CapTarget, CapsError, CapsRegistry,
    CapsServer, UploaderRoutes,
};
pub use circuit::{
    AgentMotionUpdate, CircuitError, CircuitExpect, CircuitServer, CircuitStop, CircuitTiming,
    LoginPose, LoginScene, RootArrival, SceneAnswer, SceneCall, SceneDeparture, SceneMove,
    SceneRefused, SceneReply, SceneResult, TeleportRequest, UdpCounters,
};
pub use event_queue::EventQueueError;
pub use handshake::{HandshakeConfig, StoredHandshake};
pub use return_hop::{ROOT_WAIT, ReturnPose, ReturnSlot, UPDATE_WAIT};
pub use source_hop::{
    CONNECT_INNER, FinishEvent, HopOut, HopPose, HopRequest, INTERNAL, NOT_FOUND, RESOLVE_FAILED,
    SourceHop, caps_seed, connect_reason, offline_map_block, teleport_finish_event,
};
pub use teleport::{IS_FLYING, TeleportRegion, VIA_LOCATION};
