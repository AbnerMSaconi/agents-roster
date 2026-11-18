// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use super::*;
use agents_roster_core::IdentityTranslator;

struct FakeFmt;
impl TimeFormatter for FakeFmt {
    fn format_time(&self, ms: u64) -> String {
        format!("T{ms}")
    }
    fn format_when(&self, ms: u64, _now: u64, _tr: &dyn Translator) -> String {
        format!("W{ms}")
    }
}

#[test]
fn builds_tray_summary_for_working_and_empty_snapshots() {
    let empty = build_tray_view(&Snapshot::default(), 1000, &FakeFmt, &IdentityTranslator);
    assert_eq!(empty.working_count, 0);
    assert_eq!(empty.lines[0], "no active agents");

    let snap = Snapshot {
        agents: vec![Agent {
            key: "agy:1".into(),
            source_label: "agy".into(),
            project: "app".into(),
            state: AgentState::Working,
            now: "building".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let active = build_tray_view(&snap, 1000, &FakeFmt, &IdentityTranslator);
    assert_eq!(active.working_count, 1);
    assert_eq!(active.color, [0xe8, 0xe8, 0xe8]);
    assert!(active.lines[0].contains("agy · app — working: building"));
}

#[test]
fn lists_each_exhausted_agy_model_quota_in_footers() {
    use agents_roster_core::ProviderQuota;
    let snap = Snapshot {
        quotas: vec![ProviderQuota {
            provider: "agy".into(),
            model: "Gemini 4 Argon".into(),
            exhausted: true,
            since_ms: 100,
            resets_at_ms: 900,
        }],
        ..Default::default()
    };
    let footers = build_footers(&snap, 200, &FakeFmt, &IdentityTranslator);
    assert_eq!(footers.len(), 2);
    assert_eq!(
        footers[1].0,
        "agy     Gemini 4 Argon 0% · out at T100 · resets W900"
    );
}

#[test]
fn computes_nested_depth_and_tray_prefix() {
    let snap = Snapshot {
        agents: vec![
            Agent {
                key: "agy:1".into(),
                source_label: "agy".into(),
                project: "app".into(),
                state: AgentState::Working,
                now: "working".into(),
                ..Default::default()
            },
            Agent {
                key: "claude:child".into(),
                parent_key: Some("agy:1".into()),
                source_label: "Claude Code".into(),
                project: "app".into(),
                state: AgentState::Working,
                now: "testing".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let panel = build_panel_view(&snap, 1000, &FakeFmt, &IdentityTranslator);
    assert_eq!(panel.agents[0].depth, 0);
    assert_eq!(panel.agents[1].depth, 1);
    let tray = build_tray_view(&snap, 1000, &FakeFmt, &IdentityTranslator);
    assert!(tray.lines[1].starts_with("  ↳ "));
}
