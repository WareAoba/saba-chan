#!/usr/bin/env pwsh
# ═══════════════════════════════════════════════════════════
# AppData Sync Validator
# ═══════════════════════════════════════════════════════════
# 소스 트리(saba-chan-modules/, saba-chan-extensions/)와
# 런타임 배포본(%APPDATA%/saba-chan/modules/, .../extensions/)의
# 파일 해시를 비교한다. drift 발견 시 exit 1.
#
# 목적:
#   "수정 후 AppData 복사를 잊어서 구버전이 실행되는" 실수를 CI로 차단.
#
# 사용법:
#   ./check-appdata-sync.ps1                   # 검증만
#   ./check-appdata-sync.ps1 -Fix              # 누락/drift 자동 복사
#   ./check-appdata-sync.ps1 -Scope modules    # modules만 검사
#   ./check-appdata-sync.ps1 -Strict           # AppData의 unknown 파일도 보고

[CmdletBinding()]
param(
    [ValidateSet('all','modules','extensions')]
    [string]$Scope = 'all',
    [switch]$Fix,
    [switch]$Strict
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = Split-Path -Parent $ScriptDir

function Resolve-Sibling([string]$name) {
    foreach ($c in @(
        (Join-Path (Split-Path -Parent $RepoRoot) $name),
        (Join-Path $RepoRoot $name)
    )) { if (Test-Path $c) { return $c } }
    return $null
}

# 추적 확장자: 데몬이 실행에 사용하는 자산.
# 빌드/캐시/IDE 아티팩트는 제외.
$TrackedExts = @('.py','.toml','.ini','.json','.properties','.md','.go','.mod')
$ExcludeDirs = @('__pycache__','.git','.github','.venv','venv','node_modules','target','dist','build','.pytest_cache','.mypy_cache','docs','__example')
$ExcludeTopLevelFiles = @('README.md','LICENSE','.gitignore','.gitattributes','.editorconfig')
# 런타임에서 생성/수정되는 파일 — 소스의 내용을 AppData에 덮어쓰면 안 됨.
$ExcludeFileNames = @('.deps-resolved.json')
# AppData에만 존재해야 하는 런타임 의존성 (saba-chan-modules 루트 직속).
# 이 파일들은 데몬이 설치하므로 소스 트리에 없어도 OK.
$AppDataOnlyAllowed = @{
    'modules'    = @('i18n.py','daemon_rcon.py','conftest.py')
    'extensions' = @('conftest.py')
}

function Get-TrackedFiles([string]$root) {
    Get-ChildItem -Path $root -Recurse -File -Force | Where-Object {
        $rel = $_.FullName.Substring($root.Length).TrimStart('\','/')
        $parts = $rel -split '[\\/]'
        foreach ($seg in $parts) {
            if ($ExcludeDirs -contains $seg) { return $false }
        }
        # 루트 직속 메타 파일 제외
        if ($parts.Count -eq 1 -and ($ExcludeTopLevelFiles -contains $parts[0])) { return $false }
        # 런타임 생성 파일 제외
        if ($ExcludeFileNames -contains $_.Name) { return $false }
        return $TrackedExts -contains $_.Extension.ToLower()
    }
}

function Get-FileHashFast([string]$path) {
    (Get-FileHash -Algorithm SHA256 -Path $path).Hash
}

function Compare-Tree {
    param(
        [string]$Label,
        [string]$SrcRoot,
        [string]$DstRoot,
        [string[]]$AllowUnknown
    )

    if (-not (Test-Path $SrcRoot)) {
        Write-Host "[$Label] SKIP: source not found ($SrcRoot)" -ForegroundColor Yellow
        return [PSCustomObject]@{ Label=$Label; Missing=@(); Drift=@(); Extra=@(); Ok=$true; Skipped=$true }
    }
    if (-not (Test-Path $DstRoot)) {
        Write-Host "[$Label] FAIL: AppData target missing ($DstRoot)" -ForegroundColor Red
        return [PSCustomObject]@{ Label=$Label; Missing=@('(entire tree)'); Drift=@(); Extra=@(); Ok=$false; Skipped=$false }
    }

    $missing = New-Object System.Collections.Generic.List[string]
    $drift   = New-Object System.Collections.Generic.List[string]
    $extra   = New-Object System.Collections.Generic.List[string]

    $srcFiles = Get-TrackedFiles $SrcRoot
    $srcRelSet = New-Object System.Collections.Generic.HashSet[string]

    foreach ($f in $srcFiles) {
        $rel = $f.FullName.Substring($SrcRoot.Length).TrimStart('\','/')
        [void]$srcRelSet.Add($rel.Replace('\','/'))

        $dstPath = Join-Path $DstRoot $rel
        if (-not (Test-Path $dstPath)) {
            $missing.Add($rel) | Out-Null
            if ($Fix) {
                $dstDir = Split-Path -Parent $dstPath
                if (-not (Test-Path $dstDir)) { New-Item -ItemType Directory -Path $dstDir -Force | Out-Null }
                Copy-Item -LiteralPath $f.FullName -Destination $dstPath -Force
            }
            continue
        }

        $srcHash = Get-FileHashFast $f.FullName
        $dstHash = Get-FileHashFast $dstPath
        if ($srcHash -ne $dstHash) {
            $drift.Add($rel) | Out-Null
            if ($Fix) {
                Copy-Item -LiteralPath $f.FullName -Destination $dstPath -Force
            }
        }
    }

    if ($Strict) {
        $dstFiles = Get-TrackedFiles $DstRoot
        foreach ($df in $dstFiles) {
            $rel = $df.FullName.Substring($DstRoot.Length).TrimStart('\','/').Replace('\','/')
            if ($srcRelSet.Contains($rel)) { continue }
            # AppData 루트 직속 허용 리스트 체크
            $parts = $rel -split '/'
            if ($parts.Count -eq 1 -and ($AllowUnknown -contains $parts[0])) { continue }
            $extra.Add($rel) | Out-Null
        }
    }

    $ok = ($missing.Count -eq 0) -and ($drift.Count -eq 0) -and ($extra.Count -eq 0)
    if ($Fix) { $ok = $ok -or (($missing.Count -eq 0) -and ($drift.Count -eq 0)) }

    $color = if ($ok) { 'Green' } else { 'Red' }
    $status = if ($ok) { 'OK' } else { 'DRIFT' }
    Write-Host ("[{0}] {1}  src={2} files, missing={3}, drift={4}, extra={5}" -f `
        $Label, $status, $srcFiles.Count, $missing.Count, $drift.Count, $extra.Count) `
        -ForegroundColor $color

    if ($missing.Count -gt 0) {
        Write-Host "  Missing in AppData:" -ForegroundColor Yellow
        foreach ($m in $missing) { Write-Host "    - $m" -ForegroundColor Yellow }
    }
    if ($drift.Count -gt 0) {
        Write-Host "  Hash drift (source newer/different):" -ForegroundColor Yellow
        foreach ($d in $drift) { Write-Host "    ~ $d" -ForegroundColor Yellow }
    }
    if ($extra.Count -gt 0) {
        Write-Host "  Extra in AppData (not in source, not whitelisted):" -ForegroundColor Magenta
        foreach ($e in $extra) { Write-Host "    + $e" -ForegroundColor Magenta }
    }

    return [PSCustomObject]@{
        Label=$Label; Missing=$missing; Drift=$drift; Extra=$extra; Ok=$ok; Skipped=$false
    }
}

$AppData = Join-Path $env:APPDATA 'saba-chan'
$reports = @()

if ($Scope -in 'all','modules') {
    $src = Resolve-Sibling 'saba-chan-modules'
    $dst = Join-Path $AppData 'modules'
    if ($src) {
        $reports += Compare-Tree -Label 'Modules' -SrcRoot $src -DstRoot $dst -AllowUnknown $AppDataOnlyAllowed['modules']
    } else {
        Write-Host "[Modules] SKIP: saba-chan-modules source not found" -ForegroundColor Yellow
    }
}

if ($Scope -in 'all','extensions') {
    $src = Resolve-Sibling 'saba-chan-extensions'
    $dst = Join-Path $AppData 'extensions'
    if ($src) {
        $reports += Compare-Tree -Label 'Extensions' -SrcRoot $src -DstRoot $dst -AllowUnknown $AppDataOnlyAllowed['extensions']
    } else {
        Write-Host "[Extensions] SKIP: saba-chan-extensions source not found" -ForegroundColor Yellow
    }
}

Write-Host ""
$bad = @($reports | Where-Object { -not $_.Ok -and -not $_.Skipped })
if ($bad.Count -gt 0) {
    Write-Host "AppData sync drift detected in $($bad.Count) tree(s)." -ForegroundColor Red
    if (-not $Fix) {
        Write-Host "Re-run with -Fix to auto-copy missing/drifted files." -ForegroundColor Yellow
    }
    exit 1
}

Write-Host "AppData is in sync with source." -ForegroundColor Green
exit 0
