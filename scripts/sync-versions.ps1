#!/usr/bin/env pwsh
# ─────────────────────────────────────────────────────────────────
# 버전 단일 SoT 동기화 스크립트
#
# 출처: saba-chan/Cargo.toml [workspace.package].version
# 동기화 대상:
#   - saba-chan-gui/package.json  (npm 버전)
#   - 향후 추가 가능 (electron-builder appVersion 등은 package.json에서 자동 파생)
#
# 사용법:
#   .\scripts\sync-versions.ps1           — 동기화 실행
#   .\scripts\sync-versions.ps1 -Check    — 불일치만 보고 (exit 1 if mismatch)
# ─────────────────────────────────────────────────────────────────
param(
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

# ── 1. workspace 단일 버전 추출 ──────────────────────────────────
$workspaceToml = Join-Path $root 'Cargo.toml'
if (-not (Test-Path $workspaceToml)) { throw "Cargo.toml not found: $workspaceToml" }

$tomlText = Get-Content $workspaceToml -Raw -Encoding UTF8
# [workspace.package] 섹션에서 version 필드 추출
$wsMatch = [regex]::Match(
    $tomlText,
    '(?ms)^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"'
)
if (-not $wsMatch.Success) {
    throw "[workspace.package].version not found in $workspaceToml"
}
$wsVersion = $wsMatch.Groups[1].Value
Write-Host "[sync-versions] workspace.package.version = $wsVersion" -ForegroundColor Cyan

# ── 2. 동기화 대상 정의 ──────────────────────────────────────────
$targets = @(
    @{
        Path  = Join-Path $root 'saba-chan-gui\package.json'
        Type  = 'json'
        Field = 'version'
    }
)

# ── 3. 각 대상 검사/갱신 ────────────────────────────────────────
$mismatches = @()
foreach ($t in $targets) {
    if (-not (Test-Path $t.Path)) {
        Write-Warning "  skip: $($t.Path) not found"
        continue
    }

    if ($t.Type -eq 'json') {
        $raw = Get-Content $t.Path -Raw -Encoding UTF8
        $obj = $raw | ConvertFrom-Json
        $current = $obj.($t.Field)
        if ($current -eq $wsVersion) {
            Write-Host "  OK   $($t.Path) ($current)" -ForegroundColor Green
            continue
        }
        if ($Check) {
            Write-Host "  DIFF $($t.Path) — $current ≠ $wsVersion" -ForegroundColor Yellow
            $mismatches += $t.Path
            continue
        }
        # 단순 정규식 치환 (필드 순서 보존)
        $pattern = '("' + [regex]::Escape($t.Field) + '"\s*:\s*")[^"]*(")'
        $newRaw = [regex]::Replace($raw, $pattern, '${1}' + $wsVersion + '${2}', 1)
        Set-Content -Path $t.Path -Value $newRaw -NoNewline -Encoding UTF8
        Write-Host "  WROTE $($t.Path) — $current → $wsVersion" -ForegroundColor Yellow
    }
}

if ($Check -and $mismatches.Count -gt 0) {
    Write-Error "[sync-versions] $($mismatches.Count) mismatch(es) — run scripts/sync-versions.ps1 to fix"
    exit 1
}
Write-Host "[sync-versions] done." -ForegroundColor Green
