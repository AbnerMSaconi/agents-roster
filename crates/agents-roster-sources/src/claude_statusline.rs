// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Claude Code statusline handler: updates session context and account rate limits.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agents_roster_core::{Clock, UsageWindow};
use serde_json::Value;

use crate::claude_hooks::{prepare_exchange_dir, read_stdin_json, with_session};
use crate::claude_model::{StoredLimits, StoredWindow, json_str, parse_window};
use crate::sys::SystemClock;

/// Processes a Claude Code statusline payload and returns the formatted status line string.
pub fn handle_statusline(dir: &Path) -> String {
    let Some(v) = read_stdin_json() else {
        return "agents-roster: no data".into();
    };
    let id = json_str(&v, "session_id").to_string();
    let cw = v.get("context_window");
    let pct = cw
        .and_then(|c| c.get("used_percentage"))
        .and_then(Value::as_f64);
    let limit = cw
        .and_then(|c| c.get("context_window_size"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let tokens = cw
        .and_then(|c| c.get("total_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let model = v
        .pointer("/model/display_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let project = v
        .pointer("/workspace/project_dir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    with_session(dir, &id, |ss| {
        (ss.context_pct, ss.context_limit, ss.context_tokens) = (pct, limit, tokens);
        if !model.is_empty() {
            ss.model = model;
        }
        if !project.is_empty() {
            ss.project_dir = project;
        }
        if !json_str(&v, "transcript_path").is_empty() {
            ss.transcript = json_str(&v, "transcript_path").to_string();
        }
        if !json_str(&v, "cwd").is_empty() {
            ss.cwd = json_str(&v, "cwd").to_string();
        }
    });
    let rl = v.get("rate_limits");
    let w5 = parse_window(rl.and_then(|r| r.get("five_hour")), "5h", "5 hours");
    let w7 = parse_window(rl.and_then(|r| r.get("seven_day")), "7d", "7 days");
    write_limits(dir, w5.as_ref(), w7.as_ref());
    let ctx = pct.map_or_else(|| "—".into(), |p| format!("{p:.0}%"));
    let p5 = w5.map_or_else(|| "—".into(), |w| format!("{:.0}%", w.pct));
    let p7 = w7.map_or_else(|| "—".into(), |w| format!("{:.0}%", w.pct));
    format!("ctx {ctx} · 5h {p5} · 7d {p7}")
}

fn write_limits(dir: &Path, w5: Option<&UsageWindow>, w7: Option<&UsageWindow>) {
    if (w5.is_none() && w7.is_none()) || prepare_exchange_dir(dir).is_none() {
        return;
    }
    let sub = dir.join("claude");
    let stored = StoredLimits {
        five_hour: w5.map(|w| StoredWindow {
            pct: w.pct,
            resets_at_ms: w.resets_at_ms,
        }),
        seven_day: w7.map(|w| StoredWindow {
            pct: w.pct,
            resets_at_ms: w.resets_at_ms,
        }),
        updated_at_ms: SystemClock.now_ms(),
    };
    let tmp = sub.join(".limites.tmp");
    if let Ok(b) = serde_json::to_vec(&stored)
        && fs::write(&tmp, b).is_ok()
    {
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
        let _ = fs::rename(&tmp, sub.join("limites.json"));
    }
}
