// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Slint floating panel, SNI tray icon, view-model, and embedded i18n catalog.

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
pub mod x11_hints;

pub use app::{ConfigStore, PanelConfig, UiError, run_panel};
pub use i18n::PoTranslator;
pub use settings_ui::SettingsBackend;
pub use terminal::SystemTerminalOpener;
