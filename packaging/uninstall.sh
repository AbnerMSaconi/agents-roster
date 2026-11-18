#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
# Removes the files installed by packaging/install.sh. Configuration in
# ~/.config/agents-roster is kept; delete it by hand for a clean slate.
set -euo pipefail

sudo rm -f /usr/local/bin/agents-roster
rm -f "$HOME/.local/share/applications/agents-roster.desktop" \
      "$HOME/.config/autostart/agents-roster.desktop"
echo "Removed agents-roster. Remember to delete its hooks from ~/.claude/settings.json."
