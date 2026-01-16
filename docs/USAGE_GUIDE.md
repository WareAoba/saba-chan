# 사용자 가이드 - Saba-chan 서버 시작 방법

## 🎮 서버 시작 전 필수 설정

서버를 시작하기 전에 **반드시** 인스턴스에 실행 파일 경로를 설정해야 합니다.

### 1. Palworld 서버 설정 예시

#### 방법 A: GUI에서 인스턴스 생성 시

현재 GUI에서는 경로 입력이 제한적이므로, **방법 B**를 권장합니다.

#### 방법 B: instances.json 직접 수정

1. `c:\Git\saba-chan\instances.json` 파일 열기
2. 다음과 같이 수정:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "my-palworld-1",
    "module_name": "palworld",
    "executable_path": "D:\\SteamLibrary\\steamapps\\common\\PalServer\\PalServer.exe",
    "working_dir": "D:\\SteamLibrary\\steamapps\\common\\PalServer",
    "auto_detect": true,
    "process_name": "PalServer-Win64-Shipping-Cmd",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": null
  }
]
```

**중요 사항:**
- `executable_path`: PalServer.exe의 **전체 경로** 입력
- `working_dir`: 서버 폴더 경로 (보통 .exe와 같은 폴더)
- Windows 경로는 `\\` (이중 백슬래시) 사용
- UTF-8 **BOM 없이** 저장 (VS Code는 자동으로 처리)

#### Palworld 서버 기본 경로

```
# Steam 기본 설치 경로
C:\Program Files (x86)\Steam\steamapps\common\PalServer\PalServer.exe

# 다른 드라이브에 설치한 경우
D:\SteamLibrary\steamapps\common\PalServer\PalServer.exe

# SteamCMD로 직접 설치한 경우
C:\PalServer\PalServer.exe
```

### 2. Minecraft 서버 설정 예시

```json
[
  {
    "id": "660e9500-f39c-52e5-b827-557766551111",
    "name": "my-minecraft-1",
    "module_name": "minecraft",
    "executable_path": "C:\\minecraft\\server.jar",
    "working_dir": "C:\\minecraft",
    "auto_detect": true,
    "process_name": "java",
    "port": 25565,
    "rcon_port": null,
    "rcon_password": null
  }
]
```

**Minecraft 추가 설정 (선택):**
- GUI나 API로 `java_path`, `ram` 등을 config로 전달 가능
- 기본값: `java`, `8G`

---

## 🚀 서버 시작 방법

### GUI에서 시작

1. Electron GUI 실행 (`npm start`)
2. 서버 목록에서 원하는 서버 찾기
3. **Start** 버튼 클릭
4. 상태가 `running`으로 변경되면 성공

### API로 시작 (PowerShell)

```powershell
$startBody = @{
    module = "palworld"
    config = @{
        port = 8211
    }
} | ConvertTo-Json

Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/server/my-palworld-1/start" `
    -Method Post `
    -ContentType "application/json" `
    -Body $startBody | ConvertTo-Json
```

---

## ⚠️ 에러 해결 방법

### 에러: "server_executable not specified"

**원인**: instance에 `executable_path`가 설정되지 않음

**해결**:
1. `instances.json` 파일 열기
2. 해당 인스턴스에 `executable_path` 추가
3. Core Daemon 재시작 (GUI 재시작)

### 에러: "Executable not found: [경로]"

**원인**: 입력한 경로에 실행 파일이 없음

**해결**:
1. Windows 탐색기에서 실제 경로 확인
2. `instances.json`의 경로 수정
3. 경로에 `\\` (이중 백슬래시) 사용 확인

### 에러: "Failed to start: [WinError 2]"

**원인**: Python이 파일을 찾을 수 없음 (경로 문제)

**해결**:
1. `executable_path`가 **절대 경로**인지 확인
2. `working_dir`도 함께 설정
3. 경로에 한글이 있으면 영문 경로로 변경 시도

### 서버가 시작되지만 바로 종료됨

**원인**: 서버 설정 파일 오류 또는 포트 충돌

**해결**:
1. 서버 폴더의 로그 파일 확인
2. 다른 프로그램이 같은 포트 사용 중인지 확인
3. 서버 설정 파일(PalWorldSettings.ini 등) 검증

---

## 📂 instances.json 파일 위치

```
c:\Git\saba-chan\instances.json
```

**편집 시 주의사항:**
- VS Code나 Notepad++로 열기 (메모장 ❌)
- UTF-8 인코딩, BOM 없이 저장
- JSON 문법 검증: https://jsonlint.com
- Daemon이 실행 중이면 종료 후 편집

---

## 🔍 서버가 실행 중인지 확인

### GUI에서 확인
- 상태 뱃지가 `running` (녹색)

### API로 확인
```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" | ConvertTo-Json -Depth 5
```

### Windows 작업 관리자
- `PalServer-Win64-Shipping-Cmd` 프로세스 확인

---

## 💡 팁

### 1. 여러 서버 실행
- 각 서버는 **다른 포트** 사용 필요
- `instances.json`에서 `port` 값 변경

### 2. 자동 감지 (Auto Detect)
- `auto_detect: true`면 이미 실행 중인 서버 자동 인식
- GUI에서 시작하지 않아도 프로세스 표시됨

### 3. 서버 중지
- GUI의 **Stop** 버튼 클릭
- Force stop: `Ctrl+C`를 GUI에서 누르면 강제 종료 옵션 표시

---

## 📞 추가 도움말

문제가 계속되면:
1. Core Daemon 로그 확인 (터미널 출력)
2. `PROJECT_GUIDE.md` 참조
3. `COMMUNICATION_SPEC.md`에서 API 명세 확인
