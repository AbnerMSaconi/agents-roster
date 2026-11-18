// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure view-model transformation from domain [`Snapshot`] to UI and tray state.

use agents_roster_core::time::format_duration;
use agents_roster_core::{
    AccountLimits, Agent, AgentState, ProviderQuota, Snapshot, TimeFormatter, Translator,
    UsageWindow,
};
use slint::Color;

use crate::app::{AgentUi, FooterLine};

pub const COLOR_TEXT: Color = Color::from_rgb_u8(0xe8, 0xe8, 0xe8);
pub const COLOR_ALERT: Color = Color::from_rgb_u8(0xff, 0xb3, 0x47);
pub const COLOR_CRITICAL: Color = Color::from_rgb_u8(0xff, 0x5c, 0x5c);
pub const COLOR_MUTED: Color = Color::from_rgb_u8(0x8a, 0x8a, 0x8a);

/// Pre-formatted tray tooltip and icon summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub lines: Vec<String>,
    pub working_count: usize,
    pub color: [u8; 3],
}

/// Complete UI view-model ready to push to Slint.
#[derive(Clone, PartialEq)]
pub struct PanelView {
    pub agents: Vec<AgentUi>,
    pub footers: Vec<FooterLine>,
}

fn pct_color(p: f64) -> Color {
    if p >= 95.0 {
        COLOR_CRITICAL
    } else if p >= 80.0 {
        COLOR_ALERT
    } else {
        COLOR_TEXT
    }
}

fn state_color(s: AgentState) -> Color {
    match s {
        AgentState::Working => COLOR_TEXT,
        AgentState::Waiting => COLOR_ALERT,
        AgentState::Error => COLOR_CRITICAL,
        AgentState::Idle | AgentState::Stopped => COLOR_MUTED,
    }
}

fn format_k(t: u64) -> String {
    if t >= 1000 {
        format!("{}k", t / 1000)
    } else {
        t.to_string()
    }
}

/// Converts a single domain [`Agent`] to its Slint row representation.
pub fn agent_to_ui(a: &Agent, now_ms: u64, tr: &dyn Translator) -> AgentUi {
    let (ctx, ctx_color) = match a.context_pct {
        Some(p) => (format!("ctx {p:.0}%"), pct_color(p)),
        None if a.context_tokens > 0 => (format!("ctx {}", format_k(a.context_tokens)), COLOR_TEXT),
        None => (String::new(), COLOR_MUTED),
    };
    let elapsed = if a.now_since_ms > 0 && a.state != AgentState::Stopped {
        format!(
            " · {}",
            format_duration(now_ms.saturating_sub(a.now_since_ms))
        )
    } else {
        String::new()
    };
    let done = a
        .done
        .first()
        .map(|d| {
            let dur = format_duration(now_ms.saturating_sub(d.at_ms));
            tr.format("{text} · {dur} ago", &[("text", &d.text), ("dur", &dur)])
        })
        .unwrap_or_default();
    AgentUi {
        key: a.key.clone().into(),
        depth: i32::from(a.parent_key.is_some()),
        title: a.title().into(),
        title_color: COLOR_TEXT,
        model: a.model.clone().into(),
        state: tr.translate(a.state.label_key()).as_ref().into(),
        state_color: state_color(a.state),
        ctx: ctx.into(),
        ctx_color,
        now: format!("{}{elapsed}", a.now).into(),
        next: a.next.clone().into(),
        done: done.into(),
        reason: a.reason.clone().into(),
        reason_color: if a.state == AgentState::Error {
            COLOR_CRITICAL
        } else {
            COLOR_ALERT
        },
    }
}

fn format_window(
    w: &UsageWindow,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> String {
    let pct = format!("{:.0}", w.pct);
    match w.resets_at_ms {
        Some(t) => tr.format(
            "{name} {pct}% · resets {when}",
            &[
                ("name", &w.name),
                ("pct", &pct),
                ("when", &fmt.format_when(t, now_ms, tr)),
            ],
        ),
        None => format!("{} {pct}%", w.name),
    }
}

fn limit_footer(
    lim: &AccountLimits,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> (String, Color) {
    let max_pct = lim.windows.iter().map(|w| w.pct).fold(0.0, f64::max);
    let cols: Vec<String> = lim
        .windows
        .iter()
        .map(|w| format_window(w, now_ms, fmt, tr))
        .collect();
    (
        format!("{:<7} {}", lim.provider, cols.join("   ")),
        pct_color(max_pct),
    )
}

fn quota_footer(
    q: &ProviderQuota,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> (String, Color) {
    if !q.exhausted {
        let text = tr.format(
            "{provider}     quota ok ({provider} only reports when exhausted)",
            &[("provider", &q.provider)],
        );
        return (text, COLOR_MUTED);
    }
    let at = fmt.format_time(q.since_ms);
    let when = fmt.format_when(q.resets_at_ms, now_ms, tr);
    let text = if q.model.is_empty() {
        tr.format(
            "{provider}     a quota ran out at {at} · resets {when} (other models continue)",
            &[("provider", &q.provider), ("at", &at), ("when", &when)],
        )
    } else {
        tr.format(
            "{provider}     {model} 0% · out at {at} · resets {when}",
            &[
                ("provider", &q.provider),
                ("model", &q.model),
                ("at", &at),
                ("when", &when),
            ],
        )
    };
    (text, COLOR_ALERT)
}

/// Builds footer lines from usage limits and quota notices.
pub fn build_footers(
    snap: &Snapshot,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> Vec<(String, Color)> {
    let mut out = Vec::new();
    if snap.limits.is_empty() {
        out.push((
            tr.translate("Claude  limits appear after the next Claude Code response")
                .into_owned(),
            COLOR_MUTED,
        ));
    } else {
        for lim in &snap.limits {
            out.push(limit_footer(lim, now_ms, fmt, tr));
        }
    }
    for q in &snap.quotas {
        out.push(quota_footer(q, now_ms, fmt, tr));
    }
    out
}

/// Maps a [`Snapshot`] into the Slint view-model.
pub fn build_panel_view(
    snap: &Snapshot,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> PanelView {
    let parents: std::collections::HashMap<&str, Option<&str>> = snap
        .agents
        .iter()
        .map(|a| (a.key.as_str(), a.parent_key.as_deref()))
        .collect();
    let agents = snap
        .agents
        .iter()
        .map(|a| {
            let mut ui = agent_to_ui(a, now_ms, tr);
            ui.depth = compute_depth(a.parent_key.as_deref(), &parents);
            ui
        })
        .collect();
    let footers = build_footers(snap, now_ms, fmt, tr)
        .into_iter()
        .map(|(text, color)| FooterLine {
            text: text.into(),
            color,
        })
        .collect();
    PanelView { agents, footers }
}

fn compute_depth<'a>(
    mut parent: Option<&'a str>,
    parents: &std::collections::HashMap<&str, Option<&'a str>>,
) -> i32 {
    let mut d = 0;
    while let Some(pk) = parent.filter(|k| parents.contains_key(*k) && d < 2) {
        d += 1;
        parent = parents.get(pk).copied().flatten();
    }
    d
}

/// Builds the tray icon tooltip lines, active count, and status color.
pub fn build_tray_view(
    snap: &Snapshot,
    now_ms: u64,
    fmt: &dyn TimeFormatter,
    tr: &dyn Translator,
) -> TrayView {
    let mut lines: Vec<String> = snap
        .agents
        .iter()
        .filter(|a| a.state != AgentState::Stopped)
        .map(|a| {
            let prefix = if a.parent_key.is_some() { "  ↳ " } else { "" };
            format!(
                "{prefix}{} — {}: {}",
                a.title(),
                tr.translate(a.state.label_key()),
                a.now
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(tr.translate("no active agents").into_owned());
    }
    for (text, _) in build_footers(snap, now_ms, fmt, tr) {
        lines.push(text);
    }
    let working_count = snap
        .agents
        .iter()
        .filter(|a| a.state == AgentState::Working)
        .count();
    let color = if snap
        .agents
        .iter()
        .any(|a| a.state == AgentState::Waiting && !a.reason.is_empty())
    {
        [0xff, 0xb3, 0x47]
    } else if snap.agents.iter().any(|a| a.state == AgentState::Error) {
        [0xff, 0x5c, 0x5c]
    } else if working_count > 0 {
        [0xe8, 0xe8, 0xe8]
    } else {
        [0x8a, 0x8a, 0x8a]
    };
    TrayView {
        lines,
        working_count,
        color,
    }
}

#[cfg(test)]
mod tests;
