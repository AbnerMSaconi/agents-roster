# Changelog

## 0.3.0 — 2026-11-17

- Added authorship and provenance: `agents-roster about`, the commit id in `--version`, an embedded provenance record, per-file SPDX headers and a notice in the Control Panel footer.
- Releases now verify the tag against `Cargo.toml` and the changelog, build with remapped paths (no local paths in the binary), include `BUILD-INFO.txt`, and publish a signed build provenance attestation.
- Added `cargo deny` to CI, pinned CI to Rust 1.93, and documented security and privacy in `SECURITY.md`.

- Renamed the app to **Agents Roster** (binary `agents-roster`, crates `agents-roster-*`, config in `~/.config/agents-roster/`, exchange directory `/run/user/<uid>/agents-roster`, env vars `AGENTS_ROSTER_DIR` and `AGENTS_ROSTER_BACKEND`). Removed all legacy names, Portuguese subcommands and Portuguese serde field aliases.
- Added native Wayland backend (`ROSTER_BACKEND=x11|wayland`), rounded blur region, reorganized Control Panel, and a cached `/proc` scan (~75% less CPU).

## 0.2.0 — 2026-11-17

- Added a lazy Control Center window (`Settings...` from tray and context menu) covering installed/custom AI coding services (`Claude Code`, `Antigravity (agy)`, `Codex CLI`, `OpenCode`, `Gemini CLI`, plus user-defined binary/path entries), OS notification vs widget-only toggles per event, full color/opacity theme customization with per-service agent title colors, and X11 compositor blur (`_KDE_NET_WM_BLUR_BEHIND_REGION`).
- Added `OpenAI` Codex CLI tracking (`CodexSource` + `codex_transcript.rs`) and nested "agent inside agent" discovery across `agy`, Claude Code, and Codex (both built-in subagents/swarms and child CLI processes spawned inside another agent's terminal), rendered with hierarchy indentation and `↳` indicators in the widget and tray.
- Lowered runtime memory and thread count by removing `tokio` (switching `ksni` to `blocking` + `async-io` shared with Slint, `zbus::blocking::Connection` lazily on first notification, and `std::sync::mpsc::recv_timeout` on the `collector` thread) and streaming `agy` logs/transcripts with `BufReader`.
- Added click-to-open-terminal on agent cards (`TerminalOpener` / `SystemTerminalOpener`): focuses the open X11 terminal window hosting the agent's ancestor PID, or opens the system default terminal (`$TERMINAL`, `xdg-terminal-exec`, `x-terminal-emulator`, desktop-native fallback) in its working directory.
- Hidden the floating window from the X11 taskbar and desktop pager by default (`_NET_WM_STATE_SKIP_TASKBAR` / `_NET_WM_STATE_SKIP_PAGER`, configurable via `hide_from_taskbar`).
- Listed each exhausted `agy` model quota individually in footer rows and notifications until its reset timestamp (`QuotaLogCache` in `crates/roster-sources/src/agy_quota.rs`).
- Refactored into a 5-crate Cargo workspace (`roster-core`, `roster-sources`, `roster-notify`, `roster-ui`, `roster`) following SOLID boundaries and Dependency Inversion (`AgentSource`, `ProcessTable`, `GitInfo`, `Clock`, `TimeFormatter`, `Translator`, `Notifier`).
- Eliminated all `unsafe` code across `roster-core`, `roster-sources`, `roster-notify`, and `roster` (using `std::fs::File::lock`/`try_lock`, `std::os::unix::fs::chown`, `/proc/self/status`, and `chrono`).
- Fixed active-agent tracking bugs:
  - Stopped listing exited/dead Claude Code and `agy` sessions in the panel and tray (previously kept for 30 min / 6 hours); recently ended agents are passed separately to `NotificationEngine` so completion and error notifications still fire.
  - Discovered live `agy` sessions directly from `/proc` open `presence/*.lock` file descriptors and advisory `flock` probing, supporting multiple concurrent sessions in the same directory instead of relying solely on `cache/last_conversations.json`.
  - Properly detected `agy` `Waiting` state when a `PLANNER_RESPONSE` completes without `tool_calls` (instead of leaving the agent stuck as `Working` / `Idle` on the previous tool call).
  - Checked `/proc/<pid>/stat` process state to reject zombie processes and verified `comm` against PID reuse.
  - Cleared stale `Notification` reasons on Claude Code `Stop` and `SessionEnd` hooks.
- Added English source strings with embedded Portuguese translation catalog (`crates/roster-ui/lang/pt/LC_MESSAGES/roster-ui.po`) and backward-compatible config/hook aliases.

## 0.1.0 — 2026-11-15

- Initial Rust + Slint release (`roster`).
