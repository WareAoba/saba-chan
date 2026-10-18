<#
.SYNOPSIS
  saba-chan E2E 전체를 이 PC(Windows)에서 한 번에 돌린다. 설계: tests/e2e/README.md

.DESCRIPTION
  모든 시험은 -Root(기본 C:\SabaE2E) 안에서만 돈다. 실제 %APPDATA%\saba-chan, 설치본, 기본 포트 57474는 건드리지 않고,
  끝날 때 실제 데이터 폴더가 바뀌지 않았는지 비교한다. 프로세스는 이름으로 죽이지 않는다(시험 폴더 경로가 명령줄에 있는 것만).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tests\e2e\run-pc.ps1                 # 외부망 없이 도는 전부 + Windows 전용 + GUI + relay
  powershell -ExecutionPolicy Bypass -File tests\e2e\run-pc.ps1 -Network        # + 진짜 마인크래프트 설치·실행 (바닐라·Paper·Fabric)
  powershell -ExecutionPolicy Bypass -File tests\e2e\run-pc.ps1 -Network -Steam # + Palworld·Zomboid (수 GB)
  powershell -ExecutionPolicy Bypass -File tests\e2e\run-pc.ps1 -Only "memory_limit or open_in_terminal"
#>
param(
    [string]$Root = "C:\SabaE2E",
    [switch]$Network,
    [switch]$Steam,
    [switch]$NoGui,
    [switch]$NoRelay,
    [switch]$Release,
    [switch]$Slow,
    [string]$Only = ""
)

$ErrorActionPreference = "Stop"
$E2E = $PSScriptRoot
$Repo = (Resolve-Path (Join-Path $E2E "..\..")).Path
$Git = Split-Path $Repo -Parent

function Step($msg) { Write-Host "`n== $msg" -ForegroundColor Cyan }
function Need($cmd, $hint) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { throw "$cmd 이(가) 없습니다. $hint" }
}

New-Item -ItemType Directory -Force -Path $Root | Out-Null
$Tools = Join-Path $Root "tools"
New-Item -ItemType Directory -Force -Path $Tools | Out-Null

Step "준비물 확인"
Need cargo "Rust(rustup)를 설치하세요."
Need node "Node 22.12 이상을 설치하세요."
# WindowsApps의 python.exe는 스토어 설치 안내 스텁이라 venv를 만들지 못한다
$py = Get-Command python -All -ErrorAction SilentlyContinue | Where-Object { $_.Source -notlike "*\WindowsApps\*" } | Select-Object -First 1
if (-not $py) { $py = Get-Command py -ErrorAction SilentlyContinue }
if (-not $py) { throw "Python 3.10 이상이 필요합니다 (WindowsApps 스텁 말고 실제 설치본)." }
foreach ($r in "saba-chan-modules", "saba-chan-extensions") {
    if (-not (Test-Path (Join-Path $Git $r))) { throw "$Git\$r 이 없습니다. saba-chan 체크아웃 옆에 같은 이름으로 두세요 (worktree도 됩니다)." }
}

# 가짜 마인크래프트 서버를 빌드할 JDK. 없으면 시험 폴더 안에 Temurin 21을 받는다 (PATH는 이 실행에만)
if (-not (Get-Command javac -ErrorAction SilentlyContinue)) {
    $jdk = Get-ChildItem -Directory (Join-Path $Tools "jdk*") -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $jdk) {
        Step "JDK가 없어 시험 폴더에 Temurin 21 받는 중"
        $zip = Join-Path $Tools "jdk.zip"
        Invoke-WebRequest -UseBasicParsing "https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jdk/hotspot/normal/eclipse?project=jdk" -OutFile $zip
        Expand-Archive $zip -DestinationPath $Tools -Force
        Remove-Item $zip
        $jdk = Get-ChildItem -Directory (Join-Path $Tools "jdk*") | Select-Object -First 1
    }
    $env:PATH = "$($jdk.FullName)\bin;$env:PATH"
    $env:JAVA_HOME = $jdk.FullName
}
# 시험 폴더의 모듈은 확인되지 않은 구성요소라 AppContainer로 돈다. Program Files의 JDK처럼 앱 컨테이너가 읽을 수 있게
# 시험 폴더 안 JDK에 ALL APPLICATION PACKAGES 읽기·실행을 준다
if ($env:JAVA_HOME -and $env:JAVA_HOME.StartsWith($Root, [StringComparison]::OrdinalIgnoreCase)) {
    icacls $env:JAVA_HOME /grant "*S-1-15-2-1:(OI)(CI)RX" /T /Q | Out-Null
    if ($LASTEXITCODE) { throw "JDK 폴더 권한 설정 실패" }
}

Step "바이너리 빌드"
Push-Location $Repo
try {
    $profileArgs = @()
    $profileDir = "debug"
    if ($Release) { $profileArgs = @("--release"); $profileDir = "release" }
    cargo build @profileArgs -p saba-core -p saba-chan-cli -p saba-runner
    if ($LASTEXITCODE) { throw "cargo build 실패" }
    # 업데이트 시험용: 공개 시험 키를 믿는 빌드 (따로 둔다)
    $env:CARGO_TARGET_DIR = Join-Path $Repo "target\e2e-mock"
    cargo build -p saba-core --features mock-relay
    $mockExit = $LASTEXITCODE
    Remove-Item Env:CARGO_TARGET_DIR
    if ($mockExit) { throw "mock-relay 빌드 실패" }
} finally { Pop-Location }

$markers = @()
if (-not $Network) { $markers += "not network" }
if (-not $Steam) { $markers += "not steam" }
if ($NoGui) { $markers += "not gui" }
if ($NoRelay) { $markers += "not relay_real" }
if (-not $Slow -and -not $Network) { $markers += "not slow" }

if (-not $NoGui) {
    Step "GUI 준비 (vite build, playwright-core)"
    Push-Location (Join-Path $Repo "saba-chan-gui")
    try {
        if (-not (Test-Path node_modules\electron\path.txt)) { npm ci; if ($LASTEXITCODE) { throw "GUI npm ci 실패" } }
        npx vite build; if ($LASTEXITCODE) { throw "vite build 실패" }
    } finally { Pop-Location }
    Push-Location (Join-Path $E2E "gui")
    try { npm install --no-audit --no-fund; if ($LASTEXITCODE) { throw "gui 하네스 npm install 실패" } } finally { Pop-Location }
}

$relay = Join-Path $Git "server-chan\relay-server"
if (-not $NoRelay -and (Test-Path $relay)) {
    Step "relay-server 준비"
    Push-Location $relay
    try { if (-not (Test-Path node_modules\.bin)) { npm ci; if ($LASTEXITCODE) { throw "relay npm ci 실패" } } } finally { Pop-Location }
}

Step "Python 가상환경"
$venv = Join-Path $Root "venv"
if (-not (Test-Path "$venv\Scripts\python.exe")) { & $py.Source -m venv $venv }
& "$venv\Scripts\python.exe" -m pip install -q -r (Join-Path $E2E "requirements.txt")
if ($LASTEXITCODE) { throw "pip install 실패" }

$env:SABA_E2E_ROOT = $Root
$env:SABA_E2E_BIN_DIR = Join-Path $Repo "target\$profileDir"
$env:SABA_E2E_MOCK_CORE = Join-Path $Repo "target\e2e-mock\debug\saba-core.exe"
$env:SABA_E2E_MODULES_REPO = Join-Path $Git "saba-chan-modules"
$env:SABA_E2E_EXTENSIONS_REPO = Join-Path $Git "saba-chan-extensions"
$env:SABA_E2E_SERVER_CHAN_REPO = Join-Path $Git "server-chan"

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$junit = Join-Path $Root "junit-$stamp.xml"
$report = Join-Path $Root "report-$stamp.md"
$pyArgs = @("-m", "pytest", "-p", "no:cacheprovider", "--junitxml", $junit)
if ($markers.Count) { $pyArgs += @("-m", ($markers -join " and ")) }
if ($Only) { $pyArgs += @("-k", $Only) }

Step "시험 실행: $($markers -join ' and ') $Only"
Push-Location $E2E
try {
    & "$venv\Scripts\python.exe" @pyArgs
    $code = $LASTEXITCODE
    & "$venv\Scripts\python.exe" -m harness.report $junit $report "saba-chan E2E (PC $stamp)"
} finally { Pop-Location }

Write-Host "`n결과: $report" -ForegroundColor Green
Write-Host "실행 폴더(로그·스크린샷·콘솔): $Root\run-*"
exit $code
