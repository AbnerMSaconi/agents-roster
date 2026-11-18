// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Auto-detection of installed AI coding services and tilde path expansion.

use std::path::{Path, PathBuf};

use agents_roster_core::{ServiceEntry, ServiceStatus};

/// Expands leading `~/` against `home`.
pub fn expand_tilde(home: &Path, raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else if raw == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(raw)
    }
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    let candidate = Path::new(bin);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let p = dir.join(bin);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn default_probe_dirs(home: &Path, id: &str) -> Vec<PathBuf> {
    match id {
        "claude" => vec![home.join(".claude")],
        "agy" => vec![home.join(".gemini/antigravity-cli")],
        "codex" => vec![home.join(".codex")],
        "opencode" => vec![
            home.join(".local/share/opencode"),
            home.join(".config/opencode"),
        ],
        "gemini" => vec![home.join(".gemini/tmp"), home.join(".gemini")],
        _ => Vec::new(),
    }
}

/// Probes `$PATH` and data directories to report installation status for each configured service.
pub fn detect_installed_services(home: &Path, services: &[ServiceEntry]) -> Vec<ServiceStatus> {
    services
        .iter()
        .map(|svc| probe_one_service(home, svc))
        .collect()
}

fn probe_one_service(home: &Path, svc: &ServiceEntry) -> ServiceStatus {
    let bin_path = find_in_path(&svc.process_name);
    let configured = (!svc.data_dir.trim().is_empty())
        .then(|| expand_tilde(home, svc.data_dir.trim()))
        .filter(|p| p.exists());
    let dir_path = configured.or_else(|| {
        default_probe_dirs(home, &svc.id)
            .into_iter()
            .find(|p| p.exists())
    });
    let detected_path = bin_path
        .as_ref()
        .or(dir_path.as_ref())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    ServiceStatus {
        id: svc.id.clone(),
        installed: bin_path.is_some() || dir_path.is_some(),
        detected_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde_paths_and_detects_custom_directory() {
        let tmp = std::env::temp_dir().join(format!("agents-disc-{}", std::process::id()));
        let custom_dir = tmp.join("my-agent");
        let _ = std::fs::create_dir_all(&custom_dir);
        assert_eq!(expand_tilde(&tmp, "~/my-agent"), custom_dir);

        let list = vec![ServiceEntry {
            id: "custom-1".into(),
            label: "My Agent".into(),
            process_name: "nonexistent-binary-xyz".into(),
            data_dir: "~/my-agent".into(),
            ..Default::default()
        }];
        let res = detect_installed_services(&tmp, &list);
        assert_eq!(res.len(), 1);
        assert!(res[0].installed);
        assert_eq!(res[0].detected_path, custom_dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
