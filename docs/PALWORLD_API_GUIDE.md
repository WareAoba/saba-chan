# Palworld API 구현 가이드

## 공식 API 형식

Palworld 공식 API는 다음 형식을 사용합니다:

```python
import requests
import json

url = "http://localhost:8212/v1/api/announce"

payload = json.dumps({
  "message": "Hello, Palworld!"
})
headers = {
  'Content-Type': 'application/json'
}

response = requests.request("POST", url, headers=headers, data=payload)
print(response.text)
```

## Saba Chan 통합 방식

### 시스템 아키텍처

```
┌─────────────────┐
│  GUI            │
│  CommandModal   │
└────────┬────────┘
         │ 명령어: "announce", args: {message: "Hello"}
         │
┌────────▼────────────────────────┐
│  Electron main.js               │
│  instance:executeCommand        │
│  ├─ module_name: "palworld"     │
│  ├─ endpoint: "/v1/api/announce"│
│  └─ body: {message: "Hello"}    │
└────────┬─────────────────────────┘
         │ POST /api/instance/:id/rest
         │
┌────────▼────────────────────────────────────┐
│  Core Daemon (Rust)                         │
│  execute_rest_command                       │
│  ├─ endpoint: "/v1/api/announce" (그대로)   │
│  ├─ method: POST                            │
│  └─ body: {message: "Hello"}                │
└────────┬─────────────────────────────────────┘
         │ HTTP POST with REST client
         │
┌────────▼──────────────────────────────────────┐
│  Palworld Server (127.0.0.1:8212)            │
│  POST /v1/api/announce                       │
│  Content-Type: application/json              │
│  {"message": "Hello"}                        │
└────────────────────────────────────────────────┘
```

### 동가 요청 비교

#### 1. 공식 Python 코드
```python
import requests

response = requests.post(
    "http://localhost:8212/v1/api/announce",
    json={"message": "Hello, Palworld!"}
)
```

#### 2. 우리 시스템 (GUI → Daemon)
```javascript
// GUI에서 입력
command = "announce"
args = { message: "Hello, Palworld!" }

// main.js에서 처리
{
  endpoint: "/v1/api/announce",    // 공식 형식
  method: "POST",
  body: { message: "Hello, Palworld!" },  // 공식 형식
  rest_host: "127.0.0.1",
  rest_port: 8212,
  username: "admin",
  password: "8434"
}

// daemon이 받아서 REST client로 실행
HTTP POST: http://127.0.0.1:8212/v1/api/announce
Headers: Content-Type: application/json
Body: { "message": "Hello, Palworld!" }
```

#### 3. 우리 시스템 (Python 모듈 직접 호출)
```python
import urllib.request
import json
import os

DAEMON_API_URL = os.getenv('DAEMON_API_URL', 'http://127.0.0.1:57474')

# 모듈에서 daemon 호출
url = f"{DAEMON_API_URL}/api/instance/{instance_id}/rest"

payload = {
    "endpoint": "/v1/api/announce",  # Palworld 공식 형식
    "method": "POST",
    "body": {"message": "Hello, Palworld!"},
    "rest_host": "127.0.0.1",
    "rest_port": 8212,
    "username": "admin",
    "password": "8434"
}

# daemon에 요청
data = json.dumps(payload).encode('utf-8')
req = urllib.request.Request(
    url,
    data=data,
    headers={'Content-Type': 'application/json'}
)

with urllib.request.urlopen(req) as response:
    result = json.loads(response.read().decode('utf-8'))
```

## Palworld REST API 엔드포인트

### 모든 지원 엔드포인트

| 명령어 | 엔드포인트 | 메서드 | 설명 |
|--------|-----------|--------|------|
| announce | `/v1/api/announce` | POST | 공지사항 방송 |
| info | `/v1/api/info` | GET | 서버 정보 조회 |
| metrics | `/v1/api/metrics` | GET | 서버 통계 조회 |
| players | `/v1/api/players` | GET | 플레이어 목록 조회 |
| save | `/v1/api/save` | POST | 서버 데이터 저장 |
| shutdown | `/v1/api/shutdown` | POST | 서버 종료 예약 |
| kick | `/v1/api/kick` | POST | 플레이어 강제 퇴장 |
| ban | `/v1/api/ban` | POST | 플레이어 차단 |
| unban | `/v1/api/unban` | POST | 차단 해제 |

### 요청 예제

#### announce - 공지사항
```
POST /v1/api/announce
{
  "message": "Server will restart in 10 minutes"
}
```

#### info - 서버 정보
```
GET /v1/api/info
```

#### save - 데이터 저장
```
POST /v1/api/save
```

#### shutdown - 종료 예약
```
POST /v1/api/shutdown
{
  "seconds": 300
}
```

#### kick - 플레이어 강제 퇴장
```
POST /v1/api/kick
{
  "user_id": "12345"
}
```

#### ban - 플레이어 차단
```
POST /v1/api/ban
{
  "user_id": "12345"
}
```

## Saba Chan 구현

### modules/palworld/lifecycle.py

```python
import urllib.request
import json
import os

DAEMON_API_URL = os.getenv('DAEMON_API_URL', 'http://127.0.0.1:57474')

def command(instance_id, cmd, *args):
    """
    Palworld 명령어 실행
    
    Args:
        instance_id: 인스턴스 ID
        cmd: 명령어 (announce, info, save, ...)
        *args: 명령어 인자들
    
    Returns:
        명령어 실행 결과
    
    예제:
        command(instance_id, "announce", "Server restarting in 5 minutes")
        command(instance_id, "shutdown", "300")
    """
    url = f"{DAEMON_API_URL}/api/instance/{instance_id}/rest"
    
    # Palworld 공식 API 엔드포인트
    endpoint_map = {
        'announce': '/v1/api/announce',
        'kick': '/v1/api/kick',
        'ban': '/v1/api/ban',
        'unban': '/v1/api/unban',
        'info': '/v1/api/info',
        'players': '/v1/api/players',
        'metrics': '/v1/api/metrics',
        'save': '/v1/api/save',
        'shutdown': '/v1/api/shutdown'
    }
    
    endpoint = endpoint_map.get(cmd, f'/v1/api/{cmd}')
    
    # 명령어별 body 구성
    body = {}
    if cmd == 'announce' and args:
        body['message'] = args[0]
    elif cmd == 'shutdown' and args:
        body['seconds'] = int(args[0]) if args[0].isdigit() else 300
    elif cmd in ('kick', 'ban', 'unban') and args:
        body['user_id'] = args[0]
    elif len(args) > 0:
        body['args'] = list(args)
    
    # Daemon API 호출
    payload = {
        "endpoint": endpoint,
        "method": "POST" if cmd != 'info' and cmd != 'players' and cmd != 'metrics' else "GET",
        "body": body,
        "instance_id": instance_id
    }
    
    data = json.dumps(payload).encode('utf-8')
    req = urllib.request.Request(
        url,
        data=data,
        headers={'Content-Type': 'application/json'}
    )
    
    try:
        with urllib.request.urlopen(req) as response:
            result = json.loads(response.read().decode('utf-8'))
            if result.get('success'):
                return result.get('data', {})
            else:
                raise Exception(result.get('error', 'Unknown error'))
    except Exception as e:
        raise Exception(f"Failed to execute command: {str(e)}")
```

### electron_gui/main.js 

```javascript
} else if (instance.module_name === 'palworld') {
    // Palworld는 REST API 사용
    console.log(`[Main] Using REST API protocol for Palworld`);
    protocolUrl = `${IPC_BASE}/api/instance/${id}/rest`;
    
    // Palworld 공식 API 형식: /v1/api/{endpoint}
    commandPayload = {
        endpoint: `/v1/api/${command.command}`,
        method: 'POST',
        body: command.args || {},
        instance_id: id,
        rest_host: instance.rest_host,
        rest_port: instance.rest_port,
        username: instance.rest_username,
        password: instance.rest_password
    };
}
```

## 테스트 방법

### 1. CLI 테스트 (PowerShell)
```powershell
$instanceId = "68b29cef-e584-4bd0-91dc-771865e31e25"

$payload = @{
    endpoint="/v1/api/announce"
    method="POST"
    body=@{message="Hello from Saba Chan!"}
    rest_host="127.0.0.1"
    rest_port=8212
    username="admin"
    password="8434"
} | ConvertTo-Json

Invoke-RestMethod `
  -Uri "http://127.0.0.1:57474/api/instance/$instanceId/rest" `
  -Method Post `
  -ContentType "application/json" `
  -Body $payload
```

### 2. GUI 테스트
1. Palworld 인스턴스 선택
2. "명령어" 버튼 클릭
3. 명령어 입력: "announce"
4. 파라미터 입력: message = "Test message"
5. ⏎ 실행
6. 결과 확인

### 3. 공식 Python 코드와 비교
```python
# 공식 방식 (직접 API 호출)
import requests
response = requests.post(
    "http://localhost:8212/v1/api/announce",
    json={"message": "Hello, Palworld!"}
)

# Saba Chan 방식 (daemon 경유)
# GUI: command="announce", args={message: "Hello, Palworld!"}
# → daemon이 같은 요청을 Palworld로 전송
```

## 주요 포인트

### ✅ 공식 API 호환성
- 엔드포인트: `/v1/api/{command}` (공식 형식 유지)
- 메서드: POST/GET (공식 형식 유지)
- Body: JSON (공식 형식 유지)

### ✅ 시스템 통합
- GUI에서 "announce" 입력 → daemon이 `/v1/api/announce`로 변환
- Python 모듈에서도 공식 엔드포인트 사용
- 모든 계층에서 공식 형식 준수

### ✅ 확장성
- 새로운 엔드포인트 추가 시 `endpoint_map`에만 추가
- daemon과 GUI는 변경 불필요
- 플러그 앤 플레이 방식

## 결론

Saba Chan의 Palworld 통합은 **공식 API 형식을 완벽하게 따르면서도** 통일된 인터페이스를 제공합니다:

- 🎮 GUI에서: 간단한 명령어 입력
- 🔧 daemon에서: 공식 API 형식으로 변환
- 📡 서버와: 공식 형식으로 통신

이를 통해 **유지보수성**, **확장성**, **공식 호환성**을 모두 확보했습니다!
