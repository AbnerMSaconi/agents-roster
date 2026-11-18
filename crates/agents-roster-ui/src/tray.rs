// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! KDE SNI tray icon with 32x32 working-agent counter and context menu.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use agents_roster_core::{PanelSettings, Translator};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};

/// Actions triggered from the tray icon or panel context menu.
#[derive(Debug, Clone)]
pub enum TrayCommand {
    ToggleVisible,
    AlwaysOnTop(bool),
    Reminders(bool),
    OpenSettings,
    ApplySettings(Box<PanelSettings>),
    Quit,
}

/// SNI tray icon state managed by `ksni`.
pub struct TrayIcon {
    pub tx: Sender<TrayCommand>,
    pub tr: Arc<dyn Translator>,
    pub lines: Vec<String>,
    pub working_count: usize,
    pub color: [u8; 3],
    pub always_on_top: bool,
    pub reminders: bool,
}

impl ksni::Tray for TrayIcon {
    fn id(&self) -> String {
        "agents-roster".into()
    }

    fn title(&self) -> String {
        self.tr.translate("Agents").into_owned()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![render_icon(self.working_count, self.color)]
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

/// 3x5 bitmap digits (bit 2 = left column).
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

fn paint_digit(px: &mut [[u8; 4]], size: usize, digit: u8, x0: usize, y0: usize, color: [u8; 3]) {
    const SCALE: usize = 4;
    let bitmap = DIGITS[(digit - b'0') as usize];
    for (row, bits) in bitmap.iter().enumerate() {
        for col in 0..3 {
            if bits & (4 >> col) == 0 {
                continue;
            }
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let x = x0 + col * SCALE + dx;
                    let y = y0 + row * SCALE + dy;
                    px[y * size + x] = [255, color[0], color[1], color[2]];
                }
            }
        }
    }
}

/// Renders a 32x32 ARGB rounded dark badge displaying `n` working agents.
pub fn render_icon(n: usize, color: [u8; 3]) -> Icon {
    const SIZE: usize = 32;
    let mut px = vec![[0u8; 4]; SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (cx, cy) = (x.min(SIZE - 1 - x), y.min(SIZE - 1 - y));
            if !(cx < 5 && cy < 5 && (5 - cx).pow(2) + (5 - cy).pow(2) > 26) {
                px[y * SIZE + x] = [235, 20, 20, 24];
            }
        }
    }
    let digit = if n > 9 { b'9' } else { b'0' + n as u8 };
    let x0 = (SIZE - 3 * 4) / 2;
    let y0 = (SIZE - 5 * 4) / 2;
    paint_digit(&mut px, SIZE, digit, x0, y0, color);
    Icon {
        width: SIZE as i32,
        height: SIZE as i32,
        data: px.into_iter().flatten().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_argb_buffer_size() {
        for n in [0, 3, 12] {
            let icon = render_icon(n, [232, 232, 232]);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
        }
    }
}
