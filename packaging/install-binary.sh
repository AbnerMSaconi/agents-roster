#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
# Installer shipped inside the release tarball (installs the prebuilt binary).
set -euo pipefail
cd "$(dirname "$0")"

sudo install -m 755 agents-roster /usr/local/bin/agents-roster
install -Dm 644 packaging/agents-roster.desktop "$HOME/.local/share/applications/agents-roster.desktop"
install -Dm 644 packaging/agents-roster-autostart.desktop "$HOME/.config/autostart/agents-roster.desktop"
echo "Installed /usr/local/bin/agents-roster. See docs/INSTALL.md to set up the Claude Code hooks."
