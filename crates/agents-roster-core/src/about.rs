// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Authorship and license notice (the program's "Appropriate Legal Notices").
//!
//! The same text is shown by `agents-roster about` and in the Control Panel footer. `NOTICE`
//! requires derivative works to preserve it (GNU AGPL v3, sections 5(d) and 7(b)).

/// Name of the original author and copyright holder.
pub const AUTHOR: &str = "AbnerMSaconi";
/// SPDX license identifier of the original work.
pub const LICENSE: &str = "AGPL-3.0-only";
/// Home of the original project.
pub const REPOSITORY: &str = "https://github.com/AbnerMSaconi/agents-roster";

/// One-line notice for the Control Panel footer.
pub fn short_notice() -> String {
    format!("(c) 2026 {AUTHOR} · {LICENSE}")
}

/// Full notice printed by `agents-roster about`; `provenance` is the record embedded in the binary.
pub fn full_notice(version: &str, commit: &str, provenance: &str) -> String {
    format!(
        "Agents Roster {version} (commit {commit})\n\
         Copyright (C) 2026 {AUTHOR}\n\
         License: GNU Affero General Public License v3.0 only (see LICENSE and NOTICE)\n\
         Original project: {REPOSITORY}\n\
         \n\
         This is free software: you may redistribute and modify it under the AGPL v3, keeping this\n\
         notice and the author attribution. It comes with ABSOLUTELY NO WARRANTY.\n\
         \n\
         provenance: {provenance}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_carry_author_license_and_origin() {
        let full = full_notice("1.2.3", "abc", "agents-roster-provenance;x");
        assert!(full.contains("Copyright (C) 2026 AbnerMSaconi"));
        assert!(full.contains("GNU Affero General Public License v3.0"));
        assert!(full.contains(REPOSITORY));
        assert!(full.contains("1.2.3") && full.contains("abc"));
        assert!(short_notice().contains(AUTHOR));
    }
}
