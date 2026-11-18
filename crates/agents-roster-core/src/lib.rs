// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure domain model, traits, and notification state machine for `agents-roster`.

pub mod about;
pub mod collector;
pub mod model;
pub mod notify_engine;
pub mod notify_rules;
pub mod settings;
pub mod text;
pub mod theme_model;
pub mod time;
pub mod traits;

pub use collector::SnapshotCollector;
pub use model::{
    AccountLimits, Agent, AgentState, DoneItem, Notification, ProviderQuota, Snapshot, UsageWindow,
};
pub use notify_engine::{LONG_TURN_MS, NotificationEngine};
pub use settings::{
    NotificationPrefs, PanelSettings, ServiceEntry, ServiceStatus, ThemeConfig, add_custom_service,
    default_services, merge_default_services,
};
pub use theme_model::{
    PALETTE_SWATCHES, apply_theme_preset, format_hex_rgb, get_theme_hex, parse_hex_rgb,
    parse_hex_rgba, set_theme_hex,
};
pub use traits::{
    AgentSource, Clock, GitInfo, IdentityTranslator, Notifier, NotifyError, ProcessInfo,
    ProcessTable, SourceReport, TerminalOpener, TimeFormatter, Translator,
};
