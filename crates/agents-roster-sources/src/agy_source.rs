// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! [`AgentSource`] tracking live `agy` processes via `/proc` and `presence/*.lock`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use agents_roster_core::text::{format_model_name, project_name};
use agents_roster_core::time::format_duration;
use agents_roster_core::{
    Agent, AgentSource, AgentState, Clock, GitInfo, ProcessTable, ProviderQuota, SourceReport,
    TimeFormatter, Translator,
};

use crate::agy_cache::{
    lock_id_from_path, lookup_conv_by_cwd, read_last_conversations, read_metadata_workspaces,
};
use crate::agy_quota::QuotaLogCache;
use crate::agy_subagents::collect_active_subagents;
use crate::agy_transcript::{ConversationState, advance_transcript};

const IDLE_THRESHOLD_MS: u64 = 5 * 60_000;

#[derive(Clone)]
struct LiveSession {
    pid: Option<u32>,
    cwd: PathBuf,
}

/// Discovers active `agy` CLI sessions on the system and reads their incremental transcripts.
pub struct AgySource {
    root: PathBuf,
    context_limit: u64,
    procs: Arc<dyn ProcessTable>,
    git: Arc<dyn GitInfo>,
    clock: Arc<dyn Clock>,
    fmt: Arc<dyn TimeFormatter>,
    conversations: HashMap<String, ConversationState>,
    active_sessions: HashMap<String, LiveSession>,
    quota_cache: QuotaLogCache,
}

impl AgySource {
    /// Creates an agy source reading from `root` (normally `~/.gemini/antigravity-cli`).
    pub fn new(
        root: PathBuf,
        context_limit: u64,
        procs: Arc<dyn ProcessTable>,
        git: Arc<dyn GitInfo>,
        clock: Arc<dyn Clock>,
        fmt: Arc<dyn TimeFormatter>,
    ) -> Self {
        Self {
            root,
            context_limit,
            procs,
            git,
            clock,
            fmt,
            conversations: HashMap::new(),
            active_sessions: HashMap::new(),
            quota_cache: QuotaLogCache::default(),
        }
    }

    fn transcript_path(&self, id: &str) -> PathBuf {
        self.root
            .join("brain")
            .join(id)
            .join(".system_generated/logs/transcript.jsonl")
    }

    fn discover_live_sessions(&self) -> HashMap<String, LiveSession> {
        let mut live: HashMap<String, LiveSession> = HashMap::new();
        let last_map = read_last_conversations(&self.root);
        for proc in self.procs.processes_by_comm("agy") {
            let mut matched = false;
            for fd_path in &proc.open_paths {
                if let Some(id) = lock_id_from_path(fd_path, &self.root) {
                    live.entry(id).or_insert_with(|| LiveSession {
                        pid: Some(proc.pid),
                        cwd: proc.cwd.clone().unwrap_or_default(),
                    });
                    matched = true;
                }
            }
            if !matched
                && let Some(cwd) = &proc.cwd
                && let Some(id) = lookup_conv_by_cwd(cwd, &last_map)
            {
                live.entry(id).or_insert_with(|| LiveSession {
                    pid: Some(proc.pid),
                    cwd: cwd.clone(),
                });
            }
        }
        self.add_locked_presence_sessions(&mut live, &last_map);
        live
    }

    fn add_locked_presence_sessions(
        &self,
        live: &mut HashMap<String, LiveSession>,
        last_map: &HashMap<String, String>,
    ) {
        let Ok(rd) = fs::read_dir(self.root.join("presence")) else {
            return;
        };
        let meta = read_metadata_workspaces(&self.root);
        let rev_last: HashMap<&str, &str> = last_map
            .iter()
            .map(|(dir, id)| (id.as_str(), dir.as_str()))
            .collect();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lock") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if live.contains_key(id) || !self.procs.is_file_locked_by_active(&path) {
                continue;
            }
            let cwd = meta
                .get(id)
                .cloned()
                .or_else(|| rev_last.get(id).map(PathBuf::from))
                .unwrap_or_default();
            live.insert(id.to_string(), LiveSession { pid: None, cwd });
        }
    }

    fn build_agent(
        &mut self,
        id: &str,
        session: &LiveSession,
        is_live: bool,
        now_ms: u64,
        quotas: &[ProviderQuota],
        tr: &dyn Translator,
    ) -> Agent {
        let cwd = &session.cwd;
        let path = self.transcript_path(id);
        let c = self.conversations.entry(id.to_string()).or_default();
        advance_transcript(c, &path, tr);
        let (state, reason) = resolve_state(c, is_live, now_ms, quotas, self.fmt.as_ref(), tr);
        let mut done = if cwd.as_os_str().is_empty() {
            Vec::new()
        } else {
            self.git.commits_since(cwd, c.started_at_ms, 4, tr)
        };
        done.extend(c.done.iter().cloned());
        done.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
        done.truncate(6);
        let pct = (self.context_limit > 0 && c.context > 0)
            .then(|| c.context as f64 * 100.0 / self.context_limit as f64);
        let proj = if cwd.as_os_str().is_empty() {
            id.chars().take(8).collect()
        } else {
            project_name(&cwd.to_string_lossy())
        };
        let now_text = if is_live {
            if c.now.is_empty() {
                tr.translate("session opened").into_owned()
            } else {
                c.now.clone()
            }
        } else {
            tr.translate("process exited").into_owned()
        };
        Agent {
            key: format!("agy:{id}"),
            parent_key: None,
            source_id: "agy".into(),
            source_label: "agy".into(),
            pid: if is_live { session.pid } else { None },
            working_dir: cwd.to_string_lossy().into_owned(),
            project: proj,
            model: format_model_name(&c.model),
            state,
            reason,
            now: now_text,
            now_since_ms: if is_live {
                c.now_since_ms
            } else {
                c.last_at_ms
            },
            next: if is_live {
                c.next.clone()
            } else {
                String::new()
            },
            done,
            context_tokens: c.context,
            context_limit: self.context_limit,
            context_pct: pct,
            updated_at_ms: c.last_at_ms,
        }
    }
}

impl AgentSource for AgySource {
    fn collect(&mut self, tr: &dyn Translator) -> SourceReport {
        let now_ms = self.clock.now_ms();
        let live = self.discover_live_sessions();
        let quotas = self
            .quota_cache
            .read_active_quotas(&self.root.join("log"), now_ms);
        let mut report = SourceReport::default();

        for (id, session) in &live {
            report
                .agents
                .push(self.build_agent(id, session, true, now_ms, &quotas, tr));
        }
        let parents: HashMap<String, (Option<u32>, PathBuf)> = live
            .iter()
            .map(|(id, s)| (id.clone(), (s.pid, s.cwd.clone())))
            .collect();
        let (sub_agents, linked, sub_ids) = collect_active_subagents(
            &self.root,
            &parents,
            &mut self.conversations,
            self.context_limit,
            now_ms,
            tr,
        );
        report.agents.iter_mut().for_each(|a| {
            if let Some(pk) = a.key.strip_prefix("agy:").and_then(|id| linked.get(id)) {
                a.parent_key = Some(format!("agy:{pk}"));
            }
        });
        report.agents.extend(sub_agents);
        for (id, session) in std::mem::take(&mut self.active_sessions) {
            if !live.contains_key(&id) {
                report
                    .recently_ended
                    .push(self.build_agent(&id, &session, false, now_ms, &quotas, tr));
            }
        }
        self.conversations
            .retain(|id, _| live.contains_key(id) || sub_ids.contains(id));
        self.active_sessions = live;
        if !quotas.is_empty() {
            report.quotas = quotas;
        } else if !report.agents.is_empty() {
            report.quotas.push(ProviderQuota {
                provider: "agy".into(),
                model: String::new(),
                exhausted: false,
                since_ms: 0,
                resets_at_ms: 0,
            });
        }
        report
    }
}

fn resolve_state(
    c: &ConversationState,
    is_live: bool,
    now_ms: u64,
    quotas: &[ProviderQuota],
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> (AgentState, String) {
    if is_live {
        if c.waiting_for_user {
            return (AgentState::Waiting, String::new());
        }
        if c.last_at_ms > 0 && now_ms.saturating_sub(c.last_at_ms) > IDLE_THRESHOLD_MS {
            let dur = format_duration(now_ms.saturating_sub(c.last_at_ms));
            return (
                AgentState::Idle,
                tr.format("no activity for {dur}", &[("dur", &dur)]),
            );
        }
        return (AgentState::Working, String::new());
    }
    let matched = quotas
        .iter()
        .find(|q| c.last_at_ms <= q.since_ms + 120_000 && c.last_at_ms + 30 * 60_000 >= q.since_ms);
    if let Some(q) = matched {
        let when = fmt.format_when(q.resets_at_ms, now_ms, tr);
        (
            AgentState::Error,
            tr.format("agy quota exhausted · resets {when}", &[("when", &when)]),
        )
    } else if !c.error.is_empty() {
        (AgentState::Error, c.error.clone())
    } else {
        (AgentState::Stopped, String::new())
    }
}
