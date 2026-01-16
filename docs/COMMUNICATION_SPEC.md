# 프로세스 간 통신 명세서 (IPC Communication Specification)

> 이 문서는 Saba-chan의 모든 프로세스 간 통신 구조를 정의합니다.

## 🏗️ 아키텍처 개요

```
┌─────────────────────────────────────────────────────────────────┐
│                         Electron GUI                            │
│  ┌──────────────────┐        ┌──────────────────────────────┐  │
│  │   React App.js   │ ─IPC─► │    main.js (IPC Bridge)      │  │
│  │   (Renderer)     │ ◄─IPC─ │    (Electron Main Process)   │  │
│  └──────────────────┘        └──────────────┬───────────────┘  │
└────────────────────────────────────────────│───────────────────┘
                                              │
                                              │ HTTP REST
                                              │ 127.0.0.1:57474
                                              ▼
                                 ┌────────────────────────┐
                                 │   Core Daemon (Rust)   │
                                 │   Axum HTTP Server     │
                                 └───────────┬────────────┘
                                             │
                                ┌────────────┴────────────┐
                                │                         │
                          ┌─────▼─────┐           ┌──────▼──────┐
                          │ Supervisor │           │  Instances  │
                          │  (Modules) │           │   (JSON)    │
                          └────────────┘           └─────────────┘
```

---

## 📡 통신 계층 (Communication Layers)

### Layer 1: React ↔ Electron (IPC)
- **프로토콜**: Electron IPC (ipcRenderer.invoke)
- **방향**: 양방향
- **보안**: 동일 프로세스 내부 통신

### Layer 2: Electron ↔ Core Daemon (HTTP)
- **프로토콜**: HTTP/1.1 REST API
- **주소**: `http://127.0.0.1:57474`
- **방향**: 요청-응답
- **보안**: localhost 바인딩 (외부 접속 불가)

### Layer 3: Core Daemon ↔ Python Modules (Process)
- **프로토콜**: stdin/stdout (JSON)
- **방향**: 단방향 (요청 → 응답)
- **실행**: `python <module_path> <function> <config_json>`

---

## 🔌 API 엔드포인트 명세

### 1. 서버 목록 조회

**요청**:
```http
GET /api/servers
```

**응답**:
```json
{
  "servers": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "my-palworld-1",
      "module": "palworld",
      "status": "running",
      "pid": 12345,
      "uptime_seconds": 3600
    }
  ]
}
```

### 2. 서버 시작

**요청**:
```http
POST /api/server/:name/start
Content-Type: application/json

{
  "module": "palworld",
  "config": {
    "port": 8211,
    "max_players": 32
  }
}
```

**Rust 구조체**:
```rust
pub struct ServerStartRequest {
    pub module: String,
    #[serde(default)]
    pub config: Value,
}
```

**응답 (성공)**:
```json
{
  "success": true,
  "server": "my-palworld-1",
  "pid": 12345,
  "message": "Server 'my-palworld-1' started with PID 12345"
}
```

**응답 (실패)**:
```json
{
  "error": "Failed to start server: Module not found"
}
```

### 3. 서버 중지

**요청**:
```http
POST /api/server/:name/stop
Content-Type: application/json

{
  "force": false
}
```

**Rust 구조체**:
```rust
pub struct ServerStopRequest {
    #[serde(default)]
    pub force: bool,
}
```

**응답**:
```json
{
  "success": true,
  "server": "my-palworld-1",
  "message": "Server 'my-palworld-1' stopped"
}
```

### 4. 서버 상태 조회

**요청**:
```http
GET /api/server/:name/status
```

**응답**:
```json
{
  "success": true,
  "status": "running",
  "pid": 12345,
  "message": "Server is running"
}
```

### 5. 모듈 목록 조회

**요청**:
```http
GET /api/modules
```

**응답**:
```json
{
  "modules": [
    {
      "name": "palworld",
      "version": "0.1.0",
      "description": "Palworld dedicated server module",
      "path": "./modules/palworld"
    }
  ]
}
```

### 6. 인스턴스 목록 조회

**요청**:
```http
GET /api/instances
```

**응답**:
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "my-palworld-1",
    "module_name": "palworld",
    "executable_path": null,
    "working_dir": null,
    "auto_detect": true,
    "process_name": "PalServer-Win64-Shipping-Cmd",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": null
  }
]
```

### 7. 인스턴스 생성

**요청**:
```http
POST /api/instances
Content-Type: application/json

{
  "name": "my-minecraft-1",
  "module_name": "minecraft",
  "executable_path": "C:/minecraft/server.jar",
  "working_dir": "C:/minecraft",
  "port": 25565
}
```

**응답**:
```json
{
  "success": true,
  "id": "660e9500-f39c-52e5-b827-557766551111"
}
```

### 8. 인스턴스 조회

**요청**:
```http
GET /api/instance/:id
```

**응답**: 인스턴스 객체 (위 참조)

### 9. 인스턴스 삭제

**요청**:
```http
DELETE /api/instance/:id
```

**응답**:
```json
{
  "success": true
}
```

---

## 🎯 Electron IPC 브릿지

Electron의 `preload.js`를 통해 다음 API를 노출합니다:

```javascript
window.api = {
  // 서버 관리
  serverList: () => ipcRenderer.invoke('server:list'),
  serverStart: (name, options) => ipcRenderer.invoke('server:start', name, options),
  serverStop: (name, options) => ipcRenderer.invoke('server:stop', name, options),
  serverStatus: (name) => ipcRenderer.invoke('server:status', name),
  
  // 모듈 관리
  moduleList: () => ipcRenderer.invoke('module:list'),
  
  // 인스턴스 관리
  instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
  instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
  
  // 설정
  settingsLoad: () => ipcRenderer.invoke('settings:load'),
  settingsSave: (settings) => ipcRenderer.invoke('settings:save', settings),
  settingsGetPath: () => ipcRenderer.invoke('settings:getPath'),
}
```

### 사용 예시 (React)

```javascript
// 서버 시작
const handleStart = async (name, module) => {
  const result = await window.api.serverStart(name, {
    module: module,
    config: { port: 8211 }
  });
  if (result.error) {
    console.error(result.error);
  }
};

// 인스턴스 생성
const handleAddServer = async () => {
  const result = await window.api.instanceCreate({
    name: 'my-server-1',
    module_name: 'palworld'
  });
};
```

---

## ⚠️ 중요 사항 (CRITICAL)

### 1. 이름 vs ID 구분
- **API 경로**: `/api/server/:name` (사용자 지정 이름)
- **내부 저장**: `instance.id` (UUID)
- **주의**: 이름이 변경될 수 있으므로 내부적으로는 ID 사용 권장

### 2. 모듈명 전달
- 서버 시작/중지 시 **반드시** 모듈명 필요
- Instance에 저장된 `module_name` 사용
- 하드코딩 금지 ❌

### 3. 에러 응답 형식 통일
```json
{
  "error": "Error message here"
}
```

### 4. 성공 응답 형식
```json
{
  "success": true,
  "...": "additional fields"
}
```

---

## 🔍 디버깅 방법

### HTTP API 테스트 (PowerShell)

```powershell
# 서버 목록
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" | ConvertTo-Json -Depth 5

# 모듈 목록
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/modules" | ConvertTo-Json -Depth 5

# 인스턴스 생성
$body = @{
    name = "test-server"
    module_name = "palworld"
} | ConvertTo-Json

Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/instances" `
    -Method Post `
    -ContentType "application/json" `
    -Body $body | ConvertTo-Json

# 서버 시작
$startBody = @{
    module = "palworld"
    config = @{}
} | ConvertTo-Json

Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/server/test-server/start" `
    -Method Post `
    -ContentType "application/json" `
    -Body $startBody | ConvertTo-Json
```

### Electron DevTools

```javascript
// 콘솔에서 직접 API 호출
await window.api.serverList();
await window.api.moduleList();
await window.api.serverStart('my-server', { module: 'palworld' });
```

---

## 📅 변경 이력

| 날짜 | 변경 내용 |
|------|----------|
| 2026-01-16 | COMMUNICATION_SPEC.md 초안 작성 |
| 2026-01-16 | ServerStartRequest 구조 간소화 (resource 제거) |
| 2026-01-16 | stop_server/get_status에서 instance 조회 방식 통일 |

---

## 🔗 관련 문서

- [API_SPEC.md](API_SPEC.md) - Python 모듈 프로토콜
- [PROJECT_GUIDE.md](PROJECT_GUIDE.md) - 전체 프로젝트 가이드
- [README.md](README.md) - 프로젝트 개요
