// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agents_roster_core::{
    AgentSource, AgentState, Clock, DoneItem, GitInfo, IdentityTranslator, ProcessInfo,
    ProcessTable, TimeFormatter, Translator,
};
use agents_roster_sources::{AgySource, ClaudeCodeSource, CodexSource};

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.0
    }
}

struct FakeFmt;
impl TimeFormatter for FakeFmt {
    fn format_time(&self, ms: u64) -> String {
        format!("T{ms}")
    }
    fn format_when(&self, ms: u64, _now: u64, _tr: &dyn Translator) -> String {
        format!("W{ms}")
    }
}

struct NoGit;
impl GitInfo for NoGit {
    fn commits_since(&self, _d: &Path, _s: u64, _m: usize, _t: &dyn Translator) -> Vec<DoneItem> {
        Vec::new()
    }
}

#[derive(Default)]
struct FakeProcs {
    procs: Mutex<Vec<ProcessInfo>>,
    alive_pids: Mutex<Vec<u32>>,
}

impl ProcessTable for FakeProcs {
    fn is_alive(&self, pid: u32, _comm: Option<&str>) -> bool {
        self.alive_pids.lock().unwrap().contains(&pid)
    }
    fn any_alive(&self, comm: &str) -> bool {
        self.procs.lock().unwrap().iter().any(|p| p.comm == comm)
    }
    fn processes_by_comm(&self, comm: &str) -> Vec<ProcessInfo> {
        self.procs
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.comm == comm)
            .cloned()
            .collect()
    }
    fn ancestor_pids(&self, pid: u32) -> Vec<u32> {
        vec![pid]
    }
    fn is_file_locked(&self, _path: &Path) -> bool {
        false
    }
}

#[test]
fn agy_source_collects_live_subagents_and_skips_killed_ones() {
    let root = std::env::temp_dir().join(format!("agents-agy-sub-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let p_sys = root.join("brain/parent-1/.system_generated");
    fs::create_dir_all(p_sys.join("subagents")).unwrap();
    fs::create_dir_all(p_sys.join("swarm")).unwrap();
    fs::create_dir_all(root.join("brain/child-run/.system_generated/logs")).unwrap();
    fs::create_dir_all(root.join("presence")).unwrap();
    fs::create_dir_all(root.join("cache")).unwrap();
    let mut last = HashMap::new();
    last.insert("/tmp/proj".to_string(), "parent-1".to_string());
    fs::write(
        root.join("cache/last_conversations.json"),
        serde_json::to_vec(&last).unwrap(),
    )
    .unwrap();
    fs::write(
        p_sys.join("subagents/child-run.json"),
        r#"{"conversationId":"child-run","subagentDescriptor":{"typeName":"worker","role":"Swarm Worker (ui)"},"state":"SUBAGENT_STATE_RUNNING"}"#,
    )
    .unwrap();
    fs::write(
        p_sys.join("subagents/child-dead.json"),
        r#"{"conversationId":"child-dead","subagentDescriptor":{"typeName":"worker","role":"Done Worker"},"state":"SUBAGENT_STATE_KILLED"}"#,
    )
    .unwrap();
    fs::write(
        p_sys.join("swarm/members.json"),
        r#"{"members":{"ui":{"conversation_id":"child-run","role":"worker","phase":"RUNNING","killed":false,"model":"gemini-4-hydrogen-high"}}}"#,
    )
    .unwrap();

    let procs = Arc::new(FakeProcs {
        procs: Mutex::new(vec![ProcessInfo {
            pid: 55,
            comm: "agy".into(),
            cwd: Some(PathBuf::from("/tmp/proj")),
            open_paths: vec![root.join("presence/parent-1.lock")],
        }]),
        alive_pids: Mutex::new(vec![55]),
    });
    let mut source = AgySource::new(
        root.clone(),
        1_000_000,
        procs,
        Arc::new(NoGit),
        Arc::new(FixedClock(100_000)),
        Arc::new(FakeFmt),
    );
    let report = source.collect(&IdentityTranslator);
    assert_eq!(report.agents.len(), 2);
    let child = report
        .agents
        .iter()
        .find(|a| a.key == "agy:child-run")
        .unwrap();
    assert_eq!(child.parent_key.as_deref(), Some("agy:parent-1"));
    assert_eq!(child.pid, Some(55));
    assert_eq!(child.project, "Swarm Worker (ui)");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn claude_source_collects_meta_subagents_and_unhooked_processes() {
    let dir = std::env::temp_dir().join(format!("agents-claude-sub-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("claude")).unwrap();
    let proj_dir = dir.join("transcripts/sess-1/subagents");
    fs::create_dir_all(&proj_dir).unwrap();
    fs::write(
        proj_dir.join("agent-a1.meta.json"),
        r#"{"agentType":"explore","description":"Check crates","pickedEffortModel":"claude-haiku-5-5","finished":false}"#,
    )
    .unwrap();
    fs::write(
        proj_dir.join("agent-a2.meta.json"),
        r#"{"agentType":"explore","description":"Old job","finished":true}"#,
    )
    .unwrap();
    let transcript = dir.join("transcripts/sess-1.jsonl");
    let ss_json = serde_json::json!({
        "id": "sess-1",
        "cwd": "/tmp/repo",
        "transcript": transcript,
        "claude_pid": 100,
        "state": "Working",
        "now": "orchestrating",
        "updated_at_ms": 99_000
    });
    fs::write(dir.join("claude/sess-1.json"), ss_json.to_string()).unwrap();

    let procs = Arc::new(FakeProcs {
        procs: Mutex::new(vec![
            ProcessInfo {
                pid: 100,
                comm: "claude".into(),
                cwd: Some(PathBuf::from("/tmp/repo")),
                open_paths: Vec::new(),
            },
            ProcessInfo {
                pid: 200,
                comm: "claude".into(),
                cwd: Some(PathBuf::from("/tmp/child-repo")),
                open_paths: Vec::new(),
            },
        ]),
        alive_pids: Mutex::new(vec![100, 200]),
    });
    let mut source = ClaudeCodeSource::new(
        dir.clone(),
        procs,
        Arc::new(NoGit),
        Arc::new(FixedClock(100_000)),
    );
    let report = source.collect(&IdentityTranslator);
    assert_eq!(report.agents.len(), 3);
    let sub = report
        .agents
        .iter()
        .find(|a| a.key == "claude:sub:agent-a1")
        .unwrap();
    assert_eq!(sub.parent_key.as_deref(), Some("claude:sess-1"));
    assert_eq!(sub.pid, Some(100));
    assert!(report.agents.iter().any(|a| a.key == "claude:pid:200"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn codex_source_collects_rollouts_and_nested_subagents() {
    let root = std::env::temp_dir().join(format!("agents-codex-sub-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let day = root.join("sessions/2026/11/17");
    fs::create_dir_all(&day).unwrap();
    let main_log = day.join("rollout-2026-main.jsonl");
    let sub_log = day.join("rollout-2026-sub.jsonl");
    fs::write(
        &main_log,
        concat!(
            r#"{"timestamp":"2026-11-17T22:00:00Z","type":"session_meta","payload":{"id":"codex-main","cwd":"/tmp/demo","model":"gpt-5.4-codex"}}"#,
            "\n",
            r#"{"timestamp":"2026-11-17T22:00:05Z","type":"response_item","payload":{"type":"function_call","name":"spawn_agent","arguments":"{\"agent\":\"worker\"}"}}"#,
            "\n"
        ),
    )
    .unwrap();
    fs::write(
        &sub_log,
        concat!(
            r#"{"timestamp":"2026-11-17T22:00:06Z","type":"session_meta","payload":{"id":"codex-sub-1","cwd":"/tmp/demo","model":"gpt-5.4-codex","source":{"subagent":{"thread_spawn":{"parent_thread_id":"codex-main","agent_role":"worker"}}}}}"#,
            "\n",
            r#"{"timestamp":"2026-11-17T22:00:07Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}"}}"#,
            "\n"
        ),
    )
    .unwrap();

    let procs = Arc::new(FakeProcs {
        procs: Mutex::new(vec![ProcessInfo {
            pid: 777,
            comm: "codex".into(),
            cwd: Some(PathBuf::from("/tmp/demo")),
            open_paths: vec![main_log],
        }]),
        alive_pids: Mutex::new(vec![777]),
    });
    let mut source = CodexSource::new(
        root.clone(),
        procs,
        Arc::new(NoGit),
        Arc::new(FixedClock(1_794_952_810_000)),
    );
    let report = source.collect(&IdentityTranslator);
    assert_eq!(report.agents.len(), 2);
    let main = report
        .agents
        .iter()
        .find(|a| a.key == "codex:codex-main")
        .unwrap();
    let sub = report
        .agents
        .iter()
        .find(|a| a.key == "codex:codex-sub-1")
        .unwrap();
    assert_eq!(main.state, AgentState::Working);
    assert_eq!(sub.parent_key.as_deref(), Some("codex:codex-main"));
    assert_eq!(sub.project, "worker");
    assert_eq!(sub.pid, Some(777));

    let _ = fs::remove_dir_all(&root);
}
