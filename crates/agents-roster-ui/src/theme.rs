// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Theme palette resolution and live Slint color application.

use std::collections::HashMap;

use agents_roster_core::{Agent, ServiceEntry, ThemeConfig, parse_hex_rgba};
use slint::{Color, Global};

use crate::app::{Colors, Panel};
use crate::view_model::{COLOR_ALERT, COLOR_CRITICAL, COLOR_MUTED, COLOR_TEXT, PanelView};

/// Resolved Slint RGBA palette plus per-service agent title colors.
#[derive(Clone, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub alert: Color,
    pub critical: Color,
    pub service_colors: HashMap<String, Color>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_config(&ThemeConfig::default(), &[])
    }
}

pub(crate) fn hex_to_color(hex: &str, opacity: f32, fallback: [u8; 3]) -> Color {
    let [r, g, b, a] = parse_hex_rgba(hex, opacity, fallback);
    Color::from_argb_u8(a, r, g, b)
}

impl Palette {
    /// Converts persisted [`ThemeConfig`] and per-service colors into Slint colors.
    pub fn from_config(theme: &ThemeConfig, services: &[ServiceEntry]) -> Self {
        let text = hex_to_color(&theme.text_color, 1.0, [0xe8, 0xe8, 0xe8]);
        let mut service_colors = HashMap::new();
        for svc in services {
            if !svc.id.is_empty() {
                service_colors.insert(
                    svc.id.clone(),
                    hex_to_color(&svc.title_color, 1.0, [0xe8, 0xe8, 0xe8]),
                );
            }
        }
        Self {
            bg: hex_to_color(&theme.bg_color, theme.bg_opacity, [0x14, 0x14, 0x18]),
            border: hex_to_color(
                &theme.border_color,
                theme.border_opacity,
                [0xff, 0xff, 0xff],
            ),
            text,
            muted: hex_to_color(&theme.muted_color, 1.0, [0x8a, 0x8a, 0x8a]),
            alert: hex_to_color(&theme.alert_color, 1.0, [0xff, 0xb3, 0x47]),
            critical: hex_to_color(&theme.critical_color, 1.0, [0xff, 0x5c, 0x5c]),
            service_colors,
        }
    }

    /// Resolves the custom title color for a given `source_id`, falling back to primary text color.
    pub fn title_color_for(&self, source_id: &str) -> Color {
        self.service_colors
            .get(source_id)
            .copied()
            .unwrap_or(self.text)
    }

    /// Applies palette colors to a running Slint [`Panel`] (blur is applied separately).
    pub fn apply_to_panel(&self, panel: &Panel) {
        let colors = Colors::get(panel);
        colors.set_bg(self.bg);
        colors.set_border(self.border);
        colors.set_text(self.text);
        colors.set_muted(self.muted);
        colors.set_alert(self.alert);
        colors.set_critical(self.critical);
    }

    /// Maps default semantic colors and per-service agent title colors onto a built [`PanelView`].
    pub fn recolor_view(&self, view: &mut PanelView, agents: &[Agent]) {
        for (ui, agent) in view.agents.iter_mut().zip(agents.iter()) {
            ui.title_color = self.title_color_for(&agent.source_id);
            ui.state_color = self.remap(ui.state_color);
            ui.ctx_color = self.remap(ui.ctx_color);
            ui.reason_color = self.remap(ui.reason_color);
        }
        for f in &mut view.footers {
            f.color = self.remap(f.color);
        }
    }

    fn remap(&self, c: Color) -> Color {
        if c == COLOR_TEXT {
            self.text
        } else if c == COLOR_MUTED {
            self.muted
        } else if c == COLOR_ALERT {
            self.alert
        } else if c == COLOR_CRITICAL {
            self.critical
        } else {
            c
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AgentUi;

    #[test]
    fn recolors_service_title_and_semantic_palette() {
        let theme = ThemeConfig {
            text_color: "#ffffff".into(),
            alert_color: "#00ff00".into(),
            ..Default::default()
        };
        let services = vec![ServiceEntry {
            id: "claude".into(),
            title_color: "#66d9ef".into(),
            ..Default::default()
        }];
        let pal = Palette::from_config(&theme, &services);
        let mut view = PanelView {
            agents: vec![AgentUi {
                key: "claude:1".into(),
                state_color: COLOR_ALERT,
                ..Default::default()
            }],
            footers: Vec::new(),
        };
        pal.recolor_view(
            &mut view,
            &[Agent {
                key: "claude:1".into(),
                source_id: "claude".into(),
                ..Default::default()
            }],
        );
        assert_eq!(
            view.agents[0].title_color,
            Color::from_argb_u8(255, 0x66, 0xd9, 0xef)
        );
        assert_eq!(
            view.agents[0].state_color,
            Color::from_argb_u8(255, 0x00, 0xff, 0x00)
        );
    }
}
