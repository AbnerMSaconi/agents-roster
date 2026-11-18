// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! On-disk JSON models and tool description helpers for Claude Code hook state.

use agents_roster_core::text::{base_name, truncate};
use agents_roster_core::time::parse_iso_ms;
use agents_roster_core::{AgentState, DoneItem, Translator, UsageWindow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Task item synced from Claude Code `TodoWrite` / `TaskCreated` hooks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskItem {
    pub text: String,
    pub status: String,
}

/// Active built-in subagent recorded via `SubagentStart` / `SubagentStop` hooks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentEntry {
    pub id: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub started_at_ms: u64,
}

/// On-disk per-session state shared between Claude Code hooks and the panel reader.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub project_dir: String,
    pub model: String,
    pub transcript: String,
    pub claude_pid: u32,
    pub state: AgentState,
    pub reason: String,
    pub now: String,
    pub now_since_ms: u64,
    pub next: String,
    pub turn_start_ms: u64,
    pub last_turn_ms: u64,
    pub done: Vec<DoneItem>,
    pub tasks: Vec<TaskItem>,
    pub context_tokens: u64,
    pub context_limit: u64,
    pub context_pct: Option<f64>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub closed: bool,
    #[serde(default)]
    pub subagents: Vec<SubagentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct StoredLimits {
    pub five_hour: Option<StoredWindow>,
    pub seven_day: Option<StoredWindow>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StoredWindow {
    pub pct: f64,
    pub resets_at_ms: Option<u64>,
}

pub(crate) fn json_str<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

pub(crate) fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Formats a Claude Code tool invocation for display.
pub fn describe_tool(tool: &str, input: &Value, tr: &dyn Translator) -> String {
    let desc = json_str(input, "description");
    let text = match tool {
        "Bash" if !desc.is_empty() => desc.to_string(),
        "Bash" => format!("$ {}", json_str(input, "command")),
        "Read" => tr.format(
            "reading {file}",
            &[("file", base_name(json_str(input, "file_path")))],
        ),
        "Edit" | "MultiEdit" => tr.format(
            "editing {file}",
            &[("file", base_name(json_str(input, "file_path")))],
        ),
        "Write" => tr.format(
            "writing {file}",
            &[("file", base_name(json_str(input, "file_path")))],
        ),
        "NotebookEdit" => tr.format(
            "editing {file}",
            &[("file", base_name(json_str(input, "notebook_path")))],
        ),
        "Grep" => tr.format(
            "searching \"{pattern}\"",
            &[("pattern", json_str(input, "pattern"))],
        ),
        "Glob" => tr.format(
            "finding {pattern}",
            &[("pattern", json_str(input, "pattern"))],
        ),
        "WebSearch" => tr.format(
            "web search \"{query}\"",
            &[("query", json_str(input, "query"))],
        ),
        "WebFetch" => tr.format("fetching {url}", &[("url", json_str(input, "url"))]),
        "Agent" | "Task" => tr.format(
            "subagent: {desc}",
            &[("desc", json_str(input, "description"))],
        ),
        "TodoWrite" => tr.translate("updating task list").into_owned(),
        "Skill" => tr.format("skill {name}", &[("name", json_str(input, "skill"))]),
        "AskUserQuestion" => tr.translate("asking you").into_owned(),
        "Artifact" => tr.translate("publishing artifact").into_owned(),
        other => other.trim_start_matches("mcp__").replace("__", " · "),
    };
    truncate(&text, 90)
}

pub(crate) fn parse_tasks(input: &Value) -> Option<Vec<TaskItem>> {
    let list = input.get("todos")?.as_array()?;
    Some(
        list.iter()
            .map(|t| {
                let raw = if json_str(t, "content").is_empty() {
                    json_str(t, "subject")
                } else {
                    json_str(t, "content")
                };
                TaskItem {
                    text: truncate(raw, 90),
                    status: json_str(t, "status").to_string(),
                }
            })
            .collect(),
    )
}

pub(crate) fn parse_window(
    v: Option<&Value>,
    name: &'static str,
    long_key: &'static str,
) -> Option<UsageWindow> {
    let v = v?;
    let pct = v
        .get("used_percentage")
        .or_else(|| v.get("utilization"))
        .and_then(Value::as_f64)?;
    let resets_at_ms = match v.get("resets_at") {
        Some(Value::Number(n)) => n.as_f64().map(|x| {
            if x > 1e12 {
                x as u64
            } else {
                (x * 1000.0) as u64
            }
        }),
        Some(Value::String(t)) => parse_iso_ms(t),
        _ => None,
    };
    Some(UsageWindow {
        name: name.into(),
        long_name_key: long_key.into(),
        pct,
        resets_at_ms,
    })
}
