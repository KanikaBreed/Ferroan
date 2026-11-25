// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-msg/src/lib.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Quaternion packing follows OpenMetaverse Quaternion.ToBytes / FromBytes
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// originally the Second Life message codecs (LGPL-2.1).
// Eight trailing message layouts follow Second Life viewer
//   scripts/messages/message_template.msg @ ff75ab037607d8fe305f43f8b1f43126693ba2db
//   Copyright (c) Linden Research, Inc., originally LGPL 2.1.

//! LLUDP messages generated from libopenmetaverse's template plus eight viewer tails.
//!
//! Framing and zero-coding follow Tranquillity's send path and OpenMetaverse's
//! `Header` / `Helpers.ZeroDecode`. See `docs/PROTOCOL.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use glam::{DVec3, Quat, Vec3, Vec4};
use uuid::Uuid;

pub mod packet;
pub mod reliability;

pub use packet::{IncomingPacket, PacketFlags, build_packet, parse_packet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsgId {
    High(u8),
    Medium(u8),
    Low(u16),
}

impl MsgId {
    pub fn encode(self, w: &mut Vec<u8>) {
        match self {
            MsgId::High(n) => w.push(n),
            MsgId::Medium(n) => {
                w.push(0xFF);
                w.push(n);
            }
            MsgId::Low(n) => {
                w.push(0xFF);
                w.push(0xFF);
                w.extend_from_slice(&n.to_be_bytes());
            }
        }
    }

    /// Decode a message number; returns the id and its encoded length.
    pub fn decode(b: &[u8]) -> Result<(MsgId, usize), DecodeError> {
        match b {
            [0xFF, 0xFF, hi, lo, ..] => Ok((MsgId::Low(u16::from_be_bytes([*hi, *lo])), 4)),
            [0xFF, 0xFF, ..] => Err(DecodeError::Eof),
            [0xFF, n, ..] => Ok((MsgId::Medium(*n), 2)),
            [0xFF] => Err(DecodeError::Eof),
            [n, ..] => Ok((MsgId::High(*n), 1)),
            [] => Err(DecodeError::Eof),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MsgInfo {
    pub name: &'static str,
    pub zerocoded: bool,
    pub trusted: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unexpected end of message")]
    Eof,
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
}

/// A typed template message.
pub trait Msg: Sized {
    const NAME: &'static str;
    const ID: MsgId;
    const ZEROCODED: bool;
    const TRUSTED: bool;
    fn encode_body(&self, w: &mut Writer);
    fn decode_body(r: &mut Reader) -> Result<Self, DecodeError>;

    /// Decode from a body slice (after the message number, zero-decoded).
    fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        Self::decode_body(&mut r)
    }
}

static ZEROS: [u8; 64] = [0; 64];

/// Little-endian body reader.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            // Older simulators may omit trailing fields: LL zero-fills them.
            if self.remaining() == 0 && n <= ZEROS.len() {
                return Ok(&ZEROS[..n]);
            }
            return Err(DecodeError::Eof);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    #[inline]
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    #[inline]
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    #[inline]
    pub fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.u8()? as i8)
    }
    #[inline]
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }
    #[inline]
    pub fn vec3(&mut self) -> Result<Vec3, DecodeError> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    #[inline]
    pub fn dvec3(&mut self) -> Result<DVec3, DecodeError> {
        Ok(DVec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    #[inline]
    pub fn vec4(&mut self) -> Result<Vec4, DecodeError> {
        Ok(Vec4::new(
            self.f32()?,
            self.f32()?,
            self.f32()?,
            self.f32()?,
        ))
    }
    /// Packed LLQuaternion: x, y, z, with w reconstructed.
    ///
    /// OpenMetaverse `Quaternion.FromBytes(..., normalized: true)` uses
    /// `1 - x² - y² - z² > 1e-6`.
    #[inline]
    pub fn quat(&mut self) -> Result<Quat, DecodeError> {
        let v = self.vec3()?;
        Ok(unpack_quat(v))
    }
    #[inline]
    pub fn uuid(&mut self) -> Result<Uuid, DecodeError> {
        Ok(Uuid::from_bytes(self.arr()?))
    }
    #[inline]
    pub fn ipaddr(&mut self) -> Result<[u8; 4], DecodeError> {
        self.arr::<4>()
    }
    /// Ports are transported in network byte order.
    #[inline]
    pub fn ipport(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    pub fn fixed(&mut self, n: usize) -> Result<Vec<u8>, DecodeError> {
        Ok(self.take(n)?.to_vec())
    }
    pub fn var1(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u8()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub fn var2(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u16()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    /// Variable block count; a missing count at the end of data means zero
    /// blocks (matches `LLTemplateMessageReader::decodeData`).
    pub fn block_count(&mut self) -> Result<usize, DecodeError> {
        if self.remaining() == 0 {
            return Ok(0);
        }
        Ok(self.u8()? as usize)
    }
}

/// Reconstruct w from a packed (x, y, z) unit quaternion.
///
/// Matches `Quaternion.FromBytes` when `normalized` is true: w is the square
/// root only when `1 - x² - y² - z²` is greater than `1e-6`.
pub fn unpack_quat(v: Vec3) -> Quat {
    let t = 1.0 - v.x * v.x - v.y * v.y - v.z * v.z;
    let w = if t > 1e-6 { t.sqrt() } else { 0.0 };
    Quat::from_xyzw(v.x, v.y, v.z, w)
}

/// Absolute gap allowed on one packed component when a client datagram is
/// compared field by field.
///
/// Ferrobot normalizes with glam, forces `w` non-negative and writes `x`, `y`
/// and `z`. [`pack_quat`] rescales with the OpenMetaverse length rule. Across
/// the headings the protocol test scans, those two stay inside this gap. A
/// larger gap is a different rotation, not packing.
pub const QUAT_PACKING_TOLERANCE: f32 = 2.0e-6;

/// `Quaternion.LengthSquared` as OpenMetaverse computes it on x86-64.
///
/// With SSE4.1 it is `Sse41.DotProduct(q, q, 0xf1)`, and `DPPS` adds the
/// products in pairs: `(x² + y²) + (z² + w²)`. The scalar branch other
/// machines take adds left to right and can differ in the last bit. x86-64 is
/// the reference (D71 in `docs/DECISIONS.md`).
#[inline]
pub fn quat_length_squared(q: Quat) -> f32 {
    (q.x * q.x + q.y * q.y) + (q.z * q.z + q.w * q.w)
}

/// Pack a quaternion the way `Quaternion.ToBytes` does.
///
/// The length is [`quat_length_squared`]. When it is `> 1e-6` or `< 0.9999`, the three
/// written floats are `scale * x`, `scale * y` and `scale * z`, with
/// `scale = -1/sqrt(length)` when `w < 0` and `1/sqrt(length)` otherwise.
/// Otherwise three zeroes are written. A real length cannot miss both
/// comparisons, so that branch is the non-finite path. The rebuilt `w` from
/// [`unpack_quat`] is not the `w` that was scaled, so bytes this function just
/// wrote are not always what a second call writes.
pub fn pack_quat(q: Quat) -> [f32; 3] {
    let norm = quat_length_squared(q);
    if norm > 1e-6 || norm < 0.9999 {
        let scale = if q.w < 0.0 {
            -1.0 / norm.sqrt()
        } else {
            1.0 / norm.sqrt()
        };
        x86_nan([scale * q.x, scale * q.y, scale * q.z], q)
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// The NaN an x86-64 SSE operation writes when no operand is a NaN, such as
/// `inf * 0`: quiet, with the sign bit set. ARM64 writes it positive
/// (`0x7FC00000`).
pub const X86_DEFAULT_NAN: u32 = 0xFFC0_0000;

/// A NaN made from finite or infinite inputs, such as the zero quaternion's
/// `(1 / sqrt(0)) * 0`, gets x86-64's bits on every machine (D71). A NaN input
/// is left to the hardware: the oracle has none.
fn x86_nan(packed: [f32; 3], q: Quat) -> [f32; 3] {
    if q.is_nan() {
        return packed;
    }
    packed.map(|value| {
        if value.is_nan() {
            f32::from_bits(X86_DEFAULT_NAN)
        } else {
            value
        }
    })
}

/// Little-endian body writer.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
    /// Set by [`Writer::record_quat_marks`]. Each `true` is a byte [`Writer::quat`] wrote.
    quat_marks: Option<Vec<bool>>,
    /// Set by [`Writer::keep_packed_quats`].
    keep_quats: bool,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256),
            quat_marks: None,
            keep_quats: false,
        }
    }

    /// Record which bytes the next [`Writer::quat`] calls write.
    pub(crate) fn record_quat_marks(&mut self) {
        self.quat_marks = Some(Vec::new());
    }

    /// Write each quaternion's x, y and z as they are, with no `ToBytes`
    /// rescale. A packet decoded and encoded this way gives back its own bytes,
    /// so redaction and the capture check change nothing but identities.
    pub(crate) fn keep_packed_quats(&mut self) {
        self.keep_quats = true;
    }

    /// Marks from [`Writer::record_quat_marks`], padded to the body length.
    pub(crate) fn take_quat_marks(&mut self) -> Vec<bool> {
        let mut marks = self.quat_marks.take().unwrap_or_default();
        if marks.len() < self.buf.len() {
            marks.resize(self.buf.len(), false);
        }
        marks
    }
    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    #[inline]
    pub fn i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }
    #[inline]
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    #[inline]
    pub fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    #[inline]
    pub fn dvec3(&mut self, v: DVec3) {
        self.f64(v.x);
        self.f64(v.y);
        self.f64(v.z);
    }
    #[inline]
    pub fn vec4(&mut self, v: Vec4) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
        self.f32(v.w);
    }
    #[inline]
    pub fn quat(&mut self, q: Quat) {
        let start = self.buf.len();
        let packed = if self.keep_quats {
            [q.x, q.y, q.z]
        } else {
            pack_quat(q)
        };
        self.f32(packed[0]);
        self.f32(packed[1]);
        self.f32(packed[2]);
        if let Some(marks) = self.quat_marks.as_mut() {
            if marks.len() < start {
                marks.resize(start, false);
            }
            marks.extend(std::iter::repeat_n(true, self.buf.len() - start));
        }
    }
    #[inline]
    pub fn uuid(&mut self, v: Uuid) {
        self.buf.extend_from_slice(v.as_bytes());
    }
    #[inline]
    pub fn ipaddr(&mut self, v: [u8; 4]) {
        self.buf.extend_from_slice(&v);
    }
    #[inline]
    pub fn ipport(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn fixed(&mut self, v: &[u8], n: usize) {
        let m = v.len().min(n);
        self.buf.extend_from_slice(&v[..m]);
        self.buf.resize(self.buf.len() + (n - m), 0);
    }
    pub fn var1(&mut self, v: &[u8]) {
        let n = v.len().min(255);
        self.buf.push(n as u8);
        self.buf.extend_from_slice(&v[..n]);
    }
    pub fn var2(&mut self, v: &[u8]) {
        let n = v.len().min(65535);
        self.u16(n as u16);
        self.buf.extend_from_slice(&v[..n]);
    }
}

/// Helper: build a NUL-terminated string field as SL expects.
pub fn str_field(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// Helper: decode a (possibly NUL-terminated) string field.
pub fn field_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

#[allow(clippy::all, non_snake_case, dead_code, unused_variables)]
pub mod msgs {
    use crate::Msg;
    use crate::MsgId;
    use crate::MsgInfo;
    #[allow(unused_imports)]
    use crate::{DecodeError, Reader, Writer};
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

pub use msgs::{
    IdCheck, id_check, message_info, reencode_body, reencode_body_keeping_quats,
    reencode_body_quat_marks, scrub_body,
};
pub use packet::{
    client_datagram_within_quat_packing, reencode_datagram, reencode_datagram_keeping_quats,
    scrub_datagram,
};

#[cfg(test)]
mod tests {
    use super::msgs::*;
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(StartPingCheck::ID, MsgId::High(1));
        assert_eq!(PacketAck::ID, MsgId::Low(0xFFFB));
        assert_eq!(UseCircuitCode::ID, MsgId::Low(3));
        const { assert!(ObjectUpdate::ZEROCODED) };
        assert_eq!(message_info(MsgId::High(1)).unwrap().name, "StartPingCheck");
    }

    #[test]
    fn id_checks_follow_the_agent_data_rule() {
        assert_eq!(
            id_check(SetAlwaysRun::ID),
            Some(IdCheck {
                agent: 0,
                session: 16
            })
        );
        for message in [
            ChildAgentUpdate::ID,
            ChildAgentAlive::ID,
            ChildAgentPositionUpdate::ID,
        ] {
            assert_eq!(
                id_check(message),
                Some(IdCheck {
                    agent: 12,
                    session: 28
                })
            );
        }
        assert_eq!(
            id_check(RemoveAttachment::ID),
            Some(IdCheck {
                agent: 0,
                session: 16
            })
        );
        assert!(id_check(UseCircuitCode::ID).is_none());
        assert!(id_check(PacketAck::ID).is_none());
        assert!(id_check(DisableSimulator::ID).is_none());
    }

    #[test]
    fn roundtrip_chat() {
        let mut m = ChatFromViewer::default();
        m.agent_data.agent_id = Uuid::from_u128(1);
        m.chat_data.message = str_field("hello");
        m.chat_data.channel = 0;
        m.chat_data.type_ = 1;
        let mut w = Writer::new();
        m.encode_body(&mut w);
        let back = ChatFromViewer::decode(&w.buf).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn scrub_replaces_the_circuit_identity_and_still_round_trips() {
        let mut m = UseCircuitCode::default();
        m.circuit_code.code = 99;
        m.circuit_code.session_id = Uuid::from_u128(0x1111);
        m.circuit_code.id = Uuid::from_u128(0x2222);
        let mut raw = Vec::new();
        packet::build_packet(&m, 7, true, false, &[], &mut raw);
        assert_eq!(reencode_datagram(&raw).unwrap(), raw);
        let scrubbed = scrub_datagram(&raw).unwrap();
        assert_ne!(scrubbed, raw);
        assert_eq!(reencode_datagram(&scrubbed).unwrap(), scrubbed);
        let back = parse_packet(&scrubbed)
            .unwrap()
            .decode::<UseCircuitCode>()
            .unwrap();
        assert_eq!(back.circuit_code.code, 1);
        assert_eq!(back.circuit_code.session_id, Uuid::from_u128(2));
        assert_eq!(back.circuit_code.id, Uuid::from_u128(1));
    }

    #[test]
    fn scrub_replaces_the_chat_owner_and_keeps_the_message() {
        let mut m = ChatFromSimulator::default();
        m.chat_data.from_name = b"Ada".to_vec();
        m.chat_data.source_id = Uuid::from_u128(0x3333);
        m.chat_data.owner_id = Uuid::from_u128(0x4444);
        m.chat_data.message = b"hello".to_vec();
        let mut raw = Vec::new();
        packet::build_packet(&m, 3, false, false, &[], &mut raw);
        let scrubbed = scrub_datagram(&raw).unwrap();
        let back = parse_packet(&scrubbed)
            .unwrap()
            .decode::<ChatFromSimulator>()
            .unwrap();
        assert_eq!(back.chat_data.from_name, b"Redacted");
        assert_eq!(back.chat_data.owner_id, Uuid::from_u128(1));
        assert_eq!(back.chat_data.source_id, Uuid::from_u128(0x3333));
        assert_eq!(back.chat_data.message, b"hello");
        assert_eq!(reencode_datagram(&scrubbed).unwrap(), scrubbed);
    }

    #[test]
    fn quat_roundtrip() {
        let q = Quat::from_rotation_z(1.0);
        let mut w = Writer::new();
        w.quat(q);
        let mut r = Reader::new(&w.buf);
        let b = r.quat().unwrap();
        assert!(q.dot(b).abs() > 0.9999);
    }
}
