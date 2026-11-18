// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Persistent panel configuration (`~/.config/agents-roster/config.toml`).

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use agents_roster_core::{
    NotificationPrefs, PanelSettings, ServiceEntry, ThemeConfig, default_services,
    merge_default_services,
};
use agents_roster_ui::{ConfigStore, PanelConfig};
use serde::{Deserialize, Serialize};

/// Application configuration persisted as TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub x: i32,
    pub y: i32,
    pub always_on_top: bool,
    pub visible: bool,
    pub hide_from_taskbar: bool,
    pub reminder_min: u64,
    pub agy_context_limit: u64,
    pub interval_s: u64,
    pub theme: ThemeConfig,
    pub notifications: NotificationPrefs,
    pub services: Vec<ServiceEntry>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            x: 60,
            y: 300,
            always_on_top: true,
            visible: true,
            hide_from_taskbar: true,
            reminder_min: 15,
            agy_context_limit: 200_000,
            interval_s: 2,
            theme: ThemeConfig::default(),
            notifications: NotificationPrefs::default(),
            services: default_services(),
        }
    }
}

impl From<&AppConfig> for PanelSettings {
    fn from(c: &AppConfig) -> Self {
        let mut services = c.services.clone();
        merge_default_services(&mut services);
        Self {
            always_on_top: c.always_on_top,
            visible: c.visible,
            hide_from_taskbar: c.hide_from_taskbar,
            reminder_min: c.reminder_min,
            interval_s: c.interval_s,
            agy_context_limit: c.agy_context_limit,
            theme: c.theme.clone(),
            notifications: c.notifications.clone(),
            services,
        }
    }
}

impl From<&AppConfig> for PanelConfig {
    fn from(c: &AppConfig) -> Self {
        Self {
            x: c.x,
            y: c.y,
            always_on_top: c.always_on_top,
            visible: c.visible,
            hide_from_taskbar: c.hide_from_taskbar,
            reminder_min: c.reminder_min,
            interval_s: c.interval_s,
        }
    }
}

fn config_base() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"),
        PathBuf::from,
    )
}

fn new_config_path() -> PathBuf {
    config_base().join("agents-roster/config.toml")
}

impl AppConfig {
    /// Loads configuration from `agents-roster/config.toml`, or defaults when absent or invalid.
    pub fn load() -> Self {
        for path in [new_config_path()] {
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(mut cfg) = toml::from_str::<Self>(&text)
            {
                merge_default_services(&mut cfg.services);
                return cfg;
            }
        }
        Self::default()
    }

    /// Saves configuration atomically to `agents-roster/config.toml`.
    pub fn save(&self) {
        let path = new_config_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string(self) {
            let tmp = path.with_extension("tmp");
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }
}

/// Persists UI panel changes back to [`AppConfig`].
pub struct TomlConfigStore {
    inner: Mutex<AppConfig>,
}

impl TomlConfigStore {
    pub fn new(initial: AppConfig) -> Self {
        Self {
            inner: Mutex::new(initial),
        }
    }

    pub fn save_panel_settings(&self, s: &PanelSettings) {
        if let Ok(mut cfg) = self.inner.lock() {
            cfg.visible = s.visible;
            cfg.always_on_top = s.always_on_top;
            cfg.hide_from_taskbar = s.hide_from_taskbar;
            cfg.reminder_min = s.reminder_min;
            cfg.interval_s = s.interval_s;
            cfg.agy_context_limit = s.agy_context_limit;
            cfg.theme.clone_from(&s.theme);
            cfg.notifications.clone_from(&s.notifications);
            cfg.services.clone_from(&s.services);
            cfg.save();
        }
    }
}

impl ConfigStore for TomlConfigStore {
    fn save(&self, panel: &PanelConfig) {
        if let Ok(mut cfg) = self.inner.lock() {
            cfg.x = panel.x;
            cfg.y = panel.y;
            cfg.always_on_top = panel.always_on_top;
            cfg.visible = panel.visible;
            cfg.reminder_min = panel.reminder_min;
            cfg.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_real_config_file() {
        let raw = "\
x = 1440
y = 893
always_on_top = true
visible = true
reminder_min = 15
agy_context_limit = 200000
interval_s = 2
";
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.x, 1440);
        assert_eq!(cfg.y, 893);
        assert!(cfg.always_on_top);
        assert!(cfg.visible);
        assert_eq!(cfg.reminder_min, 15);
        assert_eq!(cfg.agy_context_limit, 200_000);
        assert_eq!(cfg.interval_s, 2);
    }

    #[test]
    fn round_trips_full_config_with_custom_theme_and_services() {
        let mut cfg = AppConfig::default();
        cfg.theme.bg_color = "#221133".into();
        cfg.theme.bg_opacity = 0.65;
        cfg.theme.blur = true;
        cfg.services[0].title_color = "#66d9ef".into();
        let serialized = toml::to_string(&cfg).unwrap();
        let loaded: AppConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(loaded.theme.bg_color, "#221133");
        assert!(loaded.theme.blur);
        assert_eq!(loaded.services[0].title_color, "#66d9ef");
    }
}
