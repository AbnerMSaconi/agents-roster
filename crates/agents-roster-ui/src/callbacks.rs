// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Slint window drag/drop, agent-click, and context-menu callback wiring.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use agents_roster_core::{Agent, TerminalOpener};
use slint::{ComponentHandle, PhysicalPosition};

use crate::x11_hints::{is_wayland, start_native_move};

use crate::app::{ConfigStore, Panel, PanelConfig};
use crate::tray::TrayCommand;

pub(crate) fn update_ui(panel: &slint::Weak<Panel>, f: impl FnOnce(Panel) + Send + 'static) {
    let w = panel.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(p) = w.upgrade() {
            f(p);
        }
    });
}

pub(crate) fn wire_panel_callbacks(
    panel: &Panel,
    cfg: &Arc<Mutex<PanelConfig>>,
    store: &Arc<dyn ConfigStore>,
    agents: &Arc<Mutex<Vec<Agent>>>,
    opener: &Arc<dyn TerminalOpener>,
    tx: &mpsc::Sender<TrayCommand>,
) {
    let w = panel.as_weak();
    panel.on_drag(move |dx, dy| {
        if let Some(p) = w.upgrade() {
            if start_native_move(&p) {
                return;
            }
            let s = p.window().scale_factor();
            let pos = p.window().position();
            p.window().set_position(PhysicalPosition::new(
                pos.x + (dx * s) as i32,
                pos.y + (dy * s) as i32,
            ));
        }
    });
    let (w, cfg_c, store_c) = (panel.as_weak(), cfg.clone(), store.clone());
    panel.on_drop_window(move || {
        if let (Some(p), Ok(mut c)) = (w.upgrade(), cfg_c.lock()) {
            // Wayland never reports a global position, so there is nothing to persist.
            if is_wayland(&p) {
                return;
            }
            let pos = p.window().position();
            (c.x, c.y) = (pos.x, pos.y);
            store_c.save(&c);
        }
    });
    let (agents_c, opener_c) = (agents.clone(), opener.clone());
    panel.on_open_agent(move |key| {
        let found = agents_c
            .lock()
            .ok()
            .and_then(|list| list.iter().find(|a| a.key == key.as_str()).cloned());
        if let Some(agent) = found {
            let opener = opener_c.clone();
            std::thread::spawn(move || opener.open_agent_terminal(&agent));
        }
    });
    wire_menu_callbacks(panel, tx);
}

fn wire_menu_callbacks(panel: &Panel, tx: &mpsc::Sender<TrayCommand>) {
    let (t1, t2, t3, t4, t5) = (tx.clone(), tx.clone(), tx.clone(), tx.clone(), tx.clone());
    let (w2, w3) = (panel.as_weak(), panel.as_weak());
    panel.on_toggle_visible(move || {
        let _ = t1.send(TrayCommand::ToggleVisible);
    });
    panel.on_toggle_top(move || {
        if let Some(p) = w2.upgrade() {
            let _ = t2.send(TrayCommand::AlwaysOnTop(!p.get_keep_on_top()));
        }
    });
    panel.on_toggle_reminders(move || {
        if let Some(p) = w3.upgrade() {
            let _ = t3.send(TrayCommand::Reminders(!p.get_reminders()));
        }
    });
    panel.on_open_settings(move || {
        let _ = t4.send(TrayCommand::OpenSettings);
    });
    panel.on_quit(move || {
        let _ = t5.send(TrayCommand::Quit);
    });
}
