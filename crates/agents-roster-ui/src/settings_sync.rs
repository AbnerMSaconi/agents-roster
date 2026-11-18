// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Control Panel property synchronization and configuration mutations.

use std::collections::HashMap;

use agents_roster_core::{
    PanelSettings, ServiceEntry, ServiceStatus, format_hex_rgb, get_theme_hex, parse_hex_rgb,
};
use slint::{Color, ModelRc, VecModel};

use crate::app::{ItemColorRow, ServiceControlRow, SettingsWindow};
use crate::theme::hex_to_color;

/// Applies a named boolean toggle from `SettingsWindow` to `cfg`.
pub(crate) fn apply_bool_setting(cfg: &mut PanelSettings, key: &str, value: bool) {
    match key {
        "visible" => cfg.visible = value,
        "always_on_top" => cfg.always_on_top = value,
        "hide_from_taskbar" => cfg.hide_from_taskbar = value,
        "blur_enabled" => cfg.theme.blur = value,
        "os_on_started" => cfg.notifications.os_on_started = value,
        "os_on_waiting" => cfg.notifications.os_on_waiting = value,
        "os_on_finished" => cfg.notifications.os_on_finished = value,
        "os_on_error" => cfg.notifications.os_on_error = value,
        "os_on_done_item" => cfg.notifications.os_on_done_item = value,
        "os_on_context_high" => cfg.notifications.os_on_context_high = value,
        "os_on_quota" => cfg.notifications.os_on_quota = value,
        "os_on_reminder" => {
            cfg.notifications.os_on_reminder = value;
            cfg.reminder_min = if value { cfg.reminder_min.max(15) } else { 0 };
        }
        _ => {}
    }
}

/// Applies a named integer setting (`interval_s`, `reminder_min`, opacities) to `cfg`.
pub(crate) fn apply_int_setting(cfg: &mut PanelSettings, key: &str, value: i32) {
    let pct = (value.clamp(0, 100) as f32) / 100.0;
    match key {
        "interval_s" => cfg.interval_s = u64::try_from(value.clamp(1, 60)).unwrap_or(2),
        "reminder_min" => {
            cfg.reminder_min = u64::try_from(value.clamp(1, 240)).unwrap_or(15);
            cfg.notifications.os_on_reminder = true;
        }
        "bg_opacity" => cfg.theme.bg_opacity = pct,
        "border_opacity" => cfg.theme.border_opacity = pct,
        _ => {}
    }
}

/// Syncs all [`PanelSettings`] fields, services, and color editor state into `SettingsWindow`.
pub(crate) fn sync_settings_window(
    w: &SettingsWindow,
    cfg: &PanelSettings,
    statuses: &[ServiceStatus],
) {
    w.set_legal_notice(agents_roster_core::about::short_notice().into());
    w.set_cfg_visible(cfg.visible);
    w.set_cfg_always_on_top(cfg.always_on_top);
    w.set_cfg_skip_taskbar(cfg.hide_from_taskbar);
    w.set_cfg_interval_s(cfg.interval_s as i32);

    let installed = statuses.iter().filter(|s| s.installed).count();
    w.set_services_summary(format!("{installed}/{} installed", cfg.services.len()).into());
    sync_service_rows(w, &cfg.services, statuses);
    sync_notifications(w, cfg);
    sync_theme(w, cfg);

    let target = w.get_active_color_target().to_string();
    sync_active_color(w, cfg, &target);
}

pub(crate) fn sync_service_rows(
    w: &SettingsWindow,
    services: &[ServiceEntry],
    statuses: &[ServiceStatus],
) {
    let map: HashMap<&str, &ServiceStatus> = statuses.iter().map(|s| (s.id.as_str(), s)).collect();
    let rows: Vec<ServiceControlRow> = services
        .iter()
        .map(|s| {
            let st = map.get(s.id.as_str()).copied();
            ServiceControlRow {
                id: s.id.clone().into(),
                label: s.label.clone().into(),
                enabled: s.enabled,
                installed: st.is_some_and(|v| v.installed),
                detected_path: st
                    .map(|v| v.detected_path.clone())
                    .unwrap_or_default()
                    .into(),
                process_name: s.process_name.clone().into(),
                data_dir: s.data_dir.clone().into(),
                title_color: hex_to_color(&s.title_color, 1.0, [0xe8, 0xe8, 0xe8]),
                builtin: s.builtin,
            }
        })
        .collect();
    w.set_service_rows(ModelRc::new(VecModel::from(rows)));
}

fn sync_notifications(w: &SettingsWindow, cfg: &PanelSettings) {
    let n = &cfg.notifications;
    w.set_os_on_started(n.os_on_started);
    w.set_os_on_waiting(n.os_on_waiting);
    w.set_os_on_finished(n.os_on_finished);
    w.set_os_on_error(n.os_on_error);
    w.set_os_on_done_item(n.os_on_done_item);
    w.set_os_on_context_high(n.os_on_context_high);
    w.set_os_on_quota(n.os_on_quota);
    w.set_os_on_reminder(n.os_on_reminder);
    w.set_cfg_reminder_min(if cfg.reminder_min == 0 {
        15
    } else {
        cfg.reminder_min as i32
    });
}

fn sync_theme(w: &SettingsWindow, cfg: &PanelSettings) {
    let th = &cfg.theme;
    w.set_th_bg_color(hex_to_color(&th.bg_color, 1.0, [0x14, 0x14, 0x18]));
    w.set_th_bg_hex(norm_hex(&th.bg_color).into());
    w.set_th_bg_opacity((th.bg_opacity * 100.0).round() as i32);
    w.set_th_border_color(hex_to_color(&th.border_color, 1.0, [0xff, 0xff, 0xff]));
    w.set_th_border_hex(norm_hex(&th.border_color).into());
    w.set_th_border_opacity((th.border_opacity * 100.0).round() as i32);
    w.set_th_blur_enabled(th.blur);
    w.set_th_text_color(hex_to_color(&th.text_color, 1.0, [0xe8, 0xe8, 0xe8]));
    w.set_th_text_hex(norm_hex(&th.text_color).into());
    w.set_th_muted_color(hex_to_color(&th.muted_color, 1.0, [0x8a, 0x8a, 0x8a]));
    w.set_th_muted_hex(norm_hex(&th.muted_color).into());
    w.set_th_alert_color(hex_to_color(&th.alert_color, 1.0, [0xff, 0xb3, 0x47]));
    w.set_th_alert_hex(norm_hex(&th.alert_color).into());
    w.set_th_critical_color(hex_to_color(&th.critical_color, 1.0, [0xff, 0x5c, 0x5c]));
    w.set_th_critical_hex(norm_hex(&th.critical_color).into());
    sync_item_color_rows(w, cfg);
}

fn sync_item_color_rows(w: &SettingsWindow, cfg: &PanelSettings) {
    let default_hex = norm_hex(&cfg.theme.text_color);
    let rows: Vec<ItemColorRow> = cfg
        .services
        .iter()
        .map(|s| {
            let hex = norm_hex(&s.title_color);
            ItemColorRow {
                key: s.id.clone().into(),
                label: s.label.clone().into(),
                color: hex_to_color(&hex, 1.0, [0xe8, 0xe8, 0xe8]),
                is_custom: hex != default_hex,
                hex: hex.into(),
            }
        })
        .collect();
    w.set_item_color_rows(ModelRc::new(VecModel::from(rows)));
}

/// Syncs the inline RGB editor preview for `target`.
pub(crate) fn sync_active_color(w: &SettingsWindow, cfg: &PanelSettings, target: &str) {
    let hex = get_theme_hex(cfg, target);
    let rgb = parse_hex_rgb(&hex).unwrap_or([0x14, 0x14, 0x18]);
    w.set_active_color_hex(hex.into());
    w.set_active_color_preview(Color::from_rgb_u8(rgb[0], rgb[1], rgb[2]));
    w.set_active_r(i32::from(rgb[0]));
    w.set_active_g(i32::from(rgb[1]));
    w.set_active_b(i32::from(rgb[2]));
}

fn norm_hex(raw: &str) -> String {
    parse_hex_rgb(raw).map_or_else(|| "#e8e8e8".into(), format_hex_rgb)
}
