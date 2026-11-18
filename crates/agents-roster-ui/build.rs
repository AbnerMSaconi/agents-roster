// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

use std::{env, fmt::Write, fs, path::PathBuf};

const PO_PT: &str = "lang/pt/LC_MESSAGES/agents-roster-ui.po";

fn fail(msg: &str) -> ! {
    eprintln!("agents-roster-ui build.rs: {msg}");
    std::process::exit(1);
}

fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    if let Err(e) = slint_build::compile_with_config("ui/panel.slint", config) {
        fail(&format!("failed to compile ui/panel.slint: {e}"));
    }

    println!("cargo:rerun-if-changed={PO_PT}");
    let text =
        fs::read_to_string(PO_PT).unwrap_or_else(|e| fail(&format!("failed to read {PO_PT}: {e}")));
    let mut output = String::from("pub static CATALOG_PT: &[(&str, &str)] = &[\n");
    for (en, pt) in read_po(&text) {
        let _ = writeln!(output, "    ({en:?}, {pt:?}),");
    }
    output.push_str("];\n");
    let out_dir = env::var("OUT_DIR").unwrap_or_else(|e| fail(&format!("OUT_DIR missing: {e}")));
    let dest = PathBuf::from(out_dir).join("catalog_pt.rs");
    if let Err(e) = fs::write(dest, output) {
        fail(&format!("failed to write catalog_pt.rs: {e}"));
    }
}

fn read_po(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for block in text.split("\n\n") {
        let (mut id, mut tr) = (String::new(), String::new());
        let (mut field, mut fuzzy) = (0u8, false);
        for row in block.lines().map(str::trim) {
            if row.starts_with("#,") && row.contains("fuzzy") {
                fuzzy = true;
            } else if let Some(r) = row.strip_prefix("msgid ") {
                (field, id) = (1, unescape(r));
            } else if let Some(r) = row.strip_prefix("msgstr ") {
                (field, tr) = (2, unescape(r));
            } else if row.starts_with('"') {
                if field == 1 {
                    id.push_str(&unescape(row));
                } else if field == 2 {
                    tr.push_str(&unescape(row));
                }
            }
        }
        if !fuzzy && !id.is_empty() && !tr.is_empty() {
            pairs.push((id, tr));
        }
    }
    pairs
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {}
        }
    }
    out
}
