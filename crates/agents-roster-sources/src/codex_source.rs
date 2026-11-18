// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! [`AgentSource`] tracking live `OpenAI` Codex CLI processes and subagent rollouts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agents_roster_core::text::project_name;
use agents_roster_core::time::format_duration;
use agents_roster_core::{
    Agent, AgentSource, AgentState, Clock, GitInfo, ProcessInfo, ProcessTable, SourceReport,
    Translator,
};

use crate::codex_transcript::{CodexConvState, advance_codex_rollout, recent_rollout_files};

const IDLE_THRESHOLD_MS: u64 = 5 * 60_000;
const ROLLOUT_LOOKBACK_MS: u64 = 12 * 3_600_000;

/// Discovers active `codex` CLI processes and their session/subagent rollouts in `~/.codex`.
pub struct CodexSource {
    root: PathBuf,
    procs: Arc<dyn ProcessTable>,
    git: Arc<dyn GitInfo>,
    clock: Arc<dyn Clock>,
    rollouts: HashMap<PathBuf, CodexConvState>,
    active_agents: HashMap<String, Agent>,
}

impl CodexSource {
    /// Creates a Codex source reading from `root` (normally `~/.codex`).
    pub fn new(
        root: PathBuf,
        procs: Arc<dyn ProcessTable>,
        git: Arc<dyn GitInfo>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            root,
            procs,
            git,
            clock,
            rollouts: HashMap::new(),
            active_agents: HashMap::new(),
        }
    }

    fn refresh_rollouts(&mut self, procs: &[ProcessInfo], now_ms: u64, tr: &dyn Translator) {
        let mut paths =
            recent_rollout_files(&self.root.join("sessions"), now_ms, ROLLOUT_LOOKBACK_MS);
        for proc in procs {
            for p in &proc.open_paths {
                if p.extension().and_then(|e| e.to_str()) == Some("jsonl")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("rollout-"))
                {
                    paths.insert(p.clone());
                }
            }
        }
        self.rollouts.retain(|p, _| paths.contains(p));
        for path in paths {
            let state = self.rollouts.entry(path.clone()).or_default();
            advance_codex_rollout(state, &path, tr);
        }
    }

    fn match_proc_rollout(
        &self,
        proc: &ProcessInfo,
        used: &HashSet<PathBuf>,
        now_ms: u64,
    ) -> Option<PathBuf> {
        for p in &proc.open_paths {
            if !used.contains(p) && self.rollouts.contains_key(p) {
                return Some(p.clone());
            }
        }
        let cwd_str = proc
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())?;
        self.rollouts
            .iter()
            .filter(|(p, c)| {
                !used.contains(*p)
                    && c.parent_session_id.is_none()
                    && !c.exited
                    && c.cwd == cwd_str
                    && now_ms.saturating_sub(c.last_at_ms) <= ROLLOUT_LOOKBACK_MS
            })
            .max_by_key(|(_, c)| c.last_at_ms)
            .map(|(p, _)| p.clone())
    }

    fn build_from_rollout(
        &self,
        c: &CodexConvState,
        pid: Option<u32>,
        now_ms: u64,
        tr: &dyn Translator,
    ) -> Agent {
        let mut done = collect_codex_done(self.git.as_ref(), c, tr);
        done.truncate(6);
        let (state, reason) = resolve_codex_state(c, now_ms, tr);
        let pct = (c.context_limit > 0 && c.context_tokens > 0)
            .then(|| c.context_tokens as f64 * 100.0 / c.context_limit as f64);
        Agent {
            key: format!("codex:{}", c.session_id),
            parent_key: c.parent_session_id.as_ref().map(|p| format!("codex:{p}")),
            source_id: "codex".into(),
            source_label: "Codex".into(),
            pid,
            working_dir: c.cwd.clone(),
            project: codex_project_label(c),
            model: c.model.clone(),
            state,
            reason,
            now: if c.now.is_empty() {
                tr.translate("session opened").into_owned()
            } else {
                c.now.clone()
            },
            now_since_ms: if c.now_since_ms > 0 {
                c.now_since_ms
            } else {
                now_ms
            },
            next: c.next.clone(),
            done,
            context_tokens: c.context_tokens,
            context_limit: c.context_limit,
            context_pct: pct,
            updated_at_ms: c.last_at_ms.max(c.now_since_ms),
        }
    }
}

fn build_proc_fallback(proc: &ProcessInfo, now_ms: u64, tr: &dyn Translator) -> Agent {
    let cwd = proc
        .cwd
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = if cwd.is_empty() {
        format!("pid {}", proc.pid)
    } else {
        project_name(&cwd)
    };
    Agent {
        key: format!("codex:pid:{}", proc.pid),
        source_id: "codex".into(),
        source_label: "Codex".into(),
        pid: Some(proc.pid),
        working_dir: cwd,
        project: title,
        state: AgentState::Working,
        now: tr.translate("running in terminal").into_owned(),
        now_since_ms: now_ms,
        updated_at_ms: now_ms,
        ..Default::default()
    }
}

impl AgentSource for CodexSource {
    fn collect(&mut self, tr: &dyn Translator) -> SourceReport {
        let now_ms = self.clock.now_ms();
        let procs = self.procs.processes_by_comm("codex");
        let mut report = SourceReport::default();
        if procs.is_empty() && self.active_agents.is_empty() {
            return report;
        }
        if !procs.is_empty() {
            self.refresh_rollouts(&procs, now_ms, tr);
        }
        let mut used = HashSet::new();
        let mut parent_pids: HashMap<String, Option<u32>> = HashMap::new();
        let mut next_active = HashMap::new();
        for proc in &procs {
            let agent = match self.match_proc_rollout(proc, &used, now_ms) {
                Some(path) => {
                    used.insert(path.clone());
                    let c = &self.rollouts[&path];
                    if !c.session_id.is_empty() {
                        parent_pids.insert(c.session_id.clone(), Some(proc.pid));
                    }
                    self.build_from_rollout(c, Some(proc.pid), now_ms, tr)
                }
                None => build_proc_fallback(proc, now_ms, tr),
            };
            next_active.insert(agent.key.clone(), agent.clone());
            report.agents.push(agent);
        }
        self.append_subagents(
            &parent_pids,
            &used,
            now_ms,
            tr,
            &mut next_active,
            &mut report,
        );
        self.finalize_ended(now_ms, tr, &mut next_active, &mut report);
        report
    }
}

impl CodexSource {
    fn append_subagents(
        &self,
        parent_pids: &HashMap<String, Option<u32>>,
        used: &HashSet<PathBuf>,
        now_ms: u64,
        tr: &dyn Translator,
        next_active: &mut HashMap<String, Agent>,
        report: &mut SourceReport,
    ) {
        for (path, c) in &self.rollouts {
            let Some(pid_id) = c.parent_session_id.as_ref() else {
                continue;
            };
            if used.contains(path) || c.exited || c.session_id.is_empty() {
                continue;
            }
            let Some(&parent_pid) = parent_pids.get(pid_id) else {
                continue;
            };
            let agent = self.build_from_rollout(c, parent_pid, now_ms, tr);
            if !next_active.contains_key(&agent.key) {
                next_active.insert(agent.key.clone(), agent.clone());
                report.agents.push(agent);
            }
        }
    }

    fn finalize_ended(
        &mut self,
        now_ms: u64,
        tr: &dyn Translator,
        next_active: &mut HashMap<String, Agent>,
        report: &mut SourceReport,
    ) {
        let prev = std::mem::take(&mut self.active_agents);
        for (key, mut agent) in prev {
            if !next_active.contains_key(&key) {
                agent.state = AgentState::Stopped;
                agent.now = tr.translate("process exited").into_owned();
                agent.updated_at_ms = now_ms;
                report.recently_ended.push(agent);
            }
        }
        self.active_agents = std::mem::take(next_active);
    }
}

fn collect_codex_done(
    git: &dyn GitInfo,
    c: &CodexConvState,
    tr: &dyn Translator,
) -> Vec<agents_roster_core::DoneItem> {
    let mut done = if c.cwd.is_empty() {
        Vec::new()
    } else {
        git.commits_since(Path::new(&c.cwd), c.started_at_ms, 3, tr)
    };
    done.extend(c.done.iter().cloned());
    done.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    done
}

fn codex_project_label(c: &CodexConvState) -> String {
    if c.parent_session_id.is_some() && !c.subagent_role.is_empty() {
        return c.subagent_role.clone();
    }
    if !c.cwd.is_empty() {
        return project_name(&c.cwd);
    }
    c.session_id.chars().take(8).collect()
}

fn resolve_codex_state(
    c: &CodexConvState,
    now_ms: u64,
    tr: &dyn Translator,
) -> (AgentState, String) {
    if c.waiting {
        return (AgentState::Waiting, String::new());
    }
    if c.last_at_ms > 0 && now_ms.saturating_sub(c.last_at_ms) > IDLE_THRESHOLD_MS {
        let dur = format_duration(now_ms.saturating_sub(c.last_at_ms));
        return (
            AgentState::Idle,
            tr.format("no activity for {dur}", &[("dur", &dur)]),
        );
    }
    (AgentState::Working, String::new())
}
