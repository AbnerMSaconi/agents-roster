// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use super::*;

#[test]
fn picks_closest_ancestor_and_breaks_ties_by_project_title() {
    let windows = vec![
        ClientWindow {
            id: 1,
            pid: 500,
            title: "other — Konsole".into(),
            desktop: None,
        },
        ClientWindow {
            id: 2,
            pid: 500,
            title: "agents-roster : agy — Konsole".into(),
            desktop: None,
        },
        ClientWindow {
            id: 3,
            pid: 900,
            title: "outer".into(),
            desktop: None,
        },
    ];
    let picked = pick_best_window(&windows, &[700, 600, 500, 900], "agents-roster").unwrap();
    assert_eq!(picked.id, 2);
}

#[test]
fn orders_terminal_candidates_by_env_and_desktop() {
    let kde = linux_terminal_candidates(Some("kitty"), "KDE");
    assert_eq!(kde[0], "kitty");
    assert_eq!(kde[1], "xdg-terminal-exec");
    assert_eq!(kde[2], "x-terminal-emulator");
    assert_eq!(kde[3], "konsole");

    let gnome = linux_terminal_candidates(None, "ubuntu:GNOME");
    assert_eq!(gnome[0], "xdg-terminal-exec");
    assert_eq!(gnome[2], "kgx");
}
