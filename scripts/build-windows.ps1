#Requires -Version 5.0
<#
.SYNOPSIS
Saba-chan Windows Release Build (Parallel)

배포물:
  saba-core.exe              - Rust IPC 데몬
  saba-chan-cli.exe           - Rust TUI 클라이언트
  saba-chan-gui.exe           - Electron GUI
  saba-chan-updater.exe       - 업데이터 (GUI + CLI 모드)
  saba-chan-installer.exe     - 인스톨러/언인스톨러 (Tauri)
  locales/                   - 다국어 리소스

.EXAMPLE
.\build-windows.ps1
.\build-windows.ps1 -OutputDir "my-build"
#>

param(
    [string]$OutputDir = "release-build"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$ReleaseDir = Join-Path $ProjectRoot $OutputDir

# 단일 SoT: [workspace.package].version 에서 버전 추출 (sync-versions.ps1과 동일 로직)
function Get-WorkspaceVersion {
    param([string]$RepoRoot)
    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    $content = Get-Content $cargoToml -Raw
    $inWsPkg = $false
    foreach ($line in ($content -split "`n")) {
        $stripped = $line.Trim()
        if ($stripped -eq "[workspace.package]") { $inWsPkg = $true; continue }
        if ($stripped -match '^\[' -and $stripped -ne "[workspace.package]") { $inWsPkg = $false; continue }
        if ($inWsPkg -and $stripped -match '^version\s*=\s*"([^"]+)"') { return $matches[1] }
    }
    throw "Cannot read [workspace.package].version from Cargo.toml"
}
$WorkspaceVersion = Get-WorkspaceVersion -RepoRoot $ProjectRoot
$DistDir = Join-Path $ReleaseDir "saba-chan-v$WorkspaceVersion"

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

Write-Host "================================" -ForegroundColor Cyan
Write-Host "Saba-chan Windows Release Build" -ForegroundColor Cyan
Write-Host "  (Parallel Mode)" -ForegroundColor DarkCyan
Write-Host "================================" -ForegroundColor Cyan
Write-Host ""

# --- 1. Clean ---
Write-Host "[1/5] Cleaning..." -ForegroundColor Yellow
if (Test-Path $ReleaseDir) {
    Get-ChildItem $ReleaseDir -Recurse -Filter "*.exe" -ErrorAction SilentlyContinue | ForEach-Object {
        $p = $_.FullName
        try { Remove-Item $p -Force -ErrorAction Stop } catch {
            Rename-Item $p "$p.old" -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-Item $ReleaseDir -Recurse -Force -ErrorAction SilentlyContinue
}
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
Write-Host "  [OK]" -ForegroundColor Green

# --- 2. Parallel Builds ---
Write-Host "[2/5] Starting parallel builds..." -ForegroundColor Yellow
Write-Host "  Rust (daemon+cli+updater) | GUI (vite+electron)" -ForegroundColor DarkGray
Write-Host ""

# 2a. Rust workspace build (all 3 binaries at once)
$jobRust = Start-Job -Name "Rust" -ScriptBlock {
    param($root)
    Set-Location $root
    $env:CARGO_TERM_COLOR = "never"

    function Stop-BuildLockProcesses {
        $names = @("saba-core", "saba-chan", "saba-chan-cli", "saba-chan-updater", "saba-chan-installer")
        foreach ($n in $names) {
            Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object {
                try { Stop-Process -Id $_.Id -Force -ErrorAction Stop } catch {}
            }
        }
        Start-Sleep -Milliseconds 400
    }

    # ErrorActionPreference=Continue so stderr warnings don't become terminating errors
    $ErrorActionPreference = "Continue"
    $maxAttempts = 5
    $output = $null
    $ec = 1
    # Cargo가 release 산출물을 덮어쓸 때 Windows Defender(또는 Explorer)가 잠금을
    # 잡고 있어 "failed to remove file ... os error 5/32" 형태로 실패하는 경우가 잦다.
    # saba-core / saba-chan-installer가 특히 큰 바이너리라 가장 자주 부딪힌다.
    # 근본 해결은 Defender 제외 경로 등록(`Add-MpPreference -ExclusionPath`)이며,
    # 그 외 환경에서는 잠긴 파일을 .old로 옆으로 치우고(rename은 핸들 보유 중에도
    # 보통 성공) 재시도해 우회한다.
    $lockableTargets = @(
        "target\release\saba-core.exe",
        "target\release\saba-chan-cli.exe",
        "target\release\saba-chan-updater.exe",
        "target\release\saba-chan-installer.exe"
    )

    function Clear-LockedTargets {
        param([string]$Root, [string[]]$Rels)
        foreach ($rel in $Rels) {
            $p = Join-Path $Root $rel
            if (-not (Test-Path $p)) { continue }
            try { Remove-Item $p -Force -ErrorAction Stop; continue } catch {}
            try {
                $stash = "$p.lock-$([Guid]::NewGuid().ToString('N').Substring(0,8)).old"
                Move-Item $p $stash -Force -ErrorAction Stop
            } catch { Start-Sleep -Milliseconds 1500 }
        }
    }

    # cargo 시작 전 잠긴 산출물을 미리 제거/우회 (이전 빌드의 Defender 잠금 잔재 처리)
    Clear-LockedTargets -Root $root -Rels $lockableTargets

    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        $output = & cargo build --release --workspace 2>&1
        $ec = $LASTEXITCODE
        if ($ec -eq 0) { break }

        $outText = ($output | Out-String)
        $isAccessDeniedLock = $outText -match "failed to remove file" -and `
            ($outText -match "saba-core\.exe|saba-chan-installer\.exe|saba-chan-updater\.exe|saba-chan-cli\.exe") -and `
            ($outText -match "os error (5|32)" -or $outText -match "access" -or $outText -match "蜷ｦ" -or $outText -match "・ｴ增")
        if (-not $isAccessDeniedLock -or $attempt -eq $maxAttempts) {
            break
        }

        Write-Host "[Rust] Detected locked output binary. Releasing and retrying... ($attempt/$maxAttempts)"
        Stop-BuildLockProcesses
        Clear-LockedTargets -Root $root -Rels $lockableTargets
        # Defender 스캔이 풀리는 시간을 충분히 준다 (지수 백오프 1.5s → 6s)
        $delay = [Math]::Min(1500 * [Math]::Pow(1.5, $attempt - 1), 6000)
        Start-Sleep -Milliseconds $delay
    }
    $ErrorActionPreference = "Stop"
    if ($ec -ne 0) { throw "Cargo build failed (exit $ec):`n$($output | Out-String)" }

    $targets = @(
        @{ Name = "saba-core.exe";       Path = (Join-Path $root "target\release\saba-core.exe") },
        @{ Name = "saba-chan-cli.exe";      Path = (Join-Path $root "target\release\saba-chan-cli.exe") },
        @{ Name = "saba-chan-updater.exe";  Path = (Join-Path $root "target\release\saba-chan-updater.exe") },
        @{ Name = "saba-chan-installer.exe"; Path = (Join-Path $root "target\release\saba-chan-installer.exe") }
    )
    $info = @()
    foreach ($t in $targets) {
        if (-not (Test-Path $t.Path)) { throw "$($t.Name) not found at $($t.Path)" }
        $mb = [Math]::Round((Get-Item $t.Path).Length / 1MB, 2)
        $info += "$($t.Name) ($mb MB)"
    }
    return @{ Targets = $targets; Info = $info }
} -ArgumentList $ProjectRoot

# 2b. Electron GUI (npm install + vite build + electron-builder)
$jobGUI = Start-Job -Name "GUI" -ScriptBlock {
    param($root)
    $guiDir = Join-Path $root "saba-chan-gui"
    Set-Location $guiDir

    @("dist", "build", "electron-dist") | ForEach-Object {
        if (Test-Path $_) { Remove-Item $_ -Recurse -Force -ErrorAction SilentlyContinue }
    }
    Start-Sleep -Milliseconds 300

    $ErrorActionPreference = "Continue"
    if (-not (Test-Path "node_modules")) {
        & npm install --quiet --no-save 2>&1 | Out-Null
    }
    & npm install --save-dev terser --quiet --no-save 2>&1 | Out-Null

    $buildOut = & npm run build --silent 2>&1
    $ec = $LASTEXITCODE
    if ($ec -ne 0) { throw "npm run build failed (exit $ec):`n$($buildOut | Out-String)" }

    $pkgOut = & npm run package --silent 2>&1
    $ec = $LASTEXITCODE
    if ($ec -ne 0) { throw "npm run package failed (exit $ec):`n$($pkgOut | Out-String)" }
    $ErrorActionPreference = "Stop"

    $exe = Get-ChildItem -Path "electron-dist" -Filter "Saba-chan.exe" -ErrorAction SilentlyContinue |
           Select-Object -First 1
    if (-not $exe) {
        $exe = Get-ChildItem -Path "electron-dist" -Filter "*.exe" -ErrorAction SilentlyContinue |
               Select-Object -First 1
    }
    if (-not $exe) { throw "GUI exe not found in electron-dist/" }
    $mb = [Math]::Round($exe.Length / 1MB, 2)
    return @{ Path = $exe.FullName; Info = "saba-chan-gui.exe ($mb MB)" }
} -ArgumentList $ProjectRoot

# Wait for all jobs
$allJobs = @($jobRust, $jobGUI)
$failedJobs = @()
$rustResult = $null
$guiResult = $null

foreach ($job in $allJobs) {
    $result = $null
    try {
        $result = $job | Wait-Job | Receive-Job -ErrorAction Stop
    }
    catch {
        $failedJobs += $job.Name
        Write-Host "  [$($job.Name)] FAILED: $_" -ForegroundColor Red
        Remove-Job $job -Force
        continue
    }

    switch ($job.Name) {
        "Rust" {
            $rustResult = $result
            foreach ($i in $result.Info) { Write-Host "  [Rust] $i" -ForegroundColor Green }
        }
        "GUI" {
            $guiResult = $result
            Write-Host "  [GUI] $($result.Info)" -ForegroundColor Green
        }
    }
    Remove-Job $job -Force
}

if ($failedJobs.Count -gt 0) {
    Write-Host ""
    Write-Host "BUILD FAILED: $($failedJobs -join ', ')" -ForegroundColor Red
    exit 1
}

# --- 3. Collect Artifacts ---
Write-Host ""
Write-Host "[3/5] Collecting artifacts..." -ForegroundColor Yellow

foreach ($t in $rustResult.Targets) {
    $dest = Join-Path $DistDir $t.Name
    if (Test-Path $dest) {
        try { Remove-Item $dest -Force -ErrorAction Stop } catch {
            Rename-Item $dest "$dest.old" -Force -ErrorAction SilentlyContinue
        }
    }
    Copy-Item -Path $t.Path -Destination $dest -Force
}

Copy-Item -Path $guiResult.Path -Destination (Join-Path $DistDir "saba-chan-gui.exe") -Force

Write-Host "  [OK]" -ForegroundColor Green

# --- 4. Resources ---
Write-Host "[4/5] Preparing resources..." -ForegroundColor Yellow

$localeSrc = Join-Path $ProjectRoot "locales"
if (Test-Path $localeSrc) {
    Copy-Item -Path $localeSrc -Destination (Join-Path $DistDir "locales") -Recurse -Force
}

# 공유 모듈 유틸리티 (i18n.py, daemon_rcon.py) — 인스톨러가 APPDATA/modules/로 설치
$sharedModules = @("i18n.py", "daemon_rcon.py")
$modulesSrc = Join-Path $ProjectRoot "modules"
foreach ($mod in $sharedModules) {
    $src = Join-Path $modulesSrc $mod
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination (Join-Path $DistDir $mod) -Force
    } else {
        Write-Host "  [WARN] $mod not found at $src" -ForegroundColor Yellow
    }
}

Write-Host "  [OK]" -ForegroundColor Green

# --- 5. Summary ---
$stopwatch.Stop()
$elapsed = $stopwatch.Elapsed

$totalSize = 0
Get-ChildItem -Path $DistDir -Recurse -File | ForEach-Object { $totalSize += $_.Length }
$totalMB = [Math]::Round($totalSize / 1MB, 2)

Write-Host ""
Write-Host "================================" -ForegroundColor Green
Write-Host "BUILD COMPLETE" -ForegroundColor Green
Write-Host "================================" -ForegroundColor Green
Write-Host "Output: $DistDir" -ForegroundColor Cyan
Write-Host "Time:   $($elapsed.Minutes)m $($elapsed.Seconds)s" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Contents:" -ForegroundColor Gray
Get-ChildItem -Path $DistDir | ForEach-Object {
    if ($_.PSIsContainer) {
        $dirSize = 0
        Get-ChildItem -Path $_.FullName -Recurse -File | ForEach-Object { $dirSize += $_.Length }
        $dirMB = [Math]::Round($dirSize / 1MB, 2)
        Write-Host "    $($_.Name)/  ($dirMB MB)" -ForegroundColor Gray
    }
    else {
        $fileMB = [Math]::Round($_.Length / 1MB, 2)
        Write-Host "    $($_.Name)  ($fileMB MB)" -ForegroundColor Gray
    }
}
Write-Host ""
Write-Host "  Total: $totalMB MB" -ForegroundColor White
Write-Host ""
