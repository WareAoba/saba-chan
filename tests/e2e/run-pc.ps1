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
$Root = (Resolve-Path $Root).Path.TrimEnd('\')
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
# 시험 폴더의 모듈은 AppContainer로 돈다. 시험 폴더 안 JDK는 데몬이 JAVA_HOME·PATH에서 찾아 읽기를 준다 (권한을 따로 주지 않는다)

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
    # 적용 시험용 업데이터도 같은 키를 믿게 (릴리스 업데이터는 시험 키 서명을 거부한다)
    if (-not $mockExit) {
        cargo build -p saba-chan-updater --features saba-chan-updater-lib/mock-relay
        $mockExit = $LASTEXITCODE
    }
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
    Step "GUI 준비 (vite build, Tauri 빌드, tauri-driver, msedgedriver)"
    Push-Location (Join-Path $Repo "saba-chan-gui")
    try {
        if (-not (Test-Path node_modules\.bin\vite.cmd)) { npm ci; if ($LASTEXITCODE) { throw "GUI npm ci 실패" } }
        # 화면(build/)은 실행 파일에 들어가므로 Tauri 빌드보다 먼저
        npx vite build; if ($LASTEXITCODE) { throw "vite build 실패" }
    } finally { Pop-Location }
    Push-Location $Repo
    try { cargo build @profileArgs -p saba-chan-gui; if ($LASTEXITCODE) { throw "GUI 빌드 실패" } } finally { Pop-Location }
    $env:SABA_E2E_GUI_EXE = Join-Path $Repo "target\$profileDir\saba-chan-gui.exe"
    # 데몬 기동 실패 시험은 배포 빌드가 필요하다. -Release가 아니면 예전 target\release 빌드를 쓰지 않게 none(건너뜀)
    # (PowerShell은 빈 값을 넣으면 변수를 지운다)
    $env:SABA_E2E_GUI_RELEASE_EXE = if ($Release) { $env:SABA_E2E_GUI_EXE } else { "none" }
    # tauri-driver는 사용자 ~/.cargo가 아니라 시험 폴더에 설치한다 (PATH는 이 실행에만)
    $cargoTools = Join-Path $Tools "cargo"
    if (-not (Test-Path (Join-Path $cargoTools "bin\tauri-driver.exe"))) {
        cargo install tauri-driver --locked --root $cargoTools; if ($LASTEXITCODE) { throw "tauri-driver 설치 실패" }
    }
    $env:PATH = "$cargoTools\bin;$env:PATH"
    # WebView2와 같은 버전의 msedgedriver를 시험 폴더에 받는다 (PATH는 이 실행에만)
    $wv = (Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue).pv
    if (-not $wv) { $wv = (Get-ItemProperty "HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue).pv }
    if (-not $wv) {
        $wv = (Get-Item "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" -ErrorAction SilentlyContinue).VersionInfo.ProductVersion
        if ($wv) { Write-Warning "WebView2 런타임 버전을 못 찾아 Edge 버전($wv)을 씁니다. 버전이 다르면 GUI 세션이 열리지 않습니다." }
    }
    if (-not $wv) { throw "WebView2 런타임 버전을 찾지 못했습니다." }
    $edgeDir = Join-Path $Tools "msedgedriver-$wv"
    if (-not (Test-Path (Join-Path $edgeDir "msedgedriver.exe"))) {
        $zip = Join-Path $Tools "msedgedriver.zip"
        $arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "win64" }
        Invoke-WebRequest -UseBasicParsing "https://msedgedriver.microsoft.com/$wv/edgedriver_$arch.zip" -OutFile $zip
        Expand-Archive $zip -DestinationPath $edgeDir -Force
        Remove-Item $zip
    }
    $env:PATH = "$edgeDir;$env:PATH"
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
$env:SABA_E2E_MOCK_UPDATER = Join-Path $Repo "target\e2e-mock\debug\saba-chan-updater.exe"
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
