// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Appearance, notification routing, and AI service configuration domain types.

use serde::{Deserialize, Serialize};

/// Controls which events dispatch OS desktop notifications vs staying widget-only.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationPrefs {
    pub os_on_started: bool,
    pub os_on_waiting: bool,
    pub os_on_finished: bool,
    pub os_on_error: bool,
    pub os_on_done_item: bool,
    pub os_on_context_high: bool,
    pub os_on_quota: bool,
    pub os_on_reminder: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            os_on_started: true,
            os_on_waiting: true,
            os_on_finished: true,
            os_on_error: true,
            os_on_done_item: true,
            os_on_context_high: true,
            os_on_quota: true,
            os_on_reminder: true,
        }
    }
}

/// Configured AI coding agent service (built-in or user-added).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceEntry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub process_name: String,
    pub data_dir: String,
    pub title_color: String,
    pub builtin: bool,
}

impl Default for ServiceEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            enabled: true,
            process_name: String::new(),
            data_dir: String::new(),
            title_color: "#e8e8e8".into(),
            builtin: false,
        }
    }
}

/// Auto-detection probe result for one [`ServiceEntry`] (shown in Settings UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub id: String,
    pub installed: bool,
    pub detected_path: String,
}

/// Fully customizable widget palette, transparency, and blur setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub bg_color: String,
    pub bg_opacity: f32,
    pub border_color: String,
    pub border_opacity: f32,
    pub text_color: String,
    pub muted_color: String,
    pub alert_color: String,
    pub critical_color: String,
    pub blur: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg_color: "#141418".into(),
            bg_opacity: 0.88,
            border_color: "#ffffff".into(),
            border_opacity: 0.16,
            text_color: "#e8e8e8".into(),
            muted_color: "#8a8a8a".into(),
            alert_color: "#ffb347".into(),
            critical_color: "#ff5c5c".into(),
            blur: false,
        }
    }
}

/// Combined live settings snapshot shared between Settings UI, collector, and panel.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSettings {
    pub always_on_top: bool,
    pub visible: bool,
    pub hide_from_taskbar: bool,
    pub reminder_min: u64,
    pub interval_s: u64,
    pub agy_context_limit: u64,
    pub theme: ThemeConfig,
    pub notifications: NotificationPrefs,
    pub services: Vec<ServiceEntry>,
}

impl Default for PanelSettings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            visible: true,
            hide_from_taskbar: true,
            reminder_min: 15,
            interval_s: 2,
            agy_context_limit: 200_000,
            theme: ThemeConfig::default(),
            notifications: NotificationPrefs::default(),
            services: default_services(),
        }
    }
}

/// Built-in coding agent services shipped with default discovery hints.
pub fn default_services() -> Vec<ServiceEntry> {
    vec![
        ServiceEntry {
            id: "claude".into(),
            label: "Claude Code".into(),
            enabled: true,
            process_name: "claude".into(),
            data_dir: String::new(),
            title_color: "#e8e8e8".into(),
            builtin: true,
        },
        ServiceEntry {
            id: "agy".into(),
            label: "Antigravity (agy)".into(),
            enabled: true,
            process_name: "agy".into(),
            data_dir: "~/.gemini/antigravity-cli".into(),
            title_color: "#e8e8e8".into(),
            builtin: true,
        },
        ServiceEntry {
            id: "codex".into(),
            label: "Codex CLI".into(),
            enabled: true,
            process_name: "codex".into(),
            data_dir: "~/.codex".into(),
            title_color: "#e8e8e8".into(),
            builtin: true,
        },
        ServiceEntry {
            id: "opencode".into(),
            label: "OpenCode".into(),
            enabled: true,
            process_name: "opencode".into(),
            data_dir: "~/.local/share/opencode".into(),
            title_color: "#e8e8e8".into(),
            builtin: true,
        },
        ServiceEntry {
            id: "gemini".into(),
            label: "Gemini CLI".into(),
            enabled: true,
            process_name: "gemini".into(),
            data_dir: "~/.gemini/tmp".into(),
            title_color: "#e8e8e8".into(),
            builtin: true,
        },
    ]
}

/// Merges saved service overrides with built-in defaults so newly supported agents appear.
pub fn merge_default_services(saved: &mut Vec<ServiceEntry>) {
    for def in default_services() {
        if !saved.iter().any(|s| s.id == def.id) {
            saved.push(def);
        }
    }
}

/// Appends a user-defined coding agent service and returns `false` if both label and CLI are empty.
pub fn add_custom_service(
    services: &mut Vec<ServiceEntry>,
    label: &str,
    proc_name: &str,
    data_dir: &str,
    color: &str,
) -> bool {
    let (name, proc_s) = (label.trim(), proc_name.trim());
    if name.is_empty() && proc_s.is_empty() {
        return false;
    }
    let base = if name.is_empty() { proc_s } else { name };
    let slug: String = base
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    services.push(ServiceEntry {
        id: format!("custom-{slug}"),
        label: base.into(),
        process_name: if proc_s.is_empty() { base } else { proc_s }.into(),
        data_dir: data_dir.trim().into(),
        title_color: if color.trim().is_empty() {
            "#66d9ef"
        } else {
            color.trim()
        }
        .into(),
        ..Default::default()
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_model::parse_hex_rgba;

    #[test]
    fn parses_hex_colors_and_clamps_opacity() {
        assert_eq!(parse_hex_rgba("#141418", 0.5, [0, 0, 0]), [20, 20, 24, 128]);
        assert_eq!(parse_hex_rgba("f80", 2.0, [0, 0, 0]), [255, 136, 0, 255]);
        assert_eq!(
            parse_hex_rgba("invalid", -1.0, [10, 20, 30]),
            [10, 20, 30, 0]
        );
        assert_eq!(
            parse_hex_rgba("\u{00e1}\u{00e9}\u{00ed}", 1.0, [1, 2, 3]),
            [1, 2, 3, 255]
        );
        assert_eq!(parse_hex_rgba("\u{00e1}b", 1.0, [1, 2, 3]), [1, 2, 3, 255]);
    }

    #[test]
    fn merges_missing_builtin_services_preserving_custom_entries() {
        let mut list = vec![ServiceEntry {
            id: "claude".into(),
            title_color: "#ff0000".into(),
            builtin: true,
            ..Default::default()
        }];
        merge_default_services(&mut list);
        assert!(list.iter().any(|s| s.id == "opencode"));
        assert!(list.iter().any(|s| s.id == "codex"));
        assert_eq!(list[0].title_color, "#ff0000");
    }
}
