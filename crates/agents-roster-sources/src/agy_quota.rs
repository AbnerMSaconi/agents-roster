// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Scans recent agy glog files for all active per-model quota exhaustion windows.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use agents_roster_core::ProviderQuota;
use agents_roster_core::text::format_model_name;
use agents_roster_core::time::parse_go_duration_ms;

use crate::sys::parse_glog_ms;

const LOG_MAX_AGE_SECS: u64 = 7 * 24 * 3600;

struct CachedLog {
    len: u64,
    mtime_ms: u64,
    hits: Vec<ProviderQuota>,
}

/// Caches parsed `cli-*.log` quota events so unchanged logs are not re-read on every poll tick.
#[derive(Default)]
pub struct QuotaLogCache {
    files: HashMap<PathBuf, CachedLog>,
}

impl QuotaLogCache {
    /// Returns all currently active (non-reset) exhausted agy model quotas, deduplicated per model.
    pub fn read_active_quotas(&mut self, log_dir: &Path, now_ms: u64) -> Vec<ProviderQuota> {
        let mut by_model: HashMap<String, ProviderQuota> = HashMap::new();
        let Ok(rd) = fs::read_dir(log_dir) else {
            return Vec::new();
        };
        let mut seen_paths = HashSet::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("log") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let recent = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|d| d.as_secs() < LOG_MAX_AGE_SECS);
            if !recent {
                continue;
            }
            seen_paths.insert(path.clone());
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            for q in self.hits_for_file(&path, meta.len(), mtime_ms, now_ms) {
                if q.resets_at_ms > now_ms {
                    by_model
                        .entry(q.model.clone())
                        .and_modify(|prev| {
                            if q.since_ms > prev.since_ms {
                                *prev = q.clone();
                            }
                        })
                        .or_insert(q);
                }
            }
        }
        self.files.retain(|p, _| seen_paths.contains(p));
        let mut out: Vec<ProviderQuota> = by_model.into_values().collect();
        out.sort_by(|a, b| {
            a.resets_at_ms
                .cmp(&b.resets_at_ms)
                .then(a.model.cmp(&b.model))
        });
        out
    }

    fn hits_for_file(
        &mut self,
        path: &Path,
        len: u64,
        mtime_ms: u64,
        now_ms: u64,
    ) -> Vec<ProviderQuota> {
        if let Some(cached) = self.files.get(path)
            && cached.len == len
            && cached.mtime_ms == mtime_ms
        {
            return cached.hits.clone();
        }
        let hits = parse_log_file(path, now_ms);
        self.files.insert(
            path.to_path_buf(),
            CachedLog {
                len,
                mtime_ms,
                hits: hits.clone(),
            },
        );
        hits
    }
}

/// Parses one `cli-*.log` file forward, associating each `RESOURCE_EXHAUSTED` with the active model.
pub fn parse_log_file(path: &Path, now_ms: u64) -> Vec<ProviderQuota> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut reader = BufReader::with_capacity(16 * 1024, file);
    let mut line_buf = Vec::with_capacity(512);
    let mut active_model = String::new();
    let mut hits = Vec::new();
    loop {
        line_buf.clear();
        let Ok(n) = reader.read_until(b'\n', &mut line_buf) else {
            break;
        };
        if n == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&line_buf);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(m) = extract_selected_model(line) {
            active_model = m;
            continue;
        }
        let Some(idx) = line.find("Resets in ") else {
            continue;
        };
        if !line.contains("RESOURCE_EXHAUSTED") && !line.contains("quota") {
            continue;
        }
        let dur: String = line[idx + 10..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if let (Some(t), Some(d)) = (parse_glog_ms(line, now_ms), parse_go_duration_ms(&dur)) {
            hits.push(ProviderQuota {
                provider: "agy".into(),
                model: active_model.clone(),
                exhausted: true,
                since_ms: t,
                resets_at_ms: t + d,
            });
        }
    }
    hits
}

fn extract_selected_model(line: &str) -> Option<String> {
    if let Some(rest) = line
        .split_once("Propagating selected model override to backend: label=\"")
        .map(|x| x.1)
        && let Some(label) = rest.split('"').next()
    {
        return Some(strip_effort_suffix(label));
    }
    if let Some(rest) = line.split_once("Print mode: starting (").map(|x| x.1)
        && let Some(model_part) = rest.split("model=\"").nth(1)
        && let Some(raw) = model_part.split('"').next()
    {
        return Some(normalize_model_label(raw));
    }
    if let Some(rest) = line.split_once("Resolving model ").map(|x| x.1) {
        let trimmed = rest.trim();
        if !trimmed.is_empty() {
            return Some(normalize_model_label(trimmed));
        }
    }
    None
}

fn strip_effort_suffix(label: &str) -> String {
    let trimmed = label.trim();
    for suffix in [" (High)", " (Medium)", " (Low)"] {
        if let Some(base) = trimmed.strip_suffix(suffix) {
            return base.to_string();
        }
    }
    trimmed.to_string()
}

fn normalize_model_label(raw: &str) -> String {
    let stripped = strip_effort_suffix(raw);
    if !stripped.contains('-') {
        return stripped;
    }
    let base = stripped
        .strip_suffix("-high")
        .or_else(|| stripped.strip_suffix("-medium"))
        .or_else(|| stripped.strip_suffix("-low"))
        .unwrap_or(&stripped);
    let formatted = format_model_name(base);
    if formatted != base {
        return format!("Claude {formatted}");
    }
    base.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_model_labels_and_strips_effort() {
        assert_eq!(
            strip_effort_suffix("Gemini 4 Argon (High)"),
            "Gemini 4 Argon"
        );
        assert_eq!(
            normalize_model_label("gemini-4-argon-high"),
            "Gemini 4 Argon"
        );
        assert_eq!(
            normalize_model_label("claude-sonnet-5-5-high"),
            "Claude Sonnet 5.5"
        );
    }
}
