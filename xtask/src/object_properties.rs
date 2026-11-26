// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pin8 object properties, the physics event and the two root kills.
//! Each datagram is re-encoded before any identifier is replaced. The
//! published bytes are the message body after the id, with the prim-update
//! aliases applied.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_capture::{Kind, Record, read_capture};
use ferroan_protocol::msgs::{
    DeRezObject, KillObject, ObjectProperties, ObjectPropertiesFamily, ObjectSelect, ObjectUpdate,
    RequestObjectPropertiesFamily,
};
use ferroan_protocol::{IncomingPacket, Msg, Writer, parse_packet};
use uuid::Uuid;

use super::login_capture::{Message, bounded, object_cross::decode_original, records};

const LIMIT: u64 = 64 * 1024 * 1024;
const OWNER: &str = crate::live::ESTATE_OWNER;
const LINKSET: &str = "65000000-0000-4000-8000-000000000001";
const FAMILY_ROOTS: usize = PRIM_CAPTURE_COUNT - 1;

struct Aliases {
    uuid: BTreeMap<Uuid, Uuid>,
    local_of: BTreeMap<Uuid, u32>,
    owner: Uuid,
}

struct Captured {
    families: Vec<Vec<u8>>,
    objects: Vec<Vec<u8>>,
    kills: Vec<Vec<u8>>,
    physics: Vec<Vec<u8>>,
}

pub(super) fn extract(root: &Path, ferroan: bool) -> Result<String, String> {
    let udp = read_capture(&bounded(&root.join("capture-udp.cap"), LIMIT)?)
        .map_err(|error| error.to_string())?;
    let http = records(&root.join("proxy-home.cap"))?;
    let captured = select(&udp, &http)?;
    if ferroan {
        compare(&fixture_dir(), &captured)?;
        return Ok(
            "object-properties: family, object, kill and physics bytes match the Pin8 fixture"
                .into(),
        );
    }
    let dir = root.join("object-properties");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    write_all(&dir, &captured)?;
    Ok(format!(
        "object-properties: {} family, {} object, {} kill, {} physics; fixture in {}",
        captured.families.len(),
        captured.objects.len(),
        captured.kills.len(),
        captured.physics.len(),
        dir.display()
    ))
}

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/object-properties")
}

fn select(udp: &[Record], http: &[Record]) -> Result<Captured, String> {
    let aliases = aliases()?;
    let mut local_by_full = BTreeMap::<Uuid, u32>::new();
    let mut families = Vec::new();
    let mut objects = Vec::new();
    let mut kills = Vec::new();
    let mut requests = Vec::new();
    let mut selected = Vec::new();
    let mut deleted = Vec::new();
    for record in udp {
        if record.kind != Kind::UdpIn && record.kind != Kind::UdpOut {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        if packet.resent() {
            continue;
        }
        match (record.kind, packet.id) {
            (Kind::UdpOut, ObjectUpdate::ID) => {
                note_locals(&packet, &record.bytes, &aliases, &mut local_by_full)?;
            }
            (Kind::UdpIn, RequestObjectPropertiesFamily::ID) => {
                let message =
                    decode_original::<RequestObjectPropertiesFamily>(&packet, &record.bytes)?;
                if message.agent_data.agent_id != aliases.owner {
                    return Err("properties family request is not the estate owner".into());
                }
                if message.object_data.request_flags != 0 {
                    return Err("properties family request flags are not zero".into());
                }
                requests.push(message.object_data.object_id);
            }
            (Kind::UdpIn, ObjectSelect::ID) => {
                let message = decode_original::<ObjectSelect>(&packet, &record.bytes)?;
                if message.agent_data.agent_id != aliases.owner {
                    return Err("object select is not the estate owner".into());
                }
                selected.push(
                    message
                        .object_data
                        .iter()
                        .map(|block| block.object_local_id)
                        .collect::<Vec<_>>(),
                );
            }
            (Kind::UdpIn, DeRezObject::ID) => {
                let message = decode_original::<DeRezObject>(&packet, &record.bytes)?;
                if message.agent_data.agent_id != aliases.owner
                    || message.agent_block.destination != 6
                {
                    continue;
                }
                deleted.push(
                    message
                        .object_data
                        .iter()
                        .map(|block| block.object_local_id)
                        .collect::<Vec<_>>(),
                );
            }
            (Kind::UdpOut, ObjectPropertiesFamily::ID) => {
                families.push(family_body(&packet, &record.bytes, &aliases)?);
            }
            (Kind::UdpOut, ObjectProperties::ID) => {
                objects.push(object_body(&packet, &record.bytes, &aliases)?);
            }
            (Kind::UdpOut, KillObject::ID) => {
                if let Some(body) = kill_body(&packet, &record.bytes, &local_by_full, &aliases)? {
                    kills.push(body);
                }
            }
            _ => {}
        }
    }
    expect_script(&aliases, &local_by_full, &requests, &selected, &deleted)?;
    expect_replies(&families, &objects, &kills)?;
    let physics = physics_bodies(http, &local_by_full, &aliases)?;
    Ok(Captured {
        families,
        objects,
        kills,
        physics,
    })
}

fn aliases() -> Result<Aliases, String> {
    let map: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/prim-updates/map.json"))
            .map_err(|error| error.to_string())?;
    let mut uuid = BTreeMap::new();
    let mut local_of = BTreeMap::new();
    let owner = parse_uuid(OWNER, "harness owner")?;
    let owner_alias = parse_uuid(
        map["owner"]["capture_uuid"]
            .as_str()
            .ok_or("missing owner alias")?,
        "owner alias",
    )?;
    uuid.insert(owner, owner_alias);
    for part in map["parts"].as_array().ok_or("missing prim map")? {
        let store = parse_uuid(
            part["store_uuid"].as_str().ok_or("missing store uuid")?,
            "store uuid",
        )?;
        let capture = parse_uuid(
            part["capture_uuid"]
                .as_str()
                .ok_or("missing capture uuid")?,
            "capture uuid",
        )?;
        uuid.insert(store, capture);
        let number = u32::try_from(capture.as_u128()).map_err(|_| "capture uuid does not fit")?;
        local_of.insert(store, number);
    }
    if local_of.len() != PRIM_CAPTURE_COUNT {
        return Err("prim map does not list the stored parts".into());
    }
    Ok(Aliases {
        uuid,
        local_of,
        owner,
    })
}

fn parse_uuid(text: &str, what: &str) -> Result<Uuid, String> {
    text.parse().map_err(|_| format!("{what} is not a uuid"))
}

fn note_locals(
    packet: &IncomingPacket,
    bytes: &[u8],
    aliases: &Aliases,
    local_by_full: &mut BTreeMap<Uuid, u32>,
) -> Result<(), String> {
    let message = decode_original::<ObjectUpdate>(packet, bytes)?;
    for block in message.object_data {
        if block.p_code == 47 || !aliases.local_of.contains_key(&block.full_id) {
            continue;
        }
        match local_by_full.get(&block.full_id) {
            Some(seen) if *seen != block.id => {
                return Err(format!("stored part {} changed local id", block.full_id));
            }
            Some(_) => {}
            None => {
                local_by_full.insert(block.full_id, block.id);
            }
        }
    }
    Ok(())
}

fn expect_script(
    aliases: &Aliases,
    local_by_full: &BTreeMap<Uuid, u32>,
    requests: &[Uuid],
    selected: &[Vec<u32>],
    deleted: &[Vec<u32>],
) -> Result<(), String> {
    let mut want = aliases.local_of.keys().copied().collect::<Vec<_>>();
    want.sort();
    if requests != want.as_slice() {
        return Err(format!(
            "expected one family request per stored part in uuid order, found {}",
            requests.len()
        ));
    }
    let live: Vec<u32> = want
        .iter()
        .map(|id| {
            local_by_full
                .get(id)
                .copied()
                .ok_or_else(|| format!("no full update for {id}"))
        })
        .collect::<Result<_, _>>()?;
    if selected != [live.clone()] {
        return Err("object select is not the eight parts in store uuid order".into());
    }
    let linkset = LINKSET.parse::<Uuid>().map_err(|_| "linkset uuid")?;
    let root = *local_by_full
        .get(&linkset)
        .ok_or("no full update for the linkset root")?;
    if deleted != [vec![root]] {
        return Err("delete is not the linkset root".into());
    }
    Ok(())
}

fn family_body(
    packet: &IncomingPacket,
    bytes: &[u8],
    aliases: &Aliases,
) -> Result<Vec<u8>, String> {
    let mut message = proven::<ObjectPropertiesFamily>(packet, bytes)?;
    let block = &mut message.object_data;
    block.object_id = alias_uuid(block.object_id, "ObjectID", aliases)?;
    block.owner_id = alias_uuid(block.owner_id, "OwnerID", aliases)?;
    block.group_id = alias_uuid(block.group_id, "GroupID", aliases)?;
    block.last_owner_id = alias_uuid(block.last_owner_id, "LastOwnerID", aliases)?;
    Ok(body_bytes(&message))
}

fn object_body(
    packet: &IncomingPacket,
    bytes: &[u8],
    aliases: &Aliases,
) -> Result<Vec<u8>, String> {
    let mut message = proven::<ObjectProperties>(packet, bytes)?;
    for block in &mut message.object_data {
        block.object_id = alias_uuid(block.object_id, "ObjectID", aliases)?;
        block.creator_id = alias_uuid(block.creator_id, "CreatorID", aliases)?;
        block.owner_id = alias_uuid(block.owner_id, "OwnerID", aliases)?;
        block.group_id = alias_uuid(block.group_id, "GroupID", aliases)?;
        block.item_id = alias_uuid(block.item_id, "ItemID", aliases)?;
        block.folder_id = alias_uuid(block.folder_id, "FolderID", aliases)?;
        block.from_task_id = alias_uuid(block.from_task_id, "FromTaskID", aliases)?;
        block.last_owner_id = alias_uuid(block.last_owner_id, "LastOwnerID", aliases)?;
    }
    Ok(body_bytes(&message))
}

fn kill_body(
    packet: &IncomingPacket,
    bytes: &[u8],
    local_by_full: &BTreeMap<Uuid, u32>,
    aliases: &Aliases,
) -> Result<Option<Vec<u8>>, String> {
    let mut message = proven::<KillObject>(packet, bytes)?;
    let mut aliases_for_blocks = Vec::with_capacity(message.object_data.len());
    for block in &message.object_data {
        aliases_for_blocks.push(alias_local(block.id, local_by_full, aliases)?);
    }
    if aliases_for_blocks.iter().all(Option::is_none) {
        return Ok(None);
    }
    if aliases_for_blocks.iter().any(Option::is_none) {
        return Err("KillObject mixes a stored part with another id".into());
    }
    for (block, alias) in message.object_data.iter_mut().zip(aliases_for_blocks) {
        block.id = alias.ok_or("stored kill lost its alias")?;
    }
    if message.object_data.len() != 1 || message.object_data[0].id != 1 {
        return Err("prim KillObject is not the linkset root".into());
    }
    Ok(Some(body_bytes(&message)))
}

fn proven<M: Msg>(packet: &IncomingPacket, bytes: &[u8]) -> Result<M, String> {
    let message = decode_original::<M>(packet, bytes)?;
    // `packet.body` is the zero-decoded message after the id. The datagram
    // check above already included the id, the header and any appended acks.
    if body_bytes(&message) != packet.body {
        return Err(format!(
            "unredacted {} body differs from the captured payload",
            M::NAME
        ));
    }
    Ok(message)
}

fn body_bytes<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

fn alias_uuid(id: Uuid, field: &str, aliases: &Aliases) -> Result<Uuid, String> {
    if id.is_nil() {
        return Ok(id);
    }
    aliases
        .uuid
        .get(&id)
        .copied()
        .ok_or_else(|| format!("{field} {id} is not in the prim map"))
}

fn alias_local(
    local_id: u32,
    local_by_full: &BTreeMap<Uuid, u32>,
    aliases: &Aliases,
) -> Result<Option<u32>, String> {
    let Some(full_id) = local_by_full
        .iter()
        .find_map(|(full_id, live)| (*live == local_id).then_some(*full_id))
    else {
        return Ok(None);
    };
    aliases
        .local_of
        .get(&full_id)
        .copied()
        .map(Some)
        .ok_or_else(|| format!("local id {local_id} has no alias"))
}

fn expect_replies(
    families: &[Vec<u8>],
    objects: &[Vec<u8>],
    kills: &[Vec<u8>],
) -> Result<(), String> {
    if families.len() != FAMILY_ROOTS {
        return Err(format!(
            "expected {FAMILY_ROOTS} family packets, found {}",
            families.len()
        ));
    }
    if objects.is_empty() {
        return Err("no ObjectProperties packets".into());
    }
    if kills.len() != 2 {
        return Err(format!(
            "expected two prim KillObject packets, found {}",
            kills.len()
        ));
    }
    Ok(())
}

fn physics_bodies(
    http: &[Record],
    local_by_full: &BTreeMap<Uuid, u32>,
    aliases: &Aliases,
) -> Result<Vec<Vec<u8>>, String> {
    let mut stack = Vec::new();
    let mut bodies = Vec::new();
    for record in http {
        match record.kind {
            Kind::HttpRequest => {
                let message = Message::parse(&record.bytes)?;
                let poll = message.method == "POST"
                    && message.path.starts_with("/CE/")
                    && !probe(&record.bytes);
                stack.push(poll);
            }
            Kind::HttpResponse => {
                let poll = stack.pop().ok_or("event-queue response has no request")?;
                if !poll {
                    continue;
                }
                let message = Message::parse(&record.bytes)?;
                if message.status != Some(200) {
                    continue;
                }
                let text = message.text()?;
                for element in physics_elements(text.as_bytes())? {
                    bodies.push(redact_local_ids(element, local_by_full, aliases)?);
                }
            }
            _ => {}
        }
    }
    if bodies.is_empty() {
        return Err("no ObjectPhysicsProperties event".into());
    }
    Ok(bodies)
}

fn probe(bytes: &[u8]) -> bool {
    let needle = b"x-ferroan-probe";
    bytes
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn physics_elements(body: &[u8]) -> Result<Vec<&[u8]>, String> {
    let needle = b"<string>ObjectPhysicsProperties</string>";
    let mut elements = Vec::new();
    let mut from = 0;
    while let Some(at) = body[from..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let start = body[..from + at]
            .windows(5)
            .rposition(|window| window == b"<map>")
            .ok_or("ObjectPhysicsProperties has no opening map")?;
        let end = map_end(body, start)?;
        elements.push(&body[start..end]);
        from = end;
    }
    Ok(elements)
}

fn map_end(body: &[u8], start: usize) -> Result<usize, String> {
    let mut index = start;
    let mut depth = 0i32;
    while index < body.len() {
        if body[index..].starts_with(b"<map>") {
            depth += 1;
            index += 5;
            continue;
        }
        if body[index..].starts_with(b"</map>") {
            depth -= 1;
            index += 6;
            if depth == 0 {
                return Ok(index);
            }
            continue;
        }
        index += 1;
    }
    Err("ObjectPhysicsProperties map does not close".into())
}

fn redact_local_ids(
    xml: &[u8],
    local_by_full: &BTreeMap<Uuid, u32>,
    aliases: &Aliases,
) -> Result<Vec<u8>, String> {
    let needle = b"<key>LocalID</key>";
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let after = at + needle.len();
        out.extend_from_slice(&rest[..after]);
        rest = &rest[after..];
        let open = b"<integer>";
        let close = b"</integer>";
        if !rest.starts_with(open) {
            return Err("LocalID is not an integer element".into());
        }
        let Some(end) = rest.windows(close.len()).position(|window| window == close) else {
            return Err("LocalID integer does not close".into());
        };
        let digits = std::str::from_utf8(&rest[open.len()..end])
            .map_err(|_| "LocalID is not text")?
            .trim();
        let local_id: u32 = digits.parse().map_err(|_| "LocalID is not a number")?;
        let alias = alias_local(local_id, local_by_full, aliases)?
            .ok_or_else(|| format!("physics LocalID {local_id} is not a stored part"))?;
        out.extend_from_slice(open);
        out.extend_from_slice(alias.to_string().as_bytes());
        out.extend_from_slice(close);
        rest = &rest[end + close.len()..];
    }
    out.extend_from_slice(rest);
    Ok(out)
}

fn write_all(dir: &Path, captured: &Captured) -> Result<(), String> {
    for (index, bytes) in captured.families.iter().enumerate() {
        write(dir, &format!("family-{}.bin", index + 1), bytes)?;
    }
    for (index, bytes) in captured.objects.iter().enumerate() {
        write(dir, &format!("object-{}.bin", index + 1), bytes)?;
    }
    for (index, bytes) in captured.kills.iter().enumerate() {
        write(dir, &format!("kill-{}.bin", index + 1), bytes)?;
    }
    for (index, bytes) in captured.physics.iter().enumerate() {
        write(dir, &format!("physics-{}.xml", index + 1), bytes)?;
    }
    Ok(())
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(dir.join(name), bytes).map_err(|error| error.to_string())
}

fn compare(dir: &Path, captured: &Captured) -> Result<(), String> {
    let pairs = listed(captured);
    let mut expected = BTreeSet::new();
    for (name, bytes) in &pairs {
        let stored = fs::read(dir.join(name)).map_err(|_| format!("missing fixture {name}"))?;
        if stored != **bytes {
            return Err(format!("{name} differs from the Pin8 fixture"));
        }
        expected.insert(name.as_str());
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == "README.md" {
            continue;
        }
        if !expected.contains(name) {
            return Err(format!("fixture has extra {name}"));
        }
    }
    Ok(())
}

fn listed(captured: &Captured) -> Vec<(String, &Vec<u8>)> {
    let mut pairs = Vec::new();
    for (index, bytes) in captured.families.iter().enumerate() {
        pairs.push((format!("family-{}.bin", index + 1), bytes));
    }
    for (index, bytes) in captured.objects.iter().enumerate() {
        pairs.push((format!("object-{}.bin", index + 1), bytes));
    }
    for (index, bytes) in captured.kills.iter().enumerate() {
        pairs.push((format!("kill-{}.bin", index + 1), bytes));
    }
    for (index, bytes) in captured.physics.iter().enumerate() {
        pairs.push((format!("physics-{}.xml", index + 1), bytes));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_physics_local_id_keeps_the_surrounding_bytes() {
        let xml =
            b"<map><key>LocalID</key><integer>42</integer><key>Density</key><real>0</real></map>";
        let full = Uuid::from_u128(7);
        let mut local_by_full = BTreeMap::new();
        local_by_full.insert(full, 42);
        let mut aliases = Aliases {
            uuid: BTreeMap::new(),
            local_of: BTreeMap::new(),
            owner: Uuid::nil(),
        };
        aliases.local_of.insert(full, 3);
        let redacted = redact_local_ids(xml, &local_by_full, &aliases).unwrap();
        assert_eq!(
            redacted,
            b"<map><key>LocalID</key><integer>3</integer><key>Density</key><real>0</real></map>"
        );
    }
}
