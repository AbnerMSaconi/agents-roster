// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! agents-roster — floating panel and tray icon showing live Claude Code and agy agents.
//!
//!     agents-roster                               opens the panel (default)
//!     agents-roster --settings                    opens the panel and the Control Panel window
//!     agents-roster status [--dir D]              prints the current snapshot as JSON (diagnostics)
//!     agents-roster collect <Event> --dir D       Claude Code hook (reads JSON from stdin)
//!     agents-roster statusline --dir D            Claude Code statusline (reads JSON from stdin)
//!     agents-roster --version                     prints the release version and commit
//!     agents-roster about                         prints authorship, license and provenance
//!
//! `--dir` is the exchange directory between hooks and the panel.
//! Default: `$AGENTS_ROSTER_DIR`, or `/run/user/<uid>/agents-roster` on Linux and
//! `%LOCALAPPDATA%\agents-roster\run` on Windows.

// GUI subsystem on Windows: no console window behind the panel. CLI commands re-attach to
// the parent console (or keep the piped std handles Claude Code hands to hooks).
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;

/// Machine-greppable provenance record embedded in the binary
/// (`strings agents-roster | grep agents-roster-provenance`).
static PROVENANCE: &str = concat!(
    "agents-roster-provenance;name=agents-roster;author=AbnerMSaconi;license=AGPL-3.0-only;",
    "repo=https://github.com/AbnerMSaconi/agents-roster;version=",
    env!("CARGO_PKG_VERSION"),
    ";commit=",
    env!("AGENTS_ROSTER_COMMIT")
);

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agents_roster_core::{
    AgentSource, PanelSettings, ServiceEntry, ServiceStatus, SnapshotCollector, TerminalOpener,
    Translator,
};
use agents_roster_notify::DesktopNotifier;
use agents_roster_sources::{
    AgySource, ChronoTimeFormatter, ClaudeCodeSource, CodexSource, GenericProcessSource,
    GitReflogInfo, SystemClock, SystemProcessTable, detect_installed_services, expand_tilde,
    handle_hook, handle_statusline,
};
use agents_roster_ui::{PoTranslator, SettingsBackend, SystemTerminalOpener, run_panel};
use anyhow::Result;

use config::{AppConfig, TomlConfigStore};

fn exchange_dir(args: &[String]) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--dir")
        && let Some(d) = args.get(i + 1)
    {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("AGENTS_ROSTER_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map_or_else(|| PathBuf::from(std::env::var("TEMP").unwrap_or_default()), PathBuf::from);
        return base.join("agents-roster").join("run");
    }
    #[cfg(not(windows))]
    PathBuf::from(format!("/run/user/{}/agents-roster", agents_roster_sources::current_uid()))
}

struct HostBackend {
    dir: PathBuf,
    home: PathBuf,
    store: Arc<TomlConfigStore>,
}

impl SettingsBackend for HostBackend {
    fn detect_services(&self, services: &[ServiceEntry]) -> Vec<ServiceStatus> {
        detect_installed_services(&self.home, services)
    }

    fn rebuild_collector(&self, settings: &PanelSettings) -> SnapshotCollector {
        build_collector_from_settings(&self.dir, &self.home, settings)
    }

    fn save_settings(&self, settings: &PanelSettings) {
        self.store.save_panel_settings(settings);
    }
}

fn build_collector_from_settings(
    dir: &Path,
    home: &Path,
    settings: &PanelSettings,
) -> SnapshotCollector {
    let procs = Arc::new(SystemProcessTable);
    let git = Arc::new(GitReflogInfo);
    let clock = Arc::new(SystemClock);
    let fmt = Arc::new(ChronoTimeFormatter);
    let mut sources: Vec<Box<dyn AgentSource>> = Vec::new();
    for svc in settings.services.iter().filter(|s| s.enabled) {
        let custom_dir =
            (!svc.data_dir.trim().is_empty()).then(|| expand_tilde(home, &svc.data_dir));
        match svc.id.as_str() {
            "claude" => sources.push(Box::new(ClaudeCodeSource::new(
                custom_dir.unwrap_or_else(|| dir.to_path_buf()),
                procs.clone(),
                git.clone(),
                clock.clone(),
            ))),
            "agy" => sources.push(Box::new(AgySource::new(
                custom_dir.unwrap_or_else(|| home.join(".gemini/antigravity-cli")),
                settings.agy_context_limit,
                procs.clone(),
                git.clone(),
                clock.clone(),
                fmt.clone(),
            ))),
            "codex" => sources.push(Box::new(CodexSource::new(
                custom_dir.unwrap_or_else(|| home.join(".codex")),
                procs.clone(),
                git.clone(),
                clock.clone(),
            ))),
            _ => sources.push(Box::new(GenericProcessSource::new(
                svc.id.clone(),
                svc.label.clone(),
                svc.process_name.clone(),
                custom_dir.unwrap_or_default(),
                procs.clone(),
                clock.clone(),
            ))),
        }
    }
    SnapshotCollector::with_processes(sources, procs)
}

/// `$HOME` (Unix) or `%USERPROFILE%` (Windows), falling back to the filesystem root.
fn home_dir() -> PathBuf {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var_os(key).map_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR), PathBuf::from)
}

fn build_collector(dir: &Path, settings: &PanelSettings) -> SnapshotCollector {
    let home = home_dir();
    build_collector_from_settings(dir, &home, settings)
}

/// Takes an exclusive lock so only one panel runs per exchange directory.
///
/// Returns the held lock file (keep it alive for the whole process), or `None` when another
/// instance already owns it.
fn acquire_single_instance(dir: &Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(dir).ok();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("panel.lock"))
        .ok()?;
    // Only an actively held lock blocks startup; if locking is unsupported, run anyway.
    match file.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => None,
        _ => Some(file),
    }
}

fn run_gui(dir: PathBuf, open_settings: bool) -> Result<()> {
    let Some(_instance_lock) = acquire_single_instance(&dir) else {
        eprintln!("agents-roster: already running");
        return Ok(());
    };
    let cfg = AppConfig::load();
    let initial = (&cfg).into();
    let initial_settings = PanelSettings::from(&cfg);
    let home = home_dir();
    let collector = build_collector_from_settings(&dir, &home, &initial_settings);
    let store = Arc::new(TomlConfigStore::new(cfg));
    let backend: Arc<dyn SettingsBackend> = Arc::new(HostBackend {
        dir,
        home,
        store: store.clone(),
    });
    let tr = PoTranslator::from_env();
    let app_name = if tr.translate("Agents") == "Agentes" {
        "Agentes"
    } else {
        "Agents"
    };
    let notifier = Box::new(DesktopNotifier::new(app_name.into(), "agents-roster".into()));
    let opener: Arc<dyn TerminalOpener> =
        Arc::new(SystemTerminalOpener::new(Arc::new(SystemProcessTable)));
    let clock = Arc::new(SystemClock);
    let fmt = Arc::new(ChronoTimeFormatter);
    run_panel(
        initial,
        initial_settings,
        store,
        backend,
        collector,
        notifier,
        &opener,
        clock,
        fmt,
        tr,
        open_settings,
    )?;
    Ok(())
}

/// Re-attaches stdout/stderr to the parent console for CLI use under the GUI subsystem.
/// Skipped when std handles are already valid (piped hooks, redirected output).
#[cfg(windows)]
#[allow(unsafe_code)] // Audited Win32 console calls.
fn attach_parent_console() {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    use windows::core::w;
    unsafe {
        if let Ok(h) = GetStdHandle(STD_OUTPUT_HANDLE)
            && !h.is_invalid()
            && !h.0.is_null()
        {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        for std_id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if let Ok(h) = CreateFileW(
                w!("CONOUT$"),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            ) {
                let _ = SetStdHandle(std_id, h);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = exchange_dir(&args);
    let cmd = args.get(1).map(String::as_str);
    #[cfg(windows)]
    if !matches!(cmd, None | Some("--dir" | "--settings")) {
        attach_parent_console();
    }
    match cmd {
        Some("collect") => {
            if let Some(ev) = args.get(2) {
                handle_hook(ev, &dir);
            }
        }
        Some("statusline") => println!("{}", handle_statusline(&dir)),
        Some("status") => {
            let cfg = AppConfig::load();
            let tr = PoTranslator::from_env();
            let mut collector = build_collector(&dir, &PanelSettings::from(&cfg));
            let snap = collector.collect(&tr);
            println!(
                "{}",
                serde_json::to_string_pretty(&snap).unwrap_or_default()
            );
        }
        Some("-V" | "--version" | "version") => {
            println!(
                "agents-roster {} (commit {})",
                env!("CARGO_PKG_VERSION"),
                env!("AGENTS_ROSTER_COMMIT")
            );
        }
        Some("about") => println!(
            "{}",
            agents_roster_core::about::full_notice(
                env!("CARGO_PKG_VERSION"),
                env!("AGENTS_ROSTER_COMMIT"),
                std::hint::black_box(PROVENANCE),
            )
        ),
        Some("-h" | "--help" | "help") => {
            let help = include_str!("main.rs")
                .lines()
                .skip_while(|l| !l.starts_with("//!"))
                .take_while(|l| l.starts_with("//!"))
                .map(|l| l.trim_start_matches("//!"))
                .collect::<Vec<_>>()
                .join("\n");
            println!("{help}");
        }
        None | Some("--dir" | "--settings") => {
            let open_settings = args.iter().any(|a| a == "--settings");
            if let Err(e) = run_gui(dir, open_settings) {
                eprintln!("agents-roster: {e}");
                std::process::exit(1);
            }
        }
        // An unknown command (e.g. a stale hook) must never open a window. Exit 1, not 2: Claude Code
        // treats a hook exiting with 2 as a blocking error and feeds its stderr to the model.
        Some(other) => {
            eprintln!("agents-roster: unknown command `{other}` (try --help)");
            std::process::exit(1);
        }
    }
}
