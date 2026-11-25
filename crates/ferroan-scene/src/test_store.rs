// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A private writable copy of a committed SQLite fixture.
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) struct CopiedStore {
    dir: PathBuf,
}

impl CopiedStore {
    pub(crate) fn new(fixture: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ferroan-regionstore-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let store = Self { dir };
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(fixture);
        std::fs::copy(source, store.dir.join("store.db")).unwrap();
        store
    }

    pub(crate) fn connection(&self) -> String {
        format!(
            "Data Source={};Version=3",
            self.dir.join("store.db").display()
        )
    }
}

impl Drop for CopiedStore {
    fn drop(&mut self) {
        // A transient Windows sharing violation must not fail a store assertion.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
