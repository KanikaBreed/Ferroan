// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TerrainError {
    #[error("terrain edge is outside 16..=4096 or is not a multiple of 16")]
    Edge,
    #[error("terrain blob exceeds the byte cap")]
    Blob,
    #[error("terrain revision {0} is not Variable2D (22) or Variable2DGzip (23)")]
    Revision(i32),
    #[error("terrain blob is truncated or not gzip")]
    Damaged,
    #[error("terrain store: {0}")]
    Store(String),
    #[error("terrain frame stopped")]
    Stopped,
    #[error("terrain frame thread did not start")]
    Thread,
}
