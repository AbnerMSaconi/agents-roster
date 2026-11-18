# Usage

## The panel

A small always-visible window lists every running agent with its project, state (working, waiting, idle,
stopped), the current tool, the next step, completed milestones, context-window usage and, for Claude Code,
the 5-hour and 7-day limits. Agents started inside other agents appear indented under their parent.

- **Click** an agent to focus the terminal running it, or open your default terminal in its project directory.
- **Drag** with the left mouse button to move the panel.
- **Right-click** for Hide, Always on top, Reminders while working, Control Panel and Quit.
- The **tray icon** shows how many agents are working; click it to show or hide the panel.

## Control Panel

Open it from the tray or right-click menu. Changes apply immediately.

| Tab | What it controls |
|---|---|
| **General** | Show the widget, always on top, hide from the taskbar, update interval (1–10 s). |
| **AI Services** | Which agents are tracked, their CLI name and data directory, a rescan button, and custom agents. |
| **Notifications** | Which events raise an OS notification (started, needs input, finished, error, milestones, context 80/90/95 %, quota) and the reminder interval. |
| **Appearance** | Presets, background/border color and opacity, blur, text and status colors, per-agent name colors. |

*Reset defaults* asks for a second click before it restores everything.

## Command line

```
agents-roster                          open the panel (default)
agents-roster status [--dir D]         print the current snapshot as JSON
agents-roster collect <Event> --dir D  Claude Code hook (reads JSON on stdin)
agents-roster statusline --dir D       Claude Code statusline (reads JSON on stdin)
agents-roster --version                print the release version
```

## Environment variables

| Variable | Meaning |
|---|---|
| `AGENTS_ROSTER_DIR` | Exchange directory (default `/run/user/<uid>/agents-roster`). `--dir` wins. |
| `AGENTS_ROSTER_BACKEND` | `x11` or `wayland` to force a display backend. |
| `LC_ALL`, `LC_MESSAGES`, `LANG` | A Portuguese locale switches the UI to Brazilian Portuguese. |

## Files

| Path | Purpose |
|---|---|
| `~/.config/agents-roster/config.toml` | Settings (managed by the Control Panel). |
| `/run/user/<uid>/agents-roster/` | Session state written by the Claude Code hooks; cleared on logout. |

## Troubleshooting

- **A Claude Code session shows "running in terminal" with no details** — its hooks are not installed, or
  the session was started before you added them. See [INSTALL](INSTALL.md#3-connect-claude-code-optional-but-recommended).
- **An agent is missing** — open *AI Services*, press *Rescan*, and check that its CLI name and data
  directory match what is installed.
- **A stopped (Ctrl+Z) agent is not listed** — suspended processes are treated as inactive on purpose.
- **No tray icon on GNOME** — install the AppIndicator extension; the panel still works without it.
- **Blur has no effect** — it needs KWin (or a compositor honoring `_KDE_NET_WM_BLUR_BEHIND_REGION`) on X11
  and a widget opacity below 100 %. It is not implemented on Wayland yet.
- **Panel does not stay on top or remember its position on Wayland** — Wayland does not let apps do either.
  Use a window rule from your compositor.
