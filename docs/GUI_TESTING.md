# GUI 테스트 가이드

## 개요
이제 Electron GUI에서 게임 서버 명령어를 실행할 수 있습니다.

## 시스템 아키텍처

```
┌─────────────────────────────────────────┐
│  Electron GUI (React)                   │
│  ├── CommandModal.js                    │
│  │   └── 명령어 입력 UI                 │
│  └── main.js (IPC handlers)             │
│      └── 프로토콜 라우팅                 │
└──────────────┬──────────────────────────┘
               │
               │ IPC + HTTP
               ▼
┌─────────────────────────────────────────┐
│  Core Daemon (Rust)                     │
│  ├── IPC 서버 (127.0.0.1:57474)        │
│  ├── RCON 클라이언트 (Minecraft)        │
│  ├── REST 클라이언트 (Palworld)         │
│  └── Protocol clients                   │
└──────────────┬──────────────────────────┘
               │
        ┌──────┴──────────┐
        │                 │
        ▼                 ▼
    ┌─────────────┐  ┌─────────────┐
    │  Minecraft  │  │  Palworld   │
    │  Server     │  │  Server     │
    │  (RCON)     │  │  (REST)     │
    └─────────────┘  └─────────────┘
```

## 프로토콜 라우팅

### Minecraft (RCON 권장)
- **엔드포인트**: `POST /api/instance/:id/rcon`
- **명령어**: `say`, `give`, `save-all`, `list`, `weather`, `difficulty`
- **예시**:
  ```json
  {
    "command": "say",
    "args": { "message": "Hello Minecraft!" }
  }
  ```

### Palworld (REST API 권장)
- **엔드포인트**: `POST /api/instance/:id/rest`
- **명령어**: `announce`, `kick`, `ban`, `info`, `players`, `metrics`
- **예시**:
  ```json
  {
    "endpoint": "/api/announce",
    "method": "POST",
    "body": { "message": "Hello Palworld!" }
  }
  ```

## GUI 사용법

### 1. 서버 인스턴스 생성

1. GUI 실행: `npm start` (electron_gui 폴더에서)
2. "서버 관리" 탭
3. "+" 버튼으로 새 인스턴스 생성
4. 다음 정보 입력:
   - **이름**: 서버 이름 (예: "메인 마크")
   - **모듈**: "minecraft" 또는 "palworld"
   - **실행파일 경로**: 서버 실행 파일 경로
   - **포트**: 서버 포트 (예: Minecraft 25565, Palworld 8211)

### 2. 프로토콜 설정

#### Minecraft 인스턴스
```json
{
  "rcon_port": 25575,
  "rcon_password": "your_password"
}
```

#### Palworld 인스턴스
```json
{
  "rest_host": "127.0.0.1",
  "rest_port": 8212,
  "rest_username": "admin",
  "rest_password": "password"
}
```

### 3. 명령어 실행

1. 인스턴스 선택
2. "명령어" 버튼 클릭
3. 명령어 입력 (자동완성 지원)
4. 파라미터 입력 (필요시)
5. "⏎ 실행" 버튼 클릭

## 테스트 시나리오

### 테스트 1: Minecraft RCON
```bash
# 1. 로컬 Minecraft 서버 실행 (RCON 활성화)
# server.properties에서:
# - enable-rcon=true
# - rcon.port=25575
# - rcon.password=test123

# 2. GUI에서 인스턴스 생성
#    - 모듈: minecraft
#    - RCON 포트: 25575
#    - RCON 비밀번호: test123

# 3. 명령어 실행
#    - "say Hello from GUI!"
#    - 예상 결과: 서버에 메시지 출력
```

### 테스트 2: Palworld REST API
```bash
# 1. 로컬 Palworld 서버 실행 (REST API 활성화)
# PalWorldSettings.ini에서:
# AdminPassword=test123
# PublicPort=8211

# 2. GUI에서 인스턴스 생성
#    - 모듈: palworld
#    - REST 호스트: 127.0.0.1
#    - REST 포트: 8212
#    - REST 사용자명: admin
#    - REST 비밀번호: test123

# 3. 명령어 실행
#    - "announce Hello from GUI!"
#    - 예상 결과: 서버에 공지 출력
```

## 명령어 매핑

### Minecraft 명령어

| 명령어 | 파라미터 | 설명 |
|--------|---------|------|
| `say` | message | 서버 채팅 메시지 |
| `give` | player, item, amount | 플레이어에게 아이템 제공 |
| `save-all` | - | 서버 데이터 저장 |
| `list` | - | 온라인 플레이어 목록 |
| `weather` | type, duration | 날씨 변경 (clear, rain, thunder) |
| `difficulty` | level | 난이도 변경 (peaceful, easy, normal, hard) |

### Palworld 명령어

| 명령어 | 파라미터 | 설명 |
|--------|---------|------|
| `announce` | message | 서버 공지사항 |
| `kick` | userid, message | 플레이어 강퇴 |
| `ban` | userid, message | 플레이어 밴 |
| `unban` | userid | 플레이어 언밴 |
| `info` | - | 서버 정보 조회 |
| `players` | - | 플레이어 목록 조회 |
| `metrics` | - | 서버 통계 조회 |
| `shutdown` | seconds | 서버 종료 예약 |

## 에러 처리

### 연결 실패
```
[Daemon connection error: Failed to connect to daemon]
```
**해결책**: 
- 데몬이 실행 중인지 확인
- 포트 57474가 사용 중인지 확인
- 방화벽 설정 확인

### 인증 실패
```
[RCON/REST authentication failed]
```
**해결책**:
- RCON 비밀번호 확인 (Minecraft)
- REST API 자격증명 확인 (Palworld)

### 타임아웃
```
[Command execution timeout]
```
**해결책**:
- 서버 연결 상태 확인
- 네트워크 연결 확인
- 서버 로그 확인

## 디버깅

### 로그 확인

1. **Electron 콘솔**: F12 (개발자 도구)
2. **Daemon 로그**: 데몬 시작 시 stderr 출력
3. **모듈 로그**: `./logs/` 디렉토리

### 문제 진단

```javascript
// Browser Console (F12)에서 실행:
await window.api.daemonStatus()  // 데몬 상태 확인
await window.api.serverList()    // 서버 목록 확인
await window.api.instanceList()  // 인스턴스 목록 확인
```

## 다음 단계

1. ✅ 기본 GUI 통합 완료
2. ⏳ 실제 게임 서버에서 테스트
3. ⏳ 응답 결과 표시 개선
4. ⏳ 명령어 히스토리 추가
5. ⏳ 서버 콘솔 로그 스트리밍

## 참고

- 데몬 API 문서: [docs/PROTOCOL_CLIENT_DESIGN.md](PROTOCOL_CLIENT_DESIGN.md)
- 프로토콜 클라이언트: [src/protocol/](../src/protocol/)
- 모듈 API 호출: [modules/](../modules/)
