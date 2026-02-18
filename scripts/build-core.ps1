#!/usr/bin/env pwsh
# saba-core 릴리스 빌드 후 GUI bin 폴더로 복사
param(
    [switch]$Debug,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

Push-Location $root
try {
    if ($Check) {
        Write-Host "[1/1] cargo check ..." -ForegroundColor Cyan
        cargo check
        if ($LASTEXITCODE -ne 0) { throw "cargo check failed (exit $LASTEXITCODE)" }
        Write-Host "OK" -ForegroundColor Green
        return
    }

    $profile = if ($Debug) { 'debug' } else { 'release' }

    if ($Debug) {
        Write-Host "[1/2] cargo build ..." -ForegroundColor Cyan
        cargo build
    } else {
        Write-Host "[1/2] cargo build --release ..." -ForegroundColor Cyan
        cargo build --release
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }

    $src = Join-Path $root "target\$profile\saba-core.exe"
    $binDir = Join-Path $root "saba-chan-gui\bin"
    $dst = Join-Path $binDir "saba-core.exe"

    if (-not (Test-Path $src)) { throw "Binary not found: $src" }

    Write-Host "[2/3] Copying binary to GUI bin ..." -ForegroundColor Cyan
    Copy-Item $src $dst -Force
    $info = Get-Item $dst
    Write-Host ("  {0}  ({1:N0} KB  {2})" -f $info.Name, ($info.Length / 1KB), $info.LastWriteTime) -ForegroundColor Green

    # ── extensions/ 폴더를 앱 루트(bin/../)에 복사 (바이너리 밖 외부 로드) ──
    Write-Host "[3/3] Syncing extensions/ ..." -ForegroundColor Cyan
    $extSrc = Join-Path $root "extensions"
    $guiRoot = Join-Path $root "saba-chan-gui"
    $extDst = Join-Path $guiRoot "extensions"
    if (Test-Path $extSrc) {
        if (-not (Test-Path $extDst)) { New-Item -ItemType Directory -Path $extDst -Force | Out-Null }
        Copy-Item (Join-Path $extSrc "*.py") $extDst -Force
        $count = (Get-ChildItem $extDst -Filter "*.py").Count
        Write-Host "  $count extension(s) synced to $extDst" -ForegroundColor Green
    } else {
        Write-Host "  extensions/ not found — skipped" -ForegroundColor Yellow
    }
} finally {
    Pop-Location
}
