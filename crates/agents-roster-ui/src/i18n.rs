// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! UI translation catalog backed by `lang/pt/LC_MESSAGES/agents-roster-ui.po`.

use std::borrow::Cow;

use agents_roster_core::Translator;

include!(concat!(env!("OUT_DIR"), "/catalog_pt.rs"));

/// Detects whether `LC_ALL`, `LC_MESSAGES`, or `LANG` requests Portuguese.
pub fn detect_portuguese() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .is_some_and(|loc| loc.to_ascii_lowercase().starts_with("pt"))
}

/// [`Translator`] backed by the embedded `.po` catalog.
#[derive(Debug, Clone, Copy)]
pub struct PoTranslator {
    portuguese: bool,
}

impl PoTranslator {
    /// Creates a translator from the current locale environment variables.
    pub fn from_env() -> Self {
        Self {
            portuguese: detect_portuguese(),
        }
    }

    /// Creates a translator with an explicit language flag.
    pub fn new(portuguese: bool) -> Self {
        Self { portuguese }
    }

    /// Applies the selected language to Slint's `@tr(...)` runtime.
    pub fn apply_to_slint(self) {
        let language = if self.portuguese { "pt" } else { "" };
        if let Err(e) = slint::select_bundled_translation(language) {
            tracing::warn!("UI translation unavailable: {e}");
        }
    }
}

impl Translator for PoTranslator {
    fn translate<'a>(&self, key: &'a str) -> Cow<'a, str> {
        if let Some((_, pt)) = self
            .portuguese
            .then(|| CATALOG_PT.iter().find(|(k, _)| *k == key))
            .flatten()
        {
            return Cow::Borrowed(pt);
        }
        Cow::Borrowed(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(s: &str) -> Vec<&str> {
        let mut v = Vec::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            match rest[i..].find('}') {
                Some(j) => {
                    v.push(&rest[i..=i + j]);
                    rest = &rest[i + j + 1..];
                }
                None => break,
            }
        }
        v.sort_unstable();
        v
    }

    #[test]
    fn catalog_keeps_markers_and_unique_keys() {
        for (i, (en, pt)) in CATALOG_PT.iter().enumerate() {
            assert_eq!(markers(en), markers(pt), "markers differ in: {en}");
            assert!(
                CATALOG_PT[i + 1..].iter().all(|(other, _)| other != en),
                "repeated key: {en}"
            );
        }
    }

    #[test]
    fn translates_based_on_locale_flag() {
        let en = PoTranslator::new(false);
        let pt = PoTranslator::new(true);
        assert_eq!(en.translate("Hide"), "Hide");
        assert_eq!(pt.translate("Hide"), "Esconder");
        assert_eq!(pt.translate("unknown key"), "unknown key");
    }
}
