#!/usr/bin/env pwsh
# Saba-chan 전체 테스트 실행 스크립트

$ErrorActionPreference = "Continue"

Write-Host "=================================" -ForegroundColor Cyan
Write-Host "   Saba-chan Test Suite Runner   " -ForegroundColor Cyan
Write-Host "=================================" -ForegroundColor Cyan
Write-Host ""

$TotalTests = 0
$PassedTests = 0
$FailedTests = 0
$SkippedTests = 0

# 시작 시간
$StartTime = Get-Date

# ===== 1. Rust Daemon 통합 테스트 =====
Write-Host "[1/3] Running Rust Daemon Integration Tests..." -ForegroundColor Yellow
Write-Host "      Location: tests/daemon_integration.rs" -ForegroundColor Gray
Write-Host ""

$RustOutput = cargo test --test daemon_integration 2>&1
$RustExitCode = $LASTEXITCODE

if ($RustExitCode -eq 0) {
    Write-Host "✓ Rust Tests: PASSED" -ForegroundColor Green
    
    # 테스트 수 추출
    $RustCount = 0
    if ($RustOutput -match "(\d+) passed") {
        $RustCount = [int]$Matches[1]
    }
    
    $TotalTests += $RustCount
    $PassedTests += $RustCount
} else {
    Write-Host "✗ Rust Tests: FAILED" -ForegroundColor Red
    
    # 실패 정보 추출
    if ($RustOutput -match "(\d+) passed") {
        $Passed = [int]$Matches[1]
        $TotalTests += $Passed
        $PassedTests += $Passed
    }
    if ($RustOutput -match "(\d+) failed") {
        $Failed = [int]$Matches[1]
        $TotalTests += $Failed
        $FailedTests += $Failed
    }
}

Write-Host ""
Write-Host "----------------------------------------" -ForegroundColor Gray

# ===== 2. Electron GUI 통합 테스트 =====
Write-Host "[2/3] Running Electron GUI Integration Tests..." -ForegroundColor Yellow
Write-Host "      Location: saba-chan-gui/src/test/integration.test.js" -ForegroundColor Gray
Write-Host ""

Push-Location saba-chan-gui

# npm install 확인
if (-not (Test-Path "node_modules")) {
    Write-Host "      Installing dependencies..." -ForegroundColor Gray
    npm install --silent
}

$GuiOutput = npm test integration.test.js 2>&1
$GuiExitCode = $LASTEXITCODE

Pop-Location

if ($GuiExitCode -eq 0) {
    Write-Host "✓ Electron GUI Tests: PASSED" -ForegroundColor Green
    
    # Jest 테스트 수 추출
    if ($GuiOutput -match "Tests:\s+(\d+) passed") {
        $GuiCount = [int]$Matches[1]
        $TotalTests += $GuiCount
        $PassedTests += $GuiCount
    }
} else {
    Write-Host "✗ Electron GUI Tests: FAILED" -ForegroundColor Red
    
    if ($GuiOutput -match "Tests:\s+(\d+) failed.*?(\d+) passed") {
        $Failed = [int]$Matches[1]
        $Passed = [int]$Matches[2]
        $TotalTests += ($Failed + $Passed)
        $PassedTests += $Passed
        $FailedTests += $Failed
    } elseif ($GuiOutput -match "Tests:\s+(\d+) passed") {
        $Passed = [int]$Matches[1]
        $TotalTests += $Passed
        $PassedTests += $Passed
    }
}

Write-Host ""
Write-Host "----------------------------------------" -ForegroundColor Gray

# ===== 3. Discord Bot 통합 테스트 =====
Write-Host "[3/3] Running Discord Bot Integration Tests..." -ForegroundColor Yellow
Write-Host "      Location: discord_bot/test/integration.test.js" -ForegroundColor Gray
Write-Host ""

Push-Location discord_bot

# npm install 확인
if (-not (Test-Path "node_modules")) {
    Write-Host "      Installing dependencies..." -ForegroundColor Gray
    npm install --silent
}

$BotOutput = npm test integration.test.js 2>&1
$BotExitCode = $LASTEXITCODE

Pop-Location

if ($BotExitCode -eq 0) {
    Write-Host "✓ Discord Bot Tests: PASSED" -ForegroundColor Green
    
    if ($BotOutput -match "Tests:\s+(\d+) passed") {
        $BotCount = [int]$Matches[1]
        $TotalTests += $BotCount
        $PassedTests += $BotCount
    }
} else {
    Write-Host "✗ Discord Bot Tests: FAILED" -ForegroundColor Red
    
    if ($BotOutput -match "Tests:\s+(\d+) failed.*?(\d+) passed") {
        $Failed = [int]$Matches[1]
        $Passed = [int]$Matches[2]
        $TotalTests += ($Failed + $Passed)
        $PassedTests += $Passed
        $FailedTests += $Failed
    } elseif ($BotOutput -match "Tests:\s+(\d+) passed") {
        $Passed = [int]$Matches[1]
        $TotalTests += $Passed
        $PassedTests += $Passed
    }
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan

# ===== 최종 결과 =====
$EndTime = Get-Date
$Duration = $EndTime - $StartTime

Write-Host ""
Write-Host "            TEST SUMMARY               " -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

Write-Host "  Total Tests:    " -NoNewline
Write-Host "$TotalTests" -ForegroundColor White

Write-Host "  Passed:         " -NoNewline
Write-Host "$PassedTests" -ForegroundColor Green

if ($FailedTests -gt 0) {
    Write-Host "  Failed:         " -NoNewline
    Write-Host "$FailedTests" -ForegroundColor Red
}

if ($SkippedTests -gt 0) {
    Write-Host "  Skipped:        " -NoNewline
    Write-Host "$SkippedTests" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "  Duration:       " -NoNewline
Write-Host "$($Duration.TotalSeconds.ToString('0.00')) seconds" -ForegroundColor White

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 전체 성공 여부
$AllPassed = ($RustExitCode -eq 0) -and ($GuiExitCode -eq 0) -and ($BotExitCode -eq 0)

if ($AllPassed) {
    Write-Host "  ✓ ALL TESTS PASSED  " -ForegroundColor Black -BackgroundColor Green
    Write-Host ""
    exit 0
} else {
    Write-Host "  ✗ SOME TESTS FAILED  " -ForegroundColor White -BackgroundColor Red
    Write-Host ""
    
    # 실패한 컴포넌트 출력
    Write-Host "Failed Components:" -ForegroundColor Red
    if ($RustExitCode -ne 0) { Write-Host "  - Rust Daemon" -ForegroundColor Red }
    if ($GuiExitCode -ne 0) { Write-Host "  - Electron GUI" -ForegroundColor Red }
    if ($BotExitCode -ne 0) { Write-Host "  - Discord Bot" -ForegroundColor Red }
    Write-Host ""
    
    exit 1
}
