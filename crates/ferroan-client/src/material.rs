// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (ProcessEntityUpdates' needMaterials send, 5731–5786,
//      and GenericStreamingMessageHeader)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The separate material-override datagram on Task.
//!
//! Pin8 writes it after the full, probe, terse and animation sends. A tree is
//! left out at dequeue. An empty override list uses a different key order from
//! a list that has entries. A notation past 4080 bytes is refused before that
//! string is built. The HTTP RenderMaterials capability is not this path.

use std::borrow::Cow;

use ferroan_protocol::msgs::GenericStreamingMessage;
use ferroan_protocol::msgs::generic_streaming_message::{DataBlock, MethodData};
use ferroan_protocol::{Msg, Writer};

/// `GenericStreamingMessage` method for a glTF material override.
pub const METHOD: u16 = 16757;

/// `UDPPacketBuffer.BUFFER_SIZE` minus the 16 bytes Pin8 keeps in front of the text.
pub const MAX_NOTATION_BYTES: usize = 4080;

const TREE_PCODE: [u8; 3] = [95, 111, 255];

/// One stored override face. `data` is spliced raw, the way `OSDllsdxml` is written.
pub struct Face<'a> {
    pub te_index: u8,
    pub data: &'a str,
}

/// The plain message body, or `None` when Pin8 sends nothing.
///
/// `faces` is `None` when render materials are absent. An empty slice is the
/// clear packet. The body is the message number plus the unencoded template body.
pub fn packet(local_id: u32, pcode: u8, faces: Option<&[Face<'_>]>) -> Option<Vec<u8>> {
    if TREE_PCODE.contains(&pcode) {
        return None;
    }
    let faces = faces?;
    let nonempty = !faces.is_empty();
    let notation = if nonempty {
        let parts = nonempty_parts(local_id, faces);
        let bytes = notation_bytes(&parts);
        if bytes > MAX_NOTATION_BYTES as u64 {
            refuse_over_cap(bytes);
            return None;
        }
        join_parts(&parts)
    } else {
        empty_notation(local_id)
    };
    if !within_cap(&notation, nonempty) {
        return None;
    }
    Some(encode(&notation))
}

/// Nonempty maps are `id`, `te`, `od`, and the id is the signed cast of the local id.
///
/// Face text stays borrowed. The caller measures [`notation_bytes`] before
/// [`join_parts`], so an override of several megabytes is not copied into one string.
fn nonempty_parts<'a>(local_id: u32, faces: &'a [Face<'a>]) -> Vec<Cow<'a, str>> {
    let mut parts = Vec::with_capacity(faces.len().saturating_mul(4).saturating_add(3));
    parts.push(Cow::Borrowed("{'id':i"));
    parts.push(Cow::Owned((local_id as i32).to_string()));
    parts.push(Cow::Borrowed(",'te':["));
    for (index, face) in faces.iter().enumerate() {
        if index > 0 {
            parts.push(Cow::Borrowed(","));
        }
        parts.push(Cow::Owned(format!("i{}", face.te_index)));
    }
    parts.push(Cow::Borrowed("],'od':["));
    for (index, face) in faces.iter().enumerate() {
        if index > 0 {
            parts.push(Cow::Borrowed(","));
        }
        if face.data.is_empty() {
            parts.push(Cow::Borrowed("!"));
        } else {
            parts.push(Cow::Borrowed(face.data));
        }
    }
    parts.push(Cow::Borrowed("]}"));
    parts
}

/// UTF-8 length of `parts`. Each face's stored text is counted, not copied.
fn notation_bytes(parts: &[Cow<'_, str>]) -> u64 {
    parts
        .iter()
        .fold(0u64, |sum, part| sum.saturating_add(part.len() as u64))
}

fn join_parts(parts: &[Cow<'_, str>]) -> String {
    let mut notation = String::new();
    for part in parts {
        notation.push_str(part);
    }
    notation
}

/// The empty list interpolates the unsigned local id and orders the keys `id`, `od`, `te`.
fn empty_notation(local_id: u32) -> String {
    format!("{{'id':i{local_id},'od':[],'te':[]}}")
}

/// A nonempty string shorter than four UTF-16 code units is skipped.
/// [`packet`] refuses a notation past [`MAX_NOTATION_BYTES`] before [`join_parts`].
fn within_cap(notation: &str, nonempty: bool) -> bool {
    !nonempty || notation.encode_utf16().count() >= 4
}

fn refuse_over_cap(observed: u64) {
    ferroan_diag::refuse(
        ferroan_diag::OBJECT_MATERIAL_NOTATION_BYTES,
        ferroan_diag::Kind::Hard,
        MAX_NOTATION_BYTES as u64,
        observed,
        "material",
    );
}

fn encode(notation: &str) -> Vec<u8> {
    let message = GenericStreamingMessage {
        method_data: MethodData { method: METHOD },
        data_block: DataBlock {
            data: notation.as_bytes().to_vec(),
        },
    };
    let mut writer = Writer::new();
    GenericStreamingMessage::ID.encode(&mut writer.buf);
    message.encode_body(&mut writer);
    writer.buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::packet::{build_raw, parse_packet};
    use ferroan_protocol::{Msg, PacketFlags};

    fn face<'a>(te_index: u8, data: &'a str) -> Face<'a> {
        Face { te_index, data }
    }

    fn notation(local_id: u32, pcode: u8, faces: Option<&[Face<'_>]>) -> Option<String> {
        let body = packet(local_id, pcode, faces)?;
        let message = GenericStreamingMessage::decode(&body[1..]).unwrap();
        assert_eq!(message.method_data.method, METHOD);
        Some(String::from_utf8(message.data_block.data).unwrap())
    }

    #[test]
    fn two_faces_keep_insertion_order_and_splice_the_stored_text() {
        let faces = [face(0, "{'bc':[r1,r0,r0,r1]}"), face(2, "{'ds':1}")];
        assert_eq!(
            notation(7, 9, Some(&faces)).unwrap(),
            "{'id':i7,'te':[i0,i2],'od':[{'bc':[r1,r0,r0,r1]},{'ds':1}]}"
        );
    }

    #[test]
    fn an_empty_face_is_undef_and_the_clear_packet_uses_the_other_key_order() {
        let faces = [face(0, ""), face(2, "{'ds':1}")];
        assert_eq!(
            notation(7, 9, Some(&faces)).unwrap(),
            "{'id':i7,'te':[i0,i2],'od':[!,{'ds':1}]}"
        );
        assert_eq!(
            notation(7, 9, Some(&[])).unwrap(),
            "{'id':i7,'od':[],'te':[]}"
        );
        assert_eq!(
            notation(u32::MAX, 9, Some(&[])).unwrap(),
            "{'id':i4294967295,'od':[],'te':[]}"
        );
        let high = [face(0, "{'bc':[r1,r0,r0,r1]}")];
        assert_eq!(
            notation(u32::MAX, 9, Some(&high)).unwrap(),
            "{'id':i-1,'te':[i0],'od':[{'bc':[r1,r0,r0,r1]}]}"
        );
    }

    #[test]
    fn a_tree_or_a_missing_material_list_sends_nothing() {
        let faces = [face(0, "{'tree':1}")];
        for pcode in TREE_PCODE {
            assert!(packet(3, pcode, Some(&faces)).is_none());
        }
        assert!(packet(3, 9, None).is_none());
    }

    #[test]
    fn a_short_nonempty_string_is_skipped_and_the_clear_packet_is_not() {
        assert!(!within_cap("abc", true));
        assert!(within_cap("abcd", true));
        assert!(within_cap("abc", false));
    }

    #[test]
    fn notation_past_the_datagram_budget_is_skipped_and_the_boundary_is_kept() {
        let prefix = "{'id':i1,'te':[i0],'od':[";
        let suffix = "]}";
        let room = MAX_NOTATION_BYTES - prefix.len() - suffix.len();
        let fitting = "a".repeat(room);
        let over = "a".repeat(room + 1);
        let kept = [face(0, &fitting)];
        let dropped = [face(0, &over)];
        let parts = nonempty_parts(1, &dropped);
        assert!(notation_bytes(&parts) > MAX_NOTATION_BYTES as u64);
        let body = packet(1, 9, Some(&kept)).unwrap();
        let message = GenericStreamingMessage::decode(&body[1..]).unwrap();
        assert_eq!(message.data_block.data.len(), MAX_NOTATION_BYTES);
        assert!(packet(1, 9, Some(&dropped)).is_none());
        assert!(ferroan_diag::metrics().contains(
            "ferroan_limit_hits_total{limit=\"object.material-notation-bytes\",action=\"refuse\"}"
        ));
    }

    #[test]
    fn the_datagram_is_reliable_and_left_unencoded() {
        let faces = [face(0, "{'bc':[r1,r0,r0,r1]}")];
        let body = packet(4, 9, Some(&faces)).unwrap();
        let mut datagram = Vec::new();
        build_raw(&body, false, 9, true, false, &[], &mut datagram);
        assert_eq!(datagram[0] & PacketFlags::ZEROCODED, 0);
        assert_ne!(datagram[0] & PacketFlags::RELIABLE, 0);
        let packet = parse_packet(&datagram).unwrap();
        let message = packet.decode::<GenericStreamingMessage>().unwrap();
        assert_eq!(message.method_data.method, METHOD);
        assert!(message.data_block.data.starts_with(b"{'id':i4,"));
    }
}
