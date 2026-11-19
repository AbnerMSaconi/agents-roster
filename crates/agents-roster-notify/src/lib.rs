// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Desktop notification delivery.
//!
//! Linux: `org.freedesktop.Notifications` over D-Bus (KDE / freedesktop).
//! Windows 11: WinRT toast notifications under a registered AppUserModelID.

#[cfg(unix)]
mod dbus;
#[cfg(windows)]
mod toast;

#[cfg(unix)]
pub use dbus::KdeNotifier;
#[cfg(unix)]
pub use dbus::KdeNotifier as DesktopNotifier;
#[cfg(windows)]
pub use toast::ToastNotifier;
#[cfg(windows)]
pub use toast::ToastNotifier as DesktopNotifier;

use agents_roster_core::{Notification, Notifier, NotifyError};

/// No-op [`Notifier`] useful for headless tests or disabled notifications.
#[derive(Debug, Default)]
pub struct NullNotifier {
    /// Captured notifications in test runs.
    pub sent: Vec<Notification>,
}

impl Notifier for NullNotifier {
    fn notify(&mut self, note: &Notification) -> Result<(), NotifyError> {
        self.sent.push(note.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_notifier_records_notifications() {
        let mut notifier = NullNotifier::default();
        let note = Notification {
            title: "Test".into(),
            body: "Body".into(),
            urgent: true,
        };
        notifier.notify(&note).unwrap();
        assert_eq!(notifier.sent, vec![note]);
    }
}
