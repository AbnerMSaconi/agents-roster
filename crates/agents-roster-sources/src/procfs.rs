// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! `/proc` process table and advisory lock probing (without spawning `ps` or `lsof`).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agents_roster_core::{ProcessInfo, ProcessTable};

/// Live `/proc` reader implementing [`ProcessTable`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcFsTable;

/// Process states that mean the process is gone (`Z`/`X`) or suspended (`T`/`t`, e.g. Ctrl+Z).
/// Suspended agents cannot make progress, so they are not reported as active.
pub fn is_inactive_state(state: char) -> bool {
    matches!(state, 'Z' | 'X' | 'T' | 't')
}

/// Parses `(state_char, ppid)` from the contents of `/proc/<pid>/stat`.
pub fn parse_stat(stat: &str) -> Option<(char, u32)> {
    // `comm` can contain spaces and parentheses; fields begin after the last ')'.
    let after = stat.rsplit_once(')')?.1.trim_start();
    let mut parts = after.split_whitespace();
    let state = parts.next()?.chars().next()?;
    let ppid = parts.next()?.parse().ok()?;
    Some((state, ppid))
}

/// Parses `(state, ppid, starttime)` from `/proc/<pid>/stat`.
fn parse_stat_full(stat: &str) -> Option<(char, u32, u64)> {
    let after = stat.rsplit_once(')')?.1.trim_start();
    let mut parts = after.split_whitespace();
    let state = parts.next()?.chars().next()?;
    let ppid = parts.next()?.parse().ok()?;
    let started = parts.nth(17)?.parse().ok()?;
    Some((state, ppid, started))
}

fn read_stat(pid: u32) -> Option<(char, u32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat(&stat)
}

fn read_comm(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn read_open_paths(pid: u32) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| fs::read_link(e.path()).ok())
        .collect()
}

/// Executable basename and (for node-like runtimes) script stem parsed from `cmdline`.
fn cmdline_names(pid: u32) -> (Option<String>, Option<String>) {
    let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return (None, None);
    };
    let mut parts = raw.split(|&b| b == 0).filter(|a| !a.is_empty());
    let first = parts.next().unwrap_or(&[]);
    let exe = Path::new(String::from_utf8_lossy(first).as_ref())
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string);
    let script = if matches!(exe.as_deref(), Some("node" | "nodejs" | "bun")) {
        parts.next().and_then(|s| {
            Path::new(String::from_utf8_lossy(s).as_ref())
                .file_stem()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
    } else {
        None
    };
    (exe, script)
}

fn matches_name(pid: u32, comm: &str, expected: &str) -> bool {
    if comm == expected {
        return true;
    }
    let (exe, script) = cmdline_names(pid);
    exe.as_deref() == Some(expected) || script.as_deref() == Some(expected)
}

/// Immutable per-process identity, cached by `(pid, starttime)` so `/proc/<pid>/{comm,cmdline}`
/// are read once per process lifetime instead of once per scan.
#[derive(Clone)]
struct ProcEntry {
    pid: u32,
    comm: String,
    exe: Option<String>,
    script: Option<String>,
}

impl ProcEntry {
    fn matches(&self, expected: &str) -> bool {
        self.comm == expected
            || self.exe.as_deref() == Some(expected)
            || self.script.as_deref() == Some(expected)
    }
}

#[derive(Default)]
struct ScanCache {
    at: Option<Instant>,
    live: Arc<Vec<ProcEntry>>,
    identities: HashMap<u32, (u64, ProcEntry)>,
}

/// Scans are shared by every source within this window (one tick triggers several lookups).
const SCAN_TTL: Duration = Duration::from_millis(800);

static SCAN: Mutex<Option<ScanCache>> = Mutex::new(None);

/// Returns the live (non-zombie) process list, rescanning `/proc` at most once per [`SCAN_TTL`].
fn snapshot() -> Arc<Vec<ProcEntry>> {
    let mut guard = SCAN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = guard.get_or_insert_with(ScanCache::default);
    if let Some(at) = cache.at
        && at.elapsed() < SCAN_TTL
    {
        return cache.live.clone();
    }
    let mut live = Vec::with_capacity(cache.live.len().max(256));
    let mut seen = HashSet::new();
    if let Ok(rd) = fs::read_dir("/proc") {
        for entry in rd.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some((state, _, started)) = parse_stat_full(&stat) else {
                continue;
            };
            if is_inactive_state(state) {
                continue;
            }
            seen.insert(pid);
            let ident = match cache.identities.get(&pid) {
                Some((st, e)) if *st == started => e.clone(),
                _ => {
                    let Some(comm) = read_comm(pid) else { continue };
                    let (exe, script) = cmdline_names(pid);
                    let e = ProcEntry {
                        pid,
                        comm,
                        exe,
                        script,
                    };
                    cache.identities.insert(pid, (started, e.clone()));
                    e
                }
            };
            live.push(ident);
        }
    }
    cache.identities.retain(|pid, _| seen.contains(pid));
    cache.live = Arc::new(live);
    cache.at = Some(Instant::now());
    cache.live.clone()
}

impl ProcessTable for ProcFsTable {
    fn is_alive(&self, pid: u32, expected_comm: Option<&str>) -> bool {
        if pid == 0 {
            return false;
        }
        let Some((state, _)) = read_stat(pid) else {
            return false;
        };
        if is_inactive_state(state) {
            return false;
        }
        let Some(expected) = expected_comm else {
            return true;
        };
        read_comm(pid).is_some_and(|c| matches_name(pid, &c, expected))
    }

    fn any_alive(&self, comm: &str) -> bool {
        snapshot().iter().any(|e| e.matches(comm))
    }

    fn processes_by_comm(&self, target: &str) -> Vec<ProcessInfo> {
        snapshot()
            .iter()
            .filter(|e| e.matches(target))
            .map(|e| ProcessInfo {
                pid: e.pid,
                comm: e.comm.clone(),
                cwd: fs::read_link(format!("/proc/{}/cwd", e.pid)).ok(),
                open_paths: read_open_paths(e.pid),
            })
            .collect()
    }

    fn ancestor_pids(&self, pid: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut cur = pid;
        for _ in 0..24 {
            if cur <= 1 || chain.contains(&cur) {
                break;
            }
            chain.push(cur);
            let Some((_, parent)) = read_stat(cur) else {
                break;
            };
            cur = parent;
        }
        chain
    }

    fn is_file_locked_by_active(&self, path: &Path) -> bool {
        if !self.is_file_locked(path) {
            return false;
        }
        let holders = fd_holders(path);
        // Holders we cannot see (other namespace/user) keep the previous behaviour: locked = active.
        holders.is_empty() || holders.iter().any(|&pid| !stopped(pid))
    }

    fn is_file_locked(&self, path: &Path) -> bool {
        let Ok(file) = File::open(path) else {
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(TryLockError::WouldBlock) => true,
            Err(_) => false,
        }
    }
}

/// PIDs that have `path` open, found through `/proc/<pid>/fd`.
fn fd_holders(path: &Path) -> Vec<u32> {
    let Ok(rd) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| {
            fs::read_dir(format!("/proc/{pid}/fd")).is_ok_and(|fds| {
                fds.flatten()
                    .any(|fd| fs::read_link(fd.path()).is_ok_and(|t| t == path))
            })
        })
        .collect()
}

fn stopped(pid: u32) -> bool {
    read_stat(pid).is_some_and(|(state, _)| matches!(state, 'T' | 't'))
}

/// Walks up to 16 ancestor processes looking for `target_comm`.
pub fn find_ancestor(target_comm: &str) -> Option<u32> {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..16 {
        if let Some(c) = read_comm(pid)
            && matches_name(pid, &c, target_comm)
        {
            return Some(pid);
        }
        pid = read_stat(pid)?.1;
        if pid <= 1 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_with_spaces_and_parens_in_comm() {
        let sample = "12345 (Web (Content)) S 4321 12345 12345 0 -1 4194304";
        assert_eq!(parse_stat(sample), Some(('S', 4321)));
        let zombie = "99 (claude) Z 1 99 99 0 -1 0";
        assert_eq!(parse_stat(zombie), Some(('Z', 1)));
    }

    #[test]
    fn suspended_and_dead_states_are_inactive() {
        for st in ['Z', 'X', 'T', 't'] {
            assert!(is_inactive_state(st), "{st} should be inactive");
        }
        for st in ['R', 'S', 'D', 'I'] {
            assert!(!is_inactive_state(st), "{st} should be active");
        }
    }

    #[test]
    fn detects_advisory_flock_held_by_process() {
        let dir = std::env::temp_dir().join(format!("agents-procfs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lock");
        let file = File::create(&path).unwrap();
        let table = ProcFsTable;
        assert!(!table.is_file_locked(&path));
        file.lock().unwrap();
        assert!(table.is_file_locked(&path));
        file.unlock().unwrap();
        assert!(!table.is_file_locked(&path));
        fs::remove_dir_all(&dir).ok();
    }
}
