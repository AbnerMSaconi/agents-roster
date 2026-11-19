// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Slint floating panel, tray icon, view-model, and embedded i18n catalog.
//!
//! Platform backends: Linux uses the SNI tray (`ksni`) and X11/Wayland window hints;
//! Windows 11 uses the Win32 notification-area icon and DWM window hints.

pub mod agent_control;
pub mod app;
pub mod callbacks;
pub mod commands;
pub mod i18n;
pub(crate) mod settings_sync;
pub mod settings_ui;
pub mod terminal;
pub mod theme;
pub mod tray;
pub mod view_model;

#[cfg(target_os = "linux")]
pub mod x11_hints;
#[cfg(target_os = "linux")]
pub use x11_hints as window_hints;

#[cfg(target_os = "windows")]
pub mod win_hints;
#[cfg(target_os = "windows")]
pub use win_hints as window_hints;

#[cfg(target_os = "linux")]
#[path = "tray_linux.rs"]
mod tray_platform;
#[cfg(target_os = "windows")]
#[path = "tray_win.rs"]
mod tray_platform;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!("agents-roster supports Linux (X11/Wayland) and Windows 11");

pub use app::{ConfigStore, PanelConfig, UiError, run_panel};
pub use i18n::PoTranslator;
pub use settings_ui::SettingsBackend;
pub use terminal::SystemTerminalOpener;
