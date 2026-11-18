// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Focuses an open terminal running an agent or launches the system default terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use agents_roster_core::{Agent, ProcessTable, TerminalOpener};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConfigureWindowAux, ConnectionExt,
    EventMask, StackMode,
};

/// Cross-desktop [`TerminalOpener`] that prefers focusing an open X11 terminal window.
pub struct SystemTerminalOpener {
    procs: Arc<dyn ProcessTable>,
}

impl SystemTerminalOpener {
    /// Creates a terminal opener backed by `procs` for ancestor PID resolution.
    pub fn new(procs: Arc<dyn ProcessTable>) -> Self {
        Self { procs }
    }
}

impl TerminalOpener for SystemTerminalOpener {
    fn open_agent_terminal(&self, agent: &Agent) {
        if let Some(pid) = agent.pid {
            let ancestors = self.procs.ancestor_pids(pid);
            if !ancestors.is_empty() && focus_x11_window(&ancestors, &agent.project) {
                return;
            }
        }
        let dir = resolve_working_dir(&agent.working_dir);
        if !launch_default_terminal(&dir) {
            tracing::warn!(
                "no supported terminal emulator found to open {}",
                dir.display()
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientWindow {
    id: u32,
    pid: u32,
    title: String,
    desktop: Option<u32>,
}

fn pick_best_window(
    windows: &[ClientWindow],
    ancestors: &[u32],
    project: &str,
) -> Option<ClientWindow> {
    let mut best: Option<(usize, bool, &ClientWindow)> = None;
    for win in windows {
        let Some(rank) = ancestors.iter().position(|&p| p == win.pid) else {
            continue;
        };
        let title_hit = !project.is_empty() && win.title.contains(project);
        let better = match best {
            None => true,
            Some((best_rank, best_hit, _)) => {
                rank < best_rank || (rank == best_rank && title_hit && !best_hit)
            }
        };
        if better {
            best = Some((rank, title_hit, win));
        }
    }
    best.map(|(_, _, win)| win.clone())
}

fn focus_x11_window(ancestors: &[u32], project: &str) -> bool {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let windows = read_client_windows(&conn, root);
    let Some(target) = pick_best_window(&windows, ancestors, project) else {
        return false;
    };
    activate_x11_window(&conn, root, &target).is_ok()
}

fn read_client_windows<C: Connection>(conn: &C, root: u32) -> Vec<ClientWindow> {
    let Ok(client_list) = intern(conn, b"_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    let Ok(wm_pid) = intern(conn, b"_NET_WM_PID") else {
        return Vec::new();
    };
    let Ok(net_name) = intern(conn, b"_NET_WM_NAME") else {
        return Vec::new();
    };
    let Ok(utf8) = intern(conn, b"UTF8_STRING") else {
        return Vec::new();
    };
    let Ok(wm_desktop) = intern(conn, b"_NET_WM_DESKTOP") else {
        return Vec::new();
    };

    let Ok(reply) = conn.get_property(false, root, client_list, AtomEnum::WINDOW, 0, 1024) else {
        return Vec::new();
    };
    let Ok(prop) = reply.reply() else {
        return Vec::new();
    };
    let Some(ids) = prop.value32() else {
        return Vec::new();
    };
    ids.filter_map(|id| {
        let pid = read_u32_prop(conn, id, wm_pid, AtomEnum::CARDINAL)?;
        let title = read_title(conn, id, net_name, utf8);
        let desktop = read_u32_prop(conn, id, wm_desktop, AtomEnum::CARDINAL);
        Some(ClientWindow {
            id,
            pid,
            title,
            desktop,
        })
    })
    .collect()
}

fn activate_x11_window<C: Connection>(
    conn: &C,
    root: u32,
    target: &ClientWindow,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(desktop) = target.desktop
        && desktop != u32::MAX
    {
        let cur_desk = intern(conn, b"_NET_CURRENT_DESKTOP")?;
        if read_u32_prop(conn, root, cur_desk, AtomEnum::CARDINAL) != Some(desktop) {
            send_client_msg(conn, root, root, cur_desk, [desktop, 0, 0, 0, 0])?;
        }
    }
    let active_atom = intern(conn, b"_NET_ACTIVE_WINDOW")?;
    let _ = conn.map_window(target.id);
    let _ = conn.configure_window(
        target.id,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    send_client_msg(conn, root, target.id, active_atom, [2, 0, 0, 0, 0])?;
    conn.flush()?;
    Ok(())
}

fn intern<C: Connection>(conn: &C, name: &[u8]) -> Result<Atom, Box<dyn std::error::Error>> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

fn read_u32_prop<C: Connection, T: Into<Atom>>(
    conn: &C,
    win: u32,
    prop: Atom,
    kind: T,
) -> Option<u32> {
    let reply = conn
        .get_property(false, win, prop, kind, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32()?.next()
}

fn read_title<C: Connection>(conn: &C, win: u32, net_name: Atom, utf8: Atom) -> String {
    if let Ok(cookie) = conn.get_property(false, win, net_name, utf8, 0, 256)
        && let Ok(reply) = cookie.reply()
        && !reply.value.is_empty()
    {
        return String::from_utf8_lossy(&reply.value).into_owned();
    }
    if let Ok(cookie) = conn.get_property(false, win, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 256)
        && let Ok(reply) = cookie.reply()
    {
        return String::from_utf8_lossy(&reply.value).into_owned();
    }
    String::new()
}

fn send_client_msg<C: Connection>(
    conn: &C,
    root: u32,
    win: u32,
    kind: Atom,
    data: [u32; 5],
) -> Result<(), Box<dyn std::error::Error>> {
    let event = ClientMessageEvent::new(32, win, kind, ClientMessageData::from(data));
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
        event,
    )?
    .check()?;
    Ok(())
}

fn resolve_working_dir(raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if !raw.is_empty() && path.is_dir() {
        return path.to_path_buf();
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn linux_terminal_candidates(env_terminal: Option<&str>, desktop: &str) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(term) = env_terminal.map(str::trim).filter(|s| !s.is_empty()) {
        list.push(term.to_string());
    }
    list.push("xdg-terminal-exec".into());
    list.push("x-terminal-emulator".into());
    let upper = desktop.to_ascii_uppercase();
    if upper.contains("KDE") {
        list.push("konsole".into());
    } else if upper.contains("GNOME") {
        list.extend(["kgx".into(), "ptyxis".into(), "gnome-terminal".into()]);
    } else if upper.contains("XFCE") {
        list.push("xfce4-terminal".into());
    }
    for bin in [
        "konsole",
        "gnome-terminal",
        "kgx",
        "ptyxis",
        "xfce4-terminal",
        "alacritty",
        "kitty",
        "ghostty",
        "wezterm",
        "foot",
        "tilix",
        "terminator",
        "mate-terminal",
        "lxterminal",
        "xterm",
    ] {
        if !list.iter().any(|x| x == bin) {
            list.push(bin.into());
        }
    }
    list
}

fn launch_default_terminal(dir: &Path) -> bool {
    #[cfg(target_os = "macos")]
    return Command::new("open")
        .args(["-a", "Terminal"])
        .arg(dir)
        .spawn()
        .is_ok();

    #[cfg(target_os = "windows")]
    return Command::new("cmd")
        .args(["/C", "start", "wt", "-d"])
        .arg(dir)
        .spawn()
        .is_ok()
        || Command::new("cmd")
            .args(["/C", "start", "cmd"])
            .current_dir(dir)
            .spawn()
            .is_ok();

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let env_term = std::env::var("TERMINAL").ok();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        for candidate in linux_terminal_candidates(env_term.as_deref(), &desktop) {
            let mut parts = candidate.split_whitespace();
            let Some(bin) = parts.next() else { continue };
            let args: Vec<&str> = parts.collect();
            if Command::new(bin)
                .args(args)
                .current_dir(dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
