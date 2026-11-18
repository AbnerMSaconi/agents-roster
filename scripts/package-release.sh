#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
# Builds a release tarball: dist/agents-roster-<version>-linux-<arch>.tar.gz (+ .sha256).
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
arch=$(uname -m)
name="agents-roster-${version}-linux-${arch}"
stage="dist/${name}"

./scripts/build-release.sh
rm -rf "dist/${name}" "dist/${name}.tar.gz"
mkdir -p "${stage}/packaging" "${stage}/docs"
install -m 755 target/release/agents-roster "${stage}/agents-roster"
install -m 644 packaging/*.desktop "${stage}/packaging/"
install -m 755 packaging/install-binary.sh "${stage}/install.sh"
install -m 755 packaging/uninstall.sh "${stage}/uninstall.sh"
cp docs/INSTALL.md docs/USAGE.md docs/COMPATIBILITY.md "${stage}/docs/"
cp README.md LICENSE NOTICE THIRD_PARTY.md CHANGELOG.md SECURITY.md "${stage}/"
cp docs/PROVENANCE.md "${stage}/docs/"
cp -r docs/keys "${stage}/docs/keys"
mkdir -p "${stage}/scripts"
install -m 755 scripts/verify-release.sh "${stage}/scripts/"
{
  echo "name: agents-roster"
  echo "version: ${version}"
  echo "commit: ${AGENTS_ROSTER_COMMIT:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
  echo "target: linux-${arch}"
  echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "rustc: $(rustc --version)"
} > "${stage}/BUILD-INFO.txt"

tar -C dist -czf "dist/${name}.tar.gz" "${name}"
(cd dist && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256")
echo "Built dist/${name}.tar.gz"
