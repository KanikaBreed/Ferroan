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
//! Dequeue estimates and the encoded packet split are independent.

use crate::AVATAR_TERSE_LEN;
use ferroan_protocol::{
    Msg, Writer,
    msgs::{ObjectUpdate, object_update::ObjectData},
    packet::{HEADER_SIZE, MAX_DATAGRAM},
};

/// One Task queue-empty callback, bounded by its byte estimate and reliable window.
pub struct TaskBatch {
    budget: i32,
    slots: usize,
    region_handle: u64,
    full: Option<FullPackets>,
    terse: Vec<[u8; AVATAR_TERSE_LEN]>,
}

impl TaskBatch {
    pub(crate) fn new(handle: u64, budget: i32, slots: usize) -> Self {
        Self {
            budget: if budget <= 0 { i32::MAX } else { budget },
            slots,
            region_handle: handle,
            full: None,
            terse: Vec::new(),
        }
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

    pub(crate) fn finish(self) -> (Vec<Vec<u8>>, Vec<[u8; AVATAR_TERSE_LEN]>) {
        (
            self.full.map(FullPackets::finish).unwrap_or_default(),
            self.terse,
        )
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
                let (full, _) = batch.finish();
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
}
