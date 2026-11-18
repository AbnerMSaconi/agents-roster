// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Cache readers (`last_conversations.json`, `conversation_metadata.json`) for agy discovery.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) fn read_last_conversations(root: &Path) -> HashMap<String, String> {
    fs::read(root.join("cache/last_conversations.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) fn read_metadata_workspaces(root: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let Ok(bytes) = fs::read(root.join("cache/conversation_metadata.json")) else {
        return out;
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return out;
    };
    let Some(map) = v
        .get("conversations")
        .and_then(Value::as_object)
        .or_else(|| v.as_object())
    else {
        return out;
    };
    for (id, entry) in map {
        if let Some(uri) = entry
            .pointer("/summary/WorkspaceURIs/0")
            .and_then(Value::as_str)
            && let Some(p) = uri.strip_prefix("file://")
        {
            out.insert(id.clone(), PathBuf::from(p));
        }
    }
    out
}

pub(crate) fn lock_id_from_path(path: &Path, root: &Path) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some("lock") {
        return None;
    }
    let parent = path.parent()?;
    if parent != root.join("presence")
        && parent.file_name().and_then(|n| n.to_str()) != Some("presence")
    {
        return None;
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(ToString::to_string)
}

pub(crate) fn lookup_conv_by_cwd(cwd: &Path, last_map: &HashMap<String, String>) -> Option<String> {
    if let Some(id) = last_map.get(&cwd.to_string_lossy().into_owned()) {
        return Some(id.clone());
    }
    let canon = fs::canonicalize(cwd).ok()?;
    last_map
        .iter()
        .find(|(dir, _)| fs::canonicalize(dir).ok().as_ref() == Some(&canon))
        .map(|(_, id)| id.clone())
}
