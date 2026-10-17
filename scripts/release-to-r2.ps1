# ═══════════════════════════════════════════════════════════════
# Saba-chan Relay + R2 릴리즈 배포 스크립트
# ═══════════════════════════════════════════════════════════════
# 동작:
#   1. release-build/ 에 staging zip 생성 (SHA256 계산 포함)
#   2. R2 버킷(saba-chan-releases)에 releases/<version>/<filename> 으로 업로드
#   3. ReleaseManifest JSON 생성 → ed25519 서명
#   4. POST /admin/releases 로 Relay 에 등록 (채널: nightly)
#
# 환경변수는 .release.env 에서 로드.
# ═══════════════════════════════════════════════════════════════

param(
    [Parameter(Mandatory=$true)] [string]$Version,
    [string]$Channel = "stable",
    [switch]$SkipUpload,
    [switch]$SkipRegister
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot\..

# ── 환경변수 로드 ──
$envPath = ".release.env"
if (!(Test-Path $envPath)) { throw ".release.env 없음" }
Get-Content $envPath | Where-Object { $_ -match '^\s*[^#].*=' } | ForEach-Object {
    $k, $v = $_ -split '=', 2
    Set-Item -Path "env:$($k.Trim())" -Value $v.Trim()
}

$platform   = "windows-x64"
$targetRel  = "target\release"
$stageDir   = "release-build\staging-$Version"
$bucket     = $env:R2_BUCKET
$publicBase = $env:R2_PUBLIC_BASE.TrimEnd('/')
$relayUrl   = $env:RELAY_URL.TrimEnd('/')

if (Test-Path $stageDir) { Remove-Item $stageDir -Recurse -Force }
New-Item -ItemType Directory -Path $stageDir | Out-Null

function Zip-Files {
    param([string]$ZipPath, [string[]]$Sources)
    if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
    Compress-Archive -Path $Sources -DestinationPath $ZipPath -CompressionLevel Optimal -Force
}

function Zip-Directory {
    param([string]$ZipPath, [string]$Source)
    if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
    # 폴더 내용을 루트에 담는다 (폴더 자체는 zip 루트에 포함 안 함)
    Compress-Archive -Path (Join-Path $Source '*') -DestinationPath $ZipPath -CompressionLevel Optimal -Force
}

function Get-Sha256Hex {
    param([string]$Path)
    return (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLower()
}

# ═══════════════════════════════════════════════════════════════
# Step 1: 각 컴포넌트 zip 스테이징
# ═══════════════════════════════════════════════════════════════
$assets = @()

function Stage-Component {
    param([string]$Key, [string]$Filename, [scriptblock]$Builder, [string]$InstallDir = ".")
    $out = Join-Path $stageDir $Filename
    & $Builder $out
    if (!(Test-Path $out)) { throw "zip 생성 실패: $out" }
    $sha = Get-Sha256Hex $out
    $size = (Get-Item $out).Length
    Write-Host "  [OK] $Filename ($([math]::Round($size/1MB,2)) MB) sha256=$($sha.Substring(0,12))..."
    $script:assets += @{
        key = $Key
        component = $Key
        filename = $Filename
        installDir = $InstallDir
        sizeBytes = [int64]$size
        sha256 = $sha
        localPath = $out
        r2Key = "releases/$Version/$Filename"
    }
}

Write-Host "=== [1/4] Staging zips ==="

Stage-Component "saba-core" "saba-core-windows-x64.zip" {
    param($o)
    Zip-Files $o @("$targetRel\saba-core.exe")
}

Stage-Component "cli" "saba-chan-cli-windows-x64.zip" {
    param($o)
    Zip-Files $o @("$targetRel\saba-chan-cli.exe")
}

Stage-Component "updater" "saba-chan-updater-windows-x64.zip" {
    param($o)
    Zip-Files $o @("$targetRel\saba-chan-updater.exe")
}

Stage-Component "installer" "saba-chan-installer-windows-x64.zip" {
    param($o)
    Zip-Files $o @("$targetRel\saba-chan-installer.exe")
}

# GUI: electron-dist\win-unpacked 의 내용을 그대로 압축
Stage-Component "gui" "saba-chan-gui-windows-x64.zip" {
    param($o)
    Zip-Directory $o "saba-chan-gui\electron-dist\win-unpacked"
} "."

# locales
Stage-Component "locales" "locales.zip" {
    param($o)
    Zip-Directory $o "locales"
} "locales"

# ═══════════════════════════════════════════════════════════════
# Step 2: R2 업로드
# ═══════════════════════════════════════════════════════════════
if ($SkipUpload) {
    Write-Host "=== [2/4] R2 upload SKIPPED ==="
} else {
    Write-Host "=== [2/4] R2 upload to bucket '$bucket' (S3 multipart via aws-sdk) ==="
    foreach ($a in $assets) {
        Write-Host "  -> $($a.r2Key)"
        & node "$PSScriptRoot\upload-r2.mjs" $a.r2Key $a.localPath
        if ($LASTEXITCODE -ne 0) {
            throw "R2 upload 실패: $($a.r2Key)"
        }
    }
}

# ═══════════════════════════════════════════════════════════════
# Step 3: ReleaseManifest 생성 + 서명
# ═══════════════════════════════════════════════════════════════
Write-Host "=== [3/4] Manifest generation + signing ==="

$components = @{}
foreach ($a in $assets) {
    # 단일 SoT: workspace.package.version 상속으로 모든 Rust 컴포넌트는 $Version과 동일。
    # locales는 별도 자산처럼 다루고 나머지는 모두 동일 버전을 그대로 사용한다。
    $compVer = $Version
    $components[$a.key] = @{
        version     = $compVer
        asset       = $a.filename
        sha256      = $a.sha256
        size_bytes  = $a.sizeBytes
        install_dir = $a.installDir
        url         = "$publicBase/$($a.r2Key)"
    }
}

$manifest = [ordered]@{
    version    = $Version
    channel    = $Channel
    platform   = $platform
    components = $components
}

$manifestPath = Join-Path $stageDir "manifest-unsigned.json"
$manifestJson = $manifest | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText((Resolve-Path $stageDir).Path + "\manifest-unsigned.json", $manifestJson, (New-Object System.Text.UTF8Encoding($false)))

# 서명 (빌드 이미 완료되었다 가정 — release 빌드에서만 호출)
$signedPath = Join-Path $stageDir "manifest-signed.json"
$signExe = "target\release\examples\sign_manifest.exe"
if (!(Test-Path $signExe)) {
    cargo build --release --example sign_manifest -p saba-chan-updater-lib | Out-Null
}
& $signExe $manifestPath > $signedPath
if ($LASTEXITCODE -ne 0) { throw "sign_manifest 실패" }

$signedRaw = Get-Content $signedPath -Raw
$signedJson = $signedRaw | ConvertFrom-Json
if (!$signedJson.signature -or !$signedJson.keyId) { throw "서명 JSON 형식 오류" }

Write-Host "  [OK] signed keyId=$($signedJson.keyId)"

# ═══════════════════════════════════════════════════════════════
# Step 4: Relay 등록
# ═══════════════════════════════════════════════════════════════
if ($SkipRegister) {
    Write-Host "=== [4/4] Relay register SKIPPED ==="
    return
}

Write-Host "=== [4/4] Relay register to '$relayUrl' channel='$Channel' ==="

$relayAssets = @()
foreach ($a in $assets) {
    $relayAssets += @{
        component = $a.component
        platform  = $platform
        filename  = $a.filename
        sizeBytes = $a.sizeBytes
        sha256    = $a.sha256
        r2Key     = $a.r2Key
    }
}

# signedManifest 는 문자열(JSON)로 넣어야 함
$signedManifestString = $signedRaw.Trim()

$body = @{
    version        = $Version
    channel        = $Channel
    rolloutPercent = 100
    isCritical     = $false
    signedManifest = $signedManifestString
    assets         = $relayAssets
    notes          = @{ ko = "릴레이 + R2 전환 릴리즈"; en = "Relay + R2 migration release" }
} | ConvertTo-Json -Depth 10

$headers = @{
    "Content-Type"       = "application/json"
    "x-internal-secret"  = $env:RELAY_INTERNAL_SECRET
}

try {
    $resp = Invoke-RestMethod -Uri "$relayUrl/admin/releases" -Method Post -Body $body -Headers $headers
    Write-Host "  [OK] relay response: $($resp | ConvertTo-Json -Compress)"
} catch {
    Write-Host "Relay POST 실패: $($_.Exception.Message)"
    if ($_.ErrorDetails) { Write-Host "Details: $($_.ErrorDetails.Message)" }
    throw
}

Write-Host ""
Write-Host "=== 완료 ==="
Write-Host "  Version:  $Version"
Write-Host "  Channel:  $Channel"
Write-Host "  Assets:   $($assets.Count)"
Write-Host "  Installer: $((Get-Item "$targetRel\saba-chan-installer.exe").FullName)"
