// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure notification state machine (no I/O, deterministic given clock + snapshot).

use std::collections::{HashMap, HashSet};

use crate::model::{AccountLimits, Agent, AgentState, Notification, ProviderQuota, Snapshot};
use crate::notify_rules::{emit_new_done_items, emit_state_change};
use crate::settings::NotificationPrefs;
use crate::traits::{TimeFormatter, Translator};

/// Minimum turn duration (ms) for assistant reply notifications on interactive turns.
pub const LONG_TURN_MS: u64 = 120_000;

const THRESHOLDS: [f64; 3] = [80.0, 90.0, 95.0];

/// Tracks snapshot transitions and produces non-repeating desktop notifications.
#[derive(Default)]
pub struct NotificationEngine {
    initialized: bool,
    previous: HashMap<String, Agent>,
    thresholds: HashSet<String>,
    notified_quotas: HashMap<(String, String), u64>,
    last_reminder_ms: u64,
    prefs: NotificationPrefs,
}

impl NotificationEngine {
    /// Updates OS notification routing preferences cleanly without resetting dedup state.
    pub fn set_prefs(&mut self, prefs: NotificationPrefs) {
        self.prefs = prefs;
    }
    /// Evaluates `snap` at `now_ms` and returns notifications for meaningful state transitions.
    pub fn process(
        &mut self,
        snap: &Snapshot,
        now_ms: u64,
        reminder_min: u64,
        fmt: &dyn TimeFormatter,
        tr: &dyn Translator,
    ) -> Vec<Notification> {
        let mut out = Vec::new();
        self.process_active_agents(&snap.agents, tr, &mut out);
        self.process_disappeared_agents(snap, tr, &mut out);
        self.process_limits(&snap.limits, now_ms, fmt, tr, &mut out);
        self.process_quotas(&snap.quotas, now_ms, fmt, tr, &mut out);
        self.process_reminder(&snap.agents, now_ms, reminder_min, tr, &mut out);
        self.previous = snap
            .agents
            .iter()
            .map(|a| (a.key.clone(), a.clone()))
            .collect();
        self.initialized = true;
        out
    }

    fn process_active_agents(
        &mut self,
        agents: &[Agent],
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        for agent in agents {
            if agent.parent_key.is_some() {
                continue;
            }
            if self.initialized && self.prefs.os_on_quota {
                self.check_agent_transition(agent, tr, out);
            }
            if let Some(pct) = agent.context_pct {
                self.emit_context_thresholds(agent, pct, tr, out);
            }
        }
    }

    fn check_agent_transition(
        &self,
        agent: &Agent,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        let title = agent.title();
        let Some(prev) = self.previous.get(&agent.key) else {
            if agent.state == AgentState::Working && self.prefs.os_on_started {
                let body = [agent.model.as_str(), agent.now.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                out.push(Notification {
                    title: tr.format("{title} started", &[("title", &title)]),
                    body,
                    urgent: false,
                });
            }
            return;
        };
        emit_new_done_items(agent, prev, &self.prefs, tr, out);
        if prev.state != agent.state {
            emit_state_change(prev.state, agent, &self.prefs, tr, out);
        }
    }

    fn process_disappeared_agents(
        &self,
        snap: &Snapshot,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        if !self.initialized {
            return;
        }
        let active: HashSet<&str> = snap.agents.iter().map(|a| a.key.as_str()).collect();
        let ended: HashMap<&str, &Agent> = snap
            .recently_ended
            .iter()
            .map(|a| (a.key.as_str(), a))
            .collect();
        for (key, prev) in &self.previous {
            if prev.parent_key.is_some() || active.contains(key.as_str()) {
                continue;
            }
            if let Some(last) = ended.get(key.as_str()) {
                emit_new_done_items(last, prev, &self.prefs, tr, out);
                emit_state_change(prev.state, last, &self.prefs, tr, out);
            } else if prev.state == AgentState::Working && self.prefs.os_on_finished {
                let title = prev.title();
                out.push(Notification {
                    title: tr.format("{title} ended", &[("title", &title)]),
                    body: prev
                        .done
                        .first()
                        .map(|d| d.text.clone())
                        .unwrap_or_default(),
                    urgent: false,
                });
            }
        }
    }

    fn check_threshold(&mut self, key: &str, pct: f64) -> Vec<f64> {
        let mut crossed = Vec::new();
        for limit in THRESHOLDS {
            let tag = format!("{key}:{limit}");
            if pct >= limit {
                if self.thresholds.insert(tag) && self.initialized {
                    crossed.push(limit);
                }
            } else {
                self.thresholds.remove(&tag);
            }
        }
        crossed
    }

    fn emit_context_thresholds(
        &mut self,
        agent: &Agent,
        pct: f64,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        let title = agent.title();
        for limit in self.check_threshold(&format!("ctx:{}", agent.key), pct) {
            if !self.prefs.os_on_context_high {
                continue;
            }
            let pct_s = format!("{limit:.0}");
            out.push(Notification {
                title: tr.format(
                    "{title}: context at {pct}%",
                    &[("title", &title), ("pct", &pct_s)],
                ),
                body: tr
                    .translate("Near the limit: the agent will compact or lose old details.")
                    .into_owned(),
                urgent: limit >= 95.0,
            });
        }
    }

    fn process_limits(
        &mut self,
        limits: &[AccountLimits],
        now_ms: u64,
        fmt: &dyn TimeFormatter,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        for lim in limits {
            for win in &lim.windows {
                for limit in
                    self.check_threshold(&format!("{}:{}", lim.provider, win.name), win.pct)
                {
                    if !self.prefs.os_on_quota {
                        continue;
                    }
                    let window = tr.translate(&win.long_name_key);
                    let pct_s = format!("{limit:.0}");
                    let body = win
                        .resets_at_ms
                        .map(|t| {
                            tr.format(
                                "Resets to zero {when}.",
                                &[("when", &fmt.format_when(t, now_ms, tr))],
                            )
                        })
                        .unwrap_or_default();
                    out.push(Notification {
                        title: tr.format(
                            "{provider}: {window} limit at {pct}%",
                            &[
                                ("provider", &lim.provider),
                                ("window", &window),
                                ("pct", &pct_s),
                            ],
                        ),
                        body,
                        urgent: limit >= 95.0,
                    });
                }
            }
        }
    }

    fn process_quotas(
        &mut self,
        quotas: &[ProviderQuota],
        now_ms: u64,
        fmt: &dyn TimeFormatter,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        for q in quotas.iter().filter(|q| q.exhausted) {
            let key = (q.provider.clone(), q.model.clone());
            if self.notified_quotas.get(&key) == Some(&q.since_ms) {
                continue;
            }
            if self.initialized && self.prefs.os_on_quota {
                let when = fmt.format_when(q.resets_at_ms, now_ms, tr);
                let title = if q.model.is_empty() {
                    tr.format("{provider} quota exhausted", &[("provider", &q.provider)])
                } else {
                    tr.format(
                        "{provider} · {model} quota exhausted",
                        &[("provider", &q.provider), ("model", &q.model)],
                    )
                };
                out.push(Notification {
                    title,
                    body: tr.format("The active model stops until {when}. Other {provider} models can continue.", &[("when", &when), ("provider", &q.provider)]),
                    urgent: true,
                });
            }
            self.notified_quotas.insert(key, q.since_ms);
        }
    }

    fn process_reminder(
        &mut self,
        agents: &[Agent],
        now_ms: u64,
        reminder_min: u64,
        tr: &dyn Translator,
        out: &mut Vec<Notification>,
    ) {
        let working: Vec<&Agent> = agents
            .iter()
            .filter(|a| a.state == AgentState::Working)
            .collect();
        let interval = reminder_min * 60_000;
        if self.last_reminder_ms == 0 || working.is_empty() {
            self.last_reminder_ms = now_ms;
            return;
        }
        if !self.prefs.os_on_reminder
            || interval == 0
            || now_ms.saturating_sub(self.last_reminder_ms) < interval
        {
            return;
        }
        self.last_reminder_ms = now_ms;
        let n_s = working.len().to_string();
        let title = if working.len() == 1 {
            tr.translate("1 agent working").into_owned()
        } else {
            tr.format("{n} agents working", &[("n", &n_s)])
        };
        let body: Vec<String> = working
            .iter()
            .map(|a| format!("{}: {}", a.title(), a.now))
            .collect();
        out.push(Notification {
            title,
            body: body.join("\n"),
            urgent: false,
        });
    }
}

#[cfg(test)]
mod tests;
