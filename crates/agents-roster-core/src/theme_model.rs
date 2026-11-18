// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Theme color normalization, built-in presets, and color-target helpers.

use crate::settings::{PanelSettings, ThemeConfig};

/// Quick-pick palette swatches offered in the Control Panel inline color editor.
pub const PALETTE_SWATCHES: [&str; 16] = [
    "#ffffff", "#e8e8e8", "#8a8a8a", "#141418", "#56d4dd", "#4ea1ff", "#7aa2f7", "#bd93f9",
    "#ff79c6", "#ff5c5c", "#ff8c42", "#ffb347", "#f1fa8c", "#a6e3a1", "#50fa7b", "#2ec4b6",
];

/// Parses `#RRGGBB`, `#RGB`, or hex digits without `#` into `[r, g, b]`.
pub fn parse_hex_rgb(hex: &str) -> Option<[u8; 3]> {
    let raw = hex.trim().trim_start_matches('#');
    if !raw.is_ascii() {
        return None;
    }
    if raw.len() == 6 {
        let r = u8::from_str_radix(&raw[0..2], 16).ok()?;
        let g = u8::from_str_radix(&raw[2..4], 16).ok()?;
        let b = u8::from_str_radix(&raw[4..6], 16).ok()?;
        Some([r, g, b])
    } else if raw.len() == 3 {
        let h = |i: usize| u8::from_str_radix(&raw[i..=i], 16).ok().map(|n| n * 17);
        Some([h(0)?, h(1)?, h(2)?])
    } else {
        None
    }
}

/// Formats `[r, g, b]` as lowercase `#rrggbb`.
pub fn format_hex_rgb(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Parses `#rrggbb` or `#rgb` hex strings into `[r, g, b, a]` bytes, clamping `opacity` to `0..=1`.
pub fn parse_hex_rgba(hex: &str, opacity: f32, fallback: [u8; 3]) -> [u8; 4] {
    let [r, g, b] = parse_hex_rgb(hex).unwrap_or(fallback);
    let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    [r, g, b, a]
}

/// Applies a built-in theme preset (`dark`, `nord`, `dracula`, `amber`, `contrast`, `glass`).
pub fn apply_theme_preset(theme: &mut ThemeConfig, preset: &str) {
    let (bg, bg_a, brd, brd_a, blur, txt, mutd, alt, crit) = match preset {
        "nord" => (
            "#2e3440", 0.90, "#88c0d0", 0.35, false, "#eceff4", "#81a1c1", "#ebcb8b", "#bf616a",
        ),
        "dracula" => (
            "#282a36", 0.90, "#bd93f9", 0.40, false, "#f8f8f2", "#6272a4", "#ffb86c", "#ff5555",
        ),
        "amber" => (
            "#16120c", 0.92, "#ffb347", 0.30, false, "#ffd48a", "#9c7a44", "#ff9800", "#ff453a",
        ),
        "contrast" => (
            "#000000", 0.96, "#ffffff", 0.60, false, "#ffffff", "#b0b0b0", "#ffd60a", "#ff453a",
        ),
        "glass" => (
            "#1b2028", 0.52, "#ffffff", 0.28, true, "#f2f5f9", "#b4c0d0", "#ffcc66", "#ff6b6b",
        ),
        _ => {
            *theme = ThemeConfig::default();
            return;
        }
    };
    *theme = ThemeConfig {
        bg_color: bg.into(),
        bg_opacity: bg_a,
        border_color: brd.into(),
        border_opacity: brd_a,
        text_color: txt.into(),
        muted_color: mutd.into(),
        alert_color: alt.into(),
        critical_color: crit.into(),
        blur,
    };
}

/// Reads the `#rrggbb` hex value for a target (`bg`, `border`, `text`, `muted`, `alert`, `critical`, or `item:<id>`).
pub fn get_theme_hex(settings: &PanelSettings, target: &str) -> String {
    let raw = match target {
        "bg" => &settings.theme.bg_color,
        "border" => &settings.theme.border_color,
        "text" => &settings.theme.text_color,
        "muted" => &settings.theme.muted_color,
        "alert" => &settings.theme.alert_color,
        "critical" => &settings.theme.critical_color,
        _ => {
            if let Some(id) = target.strip_prefix("item:")
                && let Some(svc) = settings.services.iter().find(|s| s.id == id)
            {
                &svc.title_color
            } else {
                &settings.theme.text_color
            }
        }
    };
    parse_hex_rgb(raw).map_or_else(|| "#e8e8e8".into(), format_hex_rgb)
}

/// Writes `#rrggbb` to the selected target in `settings`.
pub fn set_theme_hex(settings: &mut PanelSettings, target: &str, hex: &str) {
    let Some(rgb) = parse_hex_rgb(hex) else {
        return;
    };
    let norm = format_hex_rgb(rgb);
    match target {
        "bg" => settings.theme.bg_color = norm,
        "border" => settings.theme.border_color = norm,
        "text" => settings.theme.text_color = norm,
        "muted" => settings.theme.muted_color = norm,
        "alert" => settings.theme.alert_color = norm,
        "critical" => settings.theme.critical_color = norm,
        _ => {
            if let Some(id) = target.strip_prefix("item:")
                && let Some(svc) = settings.services.iter_mut().find(|s| s.id == id)
            {
                svc.title_color = norm;
            }
        }
    }
}
