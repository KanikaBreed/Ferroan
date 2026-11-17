// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared types for Ferroan.
//!
//! This crate is a placeholder. UUIDs, vectors, rotations, prim shapes and
//! permission masks land here in Phase 0 (task F01 in `TASKS.md`).

/// The OpenSim Tranquillity `develop` commit that Ferroan's ports and
/// compatibility checks currently follow. Ported files record their own
/// revision; this is the starting point.
pub const REFERENCE_TRANQUILLITY_REVISION: &str = "713857f10b8e91a639a967d4354a4682b080793c";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_revision_is_a_full_commit_hash() {
        assert_eq!(REFERENCE_TRANQUILLITY_REVISION.len(), 40);
        assert!(
            REFERENCE_TRANQUILLITY_REVISION
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}
