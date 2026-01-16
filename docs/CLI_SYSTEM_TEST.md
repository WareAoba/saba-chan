# CLI 명령어 시스템 테스트 가이드

## 아키텍처 검증

### 1. GUI 계층
✅ CommandModal 컴포넌트 생성 완료
- 명령어 드롭다운 선택
- 입력 필드 동적 생성
- 각 서버 카드에 💻 Command 버튼 추가

### 2. IPC 계층
✅ Electron IPC 핸들러 추가
- `executeCommand(id, command)` 구현
- Backend의 `/api/instance/:id/command` 호출

### 3. Backend 계층
✅ Rust API 엔드포인트 추가
- `POST /api/instance/{id}/command`
- `CommandRequest` 구조체 정의
- Supervisor.execute_command() 메서드 구현

### 4. 모듈 계층
✅ Palworld lifecycle.py 구현
- `command()` 함수 추가
- 5가지 명령어 핸들러:
  - say: 채팅 메시지 전송
  - broadcast: 공지 메시지
  - save: 월드 저장
  - info: 서버 정보
  - shutdown: 서버 종료

### 5. 설정 계층
✅ module.toml에 명령어 스키마 정의
- 명령어별 설명
- 입력 필드 정의
- 파라미터 타입 명시

---

## 엔드투엔드 플로우

### 요청 흐름
```
1. GUI: 사용자가 Command 버튼 클릭
   ↓
2. CommandModal: 명령어 선택 후 실행
   ↓
3. Electron IPC: executeCommand() 호출
   ↓
4. main.js: instance:executeCommand 핸들러
   ↓
5. Backend API: POST /api/instance/{id}/command
   ↓
6. Supervisor: execute_command() 메서드
   ↓
7. Plugin Runner: lifecycle.py command() 호출
   ↓
8. Palworld Module: 명령어 처리 및 RCON 전송
   ↓
9. 응답: 결과를 모달로 표시
```

---

## 테스트 시나리오

### 테스트 전 준비
1. Core Daemon 실행: `.\target\debug\core_daemon.exe`
2. GUI 시작: `npm start` (electron_gui/)

### 테스트 1: 명령어 UI 확인
- [ ] 서버 카드에 💻 Command 버튼 표시
- [ ] 서버가 running일 때만 활성화
- [ ] 클릭하면 CommandModal 표시

### 테스트 2: 명령어 선택
- [ ] 드롭다운에 5가지 명령어 표시
  - say
  - broadcast
  - save
  - info
  - shutdown
- [ ] 명령어 설명 표시
- [ ] 입력 필드 동적 생성

### 테스트 3: 명령어 실행
- [ ] say 명령어: 메시지 입력 후 실행
  - 예상: "Message broadcasted: {text}"
- [ ] broadcast 명령어: 메시지 입력 후 실행
  - 예상: "Notice broadcasted: {text}"
- [ ] save 명령어: 파라미터 없음
  - 예상: "World save initiated"
- [ ] info 명령어: 파라미터 없음
  - 예상: "Server info: Palworld running normally"
- [ ] shutdown 명령어: 초 입력 후 실행
  - 예상: "Server will shutdown in {N} seconds"

### 테스트 4: 에러 처리
- [ ] 필수 파라미터 누락시 오류 모달 표시
- [ ] 잘못된 명령어 오류 처리
- [ ] 네트워크 오류 처리

---

## 다음 단계

1. **실제 RCON 연결**: lifecycle.py에서 RCON 라이브러리 추가
   - mcrcon 또는 custom RCON 구현
   - 팰월드 RCON 포트 설정 (기본: 25575)

2. **명령어 응답 파싱**: 서버에서 반환하는 응답 수집
   - Info 명령어: 서버 통계 추출
   - Status 명령어: 플레이어 목록 등

3. **Minecraft 모듈**: 같은 패턴으로 구현
   - say, save, whitelist 등

4. **커스텀 명령어**: 사용자가 module.toml에 추가 가능하도록
   - 동적 필드 생성
   - 커스텀 RCON 명령어

---

## 파일 변경사항 요약

### Backend (Rust)
- `src/ipc/mod.rs`: POST /api/instance/:id/command 엔드포인트 추가
- `src/supervisor/mod.rs`: execute_command() 메서드 추가
- `src/supervisor/process.rs`: ProcessManager 구조체 추가

### Frontend (React)
- `electron_gui/src/CommandModal.js`: 명령어 입력 컴포넌트 추가
- `electron_gui/src/CommandModal.css`: 스타일시트 추가
- `electron_gui/src/App.js`: CommandModal 렌더링 추가
- `electron_gui/main.js`: instance:executeCommand 핸들러 추가
- `electron_gui/preload.js`: executeCommand API 추가

### 모듈 (Python)
- `modules/palworld/lifecycle.py`: command() 함수 추가
- `modules/palworld/module.toml`: [commands] 섹션 추가

---

## 아키텍처 다이어그램

```
┌─────────────────┐
│   React GUI     │ (CommandModal)
│  + Say Button   │
└────────┬────────┘
         │ executeCommand()
         ↓
┌─────────────────┐
│  Electron IPC   │ (main.js)
│  + preload.js   │
└────────┬────────┘
         │ POST /api/instance/:id/command
         ↓
┌─────────────────┐
│  Rust Backend   │ (Axum)
│  + IPC Server   │
└────────┬────────┘
         │ supervisor.execute_command()
         ↓
┌─────────────────┐
│  Python Module  │ (lifecycle.py)
│  + command()    │
└────────┬────────┘
         │ RCON send()
         ↓
┌─────────────────┐
│  Game Server    │ (Palworld)
│  + RCON Listen  │
└─────────────────┘
```
