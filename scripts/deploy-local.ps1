#!/usr/bin/env pwsh
<#
.SYNOPSIS
로컬 개발용 빌드 & 배포 스크립트

코어 바이너리를 빌드하여 Program Files 설치 폴더에 복사하고,
모듈/익스텐션을 %APPDATA%/saba-chan/ 아래에 배포합니다.

.DESCRIPTION
배포 대상:
  ─ Program Files\Saba-chan\
      saba-core.exe
      saba-chan-cli.exe
      saba-chan-updater.exe
      saba-chan-installer.exe
      locales/

  ─ %APPDATA%\saba-chan\modules\
      i18n.py, daemon_rcon.py        (공유 유틸리티)
      zomboid/                        (게임 모듈)
      minecraft/
      palworld/

  ─ %APPDATA%\saba-chan\extensions\
      docker/
      steamcmd/
      ue4_ini/

.PARAMETER SkipBuild
빌드를 건너뛰고 기존 바이너리만 배포합니다.

.PARAMETER Debug
디버그 프로필로 빌드합니다 (기본: release).

.PARAMETER SkipModules
모듈 배포를 건너뜁니다.

.PARAMETER SkipExtensions
익스텐션 배포를 건너뜁니다.

.PARAMETER InstallDir
코어 설치 경로 (기본: C:\Program Files\Saba-chan).

.PARAMETER ModulesRepo
saba-chan-modules 저장소 경로 (기본: ..\saba-chan-modules).

.PARAMETER ExtensionsRepo
saba-chan-extensions 저장소 경로 (기본: ..\saba-chan-extensions).

.EXAMPLE
# 전체 빌드 + 배포
.\deploy-local.ps1

# 빌드 건너뛰고 모듈만 배포
.\deploy-local.ps1 -SkipBuild -SkipExtensions

# 디버그 빌드
.\deploy-local.ps1 -Debug
#>

param(
    [switch]$SkipBuild,
    [switch]$Debug,
    [switch]$SkipModules,
    [switch]$SkipExtensions,
    [string]$InstallDir = "",
    [string]$ModulesRepo = "",
    [string]$ExtensionsRepo = ""
)

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir

# ── 경로 해석 ──────────────────────────────────────────

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:ProgramFiles "Saba-chan"
}

$AppDataDir = Join-Path $env:APPDATA "saba-chan"
$ModulesDir = Join-Path $AppDataDir "modules"
$ExtensionsDir = Join-Path $AppDataDir "extensions"

if (-not $ModulesRepo) {
    $ModulesRepo = Join-Path (Split-Path $ProjectRoot -Parent) "saba-chan-modules"
}
if (-not $ExtensionsRepo) {
    $ExtensionsRepo = Join-Path (Split-Path $ProjectRoot -Parent) "saba-chan-extensions"
}

$profile = if ($Debug) { 'debug' } else { 'release' }
$targetDir = Join-Path $ProjectRoot "target\$profile"

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

Write-Host "======================================" -ForegroundColor Cyan
Write-Host "  Saba-chan Local Deploy" -ForegroundColor Cyan
Write-Host "======================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Core  -> $InstallDir" -ForegroundColor DarkGray
Write-Host "  Modules -> $ModulesDir" -ForegroundColor DarkGray
Write-Host "  Extensions -> $ExtensionsDir" -ForegroundColor DarkGray
Write-Host "  Profile: $profile" -ForegroundColor DarkGray
Write-Host ""

# ── Helper ─────────────────────────────────────────────

function Stop-SabaProcesses {
    $names = @("saba-core", "saba-chan", "saba-chan-cli", "saba-chan-updater", "saba-chan-gui")
    $stopped = @()
    foreach ($n in $names) {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object {
            try {
                Stop-Process -Id $_.Id -Force -ErrorAction Stop
                $stopped += $n
            } catch {}
        }
    }
    if ($stopped.Count -gt 0) {
        Write-Host "  Stopped: $($stopped -join ', ')" -ForegroundColor Yellow
        Start-Sleep -Milliseconds 500
    }
}

function Copy-SafeItem {
    param([string]$Src, [string]$Dst, [switch]$Recurse)
    $dstDir = if ($Recurse) { Split-Path $Dst -Parent } else { Split-Path $Dst -Parent }
    if (-not (Test-Path $dstDir)) {
        New-Item -ItemType Directory -Path $dstDir -Force | Out-Null
    }
    if ($Recurse) {
        # 디렉터리 복사
        if (Test-Path $Dst) { Remove-Item $Dst -Recurse -Force -ErrorAction SilentlyContinue }
        Copy-Item -Path $Src -Destination $Dst -Recurse -Force
    } else {
        Copy-Item -Path $Src -Destination $Dst -Force
    }
}

function Deploy-Directory {
    <#
    .SYNOPSIS
    소스 디렉터리의 배포 대상 파일만 대상 디렉터리로 복사합니다.
    __pycache__, .pytest_cache, test_*, _sample_* 등은 제외합니다.
    #>
    param([string]$SrcDir, [string]$DstDir)
    if (-not (Test-Path $SrcDir)) {
        Write-Host "  [SKIP] Not found: $SrcDir" -ForegroundColor Yellow
        return $false
    }
    if (-not (Test-Path $DstDir)) {
        New-Item -ItemType Directory -Path $DstDir -Force | Out-Null
    }

    # 제외 패턴
    $excludeDirs = @('__pycache__', '.pytest_cache', 'node_modules', '.git', 'saba-docker-io')
    $excludeFiles = @('test_*.py', '_sample_*', 'conftest.py', '*.pyc')

    Get-ChildItem -Path $SrcDir | ForEach-Object {
        $item = $_
        if ($item.PSIsContainer) {
            if ($item.Name -in $excludeDirs) { return }
            Deploy-Directory -SrcDir $item.FullName -DstDir (Join-Path $DstDir $item.Name)
        } else {
            $skip = $false
            foreach ($pat in $excludeFiles) {
                if ($item.Name -like $pat) { $skip = $true; break }
            }
            if (-not $skip) {
                Copy-Item -Path $item.FullName -Destination (Join-Path $DstDir $item.Name) -Force
            }
        }
    }
    return $true
}

# ══════════════════════════════════════════════════════
#  1. 코어 빌드
# ══════════════════════════════════════════════════════

if (-not $SkipBuild) {
    Write-Host "[1/4] Building core ($profile)..." -ForegroundColor Yellow

    Push-Location $ProjectRoot
    try {
        Stop-SabaProcesses

        if ($Debug) {
            cargo build
        } else {
            cargo build --release
        }
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
        Write-Host "  [OK] Build complete" -ForegroundColor Green
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[1/4] Build skipped (-SkipBuild)" -ForegroundColor DarkGray
}

# ══════════════════════════════════════════════════════
#  2. 코어 바이너리 → Program Files
# ══════════════════════════════════════════════════════

Write-Host "[2/4] Deploying core to $InstallDir ..." -ForegroundColor Yellow

Stop-SabaProcesses

# 관리자 권한 확인
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator
)

if (-not (Test-Path $InstallDir)) {
    if (-not $isAdmin) {
        Write-Host "  [!] $InstallDir 생성에 관리자 권한이 필요합니다." -ForegroundColor Red
        Write-Host "      PowerShell을 관리자 권한으로 다시 실행해주세요." -ForegroundColor Red
        exit 1
    }
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

# 쓰기 권한 테스트
$testFile = Join-Path $InstallDir ".deploy-test"
try {
    [System.IO.File]::WriteAllText($testFile, "test")
    Remove-Item $testFile -Force
} catch {
    Write-Host "  [!] $InstallDir 에 쓰기 권한이 없습니다." -ForegroundColor Red
    Write-Host "      PowerShell을 관리자 권한으로 다시 실행해주세요." -ForegroundColor Red
    exit 1
}

$binaries = @(
    @{ Name = "saba-core.exe";          Src = Join-Path $targetDir "saba-core.exe" },
    @{ Name = "saba-chan-cli.exe";      Src = Join-Path $targetDir "saba-chan-cli.exe" },
    @{ Name = "saba-chan-updater.exe";  Src = Join-Path $targetDir "saba-chan-updater.exe" },
    @{ Name = "saba-chan-installer.exe"; Src = Join-Path $targetDir "saba-chan-installer.exe" }
)

$deployed = 0
foreach ($bin in $binaries) {
    if (Test-Path $bin.Src) {
        $dst = Join-Path $InstallDir $bin.Name
        Copy-Item -Path $bin.Src -Destination $dst -Force
        $mb = [Math]::Round((Get-Item $dst).Length / 1MB, 2)
        Write-Host "  $($bin.Name)  ($mb MB)" -ForegroundColor Green
        $deployed++
    } else {
        Write-Host "  [SKIP] $($bin.Name) not found" -ForegroundColor Yellow
    }
}

# locales 복사
$localesSrc = Join-Path $ProjectRoot "locales"
if (Test-Path $localesSrc) {
    $localesDst = Join-Path $InstallDir "locales"
    Copy-SafeItem -Src $localesSrc -Dst $localesDst -Recurse
    Write-Host "  locales/  (copied)" -ForegroundColor Green
}

# GUI bin 폴더에도 saba-core.exe 동기화 (개발용)
$guiBinDir = Join-Path $ProjectRoot "saba-chan-gui\bin"
$coreSrc = Join-Path $targetDir "saba-core.exe"
if ((Test-Path $coreSrc) -and (Test-Path $guiBinDir)) {
    Copy-Item -Path $coreSrc -Destination (Join-Path $guiBinDir "saba-core.exe") -Force
    Write-Host "  saba-chan-gui/bin/saba-core.exe  (synced)" -ForegroundColor DarkGreen
}

Write-Host "  [OK] $deployed binaries deployed" -ForegroundColor Green

# ══════════════════════════════════════════════════════
#  3. 모듈 → %APPDATA%/saba-chan/modules/
# ══════════════════════════════════════════════════════

if (-not $SkipModules) {
    Write-Host "[3/4] Deploying modules to $ModulesDir ..." -ForegroundColor Yellow

    if (-not (Test-Path $ModulesDir)) {
        New-Item -ItemType Directory -Path $ModulesDir -Force | Out-Null
    }

    # 3a. 공유 유틸리티 (i18n.py, daemon_rcon.py)
    $sharedUtils = @("i18n.py", "daemon_rcon.py")
    $sharedSrc = Join-Path $ProjectRoot "modules"
    foreach ($util in $sharedUtils) {
        $src = Join-Path $sharedSrc $util
        if (Test-Path $src) {
            Copy-Item -Path $src -Destination (Join-Path $ModulesDir $util) -Force
            Write-Host "  $util  (shared util)" -ForegroundColor Green
        } else {
            Write-Host "  [WARN] $util not found at $src" -ForegroundColor Yellow
        }
    }

    # 3b. 게임 모듈 (zomboid, minecraft, palworld 등)
    if (Test-Path $ModulesRepo) {
        $gameModules = Get-ChildItem -Path $ModulesRepo -Directory |
            Where-Object { $_.Name -notmatch '^(__example|\.git|\.github|\.pytest_cache|docs|__pycache__)$' }

        foreach ($mod in $gameModules) {
            $moduleToml = Join-Path $mod.FullName "module.toml"
            if (-not (Test-Path $moduleToml)) { continue }

            $dstDir = Join-Path $ModulesDir $mod.Name
            $ok = Deploy-Directory -SrcDir $mod.FullName -DstDir $dstDir
            if ($ok) {
                Write-Host "  $($mod.Name)/  (game module)" -ForegroundColor Green
            }
        }
    } else {
        Write-Host "  [WARN] Modules repo not found: $ModulesRepo" -ForegroundColor Yellow
        Write-Host "         -ModulesRepo 경로를 지정하세요" -ForegroundColor DarkGray
    }

    Write-Host "  [OK] Modules deployed" -ForegroundColor Green
} else {
    Write-Host "[3/4] Modules skipped (-SkipModules)" -ForegroundColor DarkGray
}

# ══════════════════════════════════════════════════════
#  4. 익스텐션 → %APPDATA%/saba-chan/extensions/
# ══════════════════════════════════════════════════════

if (-not $SkipExtensions) {
    Write-Host "[4/4] Deploying extensions to $ExtensionsDir ..." -ForegroundColor Yellow

    if (-not (Test-Path $ExtensionsDir)) {
        New-Item -ItemType Directory -Path $ExtensionsDir -Force | Out-Null
    }

    if (Test-Path $ExtensionsRepo) {
        $extensions = Get-ChildItem -Path $ExtensionsRepo -Directory |
            Where-Object { $_.Name -notmatch '^(\.git|\.github|\.pytest_cache|__pycache__)$' }

        foreach ($ext in $extensions) {
            $manifestJson = Join-Path $ext.FullName "manifest.json"
            if (-not (Test-Path $manifestJson)) { continue }

            $dstDir = Join-Path $ExtensionsDir $ext.Name
            $ok = Deploy-Directory -SrcDir $ext.FullName -DstDir $dstDir
            if ($ok) {
                Write-Host "  $($ext.Name)/  (extension)" -ForegroundColor Green
            }
        }
    } else {
        Write-Host "  [WARN] Extensions repo not found: $ExtensionsRepo" -ForegroundColor Yellow
        Write-Host "         -ExtensionsRepo 경로를 지정하세요" -ForegroundColor DarkGray
    }

    Write-Host "  [OK] Extensions deployed" -ForegroundColor Green
} else {
    Write-Host "[4/4] Extensions skipped (-SkipExtensions)" -ForegroundColor DarkGray
}

# ══════════════════════════════════════════════════════
#  Summary
# ══════════════════════════════════════════════════════

$stopwatch.Stop()
$elapsed = $stopwatch.Elapsed

Write-Host ""
Write-Host "======================================" -ForegroundColor Green
Write-Host "  DEPLOY COMPLETE" -ForegroundColor Green
Write-Host "======================================" -ForegroundColor Green
Write-Host "  Time: $($elapsed.Minutes)m $($elapsed.Seconds)s" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Core:       $InstallDir" -ForegroundColor White
Write-Host "  Modules:    $ModulesDir" -ForegroundColor White
Write-Host "  Extensions: $ExtensionsDir" -ForegroundColor White
Write-Host ""
