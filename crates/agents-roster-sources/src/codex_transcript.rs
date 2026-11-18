// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Incremental `OpenAI` Codex CLI rollout JSONL parser (`~/.codex/sessions/.../rollout-*.jsonl`).

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use agents_roster_core::text::{format_model_name, summarize, truncate};
use agents_roster_core::time::parse_iso_ms;
use agents_roster_core::{DoneItem, Translator};
use serde_json::Value;

/// Incremental state parsed from one Codex CLI rollout `.jsonl` file.
#[derive(Default)]
pub struct CodexConvState {
    pub offset: u64,
    pub remainder: String,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub subagent_role: String,
    pub cwd: String,
    pub model: String,
    pub started_at_ms: u64,
    pub last_at_ms: u64,
    pub now: String,
    pub now_since_ms: u64,
    pub next: String,
    pub done: Vec<DoneItem>,
    pub context_tokens: u64,
    pub context_limit: u64,
    pub waiting: bool,
    pub exited: bool,
}

fn jstr<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn push_done(c: &mut CodexConvState, text: String, at_ms: u64, is_reply: bool) {
    if text.is_empty() || c.done.first().is_some_and(|d| d.text == text) {
        return;
    }
    c.done.insert(
        0,
        DoneItem {
            text,
            at_ms,
            is_reply,
            turn_duration_ms: None,
        },
    );
    c.done.truncate(6);
}

fn extract_parent_source(payload: &Value) -> (Option<String>, String) {
    let direct = jstr(payload, "parent_thread_id");
    if !direct.is_empty() {
        return (Some(direct.to_string()), String::new());
    }
    let Some(src) = payload.get("source") else {
        return (None, String::new());
    };
    let sub = src.get("subagent").unwrap_or(src);
    let spawn = sub.get("thread_spawn").unwrap_or(sub);
    let parent = jstr(spawn, "parent_thread_id");
    let role = [jstr(spawn, "agent_role"), jstr(spawn, "agent_nickname")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("");
    if !parent.is_empty() {
        return (Some(parent.to_string()), role.to_string());
    }
    (None, role.to_string())
}

fn describe_codex_call(name: &str, args_raw: &str, tr: &dyn Translator) -> String {
    let parsed: Value = serde_json::from_str(args_raw).unwrap_or(Value::Null);
    match name {
        "exec_command" | "shell" | "container.exec" => {
            let cmd = jstr(&parsed, "cmd").if_empty_then(|| jstr(&parsed, "command"));
            tr.format("running: {cmd}", &[("cmd", &truncate(cmd, 60))])
        }
        "apply_patch" => tr.translate("editing file").into_owned(),
        "spawn_agent" => {
            let role = jstr(&parsed, "agent").if_empty_then(|| jstr(&parsed, "agent_type"));
            tr.format("subagent: {desc}", &[("desc", &truncate(role, 50))])
        }
        "" => tr.translate("working").into_owned(),
        other => format!("{}...", truncate(other, 30)),
    }
}

trait StrFallback {
    fn if_empty_then<'a>(&'a self, alt: impl FnOnce() -> &'a str) -> &'a str;
}

impl StrFallback for str {
    fn if_empty_then<'a>(&'a self, alt: impl FnOnce() -> &'a str) -> &'a str {
        if self.is_empty() { alt() } else { self }
    }
}

fn handle_session_meta(c: &mut CodexConvState, payload: &Value, ts: u64) {
    let id = jstr(payload, "id");
    if !id.is_empty() {
        c.session_id = id.to_string();
    }
    let cwd = jstr(payload, "cwd");
    if !cwd.is_empty() {
        c.cwd = cwd.to_string();
    }
    let model = jstr(payload, "model");
    if !model.is_empty() {
        c.model = format_model_name(model);
    }
    let (parent, role) = extract_parent_source(payload);
    if parent.is_some() {
        c.parent_session_id = parent;
    }
    if !role.is_empty() {
        c.subagent_role = role;
    }
    if c.started_at_ms == 0 && ts > 0 {
        c.started_at_ms = ts;
    }
}

fn handle_response_item(c: &mut CodexConvState, payload: &Value, ts: u64, tr: &dyn Translator) {
    match jstr(payload, "type") {
        "function_call" | "custom_tool_call" => {
            c.waiting = false;
            c.now = describe_codex_call(jstr(payload, "name"), jstr(payload, "arguments"), tr);
            c.now_since_ms = ts;
        }
        "message" if jstr(payload, "role") == "assistant" => {
            let text = payload
                .get("content")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    arr.iter()
                        .rev()
                        .find_map(|b| Some(jstr(b, "text")).filter(|s| !s.is_empty()))
                })
                .unwrap_or("");
            let s = summarize(text, 90);
            if !s.is_empty() {
                push_done(c, tr.format("replied: {text}", &[("text", &s)]), ts, true);
            }
            c.waiting = true;
            c.now = tr.translate("waiting for you").into_owned();
            c.now_since_ms = ts;
        }
        _ => {}
    }
}

fn handle_event_msg(c: &mut CodexConvState, payload: &Value, ts: u64, tr: &dyn Translator) {
    match jstr(payload, "type") {
        "user_message" => {
            c.waiting = false;
            let msg = summarize(jstr(payload, "message"), 75);
            c.now = tr.format("request: {prompt}", &[("prompt", &msg)]);
            c.now_since_ms = ts;
        }
        "agent_message" => {
            let msg = summarize(jstr(payload, "message"), 90);
            if !msg.is_empty() {
                push_done(c, tr.format("replied: {text}", &[("text", &msg)]), ts, true);
            }
        }
        "task_complete" | "turn_aborted" => {
            c.waiting = true;
            c.now = tr.translate("waiting for you").into_owned();
            c.now_since_ms = ts;
        }
        "shutdown_complete" | "session_ended" => c.exited = true,
        "token_count" => {
            if let Some(info) = payload.get("info") {
                if let Some(t) = info
                    .pointer("/total_token_usage/total_tokens")
                    .and_then(Value::as_u64)
                {
                    c.context_tokens = t;
                }
                if let Some(w) = info.get("model_context_window").and_then(Value::as_u64) {
                    c.context_limit = w;
                }
            }
        }
        _ => {}
    }
}

fn apply_line(c: &mut CodexConvState, line: &str, tr: &dyn Translator) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let ts = parse_iso_ms(jstr(&v, "timestamp")).unwrap_or(0);
    if ts > 0 {
        c.last_at_ms = c.last_at_ms.max(ts);
    }
    let payload = v.get("payload").unwrap_or(&Value::Null);
    match jstr(&v, "type") {
        "session_meta" => handle_session_meta(c, payload, ts),
        "turn_context" => {
            let m = jstr(payload, "model");
            if !m.is_empty() {
                c.model = format_model_name(m);
            }
        }
        "response_item" => handle_response_item(c, payload, ts, tr),
        "event_msg" => handle_event_msg(c, payload, ts, tr),
        _ => {}
    }
}

/// Incrementally parses newly appended lines from a Codex rollout JSONL file.
pub fn advance_codex_rollout(c: &mut CodexConvState, path: &Path, tr: &dyn Translator) {
    let Ok(mut f) = File::open(path) else { return };
    let Ok(size) = f.metadata().map(|m| m.len()) else {
        return;
    };
    if size < c.offset {
        *c = CodexConvState::default();
    }
    if size == c.offset || f.seek(SeekFrom::Start(c.offset)).is_err() {
        return;
    }
    let mut reader = BufReader::with_capacity(16 * 1024, f);
    let mut line_buf = Vec::new();
    let mut bytes_read = 0u64;
    while let Ok(n) = reader.read_until(b'\n', &mut line_buf) {
        if n == 0 {
            break;
        }
        bytes_read += n as u64;
        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
            let text = String::from_utf8_lossy(&line_buf);
            let combined = if c.remainder.is_empty() {
                text.into_owned()
            } else {
                std::mem::take(&mut c.remainder) + text.as_ref()
            };
            apply_line(c, &combined, tr);
        } else {
            c.remainder.push_str(&String::from_utf8_lossy(&line_buf));
        }
        line_buf.clear();
    }
    c.offset += bytes_read;
}

/// Finds recent rollout `.jsonl` files under `sessions_dir`.
pub fn recent_rollout_files(
    sessions_dir: &Path,
    now_ms: u64,
    lookback_ms: u64,
) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    collect_recent_jsonl(sessions_dir, now_ms, lookback_ms, 0, &mut out);
    out
}

fn collect_recent_jsonl(
    dir: &Path,
    now_ms: u64,
    lookback_ms: u64,
    depth: u8,
    out: &mut HashSet<PathBuf>,
) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        if meta.is_dir() && depth < 3 {
            collect_recent_jsonl(&p, now_ms, lookback_ms, depth + 1, out);
        } else if meta.is_file()
            && now_ms.saturating_sub(mtime) <= lookback_ms
            && p.extension().and_then(|e| e.to_str()) == Some("jsonl")
        {
            out.insert(p);
        }
    }
}
