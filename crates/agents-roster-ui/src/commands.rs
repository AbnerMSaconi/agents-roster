// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Command dispatcher for tray menu, panel context menu, and Control Center live updates.

use std::sync::atomic::Ordering;
use std::time::Instant;

use agents_roster_core::{NotificationEngine, PanelSettings};
use slint::ComponentHandle;

use crate::app::{LoopDeps, PanelConfig};
use crate::callbacks::update_ui;
use crate::settings_ui::open_settings_window;
use crate::theme::Palette;
use crate::tray::{TrayCommand, TrayHandle, TrayIcon};
use crate::view_model::PanelView;
use crate::window_hints::{apply_blur_behind, apply_skip_taskbar, refresh_compositor};

pub(crate) fn handle_command(
    cmd: TrayCommand,
    deps: &mut LoopDeps,
    engine: &mut NotificationEngine,
    handle: Option<&TrayHandle>,
    prev_ui: &mut Option<PanelView>,
    next_tick: &mut Instant,
) -> bool {
    match cmd {
        TrayCommand::ToggleVisible => toggle_visible(deps, prev_ui, next_tick),
        TrayCommand::AlwaysOnTop(top) => {
            if let Some(h) = handle {
                h.update(move |b: &mut TrayIcon| b.always_on_top = top);
            }
            save_cfg(deps, |c| c.always_on_top = top);
            update_ui(&deps.panel, move |p| {
                p.set_keep_on_top(top);
                crate::window_hints::apply_always_on_top(&p, top);
            });
        }
        TrayCommand::Reminders(enabled) => {
            deps.reminders.store(enabled, Ordering::Relaxed);
            if let Some(h) = handle {
                h.update(move |b: &mut TrayIcon| b.reminders = enabled);
            }
            save_cfg(deps, |c| {
                c.reminder_min = if enabled { c.reminder_min.max(15) } else { 0 };
            });
            update_ui(&deps.panel, move |p| p.set_reminders(enabled));
        }
        TrayCommand::OpenSettings => {
            let (s, b, tx) = (deps.settings.clone(), deps.backend.clone(), deps.tx.clone());
            let _ = slint::invoke_from_event_loop(move || {
                open_settings_window(&s, &b, &tx);
            });
        }
        TrayCommand::ApplySettings(new_cfg) => {
            apply_new_settings(deps, engine, handle, prev_ui, next_tick, &new_cfg);
        }
        TrayCommand::Quit => {
            let _ = slint::quit_event_loop();
            return false;
        }
    }
    true
}

fn toggle_visible(deps: &LoopDeps, prev_ui: &mut Option<PanelView>, next_tick: &mut Instant) {
    let show = !deps.visible.load(Ordering::Relaxed);
    deps.visible.store(show, Ordering::Relaxed);
    *prev_ui = None;
    *next_tick = Instant::now();
    save_cfg(deps, |c| c.visible = show);
    if show {
        deps.window_pending.store(true, Ordering::Release);
    }
    update_ui(&deps.panel, move |p| {
        if show {
            let _ = p.show();
        } else {
            let _ = p.hide();
        }
    });
}

fn apply_new_settings(
    deps: &mut LoopDeps,
    engine: &mut NotificationEngine,
    handle: Option<&TrayHandle>,
    prev_ui: &mut Option<PanelView>,
    next_tick: &mut Instant,
    new_cfg: &PanelSettings,
) {
    deps.collector = deps.backend.rebuild_collector(new_cfg);
    deps.backend.save_settings(new_cfg);
    engine.set_prefs(new_cfg.notifications.clone());
    // Window hints are only re-sent when the value actually changed.
    let hide_changed = deps
        .hide_from_taskbar
        .swap(new_cfg.hide_from_taskbar, Ordering::Relaxed)
        != new_cfg.hide_from_taskbar;
    let blur_changed = deps
        .blur_enabled
        .swap(new_cfg.theme.blur, Ordering::Relaxed)
        != new_cfg.theme.blur;
    deps.reminders
        .store(new_cfg.reminder_min > 0, Ordering::Relaxed);
    let vis_changed = deps.visible.swap(new_cfg.visible, Ordering::Relaxed) != new_cfg.visible;
    let pal = Palette::from_config(&new_cfg.theme, &new_cfg.services);
    if let Ok(mut slot) = deps.palette.lock() {
        *slot = pal.clone();
    }
    if let Ok(mut s) = deps.settings.lock() {
        *s = new_cfg.clone();
    }
    if let Ok(mut c) = deps.cfg.lock() {
        c.visible = new_cfg.visible;
        c.always_on_top = new_cfg.always_on_top;
        c.hide_from_taskbar = new_cfg.hide_from_taskbar;
        c.reminder_min = new_cfg.reminder_min;
    }
    if let Some(h) = handle {
        let (top, rem) = (new_cfg.always_on_top, new_cfg.reminder_min > 0);
        h.update(move |b: &mut TrayIcon| {
            b.always_on_top = top;
            b.reminders = rem;
        });
    }
    let (blur, top, rem, hide, show) = (
        new_cfg.theme.blur,
        new_cfg.always_on_top,
        new_cfg.reminder_min > 0,
        new_cfg.hide_from_taskbar,
        new_cfg.visible,
    );
    if show {
        deps.window_pending.store(true, Ordering::Release);
    }
    update_ui(&deps.panel, move |p| {
        pal.apply_to_panel(&p);
        p.set_keep_on_top(top);
        crate::window_hints::apply_always_on_top(&p, top);
        p.set_reminders(rem);
        if vis_changed {
            if show {
                let _ = p.show();
            } else {
                let _ = p.hide();
            }
        }
        if hide_changed {
            apply_skip_taskbar(&p, hide);
        }
        if blur_changed {
            apply_blur_behind(&p, blur);
            refresh_compositor(&p);
        }
    });
    *prev_ui = None;
    *next_tick = Instant::now();
}

fn save_cfg(deps: &LoopDeps, mutate: impl FnOnce(&mut PanelConfig)) {
    if let Ok(mut c) = deps.cfg.lock() {
        mutate(&mut c);
        deps.store.save(&c);
    }
}
