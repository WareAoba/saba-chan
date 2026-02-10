#!/usr/bin/env pwsh
# Saba-chan 빠른 테스트 실행 (데몬 제외)
# Daemon 빌드 없이 JavaScript 테스트만 실행

Write-Host "=================================" -ForegroundColor Cyan
Write-Host "   Quick Test (JS Only)          " -ForegroundColor Cyan
Write-Host "=================================" -ForegroundColor Cyan
Write-Host ""

$TotalTests = 0
$PassedTests = 0

# ===== 1. Electron GUI 테스트 =====
Write-Host "[1/2] Electron GUI Tests..." -ForegroundColor Yellow

Push-Location saba-chan-gui
$GuiOutput = npm test integration.test.js 2>&1
$GuiExitCode = $LASTEXITCODE
Pop-Location

if ($GuiExitCode -eq 0) {
    Write-Host "✓ GUI: PASSED" -ForegroundColor Green
    if ($GuiOutput -match "Tests:\s+(\d+) passed") {
        $PassedTests += [int]$Matches[1]
        $TotalTests += [int]$Matches[1]
    }
} else {
    Write-Host "✗ GUI: FAILED" -ForegroundColor Red
}

Write-Host ""

# ===== 2. Discord Bot 테스트 =====
Write-Host "[2/2] Discord Bot Tests..." -ForegroundColor Yellow

Push-Location discord_bot
$BotOutput = npm test integration.test.js 2>&1
$BotExitCode = $LASTEXITCODE
Pop-Location

if ($BotExitCode -eq 0) {
    Write-Host "✓ Bot: PASSED" -ForegroundColor Green
    if ($BotOutput -match "Tests:\s+(\d+) passed") {
        $PassedTests += [int]$Matches[1]
        $TotalTests += [int]$Matches[1]
    }
} else {
    Write-Host "✗ Bot: FAILED" -ForegroundColor Red
}

Write-Host ""
Write-Host "Total: $PassedTests / $TotalTests passed" -ForegroundColor Cyan

if ($GuiExitCode -eq 0 -and $BotExitCode -eq 0) {
    exit 0
} else {
    exit 1
}
