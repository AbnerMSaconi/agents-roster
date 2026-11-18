// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Domain types representing live agents, usage limits, and notifications.

use serde::{Deserialize, Serialize};

/// Lifecycle state of an active agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AgentState {
    /// Actively executing tools or generating output.
    Working,
    /// Finished a turn or waiting for user input/permission.
    Waiting,
    /// Process is alive, but no activity occurred for an extended period.
    #[default]
    Idle,
    /// Process exited cleanly.
    Stopped,
    /// Process stopped due to an error or exhausted quota.
    Error,
}

impl AgentState {
    /// Sort priority in the panel (active work first).
    pub fn sort_rank(self) -> u8 {
        match self {
            Self::Working => 0,
            Self::Waiting => 1,
            Self::Error => 2,
            Self::Idle => 3,
            Self::Stopped => 4,
        }
    }

    /// English key used for translation lookups.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting for you",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Error => "stopped with error",
        }
    }
}

/// A completed milestone (reply, task, or git commit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneItem {
    /// Human-readable description of what finished.
    pub text: String,
    /// Completion timestamp in Unix milliseconds.
    pub at_ms: u64,
    /// Whether this item represents an assistant text reply.
    #[serde(default)]
    pub is_reply: bool,
    /// Duration of the turn that produced this reply, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_duration_ms: Option<u64>,
}

/// Snapshot of one agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Agent {
    /// Unique identifier such as `claude:<session>` or `agy:<conversation>`.
    pub key: String,
    /// Parent agent key when this session was spawned as a subagent or terminal child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<String>,
    /// Stable source identifier (e.g. `"claude"`, `"agy"`, `"codex"`).
    pub source_id: String,
    /// Display name of the agent source (e.g. `"Claude Code"`, `"agy"`).
    pub source_label: String,
    /// Live OS process ID of the agent, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Full working directory path of the agent, if known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub working_dir: String,
    /// Short project or worktree name.
    pub project: String,
    /// Normalized model name (e.g. `"Opus 5.5"`).
    pub model: String,
    /// Current lifecycle state.
    pub state: AgentState,
    /// Optional reason (e.g. permission prompt or quota reset).
    pub reason: String,
    /// Current activity description.
    pub now: String,
    /// Timestamp when `now` started, in Unix milliseconds.
    pub now_since_ms: u64,
    /// Stated next step or active task.
    pub next: String,
    /// Completed milestones, most recent first.
    pub done: Vec<DoneItem>,
    /// Tokens currently in context (`0` if unknown).
    pub context_tokens: u64,
    /// Configured or reported context window size (`0` if unknown).
    pub context_limit: u64,
    /// Context usage percentage if known.
    pub context_pct: Option<f64>,
    /// Last observed activity timestamp in Unix milliseconds.
    pub updated_at_ms: u64,
}

impl Agent {
    /// Display title combining the source label and project name.
    pub fn title(&self) -> String {
        format!("{} · {}", self.source_label, self.project)
    }
}

/// One usage rate-limit window (such as 5-hour or 7-day).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// Short window identifier (e.g. `"5h"`, `"7d"`).
    pub name: String,
    /// Long label key for notifications (e.g. `"5 hours"`, `"7 days"`).
    pub long_name_key: String,
    /// Current utilization percentage.
    pub pct: f64,
    /// Optional reset timestamp in Unix milliseconds.
    pub resets_at_ms: Option<u64>,
}

/// Account-wide usage limits reported by a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AccountLimits {
    /// Provider display name (e.g. `"Claude"`).
    pub provider: String,
    /// Active usage windows.
    pub windows: Vec<UsageWindow>,
    /// Last update timestamp in Unix milliseconds.
    pub updated_at_ms: u64,
}

/// Provider-level quota status (for providers that only report exhaustion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderQuota {
    /// Provider display name (e.g. `"agy"`).
    pub provider: String,
    /// Specific model or quota pool that ran out (e.g. `"Gemini 4 Argon"`), empty if healthy/unknown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Whether a quota exhaustion event is currently active.
    pub exhausted: bool,
    /// Timestamp when the quota ran out, if exhausted.
    pub since_ms: u64,
    /// Timestamp when the quota resets, if exhausted.
    pub resets_at_ms: u64,
}

/// Complete snapshot of currently active agents and provider limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// Only currently active (live) agents on the system.
    pub agents: Vec<Agent>,
    /// Agents that exited recently, used solely to emit end notifications without listing dead agents.
    #[serde(skip)]
    pub recently_ended: Vec<Agent>,
    /// Account-wide rate-limit reports.
    pub limits: Vec<AccountLimits>,
    /// Provider-level quota notices.
    pub quotas: Vec<ProviderQuota>,
}

/// Desktop notification emitted by [`crate::NotificationEngine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Notification summary/title.
    pub title: String,
    /// Notification body text.
    pub body: String,
    /// Urgent notifications persist until dismissed.
    pub urgent: bool,
}
