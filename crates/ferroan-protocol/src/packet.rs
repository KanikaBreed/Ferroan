// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-msg/src/packet.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Zero-coding and the header follow
//   OpenMetaverse/Helpers.cs ZeroEncode / ZeroDecode
//   OpenMetaverse/_Packets_.cs Header
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs ZeroEncode,
//   SendPacketFinal
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c

//! Packet framing: header, zero-coding and appended acks.
//!
//! ```text
//! [flags u8][sequence u32 BE][extra_len u8][extra...][msg number][body...][acks u32 BE...][ack_count u8]
//! ```
//!
//! Tranquillity writes `extra_len` as 0. Zero-coding applies to the message
//! number and body, not to the 6-byte header and not to appended acks.
//! A zero run is `0x00` followed by a count; a count of 0 expands to nothing,
//! matching `Helpers.ZeroDecode`. The send path keeps the zerocoded form only
//! when it is not larger than the raw packet (`LLUDPServer.SendPacketData`).

use ferroan_diag::{self, Kind, UDP_DATAGRAM, UDP_ZERO_EXPANSION};

use crate::{DecodeError, Msg, MsgId, Writer};

pub struct PacketFlags;

impl PacketFlags {
    pub const ZEROCODED: u8 = 0x80;
    pub const RELIABLE: u8 = 0x40;
    pub const RESENT: u8 = 0x20;
    pub const ACK: u8 = 0x10;
}

pub const HEADER_SIZE: usize = 6;
/// `LLUDPServer.MTU`. OpenMetaverse's `Packet.MTU` is 1200 and is the split
/// size for variable blocks, which this crate does not do.
pub const MTU: usize = 1400;
/// A received datagram fits in `UDPPacketBuffer.BUFFER_SIZE`.
pub const MAX_DATAGRAM: usize = 4096;
/// Cap on bytes produced by zero-expansion. A normal packet stays under this.
/// OpenMetaverse does not cap it.
pub const MAX_ZERO_EXPANSION: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct IncomingPacket {
    pub flags: u8,
    pub sequence: u32,
    pub id: MsgId,
    /// Body after the message number, zero-decoded.
    pub body: Vec<u8>,
    pub acks: Vec<u32>,
    /// Bytes between the 6-byte header and the message number. Tranquillity
    /// always writes a length of 0.
    pub extra: Vec<u8>,
    /// Size of the whole datagram once zero-decoded (its received size when
    /// it was not zero-coded).
    pub expanded_len: usize,
}

impl IncomingPacket {
    pub fn reliable(&self) -> bool {
        self.flags & PacketFlags::RELIABLE != 0
    }
    pub fn resent(&self) -> bool {
        self.flags & PacketFlags::RESENT != 0
    }
    pub fn decode<M: Msg>(&self) -> Result<M, DecodeError> {
        M::decode(&self.body)
    }
}

fn zero_decode(src: &[u8], out: &mut Vec<u8>) -> Result<(), DecodeError> {
    let mut i = 0;
    while i < src.len() {
        if src[i] != 0 {
            out.push(src[i]);
            i += 1;
            continue;
        }
        let Some(count) = src.get(i + 1).copied() else {
            return Err(DecodeError::Malformed("zero run missing a count"));
        };
        let add = count as usize;
        let expanded = out.len().saturating_add(add);
        if expanded > MAX_ZERO_EXPANSION {
            ferroan_diag::refuse(
                UDP_ZERO_EXPANSION,
                Kind::Hard,
                MAX_ZERO_EXPANSION as u64,
                expanded as u64,
                "udp",
            );
            return Err(DecodeError::Malformed("zero expansion too large"));
        }
        out.resize(out.len() + add, 0);
        i += 2;
    }
    Ok(())
}

/// Encode `src` the way `LLUDPServer.ZeroEncode` encodes the bytes after the
/// 6-byte header. A run of 256 zeroes is `00 FF` plus a new run that starts
/// at 1, because a `byte` count wraps.
fn zero_encode(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    let mut zerocount: u8 = 0;
    while i < src.len() {
        if src[i] == 0 {
            zerocount = zerocount.wrapping_add(1);
            if zerocount == 0 {
                out.push(0);
                out.push(0xff);
                zerocount = zerocount.wrapping_add(1);
            }
        } else {
            if zerocount != 0 {
                out.push(0);
                out.push(zerocount);
                zerocount = 0;
            }
            out.push(src[i]);
        }
        i += 1;
    }
    if zerocount != 0 {
        out.push(0);
        out.push(zerocount);
    }
}

/// Parse a raw datagram.
pub fn parse_packet(data: &[u8]) -> Result<IncomingPacket, DecodeError> {
    if data.len() > MAX_DATAGRAM {
        ferroan_diag::refuse(
            UDP_DATAGRAM,
            Kind::Hard,
            MAX_DATAGRAM as u64,
            data.len() as u64,
            "udp",
        );
        return Err(DecodeError::Malformed("packet too large"));
    }
    if data.len() < HEADER_SIZE + 1 {
        return Err(DecodeError::Malformed("packet too short"));
    }
    let flags = data[0];
    let sequence = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    let extra_len = data[5] as usize;
    if data.len() < HEADER_SIZE + extra_len {
        return Err(DecodeError::Malformed("extra header overflow"));
    }
    let mut end = data.len();
    let mut acks = Vec::new();
    if flags & PacketFlags::ACK != 0 {
        let count = data[end - 1] as usize;
        let needed = count * 4 + 1;
        if end < HEADER_SIZE + extra_len + needed {
            return Err(DecodeError::Malformed("ack block too large"));
        }
        end -= 1;
        let start = end - count * 4;
        for chunk in data[start..end].as_chunks::<4>().0 {
            acks.push(u32::from_be_bytes(*chunk));
        }
        end = start;
    }
    let extra = data[HEADER_SIZE..HEADER_SIZE + extra_len].to_vec();
    let payload_raw = &data[HEADER_SIZE + extra_len..end];
    let decoded;
    let payload: &[u8] = if flags & PacketFlags::ZEROCODED != 0 {
        let mut v = Vec::new();
        zero_decode(payload_raw, &mut v)?;
        decoded = v;
        &decoded
    } else {
        payload_raw
    };
    let (id, idlen) = MsgId::decode(payload)?;
    Ok(IncomingPacket {
        flags,
        sequence,
        id,
        body: payload[idlen..].to_vec(),
        acks,
        extra,
        expanded_len: data.len() - payload_raw.len() + payload.len(),
    })
}

/// Serialize a message into a datagram. `acks` are appended (as many as fit
/// in the MTU); returns how many acks were appended.
pub fn build_packet<M: Msg>(
    msg: &M,
    sequence: u32,
    reliable: bool,
    resent: bool,
    acks: &[u32],
    out: &mut Vec<u8>,
) -> usize {
    let mut body = Writer::new();
    M::ID.encode(&mut body.buf);
    msg.encode_body(&mut body);
    build_raw(
        &body.buf,
        M::ZEROCODED,
        sequence,
        reliable,
        resent,
        acks,
        out,
    )
}

/// Length of the datagram `build_raw` frames for `payload` with no acks,
/// without building it. Throttles charge this size.
pub fn raw_len(payload: &[u8], zerocoded: bool) -> usize {
    if !zerocoded {
        return HEADER_SIZE + payload.len();
    }
    // Mirrors `zero_encode`: a run of zeros is two bytes per 255 zeros.
    let mut coded = 0;
    let mut zerocount: u8 = 0;
    for &byte in payload {
        if byte == 0 {
            zerocount = zerocount.wrapping_add(1);
            if zerocount == 0 {
                coded += 2;
                zerocount = 1;
            }
        } else {
            if zerocount != 0 {
                coded += 2;
                zerocount = 0;
            }
            coded += 1;
        }
    }
    if zerocount != 0 {
        coded += 2;
    }
    HEADER_SIZE + coded.min(payload.len())
}

/// Frame an already-encoded message (number + body).
pub fn build_raw(
    payload: &[u8],
    zerocoded: bool,
    sequence: u32,
    reliable: bool,
    resent: bool,
    acks: &[u32],
    out: &mut Vec<u8>,
) -> usize {
    out.clear();
    let mut flags = 0u8;
    if reliable {
        flags |= PacketFlags::RELIABLE;
    }
    if resent {
        flags |= PacketFlags::RESENT;
    }
    out.push(flags);
    out.extend_from_slice(&sequence.to_be_bytes());
    out.push(0); // Tranquillity writes no extra header
    if zerocoded {
        let start = out.len();
        zero_encode(payload, out);
        // SendPacketData keeps the coded form when it is not larger.
        if out.len() - start <= payload.len() {
            out[0] |= PacketFlags::ZEROCODED;
        } else {
            out.truncate(start);
            out.extend_from_slice(payload);
        }
    } else {
        out.extend_from_slice(payload);
    }
    // SendPacketFinal: `while (dataLength + 5 < MTU && ackCount < 256)`.
    let mut n = 0;
    while n < acks.len() && n < 255 && out.len() + 4 * n + 5 < MTU {
        n += 1;
    }
    if n > 0 {
        for a in &acks[..n] {
            out.extend_from_slice(&a.to_be_bytes());
        }
        out.push(n as u8);
        out[0] |= PacketFlags::ACK;
    }
    n
}

/// Decode a datagram and encode it again with the same sequence, flags and
/// acks. Zero-coding follows the flag on the datagram. Tranquillity sets
/// that flag on some messages the template marks Unencoded.
pub fn reencode_datagram(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    reencode_with(data, crate::reencode_body)
}

/// [`reencode_datagram`], but each packed quaternion keeps the x, y and z it
/// was read with instead of the `ToBytes` rescale.
///
/// A server packet in a capture is checked this way. C# writes `ToBytes`
/// output; reading that back rebuilds `w`, and packing it again does not
/// always give the same bytes (76 of the oracle batch's 1022 read-back
/// quaternions differ). Every other byte must still match.
/// Whether Ferroan packs as OpenMetaverse does is the oracle's
/// `quaternion-batch` fixture, not this check.
pub fn reencode_datagram_keeping_quats(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    reencode_with(data, crate::reencode_body_keeping_quats)
}

fn reencode_with(
    data: &[u8],
    body: fn(crate::MsgId, &[u8]) -> Result<Vec<u8>, DecodeError>,
) -> Result<Vec<u8>, DecodeError> {
    let parsed = parse_packet(data)?;
    if !parsed.extra.is_empty() {
        return Err(DecodeError::Malformed("extra header is not re-encoded"));
    }
    if crate::message_info(parsed.id).is_none() {
        return Err(DecodeError::Malformed("unknown message"));
    }
    let body = body(parsed.id, &parsed.body)?;
    Ok(frame_like(&parsed, &body))
}

/// Frame `body` with the sequence, flags and acks of `parsed`.
fn frame_like(parsed: &IncomingPacket, body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4 + body.len());
    parsed.id.encode(&mut payload);
    payload.extend_from_slice(body);
    let mut out = Vec::new();
    build_raw(
        &payload,
        parsed.flags & PacketFlags::ZEROCODED != 0,
        parsed.sequence,
        parsed.reliable(),
        parsed.resent(),
        &parsed.acks,
        &mut out,
    );
    out
}

/// Whether `original` matches [`reencode_datagram`] except for packed
/// quaternion components inside [`crate::QUAT_PACKING_TOLERANCE`].
///
/// The header, message, acks and every non-quaternion body byte must match.
/// The body length must match, so a trailing byte is not treated as packing.
/// The zero-coding must be the canonical one: the original body framed again
/// has to give back the original datagram. A packet the server sent is not
/// checked here: see [`reencode_datagram_keeping_quats`].
pub fn client_datagram_within_quat_packing(original: &[u8]) -> Result<bool, DecodeError> {
    let again = reencode_datagram(original)?;
    if again == original {
        return Ok(true);
    }
    let parsed = parse_packet(original)?;
    if frame_like(&parsed, &parsed.body) != original {
        return Ok(false);
    }
    let encoded = parse_packet(&again)?;
    if parsed.flags != encoded.flags
        || parsed.sequence != encoded.sequence
        || parsed.id != encoded.id
        || parsed.acks != encoded.acks
        || parsed.extra != encoded.extra
        || parsed.body.len() != encoded.body.len()
    {
        return Ok(false);
    }
    let (marked, marks) = crate::reencode_body_quat_marks(parsed.id, &parsed.body)?;
    if marked != encoded.body || marks.len() != parsed.body.len() {
        return Ok(false);
    }
    Ok(quat_slots_within_tolerance(
        &parsed.body,
        &encoded.body,
        &marks,
    ))
}

fn quat_slots_within_tolerance(original: &[u8], encoded: &[u8], marks: &[bool]) -> bool {
    let mut index = 0;
    while index < original.len() {
        if !marks[index] {
            if original[index] != encoded[index] {
                return false;
            }
            index += 1;
            continue;
        }
        let end = index + 12;
        if end > original.len() || marks[index..end].iter().any(|mark| !*mark) {
            return false;
        }
        if !quat_slot_within(&original[index..end], &encoded[index..end]) {
            return false;
        }
        index = end;
    }
    true
}

fn quat_slot_within(original: &[u8], encoded: &[u8]) -> bool {
    for component in 0..3 {
        let start = component * 4;
        let Some(left) = f32_le(&original[start..start + 4]) else {
            return false;
        };
        let Some(right) = f32_le(&encoded[start..start + 4]) else {
            return false;
        };
        if !left.is_finite() || !right.is_finite() {
            if left.to_bits() != right.to_bits() {
                return false;
            }
            continue;
        }
        if (left - right).abs() > crate::QUAT_PACKING_TOLERANCE {
            return false;
        }
    }
    true
}

fn f32_le(bytes: &[u8]) -> Option<f32> {
    let chunk: [u8; 4] = bytes.try_into().ok()?;
    Some(f32::from_le_bytes(chunk))
}

/// Decode, replace session, agent, name, circuit and IP fields, and encode
/// again. Other bytes follow the same path as
/// [`reencode_datagram_keeping_quats`], so packed quaternions keep their bytes.
pub fn scrub_datagram(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    reencode_with(data, crate::scrub_body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msgs::*;
    use crate::{Quat, Vec3, unpack_quat};
    use uuid::Uuid;

    #[test]
    fn raw_len_matches_the_framed_datagram() {
        let cases: [&[u8]; 6] = [
            &[],
            &[1, 2, 3],
            &[0, 0, 0, 5],
            &[7, 0],
            &[0; 300],
            &[0, 1, 0, 0, 2, 0, 0, 0],
        ];
        for payload in cases {
            for zerocoded in [false, true] {
                let mut out = Vec::new();
                build_raw(payload, zerocoded, 9, true, false, &[], &mut out);
                assert_eq!(
                    raw_len(payload, zerocoded),
                    out.len(),
                    "{payload:?} {zerocoded}"
                );
            }
        }
    }

    #[test]
    fn roundtrip_with_acks_and_zerocode() {
        let mut m = UseCircuitCode::default();
        m.circuit_code.code = 1234;
        m.circuit_code.session_id = Uuid::from_u128(5);
        m.circuit_code.id = Uuid::from_u128(0);
        let mut buf = Vec::new();
        let n = build_packet(&m, 42, true, false, &[1, 2, 3], &mut buf);
        assert_eq!(n, 3);
        let p = parse_packet(&buf).unwrap();
        assert_eq!(p.sequence, 42);
        assert!(p.reliable());
        assert_eq!(p.acks, vec![1, 2, 3]);
        assert_eq!(p.id, UseCircuitCode::ID);
        let back: UseCircuitCode = p.decode().unwrap();
        assert_eq!(back, m);

        // Zerocoded message type
        let mut o = RequestImage::default();
        o.agent_data.agent_id = Uuid::from_u128(9);
        o.request_image.push(request_image::RequestImage {
            image: Uuid::from_u128(77),
            discard_level: 0,
            download_priority: 1.0,
            packet: 0,
            type_: 0,
        });
        build_packet(&o, 7, false, false, &[], &mut buf);
        assert_eq!(
            buf[0] & PacketFlags::ZEROCODED != 0,
            RequestImage::ZEROCODED
        );
        let p = parse_packet(&buf).unwrap();
        let back: RequestImage = p.decode().unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn a_viewer_style_agent_update_quaternion_round_trips() {
        // Ferrobot writes glam's normalize, then the sign-flipped x, y, z.
        // OpenMetaverse ToBytes rescales. For some headings those bytes differ,
        // so a capture of that AgentUpdate used to fail re-encode.
        let mut heading = None;
        for step in 1..20_000 {
            let turned = ferrobot_quat(Quat::from_rotation_z(step as f32 * 0.001));
            if openmetaverse_rescale(turned) != turned {
                heading = Some(turned);
                break;
            }
        }
        let turned = heading.expect("a heading whose rescaling changes the bytes");
        let mut message = AgentUpdate::default();
        message.agent_data.agent_id = Uuid::from_u128(0x1111);
        message.agent_data.session_id = Uuid::from_u128(0x2222);
        message.agent_data.body_rotation = Quat::from_xyzw(turned[0], turned[1], turned[2], 1.0);
        message.agent_data.head_rotation = message.agent_data.body_rotation;
        message.agent_data.far = 64.0;
        let mut payload = Vec::new();
        AgentUpdate::ID.encode(&mut payload);
        let mut body = crate::Writer::new();
        message.encode_body(&mut body);
        payload.extend(body.buf);
        assert_eq!(payload.len(), 1 + 114, "AgentUpdate id plus body");
        let packed = turned
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        payload[1 + 32..1 + 44].copy_from_slice(&packed);
        payload[1 + 44..1 + 56].copy_from_slice(&packed);
        let datagram = frame_like_ferrobot(&payload, 40, false);
        let parsed = parse_packet(&datagram).unwrap();
        assert_eq!(parsed.id, AgentUpdate::ID);
        let again = reencode_datagram(&datagram).unwrap();
        assert_ne!(again, datagram);
        assert!(client_datagram_within_quat_packing(&datagram).unwrap());
        let mut torn = datagram.clone();
        torn.push(0xab);
        assert!(!matches!(
            client_datagram_within_quat_packing(&torn),
            Ok(true)
        ));
        let mut body = parsed.body.clone();
        // A non-unit x of 2 packs near 1. That gap is a different value, not packing.
        body[32..36].copy_from_slice(&2.0f32.to_le_bytes());
        body[36..40].copy_from_slice(&0.0f32.to_le_bytes());
        body[40..44].copy_from_slice(&0.0f32.to_le_bytes());
        let mut shifted = Vec::new();
        AgentUpdate::ID.encode(&mut shifted);
        shifted.extend(body);
        let shifted = frame_like_ferrobot(&shifted, 40, false);
        assert!(!client_datagram_within_quat_packing(&shifted).unwrap());
    }

    #[test]
    fn a_gap_of_five_millionths_is_outside_the_packing_tolerance() {
        // x just past 1 with y = z = 0: FromBytes sets w to 0, the length is
        // x², and ToBytes scales x back to 1. The gap is about 5e-6, inside
        // the old 1e-5 tolerance and outside 2e-6.
        let x = 1.000_005f32;
        let mut message = AgentUpdate::default();
        message.agent_data.agent_id = Uuid::from_u128(0x1111);
        message.agent_data.session_id = Uuid::from_u128(0x2222);
        message.agent_data.far = 64.0;
        let mut payload = Vec::new();
        AgentUpdate::ID.encode(&mut payload);
        let mut body = crate::Writer::new();
        message.encode_body(&mut body);
        payload.extend(body.buf);
        for slot in [1 + 32, 1 + 44] {
            payload[slot..slot + 4].copy_from_slice(&x.to_le_bytes());
            payload[slot + 4..slot + 12].fill(0);
        }
        let datagram = frame_like_ferrobot(&payload, 41, false);
        let repacked = crate::pack_quat(unpack_quat(Vec3::new(x, 0.0, 0.0)));
        let gap = (x - repacked[0]).abs();
        assert!(gap > 2.0e-6 && gap < 1.0e-5, "gap {gap}");
        assert!(gap > crate::QUAT_PACKING_TOLERANCE, "gap {gap}");
        assert!(!client_datagram_within_quat_packing(&datagram).unwrap());
    }

    #[test]
    fn a_non_quaternion_byte_the_encoder_rewrites_is_outside_packing() {
        let message = SimulatorLoad::default();
        let mut payload = Vec::new();
        SimulatorLoad::ID.encode(&mut payload);
        let mut body = crate::Writer::new();
        message.encode_body(&mut body);
        payload.extend(body.buf);
        // TimeDilation f32, AgentCount i32, then CanAcceptAgents. A stored 2
        // decodes as true and encodes as 1.
        assert_eq!(payload[4 + 8], 0);
        payload[4 + 8] = 2;
        let mut datagram = Vec::new();
        build_raw(&payload, false, 3, true, false, &[], &mut datagram);
        assert!(!client_datagram_within_quat_packing(&datagram).unwrap());
    }

    #[test]
    fn a_negative_w_quaternion_is_still_rescaled() {
        let packed = crate::pack_quat(Quat::from_xyzw(0.0, 0.2, 0.0, -0.8));
        assert!(packed[1] < 0.0, "{packed:?}");
    }

    #[test]
    fn pack_quat_matches_openmetaverse_tobytes_fixtures() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle/bytes");
        let sources = [
            (
                "quaternion-tobytes-identity.bin",
                Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
            ),
            (
                "quaternion-tobytes-tilt.bin",
                Quat::from_xyzw(0.1, -0.2, 0.3, 0.9),
            ),
            (
                "quaternion-tobytes-neg-w.bin",
                Quat::from_xyzw(0.0, 0.70710677, 0.0, -0.70710677),
            ),
        ];
        for (name, source) in sources {
            let file = std::fs::read(root.join(name)).unwrap();
            let packed = crate::pack_quat(source);
            let mut bytes = Vec::new();
            for value in packed {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            assert_eq!(bytes, file, "{name}");
            let decoded = unpack_quat(Vec3::new(packed[0], packed[1], packed[2]));
            assert_eq!(crate::pack_quat(decoded), packed, "{name} is a fixed point");
        }
    }

    /// `fixtures/oracle/bytes/quaternion-batch.bin`: 4096 records of x, y, z,
    /// w in, then `ToBytes`, then `GetBytes`, written by OpenMetaverse on
    /// x86-64 with SSE4.1.
    fn openmetaverse_batch() -> Vec<(Quat, [u8; 12])> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/oracle/bytes/quaternion-batch.bin");
        let file = std::fs::read(path).unwrap();
        assert_eq!(file.len() % 40, 0);
        file.as_chunks::<40>()
            .0
            .iter()
            .map(|record| {
                let f = |at: usize| f32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                let q = Quat::from_xyzw(f(0), f(4), f(8), f(12));
                (q, record[16..28].try_into().unwrap())
            })
            .collect()
    }

    fn le_bytes(packed: [f32; 3]) -> Vec<u8> {
        packed
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn pack_quat_matches_openmetaverse_on_thousands_of_quaternions() {
        let batch = openmetaverse_batch();
        assert_eq!(batch.len(), 4096);
        let mut sequential_differs = 0;
        let mut decoded_rescaled = 0;
        for (index, (source, expect)) in batch.iter().enumerate() {
            let packed = crate::pack_quat(*source);
            assert_eq!(le_bytes(packed), expect, "record {index} {source:?}");
            // Left-to-right sum, the scalar branch: what #211 wrote.
            let norm = source.x * source.x
                + source.y * source.y
                + source.z * source.z
                + source.w * source.w;
            if norm != crate::quat_length_squared(*source) {
                let scale = if source.w < 0.0 {
                    -1.0 / norm.sqrt()
                } else {
                    1.0 / norm.sqrt()
                };
                let scalar = [scale * source.x, scale * source.y, scale * source.z];
                if le_bytes(scalar) != expect {
                    sequential_differs += 1;
                }
            }
            // Every fourth record from the eighth on was read back with
            // FromBytes, so its w is already the rebuilt one. The #173 short
            // circuit wrote such x, y, z back unchanged.
            if index >= 8 && index % 4 == 2 && packed != [source.x, source.y, source.z] {
                decoded_rescaled += 1;
            }
        }
        // The scalar sum writes other bytes for some of these records.
        assert!(sequential_differs > 0, "the batch tells the two sums apart");
        assert!(
            decoded_rescaled > 0,
            "the batch rescales a decoded quaternion"
        );
    }

    #[test]
    fn ferrobot_headings_stay_inside_the_packing_tolerance() {
        let mut widest = 0.0f32;
        let mut differed = false;
        for step in 1..20_000 {
            let turned = ferrobot_quat(Quat::from_rotation_z(step as f32 * 0.001));
            let scaled = openmetaverse_rescale(turned);
            if scaled != turned {
                differed = true;
            }
            for (left, right) in turned.iter().zip(scaled) {
                widest = widest.max((left - right).abs());
            }
        }
        assert!(differed);
        assert!(
            widest <= crate::QUAT_PACKING_TOLERANCE,
            "widest packing gap {widest} exceeds {}",
            crate::QUAT_PACKING_TOLERANCE
        );
    }

    /// Ferrobot `Writer::quat`: normalize, force w non-negative, write x, y, z.
    fn ferrobot_quat(q: Quat) -> [f32; 3] {
        let mut q = q.normalize();
        if !q.is_finite() {
            q = Quat::IDENTITY;
        }
        if q.w < 0.0 {
            q = -q;
        }
        [q.x, q.y, q.z]
    }

    /// `Quaternion.ToBytes` applied to a quaternion unpacked from packed x, y, z.
    fn openmetaverse_rescale(packed: [f32; 3]) -> [f32; 3] {
        let q = unpack_quat(Vec3::new(packed[0], packed[1], packed[2]));
        let norm = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
        let scale = if q.w < 0.0 {
            -1.0 / norm.sqrt()
        } else {
            1.0 / norm.sqrt()
        };
        [scale * q.x, scale * q.y, scale * q.z]
    }

    /// Ferrobot keeps the coded form only when it is strictly smaller.
    fn frame_like_ferrobot(payload: &[u8], sequence: u32, reliable: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut flags = 0u8;
        if reliable {
            flags |= PacketFlags::RELIABLE;
        }
        out.push(flags);
        out.extend_from_slice(&sequence.to_be_bytes());
        out.push(0);
        let start = out.len();
        zero_encode(payload, &mut out);
        if out.len() - start < payload.len() {
            out[0] |= PacketFlags::ZEROCODED;
        } else {
            out.truncate(start);
            out.extend_from_slice(payload);
        }
        out
    }

    #[test]
    fn a_message_zerocoded_off_template_round_trips() {
        // message_template.msg marks this Unencoded. LLClientView.terseUpdateHeader
        // still sets MSG_RELIABLE | MSG_ZEROCODED ("zero code is not as spec").
        // SendViewerTime sets Header.Zerocoded on SimulatorViewerTimeMessage too.
        const {
            assert!(!ImprovedTerseObjectUpdate::ZEROCODED);
        };
        let message = ImprovedTerseObjectUpdate::default();
        let mut raw = Vec::new();
        build_packet(&message, 15, true, false, &[], &mut raw);
        assert_eq!(raw[0] & PacketFlags::ZEROCODED, 0);
        let parsed = parse_packet(&raw).unwrap();
        let mut payload = Vec::new();
        parsed.id.encode(&mut payload);
        payload.extend_from_slice(&parsed.body);
        let mut coded = Vec::new();
        build_raw(&payload, true, 15, true, false, &[], &mut coded);
        assert_ne!(coded[0] & PacketFlags::ZEROCODED, 0);
        assert_ne!(coded, raw);
        assert_eq!(reencode_datagram(&coded).unwrap(), coded);
        assert_eq!(reencode_datagram(&raw).unwrap(), raw);
    }

    /// RegionHandshake gates the whole region (the simulator sends terrain
    /// and objects only after our reply): it must decode as framed by a
    /// simulator, zerocoded with appended acks, with or without the
    /// trailing RegionInfo4 block that older simulators omit.
    #[test]
    fn region_handshake_from_a_simulator_decodes() {
        let mut m = RegionHandshake::default();
        m.region_info.region_flags = 0x0400_0000;
        m.region_info.sim_access = 13;
        m.region_info.sim_name = crate::str_field("Yikes");
        m.region_info.water_height = 20.0;
        m.region_info.terrain_detail2 = Uuid::from_u128(0xabc);
        m.region_info.terrain_height_range11 = 60.0;
        m.region_info2.region_id = Uuid::from_u128(0x1234_5678);
        m.region_info3.product_name = crate::str_field("Estate / Full Region");
        m.region_info4.push(region_handshake::RegionInfo4 {
            region_flags_extended: 0x0400_0000,
            region_protocols: 1,
        });
        let mut buf = Vec::new();
        build_packet(&m, 3, true, false, &[1, 2], &mut buf);
        assert_ne!(buf[0] & PacketFlags::ZEROCODED, 0);
        let p = parse_packet(&buf).unwrap();
        assert_eq!(
            (p.id, p.acks.as_slice()),
            (RegionHandshake::ID, &[1, 2][..])
        );
        assert_eq!(p.decode::<RegionHandshake>().unwrap(), m);

        // an older simulator: no RegionInfo4 block count at all
        m.region_info4.clear();
        let mut body = Writer::new();
        RegionHandshake::ID.encode(&mut body.buf);
        m.encode_body(&mut body);
        assert_eq!(body.buf.pop(), Some(0));
        m.omitted_trailing_blocks = 1;
        build_raw(&body.buf, true, 4, true, true, &[], &mut buf);
        let p = parse_packet(&buf).unwrap();
        assert!(p.resent());
        assert_eq!(p.decode::<RegionHandshake>().unwrap(), m);
        assert_eq!(reencode_datagram(&buf).unwrap(), buf);
    }

    #[test]
    fn malformed_packets_are_errors() {
        assert!(parse_packet(&[]).is_err());
        assert!(parse_packet(&[0, 0, 0, 0, 0]).is_err());
        // Ack flag set, count claims more bytes than the datagram holds.
        let mut ack = vec![PacketFlags::ACK, 0, 0, 0, 1, 0, 1, 0xff];
        ack.push(9);
        assert!(parse_packet(&ack).is_err());
        // A zero run whose count would expand past the cap.
        let mut bomb = vec![PacketFlags::ZEROCODED, 0, 0, 0, 1, 0, 1];
        bomb.extend(std::iter::repeat_n(0, 2000).flat_map(|_| [0, 255]));
        assert!(parse_packet(&bomb).is_err());
    }

    #[test]
    fn zero_count_of_zero_adds_nothing() {
        // Helpers.ZeroDecode: 0x00 0x00 expands to zero bytes. The 0x02 remains.
        let mut encoded = vec![PacketFlags::ZEROCODED, 0, 0, 0, 7, 0, 1, 0x00, 0x00, 0x02];
        let parsed = parse_packet(&encoded).unwrap();
        assert_eq!(parsed.sequence, 7);
        assert_eq!(parsed.body, vec![0x02]);
        encoded[0] = 0;
        let plain = parse_packet(&encoded).unwrap();
        assert_eq!(plain.body, vec![0x00, 0x00, 0x02]);
    }

    #[test]
    fn garbage_does_not_panic() {
        for len in 0..64usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let _ = parse_packet(&data);
            let mut d2 = data.clone();
            if !d2.is_empty() {
                d2[0] = 0xF0;
            }
            let _ = parse_packet(&d2);
        }
    }
}
