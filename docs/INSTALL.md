# Installation

Current release: **0.4.0** (see [CHANGELOG](../CHANGELOG.md)). Check what you have with
`agents-roster --version`.

## 1. Requirements

- Linux (x86_64 tested) with an X11 or Wayland session. See [COMPATIBILITY](COMPATIBILITY.md).
- Runtime libraries: `fontconfig` and `freetype` (present on practically every desktop), and
  `libxkbcommon`. X11 sessions also use `libX11`/`libxcb`; Wayland sessions use `libwayland-client`.
- To build from source: Rust 1.93 or newer.

## 2. Install

### From a release tarball

Download `agents-roster-<version>-linux-<arch>.tar.gz` (and its `.sha256`) from the
[Releases page](https://github.com/AbnerMSaconi/agents-roster/releases), then:

```sh
sha256sum -c agents-roster-*.tar.gz.sha256
# optional, to confirm it is the official build (see docs/PROVENANCE.md):
gh attestation verify agents-roster-*.tar.gz --repo AbnerMSaconi/agents-roster
tar xzf agents-roster-*.tar.gz
cd agents-roster-*/
./install.sh
```

### From source

```sh
git clone https://github.com/AbnerMSaconi/agents-roster.git
cd agents-roster
./packaging/install.sh
```

Both install `/usr/local/bin/agents-roster` (root-owned, so root-run Claude Code hooks cannot be tampered
with by unprivileged processes), a launcher entry and an autostart entry for your user.

Start it with `agents-roster` or from your application menu.

## 3. Connect Claude Code (optional but recommended)

agy and Codex are detected automatically from their process and data directories. Claude Code needs hooks
so the panel can show its state, task progress and context usage. Add this to `~/.claude/settings.json`
(and `/root/.claude/settings.json` if you run Claude with `sudo`), replacing `1000` with the output of `id -u`:

```json
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect SessionStart --dir /run/user/1000/agents-roster" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect UserPromptSubmit --dir /run/user/1000/agents-roster" }] }],
    "PreToolUse":       [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect PreToolUse --dir /run/user/1000/agents-roster" }] }],
    "PostToolUse":      [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect PostToolUse --dir /run/user/1000/agents-roster" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect Notification --dir /run/user/1000/agents-roster" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect Stop --dir /run/user/1000/agents-roster" }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect SessionEnd --dir /run/user/1000/agents-roster" }] }],
    "PreCompact":       [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect PreCompact --dir /run/user/1000/agents-roster" }] }],
    "TaskCreated":      [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect TaskCreated --dir /run/user/1000/agents-roster" }] }],
    "TaskCompleted":    [{ "hooks": [{ "type": "command", "timeout": 5, "command": "/usr/local/bin/agents-roster collect TaskCompleted --dir /run/user/1000/agents-roster" }] }]
  },
  "statusLine": {
    "type": "command",
    "command": "/usr/local/bin/agents-roster statusline --dir /run/user/1000/agents-roster"
  }
}
```

Merge these keys into your existing file instead of replacing it. Restart open Claude Code sessions so they
pick up the hooks. `--dir` may be omitted if `AGENTS_ROSTER_DIR` is set; the default is
`/run/user/<uid>/agents-roster`.

## 4. Verify

```sh
agents-roster --version
agents-roster status        # prints the live snapshot as JSON
```

## Update

Download the new tarball (or `git pull`) and run the installer again. Your configuration is kept.

## Uninstall

```sh
./packaging/uninstall.sh
rm -r ~/.config/agents-roster      # optional: also remove settings
```

Then remove the hooks and `statusLine` entries from `~/.claude/settings.json`.
