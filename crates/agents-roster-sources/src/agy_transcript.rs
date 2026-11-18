// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Incremental agy transcript and glog quota parser.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use agents_roster_core::text::{base_name, summarize, truncate};
use agents_roster_core::time::parse_iso_ms;
use agents_roster_core::{DoneItem, Translator};
use serde_json::Value;

/// Incremental per-conversation state parsed from `transcript.jsonl`.
#[derive(Default)]
pub struct ConversationState {
    pub offset: u64,
    pub remainder: String,
    pub model: String,
    pub started_at_ms: u64,
    pub turn_start_ms: u64,
    pub last_at_ms: u64,
    pub context: u64,
    pub waiting_for_user: bool,
    pub now: String,
    pub now_since_ms: u64,
    pub next: String,
    pub done: Vec<DoneItem>,
    pub final_text: String,
    pub error: String,
}

fn json_str<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// Unwraps JSON-encoded string values found inside agy `tool_calls[].args`.
pub fn arg_str(v: &Value, k: &str) -> String {
    let raw = match v.get(k) {
        Some(Value::String(t)) => t.clone(),
        Some(other) => other.to_string(),
        None => return String::new(),
    };
    serde_json::from_str::<String>(&raw).unwrap_or(raw)
}

/// Formats an agy tool call into a short activity description.
pub fn describe_call(call: &Value, tr: &dyn Translator) -> String {
    let args = call.get("args").cloned().unwrap_or(Value::Null);
    let summary = arg_str(&args, "toolSummary");
    if !summary.is_empty() {
        return truncate(&summary, 90);
    }
    let text = match json_str(call, "name") {
        "run_command" => format!("$ {}", arg_str(&args, "CommandLine")),
        "read_file" | "view_file" => tr.format(
            "reading {file}",
            &[(
                "file",
                base_name(&arg_str(&args, "AbsolutePath").max(arg_str(&args, "Path"))),
            )],
        ),
        "write_file" | "write_to_file" => tr.format(
            "writing {file}",
            &[(
                "file",
                base_name(&arg_str(&args, "TargetFile").max(arg_str(&args, "Path"))),
            )],
        ),
        "replace_file_content" | "edit_file" => tr.format(
            "editing {file}",
            &[(
                "file",
                base_name(&arg_str(&args, "TargetFile").max(arg_str(&args, "Path"))),
            )],
        ),
        "schedule" => tr.translate("waiting for a background task").into_owned(),
        "manage_task" => tr.format(
            "background task: {action}",
            &[("action", &arg_str(&args, "Action"))],
        ),
        other => other.replace('_', " "),
    };
    truncate(&text, 90)
}

fn strip_user_request(raw: &str) -> String {
    raw.replace("<USER_REQUEST>", "")
        .replace("</USER_REQUEST>", "")
        .trim()
        .to_string()
}

fn push_done(c: &mut ConversationState, item: DoneItem) {
    if item.text.is_empty() || c.done.first().is_some_and(|d| d.text == item.text) {
        return;
    }
    c.done.insert(0, item);
    c.done.truncate(6);
}

fn apply_step(c: &mut ConversationState, v: &Value, tr: &dyn Translator) {
    let t = parse_iso_ms(json_str(v, "created_at")).unwrap_or(0);
    if c.started_at_ms == 0 {
        c.started_at_ms = t;
    }
    c.last_at_ms = c.last_at_ms.max(t);
    if !json_str(v, "error").is_empty() {
        c.error = truncate(json_str(v, "error"), 90);
    } else if json_str(v, "status") == "DONE" {
        c.error.clear();
    }
    match json_str(v, "type") {
        "USER_INPUT" => apply_user_input(c, v, t, tr),
        "PLANNER_RESPONSE" => apply_planner_response(c, v, t, tr),
        _ => {}
    }
}

fn apply_user_input(c: &mut ConversationState, v: &Value, t: u64, tr: &dyn Translator) {
    c.waiting_for_user = false;
    c.turn_start_ms = t;
    let clean = strip_user_request(json_str(v, "content"));
    let prompt = summarize(&clean, 80);
    c.now = if prompt.is_empty() {
        tr.translate("working").into_owned()
    } else {
        tr.format("request: {prompt}", &[("prompt", &prompt)])
    };
    c.now_since_ms = t;
    c.next.clear();
}

fn apply_planner_response(c: &mut ConversationState, v: &Value, t: u64, tr: &dyn Translator) {
    if !json_str(v, "model").is_empty() {
        c.model = json_str(v, "model").to_string();
    }
    let tokens = ["input_tokens", "cache_read_tokens"]
        .iter()
        .filter_map(|k| v.get(*k)?.as_u64())
        .sum::<u64>();
    if tokens > 0 {
        c.context = tokens;
    }
    let speech = summarize(json_str(v, "content"), 110);
    let calls = v
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(call) = calls.first() {
        c.waiting_for_user = false;
        c.now = describe_call(call, tr);
        c.now_since_ms = t;
        if !speech.is_empty() {
            c.next = speech;
        }
    } else if json_str(v, "status") == "DONE" && !speech.is_empty() {
        c.waiting_for_user = true;
        c.now = tr.translate("waiting for you").into_owned();
        c.now_since_ms = t;
        c.next.clear();
        c.final_text = speech.clone();
        let turn_ms = (c.turn_start_ms > 0).then(|| t.saturating_sub(c.turn_start_ms));
        push_done(
            c,
            DoneItem {
                text: tr.format("replied: {text}", &[("text", &speech)]),
                at_ms: t,
                is_reply: true,
                turn_duration_ms: turn_ms,
            },
        );
    }
}

/// Reads newly appended lines from `transcript_path` into `c`.
pub fn advance_transcript(c: &mut ConversationState, transcript_path: &Path, tr: &dyn Translator) {
    let Ok(mut f) = File::open(transcript_path) else {
        return;
    };
    let Ok(size) = f.metadata().map(|m| m.len()) else {
        return;
    };
    if size < c.offset {
        *c = ConversationState::default();
    }
    if size == c.offset || f.seek(SeekFrom::Start(c.offset)).is_err() {
        return;
    }
    let mut reader = BufReader::with_capacity(16 * 1024, f);
    let mut chunk = Vec::with_capacity(4096);
    loop {
        chunk.clear();
        let Ok(n) = reader.read_until(b'\n', &mut chunk) else {
            break;
        };
        if n == 0 {
            break;
        }
        c.offset += n as u64;
        if chunk.ends_with(b"\n") {
            let mut full = std::mem::take(&mut c.remainder);
            full.push_str(&String::from_utf8_lossy(&chunk));
            if let Ok(v) = serde_json::from_str::<Value>(full.trim_end_matches(['\r', '\n'])) {
                apply_step(c, &v, tr);
            }
        } else {
            c.remainder.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
}
