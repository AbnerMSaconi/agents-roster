// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Desktop notification delivery via `org.freedesktop.Notifications` (KDE / freedesktop D-Bus).

use std::collections::HashMap;

use agents_roster_core::{Notification, Notifier, NotifyError};

/// Sends desktop notifications lazily over a blocking D-Bus session connection.
pub struct KdeNotifier {
    app_name: String,
    desktop_entry: String,
    conn: Option<zbus::blocking::Connection>,
}

impl KdeNotifier {
    /// Creates a lazy notifier that connects to D-Bus on the first notification.
    pub fn new(app_name: String, desktop_entry: String) -> Self {
        Self {
            app_name,
            desktop_entry,
            conn: None,
        }
    }
}

impl Notifier for KdeNotifier {
    fn notify(&mut self, note: &Notification) -> Result<(), NotifyError> {
        let conn = ensure_connection(&mut self.conn)?;
        if let Err(err) = deliver(conn, &self.app_name, &self.desktop_entry, note) {
            self.conn = None;
            return Err(err);
        }
        Ok(())
    }
}

fn ensure_connection(
    slot: &mut Option<zbus::blocking::Connection>,
) -> Result<&zbus::blocking::Connection, NotifyError> {
    if slot.is_none() {
        let conn = zbus::blocking::Connection::session()
            .map_err(|e| NotifyError::Delivery(e.to_string()))?;
        *slot = Some(conn);
    }
    slot.as_ref()
        .ok_or_else(|| NotifyError::Delivery("D-Bus session not connected".into()))
}

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

fn deliver(
    conn: &zbus::blocking::Connection,
    app_name: &str,
    desktop_entry: &str,
    note: &Notification,
) -> Result<(), NotifyError> {
    let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    hints.insert(
        "urgency",
        zbus::zvariant::Value::U8(if note.urgent { 2 } else { 1 }),
    );
    hints.insert("desktop-entry", zbus::zvariant::Value::from(desktop_entry));
    let timeout_ms = if note.urgent { 0i32 } else { 8000i32 };
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            app_name,
            0u32,
            "dialog-information",
            note.title.as_str(),
            note.body.as_str(),
            Vec::<&str>::new(),
            hints,
            timeout_ms,
        ),
    )
    .map(|_| ())
    .map_err(|e| NotifyError::Delivery(e.to_string()))
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
