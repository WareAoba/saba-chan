#Requires -Version 5.0
<#
.SYNOPSIS
Saba-chan Windows Release Build (Parallel)

배포물:
  saba-core.exe              - Rust IPC 데몬
  saba-chan-cli.exe           - Rust TUI 클라이언트
  saba-chan-gui.exe           - GUI (Tauri)
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
# 릴리스 버전은 versions.toml [release] version 하나에서 읽는다
$ReleaseVersion = (Select-String -Path (Join-Path $ProjectRoot "versions.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$DistDir = Join-Path $ReleaseDir "saba-chan-v$ReleaseVersion"

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

# --- 2. Builds ---
Write-Host "[2/5] Building..." -ForegroundColor Yellow
Write-Host "  GUI frontend (vite) -> Rust workspace (daemon+cli+updater+installer+gui)" -ForegroundColor DarkGray
Write-Host ""

# 2a. GUI 화면 빌드: Tauri GUI가 build/를 exe에 넣으므로 cargo보다 먼저 끝나야 한다
$guiDir = Join-Path $ProjectRoot "saba-chan-gui"
Push-Location $guiDir
try {
    if (Test-Path "build") { Remove-Item "build" -Recurse -Force -ErrorAction SilentlyContinue }
    $ErrorActionPreference = "Continue"
    if (-not (Test-Path "node_modules")) { & npm ci --quiet 2>&1 | Out-Null }
    $buildOut = & npm run build --silent 2>&1
    $ec = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($ec -ne 0) { throw "npm run build failed (exit $ec):`n$($buildOut | Out-String)" }
    Write-Host "  [GUI] build/ ready" -ForegroundColor Green
}
finally { Pop-Location }

# 2b. Rust workspace build (모든 바이너리를 한 번에)
$jobRust = Start-Job -Name "Rust" -ScriptBlock {
    param($root)
    Set-Location $root
    $env:CARGO_TERM_COLOR = "never"

    function Stop-BuildLockProcesses {
        # 이 저장소의 target\ 에서 뜬 것만 끝낸다 (설치된 사바쨩 데몬·서버는 그대로)
        $names = @("saba-core", "saba-runner", "saba-chan", "saba-chan-cli", "saba-chan-updater", "saba-chan-installer")
        $targetDir = (Join-Path $root "target") + "\"
        foreach ($n in $names) {
            Get-Process -Name $n -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and $_.Path.StartsWith($targetDir, [System.StringComparison]::OrdinalIgnoreCase) } |
                ForEach-Object { try { Stop-Process -Id $_.Id -Force -ErrorAction Stop } catch {} }
        }
        Start-Sleep -Milliseconds 400
    }

    # ErrorActionPreference=Continue so stderr warnings don't become terminating errors
    $ErrorActionPreference = "Continue"
    $maxAttempts = 3
    $output = $null
    $ec = 1
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        $output = & cargo build --release --workspace 2>&1
        $ec = $LASTEXITCODE
        if ($ec -eq 0) { break }

        $outText = ($output | Out-String)
        $isAccessDeniedLock = $outText -match "failed to remove file" -and $outText -match "saba-core\.exe" -and ($outText -match "os error 5" -or $outText -match "拠蜷ｦ" -or $outText -match "access")
        if (-not $isAccessDeniedLock -or $attempt -eq $maxAttempts) {
            break
        }

        Write-Host "[Rust] Detected locked saba-core.exe. Stopping related processes and retrying... ($attempt/$maxAttempts)"
        Stop-BuildLockProcesses
        try {
            $lockedExe = Join-Path $root "target\release\saba-core.exe"
            if (Test-Path $lockedExe) {
                Remove-Item $lockedExe -Force -ErrorAction SilentlyContinue
            }
        } catch {}
        Start-Sleep -Milliseconds 600
    }
    $ErrorActionPreference = "Stop"
    if ($ec -ne 0) { throw "Cargo build failed (exit $ec):`n$($output | Out-String)" }

    $targets = @(
        @{ Name = "saba-core.exe";       Path = (Join-Path $root "target\release\saba-core.exe") },
        @{ Name = "saba-runner.exe";     Path = (Join-Path $root "target\release\saba-runner.exe") },
        @{ Name = "saba-chan-cli.exe";      Path = (Join-Path $root "target\release\saba-chan-cli.exe") },
        @{ Name = "saba-chan-updater.exe";  Path = (Join-Path $root "target\release\saba-chan-updater.exe") },
        @{ Name = "saba-chan-installer.exe"; Path = (Join-Path $root "target\release\saba-chan-installer.exe") },
        @{ Name = "saba-chan-gui.exe";      Path = (Join-Path $root "target\release\saba-chan-gui.exe") }
    )
    $info = @()
    foreach ($t in $targets) {
        if (-not (Test-Path $t.Path)) { throw "$($t.Name) not found at $($t.Path)" }
        $mb = [Math]::Round((Get-Item $t.Path).Length / 1MB, 2)
        $info += "$($t.Name) ($mb MB)"
    }
    return @{ Targets = $targets; Info = $info }
} -ArgumentList $ProjectRoot

# Wait for all jobs
$allJobs = @($jobRust)
$failedJobs = @()
$rustResult = $null

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
