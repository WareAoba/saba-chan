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

## ✅ 최근 해결된 문제 (2026-01-16)

### 1. stop_server_handler 하드코딩 해결 ✅
- **문제**: 모듈명이 "minecraft"로 하드코딩되어 다른 게임 서버 중지 불가
- **해결**: instance에서 module_name을 조회하여 동적으로 사용
- **파일**: `src/ipc/mod.rs`

### 2. start_server payload 구조 통일 ✅
- **문제**: App.js는 `{ module }`, Backend는 `payload.resource.module` 파싱
- **해결**: `ServerStartRequest` 구조 간소화
  ```rust
  pub struct ServerStartRequest {
      pub module: String,
      pub config: Value,
  }
  ```
- **파일**: `src/ipc/mod.rs`, `electron_gui/main.js`

### 3. get_server_status 하드코딩 해결 ✅
- **문제**: 모듈명이 "minecraft"로 하드코딩
- **해결**: instance에서 module_name 조회
- **파일**: `src/ipc/mod.rs`

### 4. 프로세스 간 통신 표준화 완료 ✅
- **추가**: `COMMUNICATION_SPEC.md` 생성
- **내용**: 모든 API 엔드포인트, 요청/응답 구조, IPC 브릿지 명세
- **효과**: 일관된 통신 구조로 유지보수성 향상

### 5. lifecycle.py 서버 시작 기능 구현 ✅
- **문제**: 서버 시작 시 "파일을 찾을 수 없습니다" 에러
- **해결**:
  - instance의 `executable_path`, `working_dir` 정보를 config로 전달
  - lifecycle.py에서 경로 검증 및 명확한 에러 메시지 제공
  - Windows 프로세스 생성 플래그 추가 (DETACHED_PROCESS)
  - Python 명령어 자동 감지 (python3 우선, python 폴백)
- **파일**: `src/supervisor/mod.rs`, `src/plugin/mod.rs`, `modules/*/lifecycle.py`

### 6. 서버 중지 기능 개선 ✅
- **개선**: Windows taskkill, Unix kill 명령어 분기 처리
- **추가**: force 옵션 지원 (`/F` 플래그)
- **파일**: `modules/*/lifecycle.py`

### 7. 사용자 가이드 작성 ✅
- **추가**: `USAGE_GUIDE.md` 생성
- **내용**: 서버 시작 전 설정 방법, 에러 해결, instances.json 편집 가이드

---

## ✅ 최근 해결된 문제 (2026-01-19)

### REST 명령어 시스템 완전 구현 ✅
- **문제**: REST 명령어가 "성공"이라고 표시되지만 실제로 서버에 실행되지 않음
- **원인**: HTTP 클라이언트가 스텁 코드였음 (실제 HTTP 요청 안 함)
- **해결**: 
  - ureq 기반 실제 HTTP 클라이언트 구현
  - Basic Auth 지원
  - `response_text` 필드 추가로 서버 응답 표시

### ModuleInfo commands 필드 누락 수정 ✅
- **문제**: GUI에서 `commandMetadata`가 undefined로 전달됨
- **원인**: `ModuleInfo` 구조체에 `commands` 필드가 없었음
- **해결**: 
  - `src/ipc/mod.rs`의 `ModuleInfo`에 `commands: Option<ModuleCommands>` 추가
  - `list_modules`, `refresh_modules` 함수에서 commands 매핑 추가
- **파일**: `src/ipc/mod.rs`

### module.toml 명령어 정의 체계화 ✅
- **문제**: 명령어 스펙이 JavaScript에 하드코딩되어 있었음
- **해결**: `module.toml`에 완전한 명령어 정의 추가
  - `http_method`: GET/POST 구분
  - `inputs`: 명령어 파라미터 스키마 정의
  - `endpoint_template`: REST 엔드포인트 패턴
- **파일**: `modules/palworld/module.toml`

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

3. **GUI에서 instance 경로 설정**
   - 현재는 instances.json 직접 편집 필요
   - executable_path, working_dir 입력 UI 추가 필요

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

## 🔴 잠재적 문제점 및 개선 필요 사항

> ⚠️ **참고**: 아래 항목들은 당장 해결이 필요한 것이 아닙니다. 코드 리뷰 중 발견된 사항을 기록해둔 것으로, 필요시 참고하세요.

### 🟢 최근 해결됨 (2026-01-16)

#### ~~1. stop_server_handler 모듈명 하드코딩~~ ✅
- **해결**: instance에서 module_name 조회하도록 수정

#### ~~2. Electron-Backend 간 payload 불일치~~ ✅
- **해결**: `ServerStartRequest` 구조 간소화 (`resource` 제거, `module`과 `config` 직접 사용)

#### ~~3. get_server_status 모듈명 하드코딩~~ ✅
- **해결**: instance에서 module_name 조회하도록 수정

### 🟡 중간 우선순위 (안정성/완성도)

#### 4. Daemon 시작 타이밍 문제
- **파일**: `electron_gui/main.js` (141-145줄)
- **문제**: `setTimeout(createWindow, 3000)` - 고정 타임아웃
- **개선**: Health check 폴링으로 대체 권장

#### 5. 프로세스 추적 기능 미완성
- **파일**: `src/supervisor/process.rs`
- **증상**: 대부분 메서드가 `#[allow(dead_code)]`
- **의미**: ProcessTracker 기능이 실제로 활용되지 않음

#### 6. State Machine 미사용
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

### 🟢 낮은 우선순위 (코드 품질)

#### 12. 미사용 경고
- **파일**: 여러 파일
- **증상**: unused imports, dead_code 경고
- **상태**: 2026-01-16 일부 정리 완료 (Path, ServerInstance import 제거)

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
| 2026-01-16 | **치명적 이슈 3개 해결** |
| 2026-01-16 | stop_server/get_status 하드코딩 제거 |
| 2026-01-16 | ServerStartRequest payload 구조 간소화 |
| 2026-01-16 | COMMUNICATION_SPEC.md 생성 (프로세스 간 통신 표준화) |
| 2026-01-16 | 미사용 import 정리 (Path, ServerInstance) |
| 2026-01-16 | **서버 시작/중지 기능 완전 구현** ✅ |
| 2026-01-16 | lifecycle.py 개선 (경로 검증, 에러 메시지, Windows 프로세스 처리) |
| 2026-01-16 | Python 명령어 자동 감지 (python3 우선) |
| 2026-01-16 | USAGE_GUIDE.md 생성 (사용자 가이드) |
| 2026-01-16 | 아키텍처 정정: Add Server는 간단한 3필드만 (이름, 모듈, 실행파일) |
| 2026-01-16 | 게임별 상세 설정은 별도 "Settings" 화면에서 관리 (미구현) |
| 2026-01-16 | **모듈 기반 실행파일 경로 자동 로드** |
| 2026-01-16 | module.toml에 executable_path 필드 추가 |
| 2026-01-16 | 모듈 선택 시 executable_path 자동으로 폼에 채워짐 |

---

## 🔗 참고 자료

- Palworld RCON Bot: https://github.com/juunini/palworld-discord-bot
- Palworld 기본 경로: `D:\SteamLibrary\steamapps\common\PalServer`
- Palworld 프로세스: `PalServer-Win64-Shipping-Cmd`
