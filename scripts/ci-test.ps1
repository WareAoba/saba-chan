#!/usr/bin/env pwsh
# CI/CD용 테스트 스크립트 (상세 출력)

param(
    [switch]$Coverage,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"

Write-Host "CI/CD Test Pipeline" -ForegroundColor Cyan
Write-Host "===================" -ForegroundColor Cyan
Write-Host ""

# 환경 정보
Write-Host "Environment:" -ForegroundColor Gray
Write-Host "  OS:       $([System.Environment]::OSVersion.Platform)" -ForegroundColor Gray
Write-Host "  Rust:     $(rustc --version)" -ForegroundColor Gray
Write-Host "  Node:     $(node --version)" -ForegroundColor Gray
Write-Host ""

$script:TestResults = @{
    Rust = @{ Passed = 0; Failed = 0; Duration = 0 }
    GUI = @{ Passed = 0; Failed = 0; Duration = 0 }
    Bot = @{ Passed = 0; Failed = 0; Duration = 0 }
}

# ===== Rust 테스트 =====
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "RUST DAEMON TESTS" -ForegroundColor Cyan
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan

$RustStart = Get-Date

if ($Coverage) {
    Write-Host "Running with coverage..." -ForegroundColor Yellow
    cargo tarpaulin --test daemon_integration --out Xml
    $RustExitCode = $LASTEXITCODE
} else {
    if ($Verbose) {
        cargo test --test daemon_integration -- --nocapture
    } else {
        cargo test --test daemon_integration
    }
    $RustExitCode = $LASTEXITCODE
}

$RustDuration = (Get-Date) - $RustStart
$script:TestResults.Rust.Duration = $RustDuration.TotalSeconds

if ($RustExitCode -eq 0) {
    Write-Host "✓ Rust tests passed" -ForegroundColor Green
    $script:TestResults.Rust.Passed = 37
} else {
    Write-Host "✗ Rust tests failed" -ForegroundColor Red
    $script:TestResults.Rust.Failed = 1
}

Write-Host ""

# ===== GUI 테스트 =====
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "ELECTRON GUI TESTS" -ForegroundColor Cyan
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan

$GuiStart = Get-Date

Push-Location electron_gui

if (-not (Test-Path "node_modules")) {
    Write-Host "Installing dependencies..." -ForegroundColor Yellow
    npm install
}

if ($Coverage) {
    npm test -- --coverage
    $GuiExitCode = $LASTEXITCODE
} else {
    npm test integration.test.js
    $GuiExitCode = $LASTEXITCODE
}

Pop-Location

$GuiDuration = (Get-Date) - $GuiStart
$script:TestResults.GUI.Duration = $GuiDuration.TotalSeconds

if ($GuiExitCode -eq 0) {
    Write-Host "✓ GUI tests passed" -ForegroundColor Green
    $script:TestResults.GUI.Passed = 1
} else {
    Write-Host "✗ GUI tests failed" -ForegroundColor Red
    $script:TestResults.GUI.Failed = 1
}

Write-Host ""

# ===== Bot 테스트 =====
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "DISCORD BOT TESTS" -ForegroundColor Cyan
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan

$BotStart = Get-Date

Push-Location discord_bot

if (-not (Test-Path "node_modules")) {
    Write-Host "Installing dependencies..." -ForegroundColor Yellow
    npm install
}

if ($Coverage) {
    npm test -- --coverage
    $BotExitCode = $LASTEXITCODE
} else {
    npm test integration.test.js
    $BotExitCode = $LASTEXITCODE
}

Pop-Location

$BotDuration = (Get-Date) - $BotStart
$script:TestResults.Bot.Duration = $BotDuration.TotalSeconds

if ($BotExitCode -eq 0) {
    Write-Host "✓ Bot tests passed" -ForegroundColor Green
    $script:TestResults.Bot.Passed = 1
} else {
    Write-Host "✗ Bot tests failed" -ForegroundColor Red
    $script:TestResults.Bot.Failed = 1
}

Write-Host ""

# ===== 최종 리포트 =====
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "TEST REPORT" -ForegroundColor Cyan
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host ""

$TotalPassed = $script:TestResults.Rust.Passed + $script:TestResults.GUI.Passed + $script:TestResults.Bot.Passed
$TotalFailed = $script:TestResults.Rust.Failed + $script:TestResults.GUI.Failed + $script:TestResults.Bot.Failed
$TotalDuration = $script:TestResults.Rust.Duration + $script:TestResults.GUI.Duration + $script:TestResults.Bot.Duration

Write-Host "Component          Status      Duration" -ForegroundColor Gray
Write-Host "────────────────────────────────────────" -ForegroundColor Gray

$RustStatus = if ($script:TestResults.Rust.Failed -eq 0) { "PASS" } else { "FAIL" }
$RustColor = if ($script:TestResults.Rust.Failed -eq 0) { "Green" } else { "Red" }
Write-Host "Rust Daemon        " -NoNewline
Write-Host "$RustStatus" -NoNewline -ForegroundColor $RustColor
Write-Host "       $($script:TestResults.Rust.Duration.ToString('0.00'))s"

$GuiStatus = if ($script:TestResults.GUI.Failed -eq 0) { "PASS" } else { "FAIL" }
$GuiColor = if ($script:TestResults.GUI.Failed -eq 0) { "Green" } else { "Red" }
Write-Host "Electron GUI       " -NoNewline
Write-Host "$GuiStatus" -NoNewline -ForegroundColor $GuiColor
Write-Host "       $($script:TestResults.GUI.Duration.ToString('0.00'))s"

$BotStatus = if ($script:TestResults.Bot.Failed -eq 0) { "PASS" } else { "FAIL" }
$BotColor = if ($script:TestResults.Bot.Failed -eq 0) { "Green" } else { "Red" }
Write-Host "Discord Bot        " -NoNewline
Write-Host "$BotStatus" -NoNewline -ForegroundColor $BotColor
Write-Host "       $($script:TestResults.Bot.Duration.ToString('0.00'))s"

Write-Host ""
Write-Host "Total Duration: $($TotalDuration.ToString('0.00'))s" -ForegroundColor Cyan

if ($TotalFailed -eq 0) {
    Write-Host ""
    Write-Host "✓ ALL TESTS PASSED" -ForegroundColor Black -BackgroundColor Green
    exit 0
} else {
    Write-Host ""
    Write-Host "✗ $TotalFailed COMPONENT(S) FAILED" -ForegroundColor White -BackgroundColor Red
    exit 1
}
