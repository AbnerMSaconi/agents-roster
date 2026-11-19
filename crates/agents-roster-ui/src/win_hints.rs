// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Windows 11 window hints: taskbar presence (`WS_EX_TOOLWINDOW`), always-on-top
//! (winit window level) and background blur (DWM acrylic backdrop, with the legacy
//! `SetWindowCompositionAttribute` accent as a fallback for pre-22H2 builds).

#![allow(unsafe_code)] // Win32/DWM FFI: audited, confined to this module.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit::window::WindowLevel;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::ComponentHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{s, w};
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::app::Panel;

/// `DWMSBT_TRANSIENTWINDOW` = acrylic; `DWMSBT_NONE` = no backdrop.
const DWMSBT_NONE: i32 = 1;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;

/// Selects the winit backend and configures the taskbar-less tool window style at creation.
pub fn configure_backend(skip_taskbar: bool) {
    let result = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(move |attrs| attrs.with_skip_taskbar(skip_taskbar))
        .select();
    if let Err(e) = result {
        tracing::warn!("could not configure winit backend hook: {e}");
    }
}

/// Watches winit window events to apply taskbar and blur hints once the window maps or resizes.
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

/// Applies the always-on-top window level (`HWND_TOPMOST`) to the mapped window.
pub(crate) fn apply_always_on_top(panel: &Panel, on: bool) {
    let level = if on { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
    panel
        .window()
        .with_winit_window(|w| w.set_window_level(level));
}

fn panel_hwnd(panel: &Panel) -> Option<HWND> {
    panel
        .window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
            _ => None,
        })
        .flatten()
}

/// Toggles `WS_EX_TOOLWINDOW` (hidden from taskbar and Alt+Tab) on the live window.
pub(crate) fn apply_skip_taskbar(panel: &Panel, skip: bool) {
    let Some(hwnd) = panel_hwnd(panel) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let tool = WS_EX_TOOLWINDOW.0 as isize;
        let app = WS_EX_APPWINDOW.0 as isize;
        let next = if skip { (ex | tool) & !app } else { ex & !tool };
        if next != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Windows has no Wayland-style compositor restrictions: positioning is always client-side.
pub(crate) fn is_wayland(_panel: &Panel) -> bool {
    false
}

/// Interactive moves are handled by the generic drag callback on Windows.
pub(crate) fn start_native_move(_panel: &Panel) -> bool {
    false
}

/// DWM recomposes acrylic live; a redraw is enough to refresh the panel surface.
pub(crate) fn refresh_compositor(panel: &Panel) {
    panel.window().request_redraw();
}

/// Enables the Windows 11 acrylic backdrop behind the (transparent) panel window.
///
/// Falls back to the `SetWindowCompositionAttribute` acrylic accent on builds older than
/// 22H2 where `DWMWA_SYSTEMBACKDROP_TYPE` is rejected.
pub(crate) fn apply_blur_behind(panel: &Panel, enabled: bool) {
    let Some(hwnd) = panel_hwnd(panel) else { return };
    // Acrylic only shows through pixels the window leaves transparent; extend the DWM frame
    // into the whole client area so the backdrop is visible behind the software renderer.
    let margins = if enabled {
        MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
    } else {
        MARGINS::default()
    };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
    let backdrop = if enabled { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
    let ok = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&backdrop as *const i32).cast(),
            core::mem::size_of::<i32>() as u32,
        )
    }
    .is_ok();
    if !ok {
        accent_blur_fallback(hwnd, enabled);
    }
}

// --- Pre-22H2 fallback: undocumented-but-stable SetWindowCompositionAttribute accent API. ---

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttrData {
    attribute: u32,
    data: *mut core::ffi::c_void,
    size_of_data: u32,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

type SetWcaFn = unsafe extern "system" fn(HWND, *mut WindowCompositionAttrData) -> i32;

/// Resolves the undocumented export at runtime (it is absent from `user32.lib`).
fn set_window_composition_attribute() -> Option<SetWcaFn> {
    let module = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    let proc = unsafe { GetProcAddress(module, s!("SetWindowCompositionAttribute")) }?;
    Some(unsafe { core::mem::transmute::<unsafe extern "system" fn() -> isize, SetWcaFn>(proc) })
}

fn accent_blur_fallback(hwnd: HWND, enabled: bool) {
    let mut policy = AccentPolicy {
        accent_state: if enabled { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED },
        accent_flags: 0,
        // AABBGGRR tint; a faint dark tint keeps panel text readable over bright wallpapers.
        gradient_color: 0x4014_1419,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttrData {
        attribute: WCA_ACCENT_POLICY,
        data: (&mut policy as *mut AccentPolicy).cast(),
        size_of_data: core::mem::size_of::<AccentPolicy>() as u32,
    };
    if let Some(f) = set_window_composition_attribute() {
        let _ = unsafe { f(hwnd, &mut data) };
    }
}
