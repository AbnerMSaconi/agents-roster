# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
#
# Agents Roster — Windows 11 installer (per-user, no elevation).
#
#   .\install.ps1                 build from source (needs Rust) and install
#   .\install.ps1 -Binary .\agents-roster.exe   install a prebuilt binary (release zip)
#   .\install.ps1 -Autostart      also start with Windows (HKCU Run key)
#
# Installs to %LOCALAPPDATA%\Programs\agents-roster, adds it to the user PATH and prints
# the Claude Code hooks snippet. Uninstall with .\uninstall.ps1.

[CmdletBinding()]
param(
    [string]$Binary = "",
    [switch]$Autostart
)

$ErrorActionPreference = "Stop"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\agents-roster"
$ExeName = "agents-roster.exe"

# --- Locate or build the binary -------------------------------------------------------------
if ($Binary) {
    if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { throw "Binary not found: $Binary" }
    $src = (Resolve-Path -LiteralPath $Binary).Path
} else {
    $repoRoot = Split-Path -Parent $PSScriptRoot
    if (-not (Test-Path (Join-Path $repoRoot "Cargo.toml"))) {
        # Running from a release zip without -Binary: look next to this script.
        $nextToScript = Join-Path $PSScriptRoot $ExeName
        $nextToPackage = Join-Path $repoRoot $ExeName
        if (Test-Path -LiteralPath $nextToScript -PathType Leaf) {
            $src = $nextToScript
        } elseif (Test-Path -LiteralPath $nextToPackage -PathType Leaf) {
            $src = $nextToPackage
        } else {
            throw "No prebuilt binary found. Pass -Binary .\agents-roster.exe or run from the source tree with Rust installed."
        }
    } else {
        Write-Host "Building release binary (cargo build --release)..." -ForegroundColor Cyan
        Push-Location $repoRoot
        try {
            cargo build --release --locked
            if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
        } finally { Pop-Location }
        $src = Join-Path $repoRoot "target\release\$ExeName"
        if (-not (Test-Path -LiteralPath $src -PathType Leaf)) { throw "Build completed but executable was not found: $src" }
    }
}

# --- Stop a running instance and copy -------------------------------------------------------
Get-Process agents-roster -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $src (Join-Path $InstallDir $ExeName) -Force
$exe = Join-Path $InstallDir $ExeName
Write-Host "Installed: $exe" -ForegroundColor Green

# --- User PATH (so Claude Code hooks can call `agents-roster` by name) -----------------------
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not ($userPath -split ";" | Where-Object { $_ -eq $InstallDir })) {
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    # Broadcast the change so newly launched apps pick it up without a logoff.
    if (-not ("Win32.NativeMethods" -as [type])) {
        Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
    }
    $result = [UIntPtr]::Zero
    [Win32.NativeMethods]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, "Environment", 2, 5000, [ref]$result) | Out-Null
    Write-Host "Added to user PATH (open a NEW terminal to pick it up)." -ForegroundColor Green
}

# --- Optional autostart ----------------------------------------------------------------------
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
if ($Autostart) {
    Set-ItemProperty -Path $runKey -Name "AgentsRoster" -Value "`"$exe`""
    Write-Host "Autostart enabled (HKCU Run key)." -ForegroundColor Green
}

# --- Claude Code hooks snippet ---------------------------------------------------------------
$hookExe = $exe -replace "\\", "/"
Write-Host ""
Write-Host "Add this to your Claude Code settings (~/.claude/settings.json) to connect hooks:" -ForegroundColor Cyan
Write-Host @"
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "command": "$hookExe collect SessionStart" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "$hookExe collect UserPromptSubmit" }] }],
    "PreToolUse":       [{ "hooks": [{ "type": "command", "command": "$hookExe collect PreToolUse" }] }],
    "PostToolUse":      [{ "hooks": [{ "type": "command", "command": "$hookExe collect PostToolUse" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "$hookExe collect Notification" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "$hookExe collect Stop" }] }],
    "SubagentStart":    [{ "hooks": [{ "type": "command", "command": "$hookExe collect SubagentStart" }] }],
    "SubagentStop":     [{ "hooks": [{ "type": "command", "command": "$hookExe collect SubagentStop" }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "$hookExe collect SessionEnd" }] }]
  },
  "statusLine": { "type": "command", "command": "$hookExe statusline" }
}
"@
Write-Host ""
Write-Host "Start the panel with:  & `"$exe`"" -ForegroundColor Cyan
Write-Host "See docs/INSTALL-WINDOWS.md for details and troubleshooting."
