#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
set -euo pipefail

cargo build --release -p agents-roster
sudo install -m 755 target/release/agents-roster /usr/local/bin/agents-roster
install -Dm 644 packaging/agents-roster.desktop "$HOME/.local/share/applications/agents-roster.desktop"
install -Dm 644 packaging/agents-roster-autostart.desktop "$HOME/.config/autostart/agents-roster.desktop"
echo "Installed /usr/local/bin/agents-roster and desktop entries."
