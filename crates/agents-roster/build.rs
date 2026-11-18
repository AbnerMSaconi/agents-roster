// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use std::env;

/// Short commit id for `--version`: `AGENTS_ROSTER_COMMIT` (set by CI) or `git rev-parse`, else `unknown`.
fn commit_id() -> String {
    if let Ok(v) = env::var("AGENTS_ROSTER_COMMIT")
        && !v.trim().is_empty()
    {
        return v.trim().chars().take(9).collect();
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_owned())
}

fn main() {
    println!("cargo:rustc-env=AGENTS_ROSTER_COMMIT={}", commit_id());
    println!("cargo:rerun-if-env-changed=AGENTS_ROSTER_COMMIT");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
