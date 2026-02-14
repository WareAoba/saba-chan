#Requires -Version 5.0
<#
.SYNOPSIS
Saba-chan Windows Release Build (Parallel)

배포물:
  core_daemon.exe            - Rust IPC 데몬
  saba-chan-cli.exe           - Rust TUI 클라이언트
  saba-chan-gui.exe           - Electron GUI
  saba-chan-updater.exe       - 업데이터 (GUI + CLI 모드)
  discord_bot/               - Node.js Discord 봇
  config/                    - global.toml, updater.toml
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
$DistDir = Join-Path $ReleaseDir "saba-chan-v0.1.0"

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
Write-Host "  Rust (daemon+cli+updater) | GUI (vite+electron) | Discord Bot" -ForegroundColor DarkGray
Write-Host ""

# 2a. Rust workspace build (all 3 binaries at once)
$jobRust = Start-Job -Name "Rust" -ScriptBlock {
    param($root)
    Set-Location $root
    $env:CARGO_TERM_COLOR = "never"

    # ErrorActionPreference=Continue so stderr warnings don't become terminating errors
    $ErrorActionPreference = "Continue"
    $output = & cargo build --release --workspace 2>&1
    $ec = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($ec -ne 0) { throw "Cargo build failed (exit $ec):`n$($output | Out-String)" }

    $targets = @(
        @{ Name = "core_daemon.exe";       Path = (Join-Path $root "target\release\core_daemon.exe") },
        @{ Name = "saba-chan-cli.exe";      Path = (Join-Path $root "target\release\saba-chan-cli.exe") },
        @{ Name = "saba-chan-updater.exe";  Path = (Join-Path $root "target\release\saba-chan-updater.exe") }
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

    & npm run build --silent 2>&1 | Out-Null
    $ec = $LASTEXITCODE
    if ($ec -ne 0) { throw "npm run build failed (exit $ec)" }

    & npm run package --silent 2>&1 | Out-Null
    $ec = $LASTEXITCODE
    if ($ec -ne 0) { throw "npm run package failed (exit $ec)" }
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

# 2c. Discord Bot (npm install --production)
$jobBot = Start-Job -Name "Bot" -ScriptBlock {
    param($root)
    $botDir = Join-Path $root "discord_bot"
    Set-Location $botDir

    if (Test-Path "node_modules") {
        Remove-Item "node_modules" -Recurse -Force -ErrorAction SilentlyContinue
    }
    $ErrorActionPreference = "Continue"
    & npm install --omit=dev --quiet --no-save 2>&1 | Out-Null
    $ec = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($ec -ne 0) { throw "npm install failed (exit $ec)" }
    return @{ Path = $botDir }
} -ArgumentList $ProjectRoot

# Wait for all jobs
$allJobs = @($jobRust, $jobGUI, $jobBot)
$failedJobs = @()
$rustResult = $null
$guiResult = $null
$botResult = $null

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
        "Bot" {
            $botResult = $result
            Write-Host "  [Bot] OK" -ForegroundColor Green
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

$botDest = Join-Path $DistDir "discord_bot"
Copy-Item -Path $botResult.Path -Destination $botDest -Recurse -Force -ErrorAction SilentlyContinue
$testDir = Join-Path $botDest "test"
if (Test-Path $testDir) { Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue }

Write-Host "  [OK]" -ForegroundColor Green

# --- 4. Resources & Configs ---
Write-Host "[4/5] Preparing resources..." -ForegroundColor Yellow

$configDir = Join-Path $DistDir "config"
New-Item -ItemType Directory -Path $configDir -Force | Out-Null
Copy-Item -Path (Join-Path $ProjectRoot "config\global.toml") -Destination $configDir -Force -ErrorAction SilentlyContinue
Copy-Item -Path (Join-Path $ProjectRoot "config\updater.toml") -Destination $configDir -Force -ErrorAction SilentlyContinue

$localeSrc = Join-Path $ProjectRoot "locales"
if (Test-Path $localeSrc) {
    Copy-Item -Path $localeSrc -Destination (Join-Path $DistDir "locales") -Recurse -Force
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
