# Windows 11 installation

The Windows port is an active, not yet independently verified port. Use a 64-bit Windows 11 machine for local builds and runtime testing.

## Requirements

- Windows 11 x64.
- Rust 1.93 or newer with the `x86_64-pc-windows-msvc` toolchain.
- Visual Studio 2022 or newer Build Tools with the **Desktop development with C++** workload and a Windows 10/11 SDK.
- Git (to clone the repository).

Rust's MSVC target needs the Visual Studio linker and Windows SDK. See [Microsoft's Rust setup guide](https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup).

## Build and verify

Open PowerShell in the repository root:

```powershell
rustup show active-toolchain
rustup target list --installed
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release --locked
```

The default MSVC toolchain should be `stable-x86_64-pc-windows-msvc` (or the pinned 1.93.0 equivalent). If needed, select it with `rustup default stable-x86_64-pc-windows-msvc`.

## Install

From the repository root, run PowerShell as the current user (no elevation required):

```powershell
.\packaging\install.ps1 -Autostart
```

From an extracted release archive, run `packaging\install.ps1` from the archive root; the script locates the executable beside the `packaging` directory. To install a custom build, pass its path explicitly:

```powershell
.\packaging\install.ps1 -Binary .\target\release\agents-roster.exe
```

The installer copies the executable to `%LOCALAPPDATA%\Programs\agents-roster`, adds that directory to the current user's `PATH`, and optionally enables startup through the current user's Run registry key. Open a new terminal after changing `PATH`.

Uninstall with:

```powershell
.\packaging\uninstall.ps1
```

Add `-Purge` only if you also want to delete the application's configuration and hook exchange data. Back up anything you want to keep first.

## Claude Code hooks

The installer prints the hooks and status-line configuration to add to `~/.claude/settings.json`. The exchange directory defaults to `%LOCALAPPDATA%\agents-roster\run`; the `AGENTS_ROSTER_DIR` environment variable or `--dir` argument can override it.

## Current limitations

- Process command-line and current-directory inspection relies on reading another process's PEB. Windows may deny access to elevated, protected, or otherwise restricted processes; those processes can appear with reduced details.
- Open-file path discovery is best-effort and deliberately bounded to avoid blocking the UI refresh.
- Windows toast notifications depend on the per-user AppUserModelId/Start Menu registration.
- The Windows path must be exercised on a real Windows 11 machine; a successful CI build does not validate tray lifecycle, notification behavior, window hints, or all process-access cases.
