#!/usr/bin/env pwsh
# Unified test runner for saba-chan (Windows/PowerShell)

param(
    [switch]$NoInstall,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = if ((Split-Path -Leaf $ScriptDir) -ieq "scripts") {
    Split-Path -Parent $ScriptDir
} else {
    $ScriptDir
}
Set-Location $RepoRoot

$results = New-Object System.Collections.Generic.List[object]

function Write-Section($title) {
    Write-Host ""
    Write-Host "==================================================" -ForegroundColor Cyan
    Write-Host " $title" -ForegroundColor Cyan
    Write-Host "==================================================" -ForegroundColor Cyan
}

function Ensure-NpmDependencies($dir, $label) {
    if ($NoInstall) {
        return
    }

    if (-not (Test-Path (Join-Path $dir "node_modules"))) {
        Write-Host "[$label] node_modules not found. Installing dependencies..." -ForegroundColor Yellow
        Push-Location $dir
        npm install --silent
        $code = $LASTEXITCODE
        Pop-Location

        if ($code -ne 0) {
            throw "[$label] npm install failed (exit $code)"
        }
    }
}

function Run-Step($name, $workingDir, $command) {
    Write-Host "[$name] $command" -ForegroundColor Yellow

    $start = Get-Date
    Push-Location $workingDir

    if ($Verbose) {
        Invoke-Expression $command
    } else {
        Invoke-Expression $command | Out-Host
    }

    $exitCode = $LASTEXITCODE
    Pop-Location

    $duration = (Get-Date) - $start
    $status = if ($exitCode -eq 0) { "PASS" } else { "FAIL" }

    $results.Add([PSCustomObject]@{
        Name = $name
        Status = $status
        ExitCode = $exitCode
        DurationSec = [math]::Round($duration.TotalSeconds, 2)
    }) | Out-Null

    if ($exitCode -eq 0) {
        Write-Host "[$name] PASS (${duration.TotalSeconds}s)" -ForegroundColor Green
    } else {
        Write-Host "[$name] FAIL (exit $exitCode, ${duration.TotalSeconds}s)" -ForegroundColor Red
    }
}

Write-Section "Saba-chan Unified Test Runner"
Write-Host "Repository: $RepoRoot" -ForegroundColor Gray
Write-Host "NoInstall : $NoInstall" -ForegroundColor Gray
Write-Host "Verbose   : $Verbose" -ForegroundColor Gray

try {
    Ensure-NpmDependencies (Join-Path $RepoRoot "saba-chan-gui") "GUI"
    Ensure-NpmDependencies (Join-Path $RepoRoot "discord_bot") "Discord"
} catch {
    Write-Host $_ -ForegroundColor Red
    exit 1
}

Write-Section "Running Test Suites"

Run-Step "Rust-Daemon-Integration" $RepoRoot "cargo test --test daemon_integration"
Run-Step "Rust-Updater-Integration" $RepoRoot "cargo test --test updater_integration"
Run-Step "GUI-Vitest" (Join-Path $RepoRoot "saba-chan-gui") "npm test -- --run"
Run-Step "Discord-Jest" (Join-Path $RepoRoot "discord_bot") "npm test"

Write-Section "Summary"
$results | Format-Table -AutoSize

$failed = @($results | Where-Object { $_.Status -eq "FAIL" })
if ($failed.Count -gt 0) {
    Write-Host "Failed suites:" -ForegroundColor Red
    foreach ($item in $failed) {
        Write-Host " - $($item.Name) (exit $($item.ExitCode))" -ForegroundColor Red
    }
    exit 1
}

Write-Host "All test suites passed." -ForegroundColor Green
exit 0
