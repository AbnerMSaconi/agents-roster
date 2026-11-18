// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! [`AgentSource`] reading live Claude Code sessions from hook exchange files.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agents_roster_core::text::base_name;
use agents_roster_core::{
    AccountLimits, Agent, AgentSource, AgentState, Clock, GitInfo, ProcessTable, SourceReport,
    Translator, UsageWindow,
};

use crate::claude_model::{Session, StoredLimits};
use crate::claude_subagents::{collect_session_subagents, discover_unhooked_claude};

const RECENT_END_WINDOW_MS: u64 = 15_000;
const STALE_WORKING_MS: u64 = 10 * 60_000;
const MAX_LIVE_AGE_MS: u64 = 12 * 3_600_000;

/// Reads active Claude Code sessions and rate limits from `<exchange_dir>/claude/`.
pub struct ClaudeCodeSource {
    dir: PathBuf,
    procs: Arc<dyn ProcessTable>,
    git: Arc<dyn GitInfo>,
    clock: Arc<dyn Clock>,
    active_keys: HashSet<String>,
}

impl ClaudeCodeSource {
    /// Creates a new Claude Code source backed by the given adapters.
    pub fn new(
        dir: PathBuf,
        procs: Arc<dyn ProcessTable>,
        git: Arc<dyn GitInfo>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            dir,
            procs,
            git,
            clock,
            active_keys: HashSet::new(),
        }
    }

    fn is_session_alive(&self, ss: &Session, now: u64) -> bool {
        if ss.closed || now.saturating_sub(ss.updated_at_ms) > MAX_LIVE_AGE_MS {
            return false;
        }
        // Sessions whose hook could not find its `claude` process (pid 0) are bound to a process
        // by working directory in `bind_unknown_pids`; any left unbound have no live process.
        ss.claude_pid != 0 && self.procs.is_alive(ss.claude_pid, Some("claude"))
    }

    /// Reads every session file in `sub`, newest first, binding sessions recorded without a pid
    /// to the unclaimed `claude` process running in the same directory.
    fn read_sessions(&self, sub: &Path, now: u64) -> Vec<Session> {
        let mut sessions: Vec<Session> = fs::read_dir(sub)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("json")
                    && p.file_name() != Some("limites.json".as_ref())
            })
            .filter_map(|p| fs::read(p).ok())
            .filter_map(|b| serde_json::from_slice::<Session>(&b).ok())
            .collect();
        sessions.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        let mut claimed: HashSet<u32> = sessions
            .iter()
            .filter(|s| s.claude_pid != 0)
            .map(|s| s.claude_pid)
            .collect();
        let mut procs: Option<Vec<agents_roster_core::ProcessInfo>> = None;
        for ss in sessions.iter_mut().filter(|s| {
            s.claude_pid == 0 && !s.closed && now.saturating_sub(s.updated_at_ms) <= MAX_LIVE_AGE_MS
        }) {
            let dir = if ss.project_dir.is_empty() {
                &ss.cwd
            } else {
                &ss.project_dir
            };
            let list = procs.get_or_insert_with(|| self.procs.processes_by_comm("claude"));
            let found = list.iter().find(|p| {
                !claimed.contains(&p.pid)
                    && p.cwd
                        .as_deref()
                        .is_some_and(|c| c == Path::new(dir.as_str()))
            });
            if let Some(p) = found {
                ss.claude_pid = p.pid;
                claimed.insert(p.pid);
            }
        }
        sessions
    }

    fn build_agent(&self, ss: Session, alive: bool, now: u64, tr: &dyn Translator) -> Agent {
        let dir = if ss.project_dir.is_empty() {
            &ss.cwd
        } else {
            &ss.project_dir
        };
        let mut state = if alive { ss.state } else { AgentState::Stopped };
        if state == AgentState::Working && now.saturating_sub(ss.updated_at_ms) > STALE_WORKING_MS {
            state = AgentState::Idle;
        }
        let mut done = ss.done.clone();
        if !dir.is_empty() {
            done.extend(
                self.git
                    .commits_since(Path::new(dir), ss.started_at_ms, 3, tr),
            );
            done.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
            done.truncate(6);
        }
        let next_task = ss
            .tasks
            .iter()
            .find(|t| t.status == "in_progress")
            .or_else(|| ss.tasks.iter().find(|t| t.status == "pending"));
        Agent {
            key: format!("claude:{}", ss.id),
            parent_key: None,
            source_id: "claude".into(),
            source_label: "Claude Code".into(),
            pid: (ss.claude_pid != 0).then_some(ss.claude_pid),
            working_dir: dir.clone(),
            project: base_name(dir).to_string(),
            model: ss.model,
            state,
            reason: ss.reason,
            now: if alive {
                ss.now
            } else {
                tr.translate("session closed").into_owned()
            },
            now_since_ms: ss.now_since_ms,
            next: next_task.map(|t| t.text.clone()).unwrap_or(ss.next),
            done,
            context_tokens: ss.context_tokens,
            context_limit: ss.context_limit,
            context_pct: ss.context_pct,
            updated_at_ms: ss.updated_at_ms,
        }
    }

    fn read_limits(&self) -> Option<AccountLimits> {
        let stored: StoredLimits = fs::read(self.dir.join("claude/limites.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())?;
        let mut windows = Vec::new();
        if let Some(w) = stored.five_hour {
            windows.push(UsageWindow {
                name: "5h".into(),
                long_name_key: "5 hours".into(),
                pct: w.pct,
                resets_at_ms: w.resets_at_ms,
            });
        }
        if let Some(w) = stored.seven_day {
            windows.push(UsageWindow {
                name: "7d".into(),
                long_name_key: "7 days".into(),
                pct: w.pct,
                resets_at_ms: w.resets_at_ms,
            });
        }
        (!windows.is_empty()).then_some(AccountLimits {
            provider: "Claude".into(),
            windows,
            updated_at_ms: stored.updated_at_ms,
        })
    }
}

impl AgentSource for ClaudeCodeSource {
    fn collect(&mut self, tr: &dyn Translator) -> SourceReport {
        let sub = self.dir.join("claude");
        let now = self.clock.now_ms();
        let mut report = SourceReport::default();
        let mut next_active = HashSet::new();
        let mut hooked_pids = HashSet::new();
        for ss in self.read_sessions(&sub, now) {
            let key = format!("claude:{}", ss.id);
            let alive = self.is_session_alive(&ss, now);
            if alive {
                hooked_pids.insert(ss.claude_pid);
                let dir = if ss.project_dir.is_empty() {
                    ss.cwd.clone()
                } else {
                    ss.project_dir.clone()
                };
                let subs = collect_session_subagents(&ss, &dir, now, tr);
                next_active.insert(key);
                for sub_agent in subs {
                    next_active.insert(sub_agent.key.clone());
                    report.agents.push(sub_agent);
                }
                report.agents.push(self.build_agent(ss, true, now, tr));
            } else if self.active_keys.contains(&key)
                || now.saturating_sub(ss.updated_at_ms) <= RECENT_END_WINDOW_MS
            {
                report
                    .recently_ended
                    .push(self.build_agent(ss, false, now, tr));
            }
        }
        for unhooked in discover_unhooked_claude(self.procs.as_ref(), &hooked_pids, now, tr) {
            next_active.insert(unhooked.key.clone());
            report.agents.push(unhooked);
        }
        self.active_keys = next_active;
        if let Some(lim) = self.read_limits() {
            report.limits.push(lim);
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agents_roster_core::{IdentityTranslator, ProcessInfo};

    struct FixedClock(u64);
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            self.0
        }
    }

    struct FakeProcs(Vec<u32>);
    impl ProcessTable for FakeProcs {
        fn is_alive(&self, pid: u32, _comm: Option<&str>) -> bool {
            self.0.contains(&pid)
        }
        fn any_alive(&self, _comm: &str) -> bool {
            !self.0.is_empty()
        }
        fn processes_by_comm(&self, _comm: &str) -> Vec<ProcessInfo> {
            self.0
                .iter()
                .map(|&pid| ProcessInfo {
                    pid,
                    comm: "claude".into(),
                    cwd: Some(PathBuf::from(format!("/tmp/proj-{pid}"))),
                    open_paths: Vec::new(),
                })
                .collect()
        }
        fn ancestor_pids(&self, pid: u32) -> Vec<u32> {
            vec![pid]
        }
        fn is_file_locked(&self, _path: &Path) -> bool {
            false
        }
    }

    struct NoGit;
    impl GitInfo for NoGit {
        fn commits_since(
            &self,
            _d: &Path,
            _s: u64,
            _m: usize,
            _t: &dyn Translator,
        ) -> Vec<agents_roster_core::DoneItem> {
            Vec::new()
        }
    }

    fn write_session(sub: &Path, id: &str, cwd: &str, pid: u32) {
        let json = format!(
            r#"{{"id":"{id}","cwd":"{cwd}","claude_pid":{pid},"state":"Idle","updated_at_ms":1794945990000,"closed":false}}"#
        );
        fs::write(sub.join(format!("{id}.json")), json).unwrap();
    }

    #[test]
    fn pidless_session_binds_to_process_in_same_dir_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("agents-claude-pid0-{}", std::process::id()));
        let sub = dir.join("claude");
        fs::create_dir_all(&sub).unwrap();
        // 42 runs in /tmp/proj-42; the pid-less session matches it, the other one matches nothing.
        write_session(&sub, "bound", "/tmp/proj-42", 0);
        write_session(&sub, "orphan", "/tmp/elsewhere", 0);
        let mut src = ClaudeCodeSource::new(
            dir.clone(),
            Arc::new(FakeProcs(vec![42])),
            Arc::new(NoGit),
            Arc::new(FixedClock(1_794_946_000_000)),
        );
        let report = src.collect(&IdentityTranslator);
        let keys: Vec<_> = report.agents.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["claude:bound"], "no ghost and no pid:42 duplicate");
        assert_eq!(report.agents[0].pid, Some(42));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn excludes_dead_and_closed_sessions_from_active_agents() {
        let dir = std::env::temp_dir().join(format!("agents-claude-src-{}", std::process::id()));
        let sub = dir.join("claude");
        fs::create_dir_all(&sub).unwrap();
        let now = 1_794_946_000_000u64;
        fs::write(sub.join("alive.json"), r#"{"id":"alive","cwd":"/tmp/app","claude_pid":42,"state":"Working","now":"editing x.rs","updated_at_ms":1794945990000,"closed":false}"#).unwrap();
        fs::write(sub.join("deadpid.json"), r#"{"id":"deadpid","cwd":"/tmp/old","claude_pid":99,"state":"Working","now":"stale","updated_at_ms":1794945990000,"closed":false}"#).unwrap();
        fs::write(sub.join("closed.json"), r#"{"id":"closed","cwd":"/tmp/done","claude_pid":42,"state":"Stopped","now":"session closed","updated_at_ms":1794945990000,"closed":true}"#).unwrap();

        let mut src = ClaudeCodeSource::new(
            dir.clone(),
            Arc::new(FakeProcs(vec![42])),
            Arc::new(NoGit),
            Arc::new(FixedClock(now)),
        );
        let report = src.collect(&IdentityTranslator);
        assert_eq!(report.agents.len(), 1);
        assert_eq!(report.agents[0].key, "claude:alive");
        assert_eq!(report.recently_ended.len(), 2);
        fs::remove_dir_all(&dir).ok();
    }
}
