// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run the substep accumulator oracle against the pinned checkout.
//! Build output goes under `.scratch/`, never into the checkout.
//!
//! `jolt generate` and `jolt verify` stay with the harness wrapper (P3.0).

use std::process::ExitCode;

pub(crate) fn substeps(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let fixture = root.join("fixtures/jolt/substeps.json");
    let action = if writing {
        "substeps-generate"
    } else {
        "substeps-verify"
    };
    crate::fixture_oracle::run("jolt", "JoltOracle.csproj", &fixture, action)
}
