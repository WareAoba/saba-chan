#Requires -Version 5.0
<#
.SYNOPSIS
Saba-chan Windows Release Build
단일 배포 패키지 생성 (모든 언어, 모든 기능 포함)

배포물 구성:
  core_daemon.exe   — Rust IPC 데몬
  saba-cli.exe      — Rust TUI 클라이언트 (아이콘 임베드)
  saba-chan-gui.exe  — Electron GUI
  discord_bot/      — Node.js Discord 봇
  config/           — global.toml
  locales/          — 다국어 리소스

모듈(modules/)은 사용자가 별도로 설치하므로 포함하지 않습니다.

.EXAMPLE
.\build-windows.ps1
.\build-windows.ps1 -OutputDir "my-build"
#>

param(
    [string]$OutputDir = "release-build"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$ReleaseDir = Join-Path $ProjectRoot $OutputDir
$DistDir = Join-Path $ReleaseDir "saba-chan-v0.1.0"

Write-Host "================================" -ForegroundColor Cyan
Write-Host "Saba-chan Windows Release Build" -ForegroundColor Cyan
Write-Host "================================" -ForegroundColor Cyan
Write-Host ""

# ─── 1. Clean ───
Write-Host "[1/6] Cleaning..." -ForegroundColor Yellow
if (Test-Path $ReleaseDir) {
    Remove-Item $ReleaseDir -Recurse -Force -ErrorAction SilentlyContinue
}
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
Write-Host "  [OK]" -ForegroundColor Green

# ─── 2. Core Daemon (Rust) ───
Write-Host "[2/6] Building core daemon (Rust)..." -ForegroundColor Yellow
try {
    Push-Location $ProjectRoot
    cargo build --release --quiet
    
    $DaemonExe = "target\release\core_daemon.exe"
    if (-not (Test-Path $DaemonExe)) {
        throw "core_daemon.exe not found"
    }
    
    Copy-Item -Path $DaemonExe -Destination $DistDir -Force
    $Size = [Math]::Round((Get-Item $DaemonExe).Length / 1MB, 2)
    Write-Host "  [OK] core_daemon.exe ($Size MB)" -ForegroundColor Green
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}

# ─── 3. CLI (Rust, saba-cli.exe) ───
Write-Host "[3/6] Building CLI (Rust)..." -ForegroundColor Yellow
try {
    Push-Location (Join-Path $ProjectRoot "saba-chan-cli")
    cargo build --release --quiet
    
    $CliExe = "target\release\saba-cli.exe"
    if (-not (Test-Path $CliExe)) {
        throw "saba-cli.exe not found"
    }
    
    Copy-Item -Path $CliExe -Destination $DistDir -Force
    $Size = [Math]::Round((Get-Item $CliExe).Length / 1MB, 2)
    Write-Host "  [OK] saba-cli.exe ($Size MB)" -ForegroundColor Green
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}

# ─── 4. Electron GUI ───
Write-Host "[4/6] Building Electron GUI..." -ForegroundColor Yellow
try {
    Push-Location (Join-Path $ProjectRoot "saba-chan-gui")
    
    # Clean build directories
    @("dist", "build", "electron-dist") | ForEach-Object {
        if (Test-Path $_) {
            Remove-Item $_ -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Start-Sleep -Milliseconds 500
    
    # Install dependencies
    if (-not (Test-Path "node_modules")) {
        npm install --quiet --no-save
    }
    npm install --save-dev terser --quiet --no-save 2>$null
    
    # Vite build → Electron package
    npm run build --silent
    npm run package --silent
    
    $GuiExe = Get-ChildItem -Path "electron-dist" -Filter "*.exe" -Recurse -ErrorAction SilentlyContinue |
              Select-Object -First 1
    if ($GuiExe) {
        Copy-Item -Path $GuiExe.FullName -Destination (Join-Path $DistDir "saba-chan-gui.exe") -Force
        $Size = [Math]::Round($GuiExe.Length / 1MB, 2)
        Write-Host "  [OK] saba-chan-gui.exe ($Size MB)" -ForegroundColor Green
    }
    else {
        throw "GUI exe not found in electron-dist/"
    }
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}

# ─── 5. Discord Bot ───
Write-Host "[5/6] Bundling Discord bot..." -ForegroundColor Yellow
try {
    Push-Location (Join-Path $ProjectRoot "discord_bot")
    
    # Production dependencies only
    if (Test-Path "node_modules") {
        Remove-Item "node_modules" -Recurse -Force -ErrorAction SilentlyContinue
    }
    npm install --production --quiet --no-save
    
    $BotDest = Join-Path $DistDir "discord_bot"
    Copy-Item -Path (Get-Location).Path -Destination $BotDest -Recurse -Force -ErrorAction SilentlyContinue
    
    # Remove test files from dist
    $TestDir = Join-Path $BotDest "test"
    if (Test-Path $TestDir) {
        Remove-Item $TestDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    
    $BotSize = 0
    Get-ChildItem -Path $BotDest -Recurse -File | ForEach-Object { $BotSize += $_.Length }
    $BotMB = [Math]::Round($BotSize / 1MB, 2)
    
    Write-Host "  [OK] discord_bot/ ($BotMB MB)" -ForegroundColor Green
}
catch {
    Write-Host "  [WARN] Discord bot skipped: $_" -ForegroundColor Yellow
}
finally {
    Pop-Location
}

# ─── 6. Resources & Configs ───
Write-Host "[6/6] Preparing resources..." -ForegroundColor Yellow
try {
    # config/
    $ConfigDir = Join-Path $DistDir "config"
    New-Item -ItemType Directory -Path $ConfigDir -Force | Out-Null
    Copy-Item -Path (Join-Path $ProjectRoot "config\global.toml") -Destination $ConfigDir -Force -ErrorAction SilentlyContinue
    
    # locales/ (전체 다국어 리소스)
    $LocaleSrc = Join-Path $ProjectRoot "locales"
    if (Test-Path $LocaleSrc) {
        Copy-Item -Path $LocaleSrc -Destination (Join-Path $DistDir "locales") -Recurse -Force
    }
    
    Write-Host "  [OK]" -ForegroundColor Green
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
}

# ─── ZIP ───
Write-Host ""
Write-Host "[ZIP] Creating archive..." -ForegroundColor Yellow

$ZipPath = Join-Path $ReleaseDir "saba-chan-v0.1.0-windows.zip"
if (Test-Path $ZipPath) {
    Remove-Item $ZipPath -Force
}
Compress-Archive -Path $DistDir -DestinationPath $ZipPath -Force

$TotalSize = 0
Get-ChildItem -Path $DistDir -Recurse -File | ForEach-Object { $TotalSize += $_.Length }
$TotalMB = [Math]::Round($TotalSize / 1MB, 2)
$ZipSize = [Math]::Round((Get-Item $ZipPath).Length / 1MB, 2)

Write-Host "  [OK]" -ForegroundColor Green
Write-Host ""

# ─── Summary ───
Write-Host "================================" -ForegroundColor Green
Write-Host "BUILD COMPLETE" -ForegroundColor Green
Write-Host "================================" -ForegroundColor Green
Write-Host "Output: $DistDir" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Contents:" -ForegroundColor Gray
Get-ChildItem -Path $DistDir | ForEach-Object {
    if ($_.PSIsContainer) {
        $DirSize = 0
        Get-ChildItem -Path $_.FullName -Recurse -File | ForEach-Object { $DirSize += $_.Length }
        $DirMB = [Math]::Round($DirSize / 1MB, 2)
        Write-Host "    $($_.Name)/  ($DirMB MB)" -ForegroundColor Gray
    }
    else {
        $FileMB = [Math]::Round($_.Length / 1MB, 2)
        Write-Host "    $($_.Name)  ($FileMB MB)" -ForegroundColor Gray
    }
}
Write-Host ""
Write-Host "  Total:      $TotalMB MB" -ForegroundColor White
Write-Host "  ZIP:        $ZipSize MB  ($ZipPath)" -ForegroundColor White
Write-Host ""
