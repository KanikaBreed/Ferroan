// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pin8 parcel arrivals. Keep original Land datagrams. Other messages contribute
//! timing and sequence metadata only; no agent, session or capability is published.
use super::login_capture::{bounded, object_cross::decode_original};
use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use ferroan_capture::{Kind, Record, read_capture, write_capture};
use ferroan_protocol::{Msg, msgs::*, parse_packet};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

const OWNERS: [(&str, &str); 2] = [
    ("owner", crate::live::ESTATE_OWNER),
    ("member", crate::live::CHAT_USER),
];
const LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Event {
    ms: u64,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flags: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rates: Option<[f32; 7]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parcel_sequence: Option<i32>,
}

impl Event {
    fn at(ms: u64, name: &str) -> Self {
        Self {
            ms,
            name: name.into(),
            seq: None,
            flags: None,
            rates: None,
            parcel_sequence: None,
        }
    }
}

fn capture(path: &Path) -> Result<Vec<Record>, String> {
    read_capture(&bounded(path, LIMIT)?).map_err(|e| e.to_string())
}

fn gate_millis(line: &str, utc: bool) -> Result<u64, String> {
    let stamp = NaiveDateTime::parse_from_str(
        line.get(..23).ok_or("missing log timestamp")?,
        "%Y-%m-%d %H:%M:%S,%3f",
    )
    .map_err(|e| e.to_string())?;
    let ms = if utc {
        Utc.from_utc_datetime(&stamp).timestamp_millis()
    } else {
        Local
            .from_local_datetime(&stamp)
            .single()
            .ok_or("ambiguous region log timestamp")?
            .timestamp_millis()
    };
    u64::try_from(ms).map_err(|_| "negative log timestamp".to_string())
}

fn gate_lines(path: &Path, needle: &str, utc: bool) -> Result<Vec<u64>, String> {
    let bytes = bounded(path, LIMIT)?;
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| line.contains(needle))
        .map(|line| gate_millis(line, utc))
        .collect()
}

fn gates(root: &Path, agent: &str) -> Result<Vec<u64>, String> {
    let needle = format!("SendInitialData for \"{agent}\"");
    let mut csharp = Vec::new();
    let home = root.join("home");
    if home.is_dir() {
        for file in fs::read_dir(&home).map_err(|e| e.to_string())? {
            let path = file.map_err(|e| e.to_string())?.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if !(name.starts_with("OpenSim.Server.RegionServer") && name.ends_with(".log")) {
                continue;
            }
            csharp.extend(gate_lines(&path, &needle, false)?);
        }
    }
    // A C# gate is local time in the OpenSim log. Do not also read region.log.
    if !csharp.is_empty() {
        return Ok(csharp);
    }
    // Ferroan writes the same sentence to region.log, stamped in UTC.
    let region_log = home.join("region.log");
    if region_log.is_file() {
        return gate_lines(&region_log, &needle, true);
    }
    Ok(Vec::new())
}

fn accept_channel(bytes: &[u8], revision: &str) -> Result<(), String> {
    let text = bytes
        .strip_suffix(&[0])
        .and_then(|body| std::str::from_utf8(body).ok())
        .unwrap_or("");
    if text.starts_with("Ferroan ") {
        return Ok(());
    }
    crate::arrival_fixtures::pinned_channel(bytes, revision).map(|_| ())
}

fn seed_times(records: &[Record], start: u64, end: u64) -> Result<(u64, u64), String> {
    let mut seed = None;
    for pair in records.windows(2) {
        let [req, res] = pair else { continue };
        if req.kind != Kind::HttpRequest
            || !(start..=end).contains(&req.time_ms)
            || !req.bytes.starts_with(b"POST /CAPS/")
            || !req.bytes.windows(13).any(|s| s == b"EventQueueGet")
        {
            continue;
        }
        if res.kind != Kind::HttpResponse
            || !res.bytes.starts_with(b"HTTP/1.1 200")
            || !res.bytes.windows(13).any(|s| s == b"EventQueueGet")
            || seed.is_some()
        {
            return Err("invalid or repeated seed exchange".into());
        }
        seed = Some((req.time_ms, res.time_ms));
    }
    seed.ok_or("missing seed exchange".into())
}

fn timeline(
    root: &Path,
    udp: &[Record],
    http: &[Record],
    revision: &str,
    agent: &str,
) -> Result<(Vec<Event>, Vec<Record>), String> {
    let mut last_logout = 0;
    let mut start = None;
    let mut end = None;
    let mut events = Vec::new();
    let mut land = Vec::new();
    for record in udp {
        let p = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
        if p.id == UseCircuitCode::ID && record.kind == Kind::UdpIn {
            let id = p
                .decode::<UseCircuitCode>()
                .map_err(|e| e.to_string())?
                .circuit_code
                .id;
            if id.to_string() == agent {
                start.get_or_insert(record.time_ms);
            } else if start.is_some() {
                break;
            }
        }
        if start.is_none() {
            if p.id == LogoutReply::ID && record.kind == Kind::UdpOut {
                last_logout = record.time_ms;
            }
            continue;
        }
        let mut rates = None;
        let mut parcel_sequence = None;
        let name = match (record.kind, p.id) {
            (Kind::UdpIn, UseCircuitCode::ID) => "UseCircuitCode",
            (Kind::UdpIn, CompleteAgentMovement::ID) => "CompleteAgentMovement",
            (Kind::UdpOut, AgentMovementComplete::ID) => {
                let msg = p
                    .decode::<AgentMovementComplete>()
                    .map_err(|e| e.to_string())?;
                accept_channel(&msg.sim_data.channel_version, revision)?;
                "AgentMovementComplete"
            }
            (Kind::UdpOut, RegionHandshake::ID) => "RegionHandshake",
            (Kind::UdpIn, RegionHandshakeReply::ID) => "RegionHandshakeReply",
            (Kind::UdpIn, LogoutRequest::ID) => "LogoutRequest",
            (Kind::UdpOut, LogoutReply::ID) => {
                end = Some(record.time_ms);
                "LogoutReply"
            }
            (Kind::UdpIn, AgentThrottle::ID) => {
                let msg = p.decode::<AgentThrottle>().map_err(|e| e.to_string())?;
                let raw = msg.throttle.throttles.as_slice().as_chunks::<4>();
                if raw.0.len() != 7 || !raw.1.is_empty() {
                    return Err("invalid throttle group".into());
                }
                rates = Some(std::array::from_fn(|i| f32::from_le_bytes(raw.0[i])));
                "AgentThrottle"
            }
            (Kind::UdpOut, LayerData::ID) => {
                let msg = p.decode::<LayerData>().map_err(|e| e.to_string())?;
                if !matches!(msg.layer_id.type_, b'L' | b'M') {
                    continue;
                }
                decode_original::<LayerData>(&p, &record.bytes)?;
                "LayerData"
            }
            (Kind::UdpOut, ParcelOverlay::ID) => {
                parcel_sequence = Some(
                    decode_original::<ParcelOverlay>(&p, &record.bytes)?
                        .parcel_data
                        .sequence_id,
                );
                "ParcelOverlay"
            }
            _ => continue,
        };
        if matches!(name, "ParcelOverlay" | "LayerData") {
            land.push(Record {
                kind: record.kind,
                time_ms: record.time_ms,
                bytes: record.bytes.clone(),
            });
        }
        let mut event = Event::at(record.time_ms, name);
        event.seq = Some(p.sequence);
        event.flags = Some(p.flags);
        event.rates = rates;
        event.parcel_sequence = parcel_sequence;
        events.push(event);
        if end.is_some() {
            break;
        }
    }
    let (start, end) = (
        start.ok_or("missing circuit")?,
        end.ok_or("missing logout")?,
    );
    // The seed HTTP request and UseCircuitCode use separate sockets. Either can
    // reach the simulator first; each viewer starts after the prior logout.
    let (req, res) = seed_times(http, last_logout, end)?;
    let origin = start.min(req);
    for record in &mut land {
        record.time_ms = record
            .time_ms
            .checked_sub(origin)
            .ok_or("Land precedes login")?;
    }
    events.push(Event::at(req, "SeedRequest"));
    events.push(Event::at(res, "SeedResponse"));
    let stamps = gates(root, agent)?;
    let [gate] = stamps.as_slice() else {
        return Err("expected one initial-data gate for this viewer".into());
    };
    if !(start..=end).contains(gate) {
        return Err("initial-data gate outside observed circuit".into());
    }
    events.push(Event::at(*gate, "SendInitialData"));
    events.sort_by_key(|e| e.ms);
    for e in &mut events {
        e.ms =
            e.ms.checked_sub(origin)
                .ok_or("event precedes this circuit")?;
    }
    Ok((events, land))
}
pub(super) fn extract(root: &Path) -> Result<String, String> {
    let udp = capture(&root.join("capture-udp.cap"))?;
    let http = capture(&root.join("proxy-home.cap"))?;
    let revision = crate::live::read_pin()?;
    let out = root.join("parcel-timing");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (name, agent) in OWNERS {
        let (events, land) = timeline(root, &udp, &http, &revision, agent)?;
        let json = serde_json::to_vec_pretty(&events).map_err(|e| e.to_string())?;
        fs::write(out.join(format!("{name}.json")), json).map_err(|e| e.to_string())?;
        fs::write(
            out.join(format!("{name}.cap")),
            write_capture(&land).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    replay(&out)
}

/// A parcel sequence is the chunk number (0..3 here). LLUDP's sequence
/// identifies one reliable transmission even when it is resent after another pass.
#[derive(Default)]
struct Passes {
    originals: BTreeMap<u32, (i32, Vec<u8>)>,
    first_sends: Vec<u64>,
    resends: usize,
}

impl Passes {
    fn note(
        &mut self,
        seq: u32,
        resent: bool,
        chunk: i32,
        body: &[u8],
        time: u64,
    ) -> Result<(), String> {
        if let Some((first, bytes)) = self.originals.get(&seq) {
            if !resent || *first != chunk || bytes != body {
                return Err(
                    "repeated LLUDP sequence changed its parcel chunk, body or resend flag".into(),
                );
            }
            self.resends += 1;
        } else {
            if resent {
                return Err("parcel resend lacks original LLUDP transmission".into());
            }
            if chunk != (self.first_sends.len() % 4) as i32 {
                return Err(format!("out-of-order new parcel chunk {chunk}"));
            }
            self.originals.insert(seq, (chunk, body.to_vec()));
            self.first_sends.push(time);
        }
        Ok(())
    }
}

struct Arrival {
    passes: Passes,
    rates: Vec<(u64, [f32; 7])>,
    terrain: usize,
    first_terrain: Option<u64>,
    gate: u64,
}

fn viewer(out: &Path, name: &str) -> Result<Arrival, String> {
    let events: Vec<Event> =
        serde_json::from_slice(&bounded(&out.join(format!("{name}.json")), LIMIT)?)
            .map_err(|e| e.to_string())?;
    if events.windows(2).any(|w| w[0].ms > w[1].ms) {
        return Err("unordered arrival events".into());
    }
    let first = |label: &str| {
        events
            .iter()
            .find(|e| e.name == label)
            .map(|e| e.ms)
            .ok_or_else(|| format!("missing {label}"))
    };
    let gate = first("SendInitialData")?;
    if first("UseCircuitCode")?.min(first("SeedRequest")?) != 0
        || first("CompleteAgentMovement")? > first("AgentMovementComplete")?
        || first("AgentMovementComplete")? > gate
        || first("RegionHandshake")? > first("RegionHandshakeReply")?
        || first("SeedRequest")? > first("SeedResponse")?
        || first("SeedResponse")? > gate
        || gate >= first("LogoutRequest")?
        || first("LogoutRequest")? > first("LogoutReply")?
    {
        return Err(format!("{name}: invalid arrival chronology"));
    }
    let golden = bounded(
        &crate::repo_root().join(format!("fixtures/parcel-overlay/parcel-multi-{name}.bin")),
        8192,
    )?;
    let chunks = golden.as_chunks::<1030>();
    if chunks.0.len() != 4 || !chunks.1.is_empty() {
        return Err("invalid Q60b fixture length".into());
    }
    let mut arrival = Arrival {
        passes: Passes::default(),
        rates: Vec::new(),
        terrain: 0,
        first_terrain: None,
        gate,
    };
    let land = capture(&out.join(format!("{name}.cap")))?;
    let mut records = land.iter();
    for e in &events {
        match e.name.as_str() {
            "AgentThrottle" => {
                if e.flags
                    .is_some_and(|f| f & ferroan_protocol::PacketFlags::RESENT != 0)
                {
                    continue;
                }
                arrival
                    .rates
                    .push((e.ms, e.rates.ok_or("missing throttle rates")?));
            }
            "LayerData" | "ParcelOverlay" => {
                let record = records.next().ok_or("missing Land datagram")?;
                let p = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
                if record.time_ms != e.ms
                    || record.kind != Kind::UdpOut
                    || Some(p.sequence) != e.seq
                    || Some(p.flags) != e.flags
                {
                    return Err("Land datagram disagrees with timeline".into());
                }
                if e.name == "LayerData" {
                    let m = decode_original::<LayerData>(&p, &record.bytes)?;
                    if !matches!(m.layer_id.type_, b'L' | b'M') {
                        return Err("unexpected terrain layer".into());
                    }
                    if arrival.first_terrain.is_none() {
                        arrival.first_terrain = Some(e.ms);
                    }
                    arrival.terrain += 1;
                } else {
                    let m = decode_original::<ParcelOverlay>(&p, &record.bytes)?;
                    let id = m.parcel_data.sequence_id;
                    if e.parcel_sequence != Some(id) {
                        return Err("parcel sequence differs from timeline".into());
                    }
                    let chunk = usize::try_from(id)
                        .ok()
                        .and_then(|i| chunks.0.get(i))
                        .ok_or("bad parcel sequence")?;
                    if !p.reliable() || p.body.as_slice() != chunk {
                        return Err(format!(
                            "{name}: parcel {id} differs from the Q60b payload or reliability"
                        ));
                    }
                    arrival
                        .passes
                        .note(p.sequence, p.resent(), id, &p.body, e.ms)?;
                }
            }
            _ => {}
        }
    }
    if records.next().is_some() || arrival.terrain == 0 || arrival.passes.first_sends.len() != 4 {
        return Err(format!(
            "{name}: expected terrain and exactly one four-chunk overlay through logout"
        ));
    }
    Ok(arrival)
}

fn stable(name: &str, arrival: &Arrival) -> Result<(), String> {
    let terrain = arrival
        .first_terrain
        .ok_or_else(|| format!("{name}: missing land terrain"))?;
    if arrival.passes.first_sends.iter().any(|ms| *ms >= terrain) {
        return Err(format!(
            "{name}: overlay did not precede the first land terrain datagram"
        ));
    }
    if arrival.passes.resends != 0 {
        return Err(format!("{name}: overlay was resent"));
    }
    // Pin8 packs the island into 11 LayerData datagrams. Ferroan sends one
    // patch per datagram, 256 on a 256 m region (`land_layers`).
    if !matches!(arrival.terrain, 11 | 256) {
        return Err(format!(
            "{name}: expected 11 or 256 land terrain datagrams, saw {}",
            arrival.terrain
        ));
    }
    Ok(())
}

fn replay(out: &Path) -> Result<String, String> {
    let control = viewer(out, "owner")?;
    let limited = viewer(out, "member")?;
    let [(_, fast)] = control.rates.as_slice() else {
        return Err("expected one control throttle".into());
    };
    let [(_, slow), (restore, restored)] = limited.rates.as_slice() else {
        return Err("expected limited and restored throttles".into());
    };
    let mut expected_slow = *fast;
    expected_slow[1] = 8000.;
    if fast[1] != 512000. || *slow != expected_slow || fast != restored {
        return Err("only Land may vary between throttle groups".into());
    }
    let a = &control.passes.first_sends;
    let b = &limited.passes.first_sends;
    if a[3] - a[0] >= b[3] - b[0]
        || !(b[1] < *restore && *restore < b[3])
        || b[3] - restore >= b[1] - b[0]
    {
        return Err(format!(
            "Land did not delay and resume outstanding overlay chunks: {a:?}, {b:?}, restored {restore}"
        ));
    }
    stable("owner", &control)?;
    stable("member", &limited)?;
    if !(b[0] < limited.gate && b[1] > limited.gate && b[2] > limited.gate && b[3] > limited.gate) {
        return Err(format!(
            "member's last three overlay chunks did not follow the initial-data gate: {b:?}, gate {}",
            limited.gate
        ));
    }
    Ok(format!(
        "parcel timing: owner overlay at {a:?} ms, member at {b:?} ms; Land restored at {restore} ms; initial-data gates {} / {} ms, terrain datagrams {} / {}, overlay resends {} / {}; fixtures in {}",
        control.gate,
        limited.gate,
        control.terrain,
        limited.terrain,
        control.passes.resends,
        limited.passes.resends,
        out.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin8_parcel_arrival_replays_both_circuits() {
        use sha2::{Digest, Sha256};
        let dir = crate::repo_root().join("fixtures/parcel-timing");
        replay(&dir).unwrap();
        let mut digest = Sha256::new();
        for file in ["owner.json", "owner.cap", "member.json", "member.cap"] {
            digest.update(bounded(&dir.join(file), LIMIT).unwrap());
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            "413ad50c67256eea62f7807212a46d3e689e5ee008cf73109c99c7c46799ef07"
        );
    }

    #[test]
    fn a_later_overlay_pass_is_distinct_from_a_delayed_resend() {
        let mut passes = Passes::default();
        for seq in 1..=8 {
            let chunk = ((seq - 1) % 4) as i32;
            passes
                .note(seq, false, chunk, &[chunk as u8], seq.into())
                .unwrap();
        }
        passes.note(1, true, 0, &[0], 9).unwrap();
        assert_eq!(passes.first_sends.len(), 8);
        assert_eq!(passes.resends, 1);
        assert!(passes.note(1, false, 0, &[0], 10).is_err());
        assert!(passes.note(1, true, 0, &[1], 10).is_err());
        assert!(passes.note(9, true, 0, &[0], 10).is_err());
        assert!(passes.note(1, true, 1, &[0], 10).is_err());
        assert!(passes.note(9, false, 1, &[1], 10).is_err());

        let fixture = crate::repo_root().join("fixtures/parcel-timing");
        let mut events: Vec<Event> =
            serde_json::from_slice(&fs::read(fixture.join("owner.json")).unwrap()).unwrap();
        let mut land = capture(&fixture.join("owner.cap")).unwrap();
        let logout = events
            .iter()
            .position(|e| e.name == "LogoutRequest")
            .unwrap();
        let ms = events[logout].ms - 1;
        let mut seq = events.iter().filter_map(|e| e.seq).max().unwrap();
        let mut markers = Vec::new();
        let mut datagrams = Vec::new();
        let land_events = events
            .iter()
            .filter(|e| matches!(e.name.as_str(), "ParcelOverlay" | "LayerData"));
        for (event, record) in land_events.zip(&land) {
            if event.name == "ParcelOverlay" {
                // Give each captured chunk a fresh LLUDP sequence for the second pass.
                seq += 1;
                let mut marker = event.clone();
                marker.ms = ms;
                marker.seq = Some(seq);
                markers.push(marker);
                let mut datagram = record.clone();
                datagram.time_ms = ms;
                datagram.bytes[1..5].copy_from_slice(&seq.to_be_bytes());
                datagrams.push(datagram);
            }
        }
        assert_eq!(markers.len(), 4);
        events.splice(logout..logout, markers);
        land.extend(datagrams);

        let dir = std::env::temp_dir().join(format!("parcel-timing-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("owner.json"), serde_json::to_vec(&events).unwrap()).unwrap();
        fs::write(dir.join("owner.cap"), write_capture(&land).unwrap()).unwrap();
        let result = viewer(&dir, "owner");
        let _ = fs::remove_dir_all(dir);
        assert_eq!(
            result.err().as_deref(),
            Some("owner: expected terrain and exactly one four-chunk overlay through logout")
        );
    }
}
