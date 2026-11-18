// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure traits defining the boundaries for OS, git, time, i18n, sources, and notifications.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{AccountLimits, Agent, DoneItem, Notification, ProviderQuota};

/// Monotonic/wall clock abstraction for deterministic testing.
pub trait Clock: Send + Sync {
    /// Current wall-clock time in Unix milliseconds.
    fn now_ms(&self) -> u64;
}

/// Local timezone formatter abstraction (keeps `agents-roster-core` free of OS/libc calls).
pub trait TimeFormatter: Send + Sync {
    /// Formats Unix milliseconds as local `HH:MM`.
    fn format_time(&self, ms: u64) -> String;

    /// Formats Unix milliseconds as `HH:MM` if today, or `<Day> HH:MM` otherwise.
    fn format_when(&self, ms: u64, now_ms: u64, tr: &dyn Translator) -> String;
}

/// Key-value translator with `{name}` placeholder substitution.
pub trait Translator: Send + Sync {
    /// Translates `key` into the active UI language.
    fn translate<'a>(&self, key: &'a str) -> Cow<'a, str>;

    /// Translates `key` and replaces `{name}` placeholders with `args`.
    fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.translate(key).into_owned();
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

/// Fallback translator that keeps English source strings verbatim.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityTranslator;

impl Translator for IdentityTranslator {
    fn translate<'a>(&self, key: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(key)
    }
}

/// Minimal snapshot of a running OS process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Numeric process ID.
    pub pid: u32,
    /// Process command name (`/proc/<pid>/comm`).
    pub comm: String,
    /// Current working directory, if readable.
    pub cwd: Option<PathBuf>,
    /// Open file descriptor targets, if readable.
    pub open_paths: Vec<PathBuf>,
}

/// Read-only view of running OS processes and advisory file locks.
pub trait ProcessTable: Send + Sync {
    /// Returns `true` if `pid` is alive (and non-zombie), optionally matching `expected_comm`.
    fn is_alive(&self, pid: u32, expected_comm: Option<&str>) -> bool;

    /// Returns `true` if at least one non-zombie process matches `comm`.
    fn any_alive(&self, comm: &str) -> bool;

    /// Lists live processes matching `comm`.
    fn processes_by_comm(&self, comm: &str) -> Vec<ProcessInfo>;

    /// Returns ancestor PIDs starting from `pid` up toward init (used to locate open terminal windows).
    fn ancestor_pids(&self, pid: u32) -> Vec<u32>;

    /// Returns `true` if `path` currently has an advisory lock held by another process.
    fn is_file_locked(&self, path: &Path) -> bool;

    /// Like [`ProcessTable::is_file_locked`], but a lock held only by suspended (stopped)
    /// processes does not count as active.
    fn is_file_locked_by_active(&self, path: &Path) -> bool {
        self.is_file_locked(path)
    }
}

/// Opens or focuses the terminal running an agent session.
pub trait TerminalOpener: Send + Sync {
    /// Focuses the open terminal running `agent`, or opens the system default terminal in its directory.
    fn open_agent_terminal(&self, agent: &Agent);
}

/// Repository commit history reader.
pub trait GitInfo: Send + Sync {
    /// Returns up to `max` commits in `dir` created at or after `since_ms`, newest first.
    fn commits_since(
        &self,
        dir: &Path,
        since_ms: u64,
        max: usize,
        tr: &dyn Translator,
    ) -> Vec<DoneItem>;
}

/// Output produced by one [`AgentSource`] collection pass.
#[derive(Debug, Clone, Default)]
pub struct SourceReport {
    /// Currently active (live) agents discovered by this source.
    pub agents: Vec<Agent>,
    /// Agents from this source that exited recently (used for notifications only).
    pub recently_ended: Vec<Agent>,
    /// Account-wide rate-limit reports, if any.
    pub limits: Vec<AccountLimits>,
    /// Provider-level quota status, if any.
    pub quotas: Vec<ProviderQuota>,
}

/// Pluggable agent telemetry source (Open/Closed extension point).
pub trait AgentSource: Send {
    /// Collects current live agents and provider status.
    fn collect(&mut self, tr: &dyn Translator) -> SourceReport;
}

/// Errors emitted while delivering desktop notifications.
#[derive(Debug, Error)]
pub enum NotifyError {
    /// Notification transport is unavailable or failed.
    #[error("notification delivery failed: {0}")]
    Delivery(String),
}

/// Desktop notification sink.
pub trait Notifier: Send {
    /// Sends a single desktop notification.
    ///
    /// # Errors
    /// Returns [`NotifyError::Delivery`] if the notification transport cannot accept the message.
    fn notify(&mut self, note: &Notification) -> Result<(), NotifyError>;
}
