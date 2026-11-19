// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Windows 11 toast notifications via WinRT (`Windows.UI.Notifications`).
//!
//! Unpackaged Win32 apps need a registered AppUserModelID for toasts to display with a proper
//! name and land in the Action Center. On first use this registers `HKCU\Software\Classes\
//! AppUserModelId\<AUMID>` (display name + icon) and creates a Start Menu shortcut carrying the
//! same AUMID. Both are per-user and need no elevation; failures degrade to a best-effort toast.

#![allow(unsafe_code)] // COM/WinRT FFI: audited, confined to this module.

use agents_roster_core::{Notification, Notifier, NotifyError};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCreateKeyExW,
    RegSetValueExW,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, HSTRING, HRESULT, Interface, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

/// AppUserModelID under which toasts are shown (registered per-user on first notification).
const AUMID: &str = "AbnerMSaconi.AgentsRoster";

// PROPERTYKEY for System.AppUserModel.ID (propkey.h). The SDK metadata used by
// windows-rs 0.62 does not expose the named constant on all supported targets.
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9F4C2855_F79B_4B39_A8D0_E1D42DE1D5F3),
    pid: 5,
};

fn io_error(error: std::io::Error) -> windows::core::Error {
    let code = error.raw_os_error().unwrap_or(1) as u32;
    windows::core::Error::new(
        HRESULT(0x8007_0000u32.wrapping_add(code) as i32),
        error.to_string(),
    )
}

/// Sends Windows toast notifications, registering the app identity lazily on first use.
pub struct ToastNotifier {
    app_name: String,
    registered: bool,
}

impl ToastNotifier {
    /// Creates a lazy notifier; `desktop_entry` is accepted for parity with the Linux backend
    /// and ignored (Windows identifies the app by its AppUserModelID).
    pub fn new(app_name: String, _desktop_entry: String) -> Self {
        Self { app_name, registered: false }
    }
}

impl Notifier for ToastNotifier {
    fn notify(&mut self, note: &Notification) -> Result<(), NotifyError> {
        // COM must be initialized on the calling thread (the collector thread).
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if !self.registered {
            register_app_identity(&self.app_name);
            self.registered = true;
        }
        show_toast(note)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn show_toast(note: &Notification) -> Result<(), NotifyError> {
    let duration = if note.urgent { " duration=\"long\"" } else { "" };
    let xml = format!(
        "<toast{duration}><visual><binding template=\"ToastGeneric\">\
         <text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(&note.title),
        xml_escape(&note.body),
    );
    let doc = XmlDocument::new().map_err(err)?;
    doc.LoadXml(&HSTRING::from(&xml)).map_err(err)?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(err)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)).map_err(err)?;
    notifier.Show(&toast).map_err(err)
}

fn err(e: windows::core::Error) -> NotifyError {
    NotifyError::Delivery(format!("toast: {e}"))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Registers the AUMID display name/icon under HKCU and creates the Start Menu shortcut.
fn register_app_identity(app_name: &str) {
    register_display_name(app_name);
    if let Err(e) = create_start_menu_shortcut(app_name) {
        // Without the shortcut toasts still display; they just may not persist in Action Center.
        tracing::debug!("start menu shortcut for toasts unavailable: {e}");
    }
}

fn register_display_name(app_name: &str) {
    let key = wide(&format!("Software\\Classes\\AppUserModelId\\{AUMID}"));
    let mut hkey = Default::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return;
    }
    let set_sz = |name: &str, value: &str| {
        let (n, v) = (wide(name), wide(value));
        let bytes = unsafe { core::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2) };
        unsafe { RegSetValueExW(hkey, PCWSTR(n.as_ptr()), None, REG_SZ, Some(bytes)) }
    };
    let _ = set_sz("DisplayName", app_name);
    if let Ok(exe) = std::env::current_exe() {
        let _ = set_sz("IconUri", &exe.to_string_lossy());
    }
    let _ = set_sz("IconBackgroundColor", "FF141419");
    let _ = unsafe { windows::Win32::System::Registry::RegCloseKey(hkey) };
}

/// Creates `%APPDATA%\...\Start Menu\Programs\<name>.lnk` tagged with the AUMID property.
fn create_start_menu_shortcut(app_name: &str) -> windows::core::Result<()> {
    let exe = std::env::current_exe().map_err(io_error)?;
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| {
        windows::core::Error::new(HRESULT(0x80004005u32 as i32), "APPDATA is not set")
    })?;
    let dir = std::path::Path::new(&appdata)
        .join("Microsoft/Windows/Start Menu/Programs");
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    let lnk = dir.join(format!("{app_name}.lnk"));

    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    unsafe {
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        link.SetDescription(&HSTRING::from("Agents Roster — live AI coding agents"))?;
    }

    // Tag the shortcut with System.AppUserModel.ID so the shell maps toasts to this app.
    let store: IPropertyStore = link.cast()?;
    // `PROPVARIANT::from(&str)` allocates the string with the COM allocator, which is what the
    // PROPVARIANT's `Drop` (PropVariantClear) frees. Pointing it at a Rust-owned buffer would
    // make that free corrupt the heap.
    let pv = PROPVARIANT::from(AUMID);
    unsafe {
        store.SetValue(&PKEY_APP_USER_MODEL_ID, &pv)?;
        store.Commit()?;
    }
    let file: IPersistFile = link.cast()?;
    unsafe { file.Save(PCWSTR(wide(&lnk.to_string_lossy()).as_ptr()), true) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_entities_in_titles_and_bodies() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
