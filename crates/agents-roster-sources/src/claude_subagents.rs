// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Discovers active Claude Code built-in subagents and unhooked terminal `claude` processes.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use agents_roster_core::text::{base_name, format_model_name, summarize, truncate};
use agents_roster_core::time::parse_iso_ms;
use agents_roster_core::{Agent, AgentState, ProcessTable, Translator};
use serde::Deserialize;
use serde_json::Value;

use crate::claude_model::{Session, describe_tool, json_str};

const TAIL_BYTES: i64 = 24 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentMeta {
    #[serde(default)]
    agent_type: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    picked_effort_model: String,
    #[serde(default)]
    finished: bool,
}

#[derive(Default)]
struct SubTail {
    now: String,
    at_ms: u64,
    tokens: u64,
}

fn read_subagent_tail(jsonl_path: &Path, tr: &dyn Translator) -> SubTail {
    let mut out = SubTail::default();
    let Ok(mut f) = File::open(jsonl_path) else {
        return out;
    };
    if let Ok(len) = f.metadata().map(|m| m.len())
        && len > TAIL_BYTES as u64
    {
        let _ = f.seek(SeekFrom::End(-TAIL_BYTES));
    }
    let mut raw = Vec::new();
    if f.read_to_end(&mut raw).is_err() {
        return out;
    }
    for line in String::from_utf8_lossy(&raw).lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        accumulate_tail_line(&v, tr, &mut out);
        if !out.now.is_empty() && out.at_ms > 0 {
            break;
        }
    }
    out
}

fn accumulate_tail_line(v: &Value, tr: &dyn Translator, out: &mut SubTail) {
    if out.at_ms == 0 {
        out.at_ms = parse_iso_ms(json_str(v, "timestamp")).unwrap_or(0);
    }
    if out.tokens == 0
        && let Some(u) = v.pointer("/message/usage")
    {
        let i = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let cr = u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cc = u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        out.tokens = i + cr + cc;
    }
    if out.now.is_empty() {
        out.now = extract_last_activity(v, tr);
    }
}

fn extract_last_activity(v: &Value, tr: &dyn Translator) -> String {
    let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
        return String::new();
    };
    for b in blocks.iter().rev() {
        match json_str(b, "type") {
            "tool_use" => {
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                return describe_tool(json_str(b, "name"), &input, tr);
            }
            "text" => {
                let s = summarize(json_str(b, "text"), 80);
                if !s.is_empty() {
                    return s;
                }
            }
            _ => {}
        }
    }
    String::new()
}

fn subagent_title(agent_type: &str, desc: &str, fallback_id: &str) -> String {
    let label = match (agent_type.is_empty(), desc.is_empty()) {
        (false, false) => format!("{agent_type}: {desc}"),
        (true, false) => desc.to_string(),
        (false, true) => agent_type.to_string(),
        (true, true) => fallback_id.to_string(),
    };
    truncate(&label, 44)
}

/// Collects active subagents from `<session>/subagents/*.meta.json` and hook state.
pub(crate) fn collect_session_subagents(
    ss: &Session,
    dir: &str,
    now: u64,
    tr: &dyn Translator,
) -> Vec<Agent> {
    let mut out = Vec::new();
    let mut seen_ids = HashSet::new();
    if !ss.transcript.is_empty() {
        let sub_dir = Path::new(&ss.transcript)
            .with_extension("")
            .join("subagents");
        collect_meta_dir(ss, &sub_dir, dir, now, tr, &mut seen_ids, &mut out);
    }
    for sub in &ss.subagents {
        if seen_ids.contains(&sub.id) {
            continue;
        }
        out.push(Agent {
            key: format!("claude:sub:{}", sub.id),
            parent_key: Some(format!("claude:{}", ss.id)),
            source_id: "claude".into(),
            source_label: "Claude".into(),
            pid: (ss.claude_pid != 0).then_some(ss.claude_pid),
            working_dir: dir.to_string(),
            project: subagent_title(&sub.agent_type, &sub.description, &sub.id),
            model: String::new(),
            state: AgentState::Working,
            reason: String::new(),
            now: tr.translate("subagent running").into_owned(),
            now_since_ms: sub.started_at_ms.max(1),
            next: String::new(),
            done: Vec::new(),
            context_tokens: 0,
            context_limit: 0,
            context_pct: None,
            updated_at_ms: sub.started_at_ms.max(now),
        });
    }
    out
}

fn collect_meta_dir(
    ss: &Session,
    sub_dir: &Path,
    dir: &str,
    now: u64,
    tr: &dyn Translator,
    seen: &mut HashSet<String>,
    out: &mut Vec<Agent>,
) {
    let Ok(rd) = fs::read_dir(sub_dir) else {
        return;
    };
    for entry in rd.flatten() {
        if let Some(agent) = build_meta_agent(ss, sub_dir, &entry.path(), dir, now, tr, seen) {
            out.push(agent);
        }
    }
}

fn build_meta_agent(
    ss: &Session,
    sub_dir: &Path,
    path: &Path,
    dir: &str,
    now: u64,
    tr: &dyn Translator,
    seen: &mut HashSet<String>,
) -> Option<Agent> {
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())?
        .strip_suffix(".meta.json")?;
    let meta: SubagentMeta = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if meta.finished {
        return None;
    }
    let id = stem.strip_prefix("agent-").unwrap_or(stem);
    seen.insert(id.to_string());
    seen.insert(stem.to_string());
    let tail = read_subagent_tail(&sub_dir.join(format!("{stem}.jsonl")), tr);
    let raw_model = if meta.picked_effort_model.is_empty() {
        &meta.model
    } else {
        &meta.picked_effort_model
    };
    Some(Agent {
        key: format!("claude:sub:{stem}"),
        parent_key: Some(format!("claude:{}", ss.id)),
        source_id: "claude".into(),
        source_label: "Claude".into(),
        pid: (ss.claude_pid != 0).then_some(ss.claude_pid),
        working_dir: dir.to_string(),
        project: subagent_title(&meta.agent_type, &meta.description, id),
        model: format_model_name(raw_model),
        state: AgentState::Working,
        reason: String::new(),
        now: if tail.now.is_empty() {
            tr.translate("subagent running").into_owned()
        } else {
            tail.now
        },
        now_since_ms: if tail.at_ms > 0 { tail.at_ms } else { now },
        next: String::new(),
        done: Vec::new(),
        context_tokens: tail.tokens,
        context_limit: ss.context_limit,
        context_pct: (ss.context_limit > 0 && tail.tokens > 0)
            .then(|| tail.tokens as f64 * 100.0 / ss.context_limit as f64),
        updated_at_ms: tail.at_ms.max(ss.updated_at_ms),
    })
}

/// Discovers running `claude` processes that have no active hook session file.
pub(crate) fn discover_unhooked_claude(
    procs: &dyn ProcessTable,
    hooked_pids: &HashSet<u32>,
    now: u64,
    tr: &dyn Translator,
) -> Vec<Agent> {
    let mut out = Vec::new();
    for proc in procs.processes_by_comm("claude") {
        if hooked_pids.contains(&proc.pid) {
            continue;
        }
        let cwd = proc
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let proj = if cwd.is_empty() {
            format!("pid {}", proc.pid)
        } else {
            base_name(&cwd).to_string()
        };
        out.push(Agent {
            key: format!("claude:pid:{}", proc.pid),
            parent_key: None,
            source_id: "claude".into(),
            source_label: "Claude Code".into(),
            pid: Some(proc.pid),
            working_dir: cwd,
            project: proj,
            model: String::new(),
            state: AgentState::Working,
            reason: String::new(),
            now: tr.translate("running in terminal").into_owned(),
            now_since_ms: now,
            next: String::new(),
            done: Vec::new(),
            context_tokens: 0,
            context_limit: 0,
            context_pct: None,
            updated_at_ms: now,
        });
    }
    out
}
