// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure text, model name, and project path formatting helpers.

use std::path::Path;

/// Normalizes model IDs like `claude-opus-5-5-high` into `Opus 5.5`.
pub fn format_model_name(id: &str) -> String {
    let id = id.trim();
    let stripped = id.strip_prefix("claude-").unwrap_or(id);
    for family in ["opus", "sonnet", "haiku", "fable"] {
        if let Some(rest) = stripped
            .strip_prefix(family)
            .and_then(|r| r.strip_prefix('-'))
        {
            let nums: Vec<&str> = rest
                .split('-')
                .take_while(|p| p.chars().all(|c| c.is_ascii_digit()))
                .collect();
            let mut name = family[..1].to_uppercase() + &family[1..];
            if !nums.is_empty() {
                name.push(' ');
                name.push_str(&nums.join("."));
            }
            return name;
        }
    }
    id.to_string()
}

/// Extracts the first meaningful line of markdown text, stripped of formatting and truncated to `max` chars.
pub fn summarize(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .find(|l| !l.is_empty() && !l.starts_with("```") && !l.starts_with('|'))
        .unwrap_or("");
    let clean = line.replace("**", "").replace('`', "");
    truncate(&clean, max)
}

/// Truncates `s` to at most `max` Unicode scalar values, appending `…` if clipped.
pub fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut r: String = s.chars().take(max.saturating_sub(1)).collect();
        r.push('…');
        r
    }
}

/// Returns the final path component, or the input string if none exists.
pub fn base_name(p: &str) -> &str {
    Path::new(p)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(p)
}

/// Formats `.../repo/.worktrees/branch` as `repo/branch`, otherwise returns the folder name.
pub fn project_name(p: &str) -> String {
    let path = Path::new(p);
    let name = base_name(p);
    match path.parent() {
        Some(parent) if parent.file_name().and_then(|n| n.to_str()) == Some(".worktrees") => {
            let project = parent
                .parent()
                .map(|x| base_name(&x.to_string_lossy()).to_string())
                .unwrap_or_default();
            format!("{project}/{name}")
        }
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_model_names() {
        let cases = [
            ("claude-opus-5-5-high", "Opus 5.5"),
            ("claude-sonnet-5-5", "Sonnet 5.5"),
            ("claude-haiku-5-5-low", "Haiku 5.5"),
            ("gemini-4-argon-high", "gemini-4-argon-high"),
        ];
        for (input, expected) in cases {
            assert_eq!(format_model_name(input), expected);
        }
    }

    #[test]
    fn summarizes_markdown_lines() {
        assert_eq!(summarize("\n## **Title** here\nrest", 40), "Title here");
        assert_eq!(summarize("abcdef", 4), "abc…");
        assert_eq!(summarize("```\ncode\n```", 10), "code");
    }

    #[test]
    fn formats_worktree_project_names() {
        assert_eq!(
            project_name("/home/user/projects/my-app/.worktrees/feature"),
            "my-app/feature"
        );
        assert_eq!(project_name("/home/user/projects/my_tool"), "my_tool");
    }
}
