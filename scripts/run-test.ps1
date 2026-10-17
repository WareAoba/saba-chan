param(
    [switch]$NoInstall,
    [switch]$Verbose,
    [ValidateSet('all','rust','relay','music','gui','mcp','python','modules','extensions','appdata-sync')]
    [string]$Suite = 'all'
)

$ErrorActionPreference = 'Stop'
$python = $env:SABA_PYTHON
if (-not $python) {
    $portable = Join-Path $env:APPDATA 'saba-chan/python-env/Scripts/python.exe'
    $python = if (Test-Path -LiteralPath $portable) { $portable } else { 'python' }
}
$runnerArgs = @('--suite', $Suite)
if ($NoInstall) { $runnerArgs += '--no-install' }
if ($Verbose) { $runnerArgs += '--verbose' }
& $python (Join-Path $PSScriptRoot 'run-tests.py') @runnerArgs
exit $LASTEXITCODE
