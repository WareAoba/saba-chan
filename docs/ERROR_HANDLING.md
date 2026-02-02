# 에러 처리 시스템 가이드

## 개요

Saba-chan은 3단계 에러 처리 시스템을 사용하여 사용자에게 명확하고 이해하기 쉬운 에러 메시지를 제공합니다:

1. **모듈별 에러 메시지 정의** (`module.toml`)
2. **네트워크 vs 서버 에러 구분**
3. **자동 서버 상태 확인**

---

## 1. 모듈별 에러 메시지

### module.toml 구조

각 게임 모듈은 `[errors]` 섹션에서 자체 에러 메시지를 정의합니다:

```toml
[errors]
server_not_running = "서버가 실행중이지 않습니다. 먼저 서버를 시작해주세요"
auth_failed = "인증 실패: REST API 사용자명/비밀번호 또는 RCON 비밀번호를 확인해주세요"
player_not_found = "플레이어를 찾을 수 없습니다. 닉네임 또는 SteamID를 확인해주세요"
connection_refused = "서버에 연결할 수 없습니다. 서버 포트와 REST API 설정을 확인해주세요"
timeout = "서버 응답 시간 초과. 서버 상태를 확인해주세요"
rest_api_disabled = "REST API가 비활성화되어 있습니다"
rcon_disabled = "RCON이 비활성화되어 있습니다"
internal_server_error = "서버 내부 오류가 발생했습니다"
```

### 지원되는 에러 타입

| 에러 키 | 설명 | 사용 예시 |
|---------|------|-----------|
| `server_not_running` | 서버가 실행되지 않은 상태 | 명령어 실행 시도 시 |
| `auth_failed` | 인증 실패 (401, 403) | 잘못된 비밀번호 |
| `player_not_found` | 플레이어 조회 실패 (404) | kick, ban 명령 |
| `connection_refused` | 네트워크 연결 실패 | ECONNREFUSED |
| `timeout` | 응답 시간 초과 | ETIMEDOUT |
| `rest_api_disabled` | REST API 미활성화 | API 호출 실패 |
| `rcon_disabled` | RCON 미활성화 | RCON 연결 실패 |
| `internal_server_error` | 서버 내부 오류 (500) | 서버 크래시 등 |

---

## 2. 에러 처리 플로우

### GUI (Electron)

```javascript
// main.js의 IPC 핸들러
if (error.response) {
    // HTTP 에러 (서버에서 응답이 온 경우)
    const status = error.response.status;
    const data = error.response.data;
    
    switch (status) {
        case 401:
            return { error: "인증 실패: 사용자명/비밀번호를 확인해주세요" };
        case 404:
            return { error: "서버를 찾을 수 없습니다" };
        case 503:
            return { error: "서버가 응답하지 않습니다. 서버 상태를 확인해주세요" };
    }
}

// 네트워크 에러 (서버에 연결조차 안 되는 경우)
if (error.code === 'ECONNREFUSED') {
    return { error: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요' };
}
```

### Discord 봇

```javascript
// 1. 서버 실행 상태 사전 확인
if (server.status !== 'running') {
    const moduleErrors = moduleMetadata[moduleName]?.errors || {};
    const errorMsg = moduleErrors.server_not_running || '서버가 실행중이지 않습니다';
    await message.reply(`❌ **${server.name}**: ${errorMsg}`);
    return;
}

// 2. 명령 실행 후 에러 처리
try {
    const result = await executeCommand();
    
    if (!result.success) {
        // 모듈별 정의된 에러 메시지 사용
        const moduleErrors = moduleMetadata[moduleName]?.errors || {};
        let friendlyError = result.error;
        
        if (friendlyError.includes('인증')) {
            friendlyError = moduleErrors.auth_failed || friendlyError;
        }
        
        await message.reply(`❌ ${friendlyError}`);
    }
} catch (error) {
    // 네트워크 에러 구분
    if (error.code === 'ECONNREFUSED') {
        errorMsg = moduleErrors.connection_refused || '연결 실패';
    } else if (error.code === 'ETIMEDOUT') {
        errorMsg = moduleErrors.timeout || '응답 시간 초과';
    }
}
```

---

## 3. 에러 메시지 커스터마이징

### 새 모듈에 에러 메시지 추가

1. **module.toml 파일 수정**:

```toml
[module]
name = "your_game"
# ... 기타 설정

[errors]
server_not_running = "게임 서버가 꺼져있습니다 🎮"
auth_failed = "로그인 정보가 틀렸어요! 다시 확인해주세요"
# ... 게임별 커스텀 메시지
```

2. **lifecycle.py에서 에러 반환**:

```python
def command(config):
    if not server_is_running():
        return {
            "success": False,
            "message": "server_not_running"  # 에러 키 반환
        }
```

3. **에러 메시지는 자동으로 적용됨**:
   - GUI는 하드코딩된 메시지 사용
   - Discord 봇은 module.toml의 메시지 사용

---

## 4. 에러 메시지 우선순위

1. **모듈 정의** (`module.toml [errors]`)
   - 게임별 특화된 메시지
   - Discord 봇에서 사용

2. **하드코딩** (GUI, main.js)
   - 일반적인 에러 메시지
   - 빠른 응답 필요 시

3. **서버 응답** (HTTP response body)
   - 백엔드/Python에서 반환한 원본 에러
   - 디버깅용

---

## 5. 디버깅

### 에러 로그 확인

```bash
# Discord 봇 로그
# stderr에 상세 로그 출력
[Discord] Command error: ECONNREFUSED

# GUI 로그 (DevTools Console)
console.error('[Main] Error executing command:', error.message);

# Python 모듈 로그
# lifecycle.py의 sys.stderr 출력
print(f"[Palworld] Error: {e}", file=sys.stderr)
```

### 에러 코드 추적

| 코드 | 의미 | 원인 |
|------|------|------|
| `ECONNREFUSED` | 연결 거부 | 데몬 미실행, 포트 오류 |
| `ETIMEDOUT` | 시간 초과 | 서버 무응답, 방화벽 |
| `ENOTFOUND` | 호스트 없음 | DNS 오류, 잘못된 주소 |
| `401` | 인증 필요 | 비밀번호 오류 |
| `404` | 찾을 수 없음 | 서버/플레이어 없음 |
| `500` | 서버 오류 | 내부 크래시 |
| `503` | 서비스 불가 | 서버 과부하, 미실행 |

---

## 6. 베스트 프랙티스

### ✅ 좋은 에러 메시지

```
❌ 팰월드 서버가 실행중이지 않습니다
   먼저 `!saba pw start` 명령으로 서버를 시작해주세요
```

- 무엇이 문제인지 명확함
- 해결 방법 제시
- 사용자 친화적인 언어

### ❌ 나쁜 에러 메시지

```
Error: HTTP 503
```

- 기술적 용어만 사용
- 원인 불명확
- 해결 방법 없음

---

## 7. 예제

### Palworld - 플레이어 킥

**성공 시**:
```
✅ 플레이어 'Player1'이 서버에서 퇴장되었습니다
```

**실패 시**:
```
❌ 플레이어를 찾을 수 없습니다
   닉네임 또는 SteamID를 확인해주세요
   현재 접속자: !saba pw players
```

### Minecraft - 날씨 변경

**서버 미실행**:
```
❌ 마인크래프트 서버가 실행중이지 않습니다
   먼저 서버를 시작해주세요: !saba mc start
```

**RCON 비활성화**:
```
❌ RCON이 비활성화되어 있습니다
   server.properties에서 enable-rcon=true로 설정해주세요
```

---

## 8. FAQ

**Q: 모듈별 에러 메시지를 영어로도 지원할 수 있나요?**

A: 네, `module.toml`에 언어별 섹션을 추가할 수 있습니다:

```toml
[errors.ko]
server_not_running = "서버가 실행중이지 않습니다"

[errors.en]
server_not_running = "Server is not running"
```

**Q: 에러 메시지에 변수를 넣을 수 있나요?**

A: Python lifecycle에서 포맷팅하여 반환하면 됩니다:

```python
return {
    "success": False,
    "message": f"플레이어 '{player_name}'을 찾을 수 없습니다"
}
```

**Q: 에러 메시지가 적용되지 않아요**

A: 체크리스트:
1. module.toml의 `[errors]` 섹션 확인
2. Discord 봇 재시작 (메타데이터 재로드)
3. 모듈 캐시 새로고침 (GUI → Reload Modules)

---

## 참고 파일

- [modules/palworld/module.toml](../modules/palworld/module.toml) - Palworld 에러 정의
- [modules/minecraft/module.toml](../modules/minecraft/module.toml) - Minecraft 에러 정의
- [discord_bot/index.js](../discord_bot/index.js) - Discord 봇 에러 처리
- [electron_gui/main.js](../electron_gui/main.js) - GUI 에러 처리
