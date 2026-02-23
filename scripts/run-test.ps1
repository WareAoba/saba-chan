#!/usr/bin/env pwsh
# ═══════════════════════════════════════════════════════════
# Saba-chan Unified Test Runner (Windows / PowerShell)
# ═══════════════════════════════════════════════════════════
# 전체 코드베이스 테스트를 한 번에 실행:
#   1. Rust 데몬 통합 테스트
#   2. Rust 업데이터 통합 테스트
#   3. 릴레이 서버 E2E (Vitest + PostgreSQL)
#   4. GUI E2E (Vitest + jsdom)
#   5. Discord 봇 통합 (Jest)
#
# 사용법:
#   ./run-test.ps1                  # 전체
#   ./run-test.ps1 -Suite gui       # 특정 스위트만
#   ./run-test.ps1 -NoInstall       # npm install 건너뛰기
#   ./run-test.ps1 -Verbose         # 상세 출력

param(
    [switch]$NoInstall,
    [switch]$Verbose,
    [ValidateSet('all','rust','relay','gui','discord')]
    [string]$Suite = 'all'
)

$ErrorActionPreference = "Continue"

# ── 경로 해석 ──────────────────────────────────────────────
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = if ((Split-Path -Leaf $ScriptDir) -ieq "scripts") {
    Split-Path -Parent $ScriptDir
} else { $ScriptDir }

# server-chan 은 워크스페이스 형제 또는 saba-chan 내부 모노레포 심링크
$RelayServerDir = $null
foreach ($candidate in @(
    (Join-Path (Split-Path -Parent $RepoRoot) "server-chan" "relay-server"),
    (Join-Path $RepoRoot "server-chan" "relay-server")
)) {
    if (Test-Path $candidate) { $RelayServerDir = $candidate; break }
}

Set-Location $RepoRoot

$results = New-Object System.Collections.Generic.List[object]

# ── 유틸 ────────────────────────────────────────────────────
function Write-Section($title) {
    Write-Host ""
    Write-Host ("=" * 58) -ForegroundColor Cyan
    Write-Host " $title" -ForegroundColor Cyan
    Write-Host ("=" * 58) -ForegroundColor Cyan
}

function Ensure-Npm($dir, $label) {
    if ($NoInstall) { return }
    if (-not (Test-Path (Join-Path $dir "node_modules"))) {
        Write-Host "[$label] Installing dependencies..." -ForegroundColor Yellow
        Push-Location $dir
        npm install --silent 2>&1 | Out-Null
        $code = $LASTEXITCODE; Pop-Location
        if ($code -ne 0) { throw "[$label] npm install failed (exit $code)" }
    }
}

function Run-Step($name, $workingDir, $command) {
    Write-Host ""
    Write-Host "[$name] $command" -ForegroundColor Yellow
    $start = Get-Date
    Push-Location $workingDir

    if ($Verbose) { Invoke-Expression $command }
    else { Invoke-Expression $command 2>&1 | Out-Host }

    $exitCode = $LASTEXITCODE; Pop-Location
    $dur = [math]::Round(((Get-Date) - $start).TotalSeconds, 2)
    $status = if ($exitCode -eq 0) { "PASS" } else { "FAIL" }

    $results.Add([PSCustomObject]@{ Name=$name; Status=$status; Exit=$exitCode; Sec=$dur }) | Out-Null

    if ($exitCode -eq 0) {
        Write-Host "  -> PASS (${dur}s)" -ForegroundColor Green
    } else {
        Write-Host "  -> FAIL (exit $exitCode, ${dur}s)" -ForegroundColor Red
    }
}

# ── 헤더 ────────────────────────────────────────────────────
Write-Section "Saba-chan Unified Test Runner"
Write-Host "Repository  : $RepoRoot" -ForegroundColor Gray
Write-Host "RelayServer : $(if ($RelayServerDir) { $RelayServerDir } else { '(not found)' })" -ForegroundColor Gray
Write-Host "Suite       : $Suite" -ForegroundColor Gray
Write-Host "NoInstall   : $NoInstall" -ForegroundColor Gray

# ── 의존성 설치 ─────────────────────────────────────────────
try {
    if ($Suite -in 'all','gui')     { Ensure-Npm (Join-Path $RepoRoot "saba-chan-gui") "GUI" }
    if ($Suite -in 'all','discord') { Ensure-Npm (Join-Path $RepoRoot "discord_bot") "Discord" }
    if ($Suite -in 'all','relay')   {
        if ($RelayServerDir) { Ensure-Npm $RelayServerDir "Relay" }
    }
} catch {
    Write-Host $_ -ForegroundColor Red; exit 1
}

# ── 테스트 실행 ─────────────────────────────────────────────
Write-Section "Running Test Suites"

# 1) Rust
if ($Suite -in 'all','rust') {
    Run-Step "Rust-Daemon"   $RepoRoot "cargo test --test daemon_integration"
    Run-Step "Rust-Updater"  $RepoRoot "cargo test --test updater_integration"
}

# 2) Relay Server E2E (requires PostgreSQL)
if ($Suite -in 'all','relay') {
    if ($RelayServerDir) {
        Run-Step "Relay-E2E" $RelayServerDir "npx vitest run"
    } else {
        Write-Host "[Relay-E2E] SKIP: relay-server directory not found" -ForegroundColor Yellow
        $results.Add([PSCustomObject]@{ Name="Relay-E2E"; Status="SKIP"; Exit=0; Sec=0 }) | Out-Null
    }
}

# 3) GUI E2E
if ($Suite -in 'all','gui') {
    Run-Step "GUI-E2E" (Join-Path $RepoRoot "saba-chan-gui") "npx vitest run"
}

# 4) Discord Bot Integration
if ($Suite -in 'all','discord') {
    Run-Step "Discord-Integration" (Join-Path $RepoRoot "discord_bot") "npm test"
}

# ── 요약 ────────────────────────────────────────────────────
Write-Section "Summary"
$results | Format-Table -AutoSize -Property Name,Status,Exit,Sec

$failed = @($results | Where-Object { $_.Status -eq "FAIL" })
if ($failed.Count -gt 0) {
    Write-Host "Failed suites:" -ForegroundColor Red
    foreach ($f in $failed) { Write-Host "  - $($f.Name) (exit $($f.Exit))" -ForegroundColor Red }
    exit 1
}

$skipped = @($results | Where-Object { $_.Status -eq "SKIP" })
if ($skipped.Count -gt 0) {
    Write-Host "Skipped: $($skipped.Count)" -ForegroundColor Yellow
}

Write-Host "All test suites passed." -ForegroundColor Green
exit 0
