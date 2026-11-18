// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! agents-roster — floating panel and tray icon showing live Claude Code and agy agents.
//!
//!     agents-roster                               opens the panel (default)
//!     agents-roster status [--dir D]              prints the current snapshot as JSON (diagnostics)
//!     agents-roster collect <Event> --dir D       Claude Code hook (reads JSON from stdin)
//!     agents-roster statusline --dir D            Claude Code statusline (reads JSON from stdin)
//!     agents-roster --version                     prints the release version and commit
//!     agents-roster about                         prints authorship, license and provenance
//!
//! `--dir` is the exchange directory between hooks and the panel.
//! Default: `$AGENTS_ROSTER_DIR`, or `/run/user/<uid>/agents-roster`.

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
use agents_roster_notify::KdeNotifier;
use agents_roster_sources::{
    AgySource, ChronoTimeFormatter, ClaudeCodeSource, CodexSource, GenericProcessSource,
    GitReflogInfo, ProcFsTable, SystemClock, current_uid, detect_installed_services, expand_tilde,
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
    PathBuf::from(format!("/run/user/{}/agents-roster", current_uid()))
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
    let procs = Arc::new(ProcFsTable);
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

/// `$HOME`, falling back to the filesystem root when unset.
fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
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

fn run_gui(dir: PathBuf) -> Result<()> {
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
    let notifier = Box::new(KdeNotifier::new(app_name.into(), "agents-roster".into()));
    let opener: Arc<dyn TerminalOpener> =
        Arc::new(SystemTerminalOpener::new(Arc::new(ProcFsTable)));
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
    )?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = exchange_dir(&args);
    match args.get(1).map(String::as_str) {
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
        None | Some("--dir") => {
            if let Err(e) = run_gui(dir) {
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
