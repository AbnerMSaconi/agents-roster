// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Slint window event loop and background collection/notification loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use agents_roster_core::{
    Agent, Clock, NotificationEngine, Notifier, PanelSettings, SnapshotCollector, TerminalOpener,
    TimeFormatter, Translator,
};
use ksni::blocking::{Handle as TrayHandle, TrayMethods};
use slint::{ComponentHandle, ModelRc, PhysicalPosition, VecModel};
use thiserror::Error;

use crate::callbacks::update_ui;
use crate::commands::handle_command;
use crate::i18n::PoTranslator;
use crate::settings_ui::SettingsBackend;
use crate::theme::Palette;
use crate::tray::{TrayCommand, TrayIcon};
use crate::view_model::{PanelView, build_panel_view, build_tray_view};
use crate::x11_hints;

slint::include_modules!();

/// Window and polling settings passed to the UI layer.
#[derive(Debug, Clone, Copy)]
pub struct PanelConfig {
    pub x: i32,
    pub y: i32,
    pub always_on_top: bool,
    pub visible: bool,
    pub hide_from_taskbar: bool,
    pub reminder_min: u64,
    pub interval_s: u64,
}

/// Persistence hook for window position and menu toggles.
pub trait ConfigStore: Send + Sync {
    fn save(&self, cfg: &PanelConfig);
}

/// Errors returned while running the Slint event loop.
#[derive(Debug, Error)]
pub enum UiError {
    #[error("Slint platform error: {0}")]
    Platform(#[from] slint::PlatformError),
    #[error("failed to spawn collector thread: {0}")]
    Thread(#[from] std::io::Error),
}

pub(crate) struct LoopDeps {
    pub collector: SnapshotCollector,
    pub notifier: Box<dyn Notifier>,
    pub clock: Arc<dyn Clock>,
    pub fmt: Arc<dyn TimeFormatter>,
    pub tr: Arc<dyn Translator>,
    pub store: Arc<dyn ConfigStore>,
    pub backend: Arc<dyn SettingsBackend>,
    pub cfg: Arc<Mutex<PanelConfig>>,
    pub settings: Arc<Mutex<PanelSettings>>,
    pub palette: Arc<Mutex<Palette>>,
    pub tx: mpsc::Sender<TrayCommand>,
    pub visible: Arc<AtomicBool>,
    pub reminders: Arc<AtomicBool>,
    pub hide_from_taskbar: Arc<AtomicBool>,
    pub blur_enabled: Arc<AtomicBool>,
    pub window_pending: Arc<AtomicBool>,
    pub latest_agents: Arc<Mutex<Vec<Agent>>>,
    pub panel: slint::Weak<Panel>,
}

/// Runs the Slint floating panel and background collector until quit.
///
/// # Errors
/// Returns [`UiError`] if Slint fails to initialize or the collector thread cannot be spawned.
#[allow(clippy::too_many_arguments)]
pub fn run_panel(
    initial: PanelConfig,
    initial_settings: PanelSettings,
    store: Arc<dyn ConfigStore>,
    backend: Arc<dyn SettingsBackend>,
    collector: SnapshotCollector,
    notifier: Box<dyn Notifier>,
    opener: &Arc<dyn TerminalOpener>,
    clock: Arc<dyn Clock>,
    fmt: Arc<dyn TimeFormatter>,
    tr: PoTranslator,
    open_settings: bool,
) -> Result<(), UiError> {
    x11_hints::configure_backend(initial.hide_from_taskbar);
    let panel = Panel::new()?;
    tr.apply_to_slint();
    let pal = Palette::from_config(&initial_settings.theme, &initial_settings.services);
    pal.apply_to_panel(&panel);
    panel.set_keep_on_top(initial.always_on_top);
    panel.set_reminders(initial.reminder_min > 0);
    panel
        .window()
        .set_position(PhysicalPosition::new(initial.x, initial.y));

    let visible = Arc::new(AtomicBool::new(initial.visible));
    let reminders = Arc::new(AtomicBool::new(initial.reminder_min > 0));
    let hide_tb = Arc::new(AtomicBool::new(initial.hide_from_taskbar));
    let blur_flag = Arc::new(AtomicBool::new(initial_settings.theme.blur));
    // Window hints are re-applied once the X11 window is actually mapped.
    let window_pending = Arc::new(AtomicBool::new(initial.visible));
    let cfg = Arc::new(Mutex::new(initial));
    let settings = Arc::new(Mutex::new(initial_settings));
    let palette = Arc::new(Mutex::new(pal));
    let latest_agents = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel::<TrayCommand>();

    let top_cfg = Arc::clone(&cfg);
    x11_hints::install_window_hooks(
        &panel,
        Arc::new(move || top_cfg.lock().map_or(true, |c| c.always_on_top)),
        hide_tb.clone(),
        blur_flag.clone(),
        window_pending.clone(),
    );
    crate::callbacks::wire_panel_callbacks(&panel, &cfg, &store, &latest_agents, opener, &tx);
    let deps = LoopDeps {
        collector,
        notifier,
        clock,
        fmt,
        tr: Arc::new(tr),
        store,
        backend,
        cfg,
        settings,
        palette,
        tx: tx.clone(),
        visible: visible.clone(),
        reminders,
        hide_from_taskbar: hide_tb,
        blur_enabled: blur_flag,
        window_pending,
        latest_agents,
        panel: panel.as_weak(),
    };
    if open_settings {
        let _ = tx.send(TrayCommand::OpenSettings);
    }
    std::thread::Builder::new()
        .name("collector".into())
        .stack_size(512 * 1024)
        .spawn(move || run_collector_thread(deps, initial, tx, &rx))?;

    if visible.load(Ordering::Relaxed) {
        panel.show()?;
    }
    slint::run_event_loop_until_quit()?;
    Ok(())
}

fn run_collector_thread(
    mut deps: LoopDeps,
    initial: PanelConfig,
    tx: mpsc::Sender<TrayCommand>,
    rx: &mpsc::Receiver<TrayCommand>,
) {
    let tray = TrayIcon {
        tx,
        tr: deps.tr.clone(),
        lines: Vec::new(),
        working_count: 0,
        color: [0x8a; 3],
        always_on_top: initial.always_on_top,
        reminders: initial.reminder_min > 0,
    };
    let handle = tray.spawn().ok();
    let mut engine = NotificationEngine::default();
    if let Ok(s) = deps.settings.lock() {
        engine.set_prefs(s.notifications.clone());
    }
    let interval = Duration::from_secs(initial.interval_s.max(1));
    let mut next_tick = Instant::now();
    let mut prev_ui: Option<PanelView> = None;

    loop {
        let wait = next_tick.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(cmd) => {
                if !handle_command(
                    cmd,
                    &mut deps,
                    &mut engine,
                    handle.as_ref(),
                    &mut prev_ui,
                    &mut next_tick,
                ) {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                next_tick = Instant::now() + interval;
                on_tick(&mut deps, &mut engine, handle.as_ref(), &mut prev_ui);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn on_tick(
    deps: &mut LoopDeps,
    engine: &mut NotificationEngine,
    handle: Option<&TrayHandle<TrayIcon>>,
    prev_ui: &mut Option<PanelView>,
) {
    let now_ms = deps.clock.now_ms();
    let snap = deps.collector.collect(deps.tr.as_ref());
    if let Ok(mut list) = deps.latest_agents.lock() {
        (*list).clone_from(&snap.agents);
    }
    let rem_min = if deps.reminders.load(Ordering::Relaxed) {
        deps.cfg.lock().map(|c| c.reminder_min.max(1)).unwrap_or(15)
    } else {
        0
    };
    for note in engine.process(&snap, now_ms, rem_min, deps.fmt.as_ref(), deps.tr.as_ref()) {
        if let Err(e) = deps.notifier.notify(&note) {
            tracing::warn!("{e}");
        }
    }
    if let Some(h) = handle {
        let tray_view = build_tray_view(&snap, now_ms, deps.fmt.as_ref(), deps.tr.as_ref());
        h.update(move |b: &mut TrayIcon| {
            b.lines = tray_view.lines;
            b.working_count = tray_view.working_count;
            b.color = tray_view.color;
        });
    }
    if !deps.visible.load(Ordering::Relaxed) {
        return;
    }
    let mut next_ui = build_panel_view(&snap, now_ms, deps.fmt.as_ref(), deps.tr.as_ref());
    if let Ok(pal) = deps.palette.lock() {
        pal.recolor_view(&mut next_ui, &snap.agents);
    }
    if prev_ui.as_ref() == Some(&next_ui) {
        return;
    }
    *prev_ui = Some(next_ui.clone());
    update_ui(&deps.panel, move |p| {
        p.set_agents(ModelRc::new(VecModel::from(next_ui.agents)));
        p.set_footers(ModelRc::new(VecModel::from(next_ui.footers)));
    });
}
