// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Per-agent actions from the card context menu.
//!
//! * Process control (`freeze`, `resume`, `kill`) acts on the agent's PID.
//! * Input actions (`continue`, `faster`, `interrupt`) focus the agent's *existing* terminal
//!   and inject keystrokes. They never type anything unless that window was found and focused.

use agents_roster_core::{Agent, TerminalOpener};

const TEXT_CONTINUE: &str = "continue";
const TEXT_FASTER: &str = "Please work faster: be concise, skip unnecessary exploration and \
commentary, and go straight to the result.";

/// Runs `action` (`open|continue|faster|interrupt|freeze|resume|kill`) for `agent`.
/// Blocking; call from a worker thread.
pub fn run(action: &str, agent: &Agent, opener: &dyn TerminalOpener) {
    match action {
        "open" => opener.open_agent_terminal(agent),
        "continue" => type_into_terminal(agent, opener, &Keys::Text(TEXT_CONTINUE)),
        "faster" => type_into_terminal(agent, opener, &Keys::Text(TEXT_FASTER)),
        "interrupt" => type_into_terminal(agent, opener, &Keys::Escape),
        "freeze" | "resume" | "kill" => process_action(action, agent),
        other => tracing::warn!("unknown agent action {other:?}"),
    }
}

enum Keys {
    Text(&'static str),
    Escape,
}

fn type_into_terminal(agent: &Agent, opener: &dyn TerminalOpener, keys: &Keys) {
    if !opener.focus_agent_terminal(agent) {
        tracing::warn!("no open terminal found for {}; not sending input", agent.project);
        return;
    }
    // Give the window manager a moment to hand over keyboard focus.
    std::thread::sleep(std::time::Duration::from_millis(300));
    imp::send(keys);
}

fn process_action(action: &str, agent: &Agent) {
    let Some(pid) = agent.pid else {
        tracing::warn!("{} has no known PID; cannot {action}", agent.project);
        return;
    };
    let result = match action {
        "freeze" => imp::suspend(pid, true),
        "resume" => imp::suspend(pid, false),
        _ => {
            if !imp::confirm_kill(agent, pid) {
                return;
            }
            imp::kill(pid)
        }
    };
    if let Err(e) = result {
        tracing::warn!("{action} of pid {pid} failed: {e}");
    }
}

#[cfg(windows)]
mod imp {
    #![allow(unsafe_code)] // Win32 FFI: audited, confined to this module.

    use super::{Agent, Keys};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, TerminateProcess,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VK_ESCAPE, VK_RETURN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_TOPMOST, MB_YESNO, MessageBoxW,
    };
    use windows::core::{HSTRING, s, w};

    type NtProc = unsafe extern "system" fn(HANDLE) -> i32;

    pub fn suspend(pid: u32, freeze: bool) -> Result<(), String> {
        let ntdll = unsafe { GetModuleHandleW(w!("ntdll.dll")) }.map_err(|e| e.to_string())?;
        let name = if freeze { s!("NtSuspendProcess") } else { s!("NtResumeProcess") };
        let proc = unsafe { GetProcAddress(ntdll, name) }.ok_or("ntdll export missing")?;
        let f = unsafe { core::mem::transmute::<unsafe extern "system" fn() -> isize, NtProc>(proc) };
        let h = unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, false, pid) }.map_err(|e| e.to_string())?;
        let status = unsafe { f(h) };
        let _ = unsafe { CloseHandle(h) };
        if status < 0 { Err(format!("NTSTATUS {status:#x}")) } else { Ok(()) }
    }

    pub fn kill(pid: u32) -> Result<(), String> {
        let h = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.map_err(|e| e.to_string())?;
        let r = unsafe { TerminateProcess(h, 1) }.map_err(|e| e.to_string());
        let _ = unsafe { CloseHandle(h) };
        r
    }

    pub fn confirm_kill(agent: &Agent, pid: u32) -> bool {
        let sub = if agent.parent_key.is_some() {
            "\n\nThis is a sub-agent: its PID may belong to the parent session, which would end too."
        } else {
            ""
        };
        let text = HSTRING::from(format!(
            "Kill {} \"{}\" (PID {pid})?\n\nUnsaved work in that session is lost.{sub}",
            agent.source_label, agent.project
        ));
        let r = unsafe {
            MessageBoxW(
                None,
                &text,
                w!("Agents Roster"),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST,
            )
        };
        r == IDYES
    }

    fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    pub fn send(keys: &Keys) {
        let mut inputs: Vec<INPUT> = Vec::new();
        let tap = |inputs: &mut Vec<INPUT>, vk| {
            inputs.push(key(vk, 0, KEYBD_EVENT_FLAGS(0)));
            inputs.push(key(vk, 0, KEYEVENTF_KEYUP));
        };
        match keys {
            Keys::Escape => tap(&mut inputs, VK_ESCAPE),
            Keys::Text(t) => {
                for unit in t.encode_utf16() {
                    inputs.push(key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
                    inputs.push(key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
                tap(&mut inputs, VK_RETURN);
            }
        }
        let sent = unsafe { SendInput(&inputs, core::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            tracing::warn!("SendInput delivered {sent}/{} events", inputs.len());
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Agent, Keys};
    use std::process::Command;

    fn signal(sig: &str, pid: u32) -> Result<(), String> {
        let ok = Command::new("kill")
            .args([sig, &pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?
            .success();
        if ok { Ok(()) } else { Err(format!("kill {sig} failed")) }
    }

    pub fn suspend(pid: u32, freeze: bool) -> Result<(), String> {
        signal(if freeze { "-STOP" } else { "-CONT" }, pid)
    }

    pub fn kill(pid: u32) -> Result<(), String> {
        signal("-KILL", pid)
    }

    pub fn confirm_kill(_agent: &Agent, _pid: u32) -> bool {
        tracing::warn!("kill confirmation dialog is Windows-only; refusing");
        false
    }

    pub fn send(_keys: &Keys) {
        tracing::warn!("keystroke injection is only implemented on Windows");
    }
}
