// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extract the riderless linkset edits from the two region proxies and UDP
//! captures. Raw grid traffic stays in scratch; only this object's data leaves it.

use super::{MAX_CAPTURE, Message, bounded, paired};
use crate::live::{CHAT_USER, ESTATE_OWNER};
use ferroan_capture::{Kind, Record, read_capture, write_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, ImprovedTerseObjectUpdate, KillObject, MultipleObjectUpdate,
    ObjectUpdate, UseCircuitCode,
};
use ferroan_protocol::{IncomingPacket, Msg, PacketFlags, Writer, packet::build_raw, parse_packet};
use std::{collections::BTreeSet, fs, path::Path};
use uuid::Uuid;

const ROOT: Uuid = Uuid::from_u128(0x65000000_0000_4000_8000_000000000001);
const CHILD: Uuid = Uuid::from_u128(ROOT.as_u128() + 1);
const OWNER: Uuid = Uuid::from_u128(100);

fn read(path: &Path) -> Result<Vec<Record>, String> {
    read_capture(&bounded(path, MAX_CAPTURE)?).map_err(|e| e.to_string())
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

fn exchange(records: &[Record], dest: &str, alias: &str) -> Result<(String, String), String> {
    let (viewer, region) = if alias == "home" {
        (ESTATE_OWNER, crate::live::REGION_UUID)
    } else {
        (CHAT_USER, crate::grid::NEIGHBOUR_UUID)
    };
    let login_path = format!("/agent/{viewer}/{region}/");
    let mut login_seen = false;
    let object_path = format!("/object/{ROOT}/");
    let mut exchange = None;
    for (index, r) in records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == Kind::HttpRequest)
    {
        let req = Message::parse(&r.bytes)?;
        let path = req.path.split('?').next().unwrap_or_default();
        if req.method == "QUERYACCESS" {
            if path != login_path || exchange.is_some() || login_seen {
                return Err("unexpected access query outside the viewer's login".into());
            }
            let request = super::json_body(&req, "login access query")?;
            if request["viaTeleport"] != true {
                return Err("access query did not belong to a login".into());
            }
            login_seen = true;
        }
        if !path.starts_with("/object/") {
            continue;
        }
        if exchange.is_some() || req.method != "POST" || path != object_path || path != req.path {
            return Err("expected one unadorned object POST per destination".into());
        }
        if !login_seen {
            return Err("missing viewer access evidence before the object transfer".into());
        }
        let res = paired(records, index)?;
        let body = req.text()?;
        let fields: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        if fields["destination_name"] != dest || res.status != Some(200) || res.text()? != "True" {
            return Err("wrong destination or unsuccessful object POST".into());
        }
        let body = body
            .replace(ESTATE_OWNER, &OWNER.to_string())
            .replace(dest, alias);
        let request = format!(
            "POST {path} HTTP/1.1\r\nContent-Type: {}\r\n\r\n{body}",
            req.content_type
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\n\r\n{}",
            res.content_type,
            res.text()?
        );
        exchange = Some((request, response));
    }
    exchange.ok_or_else(|| format!("{alias} received no object POST"))
}

fn project<M: Msg>(packet: &IncomingPacket, msg: &M) -> Vec<u8> {
    let mut body = Writer::new();
    M::ID.encode(&mut body.buf);
    msg.encode_body(&mut body);
    let mut bytes = Vec::new();
    build_raw(
        &body.buf,
        packet.flags & PacketFlags::ZEROCODED != 0,
        packet.sequence,
        packet.reliable(),
        packet.resent(),
        &packet.acks,
        &mut bytes,
    );
    bytes
}

fn viewer(records: Vec<Record>, agent: Uuid, alias: Uuid) -> Result<Vec<Record>, String> {
    let mut selected = Vec::new();
    let mut locals = BTreeSet::new();
    let original_owner: Uuid = ESTATE_OWNER.parse().map_err(|_| "bad harness owner")?;
    let mut session = None;
    let mut pinned_arrival = false;
    let version = format!("+{} ", &ferroan_http::REFERENCE_TRANQUILLITY_REVISION[..10]);
    for record in records {
        let packet = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
        let bytes = match (record.kind, packet.id) {
            (Kind::UdpIn, UseCircuitCode::ID) => {
                let circuit = packet
                    .decode::<UseCircuitCode>()
                    .map_err(|e| e.to_string())?
                    .circuit_code;
                if circuit.id != agent
                    || circuit.session_id.is_nil()
                    || session.is_some_and(|id| id != circuit.session_id)
                {
                    return Err("the capture has another viewer circuit".into());
                }
                session = Some(circuit.session_id);
                continue;
            }
            (Kind::UdpOut, AgentMovementComplete::ID) => {
                let arrival = packet
                    .decode::<AgentMovementComplete>()
                    .map_err(|e| e.to_string())?;
                let channel = std::str::from_utf8(&arrival.sim_data.channel_version)
                    .map_err(|_| "invalid arrival channel")?;
                if arrival.agent_data.agent_id != agent
                    || Some(arrival.agent_data.session_id) != session
                    || !channel.starts_with("OpenSim-NGC Tranquillity ")
                    || !channel.contains(&version)
                {
                    return Err(
                        "arrival does not identify this viewer and the pinned C# build".into(),
                    );
                }
                pinned_arrival = true;
                continue;
            }
            (Kind::UdpOut, ObjectUpdate::ID) => {
                let mut update = packet.decode::<ObjectUpdate>().map_err(|e| e.to_string())?;
                update
                    .object_data
                    .retain(|b| matches!(b.full_id, ROOT | CHILD));
                if update.object_data.is_empty() {
                    continue;
                }
                for b in &mut update.object_data {
                    locals.insert(b.id);
                    if b.owner_id == original_owner {
                        b.owner_id = OWNER;
                    } else if !b.owner_id.is_nil() {
                        return Err("unexpected owner in viewer capture".into());
                    }
                }
                project(&packet, &update)
            }
            (Kind::UdpIn, MultipleObjectUpdate::ID) => {
                let mut edit = packet
                    .decode::<MultipleObjectUpdate>()
                    .map_err(|e| e.to_string())?;
                if !pinned_arrival
                    || Some(edit.agent_data.session_id) != session
                    || edit.agent_data.agent_id != agent
                    || edit.object_data.len() != 1
                    || !locals.contains(&edit.object_data[0].object_local_id)
                {
                    return Err(
                        "edit did not come from the expected viewer for the captured linkset"
                            .into(),
                    );
                }
                edit.agent_data.agent_id = alias;
                edit.agent_data.session_id = Uuid::nil();
                project(&packet, &edit)
            }
            (Kind::UdpOut, ImprovedTerseObjectUpdate::ID) => {
                let mut terse = packet
                    .decode::<ImprovedTerseObjectUpdate>()
                    .map_err(|e| e.to_string())?;
                let mut parts = Vec::new();
                for part in terse.object_data {
                    let id = ferroan_protocol::Reader::new(&part.data)
                        .u32()
                        .map_err(|e| e.to_string())?;
                    if locals.contains(&id) && part.data.get(5) == Some(&0) {
                        parts.push(part);
                    }
                }
                if parts.is_empty() {
                    continue;
                }
                terse.object_data = parts;
                project(&packet, &terse)
            }
            (Kind::UdpOut, KillObject::ID) => {
                let mut kills = packet.decode::<KillObject>().map_err(|e| e.to_string())?;
                kills.object_data.retain(|b| locals.contains(&b.id));
                if kills.object_data.is_empty() {
                    continue;
                }
                project(&packet, &kills)
            }
            _ => continue,
        };
        selected.push(Record { bytes, ..record });
    }
    Ok(selected)
}

pub(in crate::grid) fn extract(run: &Path) -> Result<String, String> {
    let dir = run.join("object-cross");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (side, name, direction, agent, alias, udp) in [
        (
            "home",
            crate::grid::HOME_NAME,
            "westbound",
            ESTATE_OWNER,
            OWNER,
            "capture-udp.cap",
        ),
        (
            "neighbour",
            crate::grid::NEIGHBOUR_NAME,
            "eastbound",
            CHAT_USER,
            Uuid::from_u128(101),
            "capture-udp-neighbour.cap",
        ),
    ] {
        let http = read(&run.join(format!("proxy-{side}.cap")))?;
        let (req, res) = exchange(&http, name, side)?;
        let events = viewer(
            read(&run.join(udp))?,
            agent.parse().map_err(|_| "bad harness account")?,
            alias,
        )?;
        write(
            &dir.join(format!("{direction}.request.http")),
            req.as_bytes(),
        )?;
        write(
            &dir.join(format!("{direction}.response.http")),
            res.as_bytes(),
        )?;
        write(
            &dir.join(format!("{side}.cap")),
            &write_capture(&events).map_err(|e| e.to_string())?,
        )?;
    }
    replay(&dir)?;
    Ok(format!(
        "object cross: two object POSTs, no object access query; fixture in {}",
        dir.display()
    ))
}

struct SourceLink {
    root: u32,
    child: u32,
    x: f32,
}

fn field(part: &ferroan_llsd::xml::Element, path: &str) -> Result<String, String> {
    let el = path
        .split('/')
        .try_fold(part, |node, name| node.child(name))
        .ok_or_else(|| format!("missing XML2 field {path}"))?;
    Ok(el.text())
}

fn source_link(xml: &str) -> Result<SourceLink, String> {
    let group = ferroan_llsd::xml::parse(xml.as_bytes()).map_err(|e| e.to_string())?;
    let mut top = group.elements();
    let root = top.next().ok_or("no XML2 root part")?;
    let others = top.next().ok_or("missing OtherParts")?;
    let mut children = others.elements();
    let child = children.next().ok_or("no XML2 child")?;
    if group.name != "SceneObjectGroup"
        || root.name != "SceneObjectPart"
        || others.name != "OtherParts"
        || child.name != "SceneObjectPart"
        || top.next().is_some()
        || children.next().is_some()
    {
        return Err("crossing XML2 must contain exactly one root and one child".into());
    }
    let root_id: u32 = field(root, "LocalId")?
        .parse()
        .map_err(|_| "bad root local ID")?;
    let child_id: u32 = field(child, "LocalId")?
        .parse()
        .map_err(|_| "bad child local ID")?;
    let owner = OWNER.to_string();
    if field(root, "UUID/UUID")? != ROOT.to_string()
        || field(child, "UUID/UUID")? != CHILD.to_string()
        || root_id == 0
        || child_id == 0
        || root_id == child_id
        || field(root, "ParentID")? != "0"
        || field(child, "ParentID")? != root_id.to_string()
        || field(child, "OffsetPosition/X")? != "1"
        || field(child, "OffsetPosition/Y")? != "2"
        || field(child, "OffsetPosition/Z")? != "0.5"
        || field(root, "OwnerID/UUID")? != owner
        || field(child, "OwnerID/UUID")? != owner
        || field(root, "EveryoneMask")? != "524288"
    {
        return Err("crossing XML2 identity, relative transform or permissions differ".into());
    }
    Ok(SourceLink {
        root: root_id,
        child: child_id,
        x: field(root, "GroupPosition/X")?
            .parse()
            .map_err(|_| "bad source position")?,
    })
}

fn replay_post(
    dir: &Path,
    direction: &str,
    dest: &str,
    x: u32,
    position: &str,
) -> Result<SourceLink, String> {
    let req = Message::parse(&bounded(
        &dir.join(format!("{direction}.request.http")),
        super::MAX_BODY,
    )?)?;
    let res = Message::parse(&bounded(
        &dir.join(format!("{direction}.response.http")),
        4096,
    )?)?;
    let msg: serde_json::Value = serde_json::from_str(&req.text()?).map_err(|e| e.to_string())?;
    let xml = msg["sog"].as_str().ok_or("object POST lacks sog")?;
    if req.method != "POST"
        || req.path != format!("/object/{ROOT}/")
        || res.status != Some(200)
        || res.text()? != "True"
    {
        return Err("wrong object POST path, method or reply".into());
    }
    let region_id = if dest == "home" {
        crate::live::REGION_UUID
    } else {
        crate::grid::NEIGHBOUR_UUID
    };
    let meters_x = x.to_string();
    let meters_y = (crate::grid::HOME_Y * 256).to_string();
    if msg["destination_name"] != dest
        || msg["destination_uuid"] != region_id
        || msg["destination_x"] != meters_x
        || msg["destination_y"] != meters_y
        || msg["new_position"] != position
        || msg["extra"] != "<ExtraFromItemID>00000000-0000-0000-0000-000000000000</ExtraFromItemID>"
        || msg.get("state").is_some()
        || msg.get("hold_riders").is_some()
        || msg["modified"] != true
    {
        return Err(format!(
            "{direction} object POST has incorrect destination, extra, modified or script state"
        ));
    }
    source_link(xml)
}

fn replay_viewer(
    dir: &Path,
    side: &str,
    source: SourceLink,
    target: f32,
    after: Option<f32>,
    viewer: Uuid,
) -> Result<(), String> {
    let region_x = if side == "home" {
        crate::grid::HOME_X
    } else {
        crate::grid::NEIGHBOUR_X
    };
    let handle = ferroan_bot::scenario::region_handle(region_x, crate::grid::HOME_Y);
    let mut roots = BTreeSet::new();
    let mut edits = 0;
    let mut return_roots = BTreeSet::new();
    let mut return_parents = BTreeSet::new();
    let mut saw_source_child = false;
    for record in read(&dir.join(format!("{side}.cap")))? {
        let packet = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
        match (record.kind, packet.id) {
            (Kind::UdpOut, ObjectUpdate::ID) => {
                let update = packet.decode::<ObjectUpdate>().map_err(|e| e.to_string())?;
                if update.region_data.region_handle != handle
                    || !packet.reliable()
                    || packet.flags & PacketFlags::ZEROCODED == 0
                {
                    return Err(
                        "full prim update came from the wrong region or has incorrect wire flags"
                            .into(),
                    );
                }
                for b in update.object_data {
                    if b.object_data.len() != 60 {
                        return Err("expected full prim motion data".into());
                    }
                    let pos = ferroan_protocol::Reader::new(&b.object_data)
                        .vec3()
                        .map_err(|e| e.to_string())?;
                    if b.full_id == ROOT && b.parent_id == 0 && b.id != 0 {
                        if edits == 0
                            && b.id == source.root
                            && pos.to_array() == [source.x, 128., 40.]
                        {
                            roots.insert(b.id);
                        }
                        if after.is_some_and(|x| pos.to_array() == [x, 128., 40.]) && edits > 0 {
                            return_roots.insert(b.id);
                        }
                    } else if b.full_id == CHILD
                        && b.id != 0
                        && b.parent_id != 0
                        && b.parent_id != b.id
                        && pos.to_array() == [1., 2., 0.5]
                    {
                        if edits == 0 && b.id == source.child && b.parent_id == source.root {
                            saw_source_child = true;
                        }
                        if edits > 0 {
                            return_parents.insert(b.parent_id);
                        }
                    } else {
                        return Err(format!(
                            "{side}: unexpected linkset identity or child transform"
                        ));
                    }
                }
            }
            (Kind::UdpIn, MultipleObjectUpdate::ID) => {
                let edit = packet
                    .decode::<MultipleObjectUpdate>()
                    .map_err(|e| e.to_string())?;
                let [b] = edit.object_data.as_slice() else {
                    return Err("expected one root edit".into());
                };
                let position = [target, 128., 40.].map(f32::to_le_bytes).concat();
                if !packet.reliable()
                    || b.type_ != 9
                    || b.data != position
                    || !roots.contains(&b.object_local_id)
                    || edit.agent_data.agent_id != viewer
                    || !edit.agent_data.session_id.is_nil()
                    || (edits != 0 && !packet.resent())
                {
                    return Err(format!("{side}: group-position edit differs"));
                }
                edits += 1;
            }
            (Kind::UdpOut, ImprovedTerseObjectUpdate::ID) => {
                packet
                    .decode::<ImprovedTerseObjectUpdate>()
                    .map_err(|e| e.to_string())?;
            }
            (Kind::UdpOut, KillObject::ID) => {
                packet.decode::<KillObject>().map_err(|e| e.to_string())?;
            }
            _ => return Err("unexpected viewer capture record".into()),
        }
    }
    if edits == 0
        || !saw_source_child
        || (after.is_some() && return_roots.is_disjoint(&return_parents))
    {
        return Err(format!(
            "{side}: missing edit, linked child or return arrival"
        ));
    }
    Ok(())
}

fn replay(dir: &Path) -> Result<(), String> {
    let home = replay_post(
        dir,
        "eastbound",
        "neighbour",
        crate::grid::NEIGHBOUR_X * 256,
        "<8, 128, 40>",
    )?;
    let neighbour = replay_post(
        dir,
        "westbound",
        "home",
        crate::grid::HOME_X * 256,
        "<248, 128, 40>",
    )?;
    if home.x != 130. || neighbour.x != 8. {
        return Err("source position in XML2 differs".into());
    }
    replay_viewer(dir, "home", home, 264., Some(248.), OWNER)?;
    replay_viewer(dir, "neighbour", neighbour, -8., None, Uuid::from_u128(101))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_pin8_riderless_crossing_both_ways() {
        use sha2::{Digest, Sha256};
        let dir = crate::repo_root().join("fixtures/object-cross");
        replay(&dir).expect("crossing replay");
        let mut hashes = String::new();
        for name in [
            "eastbound.request.http",
            "eastbound.response.http",
            "home.cap",
            "neighbour.cap",
            "westbound.request.http",
            "westbound.response.http",
        ] {
            let bytes = bounded(&dir.join(name), MAX_CAPTURE).unwrap();
            hashes.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
        }
        let expected = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        assert_eq!(
            hashes,
            expected.replace("\r\n", "\n"),
            "crossing capture bytes changed"
        );
    }

    #[test]
    fn an_object_access_query_fails_even_if_the_post_would_follow() {
        for path in [
            format!("/object/{ROOT}/"),
            format!("/agent/{ROOT}/destination/"),
        ] {
            let record = Record {
                kind: Kind::HttpRequest,
                time_ms: 0,
                bytes: format!("QUERYACCESS {path} HTTP/1.1\r\n\r\n").into_bytes(),
            };
            assert!(
                exchange(&[record], "region", "home")
                    .unwrap_err()
                    .contains("access query")
            );
        }
    }
}
