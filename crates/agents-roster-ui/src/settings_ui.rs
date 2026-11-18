// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Lazy Control Panel window (`SettingsWindow`) with instant live apply.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, mpsc};

use agents_roster_core::{
    PALETTE_SWATCHES, PanelSettings, ServiceEntry, ServiceStatus, SnapshotCollector,
    add_custom_service, apply_theme_preset, format_hex_rgb, merge_default_services, set_theme_hex,
};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::SettingsWindow;
use crate::settings_sync::{
    apply_bool_setting, apply_int_setting, sync_active_color, sync_service_rows,
    sync_settings_window,
};
use crate::theme::hex_to_color;
use crate::tray::TrayCommand;

thread_local! {
    static SETTINGS_WIN: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
}

/// Host capability boundary for service detection, collector rebuilds, and persistence.
pub trait SettingsBackend: Send + Sync {
    /// Probes which configured services are installed on the machine.
    fn detect_services(&self, services: &[ServiceEntry]) -> Vec<ServiceStatus>;
    /// Rebuilds the [`SnapshotCollector`] cleanly when enabled services or paths change.
    fn rebuild_collector(&self, settings: &PanelSettings) -> SnapshotCollector;
    /// Persists full panel/theme/notification/service settings to disk.
    fn save_settings(&self, settings: &PanelSettings);
}

#[derive(Clone)]
struct UiCtx {
    weak: slint::Weak<SettingsWindow>,
    settings: Arc<Mutex<PanelSettings>>,
    statuses: Arc<Mutex<Vec<ServiceStatus>>>,
    backend: Arc<dyn SettingsBackend>,
    tx: mpsc::Sender<TrayCommand>,
}

impl UiCtx {
    fn mutate(&self, rescan: bool, rebuild_list: bool, f: impl FnOnce(&mut PanelSettings)) {
        let Some(snap) = self.settings.lock().ok().map(|mut guard| {
            f(&mut guard);
            guard.clone()
        }) else {
            return;
        };
        if rescan {
            let scanned = self.backend.detect_services(&snap.services);
            if let Ok(mut st) = self.statuses.lock() {
                *st = scanned;
            }
        }
        let st = self.statuses.lock().map(|g| g.clone()).unwrap_or_default();
        if let Some(w) = self.weak.upgrade() {
            if rebuild_list {
                sync_settings_window(&w, &snap, &st);
            } else {
                // Keep TextInput focus while editing service CLI or directory paths.
                let installed = st.iter().filter(|s| s.installed).count();
                w.set_services_summary(
                    format!("{installed}/{} installed", snap.services.len()).into(),
                );
            }
        }
        let _ = self.tx.send(TrayCommand::ApplySettings(Box::new(snap)));
    }
}

fn close_settings_window() {
    SETTINGS_WIN.with(|slot| {
        if let Some(w) = slot.borrow_mut().take() {
            let _ = w.hide();
        }
    });
}

/// Opens (or raises) the lazy Control Center window on the Slint UI thread.
pub(crate) fn open_settings_window(
    settings: &Arc<Mutex<PanelSettings>>,
    backend: &Arc<dyn SettingsBackend>,
    tx: &mpsc::Sender<TrayCommand>,
) {
    SETTINGS_WIN.with(|slot| {
        if let Some(win) = slot.borrow().as_ref() {
            if let Ok(cfg) = settings.lock() {
                let st = backend.detect_services(&cfg.services);
                sync_settings_window(win, &cfg, &st);
            }
            let _ = win.show();
            return;
        }
        let Ok(win) = SettingsWindow::new() else {
            return;
        };
        init_palette(&win);
        let st = settings
            .lock()
            .map(|c| {
                let scanned = backend.detect_services(&c.services);
                sync_settings_window(&win, &c, &scanned);
                scanned
            })
            .unwrap_or_default();
        let ctx = UiCtx {
            weak: win.as_weak(),
            settings: settings.clone(),
            statuses: Arc::new(Mutex::new(st)),
            backend: backend.clone(),
            tx: tx.clone(),
        };
        wire_callbacks(&win, &ctx);
        let _ = win.show();
        *slot.borrow_mut() = Some(win);
    });
}

fn init_palette(w: &SettingsWindow) {
    let swatches: Vec<Color> = PALETTE_SWATCHES
        .iter()
        .map(|h| hex_to_color(h, 1.0, [0xff, 0xff, 0xff]))
        .collect();
    let hexes: Vec<SharedString> = PALETTE_SWATCHES.iter().map(|&h| h.into()).collect();
    w.set_palette_swatches(ModelRc::new(VecModel::from(swatches)));
    w.set_palette_hexes(ModelRc::new(VecModel::from(hexes)));
}

fn wire_callbacks(win: &SettingsWindow, ctx: &UiCtx) {
    wire_general_and_service_callbacks(win, ctx);
    wire_color_callbacks(win, ctx);
    win.on_close_settings(close_settings_window);
    win.window().on_close_requested(|| {
        close_settings_window();
        slint::CloseRequestResponse::HideWindow
    });
}

fn wire_general_and_service_callbacks(win: &SettingsWindow, base: &UiCtx) {
    let c = base.clone();
    win.on_set_bool(move |k, v| c.mutate(false, true, |g| apply_bool_setting(g, k.as_str(), v)));
    let c = base.clone();
    win.on_set_int(move |k, v| c.mutate(false, true, |g| apply_int_setting(g, k.as_str(), v)));
    let c = base.clone();
    win.on_toggle_service(move |idx, enabled| {
        c.mutate(false, true, |g| {
            if let Some(s) = usize::try_from(idx)
                .ok()
                .and_then(|i| g.services.get_mut(i))
            {
                s.enabled = enabled;
            }
        });
    });
    let c = base.clone();
    win.on_update_service_paths(move |idx, cli, dir| {
        c.mutate(false, false, |g| {
            if let Some(s) = usize::try_from(idx)
                .ok()
                .and_then(|i| g.services.get_mut(i))
            {
                s.process_name = cli.trim().into();
                s.data_dir = dir.trim().into();
            }
        });
    });
    let c = base.clone();
    win.on_add_custom_service(move |label, cli, dir| {
        c.mutate(true, true, |g| {
            add_custom_service(&mut g.services, &label, &cli, &dir, &g.theme.text_color);
        });
    });
    let c = base.clone();
    win.on_remove_service(move |idx| {
        c.mutate(true, true, |g| {
            if let Ok(i) = usize::try_from(idx)
                && i < g.services.len()
                && !g.services[i].builtin
            {
                g.services.remove(i);
            }
        });
    });
    let c = base.clone();
    win.on_rescan_services(move || {
        let Some(w) = c.weak.upgrade() else { return };
        let Ok(cfg) = c.settings.lock().map(|g| g.clone()) else {
            return;
        };
        let scanned = c.backend.detect_services(&cfg.services);
        if let Ok(mut st) = c.statuses.lock() {
            (*st).clone_from(&scanned);
        }
        sync_service_rows(&w, &cfg.services, &scanned);
    });
}

fn wire_color_callbacks(win: &SettingsWindow, base: &UiCtx) {
    let c = base.clone();
    win.on_select_color_target(move |target, label| {
        if let (Some(w), Ok(cfg)) = (c.weak.upgrade(), c.settings.lock()) {
            w.set_active_color_target(target.clone());
            w.set_active_color_label(label);
            sync_active_color(&w, &cfg, target.as_str());
        }
    });
    let c = base.clone();
    win.on_set_active_rgb(move |r, g, b| {
        let Some(w) = c.weak.upgrade() else { return };
        let target = w.get_active_color_target().to_string();
        let hex = format_hex_rgb([
            r.clamp(0, 255) as u8,
            g.clamp(0, 255) as u8,
            b.clamp(0, 255) as u8,
        ]);
        c.mutate(false, true, |cfg| set_theme_hex(cfg, &target, &hex));
    });
    let c = base.clone();
    win.on_set_active_hex(move |hex| {
        let Some(w) = c.weak.upgrade() else { return };
        let target = w.get_active_color_target().to_string();
        c.mutate(false, true, |cfg| set_theme_hex(cfg, &target, hex.as_str()));
    });
    let c = base.clone();
    win.on_clear_item_color(move |id| {
        c.mutate(false, true, |cfg| {
            let def = cfg.theme.text_color.clone();
            if let Some(svc) = cfg.services.iter_mut().find(|s| s.id == id.as_str()) {
                svc.title_color = def;
            }
        });
    });
    let c = base.clone();
    win.on_apply_preset(move |preset| {
        c.mutate(false, true, |cfg| {
            apply_theme_preset(&mut cfg.theme, preset.as_str());
        });
    });
    let c = base.clone();
    win.on_reset_defaults(move || {
        c.mutate(true, true, |cfg| {
            *cfg = PanelSettings::default();
            merge_default_services(&mut cfg.services);
        });
    });
}
