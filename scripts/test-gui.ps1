#!/usr/bin/env pwsh
# GUI 테스트 빠른 실행 (진행 상황 표시)

Write-Host "=================================" -ForegroundColor Cyan
Write-Host "   Electron GUI Integration Test " -ForegroundColor Cyan
Write-Host "=================================" -ForegroundColor Cyan
Write-Host ""

$StartTime = Get-Date

# Daemon 빌드 확인
$DaemonPath = "target\release\core_daemon.exe"

if (-not (Test-Path $DaemonPath)) {
    Write-Host "⚠ Daemon not built yet" -ForegroundColor Yellow
    Write-Host "Building daemon (this may take 3-5 minutes on first run)..." -ForegroundColor Yellow
    Write-Host ""
    
    cargo build --release
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "❌ Daemon build failed" -ForegroundColor Red
        exit 1
    }
    
    Write-Host "✅ Daemon built successfully" -ForegroundColor Green
    Write-Host ""
}

# GUI 테스트 실행
Write-Host "[1/1] Running Electron GUI Tests..." -ForegroundColor Yellow
Write-Host ""

Push-Location saba-chan-gui

# npm install 확인
if (-not (Test-Path "node_modules")) {
    Write-Host "Installing dependencies..." -ForegroundColor Yellow
    npm install
}

# 테스트 실행 (verbose 모드)
npm test integration.test.js -- --verbose

$ExitCode = $LASTEXITCODE

Pop-Location

# 결과
$EndTime = Get-Date
$Duration = ($EndTime - $StartTime).TotalSeconds

Write-Host ""
Write-Host "=================================" -ForegroundColor Cyan
Write-Host "Duration: $($Duration.ToString('0.00'))s" -ForegroundColor Cyan

if ($ExitCode -eq 0) {
    Write-Host "✅ All tests passed!" -ForegroundColor Green
    exit 0
} else {
    Write-Host "❌ Tests failed" -ForegroundColor Red
    exit 1
}
