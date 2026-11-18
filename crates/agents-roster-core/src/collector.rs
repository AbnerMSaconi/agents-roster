// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Aggregates snapshots across all registered [`AgentSource`] implementations.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::model::{Agent, Snapshot};
use crate::traits::{AgentSource, ProcessTable, Translator};

/// Collects and sorts active agents and provider statuses from pluggable sources.
pub struct SnapshotCollector {
    sources: Vec<Box<dyn AgentSource>>,
    procs: Option<Arc<dyn ProcessTable>>,
}

impl SnapshotCollector {
    /// Builds a collector over the given agent sources.
    pub fn new(sources: Vec<Box<dyn AgentSource>>) -> Self {
        Self {
            sources,
            procs: None,
        }
    }

    /// Builds a collector that also links nested agent processes via `procs`.
    pub fn with_processes(
        sources: Vec<Box<dyn AgentSource>>,
        procs: Arc<dyn ProcessTable>,
    ) -> Self {
        Self {
            sources,
            procs: Some(procs),
        }
    }

    /// Runs one collection pass across all sources and merges the results.
    pub fn collect(&mut self, tr: &dyn Translator) -> Snapshot {
        let mut snap = Snapshot::default();
        for source in &mut self.sources {
            let report = source.collect(tr);
            snap.agents.extend(report.agents);
            snap.recently_ended.extend(report.recently_ended);
            snap.limits.extend(report.limits);
            snap.quotas.extend(report.quotas);
        }
        link_process_parents(&mut snap.agents, self.procs.as_deref());
        sort_agent_tree(&mut snap.agents);
        snap
    }
}

fn link_process_parents(agents: &mut [Agent], procs: Option<&dyn ProcessTable>) {
    let Some(procs) = procs else { return };
    let pid_map: HashMap<u32, String> = agents
        .iter()
        .filter_map(|a| a.pid.map(|pid| (pid, a.key.clone())))
        .collect();
    if pid_map.is_empty() {
        return;
    }
    for agent in agents {
        if agent.parent_key.is_some() {
            continue;
        }
        let Some(pid) = agent.pid else { continue };
        for anc in procs.ancestor_pids(pid) {
            if anc == pid {
                continue;
            }
            if let Some(parent_key) = pid_map.get(&anc)
                && *parent_key != agent.key
            {
                agent.parent_key = Some(parent_key.clone());
                break;
            }
        }
    }
}

fn effective_rank(agent: &Agent, children: &HashMap<String, Vec<Agent>>, depth: u8) -> u8 {
    let mut rank = agent.state.sort_rank();
    if depth >= 4 {
        return rank;
    }
    if let Some(list) = children.get(&agent.key) {
        for child in list {
            rank = rank.min(effective_rank(child, children, depth + 1));
        }
    }
    rank
}

fn sort_agents_slice(list: &mut [Agent], children: &HashMap<String, Vec<Agent>>) {
    list.sort_by(|a, b| {
        effective_rank(a, children, 0)
            .cmp(&effective_rank(b, children, 0))
            .then(b.updated_at_ms.cmp(&a.updated_at_ms))
    });
}

fn push_subtree(
    agent: Agent,
    children: &mut HashMap<String, Vec<Agent>>,
    out: &mut Vec<Agent>,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(agent.key.clone()) {
        return;
    }
    let key = agent.key.clone();
    out.push(agent);
    if let Some(mut list) = children.remove(&key) {
        sort_agents_slice(&mut list, children);
        for child in list {
            push_subtree(child, children, out, visited);
        }
    }
}

fn sort_agent_tree(agents: &mut Vec<Agent>) {
    let active: HashSet<String> = agents.iter().map(|a| a.key.clone()).collect();
    let mut roots = Vec::new();
    let mut children: HashMap<String, Vec<Agent>> = HashMap::new();
    for a in std::mem::take(agents) {
        match &a.parent_key {
            Some(pk) if active.contains(pk) && *pk != a.key => {
                children.entry(pk.clone()).or_default().push(a);
            }
            _ => roots.push(a),
        }
    }
    sort_agents_slice(&mut roots, &children);
    let mut visited = HashSet::new();
    for root in roots {
        push_subtree(root, &mut children, agents, &mut visited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, AgentState};
    use crate::traits::{IdentityTranslator, ProcessInfo, SourceReport};
    use std::path::Path;

    struct FakeProcs;
    impl ProcessTable for FakeProcs {
        fn is_alive(&self, _pid: u32, _expected_comm: Option<&str>) -> bool {
            true
        }
        fn any_alive(&self, _comm: &str) -> bool {
            false
        }
        fn processes_by_comm(&self, _comm: &str) -> Vec<ProcessInfo> {
            Vec::new()
        }
        fn ancestor_pids(&self, pid: u32) -> Vec<u32> {
            match pid {
                300 => vec![300, 200, 100, 1],
                100 => vec![100, 1],
                _ => vec![pid],
            }
        }
        fn is_file_locked(&self, _path: &Path) -> bool {
            false
        }
    }

    struct FakeSource(Vec<Agent>);

    impl AgentSource for FakeSource {
        fn collect(&mut self, _tr: &dyn Translator) -> SourceReport {
            SourceReport {
                agents: self.0.clone(),
                ..Default::default()
            }
        }
    }

    #[test]
    fn sorts_working_agents_before_waiting_and_idle() {
        let mut collector = SnapshotCollector::new(vec![
            Box::new(FakeSource(vec![Agent {
                key: "a:1".into(),
                state: AgentState::Idle,
                updated_at_ms: 999,
                ..Default::default()
            }])),
            Box::new(FakeSource(vec![Agent {
                key: "b:1".into(),
                state: AgentState::Working,
                updated_at_ms: 100,
                ..Default::default()
            }])),
        ]);
        let snap = collector.collect(&IdentityTranslator);
        assert_eq!(snap.agents.len(), 2);
        assert_eq!(snap.agents[0].key, "b:1");
        assert_eq!(snap.agents[1].key, "a:1");
    }

    #[test]
    fn links_child_process_and_groups_tree_under_parent() {
        let parent = Agent {
            key: "agy:1".into(),
            pid: Some(100),
            state: AgentState::Waiting,
            updated_at_ms: 50,
            ..Default::default()
        };
        let unrelated_waiting = Agent {
            key: "claude:other".into(),
            pid: Some(900),
            state: AgentState::Waiting,
            updated_at_ms: 500,
            ..Default::default()
        };
        let child = Agent {
            key: "claude:child".into(),
            pid: Some(300),
            state: AgentState::Working,
            updated_at_ms: 60,
            ..Default::default()
        };
        let mut collector = SnapshotCollector::with_processes(
            vec![
                Box::new(FakeSource(vec![parent, unrelated_waiting])),
                Box::new(FakeSource(vec![child])),
            ],
            Arc::new(FakeProcs),
        );
        let snap = collector.collect(&IdentityTranslator);
        let keys: Vec<&str> = snap.agents.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, vec!["agy:1", "claude:child", "claude:other"]);
        assert_eq!(snap.agents[1].parent_key.as_deref(), Some("agy:1"));
    }
}
