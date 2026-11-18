# agents-roster architecture

As-of: 2026-11-17 · Status: active · Binary: `agents-roster`

---

## 1. Workspace layout

| Crate | Role | Unsafe |
|---|---|---|
| `agents-roster-core` | Pure domain model (`Agent`, `AgentState`, `Snapshot`), traits (`AgentSource`, `ProcessTable`, `GitInfo`, `Clock`, `TimeFormatter`, `Translator`, `Notifier`), `SnapshotCollector`, and `NotificationEngine` | `forbid` |
| `agents-roster-sources` | OS/filesystem adapters: `ProcFsTable`, `GitReflogInfo`, `SystemClock`, `ChronoTimeFormatter`, `ClaudeCodeSource`, `AgySource`, Claude hook/statusline writers | `forbid` |
| `agents-roster-notify` | `KdeNotifier` (`org.freedesktop.Notifications` over `zbus`) and `NullNotifier` | `forbid` |
| `agents-roster-ui` | Slint floating panel, `ksni` SNI tray icon, pure `view_model`, embedded i18n (`lang/pt/LC_MESSAGES/agents-roster-ui.po`) | `deny` (Slint macro requires `allow`) |
| `agents-roster` | Thin composition root binary: CLI, TOML configuration (`~/.config/agents-roster/config.toml`), wiring | `forbid` |

## 2. Dependency inversion and Open/Closed extension

`agents-roster-core` owns all domain traits and performs zero I/O:

```text
agents-roster (composition root)
  ├── wires ClaudeCodeSource, AgySource ──► agents-roster-core::AgentSource
  ├── wires ProcFsTable, GitReflogInfo  ──► agents-roster-core::{ProcessTable, GitInfo}
  ├── wires KdeNotifier                 ──► agents-roster-core::Notifier
  └── runs  agents-roster-ui::run_panel        ──► Slint + ksni + PoTranslator
```

Adding a new AI agent provider only requires implementing `agents_roster_core::AgentSource` and registering it in
`agents-roster::build_collector`. `NotificationEngine` and `agents-roster-ui::view_model` handle arbitrary sources,
usage windows, and quota notices uniformly without source-specific branches.

## 3. Live agent discovery

- **Claude Code (`ClaudeCodeSource`):**
  - Hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop`, `SessionEnd`,
    `PreCompact`, `TaskCreated`, `TaskCompleted`) and `statusline` write per-session JSON atomically under an
    advisory `File::lock` into `<exchange_dir>/claude/`.
  - On collection, a session is active **only** if `!session.closed` and its `claude_pid` is alive and non-zombie
    in `/proc/<pid>/stat` matching `claude` (or if `claude_pid == 0`, at least one live `claude` process exists).
  - Closed or dead sessions never enter `Snapshot.agents`; transitions into `Stopped`/`Error` are passed once via
    `Snapshot.recently_ended` so `NotificationEngine` still emits completion/error notifications.
- **agy (`AgySource`):**
  - Discovers live `agy` processes via `/proc/<pid>/comm` (and `cmdline` arg0), matching each process to its
    conversation ID through open `/proc/<pid>/fd/*` links to `~/.gemini/antigravity-cli/presence/<id>.lock`
    and `File::try_lock` probes (falling back to `cache/last_conversations.json` only while a live `agy` PID
    in that working directory has not yet opened its lock file).
  - Parses `brain/<id>/.system_generated/logs/transcript.jsonl` incrementally: `USER_INPUT` and `PLANNER_RESPONSE`
    with `tool_calls` set `Working`; a completed `PLANNER_RESPONSE` (`status == "DONE"`) without `tool_calls`
    transitions the agent to `Waiting` (`"waiting for you"`) and records the reply in `done`.
  - Reads `log/*.log` for active `RESOURCE_EXHAUSTED` quota reset timestamps.
  - Exited `agy` processes are never listed in `Snapshot.agents`; recently exited sessions are handed once via
    `Snapshot.recently_ended` to `NotificationEngine`.

## 4. Notification state machine

`NotificationEngine` in `agents-roster-core` is pure and deterministic:
- Suppresses initial flood on the first snapshot.
- Emits `"started"`, `"finished"` (suppressing Claude replies shorter than 2 min), urgent `"needs you"`,
  urgent `"stopped"` on errors, `"ended"` when a working agent finishes/exits, 80/90/95% context and rate-limit
  thresholds (re-arming below threshold), quota exhaustion notices, and periodic reminders while working.

## 5. External compatibility contracts

- CLI subcommands: `collect`, `status`, `statusline`.
- Exchange directory: `--dir`, `$AGENTS_ROSTER_DIR`, or `/run/user/<uid>/agents-roster`.
- Configuration: `~/.config/agents-roster/config.toml`, edited through the Control Panel.
