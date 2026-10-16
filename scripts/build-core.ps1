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
        cargo build -p saba-core -p saba-runner
    } else {
        Write-Host "[1/2] cargo build --release ..." -ForegroundColor Cyan
        cargo build --release -p saba-core -p saba-runner
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }

    $src = Join-Path $root "target\$profile\saba-core.exe"
    $binDir = Join-Path $root "saba-chan-gui\bin"
    $dst = Join-Path $binDir "saba-core.exe"

    if (-not (Test-Path $src)) { throw "Binary not found: $src" }

    Write-Host "[2/2] Copying binary to GUI bin ..." -ForegroundColor Cyan
    Copy-Item $src $dst -Force
    # saba-runner: managed 서버의 콘솔을 쥐는 감시 프로세스. 데몬 옆에 있어야 한다 (C8b)
    $runner = Join-Path $root "target\$profile\saba-runner.exe"
    if (-not (Test-Path $runner)) { throw "Binary not found: $runner" }
    Copy-Item $runner (Join-Path $binDir "saba-runner.exe") -Force
    $info = Get-Item $dst
    Write-Host ("  {0}  ({1:N0} KB  {2})" -f $info.Name, ($info.Length / 1KB), $info.LastWriteTime) -ForegroundColor Green
} finally {
    Pop-Location
}
