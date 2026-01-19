# Saba Chan - 통합 테스트 스크립트
# GUI, Daemon, Protocol Clients 통합 테스트

param(
    [string]$TestType = "all",  # all, daemon, gui, protocol
    [string]$Module = "all",     # all, minecraft, palworld
    [int]$Port = 57474           # Daemon 포트
)

$ErrorActionPreference = "Stop"

# 색상 정의
$Colors = @{
    Green  = "`e[32m"
    Red    = "`e[31m"
    Yellow = "`e[33m"
    Blue   = "`e[34m"
    Reset  = "`e[0m"
}

function Write-Header {
    param([string]$Text)
    Write-Host "`n$($Colors.Blue)════════════════════════════════════════$($Colors.Reset)"
    Write-Host "$($Colors.Blue)  $Text$($Colors.Reset)"
    Write-Host "$($Colors.Blue)════════════════════════════════════════$($Colors.Reset)`n"
}

function Write-Success {
    param([string]$Text)
    Write-Host "$($Colors.Green)✓ $Text$($Colors.Reset)"
}

function Write-Error {
    param([string]$Text)
    Write-Host "$($Colors.Red)✗ $Text$($Colors.Reset)"
}

function Write-Info {
    param([string]$Text)
    Write-Host "$($Colors.Yellow)ℹ $Text$($Colors.Reset)"
}

# 1. 빌드 확인
Write-Header "1단계: 빌드 확인"

Write-Info "Rust 데몬 빌드 상태 확인 중..."
if (Test-Path "target/debug/core_daemon.exe") {
    Write-Success "Rust 데몬 빌드됨: target/debug/core_daemon.exe"
} elseif (Test-Path "target/release/core_daemon.exe") {
    Write-Success "Rust 데몬 빌드됨: target/release/core_daemon.exe"
} else {
    Write-Error "Rust 데몬을 찾을 수 없습니다. 먼저 'cargo build'를 실행하세요."
    exit 1
}

Write-Info "Python 모듈 문법 검사 중..."
$pythonModules = @(
    "modules/minecraft/lifecycle.py",
    "modules/palworld/lifecycle.py"
)

foreach ($module in $pythonModules) {
    if (Test-Path $module) {
        $result = python -m py_compile $module 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "$module 문법 정상"
        } else {
            Write-Error "$module 문법 오류: $result"
            exit 1
        }
    }
}

# 2. 포트 가용성 확인
Write-Header "2단계: 포트 가용성 확인"

Write-Info "포트 $Port 확인 중..."
$portInUse = netstat -ano 2>$null | Select-String ":$Port " | Select-String "LISTENING"

if ($portInUse) {
    Write-Error "포트 $Port가 이미 사용 중입니다."
    Write-Info "프로세스: $portInUse"
    exit 1
} else {
    Write-Success "포트 $Port 사용 가능"
}

# 3. 데몬 시작
Write-Header "3단계: 데몬 시작"

Write-Info "Core Daemon 시작 중..."
$daemonPath = if (Test-Path "target/release/core_daemon.exe") {
    "target/release/core_daemon.exe"
} else {
    "target/debug/core_daemon.exe"
}

# 백그라운드에서 데몬 시작
$daemonProcess = Start-Process -FilePath $daemonPath -PassThru -NoNewWindow -ErrorAction SilentlyContinue

if ($null -eq $daemonProcess) {
    Write-Error "데몬 시작 실패"
    exit 1
}

Write-Success "데몬 시작됨 (PID: $($daemonProcess.Id))"
Start-Sleep -Seconds 2

# 4. 데몬 연결 테스트
Write-Header "4단계: 데몬 연결 테스트"

$maxRetries = 10
$retries = 0
$connected = $false

while ($retries -lt $maxRetries -and -not $connected) {
    try {
        $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/api/modules" -TimeoutSec 2 -ErrorAction Stop
        Write-Success "데몬 응답 확인 (HTTP 200)"
        $connected = $true
    } catch {
        $retries++
        if ($retries -lt $maxRetries) {
            Write-Info "재시도 중... ($retries/$maxRetries)"
            Start-Sleep -Seconds 1
        }
    }
}

if (-not $connected) {
    Write-Error "데몬 연결 실패 ($maxRetries회 시도)"
    Stop-Process -Id $daemonProcess.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# 5. API 엔드포인트 테스트
Write-Header "5단계: API 엔드포인트 테스트"

Write-Info "GET /api/modules 테스트..."
try {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/modules" -Method Get -ErrorAction Stop
    Write-Success "/api/modules 응답: $($response.Count) 모듈 발견"
} catch {
    Write-Error "/api/modules 실패: $($_.Exception.Message)"
}

# 6. 테스트 인스턴스 생성
Write-Header "6단계: 테스트 인스턴스 생성"

$testInstances = @()

if ($Module -eq "minecraft" -or $Module -eq "all") {
    Write-Info "Minecraft 테스트 인스턴스 생성 중..."
    $minecraftInstance = @{
        name         = "test-minecraft-$(Get-Random)"
        module_name  = "minecraft"
        executable   = "java"
        port         = 25565
        rcon_port    = 25575
        rcon_password = "test123"
    }
    
    try {
        $response = Invoke-RestMethod `
            -Uri "http://127.0.0.1:$Port/api/instances" `
            -Method Post `
            -ContentType "application/json" `
            -Body ($minecraftInstance | ConvertTo-Json) `
            -ErrorAction Stop
        
        Write-Success "Minecraft 인스턴스 생성: $($response.id)"
        $testInstances += $response
    } catch {
        Write-Error "Minecraft 인스턴스 생성 실패: $($_.Exception.Message)"
    }
}

if ($Module -eq "palworld" -or $Module -eq "all") {
    Write-Info "Palworld 테스트 인스턴스 생성 중..."
    $palworldInstance = @{
        name           = "test-palworld-$(Get-Random)"
        module_name    = "palworld"
        executable     = "PalServer.exe"
        port           = 8211
        rest_host      = "127.0.0.1"
        rest_port      = 8212
        rest_username  = "admin"
        rest_password  = "password123"
    }
    
    try {
        $response = Invoke-RestMethod `
            -Uri "http://127.0.0.1:$Port/api/instances" `
            -Method Post `
            -ContentType "application/json" `
            -Body ($palworldInstance | ConvertTo-Json) `
            -ErrorAction Stop
        
        Write-Success "Palworld 인스턴스 생성: $($response.id)"
        $testInstances += $response
    } catch {
        Write-Error "Palworld 인스턴스 생성 실패: $($_.Exception.Message)"
    }
}

# 7. 명령어 실행 시뮬레이션
Write-Header "7단계: 명령어 실행 시뮬레이션"

foreach ($instance in $testInstances) {
    Write-Info "테스트 인스턴스: $($instance.name) (ID: $($instance.id))"
    
    if ($instance.module_name -eq "minecraft") {
        Write-Info "Minecraft RCON 테스트..."
        $testCommand = @{
            command    = "say"
            args       = @{ message = "Hello from integration test!" }
            instance_id = $instance.id
        }
        
        $rconUrl = "http://127.0.0.1:$Port/api/instance/$($instance.id)/rcon"
        Write-Info "POST $rconUrl"
        Write-Info "Payload: $($testCommand | ConvertTo-Json -Compress)"
        
        try {
            $response = Invoke-RestMethod `
                -Uri $rconUrl `
                -Method Post `
                -ContentType "application/json" `
                -Body ($testCommand | ConvertTo-Json) `
                -TimeoutSec 5 `
                -ErrorAction Stop
            
            Write-Success "RCON 명령어 응답: $($response | ConvertTo-Json -Compress)"
        } catch {
            Write-Error "RCON 명령어 실패: $($_.Exception.Message)"
        }
    }
    elseif ($instance.module_name -eq "palworld") {
        Write-Info "Palworld REST 테스트..."
        $testCommand = @{
            endpoint    = "/api/announce"
            method      = "POST"
            body        = @{ message = "Hello from integration test!" }
            instance_id = $instance.id
        }
        
        $restUrl = "http://127.0.0.1:$Port/api/instance/$($instance.id)/rest"
        Write-Info "POST $restUrl"
        Write-Info "Payload: $($testCommand | ConvertTo-Json -Compress)"
        
        try {
            $response = Invoke-RestMethod `
                -Uri $restUrl `
                -Method Post `
                -ContentType "application/json" `
                -Body ($testCommand | ConvertTo-Json) `
                -TimeoutSec 5 `
                -ErrorAction Stop
            
            Write-Success "REST 명령어 응답: $($response | ConvertTo-Json -Compress)"
        } catch {
            Write-Error "REST 명령어 실패: $($_.Exception.Message)"
        }
    }
}

# 8. 정리
Write-Header "8단계: 정리"

Write-Info "테스트 인스턴스 삭제 중..."
foreach ($instance in $testInstances) {
    try {
        Invoke-RestMethod `
            -Uri "http://127.0.0.1:$Port/api/instance/$($instance.id)" `
            -Method Delete `
            -ErrorAction Stop | Out-Null
        Write-Success "인스턴스 삭제: $($instance.id)"
    } catch {
        Write-Error "인스턴스 삭제 실패: $($instance.id)"
    }
}

Write-Info "데몬 종료 중..."
Stop-Process -Id $daemonProcess.Id -Force -ErrorAction SilentlyContinue
Write-Success "데몬 종료됨"

# 결론
Write-Header "테스트 완료"
Write-Success "모든 테스트가 완료되었습니다!"
Write-Info "다음 단계: GUI 테스트 실행 (npm start in electron_gui)"
