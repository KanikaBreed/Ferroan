// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-msg/build.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// The template this reads is libopenmetaverse/data/message_template.msg
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// originally the Second Life message template (LGPL-2.1).
// Eight trailing-block extensions follow Second Life viewer
//   scripts/messages/message_template.msg @ ff75ab037607d8fe305f43f8b1f43126693ba2db
//   Copyright (c) Linden Research, Inc., originally LGPL 2.1.
// Default IM suffix presence follows LLClientView.SendInstantMessage in
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c.

//! Generates typed message structs and codecs from `message_template.msg`.

// INFALLIBLE: this build script turns the in-repo message template into
// generated code. A bad template, a missing file, or a missing OUT_DIR fails
// the build. None of these are runtime input.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Debug)]
enum VarType {
    Simple(&'static str, &'static str), // (rust type, codec suffix)
    Fixed(usize),
    Variable(u8),
}

#[derive(Debug)]
struct Var {
    name: String,
    ty: VarType,
}

#[derive(Debug)]
enum BlockKind {
    Single,
    Multiple(usize),
    Variable,
}

#[derive(Debug)]
struct Block {
    name: String,
    kind: BlockKind,
    vars: Vec<Var>,
}

#[derive(Debug)]
struct Message {
    name: String,
    freq: String,
    number: u32,
    trusted: bool,
    zerocoded: bool,
    deprecated: bool,
    blocks: Vec<Block>,
}

/// Only these suffixes supplement the pinned libopenmetaverse template.
fn viewer_extensions(name: &str) -> usize {
    match name {
        "ImprovedInstantMessage" | "RegionInfo" => 2,
        "AlertMessage"
        | "AvatarAppearance"
        | "ChildAgentUpdate"
        | "InviteGroupResponse"
        | "SimStatus"
        | "TransferInventory" => 1,
        _ => 0,
    }
}

fn optional_suffix(m: &Message) -> usize {
    let extensions = viewer_extensions(&m.name);
    m.blocks
        .iter()
        .enumerate()
        .rev()
        .take_while(|(index, block)| {
            *index >= m.blocks.len() - extensions || matches!(block.kind, BlockKind::Variable)
        })
        .count()
}

fn fixed_width(ty: &VarType) -> Option<usize> {
    match ty {
        VarType::Simple(_, "bool" | "u8" | "i8") => Some(1),
        VarType::Simple(_, "u16" | "i16" | "ipport") => Some(2),
        VarType::Simple(_, "u32" | "i32" | "f32" | "ipaddr") => Some(4),
        VarType::Simple(_, "u64" | "i64" | "f64") => Some(8),
        VarType::Simple(_, "vec3" | "quat") => Some(12),
        VarType::Simple(_, "vec4" | "uuid") => Some(16),
        VarType::Simple(_, "dvec3") => Some(24),
        VarType::Fixed(n) => Some(*n),
        VarType::Variable(_) => None,
        VarType::Simple(_, other) => panic!("unknown field width {other}"),
    }
}

/// A requested omit applies only while the block is empty. Variable emptiness
/// is an empty collection; a vector of default elements is still data.
fn empty_block_expr(modname: &str, block: &Block) -> String {
    let field = snake(&block.name);
    match block.kind {
        BlockKind::Variable => format!("self.{field}.is_empty()"),
        BlockKind::Single => {
            format!("self.{field} == {modname}::{}::default()", block.name)
        }
        BlockKind::Multiple(_) => format!(
            "self.{field}.iter().all(|block| *block == {modname}::{}::default())",
            block.name
        ),
    }
}

/// libopenmetaverse `NeedValidateIDs`: an AgentData single that contains both
/// AgentID and SessionID. Offsets are into the message body.
fn id_offsets(message: &Message) -> Option<(usize, usize)> {
    let agent_at = message.blocks.iter().position(|block| {
        block.name == "AgentData"
            && matches!(block.kind, BlockKind::Single)
            && block.vars.iter().any(|var| var.name == "AgentID")
            && block.vars.iter().any(|var| var.name == "SessionID")
    })?;
    let mut offset = 0usize;
    for block in &message.blocks[..agent_at] {
        let width = match block.kind {
            BlockKind::Single => fixed_width_block(message, block),
            BlockKind::Multiple(count) => fixed_width_block(message, block) * count,
            BlockKind::Variable => panic!(
                "{} variable block {} precedes AgentData ids",
                message.name, block.name
            ),
        };
        offset += width;
    }
    let mut agent = None;
    let mut session = None;
    let mut inner = offset;
    for var in &message.blocks[agent_at].vars {
        if agent.is_some() && session.is_some() {
            break;
        }
        let Some(width) = fixed_width(&var.ty) else {
            panic!(
                "{} variable field {}.{} precedes agent ids",
                message.name, message.blocks[agent_at].name, var.name
            );
        };
        if var.name == "AgentID" {
            agent = Some(inner);
        }
        if var.name == "SessionID" {
            session = Some(inner);
        }
        inner += width;
    }
    Some((agent.expect("AgentID"), session.expect("SessionID")))
}

fn fixed_width_block(message: &Message, block: &Block) -> usize {
    block
        .vars
        .iter()
        .map(|var| {
            fixed_width(&var.ty).unwrap_or_else(|| {
                panic!(
                    "{} variable field {}.{} precedes AgentData ids",
                    message.name, block.name, var.name
                )
            })
        })
        .sum()
}

fn tokenize(src: &str) -> Vec<String> {
    let mut toks = Vec::new();
    for line in src.lines() {
        let line = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        let spaced = line.replace('{', " { ").replace('}', " } ");
        toks.extend(spaced.split_whitespace().map(|s| s.to_owned()));
    }
    toks
}

fn parse_num(s: &str) -> u32 {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).expect("hex number")
    } else {
        s.parse().expect("number")
    }
}

fn parse(src: &str) -> Vec<Message> {
    let toks = tokenize(src);
    let mut i = 0;
    let mut msgs = Vec::new();
    // skip "version 2.0"
    while i < toks.len() && toks[i] != "{" {
        i += 1;
    }
    while i < toks.len() {
        assert_eq!(toks[i], "{", "expected message start at token {i}");
        i += 1;
        let name = toks[i].clone();
        let freq = toks[i + 1].clone();
        let number = parse_num(&toks[i + 2]);
        let trusted = toks[i + 3] == "Trusted";
        let zerocoded = toks[i + 4] == "Zerocoded";
        i += 5;
        let mut deprecated = false;
        while toks[i] != "{" && toks[i] != "}" {
            if toks[i] == "Deprecated" {
                deprecated = true;
            }
            i += 1;
        }
        let mut blocks = Vec::new();
        while toks[i] == "{" {
            i += 1;
            let bname = toks[i].clone();
            let kind = match toks[i + 1].as_str() {
                "Single" => {
                    i += 2;
                    BlockKind::Single
                }
                "Multiple" => {
                    let n = parse_num(&toks[i + 2]) as usize;
                    i += 3;
                    BlockKind::Multiple(n)
                }
                "Variable" => {
                    i += 2;
                    BlockKind::Variable
                }
                other => panic!("bad block kind {other}"),
            };
            let mut vars = Vec::new();
            while toks[i] == "{" {
                let vname = toks[i + 1].clone();
                let ty = match toks[i + 2].as_str() {
                    "U8" => VarType::Simple("u8", "u8"),
                    "U16" => VarType::Simple("u16", "u16"),
                    "U32" => VarType::Simple("u32", "u32"),
                    "U64" => VarType::Simple("u64", "u64"),
                    "S8" => VarType::Simple("i8", "i8"),
                    "S16" => VarType::Simple("i16", "i16"),
                    "S32" => VarType::Simple("i32", "i32"),
                    "S64" => VarType::Simple("i64", "i64"),
                    "F32" => VarType::Simple("f32", "f32"),
                    "F64" => VarType::Simple("f64", "f64"),
                    "LLVector3" => VarType::Simple("glam::Vec3", "vec3"),
                    "LLVector3d" => VarType::Simple("glam::DVec3", "dvec3"),
                    "LLVector4" => VarType::Simple("glam::Vec4", "vec4"),
                    "LLQuaternion" => VarType::Simple("glam::Quat", "quat"),
                    "LLUUID" => VarType::Simple("uuid::Uuid", "uuid"),
                    "BOOL" => VarType::Simple("bool", "bool"),
                    "IPADDR" => VarType::Simple("[u8; 4]", "ipaddr"),
                    "IPPORT" => VarType::Simple("u16", "ipport"),
                    "Fixed" => VarType::Fixed(parse_num(&toks[i + 3]) as usize),
                    "Variable" => VarType::Variable(parse_num(&toks[i + 3]) as u8),
                    other => panic!("bad var type {other}"),
                };
                i += match ty {
                    VarType::Fixed(_) | VarType::Variable(_) => 5,
                    VarType::Simple(..) => 4,
                };
                vars.push(Var { name: vname, ty });
            }
            assert_eq!(toks[i], "}", "expected block end for {bname}");
            i += 1;
            blocks.push(Block {
                name: bname,
                kind,
                vars,
            });
        }
        assert_eq!(toks[i], "}", "expected message end for {name}");
        i += 1;
        msgs.push(Message {
            name,
            freq,
            number,
            trusted,
            zerocoded,
            deprecated,
            blocks,
        });
    }
    msgs
}

fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (idx, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev = if idx > 0 { Some(chars[idx - 1]) } else { None };
            let next = chars.get(idx + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    const KW: &[&str] = &[
        "type", "ref", "move", "loop", "match", "mod", "override", "self", "super", "use", "where",
        "as", "fn", "impl", "in", "for", "if", "else", "let", "struct", "enum", "trait", "static",
        "const", "box", "final", "virtual", "priv", "macro", "async", "await", "dyn", "abstract",
        "become", "do", "try", "yield", "crate", "extern", "pub", "return", "true", "false",
        "unsafe", "while", "break", "continue", "gen",
    ];
    if KW.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// Fields a committed capture must not keep. Object UUIDs stay. Avatar
/// names, sessions, circuit codes and every IP address do not.
fn scrub_line(block: &str, field: &str, ty: &VarType, rust_field: &str) -> Option<String> {
    let block = block.to_ascii_lowercase();
    let field = field.to_ascii_lowercase();
    let text = field == "name"
        || field == "fromname"
        || field == "fromagentname"
        || field == "firstname"
        || field == "lastname"
        || field == "username"
        || field == "namevalue"
        || field == "objectowner"
        || field.contains("password")
        || field.contains("passwd")
        || field.contains("secret")
        || field.contains("token")
        || field.contains("email");
    match ty {
        VarType::Simple(_, "ipaddr") => Some(format!("self.{rust_field} = [127, 0, 0, 1];")),
        VarType::Simple(_, "uuid") if field.contains("session") => {
            Some(format!("self.{rust_field} = uuid::Uuid::from_u128(2);"))
        }
        VarType::Simple(_, "uuid")
            if field.contains("agentid")
                || field.contains("userid")
                || field.contains("ownerid")
                || field.contains("creatorid")
                || field.contains("abuserid")
                || field.contains("reporterid")
                || (field == "id" && block.contains("circuit")) =>
        {
            Some(format!("self.{rust_field} = uuid::Uuid::from_u128(1);"))
        }
        VarType::Simple(_, "u32") if field == "code" || field.contains("circuitcode") => {
            Some(format!("self.{rust_field} = 1;"))
        }
        VarType::Variable(_) if text => {
            Some(format!("self.{rust_field} = b\"Redacted\".to_vec();"))
        }
        _ => None,
    }
}

fn generate(msgs: &[Message]) -> String {
    let mut s = String::new();
    s.push_str("// @generated from message_template.msg by build.rs. Do not edit.\n\n");

    for m in msgs {
        let optional = optional_suffix(m);
        let optional_start = m.blocks.len() - optional;
        let modname = snake(&m.name);
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            "Low" => format!("MsgId::Low({})", m.number),
            "Fixed" => format!("MsgId::Low({})", m.number & 0xFFFF),
            f => panic!("bad freq {f}"),
        };

        // block structs
        let _ = writeln!(s, "pub mod {modname} {{");
        let _ = writeln!(
            s,
            "    #[allow(unused_imports)] use crate::{{Reader, Writer, DecodeError}};"
        );
        for b in &m.blocks {
            let _ = writeln!(s, "    #[derive(Debug, Clone, Default, PartialEq)]");
            let _ = writeln!(s, "    pub struct {} {{", b.name);
            for v in &b.vars {
                let ty = match &v.ty {
                    VarType::Simple(t, _) => (*t).to_owned(),
                    VarType::Fixed(_) | VarType::Variable(_) => "Vec<u8>".to_owned(),
                };
                let _ = writeln!(s, "        pub {}: {},", snake(&v.name), ty);
            }
            let _ = writeln!(s, "    }}");
            let _ = writeln!(s, "    impl {} {{", b.name);
            let _ = writeln!(
                s,
                "        #[inline] pub fn encode(&self, w: &mut Writer) {{"
            );
            for v in &b.vars {
                let f = snake(&v.name);
                match &v.ty {
                    VarType::Simple(_, c) => {
                        let _ = writeln!(s, "            w.{c}(self.{f});");
                    }
                    VarType::Fixed(n) => {
                        let _ = writeln!(s, "            w.fixed(&self.{f}, {n});");
                    }
                    VarType::Variable(1) => {
                        let _ = writeln!(s, "            w.var1(&self.{f});");
                    }
                    VarType::Variable(_) => {
                        let _ = writeln!(s, "            w.var2(&self.{f});");
                    }
                }
            }
            let _ = writeln!(s, "        }}");
            let _ = writeln!(
                s,
                "        #[inline] pub fn decode(r: &mut Reader) -> Result<Self, DecodeError> {{"
            );
            let _ = writeln!(s, "            Ok(Self {{");
            for v in &b.vars {
                let f = snake(&v.name);
                match &v.ty {
                    VarType::Simple(_, c) => {
                        let _ = writeln!(s, "                {f}: r.{c}()?,");
                    }
                    VarType::Fixed(n) => {
                        let _ = writeln!(s, "                {f}: r.fixed({n})?,");
                    }
                    VarType::Variable(1) => {
                        let _ = writeln!(s, "                {f}: r.var1()?,");
                    }
                    VarType::Variable(_) => {
                        let _ = writeln!(s, "                {f}: r.var2()?,");
                    }
                }
            }
            let _ = writeln!(s, "            }})");
            let _ = writeln!(s, "        }}");
            let _ = writeln!(s, "        #[inline] pub fn scrub(&mut self) {{");
            for v in &b.vars {
                if let Some(line) = scrub_line(&b.name, &v.name, &v.ty, &snake(&v.name)) {
                    let _ = writeln!(s, "            {line}");
                }
            }
            let _ = writeln!(s, "        }}");
            let _ = writeln!(s, "    }}");
        }
        let _ = writeln!(s, "}}");

        // message struct
        let _ = writeln!(s, "#[derive(Debug, Clone, PartialEq)]");
        let _ = writeln!(s, "pub struct {} {{", m.name);
        for b in &m.blocks {
            let f = snake(&b.name);
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "    pub {f}: {modname}::{},", b.name);
                }
                _ => {
                    let _ = writeln!(s, "    pub {f}: Vec<{modname}::{}>,", b.name);
                }
            }
        }
        if optional != 0 {
            let _ = writeln!(
                s,
                "    /// Number of wholly absent blocks at the end of the body. Values above {optional} are clamped; required blocks are always encoded."
            );
            let _ = writeln!(
                s,
                "    /// Set to zero to emit every block, including empty Variable count bytes. Decoding preserves this distinction."
            );
            let _ = writeln!(s, "    pub omitted_trailing_blocks: usize,");
        }
        let _ = writeln!(s, "}}");
        let _ = writeln!(
            s,
            "impl Default for {} {{ fn default() -> Self {{ Self {{",
            m.name
        );
        for b in &m.blocks {
            let _ = writeln!(s, "    {}: Default::default(),", snake(&b.name));
        }
        if optional != 0 {
            // LLClientView.SendInstantMessage writes EstateBlock's zero U32,
            // then ends the body without a MetaData count. Other senders omit
            // their viewer-only extensions entirely at the pinned revision.
            let omitted = if m.name == "ImprovedInstantMessage" {
                1
            } else {
                viewer_extensions(&m.name)
            };
            let _ = writeln!(s, "    omitted_trailing_blocks: {omitted},");
        }
        let _ = writeln!(s, "}} }} }}");
        let _ = writeln!(s, "impl crate::Msg for {} {{", m.name);
        let _ = writeln!(s, "    const NAME: &'static str = \"{}\";", m.name);
        let _ = writeln!(s, "    const ID: MsgId = {id};");
        let _ = writeln!(s, "    const ZEROCODED: bool = {};", m.zerocoded);
        let _ = writeln!(s, "    const TRUSTED: bool = {};", m.trusted);
        let _ = writeln!(s, "    fn encode_body(&self, w: &mut Writer) {{");
        if optional != 0 {
            // Omit a requested trailing suffix only while those blocks are
            // empty. A block that holds data is written, and so is everything
            // before it; an empty block is still written when the caller sets
            // omitted_trailing_blocks to zero.
            let _ = writeln!(
                s,
                "        let requested_omit = self.omitted_trailing_blocks.min({optional});"
            );
            let _ = writeln!(s, "        let mut omit_trailing = 0usize;");
            let mut indent = 8;
            for block in m.blocks.iter().rev().take(optional) {
                let pred = empty_block_expr(&modname, block);
                let _ = writeln!(
                    s,
                    "{:indent$}if requested_omit > omit_trailing && {pred} {{",
                    "",
                    indent = indent
                );
                indent += 4;
                let _ = writeln!(s, "{:indent$}omit_trailing += 1;", "", indent = indent);
            }
            for _ in 0..optional {
                indent -= 4;
                let _ = writeln!(s, "{:indent$}}}", "", indent = indent);
            }
        }
        for (index, b) in m.blocks.iter().enumerate() {
            let f = snake(&b.name);
            if index >= optional_start {
                let remaining = m.blocks.len() - index;
                let _ = writeln!(s, "        if omit_trailing >= {remaining} {{ return; }}");
            }
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "        self.{f}.encode(w);");
                }
                BlockKind::Multiple(n) => {
                    let _ = writeln!(
                        s,
                        "        for i in 0..{n} {{ match self.{f}.get(i) {{ Some(b) => b.encode(w), None => {modname}::{}::default().encode(w) }} }}",
                        b.name
                    );
                }
                BlockKind::Variable => {
                    let _ = writeln!(
                        s,
                        "        let n = self.{f}.len().min(255); w.u8(n as u8); for b in &self.{f}[..n] {{ b.encode(w); }}"
                    );
                }
            }
        }
        let _ = writeln!(s, "    }}");
        let _ = writeln!(
            s,
            "    fn decode_body(r: &mut Reader) -> Result<Self, DecodeError> {{"
        );
        // A short optional Single is not zero-filled. That block and every
        // later optional block count as omitted, matching libopenmetaverse
        // FromBytes ignoring bytes past its shorter template. The flag exists
        // only when a later block must see it; a last-block Single just
        // returns default, and a Variable-only suffix never sets it.
        let stops_later = m.blocks.iter().enumerate().any(|(index, b)| {
            index >= optional_start
                && index + 1 < m.blocks.len()
                && matches!(b.kind, BlockKind::Single)
        });
        if optional != 0 {
            let _ = writeln!(s, "        let mut omitted_trailing_blocks = 0usize;");
            if stops_later {
                let _ = writeln!(s, "        let mut stop_trailing = false;");
            }
        }
        for (index, b) in m.blocks.iter().enumerate() {
            let f = snake(&b.name);
            let _ = writeln!(s, "        let {f} = {{");
            if index >= optional_start {
                if stops_later {
                    let _ = writeln!(
                        s,
                        "            if stop_trailing || r.remaining() == 0 {{ omitted_trailing_blocks += 1; Default::default() }} else {{"
                    );
                } else {
                    let _ = writeln!(
                        s,
                        "            if r.remaining() == 0 {{ omitted_trailing_blocks += 1; Default::default() }} else {{"
                    );
                }
            }
            match b.kind {
                BlockKind::Single => {
                    if index >= optional_start {
                        let minimum: usize = b
                            .vars
                            .iter()
                            .map(|v| {
                                fixed_width(&v.ty).unwrap_or_else(|| {
                                    panic!(
                                        "unexpected optional Single layout {}.{}",
                                        m.name, b.name
                                    )
                                })
                            })
                            .sum();
                        if index + 1 < m.blocks.len() {
                            let _ = writeln!(
                                s,
                                "            if r.remaining() < {minimum} {{ stop_trailing = true; omitted_trailing_blocks += 1; Default::default() }} else {{ {modname}::{}::decode(r)? }}",
                                b.name
                            );
                        } else {
                            let _ = writeln!(
                                s,
                                "            if r.remaining() < {minimum} {{ omitted_trailing_blocks += 1; Default::default() }} else {{ {modname}::{}::decode(r)? }}",
                                b.name
                            );
                        }
                    } else {
                        let _ = writeln!(s, "            {modname}::{}::decode(r)?", b.name);
                    }
                }
                BlockKind::Multiple(n) => {
                    let _ = writeln!(
                        s,
                        "            {{ let mut v = Vec::with_capacity({n}); for _ in 0..{n} {{ v.push({modname}::{}::decode(r)?); }} v }}",
                        b.name
                    );
                }
                BlockKind::Variable => {
                    let _ = writeln!(
                        s,
                        "            {{ let n = r.block_count()?; let mut v = Vec::with_capacity(n); for _ in 0..n {{ v.push({modname}::{}::decode(r)?); }} v }}",
                        b.name
                    );
                }
            }
            if index >= optional_start {
                let _ = writeln!(s, "            }}");
            }
            let _ = writeln!(s, "        }};");
        }
        let _ = writeln!(s, "        Ok(Self {{");
        for b in &m.blocks {
            let _ = writeln!(s, "            {},", snake(&b.name));
        }
        if optional != 0 {
            let _ = writeln!(s, "            omitted_trailing_blocks,");
        }
        let _ = writeln!(s, "        }})");
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s, "impl {} {{", m.name);
        let _ = writeln!(s, "    pub fn scrub(&mut self) {{");
        for b in &m.blocks {
            let f = snake(&b.name);
            match b.kind {
                BlockKind::Single => {
                    let _ = writeln!(s, "        self.{f}.scrub();");
                }
                _ => {
                    let _ = writeln!(s, "        for b in &mut self.{f} {{ b.scrub(); }}");
                }
            }
        }
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "}}");
    }

    // id -> info table
    s.push_str("/// Look up static information about a message id.\n");
    s.push_str("pub fn message_info(id: MsgId) -> Option<MsgInfo> {\n    match id {\n");
    for m in msgs {
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        let _ = writeln!(
            s,
            "        {id} => Some(MsgInfo {{ name: \"{}\", zerocoded: {}, trusted: {}, deprecated: {} }}),",
            m.name, m.zerocoded, m.trusted, m.deprecated
        );
    }
    s.push_str("        _ => None,\n    }\n}\n");

    // Decode and encode one body. Used to prove a C# fixture round-trips
    // through the typed structs without naming all 478 messages in the test.
    s.push_str("pub fn reencode_body(id: MsgId, body: &[u8]) -> Result<Vec<u8>, DecodeError> {\n    match id {\n");
    let mut seen = std::collections::BTreeSet::new();
    for m in msgs {
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        if !seen.insert(id.clone()) {
            panic!("duplicate message id {id} ({})", m.name);
        }
        let _ = writeln!(
            s,
            "        {id} => {{ let m = {}::decode(body)?; let mut w = Writer::new(); m.encode_body(&mut w); Ok(w.buf) }},",
            m.name
        );
    }
    s.push_str("        _ => Err(DecodeError::Malformed(\"unknown message\")),\n    }\n}\n");

    // Same decode and encode, plus which body bytes are packed quaternions.
    // The client capture check allows only those bytes to differ, and only
    // by the packing tolerance. Server packets stay byte-exact.
    s.push_str("pub fn reencode_body_quat_marks(id: MsgId, body: &[u8]) -> Result<(Vec<u8>, Vec<bool>), DecodeError> {\n    match id {\n");
    let mut marked = std::collections::BTreeSet::new();
    for m in msgs {
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        if !marked.insert(id.clone()) {
            panic!("duplicate message id {id} ({})", m.name);
        }
        let _ = writeln!(
            s,
            "        {id} => {{ let m = {}::decode(body)?; let mut w = Writer::new(); w.record_quat_marks(); m.encode_body(&mut w); let marks = w.take_quat_marks(); Ok((w.buf, marks)) }},",
            m.name
        );
    }
    s.push_str("        _ => Err(DecodeError::Malformed(\"unknown message\")),\n    }\n}\n");

    s.push_str("pub fn scrub_body(id: MsgId, body: &[u8]) -> Result<Vec<u8>, DecodeError> {\n    match id {\n");
    let mut scrubbed = std::collections::BTreeSet::new();
    for m in msgs {
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        if !scrubbed.insert(id.clone()) {
            panic!("duplicate message id {id} ({})", m.name);
        }
        let _ = writeln!(
            s,
            "        {id} => {{ let mut m = {}::decode(body)?; m.scrub(); let mut w = Writer::new(); m.encode_body(&mut w); Ok(w.buf) }},",
            m.name
        );
    }
    s.push_str("        _ => Err(DecodeError::Malformed(\"unknown message\")),\n    }\n}\n");

    // Body offsets of AgentID and SessionID. None when the template does not
    // mark the message NeedValidateIDs.
    s.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    s.push_str("pub struct IdCheck {\n    pub agent: usize,\n    pub session: usize,\n}\n");
    s.push_str("pub fn id_check(id: MsgId) -> Option<IdCheck> {\n    match id {\n");
    let mut checked = 0usize;
    let mut seen_ids = std::collections::BTreeSet::new();
    for m in msgs {
        let Some((agent, session)) = id_offsets(m) else {
            continue;
        };
        let id = match m.freq.as_str() {
            "High" => format!("MsgId::High({})", m.number),
            "Medium" => format!("MsgId::Medium({})", m.number),
            _ => format!("MsgId::Low({})", m.number & 0xFFFF),
        };
        if !seen_ids.insert(id.clone()) {
            panic!("duplicate NeedValidateIDs message id {id} ({})", m.name);
        }
        let _ = writeln!(
            s,
            "        {id} => Some(IdCheck {{ agent: {agent}, session: {session} }}),"
        );
        checked += 1;
    }
    // libopenmetaverse generates 229 Packet classes with NeedValidateIDs.
    // The same AgentData rule also matches AvatarPickerRequestBackend,
    // RemoveAttachment and UpdateAttachment, which have no generated class.
    assert_eq!(
        checked, 232,
        "NeedValidateIDs messages in message_template.msg"
    );
    s.push_str("        _ => None,\n    }\n}\n");
    s
}

fn main() {
    let path = "message_template.msg";
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-changed=build.rs");
    let src = std::fs::read_to_string(path).expect("read template");
    let msgs = parse(&src);
    let code = generate(&msgs);
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("messages.rs");
    std::fs::write(out, code).expect("write generated code");
}
