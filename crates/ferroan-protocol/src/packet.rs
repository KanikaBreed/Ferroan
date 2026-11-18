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
        if out.len() + add > MAX_ZERO_EXPANSION {
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
    let parsed = parse_packet(data)?;
    if !parsed.extra.is_empty() {
        return Err(DecodeError::Malformed("extra header is not re-encoded"));
    }
    if crate::message_info(parsed.id).is_none() {
        return Err(DecodeError::Malformed("unknown message"));
    }
    let body = crate::reencode_body(parsed.id, &parsed.body)?;
    let mut payload = Vec::with_capacity(4 + body.len());
    parsed.id.encode(&mut payload);
    payload.extend(body);
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
    Ok(out)
}

/// Decode, replace session, agent, name, circuit and IP fields, and encode
/// again. Other bytes follow the same path as [`reencode_datagram`].
pub fn scrub_datagram(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let parsed = parse_packet(data)?;
    if !parsed.extra.is_empty() {
        return Err(DecodeError::Malformed("extra header is not re-encoded"));
    }
    if crate::message_info(parsed.id).is_none() {
        return Err(DecodeError::Malformed("unknown message"));
    }
    let body = crate::scrub_body(parsed.id, &parsed.body)?;
    let mut payload = Vec::with_capacity(4 + body.len());
    parsed.id.encode(&mut payload);
    payload.extend(body);
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
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msgs::*;
    use uuid::Uuid;

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
