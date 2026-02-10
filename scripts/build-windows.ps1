#Requires -Version 5.0
<#
.SYNOPSIS
Saba-chan Windows Release Build
단일 배포 패키지 생성 (모든 언어, 모든 기능 포함)

.EXAMPLE
.\build-windows.ps1
#>

param(
    [string]$OutputDir = "release-build"
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$ReleaseDir = Join-Path $ProjectRoot $OutputDir
$DistDir = Join-Path $ReleaseDir "saba-chan-v0.1.0"

Write-Host "================================" -ForegroundColor Cyan
Write-Host "Saba-chan Windows Release Build" -ForegroundColor Cyan
Write-Host "================================" -ForegroundColor Cyan
Write-Host ""

# Clean
Write-Host "[1/5] Cleaning..." -ForegroundColor Yellow
if (Test-Path $ReleaseDir) {
    Remove-Item $ReleaseDir -Recurse -Force -ErrorAction SilentlyContinue
}
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
Write-Host "  [OK]" -ForegroundColor Green

# Build core daemon
Write-Host "[2/5] Building core daemon (Rust)..." -ForegroundColor Yellow
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

# Build GUI
Write-Host "[3/5] Building Electron GUI..." -ForegroundColor Yellow
try {
    Push-Location (Join-Path $ProjectRoot "saba-chan-gui")
    
    # Clean all build directories
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
    
    # Build
    npm run build --silent
    npm run package --silent
    
    $GuiExe = Get-ChildItem -Path "electron-dist" -Filter "*.exe" -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($GuiExe) {
        Copy-Item -Path $GuiExe.FullName -Destination (Join-Path $DistDir "saba-chan-gui.exe") -Force
        $Size = [Math]::Round($GuiExe.Length / 1MB, 2)
        Write-Host "  [OK] saba-chan-gui.exe ($Size MB)" -ForegroundColor Green
    }
    else {
        throw "GUI exe not found"
    }
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}

# Build Discord bot
Write-Host "[4/5] Building Discord bot..." -ForegroundColor Yellow
try {
    Push-Location (Join-Path $ProjectRoot "discord_bot")
    
    # Install dependencies for deployment
    if (Test-Path "node_modules") {
        Remove-Item "node_modules" -Recurse -Force -ErrorAction SilentlyContinue
    }
    npm install --production --quiet --no-save
    
    # Copy entire discord_bot folder to distribution
    $BotSrc = (Get-Location).Path
    $BotDest = Join-Path $DistDir "discord_bot"
    Copy-Item -Path $BotSrc -Destination $BotDest -Recurse -Force -ErrorAction SilentlyContinue
    
    $BotSize = 0
    Get-ChildItem -Path $BotDest -Recurse -File | ForEach-Object { $BotSize += $_.Length }
    $BotMB = [Math]::Round($BotSize / 1MB, 2)
    
    Write-Host "  [OK] Discord bot folder ($BotMB MB)" -ForegroundColor Green
}
catch {
    Write-Host "  [WARN] $_" -ForegroundColor Yellow
}
finally {
    Pop-Location
}

# Prepare resources
Write-Host "[5/5] Preparing resources and configs..." -ForegroundColor Yellow
try {
    # config 폴더 (최상위)
    $ConfigDir = Join-Path $DistDir "config"
    New-Item -ItemType Directory -Path $ConfigDir -Force | Out-Null
    Copy-Item -Path (Join-Path $ProjectRoot "config\global.toml") -Destination (Join-Path $ConfigDir "global.toml") -Force -ErrorAction SilentlyContinue
    Copy-Item -Path (Join-Path $ProjectRoot "instances.json") -Destination (Join-Path $ConfigDir "instances.json") -Force -ErrorAction SilentlyContinue
    
    # locales 폴더 (최상위)
    $LocaleSrc = Join-Path $ProjectRoot "locales"
    if (Test-Path $LocaleSrc) {
        Copy-Item -Path $LocaleSrc -Destination (Join-Path $DistDir "locales") -Recurse -Force -ErrorAction SilentlyContinue
    }
    
    Write-Host "  [OK]" -ForegroundColor Green
}
catch {
    Write-Host "  [ERROR] $_" -ForegroundColor Red
}

# Create ZIP
Write-Host ""
Write-Host "[Bonus] Creating ZIP..." -ForegroundColor Yellow

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
Write-Host "================================" -ForegroundColor Green
Write-Host "BUILD COMPLETE" -ForegroundColor Green
Write-Host "================================" -ForegroundColor Green
Write-Host "Output: $ReleaseDir" -ForegroundColor Cyan
Write-Host "  Uncompressed: $TotalMB MB" -ForegroundColor Gray
Write-Host "  Compressed:   $ZipSize MB" -ForegroundColor Gray
Write-Host ""
