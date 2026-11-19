// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Windows 11 notification-area (tray) backend: `Shell_NotifyIcon` on a dedicated thread.
//!
//! Mirrors the Linux `ksni` architecture: the icon lives on its own thread with a Win32
//! message pump; [`TrayHandle::update`] mutates shared state and posts a refresh message.
//! The icon re-adds itself when Explorer restarts (`TaskbarCreated`).

#![allow(unsafe_code)] // Win32 FFI: audited, confined to this module.

use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, HICON, KillTimer, LoadCursorW, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY,
    WM_NCCREATE, WM_NULL, WM_TIMER, WNDCLASSW, WS_POPUP,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NINF_KEY, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::core::{PCWSTR, w};

use crate::tray::{TrayCommand, TrayIcon, render_icon};

const WM_TRAY_CALLBACK: u32 = 0x8000 + 16; // WM_APP + 16
const WM_TRAY_REFRESH: u32 = 0x8000 + 1;
const WM_TRAY_QUIT: u32 = 0x8000 + 2;
const TRAY_UID: u32 = 1;
const READD_TIMER: usize = 1;

const MENU_SHOW_HIDE: usize = 1;
const MENU_ALWAYS_ON_TOP: usize = 2;
const MENU_REMINDERS: usize = 3;
const MENU_SETTINGS: usize = 4;
const MENU_QUIT: usize = 5;

/// Per-window state, owned by the tray thread (boxed into `GWLP_USERDATA`).
struct TrayWnd {
    state: Arc<Mutex<TrayIcon>>,
    icon: HICON,
    taskbar_created: u32,
    nid: NOTIFYICONDATAW,
}

/// Handle to the spawned tray icon; safe to use from any thread (updates post messages).
pub struct TrayHandle {
    hwnd: isize,
    state: Arc<Mutex<TrayIcon>>,
    thread: Option<JoinHandle<()>>,
}

impl TrayHandle {
    /// Mutates the shared tray state and schedules an icon/tooltip refresh.
    pub fn update(&self, f: impl FnOnce(&mut TrayIcon)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
        let _ = unsafe {
            PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_TRAY_REFRESH, WPARAM(0), LPARAM(0))
        };
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        let _ = unsafe {
            PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_TRAY_QUIT, WPARAM(0), LPARAM(0))
        };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Spawns the tray icon thread; returns `None` when the notification area is unavailable.
pub fn spawn_tray(state: TrayIcon) -> Option<TrayHandle> {
    let state = Arc::new(Mutex::new(state));
    let thread_state = Arc::clone(&state);
    let (ready_tx, ready_rx) = mpsc::channel::<isize>();
    let thread = std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || tray_thread(thread_state, &ready_tx))
        .ok()?;
    let hwnd = ready_rx.recv().unwrap_or(0);
    if hwnd == 0 {
        let _ = thread.join();
        return None;
    }
    Some(TrayHandle { hwnd, state, thread: Some(thread) })
}

fn tray_thread(state: Arc<Mutex<TrayIcon>>, ready: &mpsc::Sender<isize>) {
    let class_name = w!("AgentsRosterTrayWnd");
    let hinstance = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => HINSTANCE(h.0),
        Err(_) => {
            let _ = ready.send(0);
            return;
        }
    };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        lpszClassName: class_name,
        hCursor: unsafe { LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW) }
            .unwrap_or_default(),
        ..Default::default()
    };
    // A pre-existing class (previous instance in this process) is fine.
    let _ = unsafe { RegisterClassW(&wc) };

    let wnd = Box::new(TrayWnd {
        state,
        icon: HICON::default(),
        taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        nid: NOTIFYICONDATAW::default(),
    });
    let ptr = Box::into_raw(wnd);
    // A regular hidden window (not HWND_MESSAGE): it must receive the TaskbarCreated broadcast
    // and be able to own the context menu.
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            w!("Agents Roster"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            Some(ptr.cast()),
        )
    } {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("tray window creation failed: {e}");
            drop(unsafe { Box::from_raw(ptr) });
            let _ = ready.send(0);
            return;
        }
    };

    let added = unsafe { add_icon(hwnd) };
    if !added {
        // Explorer may not be up yet; retry periodically until TaskbarCreated arrives.
        let _ = unsafe { SetTimer(Some(hwnd), READD_TIMER, 5000, None) };
    }
    let _ = ready.send(hwnd.0 as isize);

    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 <= 0 {
            break; // 0 = WM_QUIT, -1 = error
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayWnd;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let wnd = unsafe { &mut *ptr };

    match msg {
        WM_TRAY_CALLBACK => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == NIN_SELECT || event == (NIN_SELECT | NINF_KEY) {
                let _ = wnd.state.lock().map(|s| s.tx.send(TrayCommand::ToggleVisible));
            } else if event == WM_CONTEXTMENU {
                unsafe { show_menu(hwnd, wnd) };
            }
            LRESULT(0)
        }
        WM_TRAY_REFRESH => {
            unsafe { refresh_icon(hwnd, wnd) };
            LRESULT(0)
        }
        WM_COMMAND => {
            unsafe { menu_command(wnd, wparam.0 & 0xFFFF) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == READD_TIMER => {
            if unsafe { add_icon(hwnd) } {
                let _ = unsafe { KillTimer(Some(hwnd), READD_TIMER) };
            }
            LRESULT(0)
        }
        WM_TRAY_QUIT => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &wnd.nid) };
            if !wnd.icon.is_invalid() {
                let _ = unsafe { DestroyIcon(wnd.icon) };
            }
            drop(unsafe { Box::from_raw(ptr) });
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if msg == wnd.taskbar_created && msg != 0 => {
            let _ = unsafe { add_icon(hwnd) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Builds the 32x32 badge HICON from the shared ARGB renderer (BGRA bottom-up + AND mask).
fn build_hicon(wnd: &TrayWnd) -> Option<HICON> {
    let (count, color) = {
        let s = wnd.state.lock().ok()?;
        (s.working_count, s.color)
    };
    let d = render_icon(count, color);
    let size = d.width as usize;
    let mut xor = vec![0u8; size * size * 4];
    let mut and = vec![0u8; size * size / 8];
    for y in 0..size {
        for x in 0..size {
            let src = (y * size + x) * 4;
            let dst = ((size - 1 - y) * size + x) * 4; // bottom-up
            let [a, r, g, b] = [d.data[src], d.data[src + 1], d.data[src + 2], d.data[src + 3]];
            xor[dst..dst + 4].copy_from_slice(&[b, g, r, a]);
            if a < 128 {
                and[y * (size / 8) + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    unsafe { CreateIcon(None, d.width, d.height, 1, 32, and.as_ptr(), xor.as_ptr()) }.ok()
}

fn tooltip_text(wnd: &TrayWnd) -> String {
    let Ok(s) = wnd.state.lock() else {
        return "Agents Roster".into();
    };
    let text = if s.lines.is_empty() {
        s.tr.translate("Agents").into_owned()
    } else {
        s.lines.join(" | ")
    };
    text.chars().take(127).collect()
}

fn fill_nid(hwnd: HWND, wnd: &mut TrayWnd) {
    let tip: Vec<u16> = tooltip_text(wnd).encode_utf16().take(127).collect();
    let icon = wnd.icon;
    let nid = &mut wnd.nid;
    nid.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_UID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = icon;
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid.szTip[tip.len()] = 0;
}

/// (Re-)adds the icon to the notification area and selects version-4 callback semantics.
unsafe fn add_icon(hwnd: HWND) -> bool {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayWnd;
    if ptr.is_null() {
        return false;
    }
    let wnd = unsafe { &mut *ptr };
    if let Some(h) = build_hicon(wnd) {
        if !wnd.icon.is_invalid() {
            let _ = unsafe { DestroyIcon(wnd.icon) };
        }
        wnd.icon = h;
    }
    fill_nid(hwnd, wnd);
    if unsafe { Shell_NotifyIconW(NIM_ADD, &wnd.nid) }.as_bool() == false {
        return false;
    }
    wnd.nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &wnd.nid) };
    true
}

unsafe fn refresh_icon(hwnd: HWND, wnd: &mut TrayWnd) {
    if let Some(h) = build_hicon(wnd) {
        let old = wnd.icon;
        wnd.icon = h;
        fill_nid(hwnd, wnd);
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &wnd.nid) };
        if !old.is_invalid() {
            let _ = unsafe { DestroyIcon(old) };
        }
    } else {
        fill_nid(hwnd, wnd);
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &wnd.nid) };
    }
}

unsafe fn show_menu(hwnd: HWND, wnd: &TrayWnd) {
    let Ok(s) = wnd.state.lock() else { return };
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
    let label = |key: &str| -> Vec<u16> {
        s.tr.translate(key).encode_utf16().chain(std::iter::once(0)).collect()
    };
    let (show, top, rem, settings, quit) = (
        label("Show/hide"),
        label("Always on top"),
        label("Reminders while working"),
        label("Control Panel..."),
        label("Quit"),
    );
    let top_flags = MF_STRING | if s.always_on_top { MF_CHECKED } else { Default::default() };
    let rem_flags = MF_STRING | if s.reminders { MF_CHECKED } else { Default::default() };
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, MENU_SHOW_HIDE, PCWSTR(show.as_ptr()));
        let _ = AppendMenuW(menu, top_flags, MENU_ALWAYS_ON_TOP, PCWSTR(top.as_ptr()));
        let _ = AppendMenuW(menu, rem_flags, MENU_REMINDERS, PCWSTR(rem.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, PCWSTR(settings.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit.as_ptr()));
    }
    drop(s);
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    // Required so the menu dismisses when the user clicks elsewhere (KB135788).
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let _ = unsafe { TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None) };
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
    let _ = unsafe { DestroyMenu(menu) };
}

unsafe fn menu_command(wnd: &mut TrayWnd, id: usize) {
    let Ok(mut s) = wnd.state.lock() else { return };
    match id {
        MENU_SHOW_HIDE => {
            let _ = s.tx.send(TrayCommand::ToggleVisible);
        }
        MENU_ALWAYS_ON_TOP => {
            s.always_on_top = !s.always_on_top;
            let _ = s.tx.send(TrayCommand::AlwaysOnTop(s.always_on_top));
        }
        MENU_REMINDERS => {
            s.reminders = !s.reminders;
            let _ = s.tx.send(TrayCommand::Reminders(s.reminders));
        }
        MENU_SETTINGS => {
            let _ = s.tx.send(TrayCommand::OpenSettings);
        }
        MENU_QUIT => {
            let _ = s.tx.send(TrayCommand::Quit);
        }
        _ => {}
    }
}
