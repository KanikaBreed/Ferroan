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
//     (ProcessEntityUpdates' full and terse budgets and full packet assembly)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPZeroEncoder.cs (Position, ZeroCount and Finish)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! ProcessEntityUpdates at Pin8, LLClientView.cs 4996–5431 and LLUDPZeroEncoder.cs.
//! Dequeue estimates and the encoded packet split are independent. Material
//! bodies ride along after the full and terse lists and do not spend the budget.

use crate::AVATAR_TERSE_LEN;
use ferroan_protocol::{
    Msg, Writer,
    msgs::{ObjectProperties, ObjectUpdate, object_update::ObjectData},
    packet::{HEADER_SIZE, MAX_DATAGRAM},
};

/// `LLUDPServer.MAXPAYLOAD`. Property packets split at this absolute position.
const MAX_PAYLOAD: usize = 1200;

/// One callback's Task sends: full packets, terse blocks, material bodies, then kills.
pub struct TaskPackets {
    pub full: Vec<Vec<u8>>,
    pub terse: Vec<[u8; AVATAR_TERSE_LEN]>,
    pub materials: Vec<Vec<u8>>,
    /// Local ids recorded from `PrimUpdateFlags.Kill` during this drain.
    pub kills: Vec<u32>,
}

/// One Task queue-empty callback, bounded by its byte estimate and reliable window.
pub struct TaskBatch {
    budget: i32,
    slots: usize,
    region_handle: u64,
    full: Option<FullPackets>,
    terse: Vec<[u8; AVATAR_TERSE_LEN]>,
    materials: Vec<Vec<u8>>,
    kills: Vec<u32>,
}

impl TaskBatch {
    /// Drain one callback, preserving full packet boundaries and the pending terse blocks.
    pub fn drain(
        feed: &dyn crate::EntityUpdates,
        handle: u64,
        budget: i32,
        slots: usize,
    ) -> TaskPackets {
        let mut batch = Self::new(handle, budget, slots);
        feed.take_updates(&mut batch);
        batch.finish()
    }

    fn new(handle: u64, budget: i32, slots: usize) -> Self {
        Self {
            budget: if budget <= 0 { i32::MAX } else { budget },
            slots,
            region_handle: handle,
            full: None,
            terse: Vec::new(),
            materials: Vec::new(),
            kills: Vec::new(),
        }
    }

    /// `PrimUpdateFlags.Kill`: 30 estimated bytes, no full or terse slot.
    pub fn note_kill(&mut self, local_id: u32) {
        self.budget -= 30;
        self.kills.push(local_id);
    }

    pub fn has_room(&self) -> bool {
        self.budget > 0 && self.slots > 0
    }

    pub fn push_terse(&mut self, block: [u8; AVATAR_TERSE_LEN]) {
        self.slots -= 1;
        self.budget -= 63 + if self.terse.is_empty() { 18 } else { 0 };
        self.terse.push(block);
    }

    pub fn push_full(&mut self, block: &ObjectData, animation_count: Option<usize>) {
        self.slots -= 1;
        let cost = if matches!(block.p_code, 95 | 111 | 255) {
            70
        } else {
            150
        };
        self.budget -= cost + if self.full.is_none() { 18 } else { 0 };
        if let Some(count) = animation_count {
            let bytes = 24usize.saturating_add(20usize.saturating_mul(count));
            self.budget = self
                .budget
                .saturating_sub(i32::try_from(bytes).unwrap_or(i32::MAX));
        }
        let mut writer = Writer::new();
        block.encode(&mut writer);
        self.full
            .get_or_insert_with(|| FullPackets::new(self.region_handle))
            .push(&writer.buf);
    }

    /// A material body is already encoded. It does not take a slot or estimated bytes.
    pub fn push_material(&mut self, body: Vec<u8>) {
        self.materials.push(body);
    }

    fn finish(self) -> TaskPackets {
        TaskPackets {
            full: self.full.map(FullPackets::finish).unwrap_or_default(),
            terse: self.terse,
            materials: self.materials,
            kills: self.kills,
        }
    }

    #[cfg(test)]
    pub(crate) fn budget(&self) -> i32 {
        self.budget
    }
}

/// LLUDPZeroEncoder.Position excludes the open run until Finish. Payloads
/// here start just after the six uncoded header bytes.
#[derive(Clone, Default)]
struct CodedBody {
    bytes: Vec<u8>,
    zeros: u8,
}

impl CodedBody {
    fn append(&mut self, data: &[u8]) {
        for &byte in data {
            if byte == 0 {
                if self.zeros == 255 {
                    self.flush_zeros();
                }
                self.zeros += 1;
            } else {
                self.flush_zeros();
                self.bytes.push(byte);
            }
        }
    }

    fn flush_zeros(&mut self) {
        if self.zeros != 0 {
            self.bytes.extend_from_slice(&[0, self.zeros]);
            self.zeros = 0;
        }
    }
}

struct FullPackets {
    header: CodedBody,
    current: CodedBody,
    count: u8,
    packets: Vec<Vec<u8>>,
}

impl FullPackets {
    fn new(handle: u64) -> Self {
        let mut prefix = Vec::new();
        ObjectUpdate::ID.encode(&mut prefix);
        prefix.extend_from_slice(&handle.to_le_bytes());
        prefix.extend_from_slice(&u16::MAX.to_le_bytes());
        prefix.push(1); // nonzero placeholder closes the header's pending run
        let mut header = CodedBody::default();
        header.append(&prefix);
        Self {
            current: header.clone(),
            header,
            count: 0,
            packets: Vec::new(),
        }
    }

    fn push(&mut self, block: &[u8]) {
        let last = self.current.bytes.len();
        let pending = self.current.zeros;
        self.current.append(block);
        if self.count != 0
            && (HEADER_SIZE + self.current.bytes.len() >= 900 || self.count == u8::MAX)
        {
            self.current.bytes.truncate(last);
            self.current.zeros = pending;
            self.emit();
            self.current.append(block);
        }
        let size =
            HEADER_SIZE + self.current.bytes.len() + if self.current.zeros == 0 { 0 } else { 2 };
        if size > MAX_DATAGRAM {
            ferroan_diag::drop_bound(
                ferroan_diag::UDP_DATAGRAM,
                ferroan_diag::Kind::Hard,
                MAX_DATAGRAM as u64,
                size as u64,
                "udp",
            );
            self.current = self.header.clone();
            return;
        }
        self.count += 1;
    }

    fn emit(&mut self) {
        if self.count == 0 {
            return;
        }
        self.current.bytes[self.header.bytes.len() - 1] = self.count;
        self.current.flush_zeros();
        let complete = std::mem::replace(&mut self.current, self.header.clone());
        self.packets.push(complete.bytes);
        self.count = 0;
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        self.emit();
        self.packets
    }
}

/// `ProcessEntityPropertyRequests` object packets. `Position` excludes the open
/// zero run. A block that would land at or past `MAXPAYLOAD` starts the next
/// packet when an earlier block is already in this one. The count byte stays
/// nonzero, so overwriting the placeholder does not change the zero coding.
pub fn object_property_packets(blocks: &[Vec<u8>]) -> Vec<Vec<u8>> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut packets = PropertyPackets::new();
    for block in blocks {
        packets.push(block);
    }
    packets.finish()
}

struct PropertyPackets {
    header: CodedBody,
    current: CodedBody,
    count: u8,
    packets: Vec<Vec<u8>>,
}

impl PropertyPackets {
    fn new() -> Self {
        let mut prefix = Vec::new();
        ObjectProperties::ID.encode(&mut prefix);
        prefix.push(1);
        let mut header = CodedBody::default();
        header.append(&prefix);
        Self {
            current: header.clone(),
            header,
            count: 0,
            packets: Vec::new(),
        }
    }

    fn push(&mut self, block: &[u8]) {
        let last = self.current.bytes.len();
        let pending = self.current.zeros;
        self.current.append(block);
        // C# `Position` is six plus this length. It splits at `Position >= MAXPAYLOAD`.
        if self.count != 0 && HEADER_SIZE + self.current.bytes.len() >= MAX_PAYLOAD {
            self.current.bytes.truncate(last);
            self.current.zeros = pending;
            self.emit();
            self.current.append(block);
        }
        let size =
            HEADER_SIZE + self.current.bytes.len() + if self.current.zeros == 0 { 0 } else { 2 };
        if size > MAX_DATAGRAM {
            ferroan_diag::drop_bound(
                ferroan_diag::UDP_DATAGRAM,
                ferroan_diag::Kind::Hard,
                MAX_DATAGRAM as u64,
                size as u64,
                "udp",
            );
            self.current = self.header.clone();
            return;
        }
        self.count += 1;
    }

    fn emit(&mut self) {
        if self.count == 0 {
            return;
        }
        self.current.bytes[self.header.bytes.len() - 1] = self.count;
        self.current.flush_zeros();
        let complete = std::mem::replace(&mut self.current, self.header.clone());
        self.packets.push(complete.bytes);
        self.count = 0;
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        self.emit();
        self.packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::parse_packet;

    #[test]
    fn position_899_keeps_a_block_and_900_splits_with_the_open_run() {
        for position in [899, 900] {
            let mut full = FullPackets::new(7);
            let header = full.header.bytes.len();
            full.push(&[7, 0, 0, 0]);
            // The next nonzero closes the three pending zeros. Its final
            // two zeros stay pending when the split decision is made.
            let mut second = vec![8; position - HEADER_SIZE - header - 3];
            second.extend([0, 0]);
            full.push(&second);
            let packets = full.finish();
            if position == 899 {
                assert_eq!(packets.len(), 1);
                assert_eq!(packets[0][header - 1], 2);
                assert_eq!(&packets[0][header..header + 3], &[7, 0, 3]);
            } else {
                assert_eq!(packets.len(), 2);
                assert_eq!(&packets[0][header - 1..], &[1, 7, 0, 3]);
                assert_eq!(packets[1][header - 1], 1);
                assert_eq!(packets[1][header], 8);
            }
            assert!(packets.last().unwrap().ends_with(&[0, 2]));
        }
    }

    #[test]
    fn a_large_first_part_that_fits_is_sent_alone_without_an_empty_packet() {
        let mut full = FullPackets::new(7);
        let at = full.header.bytes.len();
        let part = vec![1; 1000];
        full.push(&part);
        let packets = full.finish();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][at - 1], 1);
        assert_eq!(&packets[0][at..], part);
    }

    #[test]
    fn a_block_larger_than_one_datagram_leaves_the_next_part_deliverable() {
        for prefix in [false, true] {
            let mut full = FullPackets::new(99);
            let at = full.header.bytes.len();
            if prefix {
                full.push(&[1, 0]);
            }
            full.push(&vec![1; MAX_DATAGRAM]);
            full.push(&[9, 0]);
            let packets = full.finish();
            assert_eq!(packets.len(), 1 + usize::from(prefix));
            if prefix {
                assert_eq!(&packets[0][at - 1..], &[1, 1, 0, 1]);
            }
            assert_eq!(&packets.last().unwrap()[at - 1..], &[1, 9, 0, 1]);
        }
    }

    #[test]
    fn dequeue_charges_each_list_once_and_each_part_by_kind() {
        let mut batch = TaskBatch::new(7, 469, 6);
        for code in [95, 111, 255, 9] {
            batch.push_full(
                &ObjectData {
                    p_code: code,
                    ..Default::default()
                },
                None,
            );
        }
        assert_eq!(batch.budget(), 469 - 18 - 3 * 70 - 150);
        batch.push_terse([0; AVATAR_TERSE_LEN]);
        assert_eq!(batch.budget(), 10);
        batch.push_terse([0; AVATAR_TERSE_LEN]);
        assert!(!batch.has_room());
        let mut flush = TaskBatch::new(7, 0, 1);
        flush.push_terse([0; AVATAR_TERSE_LEN]);
        assert!(!flush.has_room());
    }

    #[test]
    fn a_selected_empty_animation_collection_costs_24_bytes() {
        for (count, cost) in [(None, 0), (Some(0), 24), (Some(3), 84)] {
            let mut batch = TaskBatch::new(7, 500, 1);
            batch.push_full(&ObjectData::default(), count);
            assert_eq!(batch.budget(), 500 - 18 - 150 - cost);
        }
    }

    #[test]
    fn eight_control_blocks_in_one_callback_reproduce_both_pin8_datagrams() {
        let capture = include_bytes!("../../../fixtures/prim-arrival/control.cap");
        let mut expected = Vec::new();
        let mut parts = Vec::new();
        let mut handle = None;
        for record in ferroan_capture::read_capture(capture.as_slice()).unwrap() {
            let packet = parse_packet(&record.bytes).unwrap();
            if record.kind != ferroan_capture::Kind::UdpOut || !matches!(packet.sequence, 32 | 33) {
                continue;
            }
            assert_eq!(record.bytes[0], 0xc0);
            let update = packet.decode::<ObjectUpdate>().unwrap();
            handle = Some(update.region_data.region_handle);
            assert_eq!(
                update.object_data.len(),
                if packet.sequence == 32 { 6 } else { 2 }
            );
            parts.extend(update.object_data);
            expected.push(record.bytes[HEADER_SIZE..].to_vec());
        }
        assert_eq!(parts.len(), 8);
        assert_eq!(
            expected
                .iter()
                .map(|body| HEADER_SIZE + body.len())
                .collect::<Vec<_>>(),
            [754, 733]
        );
        let mut batch = TaskBatch::new(handle.unwrap(), 0, 8);
        for part in parts {
            batch.push_full(&part, None);
        }
        assert_eq!(batch.finish().full, expected);
    }

    #[test]
    fn a_large_texture_is_charged_by_its_coded_size_and_later_updates_survive() {
        let mut batch = TaskBatch::new(7, 0, 4);
        // Pin8's room estimate for this single field already exceeds its buffer.
        let compressible = vec![0; 3000];
        assert!(compressible.len() + compressible.len() / 2 + 4 > MAX_DATAGRAM);
        for (id, texture_entry) in [
            (1, compressible.clone()),
            (2, vec![1; MAX_DATAGRAM]),
            (3, vec![7]),
        ] {
            batch.push_full(
                &ObjectData {
                    id,
                    texture_entry,
                    ..Default::default()
                },
                None,
            );
        }
        batch.push_terse([9; AVATAR_TERSE_LEN]);
        let TaskPackets { full, terse, .. } = batch.finish();
        let mut received = Vec::new();
        for body in full {
            let mut wire = vec![0xc0, 0, 0, 0, 1, 0];
            wire.extend(body);
            assert!(wire.len() <= MAX_DATAGRAM);
            let update = parse_packet(&wire)
                .unwrap()
                .decode::<ObjectUpdate>()
                .unwrap();
            received.extend(
                update
                    .object_data
                    .into_iter()
                    .map(|part| (part.id, part.texture_entry)),
            );
        }
        assert_eq!(received, [(1, compressible), (3, vec![7])]);
        assert_eq!(terse, [[9; AVATAR_TERSE_LEN]]);
    }

    #[test]
    fn both_pin8_circuits_full_prim_packets_keep_the_captured_coded_body() {
        for capture in [
            include_bytes!("../../../fixtures/prim-arrival/control.cap").as_slice(),
            include_bytes!("../../../fixtures/prim-arrival/limited.cap").as_slice(),
        ] {
            let mut compared = 0;
            for record in ferroan_capture::read_capture(capture).unwrap() {
                let packet = parse_packet(&record.bytes).unwrap();
                if packet.id != ObjectUpdate::ID {
                    continue;
                }
                let update = packet.decode::<ObjectUpdate>().unwrap();
                if update.object_data.iter().any(|part| part.p_code == 47) {
                    continue;
                }
                let mut batch = TaskBatch::new(update.region_data.region_handle, 0, 255);
                for part in &update.object_data {
                    batch.push_full(part, None);
                }
                let TaskPackets { full, .. } = batch.finish();
                let end = record.bytes.len()
                    - if packet.acks.is_empty() {
                        0
                    } else {
                        1 + 4 * packet.acks.len()
                    };
                assert_eq!(full.len(), 1);
                assert_eq!(full[0], record.bytes[HEADER_SIZE..end]);
                compared += 1;
            }
            assert!(compared >= 2);
        }
    }

    #[test]
    fn a_material_body_does_not_spend_the_budget_or_the_slot() {
        let mut batch = TaskBatch::new(7, 100, 1);
        batch.push_material(vec![31, 1]);
        assert_eq!(batch.budget(), 100);
        assert!(batch.has_room());
        batch.push_full(
            &ObjectData {
                id: 2,
                texture_entry: vec![1; MAX_DATAGRAM],
                ..Default::default()
            },
            None,
        );
        batch.push_material(b"kept".to_vec());
        let TaskPackets {
            full,
            terse,
            materials,
            ..
        } = batch.finish();
        assert!(full.is_empty());
        assert!(terse.is_empty());
        assert_eq!(materials, vec![vec![31, 1], b"kept".to_vec()]);
    }

    #[test]
    fn property_packets_split_when_the_coded_position_reaches_maxpayload() {
        let block = vec![0x11; 400];
        let packets = object_property_packets(&[block.clone(), block.clone(), block]);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0][2], 2);
        assert_eq!(packets[1][2], 1);
        assert_eq!(packets[0].len(), 3 + 800);
        assert_eq!(packets[1].len(), 3 + 400);

        // Position is 6 + bytes.len(). 1193 keeps (position 1199). 1194 splits.
        let first = vec![0x11];
        let keep = vec![0x11; 1189];
        let kept = object_property_packets(&[first.clone(), keep]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].len(), 1193);
        assert_eq!(kept[0][2], 2);

        let split = vec![0x11; 1190];
        let split_packets = object_property_packets(&[first, split]);
        assert_eq!(split_packets.len(), 2);
        assert_eq!(split_packets[0][2], 1);
        assert_eq!(split_packets[1][2], 1);

        assert!(object_property_packets(&[vec![0x11; 4100]]).is_empty());
    }

    #[test]
    fn a_kill_costs_thirty_bytes_and_no_slot() {
        let mut batch = TaskBatch::new(7, 40, 1);
        batch.note_kill(9);
        assert_eq!(batch.budget(), 10);
        assert!(batch.has_room());
        batch.note_kill(10);
        assert!(!batch.has_room());
        assert_eq!(batch.finish().kills, vec![9, 10]);
    }
}
