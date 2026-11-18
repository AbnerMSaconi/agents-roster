// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! OS, git, Claude Code, and agy adapters implementing [`agents_roster_core`] traits.

pub mod agy_cache;
pub mod agy_quota;
pub mod agy_source;
pub mod agy_subagents;
pub mod agy_transcript;
pub mod claude_hooks;
pub mod claude_model;
pub mod claude_source;
pub mod claude_statusline;
pub mod claude_subagents;
pub mod codex_source;
pub mod codex_transcript;
pub mod custom_source;
pub mod git_reflog;
pub mod procfs;
pub mod service_discovery;
pub mod sys;

pub use agy_source::AgySource;
pub use claude_hooks::handle_hook;
pub use claude_source::ClaudeCodeSource;
pub use claude_statusline::handle_statusline;
pub use codex_source::CodexSource;
pub use custom_source::GenericProcessSource;
pub use git_reflog::GitReflogInfo;
pub use procfs::ProcFsTable;
pub use service_discovery::{detect_installed_services, expand_tilde};
pub use sys::{ChronoTimeFormatter, SystemClock, current_uid, is_effective_root};
