# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 AbnerMSaconi
#
# Agents Roster — Windows 11 uninstaller (per-user).
#
#   .\uninstall.ps1           remove the app, PATH entry, autostart and toast identity
#   .\uninstall.ps1 -Purge    also delete configuration and hook exchange data

[CmdletBinding()]
param([switch]$Purge)

$ErrorActionPreference = "SilentlyContinue"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\agents-roster"

Get-Process agents-roster | Stop-Process -Force
Start-Sleep -Milliseconds 400

# Autostart
Remove-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "AgentsRoster"

# User PATH entry
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath) {
    $kept = ($userPath -split ";" | Where-Object { $_ -and $_ -ne $InstallDir }) -join ";"
    [Environment]::SetEnvironmentVariable("Path", $kept, "User")
}

# Toast identity (AppUserModelId registration + Start Menu shortcut)
Remove-Item -Recurse -Force "HKCU:\Software\Classes\AppUserModelId\AbnerMSaconi.AgentsRoster"
$shortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Agents Roster.lnk"
Remove-Item -Force $shortcut
$shortcutPt = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Agentes.lnk"
Remove-Item -Force $shortcutPt

# Binary
Remove-Item -Recurse -Force $InstallDir

if ($Purge) {
    Remove-Item -Recurse -Force (Join-Path $env:APPDATA "agents-roster")
    Remove-Item -Recurse -Force (Join-Path $env:LOCALAPPDATA "agents-roster")
    Write-Host "Configuration and exchange data removed." -ForegroundColor Yellow
}

Write-Host "Agents Roster uninstalled." -ForegroundColor Green
Write-Host "Remember to remove the agents-roster hooks from ~/.claude/settings.json if you added them."
