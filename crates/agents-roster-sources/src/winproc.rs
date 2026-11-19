// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Windows process table and file-lock probing (Toolhelp snapshot, PEB reads, handle table).
//!
//! Mirrors the Linux `procfs` backend: one cached snapshot shared by all sources per tick,
//! per-process identity cached by `(pid, creation_time)` so command lines are parsed once
//! per process lifetime. Open file paths come from the system handle table; names are
//! resolved on a sacrificial worker thread because `GetFinalPathNameByHandle` can block on
//! synchronous named-pipe handles (a hung worker is abandoned and the handle blacklisted).

#![allow(unsafe_code)] // Win32/NT FFI: every call is audited and confined to this module.

use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agents_roster_core::{ProcessInfo, ProcessTable};
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_LOCK_VIOLATION,
    ERROR_SHARING_VIOLATION, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_ACCESS_RIGHTS, PROCESS_DUP_HANDLE, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::CommandLineToArgvW;
use windows::core::PCWSTR;

// These native APIs are exported by ntdll but are not projected in windows-rs 0.62.
// Keep their ABI declarations local to this audited Windows-only module.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        information_class: i32,
        information: *mut core::ffi::c_void,
        information_length: u32,
        return_length: *mut u32,
    ) -> i32;

    fn NtQuerySystemInformation(
        information_class: i32,
        information: *mut core::ffi::c_void,
        information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

/// Live Win32 process reader implementing [`ProcessTable`].
#[derive(Debug, Default, Clone, Copy)]
pub struct WinProcTable;

/// `STILL_ACTIVE` exit code of a running process.
const STILL_ACTIVE: u32 = 259;
/// `SystemExtendedHandleInformation` class of `NtQuerySystemInformation`.
const SYSTEM_EXTENDED_HANDLE_INFORMATION: u32 = 64;
/// `STATUS_INFO_LENGTH_MISMATCH`.
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

/// Immutable per-process identity, cached by `(pid, creation_time)`.
#[derive(Clone)]
struct ProcEntry {
    pid: u32,
    parent: u32,
    /// Executable basename without `.exe`, lowercased (`claude`, `node`, `agy`).
    stem: String,
    /// For node-like runtimes: script stem (`claude`, `codex`), lowercased.
    script: Option<String>,
}

impl ProcEntry {
    fn matches(&self, expected: &str) -> bool {
        let expected = expected.to_ascii_lowercase();
        let expected = expected.strip_suffix(".exe").unwrap_or(&expected);
        self.stem == expected || self.script.as_deref() == Some(expected)
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

fn filetime_u64(ft: windows::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Creation time used to detect PID reuse; `0` when the process cannot be opened.
fn creation_time(pid: u32) -> u64 {
    let Ok(h) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return 0;
    };
    let mut born = Default::default();
    let (mut exit, mut kernel, mut user) = (Default::default(), Default::default(), Default::default());
    let ft = unsafe { GetProcessTimes(h, &mut born, &mut exit, &mut kernel, &mut user) }
        .ok()
        .map(|()| filetime_u64(born))
        .unwrap_or(0);
    let _ = unsafe { CloseHandle(h) };
    ft
}

fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> windows::core::Result<HANDLE> {
    unsafe { OpenProcess(access, false, pid) }
}

/// Executable basename (without extension, lowercased) of a live process.
fn exe_basename(pid: u32) -> Option<String> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
    let _ = unsafe { CloseHandle(h) };
    if !ok || len == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

/// Reads `len` bytes of the target process at `addr` (PEB walk); `None` on any failure.
fn read_mem(h: HANDLE, addr: usize, len: usize) -> Option<Vec<u8>> {
    if addr == 0 || len == 0 || len > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let mut got = 0usize;
    unsafe { ReadProcessMemory(h, addr as *const core::ffi::c_void, buf.as_mut_ptr().cast(), len, Some(&mut got)) }.ok()?;
    (got == len).then_some(buf)
}

fn read_usize(h: HANDLE, addr: usize) -> Option<usize> {
    let b = read_mem(h, addr, 8)?;
    Some(usize::from_le_bytes(b.try_into().ok()?))
}

/// Reads a `UNICODE_STRING` (u16 length, u16 max, ptr) stored at `addr` in the target.
fn read_unicode_string(h: HANDLE, addr: usize) -> Option<String> {
    let head = read_mem(h, addr, 16)?;
    let len = u16::from_le_bytes([head[0], head[1]]) as usize;
    let ptr = usize::from_le_bytes(head[8..16].try_into().ok()?);
    if len == 0 || len > 32 * 1024 {
        return None;
    }
    let bytes = read_mem(h, ptr, len)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units).trim_end_matches('\\').to_string())
}

/// x64 PEB offsets (stable since Vista): PEB->ProcessParameters, then CommandLine / CurrentDirectory.
const PEB_PROCESS_PARAMETERS: usize = 0x20;
const PARAMS_CURRENT_DIRECTORY: usize = 0x38;
const PARAMS_COMMAND_LINE: usize = 0x60;

#[repr(C)]
struct ProcessBasicInformation {
    reserved1: *mut core::ffi::c_void,
    peb_base_address: *mut core::ffi::c_void,
    reserved2: [*mut core::ffi::c_void; 2],
    unique_process_id: usize,
    reserved3: *mut core::ffi::c_void,
}

fn process_params_addr(h: HANDLE) -> Option<usize> {
    let mut pbi: ProcessBasicInformation = unsafe { core::mem::zeroed() };
    let status = unsafe {
        NtQueryInformationProcess(
            h,
            0, // ProcessBasicInformation
            (&mut pbi as *mut ProcessBasicInformation).cast(),
            core::mem::size_of::<ProcessBasicInformation>() as u32,
            core::ptr::null_mut(),
        )
    };
    if status != 0 {
        return None;
    }
    let peb = pbi.peb_base_address as usize;
    read_usize(h, peb + PEB_PROCESS_PARAMETERS)
}

/// Full command line of the target process (empty when unreadable, e.g. elevated processes).
fn command_line(pid: u32) -> Option<String> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ).ok()?;
    let params = process_params_addr(h);
    let cmd = params.and_then(|p| read_unicode_string(h, p + PARAMS_COMMAND_LINE));
    let _ = unsafe { CloseHandle(h) };
    cmd
}

/// Current working directory of the target process, from its PEB.
fn process_cwd(pid: u32) -> Option<PathBuf> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ).ok()?;
    let params = process_params_addr(h);
    let cwd = params.and_then(|p| read_unicode_string(h, p + PARAMS_CURRENT_DIRECTORY));
    let _ = unsafe { CloseHandle(h) };
    cwd.filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Splits a Windows command line honoring quotes, via `CommandLineToArgvW`.
fn split_cmdline(cmd: &str) -> Vec<String> {
    let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
    let mut argc = 0i32;
    let argv = unsafe { CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut argc) };
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(argc as usize);
    for i in 0..argc as isize {
        let p = unsafe { *argv.offset(i) };
        if p.is_null() {
            continue;
        }
        let mut len = 0usize;
        while unsafe { *p.0.add(len) } != 0 && len < 32 * 1024 {
            len += 1;
        }
        out.push(String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p.0, len) }));
    }
    let _ = unsafe { LocalFree(Some(HLOCAL(argv.cast()))) };
    out
}

/// Script stem for node-like runtimes, mapping npm package entry points to the agent name
/// (`@anthropic-ai/claude-code/cli.js` runs as `node`, but the agent is `claude`).
fn script_stem(cmd: &str) -> Option<String> {
    let args = split_cmdline(cmd);
    let script = args.get(1)?;
    let lower = script.to_ascii_lowercase().replace('/', "\\");
    if lower.contains("claude-code") || lower.contains("@anthropic-ai\\claude") {
        return Some("claude".into());
    }
    if lower.contains("@openai\\codex") || lower.contains("codex-cli") {
        return Some("codex".into());
    }
    Path::new(script)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

/// Returns the live process list, rescanning at most once per [`SCAN_TTL`].
fn snapshot() -> Arc<Vec<ProcEntry>> {
    let mut guard = SCAN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = guard.get_or_insert_with(ScanCache::default);
    if let Some(at) = cache.at
        && at.elapsed() < SCAN_TTL
    {
        return cache.live.clone();
    }
    let mut live = Vec::with_capacity(cache.live.len().max(256));
    let mut seen = HashSet::new();
    if let Ok(snap) = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        let mut entry = PROCESSENTRY32W { dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = unsafe { Process32FirstW(snap, &mut entry) }.is_ok();
        while more {
            let pid = entry.th32ProcessID;
            if pid > 4 {
                seen.insert(pid);
                let born = creation_time(pid);
                let ident = match cache.identities.get(&pid) {
                    Some((st, e)) if *st == born && born != 0 => e.clone(),
                    _ => {
                        let nul = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                        let exe = String::from_utf16_lossy(&entry.szExeFile[..nul]);
                        let stem = Path::new(&exe).file_stem().and_then(|s| s.to_str()).unwrap_or(&exe).to_ascii_lowercase();
                        let script = if matches!(stem.as_str(), "node" | "nodejs" | "bun") {
                            command_line(pid).as_deref().and_then(script_stem)
                        } else {
                            None
                        };
                        let e = ProcEntry { pid, parent: entry.th32ParentProcessID, stem, script };
                        cache.identities.insert(pid, (born, e.clone()));
                        e
                    }
                };
                live.push(ident);
            }
            more = unsafe { Process32NextW(snap, &mut entry) }.is_ok();
        }
        let _ = unsafe { CloseHandle(snap) };
    }
    cache.identities.retain(|pid, _| seen.contains(pid));
    cache.live = Arc::new(live);
    cache.at = Some(Instant::now());
    cache.live.clone()
}

impl ProcessTable for WinProcTable {
    fn is_alive(&self, pid: u32, expected_comm: Option<&str>) -> bool {
        if pid == 0 {
            return false;
        }
        let Ok(h) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return false;
        };
        let mut code = 0u32;
        let alive = unsafe { GetExitCodeProcess(h, &mut code) }.is_ok() && code == STILL_ACTIVE;
        let _ = unsafe { CloseHandle(h) };
        if !alive {
            return false;
        }
        let Some(expected) = expected_comm else {
            return true;
        };
        let expected = expected.to_ascii_lowercase();
        let expected = expected.strip_suffix(".exe").unwrap_or(&expected);
        if exe_basename(pid).as_deref() == Some(expected) {
            return true;
        }
        // node-hosted agents: match the script (npm installs of claude/codex).
        if matches!(exe_basename(pid).as_deref(), Some("node" | "nodejs" | "bun")) {
            return command_line(pid).as_deref().and_then(script_stem).as_deref() == Some(expected);
        }
        false
    }

    fn any_alive(&self, comm: &str) -> bool {
        snapshot().iter().any(|e| e.matches(comm))
    }

    fn processes_by_comm(&self, target: &str) -> Vec<ProcessInfo> {
        let matches: Vec<ProcEntry> = snapshot().iter().filter(|e| e.matches(target)).cloned().collect();
        if matches.is_empty() {
            return Vec::new();
        }
        let pids: Vec<u32> = matches.iter().map(|e| e.pid).collect();
        let mut paths = open_paths_for(&pids);
        matches
            .into_iter()
            .map(|e| ProcessInfo {
                pid: e.pid,
                comm: e.stem.clone(),
                cwd: process_cwd(e.pid),
                open_paths: paths.remove(&e.pid).unwrap_or_default(),
            })
            .collect()
    }

    fn ancestor_pids(&self, pid: u32) -> Vec<u32> {
        let snap = snapshot();
        let parents: HashMap<u32, u32> = snap.iter().map(|e| (e.pid, e.parent)).collect();
        let mut chain = Vec::new();
        let mut cur = pid;
        for _ in 0..24 {
            if cur <= 4 || chain.contains(&cur) {
                break;
            }
            chain.push(cur);
            let Some(&parent) = parents.get(&cur) else { break };
            cur = parent;
        }
        chain
    }

    fn is_file_locked(&self, path: &Path) -> bool {
        // Windows locks are mandatory: a sharing violation on open means another process
        // holds the file without share access; a byte-range lock shows up as WouldBlock.
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                return matches!(
                    e.raw_os_error(),
                    Some(code) if code == ERROR_SHARING_VIOLATION.0 as i32 || code == ERROR_LOCK_VIOLATION.0 as i32
                );
            }
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

/// Walks up to 16 ancestor processes looking for `target_comm` (used by Claude Code hooks).
pub fn find_ancestor(target_comm: &str) -> Option<u32> {
    let mut pid = unsafe { GetCurrentProcessId() };
    for _ in 0..16 {
        let snap = snapshot();
        let entry = snap.iter().find(|e| e.pid == pid)?;
        if entry.matches(target_comm) {
            return Some(pid);
        }
        pid = entry.parent;
        if pid <= 4 {
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Open file paths via the system handle table.
// ---------------------------------------------------------------------------------------------

#[repr(C)]
struct SystemHandleEntry {
    object: usize,
    unique_process_id: usize,
    handle_value: usize,
    granted_access: u32,
    creator_back_trace_index: u16,
    object_type_index: u16,
    handle_attributes: u32,
    reserved: u32,
}

/// Enumerates the system handle table (`SystemExtendedHandleInformation`).
fn system_handles() -> Vec<SystemHandleEntry> {
    let mut cap = 1usize << 20;
    loop {
        let mut buf = vec![0u8; cap];
        let mut needed = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_EXTENDED_HANDLE_INFORMATION as i32,
                buf.as_mut_ptr().cast(),
                cap as u32,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && cap < (64 << 20) {
            cap *= 2;
            continue;
        }
        if status != 0 || buf.len() < 16 {
            return Vec::new();
        }
        let count = usize::from_le_bytes(buf[0..8].try_into().unwrap());
        let mut out = Vec::with_capacity(count.min(1 << 20));
        for i in 0..count {
            let off = 16 + i * core::mem::size_of::<SystemHandleEntry>();
            if off + core::mem::size_of::<SystemHandleEntry>() > buf.len() {
                break;
            }
            let e = unsafe { core::ptr::read_unaligned(buf.as_ptr().add(off).cast::<SystemHandleEntry>()) };
            out.push(e);
        }
        return out;
    }
}

/// Object type index of "File" objects, probed once from a handle this process owns.
fn file_type_index(handles: &[SystemHandleEntry]) -> Option<u16> {
    static IDX: OnceLock<u16> = OnceLock::new();
    if let Some(&i) = IDX.get() {
        return Some(i);
    }
    let probe = File::open(std::env::current_exe().ok()?).ok()?;
    let mine = probe.as_raw_handle() as usize;
    let pid = unsafe { GetCurrentProcessId() } as usize;
    let idx = handles
        .iter()
        .find(|e| e.unique_process_id == pid && e.handle_value == mine)
        .map(|e| e.object_type_index)?;
    let _ = IDX.set(idx);
    Some(idx)
}

type HandleKey = (u32, usize, usize); // (pid, handle value, object address)

/// One name-resolution job: duplicate the handle into this process and ask for its path.
fn resolve_name(pid: u32, handle: usize) -> Option<PathBuf> {
    let src = open(pid, PROCESS_DUP_HANDLE).ok()?;
    let mut dup = HANDLE::default();
    let ok = unsafe {
        DuplicateHandle(
            src,
            HANDLE(handle as *mut core::ffi::c_void),
            GetCurrentProcess(),
            &mut dup,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(src) };
    if !ok {
        return None;
    }
    let mut buf = [0u16; 1024];
    let n = unsafe {
        GetFinalPathNameByHandleW(
            dup,
            &mut buf,
            windows::Win32::Storage::FileSystem::GETFINALPATHNAMEBYHANDLE_FLAGS(0),
        )
    };
    let _ = unsafe { CloseHandle(dup) };
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    let raw = String::from_utf16_lossy(&buf[..n as usize]);
    Some(normalize_nt_path(&raw))
}

/// `\\?\C:\dir` -> `C:\dir`, `\\?\UNC\server\share` -> `\\server\share`.
fn normalize_nt_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// Name resolver running on a sacrificial thread: pipe handles can block forever inside
/// `GetFinalPathNameByHandle`, so a hung worker is abandoned (64 KiB stack) and its handle
/// blacklisted. Results are cached for the lifetime of the handle.
struct NameWorker {
    jobs: Sender<(u32, usize)>,
    results: Receiver<Option<PathBuf>>,
}

impl NameWorker {
    fn spawn() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<(u32, usize)>();
        let (res_tx, res_rx) = mpsc::channel::<Option<PathBuf>>();
        let _ = std::thread::Builder::new()
            .name("handle-name".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                while let Ok((pid, handle)) = job_rx.recv() {
                    if res_tx.send(resolve_name(pid, handle)).is_err() {
                        break;
                    }
                }
            });
        Self { jobs: job_tx, results: res_rx }
    }
}

#[derive(Default)]
struct PathCache {
    names: HashMap<HandleKey, Option<PathBuf>>,
    blacklist: HashSet<HandleKey>,
    worker: Option<NameWorker>,
    /// A name lookup blocked in the kernel; do not enqueue more work behind it.
    worker_stalled: bool,
}

static PATHS: Mutex<Option<PathCache>> = Mutex::new(None);

/// Open DOS file paths per PID, best effort: unreadable or hung handles are skipped and the
/// sources fall back to cwd-based session matching.
fn open_paths_for(pids: &[u32]) -> HashMap<u32, Vec<PathBuf>> {
    let mut out: HashMap<u32, Vec<PathBuf>> = HashMap::new();
    let handles = system_handles();
    let Some(file_idx) = file_type_index(&handles) else {
        return out;
    };
    let wanted: HashSet<usize> = pids.iter().map(|&p| p as usize).collect();
    let mut guard = PATHS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = guard.get_or_insert_with(PathCache::default);
    if cache.names.len() > 32_768 {
        cache.names.clear();
        cache.blacklist.clear();
    }
    let mut queries = 0usize;
    let started = Instant::now();
    // Bound the total time spent resolving names in one UI refresh. A single hung
    // kernel lookup poisons this worker for the process lifetime; do not replace it
    // with another thread or queue additional work behind it.
    const MAX_NAME_QUERIES: usize = 64;
    const NAME_BUDGET: Duration = Duration::from_millis(750);
    for e in &handles {
        if e.object_type_index != file_idx || !wanted.contains(&e.unique_process_id) {
            continue;
        }
        let pid = e.unique_process_id as u32;
        let key: HandleKey = (pid, e.handle_value, e.object);
        if cache.blacklist.contains(&key) {
            continue;
        }
        let name = if let Some(cached) = cache.names.get(&key) {
            cached.clone()
        } else {
            if cache.worker_stalled || queries >= MAX_NAME_QUERIES || started.elapsed() >= NAME_BUDGET {
                continue;
            }
            queries += 1;
            let worker = cache.worker.get_or_insert_with(NameWorker::spawn);
            if worker.jobs.send((pid, e.handle_value)).is_err() {
                cache.worker = None;
                continue;
            }
            match worker.results.recv_timeout(Duration::from_millis(250)) {
                Ok(name) => {
                    cache.names.insert(key, name.clone());
                    name
                }
                Err(_) => {
                    // Worker is stuck inside the kernel on a pipe handle. Keep the
                    // sender (and its single worker) but permanently stop queueing jobs.
                    // Dropping/replacing it would leak one blocked OS thread per timeout.
                    cache.blacklist.insert(key);
                    cache.worker_stalled = true;
                    continue;
                }
            }
        };
        if let Some(p) = name {
            out.entry(pid).or_default().push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_nt_device_paths() {
        assert_eq!(normalize_nt_path(r"\\?\C:\Users\dev\proj"), PathBuf::from(r"C:\Users\dev\proj"));
        assert_eq!(normalize_nt_path(r"\\?\UNC\srv\share\f"), PathBuf::from(r"\\srv\share\f"));
        assert_eq!(normalize_nt_path(r"C:\plain"), PathBuf::from(r"C:\plain"));
    }

    #[test]
    fn maps_npm_entry_points_to_agent_names() {
        assert_eq!(
            script_stem(r#""C:\Program Files\nodejs\node.exe" "C:\Users\dev\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js""#).as_deref(),
            Some("claude")
        );
        assert_eq!(
            script_stem(r#"node.exe C:\npm\node_modules\@openai\codex\bin\codex.js"#).as_deref(),
            Some("codex")
        );
        assert_eq!(script_stem(r#"node C:\tools\my-script.js"#).as_deref(), Some("my-script"));
    }

    #[test]
    fn matches_names_case_insensitively_and_ignores_exe_suffix() {
        let e = ProcEntry { pid: 1, parent: 0, stem: "claude".into(), script: None };
        assert!(e.matches("claude"));
        assert!(e.matches("Claude.exe"));
        let node = ProcEntry { pid: 2, parent: 0, stem: "node".into(), script: Some("claude".into()) };
        assert!(node.matches("claude"));
        assert!(!node.matches("node2"));
    }

    #[test]
    fn lists_own_process_and_ancestors() {
        let table = WinProcTable;
        let me = std::process::id();
        assert!(table.is_alive(me, None));
        assert!(!table.is_alive(0, None));
        let chain = table.ancestor_pids(me);
        assert_eq!(chain.first(), Some(&me));
    }
}
