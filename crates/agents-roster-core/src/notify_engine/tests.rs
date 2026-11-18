// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use super::*;
use crate::model::{Agent, AgentState, DoneItem, UsageWindow};
use crate::traits::IdentityTranslator;

struct FakeTimeFmt;

impl TimeFormatter for FakeTimeFmt {
    fn format_time(&self, ms: u64) -> String {
        format!("T{ms}")
    }

    fn format_when(&self, ms: u64, _now_ms: u64, _tr: &dyn Translator) -> String {
        format!("W{ms}")
    }
}

fn agent(key: &str, state: AgentState) -> Agent {
    Agent {
        key: key.into(),
        source_id: "agy".into(),
        source_label: "agy".into(),
        project: "proj".into(),
        state,
        now: "running tests".into(),
        ..Default::default()
    }
}

#[test]
fn suppresses_startup_flood_then_notifies_new_working_agent() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);

    let s1 = Snapshot {
        agents: vec![agent("agy:1", AgentState::Working)],
        ..Default::default()
    };
    assert!(engine.process(&s1, 1000, 0, &fmt, &tr).is_empty());

    let mut s2 = s1;
    s2.agents.push(agent("agy:2", AgentState::Working));
    let notes = engine.process(&s2, 2000, 0, &fmt, &tr);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "agy · proj started");
    assert!(!notes[0].urgent);
}

#[test]
fn suppresses_short_replies_and_notifies_long_replies_and_commits() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    let s1 = Snapshot {
        agents: vec![agent("claude:1", AgentState::Working)],
        ..Default::default()
    };
    assert!(engine.process(&s1, 1000, 0, &fmt, &tr).is_empty());

    let cases = [
        (
            DoneItem {
                text: "replied: ok".into(),
                at_ms: 10,
                is_reply: true,
                turn_duration_ms: Some(30_000),
            },
            0,
        ),
        (
            DoneItem {
                text: "replied: done".into(),
                at_ms: 20,
                is_reply: true,
                turn_duration_ms: Some(150_000),
            },
            1,
        ),
        (
            DoneItem {
                text: "commit: feat".into(),
                at_ms: 30,
                is_reply: false,
                turn_duration_ms: None,
            },
            1,
        ),
    ];
    let mut snap = s1;
    for (item, expected_count) in cases {
        snap.agents[0].done.insert(0, item);
        let notes = engine.process(&snap, 2000, 0, &fmt, &tr);
        assert_eq!(notes.len(), expected_count);
    }
}

#[test]
fn notifies_when_active_agent_disappears_into_recently_ended() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    let s1 = Snapshot {
        agents: vec![agent("agy:1", AgentState::Working)],
        ..Default::default()
    };
    engine.process(&s1, 1000, 0, &fmt, &tr);

    let mut ended = agent("agy:1", AgentState::Error);
    ended.reason = "quota exceeded".into();
    let s2 = Snapshot {
        agents: vec![],
        recently_ended: vec![ended],
        ..Default::default()
    };
    let notes = engine.process(&s2, 2000, 0, &fmt, &tr);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "agy · proj stopped");
    assert_eq!(notes[0].body, "quota exceeded");
    assert!(notes[0].urgent);
}

#[test]
fn table_driven_threshold_crossings_and_rearming() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    engine.process(&Snapshot::default(), 1000, 0, &fmt, &tr);

    let steps = [
        (79.9, 0, false),
        (80.0, 1, false),
        (85.0, 0, false),
        (92.0, 1, false),
        (96.0, 1, true),
        (50.0, 0, false),
        (81.0, 1, false),
    ];
    for (pct, expected_len, expected_urgent) in steps {
        let snap = Snapshot {
            limits: vec![AccountLimits {
                provider: "Claude".into(),
                windows: vec![UsageWindow {
                    name: "5h".into(),
                    long_name_key: "5 hours".into(),
                    pct,
                    resets_at_ms: Some(9000),
                }],
                updated_at_ms: 1000,
            }],
            ..Default::default()
        };
        let notes = engine.process(&snap, 2000, 0, &fmt, &tr);
        assert_eq!(notes.len(), expected_len, "pct={pct}");
        if expected_len > 0 {
            assert_eq!(notes[0].urgent, expected_urgent, "pct={pct}");
        }
    }
}

#[test]
fn fires_periodic_reminder_only_when_agents_working() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    let snap = Snapshot {
        agents: vec![agent("agy:1", AgentState::Working)],
        ..Default::default()
    };
    engine.process(&snap, 10_000, 15, &fmt, &tr);
    assert!(
        engine
            .process(&snap, 10_000 + 14 * 60_000, 15, &fmt, &tr)
            .is_empty()
    );
    let notes = engine.process(&snap, 10_000 + 15 * 60_000, 15, &fmt, &tr);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title, "1 agent working");
}

#[test]
fn notifies_each_exhausted_model_quota_independently() {
    let mut engine = NotificationEngine::default();
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    engine.process(&Snapshot::default(), 1000, 0, &fmt, &tr);

    let argon = ProviderQuota {
        provider: "agy".into(),
        model: "Gemini 4 Argon".into(),
        exhausted: true,
        since_ms: 1000,
        resets_at_ms: 9000,
    };
    let notes1 = engine.process(
        &Snapshot {
            quotas: vec![argon.clone()],
            ..Default::default()
        },
        2000,
        0,
        &fmt,
        &tr,
    );
    assert_eq!(notes1.len(), 1);
    assert_eq!(notes1[0].title, "agy · Gemini 4 Argon quota exhausted");

    let sonnet = ProviderQuota {
        provider: "agy".into(),
        model: "Claude Sonnet 5.5".into(),
        exhausted: true,
        since_ms: 3000,
        resets_at_ms: 8000,
    };
    let notes2 = engine.process(
        &Snapshot {
            quotas: vec![argon, sonnet],
            ..Default::default()
        },
        4000,
        0,
        &fmt,
        &tr,
    );
    assert_eq!(notes2.len(), 1);
    assert_eq!(notes2[0].title, "agy · Claude Sonnet 5.5 quota exhausted");
}

#[test]
fn respects_widget_only_notification_prefs() {
    use crate::settings::NotificationPrefs;
    let mut engine = NotificationEngine::default();
    engine.set_prefs(NotificationPrefs {
        os_on_started: false,
        os_on_finished: false,
        os_on_waiting: true,
        ..Default::default()
    });
    let (fmt, tr) = (FakeTimeFmt, IdentityTranslator);
    engine.process(&Snapshot::default(), 1000, 0, &fmt, &tr);
    let started = Snapshot {
        agents: vec![agent("claude:1", AgentState::Working)],
        ..Default::default()
    };
    assert!(engine.process(&started, 2000, 0, &fmt, &tr).is_empty());

    let mut waiting = agent("claude:1", AgentState::Waiting);
    waiting.reason = "Approve command".into();
    let notes = engine.process(
        &Snapshot {
            agents: vec![waiting],
            ..Default::default()
        },
        3000,
        0,
        &fmt,
        &tr,
    );
    assert_eq!(notes.len(), 1);
    assert!(notes[0].title.contains("needs you"));
}
