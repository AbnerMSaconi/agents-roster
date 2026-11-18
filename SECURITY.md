# Security and privacy

Agents Roster shows what your AI coding agents are doing. To do that it reads data those tools
leave on your machine. This page says plainly what it reads, what it does not do, and what has
**not** been verified.

## Privacy

- **No network access.** The code and its dependencies contain no HTTP, TLS or socket (TCP/UDP)
  support. Desktop notifications go through the local D-Bus. There is no telemetry, update check or
  "phone home".
- **What it reads.** The process list (`/proc`), the data directories of the agents it tracks
  (Claude Code hook events, and the local session files of agy and Codex), and `git` reflog
  information of the project an agent works in. This can include tool names, task descriptions and
  commit subjects, and the panel shows some of it on screen.
- **What it writes.** Your settings (`~/.config/agents-roster/`) and the exchange directory
  (`/run/user/<uid>/agents-roster`, in memory-backed storage) holding the latest agent states.
  Nothing is sent anywhere.
- **Output identifies the program, not you.** `agents-roster about` and the embedded provenance
  record name the author and project, never your user name or paths.

## The Claude Code hook

`agents-roster collect <Event>` runs on every Claude Code hook event and reads that event's JSON
from stdin. Because a hook runs with the privileges of whoever runs Claude Code (including root if
you use `sudo`), install the binary where only root can modify it, as the provided installers do
(`/usr/local/bin/agents-roster`). A user-writable hook binary running as root would be a privilege
escalation path.

The exchange directory is created with mode `0755` and its state files with `0644`, so a root-run
hook can write what the panel (your user) reads. By default it lives under `/run/user/<uid>/`, which
the system makes private to your user. If you point `AGENTS_ROSTER_DIR` at a shared location, other
local users could read the agent states stored there.

## What has not been verified

- **No independent security audit** has been done. The code is reviewed only by its author, with
  automated tests and `clippy`.
- **No `unsafe` code**: it is forbidden workspace-wide (`unsafe_code = "forbid"`).
- **Dependencies** (several hundred crates, mostly from the Slint ecosystem) are not audited line by
  line. `Cargo.lock` is committed, release builds use `--locked`, and CI runs `cargo deny`
  (`deny.toml`) on every push: it fails on any known vulnerability in the RustSec database, on
  licenses incompatible with the AGPL and on unknown registries. When last run it reported no known
  vulnerabilities. One direct dependency, `chrono`, is flagged as unmaintained (an advisory, not a
  vulnerability) and is explicitly excepted in `deny.toml`; replacing it is planned.
- **Desktops other than KDE Plasma 6 on X11** have not been verified (see `docs/COMPATIBILITY.md`).

## Getting a trustworthy binary

- Download from the Releases page and verify it as described in `docs/PROVENANCE.md`
  (`sha256sum -c`, the project signature and `gh attestation verify`).
- If you share a binary you built yourself, use `scripts/build-release.sh`. A plain
  `cargo build --release` embeds your home directory and user name in panic messages.

## Reporting a vulnerability

Please report privately through the repository's **Report a vulnerability** button (GitHub security
advisories) instead of a public issue. Include the version (`agents-roster --version`) and your
distribution and desktop.
