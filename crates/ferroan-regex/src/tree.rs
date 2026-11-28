// Copyright (c) .NET Foundation and Contributors
//
// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// Ported to Rust for Ferroan from
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexNode.cs
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexParser.cs
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Concatenation, alternation, quantifiers and capturing groups.
//!
//! The shape follows `RegexParser.ScanRegex` through `AddToConcatenate`,
//! `AddAlternate`, `AddGroup` and `RegexNode.MakeQuantifier` before `Reduce`
//! merges character classes and before `FinalOptimize`. A capturing group's
//! number is the declared slot `Groups[number]` looks up. `osMatchString`
//! walks `Match.Groups` by index, and a number that is not itself one of
//! those indexes is never read. A literal, a class, an anchor, an escape or
//! a backreference is one `Unit`. Lookaround and an atomic group stay a
//! `Hold` so a quantifier wraps them; their match behaviour is a later row.

use std::fmt;

/// One node of a parsed pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A concatenation or quantifier that matches nothing.
    Empty,
    /// One atom. The class set and the backreference target are a later row.
    Unit,
    /// Pieces in order. Two or more, after empty pieces are dropped.
    Concat(Vec<Node>),
    /// Branches in order. Two or more, including an empty branch.
    Alternate(Vec<Node>),
    /// `*`, `+`, `?` or `{n,m}`, greedy unless `lazy` is set.
    Quant {
        min: i32,
        max: i32,
        lazy: bool,
        child: Box<Node>,
    },
    /// A capturing group. `number` is the declared slot.
    Capture { number: i32, child: Box<Node> },
    /// Lookaround, an atomic group, or a conditional. The inside is kept.
    Hold(Box<Node>),
}

impl Node {
    /// Nodes on the deepest path, counting this node.
    ///
    /// [`crate::parse`] refuses a tree whose depth would pass
    /// [`crate::TREE_DEPTH_LIMIT`], so a tree it returns stays within the
    /// bound this walk uses.
    #[must_use]
    pub fn depth(&self) -> u32 {
        match self {
            Node::Empty | Node::Unit => 1,
            Node::Concat(children) | Node::Alternate(children) => {
                let mut deepest = 0;
                for child in children {
                    deepest = deepest.max(child.depth());
                }
                deepest.saturating_add(1)
            }
            Node::Quant { child, .. } | Node::Capture { child, .. } | Node::Hold(child) => {
                child.depth().saturating_add(1)
            }
        }
    }

    /// One line, with no spaces, that compares trees in the group fixture.
    #[must_use]
    pub fn structure(&self) -> String {
        let mut out = String::new();
        self.write_structure(&mut out);
        out
    }

    fn write_structure(&self, out: &mut String) {
        match self {
            Node::Empty => out.push_str("empty"),
            Node::Unit => out.push_str("unit"),
            Node::Concat(children) => write_list(out, "concat", children),
            Node::Alternate(children) => write_list(out, "alt", children),
            Node::Quant {
                min,
                max,
                lazy,
                child,
            } => {
                out.push_str("quant(");
                out.push_str(&min.to_string());
                out.push(',');
                out.push_str(&max.to_string());
                out.push(',');
                out.push_str(if *lazy { "lazy" } else { "greedy" });
                out.push(',');
                child.write_structure(out);
                out.push(')');
            }
            Node::Capture { number, child } => {
                out.push_str("cap(");
                out.push_str(&number.to_string());
                out.push(',');
                child.write_structure(out);
                out.push(')');
            }
            Node::Hold(child) => {
                out.push_str("hold(");
                child.write_structure(out);
                out.push(')');
            }
        }
    }
}

fn write_list(out: &mut String, name: &str, children: &[Node]) {
    out.push_str(name);
    out.push('(');
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        child.write_structure(out);
    }
    out.push(')');
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.structure())
    }
}
