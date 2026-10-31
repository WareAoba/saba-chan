<#
.SYNOPSIS
    PR 프리뷰: GitHub Actions(preview.yml)가 빌드한 PR 실행 파일을 받아 설치본과 분리해 띄운다.

.DESCRIPTION
    PR마다 <Root>\pr-<번호>\ 에 app\(실행 파일)과 data\(데이터 폴더)를 둔다.
    SABA_DATA_DIR·IPC_BASE·SABA_IPC_PORT로 데이터 폴더와 포트를 갈라서, 설치된 사바쨩과
    %APPDATA%\saba-chan\config.json·인스턴스·노드 토큰을 건드리지 않는다.
    처음 만들 때 설치본의 모듈·익스텐션 폴더만 복사한다(인스턴스·토큰은 복사하지 않아 릴레이에 붙지 않는다).
    gh CLI 로그인이 필요하다(gh auth login).

.EXAMPLE
    scripts\preview-pr.ps1              # preview 라벨이 붙은 열린 PR에서 고른다
    scripts\preview-pr.ps1 -Pr 123      # 그 PR을 바로 띄운다
    scripts\preview-pr.ps1 -Pr 123 -Cli # 프리뷰 데몬에 붙는 CLI 창도 연다
    scripts\preview-pr.ps1 -Stop        # 떠 있는 프리뷰를 끈다
    scripts\preview-pr.ps1 -Clean       # 프리뷰를 끄고 프리뷰 폴더를 모두 지운다
    scripts\preview-pr.ps1 -Shortcut    # 바탕화면에 "사바쨩 PR 프리뷰" 바로가기를 만든다
#>
param(
    [int]$Pr,
    [switch]$Cli,
    [switch]$Stop,
    [switch]$Clean,
    [switch]$Shortcut,
    [string]$Root,
    [string]$Repo = 'WareAoba/saba-chan',
    [int]$Port = 57575
)

$ErrorActionPreference = 'Stop'
# gh가 내는 JSON(UTF-8)의 한국어 PR 제목이 깨지지 않게
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# 큰 파일은 D: 드라이브에 둔다 (없으면 로컬 앱 데이터)
if (-not $Root) {
    $Root = if (Test-Path 'D:\') { 'D:\SabaPreview' } else { Join-Path $env:LOCALAPPDATA 'SabaPreview' }
}
$Interactive = -not $PSBoundParameters.ContainsKey('Pr')

function Fail([string]$Message) {
    Write-Host $Message -ForegroundColor Red
    if ($Interactive) { [void](Read-Host '엔터를 누르면 닫습니다') }
    exit 1
}

function Write-Utf8([string]$Path, [string]$Text) {
    # Windows PowerShell 5.1의 -Encoding UTF8은 BOM을 붙이고, 데몬의 JSON 읽기는 BOM을 받지 않는다
    [System.IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding $false))
}

# 프리뷰 폴더 안의 exe로 뜬 프로세스만 고른다 (이름으로 고르지 않는다: 설치본을 건드리지 않게)
function Get-PreviewProcess([string]$Under) {
    $prefix = $Under.TrimEnd('\') + '\'
    Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $p = $null
        try { $p = $_.Path } catch {}
        $p -and $p.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
    }
}

function Stop-Preview([string]$Under) {
    $procs = @(Get-PreviewProcess $Under)
    if ($procs.Count -eq 0) { return $false }
    # GUI가 데몬을 다시 띄우지 않게 GUI부터 끈다
    foreach ($name in @('saba-chan-gui', 'saba-chan-cli', 'saba-core', 'saba-runner')) {
        $procs | Where-Object { $_.ProcessName -eq $name } | ForEach-Object {
            try { Stop-Process -Id $_.Id -Force -ErrorAction Stop } catch {}
        }
    }
    Start-Sleep -Milliseconds 800
    return $true
}

function Test-PortFree([int]$P) {
    $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, $P)
    try { $listener.Start(); return $true } catch { return $false } finally { try { $listener.Stop() } catch {} }
}

function Invoke-GhJson([string[]]$GhArgs) {
    $out = & gh @GhArgs
    if ($LASTEXITCODE -ne 0) { Fail "gh $($GhArgs[0]) $($GhArgs[1]) 실패 (종료 코드 $LASTEXITCODE)" }
    # Windows PowerShell 5.1의 ConvertFrom-Json은 JSON 배열을 Object[] 하나로 내보낸다.
    # 변수에 받은 뒤 돌려주면 원소로 펼쳐져서, 부르는 쪽의 @(...)가 7과 같게 원소 목록이 된다
    $parsed = ($out -join "`n") | ConvertFrom-Json
    return $parsed
}

# ── 바로가기·끄기·정리 ──

if ($Shortcut) {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $lnk = Join-Path $desktop '사바쨩 PR 프리뷰.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $s = $shell.CreateShortcut($lnk)
    $s.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $s.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    $s.WorkingDirectory = Split-Path $PSCommandPath -Parent
    $s.Save()
    Write-Host "바로가기를 만들었습니다: $lnk" -ForegroundColor Green
    exit 0
}

if ($Stop -or $Clean) {
    if (Test-Path $Root) {
        if (Stop-Preview $Root) { Write-Host '떠 있던 프리뷰를 껐습니다.' -ForegroundColor Green }
        else { Write-Host '떠 있는 프리뷰가 없습니다.' }
    }
    if ($Clean -and (Test-Path $Root)) {
        Get-ChildItem -LiteralPath $Root -Directory -Filter 'pr-*' | ForEach-Object {
            try { Remove-Item -LiteralPath $_.FullName -Recurse -Force; Write-Host "지웠습니다: $($_.FullName)" }
            catch { Write-Host "지우지 못했습니다: $($_.FullName) ($($_.Exception.Message))" -ForegroundColor Yellow }
        }
    }
    exit 0
}

# ── PR 고르기 ──

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Fail 'gh(GitHub CLI)가 없습니다. winget install GitHub.cli 로 설치한 뒤 gh auth login 을 한 번 해 주세요.'
}
& gh auth status *> $null
if ($LASTEXITCODE -ne 0) { Fail 'gh 로그인이 필요합니다. gh auth login 을 한 번 해 주세요.' }

if ($Interactive) {
    $prs = @(Invoke-GhJson @('pr', 'list', '--repo', $Repo, '--label', 'preview', '--state', 'open', '--json', 'number,title'))
    if ($prs.Count -eq 0) { Fail 'preview 라벨이 붙은 열린 PR이 없습니다.' }
    Write-Host ''
    Write-Host '프리뷰할 수 있는 PR' -ForegroundColor Cyan
    foreach ($p in $prs) { Write-Host ("  #{0}  {1}" -f $p.number, $p.title) }
    Write-Host ''
    $answer = Read-Host "PR 번호 (엔터 = #$($prs[0].number))"
    $Pr = if ($answer.Trim()) { [int]($answer.Trim().TrimStart('#')) } else { [int]$prs[0].number }
}

$info = Invoke-GhJson @('pr', 'view', "$Pr", '--repo', $Repo, '--json', 'number,title,headRefName,headRefOid,labels')
$hasLabel = @($info.labels | Where-Object { $_.name -eq 'preview' }).Count -gt 0

# ── 빌드 찾기 ──

# 실행 결과가 아니라 받을 수 있는 아티팩트로 고른다: 다른 라벨을 붙여 빌드를 건너뛴 실행도 success로 남는다
$found = Invoke-GhJson @('api', "repos/$Repo/actions/artifacts?name=preview-pr-$Pr&per_page=50")
$builds = @($found.artifacts | Where-Object { -not $_.expired -and $_.workflow_run } |
        Sort-Object -Property created_at -Descending |
        ForEach-Object { [pscustomobject]@{ databaseId = $_.workflow_run.id; headSha = $_.workflow_run.head_sha } })
$run = $builds | Where-Object { $_.headSha -eq $info.headRefOid } | Select-Object -First 1
if (-not $run) {
    # 왜 최신 커밋 빌드가 없는지 실행 목록으로 알려 준다
    $runs = @(Invoke-GhJson @('run', 'list', '--repo', $Repo, '--workflow', 'preview.yml', '--branch', $info.headRefName,
            '--limit', '20', '--json', 'headSha,status,conclusion,url'))
    $current = @($runs | Where-Object { $_.headSha -eq $info.headRefOid })
    $pending = $current | Where-Object { $_.status -ne 'completed' } | Select-Object -First 1
    $failed = $current | Where-Object { $_.conclusion -eq 'failure' } | Select-Object -First 1
    $older = $builds | Select-Object -First 1
    if ($pending) { $why = "최신 커밋은 아직 빌드 중입니다: $($pending.url)" }
    elseif ($failed) { $why = "최신 커밋 빌드가 실패했습니다: $($failed.url)" }
    elseif (-not $hasLabel) { $why = "PR #$Pr 에 preview 라벨이 없습니다. 라벨을 붙이면 빌드가 시작됩니다." }
    else { $why = '최신 커밋의 빌드가 없습니다(3일이 지나 지워졌을 수 있습니다). 커밋을 더하거나 라벨을 다시 붙이면 새로 빌드합니다.' }
    if (-not $older) { Fail $why }
    Write-Host "$why 이전 커밋 빌드($($older.headSha.Substring(0, 7)))로 띄웁니다." -ForegroundColor Yellow
    $run = $older
}

# ── 내려받기 ──

$box = Join-Path $Root "pr-$Pr"
$app = Join-Path $box 'app'
$data = Join-Path $box 'data'
$stamp = Join-Path $app 'preview.json'

$haveSha = $null
if (Test-Path $stamp) { try { $haveSha = (Get-Content -Raw -LiteralPath $stamp | ConvertFrom-Json).sha } catch {} }
if ($haveSha -ne $run.headSha) {
    # 새 폴더에 다 받은 뒤에 바꾼다: 받기가 실패해도 지금 프리뷰는 남는다
    $incoming = Join-Path $box 'app-new'
    if (Test-Path $incoming) { Remove-Item -LiteralPath $incoming -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $incoming | Out-Null
    Write-Host "PR #$Pr 빌드($($run.headSha.Substring(0, 7)))를 받는 중..." -ForegroundColor Cyan
    & gh run download "$($run.databaseId)" --repo $Repo --name "preview-pr-$Pr" --dir $incoming
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $incoming -Recurse -Force -ErrorAction SilentlyContinue
        Fail '빌드를 받지 못했습니다. 아티팩트는 3일 뒤 지워집니다. PR에 커밋을 더하거나 라벨을 다시 붙이면 새로 빌드합니다.'
    }
    # 실행 중인 exe는 덮어쓸 수 없다
    [void](Stop-Preview $box)
    if (Test-Path $app) { Remove-Item -LiteralPath $app -Recurse -Force }
    Rename-Item -LiteralPath $incoming -NewName 'app'
}

# ── 데이터 폴더 (처음 한 번) ──

$installedData = Join-Path $env:APPDATA 'saba-chan'
if (-not (Test-Path $data)) {
    New-Item -ItemType Directory -Force -Path $data | Out-Null
    $installed = $null
    $installedConfig = Join-Path $installedData 'config.json'
    if (Test-Path $installedConfig) { try { $installed = Get-Content -Raw -Encoding UTF8 -LiteralPath $installedConfig | ConvertFrom-Json } catch {} }
    $lang = 'ko'
    if ($installed -and $installed.gui -and $installed.gui.language) { $lang = [string]$installed.gui.language }
    $config = @{ gui = @{ language = $lang } } | ConvertTo-Json -Depth 5
    Write-Utf8 (Join-Path $data 'config.json') $config

    # 설치본의 모듈·익스텐션만 복사한다 (paths 칸으로 옮겼으면 그 폴더)
    foreach ($kind in @('modules', 'extensions')) {
        $src = Join-Path $installedData $kind
        if ($installed -and $installed.paths -and $installed.paths.$kind) { $src = [string]$installed.paths.$kind }
        if (Test-Path $src) {
            & robocopy $src (Join-Path $data $kind) /E /XD __pycache__ /NFL /NDL /NJH /NJS /NP | Out-Null
            if ($LASTEXITCODE -ge 8) { Write-Host "$kind 복사가 일부 실패했습니다 (robocopy $LASTEXITCODE)." -ForegroundColor Yellow }
        }
    }
}
# PR의 공유 모듈 파일(i18n.py 등)은 띄울 때마다 덮는다
$shared = Join-Path $app 'shared-modules'
if (Test-Path $shared) {
    $modulesDir = Join-Path $data 'modules'
    New-Item -ItemType Directory -Force -Path $modulesDir | Out-Null
    Copy-Item -Path (Join-Path $shared '*') -Destination $modulesDir -Force
}

# ── 띄우기 ──

$running = @(Get-PreviewProcess $box)
if ($running.Count -eq 0) {
    $others = @(Get-PreviewProcess $Root)
    if ($others.Count -gt 0) { Fail "다른 PR 프리뷰가 떠 있습니다. scripts\preview-pr.ps1 -Stop 으로 끈 뒤 다시 실행해 주세요." }
    if (-not (Test-PortFree $Port)) { Fail "포트 $Port 를 다른 프로그램이 쓰고 있습니다. -Port 로 다른 포트를 주세요." }
}

# 띄우기 전에 프리뷰 설정을 맞춘다: IPC 포트를 설치본과 가르고, 원격 데몬 리스너(기본 57480)는 끈다.
# 원격 데몬은 기본으로 켜져 있어 설치본과 같은 포트를 잡으려 한다
if ($running.Count -eq 0) {
    $cfgPath = Join-Path $data 'config.json'
    $cfg = $null
    if (Test-Path $cfgPath) { try { $cfg = Get-Content -Raw -Encoding UTF8 -LiteralPath $cfgPath | ConvertFrom-Json } catch {} }
    if (-not $cfg) { $cfg = New-Object PSObject }
    foreach ($sec in @('gui', 'remote')) {
        if (-not $cfg.PSObject.Properties[$sec]) { $cfg | Add-Member -NotePropertyName $sec -NotePropertyValue (New-Object PSObject) }
    }
    $cfg.gui | Add-Member -NotePropertyName ipcPort -NotePropertyValue $Port -Force
    $cfg.remote | Add-Member -NotePropertyName enabled -NotePropertyValue $false -Force
    Write-Utf8 $cfgPath ($cfg | ConvertTo-Json -Depth 20)
}

# 자식 프로세스에만 물려주고 되돌린다: 이 창에 남으면 뒤에 실행한 설치본 CLI가 프리뷰 데몬·데이터를 가리킨다
$previewEnv = @{ SABA_DATA_DIR = $data; SABA_IPC_PORT = "$Port"; IPC_BASE = "http://127.0.0.1:$Port" }
$savedEnv = @{}
foreach ($k in $previewEnv.Keys) { $savedEnv[$k] = [Environment]::GetEnvironmentVariable($k, 'Process') }
try {
    foreach ($k in $previewEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $previewEnv[$k], 'Process') }
    # 이미 떠 있으면 GUI가 단일 실행 잠금으로 그 창을 앞으로 가져온다
    Start-Process -FilePath (Join-Path $app 'saba-chan-gui.exe') -WorkingDirectory $app
    if ($Cli) {
        $cmd = "Set-Location -LiteralPath '$app'; Write-Host 'PR #$Pr 프리뷰 CLI (데몬 포트 $Port). 예: .\saba-chan-cli.exe --help' -ForegroundColor Cyan"
        Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoExit', '-NoProfile', '-Command', $cmd) -WorkingDirectory $app
    }
} finally {
    foreach ($k in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $savedEnv[$k], 'Process') }
}

Write-Host ''
Write-Host ("PR #{0} 프리뷰를 띄웠습니다: {1} ({2})" -f $Pr, $info.title, $run.headSha.Substring(0, 7)) -ForegroundColor Green
Write-Host "  실행 파일: $app"
Write-Host "  데이터:    $data (설치본과 따로)"
Write-Host '  끄기:      트레이에서 완전히 종료, 또는 scripts\preview-pr.ps1 -Stop'
if ($Interactive) { Start-Sleep -Seconds 4 }
