# ═══════════════════════════════════════════════════════════════
# Saba-chan 모듈 아이콘 R2 업로드 스크립트
# ═══════════════════════════════════════════════════════════════
# 동작:
#   - saba-chan-modules 레포의 각 모듈 디렉토리에서 icon.png 를 읽어
#     R2 버킷의 `assets/icons/modules/<module_id>.png` 에 업로드한다.
#   - 아이콘 URL은 버전과 무관하게 고정 경로를 사용한다.
#     (공개 베이스 URL 은 `DEFAULT_R2_PUBLIC_BASE` 상수와 일치해야 함)
#   - .release.env 의 R2 자격증명을 사용한다.
#
# 사용:
#   .\scripts\upload-module-icons.ps1
#   .\scripts\upload-module-icons.ps1 -ModulesRepo C:\Git\saba-chan-modules
# ═══════════════════════════════════════════════════════════════

param(
    [string]$ModulesRepo = "C:\Git\saba-chan-modules"
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot\..

# ── 환경변수 로드 ──
$envPath = ".release.env"
if (!(Test-Path $envPath)) { throw ".release.env 없음" }
Get-Content $envPath | Where-Object { $_ -match '^\s*[^#].*=' } | ForEach-Object {
    $k, $v = $_ -split '=', 2
    Set-Item -Path "env:$($k.Trim())" -Value $v.Trim()
}

if (-not (Test-Path $ModulesRepo)) {
    throw "모듈 레포를 찾을 수 없습니다: $ModulesRepo"
}

$publicBase = $env:R2_PUBLIC_BASE.TrimEnd('/')

Write-Host "=== Uploading module icons to R2 (bucket: $($env:R2_BUCKET)) ==="

# module.toml 이 존재하는 하위 디렉토리만 대상. 예외 디렉토리(__example 등)는 스킵.
$moduleDirs = Get-ChildItem -Path $ModulesRepo -Directory | Where-Object {
    (Test-Path (Join-Path $_.FullName "module.toml")) -and
    (-not $_.Name.StartsWith("__"))
}

if ($moduleDirs.Count -eq 0) {
    throw "모듈 디렉토리를 찾지 못했습니다: $ModulesRepo"
}

$uploaded = @()
$missing = @()

foreach ($dir in $moduleDirs) {
    $moduleId = $dir.Name
    $iconPath = Join-Path $dir.FullName "icon.png"

    if (!(Test-Path $iconPath)) {
        Write-Host "  [SKIP] $moduleId — icon.png 없음" -ForegroundColor Yellow
        $missing += $moduleId
        continue
    }

    $r2Key = "assets/icons/modules/$moduleId.png"
    Write-Host "  -> $r2Key"

    & node "$PSScriptRoot\upload-r2.mjs" $r2Key $iconPath
    if ($LASTEXITCODE -ne 0) {
        throw "R2 upload 실패: $r2Key"
    }

    $uploaded += [pscustomobject]@{
        moduleId = $moduleId
        url      = "$publicBase/$r2Key"
    }
}

Write-Host ""
Write-Host "=== 완료 ==="
Write-Host "업로드: $($uploaded.Count)"
foreach ($u in $uploaded) {
    Write-Host "  - $($u.moduleId): $($u.url)"
}
if ($missing.Count -gt 0) {
    Write-Host "아이콘 없음: $($missing -join ', ')" -ForegroundColor Yellow
}
