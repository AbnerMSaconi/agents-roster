// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Helper notification rules for [`crate::NotificationEngine`].

use crate::model::{Agent, AgentState, Notification};
use crate::notify_engine::LONG_TURN_MS;
use crate::settings::NotificationPrefs;
use crate::traits::Translator;

pub(crate) fn emit_new_done_items(
    agent: &Agent,
    prev: &Agent,
    prefs: &NotificationPrefs,
    tr: &dyn Translator,
    out: &mut Vec<Notification>,
) {
    if !prefs.os_on_done_item {
        return;
    }
    let prev_top = prev.done.first().map_or(0, |d| d.at_ms);
    let title = agent.title();
    for item in agent.done.iter().filter(|d| d.at_ms > prev_top).rev() {
        if item.is_reply && item.turn_duration_ms.is_some_and(|ms| ms < LONG_TURN_MS) {
            continue;
        }
        out.push(Notification {
            title: tr.format("{title} finished", &[("title", &title)]),
            body: item.text.clone(),
            urgent: false,
        });
    }
}

pub(crate) fn emit_state_change(
    prev: AgentState,
    agent: &Agent,
    prefs: &NotificationPrefs,
    tr: &dyn Translator,
    out: &mut Vec<Notification>,
) {
    let title = agent.title();
    match agent.state {
        AgentState::Waiting if prefs.os_on_waiting && !agent.reason.is_empty() => {
            out.push(Notification {
                title: tr.format("{title} needs you", &[("title", &title)]),
                body: agent.reason.clone(),
                urgent: true,
            });
        }
        AgentState::Error if prefs.os_on_error => out.push(Notification {
            title: tr.format("{title} stopped", &[("title", &title)]),
            body: agent.reason.clone(),
            urgent: true,
        }),
        AgentState::Stopped if prefs.os_on_finished && prev == AgentState::Working => {
            out.push(Notification {
                title: tr.format("{title} ended", &[("title", &title)]),
                body: agent
                    .done
                    .first()
                    .map(|d| d.text.clone())
                    .unwrap_or_default(),
                urgent: false,
            });
        }
        _ => {}
    }
}
