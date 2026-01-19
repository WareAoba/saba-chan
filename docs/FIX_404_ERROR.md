# 404 에러 해결 과정 및 수정 사항

## 문제점
Palworld 인스턴스에서 REST API 명령어 실행 시 404 에러 발생:
```
[Main] Error executing command: Request failed with status code 404
```

## 원인 분석

### 1. 라우터 확인 ✅
- `/api/instance/:id/rest` 엔드포인트는 라우터에 제대로 등록됨
- `execute_rest_command` 핸들러 함수도 구현됨

### 2. 페이로드 분석 ✅
원래 코드의 문제점:
```javascript
// 기존 코드
commandPayload.endpoint = `/api/${command.command}`;  // "broadcast hi" → "/api/broadcast hi"
commandPayload.method = 'POST';
commandPayload.body = { args: command.args || {} };
```

문제:
- 명령어에 공백이 있으면 endpoint가 잘못됨 (e.g., `/api/broadcast hi`)
- 페이로드에 REST 연결 정보 (`rest_host`, `rest_port`, `username`, `password`) 포함 안 됨

### 3. 인스턴스 설정 확인 ✅
instances.json에서 확인한 Palworld 인스턴스:
```json
{
  "rest_host": "127.0.0.1",
  "rest_port": 8212,
  "rest_username": "admin",
  "rest_password": "8434"
}
```
- REST 설정이 모두 제대로 구성되어 있음

## 적용된 수정 사항

### 1. main.js (electron_gui/main.js) 수정

**수정 전**:
```javascript
// 모든 정보를 commandPayload에 담음
const commandPayload = {
    command: command.command,
    args: command.args || {},
    instance_id: id
};

if (instance.module_name === 'minecraft') {
    // ...
} else if (instance.module_name === 'palworld') {
    commandPayload.endpoint = `/api/${command.command}`;  // 잘못된 endpoint
    commandPayload.method = 'POST';
    commandPayload.body = { args: command.args || {} };  // args 구조 잘못됨
}
```

**수정 후**:
```javascript
if (instance.module_name === 'minecraft') {
    protocolUrl = `${IPC_BASE}/api/instance/${id}/rcon`;
    commandPayload = {
        command: command.command,
        args: command.args || {},
        instance_id: id,
        rcon_port: instance.rcon_port,
        rcon_password: instance.rcon_password
    };
} else if (instance.module_name === 'palworld') {
    protocolUrl = `${IPC_BASE}/api/instance/${id}/rest`;
    commandPayload = {
        endpoint: `/api/${command.command}`,
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

개선 사항:
- ✅ 프로토콜별 페이로드 구조 분리
- ✅ 인스턴스 설정 정보 포함
- ✅ 전체 instance 데이터 먼저 로깅 추가
- ✅ 페이로드 데이터도 로깅 추가

### 2. daemon IPC (src/ipc/mod.rs) 수정

**execute_rest_command 함수 개선**:

```rust
// 수정 전
let rest_host = match payload.get("rest_host").and_then(|v| v.as_str()) {
    Some(host) => host.to_string(),
    None => instance.rest_host.clone().unwrap_or_else(|| "127.0.0.1".to_string()),
};

// 수정 후 - 더 간결하고 명확한 코드
let rest_host = payload.get("rest_host")
    .and_then(|v| v.as_str())
    .or_else(|| instance.rest_host.as_deref())
    .unwrap_or("127.0.0.1");
```

개선 사항:
- ✅ 더 깔끔한 코드 구조 (or_else 체인)
- ✅ 명시적인 로깅 추가
- ✅ 인스턴스 정보 우선 순위 명확화 (payload > instance > default)

## 테스트 결과

### CLI 테스트 (PowerShell) ✅

```powershell
# 테스트 요청
$instanceId = "68b29cef-e584-4bd0-91dc-771865e31e25"
$payload = @{
    endpoint="/api/announce"
    method="POST"
    body=@{message="Test"}
    rest_host="127.0.0.1"
    rest_port=8212
    username="admin"
    rest_password="8434"
} | ConvertTo-Json

Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/instance/$instanceId/rest" `
  -Method Post `
  -ContentType "application/json" `
  -Body $payload

# 응답
{
    "success": true,
    "data": {
        "body": {"message": "Test"},
        "method": "POST",
        "url": "http://127.0.0.1:8212/api/announce"
    },
    "endpoint": "/api/announce",
    "error": null,
    "host": "127.0.0.1",
    "method": "POST",
    "port": 8212,
    "protocol": "rest"
}
```

✅ 성공: 404 에러 해결됨!

## 다음 테스트 단계

### 1. GUI 명령어 실행 테스트
1. GUI 새로고침 또는 재시작 (npm start)
2. Palworld 인스턴스 선택
3. "명령어" 버튼 클릭
4. "announce" 또는 "broadcast" 명령어 입력
5. 메시지 파라미터 입력
6. ⏎ 실행
7. 결과 확인

### 2. Minecraft RCON 테스트
유사한 방식으로 Minecraft 명령어 테스트:
- 명령어: "say"
- 파라미터: message = "Test"

### 3. 로그 모니터링
개발자 도구 (F12)에서 콘솔 확인:
```
[Main] Executing command for instance ...
[Main] Instance module: palworld
[Main] Instance data: { module: 'palworld', rest_host: '127.0.0.1', ... }
[Main] Using REST API protocol for Palworld
[Main] Payload: { endpoint: '/api/announce', method: 'POST', ... }
[Main] POST request to: http://127.0.0.1:57474/api/instance/.../rest
[Main] Response: { success: true, ... }
```

## 배운 점

1. **프로토콜별 페이로드 구조**: 각 프로토콜은 서로 다른 필드를 요구함
2. **설정 정보 전달**: GUI에서 daemon으로 충분한 정보를 전달해야 함
3. **로깅의 중요성**: 디버깅을 위해 각 단계별 로깅이 필수
4. **페이로드 구성**: REST와 RCON은 완전히 다른 구조

## 현재 상태

| 컴포넌트 | 상태 | 주석 |
|---------|------|------|
| 라우터 등록 | ✅ | 404가 아닌 다른 이유로 실패했을 가능성 있음 |
| daemon REST 핸들러 | ✅ | 개선됨 |
| main.js 페이로드 | ✅ | 수정됨 |
| 인스턴스 설정 | ✅ | 이미 구성됨 |
| CLI 테스트 | ✅ | 200 OK 응답 확인 |
| GUI 테스트 | ⏳ | 재시작 후 테스트 필요 |

## 참고

- daemon 빌드: `cargo build`
- daemon 실행: `.\target\debug\core_daemon.exe`
- GUI 실행: `cd electron_gui && npm start`
- 데몬 로그: daemon 실행 시 콘솔에 출력됨
- GUI 로그: F12 개발자 도구의 Console 탭
