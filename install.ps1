<#
.SYNOPSIS
    Chimera Universal Installer for Windows
.DESCRIPTION
    Builds, installs, and automatically configures Chimera MCP Gateway and CLI across Windows AI harnesses (Antigravity, Cursor, Claude Desktop, Claude Code, Windsurf).
.EXAMPLE
    irm https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.ps1 | iex
    Or locally:
    .\install.ps1
#>

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

Write-Host @"
   ____ _     _                                
  / ___| |__ (_)_ __ ___   ___ _ __ __ _       
 | |   | '_ \| | '_ ` _ \ / _ \ '__/ _` |      
 | |___| | | | | | | | | |  __/ | | (_| |      
  \____|_| |_|_|_| |_| |_|\___|_|  \__,_|      
  All-in-One Dynamic MCP Gateway & Skill Engine
"@ -ForegroundColor Cyan

Write-Host "`n[1/5] Checking environment..." -ForegroundColor Yellow

# 1. Check Rust / Cargo
$cargoCmd = Get-Command "cargo" -ErrorAction SilentlyContinue
if (-not $cargoCmd) {
    Write-Host "Rust toolchain (cargo) not found on PATH." -ForegroundColor Yellow
    Write-Host "Downloading rustup-init.exe..." -ForegroundColor Cyan
    $rustupInstaller = Join-Path $env:TEMP "rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInstaller
    Write-Host "Running rustup installer..." -ForegroundColor Cyan
    Start-Process -FilePath $rustupInstaller -ArgumentList "-y" -Wait -NoNewWindow
    Remove-Item -Force $rustupInstaller -ErrorAction SilentlyContinue

    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path $cargoBin) {
        $env:PATH = "$cargoBin;$env:PATH"
    }
}

$cargoCmd = Get-Command "cargo" -ErrorAction SilentlyContinue
if (-not $cargoCmd) {
    Write-Error "Cargo could not be located. Please install Rust from https://rustup.rs and re-run this script."
}

# 2. Determine source directory
$chimeraDir = Join-Path $env:USERPROFILE ".chimera"
if (-not (Test-Path $chimeraDir)) {
    New-Item -ItemType Directory -Path $chimeraDir -Force | Out-Null
}

$scriptDir = $PSScriptRoot
if (-not $scriptDir) {
    $scriptDir = (Get-Location).Path
}

if ((Test-Path (Join-Path $scriptDir "Cargo.toml")) -and (Test-Path (Join-Path $scriptDir "chimera-cli"))) {
    $srcDir = $scriptDir
    Write-Host "[2/5] Building Chimera from local repository: $srcDir" -ForegroundColor Green
} else {
    $srcDir = Join-Path $chimeraDir "repo"
    if (Test-Path (Join-Path $srcDir ".git")) {
        Write-Host "[2/5] Updating Chimera source at $srcDir..." -ForegroundColor Cyan
        git -C $srcDir pull --ff-only | Out-Null
    } else {
        Write-Host "[2/5] Cloning Chimera repository into $srcDir..." -ForegroundColor Cyan
        git clone https://github.com/Wolfram-St/chimera-all-in-one-mcp.git $srcDir
    }
}

# 3. Build release binaries
Write-Host "[3/5] Compiling release binaries..." -ForegroundColor Cyan
Push-Location $srcDir
try {
    cargo build --release
} finally {
    Pop-Location
}

# 4. Install binaries
Write-Host "[4/5] Installing binaries and registering PATH..." -ForegroundColor Cyan
$installDir = Join-Path $env:USERPROFILE ".cargo\bin"
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

Copy-Item -Path (Join-Path $srcDir "target\release\chimera-cli.exe") -Destination (Join-Path $installDir "chimera-cli.exe") -Force
Copy-Item -Path (Join-Path $srcDir "target\release\chimera-proxy.exe") -Destination (Join-Path $installDir "chimera-proxy.exe") -Force

# Copy seeds to ~/.chimera if not existing
$registrySeed = Join-Path $srcDir "chimera_registry.json"
$targetRegistry = Join-Path $chimeraDir "chimera_registry.json"
if ((Test-Path $registrySeed) -and (-not (Test-Path $targetRegistry))) {
    Copy-Item -Path $registrySeed -Destination $targetRegistry -Force
}

$dbSeed = Join-Path $srcDir "registry.db"
$targetDb = Join-Path $chimeraDir "registry.db"
if ((Test-Path $dbSeed) -and (-not (Test-Path $targetDb))) {
    Copy-Item -Path $dbSeed -Destination $targetDb -Force
}

# Permanently add to User PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
if ($userPath -split ';' -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", [EnvironmentVariableTarget]::User)
    Write-Host "Added $installDir to persistent User PATH." -ForegroundColor Green
}
$env:PATH = "$installDir;$env:PATH"

# 5. Wire AI Harnesses
Write-Host "[5/5] Wiring Chimera Gateway into AI harnesses..." -ForegroundColor Cyan
$cliExe = Join-Path $installDir "chimera-cli.exe"
& $cliExe setup --all-clients

Write-Host @"
========================================================================
Chimera successfully installed and configured on Windows!
========================================================================
Quick Start Commands:
  chimera-cli list                 View installed and active MCP servers
  chimera-cli add <package|url>    Add zero-install MCP (npx:..., uvx:..., git)
  chimera-cli cache list           Inspect cache size across your drives
  chimera-cli cache clean          Reclaim disk space from orphaned repos
  chimera-cli setup --all-clients Wire Chimera into all agent clients
========================================================================
"@ -ForegroundColor Green
