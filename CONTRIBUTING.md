# Contributing

Bug reports, distro/compositor test results and patches are welcome.

## Build and check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`unsafe` code is forbidden workspace-wide.

By submitting a contribution you agree to license it under the GNU AGPL v3.0, keeping the project under a single license.
New source files should start with the same two-line header (`SPDX-License-Identifier: AGPL-3.0-only` and a
copyright line) used by the existing files. Run `cargo deny check` too (see `docs/RELEASING.md`).

## Testing other desktops

Pick the backend explicitly with `AGENTS_ROSTER_BACKEND=x11|wayland`.

To try Wayland without leaving your session, run a nested KWin (needs `kwin_wayland`):

```sh
cat > /tmp/run.sh <<'SH'
#!/bin/sh
AGENTS_ROSTER_BACKEND=wayland exec ./target/release/agents-roster
SH
chmod +x /tmp/run.sh
dbus-run-session kwin_wayland --socket wl-test --width 900 --height 700 --no-lockscreen /tmp/run.sh
```

When reporting a problem please include distro, desktop/compositor and version, `echo $XDG_SESSION_TYPE`,
and the output of `agents-roster` run from a terminal.

## Known gaps (good first contributions)

- **Wayland blur** — implement `org_kde_kwin_blur` (KDE) / `ext-background-effect-v1` in `crates/agents-roster-ui/src/x11_hints.rs`.
- **Wayland always-on-top** — needs `wlr-layer-shell` (Sway, Hyprland), which Slint does not provide.
- **GNOME tray** — the SNI icon needs the AppIndicator extension; a fallback is welcome.
- **Packaging** — Nix, Flatpak, AUR/deb/rpm. `packaging/install.sh` assumes a writable `/usr/local` and systemd's `/run/user/<uid>`.
