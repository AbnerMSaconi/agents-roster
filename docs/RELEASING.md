# Releasing

Every release is identified in four places that must agree: the `version` in the root
`Cargo.toml`, a `## <version>` section in `CHANGELOG.md`, the git tag `v<version>`, and the build
metadata inside the binary (`agents-roster --version` prints the version and commit).

## Steps

1. Update `version` under `[workspace.package]` in `Cargo.toml` and run `cargo build` so
   `Cargo.lock` follows.
2. Add a `## <version>` section to `CHANGELOG.md`. Its text becomes the release notes
   (`scripts/release-notes.sh <version>` previews them).
3. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` and `cargo deny check`.
4. Commit, push and wait for CI to pass.
5. Create a signed tag and push it: `git tag -s v<version> -m "Agents Roster <version>"` then
   `git push origin v<version>`.

The release workflow then fails if the tag does not match `Cargo.toml` or the CHANGELOG section is
missing, builds on Ubuntu 22.04 with remapped paths, publishes the tarball, its `.sha256` and a signed
build provenance attestation, and marks `0.x` releases as pre-releases.

## Signing with the project key

After the workflow finishes, on the author's machine:

```bash
gh release download v<version> -R AbnerMSaconi/agents-roster -D /tmp/rel
scripts/sign-release.sh ~/.ssh/agents-roster-release /tmp/rel/*.tar.gz
gh release upload v<version> /tmp/rel/*.sig -R AbnerMSaconi/agents-roster
```

Binaries you share must be built with `scripts/build-release.sh`, which strips your home directory
and username from the paths Rust embeds in panic messages. A binary built locally without git
history reports `commit unknown`.
