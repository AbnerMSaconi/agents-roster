// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! KDE SNI tray backend (`ksni`, D-Bus StatusNotifierItem).

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip, TrayMethods};

use crate::tray::{TrayCommand, TrayIcon, render_icon};

impl ksni::Tray for TrayIcon {
    fn id(&self) -> String {
        "agents-roster".into()
    }

    fn title(&self) -> String {
        self.tr.translate("Agents").into_owned()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let d = render_icon(self.working_count, self.color);
        vec![Icon { width: d.width, height: d.height, data: d.data }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.tr.translate("Agents").into_owned(),
            description: self.lines.join("\n"),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCommand::ToggleVisible);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.tr.translate("Show/hide").into_owned(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCommand::ToggleVisible);
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: self.tr.translate("Always on top").into_owned(),
                checked: self.always_on_top,
                activate: Box::new(|t: &mut Self| {
                    t.always_on_top = !t.always_on_top;
                    let _ = t.tx.send(TrayCommand::AlwaysOnTop(t.always_on_top));
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: self.tr.translate("Reminders while working").into_owned(),
                checked: self.reminders,
                activate: Box::new(|t: &mut Self| {
                    t.reminders = !t.reminders;
                    let _ = t.tx.send(TrayCommand::Reminders(t.reminders));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.tr.translate("Control Panel...").into_owned(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCommand::OpenSettings);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.tr.translate("Quit").into_owned(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Handle to the spawned SNI tray icon; updates run on the ksni service thread.
pub struct TrayHandle(ksni::blocking::Handle<TrayIcon>);

impl TrayHandle {
    /// Mutates the shared tray state and schedules a redraw.
    pub fn update(&self, f: impl FnOnce(&mut TrayIcon)) {
        self.0.update(f);
    }
}

/// Spawns the SNI tray icon on its own D-Bus service thread.
pub fn spawn_tray(state: TrayIcon) -> Option<TrayHandle> {
    state.spawn().ok().map(TrayHandle)
}
