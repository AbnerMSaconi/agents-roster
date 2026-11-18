// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Discovers active `agy` subagents and swarm workers under `<root>/brain/<parent>/.system_generated/`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use agents_roster_core::text::{format_model_name, truncate};
use agents_roster_core::{Agent, AgentState, Translator};
use serde::Deserialize;

use crate::agy_transcript::{ConversationState, advance_transcript};

#[derive(Default)]
pub(crate) struct ChildSpec {
    pub id: String,
    pub parent_id: String,
    pub label: String,
    pub model_hint: String,
    pub workspace: String,
    pub waiting: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentFile {
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    subagent_descriptor: Option<SubagentDesc>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    workspace_uris: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentDesc {
    #[serde(default)]
    type_name: String,
    #[serde(default)]
    role: String,
}

#[derive(Deserialize)]
struct SwarmFile {
    #[serde(default)]
    members: HashMap<String, SwarmMember>,
}

#[derive(Deserialize)]
struct SwarmMember {
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    desired_state: String,
    #[serde(default)]
    killed: bool,
    #[serde(default)]
    model: String,
}

fn is_terminal_state(state: &str) -> bool {
    let up = state.to_ascii_uppercase();
    ["KILLED", "DONE", "COMPLETED", "FAILED", "ERROR", "CANCEL"]
        .iter()
        .any(|k| up.contains(k))
}

fn read_swarm_members(sys_dir: &Path) -> HashMap<String, (String, SwarmMember)> {
    let mut out = HashMap::new();
    let Ok(bytes) = fs::read(sys_dir.join("swarm/members.json")) else {
        return out;
    };
    let Ok(sf) = serde_json::from_slice::<SwarmFile>(&bytes) else {
        return out;
    };
    for (name, m) in sf.members {
        if !m.conversation_id.is_empty() {
            out.insert(m.conversation_id.clone(), (name, m));
        }
    }
    out
}

fn descriptor_label(desc: Option<SubagentDesc>, swarm_name: Option<&str>, id: &str) -> String {
    if let Some(d) = desc {
        if !d.role.is_empty() {
            return truncate(&d.role, 36);
        }
        if !d.type_name.is_empty() {
            return truncate(&d.type_name, 36);
        }
    }
    swarm_name
        .filter(|s| !s.is_empty())
        .map_or_else(|| id.chars().take(8).collect(), |s| truncate(s, 36))
}

fn collect_parent_children(root: &Path, parent_id: &str, out: &mut Vec<ChildSpec>) {
    let sys_dir = root.join("brain").join(parent_id).join(".system_generated");
    let mut swarm = read_swarm_members(&sys_dir);
    read_subagent_files(&sys_dir, parent_id, &mut swarm, out);
    append_swarm_only(parent_id, swarm, out);
}

fn read_subagent_files(
    sys_dir: &Path,
    parent_id: &str,
    swarm: &mut HashMap<String, (String, SwarmMember)>,
    out: &mut Vec<ChildSpec>,
) {
    let Ok(rd) = fs::read_dir(sys_dir.join("subagents")) else {
        return;
    };
    for entry in rd.flatten() {
        let Some(spec) = parse_subagent_file(&entry.path(), parent_id, swarm) else {
            continue;
        };
        out.push(spec);
    }
}

fn parse_subagent_file(
    path: &Path,
    parent_id: &str,
    swarm: &mut HashMap<String, (String, SwarmMember)>,
) -> Option<ChildSpec> {
    let sf: SubagentFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if sf.conversation_id.is_empty() || is_terminal_state(&sf.state) {
        swarm.remove(&sf.conversation_id);
        return None;
    }
    let sw = swarm.remove(&sf.conversation_id);
    if sw.as_ref().is_some_and(|(_, m)| is_member_ended(m)) {
        return None;
    }
    let workspace = sf
        .workspace_uris
        .first()
        .map(|u| u.strip_prefix("file://").unwrap_or(u).to_string())
        .unwrap_or_default();
    let waiting = sw
        .as_ref()
        .is_some_and(|(_, m)| m.phase.eq_ignore_ascii_case("WAITING"));
    Some(ChildSpec {
        label: descriptor_label(
            sf.subagent_descriptor,
            sw.as_ref().map(|(n, _)| n.as_str()),
            &sf.conversation_id,
        ),
        model_hint: sw.map(|(_, m)| m.model).unwrap_or_default(),
        id: sf.conversation_id,
        parent_id: parent_id.to_string(),
        workspace,
        waiting,
    })
}

fn is_member_ended(m: &SwarmMember) -> bool {
    m.killed || is_terminal_state(&m.phase) || is_terminal_state(&m.desired_state)
}

fn append_swarm_only(
    parent_id: &str,
    swarm: HashMap<String, (String, SwarmMember)>,
    out: &mut Vec<ChildSpec>,
) {
    for (id, (name, m)) in swarm {
        if id == parent_id || m.role.eq_ignore_ascii_case("coordinator") || is_member_ended(&m) {
            continue;
        }
        out.push(ChildSpec {
            id,
            parent_id: parent_id.to_string(),
            label: truncate(&name, 36),
            model_hint: m.model,
            workspace: String::new(),
            waiting: m.phase.eq_ignore_ascii_case("WAITING"),
        });
    }
}

/// Discovers active `agy` subagents/swarm workers for all currently live parent IDs.
pub(crate) fn collect_active_subagents(
    root: &Path,
    parent_sessions: &HashMap<String, (Option<u32>, PathBuf)>,
    conversations: &mut HashMap<String, ConversationState>,
    context_limit: u64,
    now_ms: u64,
    tr: &dyn Translator,
) -> (Vec<Agent>, HashMap<String, String>, HashSet<String>) {
    let mut specs = Vec::new();
    for parent_id in parent_sessions.keys() {
        collect_parent_children(root, parent_id, &mut specs);
    }
    let mut agents = Vec::new();
    let mut linked_parents = HashMap::new();
    let mut active_ids = HashSet::new();
    for spec in specs {
        linked_parents.insert(spec.id.clone(), spec.parent_id.clone());
        if parent_sessions.contains_key(&spec.id) {
            continue;
        }
        active_ids.insert(spec.id.clone());
        let parent_meta = parent_sessions.get(&spec.parent_id);
        let path = root
            .join("brain")
            .join(&spec.id)
            .join(".system_generated/logs/transcript.jsonl");
        let c = conversations.entry(spec.id.clone()).or_default();
        advance_transcript(c, &path, tr);
        agents.push(build_subagent(
            &spec,
            parent_meta,
            c,
            context_limit,
            now_ms,
            tr,
        ));
    }
    (agents, linked_parents, active_ids)
}

fn resolve_subagent_state(
    spec: &ChildSpec,
    c: &ConversationState,
    tr: &dyn Translator,
) -> (AgentState, String) {
    if spec.waiting || c.waiting_for_user {
        let txt = if c.now.is_empty() {
            tr.translate("waiting on swarm board").into_owned()
        } else {
            c.now.clone()
        };
        return (AgentState::Waiting, txt);
    }
    if !c.error.is_empty() {
        return (AgentState::Error, c.error.clone());
    }
    let txt = if c.now.is_empty() {
        tr.translate("subagent running").into_owned()
    } else {
        c.now.clone()
    };
    (AgentState::Working, txt)
}

fn build_subagent(
    spec: &ChildSpec,
    parent_meta: Option<&(Option<u32>, PathBuf)>,
    c: &ConversationState,
    context_limit: u64,
    now_ms: u64,
    tr: &dyn Translator,
) -> Agent {
    let (state, now) = resolve_subagent_state(spec, c, tr);
    let working_dir = if spec.workspace.is_empty() {
        parent_meta
            .map(|(_, d)| d.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        spec.workspace.clone()
    };
    let raw_model = if c.model.is_empty() {
        &spec.model_hint
    } else {
        &c.model
    };
    let pct = (context_limit > 0 && c.context > 0)
        .then(|| c.context as f64 * 100.0 / context_limit as f64);
    Agent {
        key: format!("agy:{}", spec.id),
        parent_key: Some(format!("agy:{}", spec.parent_id)),
        source_id: "agy".into(),
        source_label: "agy".into(),
        pid: parent_meta.and_then(|(p, _)| *p),
        working_dir,
        project: spec.label.clone(),
        model: format_model_name(raw_model),
        state,
        reason: c.error.clone(),
        now,
        now_since_ms: if c.now_since_ms > 0 {
            c.now_since_ms
        } else {
            now_ms
        },
        next: c.next.clone(),
        done: c.done.clone(),
        context_tokens: c.context,
        context_limit,
        context_pct: pct,
        updated_at_ms: c.last_at_ms.max(c.now_since_ms),
    }
}
