// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Generic `/proc` + data-dir watcher for `OpenCode`, Gemini CLI, and custom coding agents.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agents_roster_core::text::project_name;
use agents_roster_core::{
    Agent, AgentSource, AgentState, Clock, ProcessInfo, ProcessTable, SourceReport, Translator,
};

/// Monitors any configured coding-agent CLI process by process name and optional data directory.
pub struct GenericProcessSource {
    id: String,
    label: String,
    process_name: String,
    data_dir: PathBuf,
    procs: Arc<dyn ProcessTable>,
    clock: Arc<dyn Clock>,
    active: HashMap<String, Agent>,
}

impl GenericProcessSource {
    /// Creates a generic process source for a built-in or user-defined AI service.
    pub fn new(
        id: String,
        label: String,
        process_name: String,
        data_dir: PathBuf,
        procs: Arc<dyn ProcessTable>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            id,
            label,
            process_name,
            data_dir,
            procs,
            clock,
            active: HashMap::new(),
        }
    }

    fn build_agent(&self, proc: &ProcessInfo, now_ms: u64, tr: &dyn Translator) -> Agent {
        let cwd = proc
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = if !cwd.is_empty() {
            project_name(&cwd)
        } else if !self.data_dir.as_os_str().is_empty() {
            project_name(&self.data_dir.to_string_lossy())
        } else {
            format!("pid {}", proc.pid)
        };
        Agent {
            key: format!("{}:pid:{}", self.id, proc.pid),
            parent_key: None,
            source_id: self.id.clone(),
            source_label: self.label.clone(),
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
}

impl AgentSource for GenericProcessSource {
    fn collect(&mut self, tr: &dyn Translator) -> SourceReport {
        let mut report = SourceReport::default();
        if self.process_name.is_empty() {
            return report;
        }
        let procs = self.procs.processes_by_comm(&self.process_name);
        if procs.is_empty() && self.active.is_empty() {
            return report;
        }
        let now_ms = self.clock.now_ms();
        let mut next_active = HashMap::new();
        for proc in &procs {
            let agent = self.build_agent(proc, now_ms, tr);
            next_active.insert(agent.key.clone(), agent.clone());
            report.agents.push(agent);
        }
        for (key, mut prev) in std::mem::take(&mut self.active) {
            if !next_active.contains_key(&key) {
                prev.state = AgentState::Stopped;
                prev.now = tr.translate("process exited").into_owned();
                prev.updated_at_ms = now_ms;
                report.recently_ended.push(prev);
            }
        }
        self.active = next_active;
        report
    }
}
