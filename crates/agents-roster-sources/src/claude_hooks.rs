// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Claude Code hook and statusline writers (read JSON from stdin, never block or fail).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};

use agents_roster_core::text::{summarize, truncate};
use agents_roster_core::{AgentState, Clock, DoneItem, IdentityTranslator, Translator};
use serde_json::Value;

use crate::claude_model::{
    Session, SubagentEntry, TaskItem, describe_tool, json_str, parse_tasks, valid_session_id,
};
use crate::procfs::find_ancestor;
use crate::sys::{SystemClock, is_effective_root};

pub(crate) fn read_stdin_json() -> Option<Value> {
    let mut s = String::new();
    std::io::stdin().take(8 << 20).read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

pub(crate) fn prepare_exchange_dir(dir: &Path) -> Option<PathBuf> {
    let sub = dir.join("claude");
    if sub.is_dir() {
        return Some(sub);
    }
    fs::create_dir_all(&sub).ok()?;
    if is_effective_root()
        && let Some(parent) = dir.parent().and_then(|p| fs::metadata(p).ok())
    {
        for p in [dir, sub.as_path()] {
            let _ = chown(p, Some(parent.uid()), Some(parent.gid()));
        }
    }
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    let _ = fs::set_permissions(&sub, fs::Permissions::from_mode(0o755));
    Some(sub)
}

pub(crate) fn with_session(dir: &Path, id: &str, f: impl FnOnce(&mut Session)) -> Option<Session> {
    if !valid_session_id(id) {
        return None;
    }
    let sub = prepare_exchange_dir(dir)?;
    let path = sub.join(format!("{id}.json"));
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(sub.join(format!(".{id}.lock")))
        .ok()?;
    let _ = lock.lock();
    let mut ss: Session = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let now = SystemClock.now_ms();
    if ss.id.is_empty() {
        (ss.id, ss.started_at_ms) = (id.to_string(), now);
    }
    f(&mut ss);
    ss.updated_at_ms = now;
    let tmp = sub.join(format!(".{id}.tmp"));
    fs::write(&tmp, serde_json::to_vec(&ss).ok()?).ok()?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
    fs::rename(&tmp, &path).ok()?;
    Some(ss)
}

fn last_assistant_text(transcript: &str) -> Option<String> {
    let mut f = File::open(transcript).ok()?;
    let start = f.metadata().ok()?.len().saturating_sub(256 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    for line in String::from_utf8_lossy(&buf).lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if json_str(&v, "type") != "assistant" {
            continue;
        }
        let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for b in blocks.iter().rev() {
            if json_str(b, "type") == "text" {
                let r = summarize(json_str(b, "text"), 110);
                if !r.is_empty() {
                    return Some(r);
                }
            }
        }
    }
    None
}

fn push_done(ss: &mut Session, text: String, at_ms: u64, is_reply: bool, turn_ms: Option<u64>) {
    if text.is_empty() || ss.done.first().is_some_and(|d| d.text == text) {
        return;
    }
    ss.done.insert(
        0,
        DoneItem {
            text,
            at_ms,
            is_reply,
            turn_duration_ms: turn_ms,
        },
    );
    ss.done.truncate(6);
}

fn apply_hook_event(ss: &mut Session, event: &str, v: &Value, now: u64) {
    let tr = IdentityTranslator;
    match event {
        "SessionStart" => {
            (ss.state, ss.now, ss.now_since_ms) =
                (AgentState::Waiting, "session opened".into(), now);
        }
        "UserPromptSubmit" => {
            (ss.state, ss.turn_start_ms, ss.now_since_ms) = (AgentState::Working, now, now);
            ss.reason.clear();
            ss.now = tr.format(
                "request: {prompt}",
                &[("prompt", &summarize(json_str(v, "prompt"), 80))],
            );
            ss.next.clear();
        }
        "PreToolUse" => apply_pre_tool_use(ss, v, now, &tr),
        "PostToolUse" => apply_post_tool_use(ss, v, now, &tr),
        "TaskCreated" | "TaskCompleted" => apply_task_event(ss, event, v, now, &tr),
        "PreCompact" => (ss.now, ss.now_since_ms) = ("compacting context".into(), now),
        "Notification" => {
            (ss.state, ss.reason) = (AgentState::Waiting, truncate(json_str(v, "message"), 90));
        }
        "Stop" => apply_stop(ss, now, &tr),
        "SubagentStart" | "SubagentStop" => apply_subagent_event(ss, event, v, now),
        "SessionEnd" => {
            (ss.state, ss.closed, ss.now, ss.now_since_ms) =
                (AgentState::Stopped, true, "session closed".into(), now);
            ss.subagents.clear();
            ss.reason.clear();
        }
        _ => {}
    }
}

fn apply_stop(ss: &mut Session, now: u64, tr: &dyn Translator) {
    ss.state = AgentState::Waiting;
    ss.reason.clear();
    ss.last_turn_ms = now.saturating_sub(ss.turn_start_ms);
    if let Some(f) = last_assistant_text(&ss.transcript) {
        push_done(
            ss,
            tr.format("replied: {text}", &[("text", &f)]),
            now,
            true,
            Some(ss.last_turn_ms),
        );
    }
    (ss.now, ss.now_since_ms) = ("waiting for you".into(), now);
    ss.next.clear();
}

fn apply_pre_tool_use(ss: &mut Session, v: &Value, now: u64, tr: &dyn Translator) {
    ss.state = AgentState::Working;
    ss.reason.clear();
    let input = v.get("tool_input").cloned().unwrap_or(Value::Null);
    (ss.now, ss.now_since_ms) = (describe_tool(json_str(v, "tool_name"), &input, tr), now);
    if let Some(t) = parse_tasks(&input) {
        ss.tasks = t;
    }
    if let Some(f) = last_assistant_text(&ss.transcript) {
        ss.next = f;
    }
}

fn apply_post_tool_use(ss: &mut Session, v: &Value, now: u64, tr: &dyn Translator) {
    let input = v.get("tool_input").cloned().unwrap_or(Value::Null);
    let Some(t) = parse_tasks(&input) else { return };
    for new_item in &t {
        let before = ss.tasks.iter().find(|a| a.text == new_item.text);
        if new_item.status == "completed" && before.is_none_or(|a| a.status != "completed") {
            push_done(
                ss,
                tr.format("task: {text}", &[("text", &new_item.text)]),
                now,
                false,
                None,
            );
        }
    }
    ss.tasks = t;
}

fn apply_task_event(ss: &mut Session, event: &str, v: &Value, now: u64, tr: &dyn Translator) {
    let text = ["task_subject", "subject", "title", "content", "description"]
        .iter()
        .map(|k| json_str(v, k))
        .find(|t| !t.is_empty())
        .unwrap_or("");
    if event == "TaskCompleted" {
        push_done(
            ss,
            tr.format("task: {text}", &[("text", &truncate(text, 90))]),
            now,
            false,
            None,
        );
    } else if !text.is_empty() {
        ss.tasks.push(TaskItem {
            text: truncate(text, 90),
            status: "pending".into(),
        });
    }
}

fn apply_subagent_event(ss: &mut Session, event: &str, v: &Value, now: u64) {
    let id = json_str(v, "agent_id");
    if id.is_empty() {
        return;
    }
    ss.subagents.retain(|s| s.id != id);
    if event == "SubagentStart" {
        let desc = ["prompt", "description"]
            .iter()
            .map(|k| json_str(v, k))
            .find(|s| !s.is_empty())
            .unwrap_or("");
        ss.subagents.push(SubagentEntry {
            id: id.to_string(),
            agent_type: json_str(v, "agent_type").to_string(),
            description: summarize(desc, 60),
            started_at_ms: now,
        });
    }
}

/// Processes one Claude Code hook event from stdin without printing or failing.
pub fn handle_hook(event: &str, dir: &Path) {
    let Some(v) = read_stdin_json() else { return };
    let id = json_str(&v, "session_id").to_string();
    let now = SystemClock.now_ms();
    let pid = find_ancestor("claude").unwrap_or(0);
    with_session(dir, &id, |ss| {
        if !json_str(&v, "cwd").is_empty() {
            ss.cwd = json_str(&v, "cwd").to_string();
        }
        if !json_str(&v, "transcript_path").is_empty() {
            ss.transcript = json_str(&v, "transcript_path").to_string();
        }
        if pid != 0 {
            ss.claude_pid = pid;
        }
        ss.closed = false;
        apply_hook_event(ss, event, &v, now);
    });
}
