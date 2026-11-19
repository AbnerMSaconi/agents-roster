# Compatibility

This page separates what has been **tested** from what is only **expected** to work. If you try another
setup, please open an issue with the result so this table can grow.

## Tested

| Component | Verified on |
|---|---|
| Distribution | Drauger OS 7.8 (Ubuntu-based), x86_64 |
| Desktop | KDE Plasma 6.6 |
| Session | X11 — full feature set |
| Session | Wayland — started in a nested KWin session only (connects and runs; interaction not exercised) |
| Rust | 1.93 |

## Expected, not yet tested

- Windows 11 x86_64 (Win32 process inspection, notification-area tray, DWM hints, and WinRT toasts are implemented; no Windows runner or physical-device validation has been recorded for this snapshot).

- Other glibc distributions (Debian, Ubuntu, Fedora, Arch, openSUSE, …) with the runtime libraries from
  [INSTALL](INSTALL.md#1-requirements).
- Other desktops with a StatusNotifierItem tray and freedesktop notifications: XFCE, Cinnamon, MATE, LXQt.
- GNOME: panel and notifications should work; the tray icon needs the AppIndicator extension.
- Sway / Hyprland: the panel runs, but "always on top" needs a window rule.
- ARM64: no reason it would not build, but no binaries are published.

## Not supported

- macOS (no backend is implemented).
- Headless servers.
- musl-based systems have not been tried.

## Features by session type

| Feature | X11 | Wayland |
|---|---|---|
| Panel, tray, notifications, agent tracking | yes | yes |
| Drag to move | yes | yes (compositor-driven) |
| Remembered position | yes | no |
| Always on top / hide from taskbar | yes | no (use a compositor rule) |
| Background blur | KWin | not implemented |

## Agents

| Agent | Detection | Details shown |
|---|---|---|
| Claude Code | Process + hooks + statusline | State, tool, next step, tasks, commits, context %, 5 h / 7 d limits, subagents |
| agy (Antigravity CLI) | Process, presence locks, transcripts | State, tool, commits, context, exhausted-quota notices, subagents |
| Codex CLI | Process + transcripts | State, context, subagents |
| OpenCode, Gemini CLI, others | Process name + data directory | Presence only (custom services) |

## Windows notes

The Windows backend targets 64-bit Windows 11. Process command-line and working-directory inspection reads the target process PEB and may be unavailable for elevated or protected processes. See [Windows installation](INSTALL-WINDOWS.md) for build and troubleshooting steps.

## Install layouts

`packaging/install.sh` writes to `/usr/local/bin`, so it does not suit immutable systems (NixOS, Fedora
Atomic) or systems without systemd's `/run/user/<uid>`. On those, build with `cargo build --release`,
place the binary anywhere and pass `--dir` to the hooks. Nix/Flatpak packages are welcome contributions.
