# IPC & API 사양

## 1. Plugin Protocol (Core Daemon ↔ Python Module)

### 요청 형식

```bash
python <module_path> <function> <config_json>
```

**예시**:
```bash
python modules/minecraft/lifecycle.py start '{"java_path":"/usr/bin/java","ram":"8G"}'
```

### 응답 형식

**stdout** (JSON):
```json
{
  "success": true,
  "pid": 12345,
  "message": "Server started"
}
```

**stderr** (로그, 무시됨):
```
[INFO] Starting Minecraft server
[DEBUG] Using java path: /usr/bin/java
```

### 함수 규약

#### `start(config: dict) -> dict`

**입력**:
```python
{
  "java_path": "/usr/bin/java",
  "server_jar": "server.jar",
  "ram": "8G"
}
```

**출력**:
```json
{
  "success": true,
  "pid": 12345,
  "message": "Server started"
}
```

#### `stop(config: dict) -> dict`

**입력**:
```python
{
  "pid": 12345
}
```

**출력**:
```json
{
  "success": true,
  "message": "Sent SIGTERM to PID 12345"
}
```

#### `status(config: dict) -> dict`

**입력**:
```python
{
  "pid": 12345
}
```

**출력**:
```json
{
  "success": true,
  "status": "running",
  "pid": 12345,
  "message": "Server is running"
}
```

### 오류 응답

```json
{
  "success": false,
  "message": "Error description"
}
```

## 2. Core Daemon IPC API

### 엔드포인트 (stub: HTTP REST / 실제: gRPC or Unix socket)

#### 서버 목록

```
GET /api/servers
```

**응답**:
```json
{
  "servers": [
    {
      "name": "minecraft-main",
      "module": "minecraft",
      "status": "running",
      "pid": 12345,
      "resource": {
        "ram": "8G",
        "cpu": 4
      }
    },
    {
      "name": "palworld-01",
      "module": "palworld",
      "status": "stopped",
      "pid": null
    }
  ]
}
```

#### 서버 상태 조회

```
GET /api/server/<name>/status
```

**응답**:
```json
{
  "name": "minecraft-main",
  "status": "running",
  "state": "RUNNING",
  "pid": 12345,
  "uptime_seconds": 3600
}
```

#### 서버 시작

```
POST /api/server/<name>/start
```

**요청**:
```json
{
  "resource": {
    "ram": "8G",
    "cpu": 4
  }
}
```

**응답**:
```json
{
  "success": true,
  "name": "minecraft-main",
  "pid": 12345,
  "state": "STARTING"
}
```

#### 서버 중지

```
POST /api/server/<name>/stop
```

**요청**:
```json
{
  "force": false
}
```

**응답**:
```json
{
  "success": true,
  "name": "minecraft-main",
  "state": "STOPPING"
}
```

## 3. Discord Bot API

### 명령어

#### /server list

```
/server list
```

**응답**:
```
Minecraft (running) - RAM: 8G, CPU: 4 cores
Palworld (stopped)
```

#### /server start

```
/server start minecraft
```

**응답**:
```
✅ Minecraft server starting... (PID: 12345)
```

#### /server stop

```
/server stop minecraft
```

**응답**:
```
⏹️ Minecraft server stopping...
```

#### /server status

```
/server status minecraft
```

**응답**:
```
📊 Minecraft Status:
- State: RUNNING
- PID: 12345
- Uptime: 2h 30m
```

## 4. Electron GUI API

### IPC Channels (Main ↔ Renderer)

#### `server:list`

```javascript
await window.api.serverList()
```

**응답**:
```json
{
  "servers": [...]
}
```

#### `server:start`

```javascript
await window.api.serverStart("minecraft-main")
```

**응답**:
```json
{
  "success": true,
  "pid": 12345
}
```

#### `server:stop`

```javascript
await window.api.serverStop("minecraft-main")
```

**응답**:
```json
{
  "success": true
}
```

#### `server:status`

```javascript
await window.api.serverStatus("minecraft-main")
```

**응답**:
```json
{
  "status": "running",
  "pid": 12345
}
```

## 5. 모듈 설정 (module.toml)

### Minecraft 예시

```toml
[module]
name = "minecraft"
version = "1.0.0"
description = "Minecraft server management"
entry = "lifecycle.py"

[config]
java_path = "/usr/bin/java"
server_jar = "server.jar"
eula = true
```

### Palworld 예시

```toml
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[config]
server_executable = "PalServer.exe"
port = 8211
```

## 6. 상태 머신 전이

```
STOPPED
  ↓ (start)
STARTING
  ↓ (successful)
RUNNING
  ↓ (stop)
STOPPING
  ↓ (confirmed)
STOPPED

RUNNING ↓ (crash)
CRASHED
  ↓ (restart or acknowledge)
STOPPED
```

## 7. 오류 코드

| 코드 | 설명 |
|------|------|
| `ERR_PROCESS_NOT_FOUND` | PID를 찾을 수 없음 |
| `ERR_INVALID_STATE` | 해당 상태에서 전이 불가능 |
| `ERR_PLUGIN_TIMEOUT` | 플러그인 실행 타임아웃 |
| `ERR_RESOURCE_LIMIT` | 자원 제한 실패 |
| `ERR_PERMISSION_DENIED` | 권한 없음 |

## 8. 보안

- IPC 서버: Unix socket (권장) 또는 로컬 gRPC
- Discord Bot: 토큰 기반 인증
- GUI: localhost only
- 모든 상태 변경은 Core Daemon을 통해서만 가능
