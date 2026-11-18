// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! X11 window hints (`_NET_WM_WINDOW_TYPE_UTILITY`, `_NET_WM_STATE_SKIP_TASKBAR`,
//! and compositor blur `_KDE_NET_WM_BLUR_BEHIND_REGION`) using `winit` hooks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::event_loop::EventLoop;
use slint::winit_030::winit::platform::wayland::EventLoopBuilderExtWayland;
use slint::winit_030::winit::platform::x11::{
    EventLoopBuilderExtX11, WindowAttributesExtX11, WindowType,
};
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit::window::WindowLevel;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, PhysicalPosition};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
};
use x11rb::wrapper::ConnectionExt as _;

use crate::app::Panel;

const NET_WM_STATE_REMOVE: u32 = 0;
const NET_WM_STATE_ADD: u32 = 1;
const SOURCE_APPLICATION: u32 = 1;

/// Selects the `winit` backend and configures `Utility` X11 window type when `skip_taskbar` is `true`.
pub fn configure_backend(skip_taskbar: bool) {
    let forced = std::env::var("AGENTS_ROSTER_BACKEND").unwrap_or_default();
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    // Without an override winit picks Wayland when `WAYLAND_DISPLAY` is set and X11 otherwise.
    if matches!(forced.as_str(), "x11" | "wayland") {
        let mut builder = EventLoop::with_user_event();
        if forced == "x11" {
            EventLoopBuilderExtX11::with_x11(&mut builder);
        } else {
            EventLoopBuilderExtWayland::with_wayland(&mut builder);
        }
        selector = selector.with_winit_event_loop_builder(builder);
    }
    let result = selector
        .with_winit_window_attributes_hook(move |attrs| {
            if skip_taskbar {
                attrs.with_x11_window_type(vec![WindowType::Utility])
            } else {
                attrs
            }
        })
        .select();
    if let Err(e) = result {
        tracing::warn!("could not configure winit backend hook: {e}");
    }
}

/// Watches `winit` window events to apply taskbar and blur hints once the X11 window maps or resizes.
pub(crate) fn install_window_hooks(
    panel: &Panel,
    always_on_top: Arc<dyn Fn() -> bool>,
    skip_flag: Arc<AtomicBool>,
    blur_flag: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
) {
    let weak = panel.as_weak();
    panel.window().on_winit_window_event(move |_win, ev| {
        if let Some(p) = weak.upgrade()
            && p.window().has_winit_window()
        {
            match ev {
                WindowEvent::RedrawRequested if pending.swap(false, Ordering::AcqRel) => {
                    apply_always_on_top(&p, always_on_top());
                    apply_skip_taskbar(&p, skip_flag.load(Ordering::Relaxed));
                    apply_blur_behind(&p, blur_flag.load(Ordering::Relaxed));
                }
                WindowEvent::Resized(_) if blur_flag.load(Ordering::Relaxed) => {
                    apply_blur_behind(&p, true);
                }
                _ => {}
            }
        }
        EventResult::Propagate
    });
}

/// Applies the always-on-top window level (`_NET_WM_STATE_ABOVE` on X11) to the mapped window.
///
/// The level set while the window is being created is not honored by every window manager, so it is
/// asserted again once the window is mapped. Without this the setting reads as on but has no effect
/// until it is toggled by hand.
pub(crate) fn apply_always_on_top(panel: &Panel, on: bool) {
    let level = if on {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    };
    panel
        .window()
        .with_winit_window(|w| w.set_window_level(level));
}

/// Applies or removes `_NET_WM_STATE_SKIP_TASKBAR` and `_NET_WM_STATE_SKIP_PAGER` on X11.
pub(crate) fn apply_skip_taskbar(panel: &Panel, skip: bool) {
    let Some(win) = panel.window().with_winit_window(x11_window_id).flatten() else {
        return;
    };
    if let Err(e) = with_conn(|c, screen| send_net_wm_state(c, screen, win, skip)) {
        tracing::debug!("failed to update X11 skip_taskbar state: {e}");
    }
}

/// Whether the panel window lives on a Wayland connection (no client-side positioning there).
pub(crate) fn is_wayland(panel: &Panel) -> bool {
    panel
        .window()
        .with_winit_window(|w| {
            matches!(
                w.window_handle().map(|h| h.as_raw()),
                Ok(RawWindowHandle::Wayland(_))
            )
        })
        .unwrap_or(false)
}

/// Hands the pointer grab to the compositor for an interactive move (the only way to move a
/// toplevel on Wayland). Returns `false` when not on Wayland or the request was rejected.
pub(crate) fn start_native_move(panel: &Panel) -> bool {
    panel
        .window()
        .with_winit_window(|w| {
            matches!(
                w.window_handle().map(|h| h.as_raw()),
                Ok(RawWindowHandle::Wayland(_))
            ) && w.drag_window().is_ok()
        })
        .unwrap_or(false)
}

/// Nudges the window by one pixel and back so the compositor re-evaluates its blur region.
///
/// `KWin` only drops a stale blur after the hint is removed once the window moves; the software
/// renderer alone repaints dirty rectangles and would leave it in place.
pub(crate) fn refresh_compositor(panel: &Panel) {
    let pos = panel.window().position();
    panel
        .window()
        .set_position(PhysicalPosition::new(pos.x + 1, pos.y));
    panel.window().set_position(pos);
    panel.window().request_redraw();
}

/// Corner radius (logical px) of the panel background in `panel.slint`; keep in sync.
const PANEL_CORNER_RADIUS: f64 = 10.0;

/// Requests native compositor background blur via `_KDE_NET_WM_BLUR_BEHIND_REGION` on X11.
///
/// The blurred region follows the rounded panel shape so no blur leaks outside the corners.
pub(crate) fn apply_blur_behind(panel: &Panel, enabled: bool) {
    let Some((win, w, h, scale)) = panel
        .window()
        .with_winit_window(|w| {
            let id = x11_window_id(w)?;
            let sz = w.inner_size();
            Some((id, sz.width.max(1), sz.height.max(1), w.scale_factor()))
        })
        .flatten()
    else {
        return;
    };
    let region = if enabled {
        rounded_region(w, h, (PANEL_CORNER_RADIUS * scale).round() as u32)
    } else {
        Vec::new()
    };
    if let Err(e) = with_conn(|conn, _| send_blur_hint(conn, win, &region)) {
        tracing::debug!("failed to update X11 blur hint: {e}");
    }
}

/// Approximates a rounded rectangle as `[x, y, w, h]` scanline rects (physical pixels).
fn rounded_region(w: u32, h: u32, radius: u32) -> Vec<u32> {
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 {
        return vec![0, 0, w, h];
    }
    let inset = |row: u32| -> u32 {
        let dy = f64::from(r) - f64::from(row) - 0.5;
        let dx = (f64::from(r).powi(2) - dy * dy).max(0.0).sqrt();
        (f64::from(r) - dx).round() as u32
    };
    let mut out = Vec::with_capacity((2 * r as usize + 1) * 4);
    for row in 0..r {
        let i = inset(row);
        out.extend_from_slice(&[i, row, w.saturating_sub(2 * i), 1]);
    }
    out.extend_from_slice(&[0, r, w, h - 2 * r]);
    for row in 0..r {
        let i = inset(row);
        out.extend_from_slice(&[i, h - 1 - row, w.saturating_sub(2 * i), 1]);
    }
    out
}

type X11Conn = x11rb::rust_connection::RustConnection;

thread_local! {
    static CONN: std::cell::RefCell<Option<(X11Conn, usize)>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` on a per-thread cached X11 connection, reconnecting once if the cached one fails.
fn with_conn<T>(
    f: impl Fn(&X11Conn, usize) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    CONN.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((conn, screen)) = slot.as_ref()
            && let Ok(v) = f(conn, *screen)
        {
            return Ok(v);
        }
        let (conn, screen) = x11rb::connect(None)?;
        let res = f(&conn, screen);
        *slot = Some((conn, screen));
        res
    })
}

fn x11_window_id(window: &slint::winit_030::winit::window::Window) -> Option<u32> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok().filter(|&id| id != 0),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

fn send_net_wm_state(
    conn: &X11Conn,
    screen_num: usize,
    win: u32,
    skip: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = conn.setup().roots[screen_num].root;
    let wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
    let skip_tb = conn
        .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")?
        .reply()?
        .atom;
    let skip_pg = conn
        .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")?
        .reply()?
        .atom;
    if skip {
        conn.change_property32(
            PropMode::APPEND,
            win,
            wm_state,
            AtomEnum::ATOM,
            &[skip_tb, skip_pg],
        )?
        .check()?;
    }
    let action = if skip {
        NET_WM_STATE_ADD
    } else {
        NET_WM_STATE_REMOVE
    };
    let ev = ClientMessageEvent::new(
        32,
        win,
        wm_state,
        [action, skip_tb, skip_pg, SOURCE_APPLICATION, 0],
    );
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    conn.send_event(false, root, mask, ev)?.check()?;
    conn.flush()?;
    Ok(())
}

fn send_blur_hint(
    conn: &X11Conn,
    win: u32,
    region: &[u32],
) -> Result<(), Box<dyn std::error::Error>> {
    let blur_atom = conn
        .intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION")?
        .reply()?
        .atom;
    if region.is_empty() {
        conn.delete_property(win, blur_atom)?.check()?;
    } else {
        conn.change_property32(
            PropMode::REPLACE,
            win,
            blur_atom,
            AtomEnum::CARDINAL,
            region,
        )?
        .check()?;
    }
    conn.flush()?;
    Ok(())
}
