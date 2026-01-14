# 🐟 Saba-chan - 프로젝트 지침서

> 이 문서는 AI 인스턴스가 프로젝트 컨텍스트를 빠르게 파악하고 작업을 이어갈 수 있도록 작성되었습니다.

## 📋 프로젝트 개요

**프로젝트명**: Saba-chan (サバちゃん)

**목적**: 여러 게임 서버(Palworld, Minecraft 등)를 통합 관리하는 모듈형 플랫폼

**핵심 개념**: 이 플랫폼은 게임 서버를 **직접 실행하지 않고**, 이미 실행 중인 서버 프로세스를 **감지하고 관리**합니다.

### 아키텍처
```
┌─────────────────┐     HTTP/IPC      ┌──────────────────┐
│  Electron GUI   │ ◄───────────────► │   Core Daemon    │
│  (React 18)     │   127.0.0.1:57474 │   (Rust/Axum)    │
└─────────────────┘                   └────────┬─────────┘
                                               │
                                    ┌──────────┴──────────┐
                                    │                     │
                              ┌─────▼─────┐        ┌──────▼──────┐
                              │  Modules  │        │  Instances  │
                              │ (ZIP/폴더) │        │ (JSON 저장) │
                              └───────────┘        └─────────────┘
```

---

## 📁 프로젝트 구조

```
c:\Git\Bot\
├── src/                          # Rust Core Daemon 소스
│   ├── main.rs                   # 진입점, 백그라운드 모니터링 태스크
│   ├── ipc/mod.rs                # Axum HTTP API 서버
│   ├── supervisor/
│   │   ├── mod.rs                # Supervisor (모듈 로더, 인스턴스 스토어, 프로세스 모니터링)
│   │   ├── process.rs            # ProcessTracker (PID 추적)
│   │   └── module_loader.rs      # 모듈 로딩 (ZIP + 폴더)
│   ├── instance/mod.rs           # ServerInstance 데이터 모델, InstanceStore
│   ├── process_monitor.rs        # Windows 프로세스 감지 (PowerShell 사용)
│   └── config.rs                 # GlobalConfig
├── modules/                      # 게임별 모듈
│   ├── palworld/
│   │   ├── module.toml           # 모듈 메타데이터
│   │   └── lifecycle.py          # 라이프사이클 스크립트 (미사용)
│   └── minecraft/
│       ├── module.toml
│       └── lifecycle.py
├── electron_gui/                 # Electron + React GUI
│   ├── main.js                   # Electron 메인 프로세스
│   ├── preload.js                # IPC API 노출
│   └── src/
│       ├── App.js                # React 메인 컴포넌트
│       └── App.css               # 스타일
├── instances.json                # 사용자 인스턴스 저장 (UTF-8, BOM 없이!)
└── PROJECT_GUIDE.md              # 이 파일
```

---

## ✅ 완료된 작업

### Core Daemon (Rust)
- [x] Axum HTTP 서버 (`127.0.0.1:57474`)
- [x] ProcessTracker - PID 추적 및 상태 관리
- [x] ModuleLoader - ZIP 및 폴더 모듈 로딩
- [x] InstanceStore - 인스턴스 CRUD 및 JSON 저장
- [x] ProcessMonitor - **PowerShell 기반** 프로세스 감지 (WMIC 사용 불가 환경 대응)
- [x] 백그라운드 모니터링 태스크 (2초 주기)
- [x] 프로세스 종료 시 tracker에서 자동 제거 (중복 로그 방지)

### API 엔드포인트
| Method | Endpoint | 설명 |
|--------|----------|------|
| GET | `/api/servers` | 인스턴스 목록 + 실행 상태 |
| GET | `/api/modules` | 사용 가능한 모듈 목록 |
| POST | `/api/instances` | 새 인스턴스 생성 |
| DELETE | `/api/instance/:id` | 인스턴스 삭제 |
| GET | `/api/server/:name/status` | 서버 상태 조회 |
| POST | `/api/server/:name/start` | 서버 시작 (미구현) |
| POST | `/api/server/:name/stop` | 서버 중지 (미구현) |

### Electron GUI
- [x] React 18 기반 UI
- [x] 서버 목록 표시 (running/stopped 뱃지)
- [x] 자동 새로고침 (2초 주기, 토글 가능)
- [x] 인스턴스 추가/삭제
- [x] 모듈 선택 시 자동 이름 생성 (`my-{모듈명}-{번호}`)
- [x] 설정 저장 (`%APPDATA%\game-server-gui\settings.json`)
- [x] 윈도우 크기 저장/복원
- [x] DevTools 디버깅 지원

### 모듈 시스템
- [x] `module.toml` 파싱
- [x] `process_name` 필드로 프로세스 자동 감지
- [x] Palworld 모듈: `PalServer-Win64-Shipping-Cmd`
- [x] Minecraft 모듈: `java` (구성 필요)

---

## 🔧 해결한 문제들

### 1. IPv6 연결 실패
- **증상**: `localhost:57474` 연결 거부
- **원인**: Node.js가 IPv6 (`::1`)로 먼저 시도
- **해결**: `127.0.0.1:57474`로 명시적 지정

### 2. WMIC 사용 불가
- **증상**: 프로세스 감지 실패
- **원인**: Windows에서 WMIC가 PATH에 없거나 미설치
- **해결**: PowerShell `Get-Process` 명령어로 대체
  ```rust
  // process_monitor.rs
  Command::new("powershell")
      .args(&["-NoProfile", "-Command", 
              "Get-Process | Select-Object Id,ProcessName,Path | ConvertTo-Csv -NoTypeInformation"])
  ```

### 3. instances.json UTF-8 BOM
- **증상**: `expected value at line 1 column 1` 파싱 오류
- **원인**: PowerShell이 UTF-8 BOM을 추가
- **해결**: BOM 없이 저장
  ```powershell
  [System.IO.File]::WriteAllText("path", "content", [System.Text.UTF8Encoding]::new($false))
  ```

### 4. 프로세스 이름 불일치
- **증상**: Palworld 서버가 항상 stopped
- **원인**: `PalServer` vs 실제 `PalServer-Win64-Shipping-Cmd`
- **해결**: `module.toml`의 `process_name` 필드 수정

### 5. 프로세스 종료 로그 무한 반복
- **증상**: "is no longer running" 메시지 무한 출력
- **해결**: tracker에서 제거하여 한 번만 출력

---

## ❌ 미구현 / 해결 못한 내용

### 높은 우선순위
1. **Command Input UI**
   - 서버에 명령어 전송하는 입력창
   - RCON 또는 stdin 방식

2. **RCON 통신 구현**
   - Palworld RCON 포트: 25575 (기본)
   - 참고: https://github.com/juunini/palworld-discord-bot
   - 주의: Palworld RCON은 Non-ASCII 미지원

3. **서버 시작/중지 기능**
   - 현재 API만 존재, 실제 구현 없음
   - `lifecycle.py` 스크립트 실행 필요

### 중간 우선순위
4. **다중 서버 동일 모듈**
   - 같은 모듈로 여러 인스턴스 구분 (포트 기반?)

5. **서버 로그 스트리밍**
   - 실시간 로그 출력 (WebSocket?)

6. **알림 시스템**
   - 서버 크래시 감지 시 알림

### 낮은 우선순위
7. **모듈 마켓플레이스**
   - 온라인 모듈 다운로드

8. **원격 접속**
   - 외부 네트워크에서 접속

---

## � 잠재적 문제점 및 개선 필요 사항

> ⚠️ **참고**: 아래 항목들은 당장 해결이 필요한 것이 아닙니다. 코드 리뷰 중 발견된 사항을 기록해둔 것으로, 필요시 참고하세요.

### 🔴 높은 우선순위 (기능에 영향)

#### 1. stop_server_handler 모듈명 하드코딩
- **파일**: `src/ipc/mod.rs` (190-207줄)
- **문제**: `let module_name = "minecraft";`로 하드코딩됨
- **영향**: palworld 등 다른 서버 중지 불가
- **해결방향**: 인스턴스에서 module_name 조회 필요

#### 2. Electron-Backend 간 payload 불일치
- **파일**: `electron_gui/main.js` (162-170줄), `electron_gui/src/App.js` (113줄)
- **문제**: 
  - App.js: `{ module }` 전송
  - main.js: `options.resource` 기대
  - Backend: `payload.resource.module` 찾음
- **영향**: 서버 시작 기능 작동 안 함

### 🟡 중간 우선순위 (안정성/완성도)

#### 3. Daemon 시작 타이밍 문제
- **파일**: `electron_gui/main.js` (141-145줄)
- **문제**: `setTimeout(createWindow, 3000)` - 고정 타임아웃
- **개선**: Health check 폴링으로 대체 권장

#### 4. 프로세스 추적 기능 미완성
- **파일**: `src/supervisor/process.rs`
- **증상**: 대부분 메서드가 `#[allow(dead_code)]`
- **의미**: ProcessTracker 기능이 실제로 활용되지 않음

#### 5. State Machine 미사용
- **파일**: `src/supervisor/state_machine.rs`
- **증상**: `State::Crashed` variant 미생성 경고
- **의미**: 상태 머신이 정의만 되고 통합 안 됨

#### 6. 미사용 설정 필드
- **파일**: `src/config/mod.rs`
- **필드**: `GlobalConfig.ipc_socket` - never read 경고

### 🟢 낮은 우선순위 (코드 품질)

#### 7. 에러 핸들링 불일치
- **파일**: `electron_gui/src/App.js`
- **문제**: `alert()` vs `console.error()` 혼용
- **개선**: 일관된 에러 표시 방식 필요

#### 8. 에러 타입 혼용
- **문제**: `anyhow::Result`와 `ProcessError` 혼용
- **개선**: 일관된 에러 처리 전략 수립

#### 9. 동기화 설계 불일치
- **파일**: `src/supervisor/process.rs`
- **문제**: `mark_crashed(&mut self)` vs 내부 `Mutex` 사용
- **개선**: `&self`로 통일하거나 설계 재검토

#### 10. 의미 없는 테스트
- **파일**: `src/ipc/mod.rs` (310-320줄)
- **문제**: 하드코딩된 JSON만 검증
- **개선**: 실제 기능을 테스트하는 integration test 필요

#### 11. create_instance 검증 부족
- **파일**: `src/ipc/mod.rs` (255-288줄)
- **개선**: 중복 ID 체크, 유효 모듈 검증 추가

### 참고: 보안 관련
- IPC 서버 `127.0.0.1` 바인딩으로 로컬 전용 - 현재는 OK
- 인증/권한 검사 없음 - 원격 접속 구현 시 필요

---

## �📝 중요한 코드 위치

### 프로세스 감지 로직
```
src/supervisor/mod.rs > monitor_processes()
src/process_monitor.rs > ProcessMonitor::find_by_name()
```

### 인스턴스 생성 시 process_name 자동 설정
```
src/ipc/mod.rs > create_instance() 
  → module metadata에서 process_name 가져옴
```

### GUI 자동 새로고침
```
electron_gui/src/App.js > useEffect with setInterval
```

### 설정 저장 경로
```
Windows: %APPDATA%\game-server-gui\settings.json
코드: electron_gui/main.js > getSettingsPath()
```

---

## 🚀 개발 환경 실행 방법

### 1. Core Daemon 빌드 및 실행
```powershell
cd c:\Git\Bot
cargo build --release
# GUI가 자동으로 실행하므로 별도 실행 불필요
```

### 2. GUI 실행
```powershell
cd c:\Git\Bot\electron_gui
npm start
```
- React 개발 서버: http://localhost:3000
- Electron이 3초 후 자동으로 창 열림
- Core Daemon도 자동 시작됨

### 3. API 직접 테스트
```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" | ConvertTo-Json -Depth 5
```

---

## ⚠️ 주의사항

1. **instances.json**은 반드시 **UTF-8 without BOM**으로 저장
2. **process_name**은 실제 프로세스 이름과 **정확히** 일치해야 함 (확장자 제외)
3. PowerShell에서 파일 저장 시 `-Encoding UTF8` 사용하면 BOM 추가됨 - 주의!
4. Core Daemon은 GUI가 자동 시작/종료함

---

## 📅 변경 이력

| 날짜 | 변경 내용 |
|------|----------|
| 2026-01-14 | 프로젝트 초기 구현 |
| 2026-01-14 | WMIC → PowerShell 전환 |
| 2026-01-14 | 프로세스 종료 감지 개선 |
| 2026-01-14 | 설정 저장 기능 추가 |
| 2026-01-15 | PROJECT_GUIDE.md 생성 |
| 2026-01-15 | 잠재적 문제점 및 개선 필요 사항 섹션 추가 |

---

## 🔗 참고 자료

- Palworld RCON Bot: https://github.com/juunini/palworld-discord-bot
- Palworld 기본 경로: `D:\SteamLibrary\steamapps\common\PalServer`
- Palworld 프로세스: `PalServer-Win64-Shipping-Cmd`
