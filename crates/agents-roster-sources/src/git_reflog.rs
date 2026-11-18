// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Reads recent commits directly from git reflog (`logs/HEAD`), without spawning `git`.

use std::fs;
use std::path::{Path, PathBuf};

use agents_roster_core::text::truncate;
use agents_roster_core::{DoneItem, GitInfo, Translator};

/// Filesystem git reflog reader implementing [`GitInfo`].
#[derive(Debug, Default, Clone, Copy)]
pub struct GitReflogInfo;

fn resolve_git_dir(dir: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let git_entry = d.join(".git");
        if git_entry.is_dir() {
            return Some(git_entry);
        }
        if git_entry.is_file() {
            let txt = fs::read_to_string(&git_entry).ok()?;
            let target = txt.trim().strip_prefix("gitdir:")?.trim();
            let p = Path::new(target);
            return Some(if p.is_absolute() {
                p.to_path_buf()
            } else {
                d.join(p)
            });
        }
        cur = d.parent();
    }
    None
}

/// Parses `<old> <new> Name <email> <unix> <tz>\tcommit: subject` into a [`DoneItem`].
pub fn parse_commit_line(line: &str, tr: &dyn Translator) -> Option<DoneItem> {
    let (header, msg) = line.split_once('\t')?;
    let subject = msg
        .strip_prefix("commit: ")
        .or_else(|| msg.strip_prefix("commit (initial): "))
        .or_else(|| msg.strip_prefix("commit (amend): "))?;
    let mut parts = header.rsplitn(3, ' ');
    let _tz = parts.next()?;
    let ts: u64 = parts.next()?.parse().ok()?;
    let short = truncate(subject, 80);
    Some(DoneItem {
        text: tr.format("commit: {subject}", &[("subject", &short)]),
        at_ms: ts * 1000,
        is_reply: false,
        turn_duration_ms: None,
    })
}

impl GitInfo for GitReflogInfo {
    fn commits_since(
        &self,
        dir: &Path,
        since_ms: u64,
        max: usize,
        tr: &dyn Translator,
    ) -> Vec<DoneItem> {
        let Some(git_dir) = resolve_git_dir(dir) else {
            return Vec::new();
        };
        let Ok(txt) = fs::read_to_string(git_dir.join("logs/HEAD")) else {
            return Vec::new();
        };
        let mut items: Vec<DoneItem> = txt
            .lines()
            .filter_map(|l| parse_commit_line(l, tr))
            .filter(|item| item.at_ms >= since_ms)
            .collect();
        items.reverse();
        items.truncate(max);
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agents_roster_core::IdentityTranslator;

    #[test]
    fn parses_commit_reflog_entries() {
        let tr = IdentityTranslator;
        let line = "0000 07fe Abner <a@b> 1794764000 -0400\tcommit (initial): E0: architecture";
        let item = parse_commit_line(line, &tr).unwrap();
        assert_eq!(item.text, "commit: E0: architecture");
        assert_eq!(item.at_ms, 1_794_764_000_000);
        assert!(
            parse_commit_line(
                "0000 07fe A <a> 1 -0400\tcheckout: moving from main to x",
                &tr
            )
            .is_none()
        );
    }
}
