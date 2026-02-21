<#
.SYNOPSIS
Build saba-docker-io for Linux (amd64)

Go cross-compilation: GOOS=linux GOARCH=amd64 go build
No extra toolchain needed — Go handles it natively.

.EXAMPLE
.\build-docker-io.ps1
#>

$ErrorActionPreference = "Stop"
$ProjectDir = Join-Path (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)) "extensions\docker\saba-docker-io"

if (-not (Test-Path $ProjectDir)) {
    Write-Host "ERROR: Project not found at $ProjectDir" -ForegroundColor Red
    exit 1
}

# Check Go is available
$goExe = Get-Command go -ErrorAction SilentlyContinue
if (-not $goExe) {
    Write-Host "ERROR: Go not found. Install from https://go.dev/dl/" -ForegroundColor Red
    exit 1
}

$OutputBinary = Join-Path $ProjectDir "saba-docker-io"

Write-Host "Building saba-docker-io (linux/amd64)..." -ForegroundColor Cyan
Write-Host "  Project: $ProjectDir" -ForegroundColor Gray

$env:GOOS = "linux"
$env:GOARCH = "amd64"
$env:CGO_ENABLED = "0"

Push-Location $ProjectDir
try {
    & go build -ldflags="-s -w" -o $OutputBinary . 2>&1 | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
    if ($LASTEXITCODE -ne 0) { throw "Go build failed" }
} finally {
    # Restore env
    Remove-Item Env:\GOOS -ErrorAction SilentlyContinue
    Remove-Item Env:\GOARCH -ErrorAction SilentlyContinue
    Remove-Item Env:\CGO_ENABLED -ErrorAction SilentlyContinue
    Pop-Location
}

if (-not (Test-Path $OutputBinary)) {
    throw "Binary not found at $OutputBinary"
}

$Size = [Math]::Round((Get-Item $OutputBinary).Length / 1MB, 2)
Write-Host ""
Write-Host "BUILD OK: $OutputBinary ($Size MB)" -ForegroundColor Green
Write-Host "  Auto-deployed by docker_engine.py ensure()." -ForegroundColor Gray
