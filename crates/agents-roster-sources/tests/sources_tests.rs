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
use agents_roster_sources::{AgySource, ClaudeCodeSource, agy_quota::QuotaLogCache};

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
struct FakeProcTable {
    procs: Mutex<Vec<ProcessInfo>>,
    locked_files: Mutex<Vec<PathBuf>>,
    alive_pids: Mutex<Vec<u32>>,
}

impl ProcessTable for FakeProcTable {
    fn is_alive(&self, pid: u32, _expected_comm: Option<&str>) -> bool {
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
    fn is_file_locked(&self, path: &Path) -> bool {
        self.locked_files.lock().unwrap().iter().any(|p| p == path)
    }
}

#[test]
fn agy_source_tracks_only_live_processes_and_transitions_to_waiting() {
    let root = std::env::temp_dir().join(format!("agents-agy-int-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("cache")).unwrap();
    fs::create_dir_all(root.join("presence")).unwrap();

    // Two concurrent live sessions in the same project, plus one dead stale entry in last_conversations.json.
    for id in ["live-1", "live-2", "dead-old"] {
        let log_dir = root.join("brain").join(id).join(".system_generated/logs");
        fs::create_dir_all(&log_dir).unwrap();
        let fixture = include_str!("fixtures/agy_transcript.jsonl");
        let content = if id == "live-1" {
            // Only first 2 lines: tool call in progress -> Working.
            fixture.lines().take(2).collect::<Vec<_>>().join("\n") + "\n"
        } else {
            // Full transcript ending in DONE without tool_calls -> Waiting.
            fixture.to_string()
        };
        fs::write(log_dir.join("transcript.jsonl"), content).unwrap();
    }

    let last_map: HashMap<&str, &str> = [("/tmp/proj", "dead-old")].into_iter().collect();
    fs::write(
        root.join("cache/last_conversations.json"),
        serde_json::to_vec(&last_map).unwrap(),
    )
    .unwrap();

    let procs = Arc::new(FakeProcTable::default());
    *procs.procs.lock().unwrap() = vec![
        ProcessInfo {
            pid: 101,
            comm: "agy".into(),
            cwd: Some(PathBuf::from("/tmp/proj")),
            open_paths: vec![root.join("presence/live-1.lock")],
        },
        ProcessInfo {
            pid: 102,
            comm: "agy".into(),
            cwd: Some(PathBuf::from("/tmp/proj")),
            open_paths: vec![root.join("presence/live-2.lock")],
        },
    ];

    let mut source = AgySource::new(
        root.clone(),
        200_000,
        procs.clone(),
        Arc::new(NoGit),
        Arc::new(FixedClock(1_794_942_135_000)),
        Arc::new(FakeFmt),
    );
    let report = source.collect(&IdentityTranslator);
    assert_eq!(report.agents.len(), 2, "dead-old must not be listed");
    let a1 = report
        .agents
        .iter()
        .find(|a| a.key == "agy:live-1")
        .unwrap();
    let a2 = report
        .agents
        .iter()
        .find(|a| a.key == "agy:live-2")
        .unwrap();
    assert_eq!(a1.state, AgentState::Working);
    assert_eq!(a1.now, "List workspace files");
    assert_eq!(a2.state, AgentState::Waiting);
    assert_eq!(a2.now, "waiting for you");
    assert_eq!(
        a2.done[0].text,
        "replied: Found the file at docs/WORKFLOW.md."
    );

    // Now live-1 exits -> disappears from active agents, appears once in recently_ended.
    procs.procs.lock().unwrap().remove(0);
    let report2 = source.collect(&IdentityTranslator);
    assert_eq!(report2.agents.len(), 1);
    assert_eq!(report2.agents[0].key, "agy:live-2");
    assert_eq!(report2.recently_ended.len(), 1);
    assert_eq!(report2.recently_ended[0].key, "agy:live-1");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn claude_source_reads_session_files_and_filters_closed_sessions() {
    let dir = std::env::temp_dir().join(format!("agents-claude-int-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let sub = dir.join("claude");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        sub.join("dead.json"),
        include_str!("fixtures/closed_claude_session.json"),
    )
    .unwrap();
    fs::write(
        sub.join("limites.json"),
        include_str!("fixtures/claude_limits.json"),
    )
    .unwrap();

    let procs = Arc::new(FakeProcTable::default());
    let mut source = ClaudeCodeSource::new(
        dir.clone(),
        procs,
        Arc::new(NoGit),
        Arc::new(FixedClock(1_794_946_000_000)),
    );
    let report = source.collect(&IdentityTranslator);
    assert!(
        report.agents.is_empty(),
        "closed session must not appear as active"
    );
    assert_eq!(report.limits.len(), 1);
    assert_eq!(report.limits[0].windows.len(), 2);
    assert!((report.limits[0].windows[0].pct - 100.0).abs() < 1e-6);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn agy_quota_cache_lists_all_exhausted_models_until_each_resets() {
    let dir = std::env::temp_dir().join(format!("agents-quota-int-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("cli-test.log"),
        include_str!("fixtures/agy_quota.log"),
    )
    .unwrap();

    let mut cache = QuotaLogCache::default();
    let sample_ts = agents_roster_sources::sys::parse_glog_ms(
        "E1117 14:37:10.000000 1 x",
        agents_roster_sources::SystemClock.now_ms(),
    )
    .unwrap();
    let active = cache.read_active_quotas(&dir, sample_ts + 60_000);
    assert_eq!(active.len(), 2);
    assert_eq!(active[0].model, "Claude Sonnet 5.5");
    assert_eq!(active[1].model, "Gemini 4 Argon");

    // 5 hours later, Claude Sonnet 5.5 has reset (4h), while Gemini 4 Argon (95h) remains listed.
    let later = cache.read_active_quotas(&dir, sample_ts + 5 * 3_600_000);
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].model, "Gemini 4 Argon");

    let _ = fs::remove_dir_all(&dir);
}
