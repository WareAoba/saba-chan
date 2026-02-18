# saba-chan 코드베이스 개선 보고서

> **작성 기준**: 전체 소스 코드 정밀 리뷰 기반  
> **대상 버전**: v0.1.0  
> **최종 업데이트**: 2026-02-16 — Phase 1~3 완료, Phase 4 거의 완료 (테스트 추가, 보안 개선, 코드 중복 해소)  
> **컨텍스트**: 이 프로젝트는 전적으로 AI 에이전트(바이브 코딩)로 개발되었으며, 향후에도 동일한 방식으로 유지보수될 예정입니다. 따라서 각 개선 항목에는 AI 에이전트에게 지시할 때 활용할 수 있도록 **구체적인 액션 아이템**을 포함합니다.

---

## 목차

1. [아키텍처 현황 요약](#1-아키텍처-현황-요약)
2. [긴급 (P0): 즉시 수정 필요](#2-긴급-p0-즉시-수정-필요)
3. [높음 (P1): 다음 마일스톤 전 해결](#3-높음-p1-다음-마일스톤-전-해결)
4. [중간 (P2): 장기적 품질 향상](#4-중간-p2-장기적-품질-향상)
5. [낮음 (P3): 나이스 투 해브](#5-낮음-p3-나이스-투-해브)
6. [데드 코드 및 스텁 정리](#6-데드-코드-및-스텁-정리)
7. [코드 중복 제거](#7-코드-중복-제거)
8. [테스트 전략](#8-테스트-전략)
9. [보안 점검](#9-보안-점검)
10. [바이브 코딩 특화 가이드라인](#10-바이브-코딩-특화-가이드라인)

---

## 1. 아키텍처 현황 요약

### 컴포넌트별 파일 크기 히트맵

| 컴포넌트 | 파일 | 줄 수 | 위험도 |
|---|---|---:|:---:|
| **Core Daemon** | `src/ipc/mod.rs` | ~~1,913~~ 546 | ✅ 분할 완료 |
| **Core Daemon** | `src/supervisor/mod.rs` | 867 | 🟡 |
| **Core Daemon** | `src/supervisor/module_loader.rs` | 657 | 🟢 |
| **Core Daemon** | `src/ipc/updates.rs` | 488 | 🟢 |
| **Core Daemon** | `src/supervisor/managed_process.rs` | ~~443~~ 471 | ✅ MC 전용 로직 제거 |
| **Core Daemon** | `src/supervisor/process.rs` | 315 | 🟢 |
| **Core Daemon** | `src/plugin/mod.rs` | ~~157~~ 120 | ✅ async 전환 |
| **Core Daemon** | `src/main.rs` | 256 | 🟢 |
| **GUI** | `saba-chan-gui/src/App.js` | ~~3,248~~ ~~2,589~~ 930 | ✅ 커스텀 훅 분할 완료 |
| **GUI** | `saba-chan-gui/src/components/UpdateModal.js` | 521 | 🟢 |
| **GUI** | `saba-chan-gui/src/components/UpdatePanel.js` | 504 | 🟢 |
| **CLI** | `saba-chan-cli/src/tui/screens/` | ~~1,326~~ 9파일 분할 | ✅ 분할 완료 |
| **CLI** | `saba-chan-cli/src/tui/commands.rs` | 990 | 🟡 |
| **Discord Bot** | `discord_bot/index.js` | 603 | 🟡 |
| **Module** | `modules/minecraft/lifecycle.py` | 1,766 | 🟡 |
| **Module** | `modules/palworld/lifecycle.py` | 1,707 | 🟡 |
| **Module Meta** | `modules/minecraft/module.toml` | 835 | 🟢 |
| **Module Meta** | `modules/palworld/module.toml` | 616 | 🟢 |

**범례**: 🔴 1500줄 이상 = 반드시 분할 / 🟡 500줄 이상 = 분할 권장 / 🟢 적정

### 스택 구성

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Electron    │  │  ratatui     │  │  discord.js  │
│  GUI (React) │  │  CLI (Rust)  │  │  Bot (Node)  │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │
       └────────┬────────┴────────┬────────┘
                │  REST API :57474│
         ┌──────┴──────┐
         │ Core Daemon │  ← Rust (tokio + axum)
         │  Supervisor │
         └──────┬──────┘
                │ subprocess (Python)
         ┌──────┴──────┐
         │   Modules   │  ← lifecycle.py + module.toml
         │  (MC, PW…)  │
         └─────────────┘
```

---

## 2. 긴급 (P0): 즉시 수정 필요

### 2.1 ✅ `src/ipc/mod.rs` — 1,913줄 모놀리식 API 핸들러

**문제**: 약 30개 이상의 REST 핸들러, 데이터 구조, 클라이언트 레지스트리, 라우터 설정이 전부 하나의 파일에 집적되어 있음. AI 에이전트가 핸들러를 추가하거나 수정할 때 컨텍스트 윈도우를 모두 소비하여 실수가 잦아질 구조.

**현재 구조**:
```
src/ipc/mod.rs       ← 1,913줄: 모든 것
src/ipc/updates.rs   ← 488줄: 업데이트 관련만 분리됨
```

**목표 구조**:
```
src/ipc/
  mod.rs              ← 라우터 조립 + IPCServer + ClientRegistry (~200줄)
  types.rs            ← ServerInfo, ModuleInfo, ModuleListResponse 등 공유 타입
  handlers/
    mod.rs            ← pub mod 선언
    server.rs         ← start/stop/status/list 핸들러 (~200줄)
    instance.rs       ← CRUD, properties, EULA, diagnose (~300줄)
    module.rs         ← module list/discovery (~100줄)
    command.rs        ← command execute, RCON, REST 프록시 (~200줄)
    managed.rs        ← managed process stdin/console (~150줄)
    client.rs         ← heartbeat, register/deregister (~100줄)
    bot.rs            ← bot config, kill bot (~100줄)
    install.rs        ← versions, install (~150줄)
  updates.rs          ← 기존 유지
```

**AI 에이전트 지시문 예시**:
> "src/ipc/mod.rs 파일을 위의 구조로 리팩터링해줘. 각 핸들러 함수의 시그니처(State<Arc<...>>)는 유지하고, mod.rs에서 Router를 조립할 때 각 모듈의 핸들러를 import해서 사용해."

**기대 효과**: 각 파일이 100~300줄로 축소되어 AI 에이전트의 컨텍스트 효율이 5배 이상 개선됨.

---

### 2.2 ✅ `saba-chan-gui/src/App.js` — ~~3,248줄~~ ~~2,589줄~~ 930줄 (2차 분할 완료)

**문제**: 전체 애플리케이션 로직—상태 관리, API 호출, 렌더링, 이벤트 핸들링—이 하나의 `App()` 함수에 포함. 이 파일의 크기 자체가 AI 에이전트의 단일 작업 범위를 초과함.

**1차 분할 (완료)**:
- `components/ServerCard.js` — 서버 카드 UI 컴포넌트 추출
- `components/ServerSettingsModal.js` — 서버 설정 모달 추출 (GeneralTab, AliasesTab, SettingsField 포함)
- `components/ConsoleView.js` — ConsolePanel + PopoutConsole 컴포넌트 추출
- `components/LoadingScreen.js` — 로딩 화면 컴포넌트 추출
- 미사용 함수 (`getStatusColor`, `getStatusIcon`) 제거
- **결과**: 3,248줄 → 2,589줄 (약 660줄 감소, 20% 축소)

**2차 분할 (완료)**: Context API 없이 커스텀 훅 패턴으로 로직 추출, 930줄로 축소
- `utils/helpers.js` — 순수 유틸리티 함수 (translateError, retryWithBackoff, waitForDaemon, safeShowToast, debugLog)
- `hooks/useWaitingImage.js` — 프로그레스 스톨 감지 + 대기 이미지
- `hooks/useConsole.js` — 콘솔 패널 상태/열기/닫기/전송/폴링/팝아웃
- `hooks/useDragReorder.js` — 서버 카드 드래그앤드롭 재정렬
- `hooks/useDiscordBot.js` — 봇 상태 폴링/시작/중지/자동시작/재런치
- `hooks/useServerActions.js` — 서버 CRUD (fetch/start/stop/status/add/delete)
- `hooks/useServerSettings.js` — 설정 모달/버전 설치/서버 리셋/별칭 관리
- **결과**: 2,589줄 → 930줄 (약 1,660줄 감소, 64% 추가 축소, 총 71% 축소)

---

### 2.3 ✅ `managed_process.rs`에 Minecraft 전용 로직 혼입

**문제**: `managed_process.rs:443줄`의 `parse_minecraft_log_level()` 함수가 게임 불문 공통 계층에 하드코딩되어 있음. 모듈 독립 원칙 위반.

**위치**: `src/supervisor/managed_process.rs` 내부

```rust
// 현재 코드 (문제)
fn parse_minecraft_log_level(line: &str) -> &str {
    // "[12:34:56 INFO]:" → "INFO" 추출
    // Minecraft 전용 로그 포맷이 코어에 존재
}
```

**해결 방안**:
1. `module.toml`에 `log_pattern` 필드 추가:
   ```toml
   [metadata]
   log_pattern = '^\[[\d:]+\s+(INFO|WARN|ERROR)\]'
   ```
2. `ManagedProcess`에서 모듈 메타데이터의 `log_pattern`을 regex로 컴파일하여 사용
3. 패턴 미지정 시 기본값(단순 출력)으로 폴백

---

## 3. 높음 (P1): 다음 마일스톤 전 해결

### 3.1 ✅ `StateMachine` — `#[allow(dead_code)]` 제거, TODO 주석으로 교체

**파일**: `src/supervisor/state_machine.rs` (90줄)

**현재 상태**: 모든 public 함수에 `#[allow(dead_code)]`가 붙어 있음. `Supervisor`는 이 상태 머신을 사용하지 않고, `is_running` 불리언 플래그와 프로세스 존재 여부로 상태를 판단하고 있음.

**선택지**:

| 옵션 | 설명 | 권장도 |
|---|---|:---:|
| A. 통합 | `Supervisor`의 서버 상태 추적을 `StateMachine`으로 교체 | ⭐⭐⭐ |
| B. 삭제 | 사용하지 않으므로 파일 제거 | ⭐⭐ |

**옵션 A 채택 시 구체 지시문**:
> "`Supervisor`에서 각 서버 인스턴스의 상태를 `StateMachine`으로 관리하도록 변경해줘. `HashMap<String, StateMachine>`을 추가하고, `start_server`에서 `Stopped→Starting→Running` 전이, `stop_server`에서 `Running→Stopping→Stopped` 전이를 호출해. API의 `get_server_status`에서 StateMachine의 상태를 직접 반환하도록 개선."

---

### 3.2 ✅ `PathDetector` 완전 미사용 → 삭제 완료

**파일**: `src/path_detector.rs` (95줄)

**현재 상태**: 모든 함수에 `#[allow(dead_code)]`. 서버 실행 파일 경로의 자동 탐지를 위해 작성되었으나, 실제로는 인스턴스 생성 시 사용자가 경로를 직접 지정하는 방식.

**권장**: `module.toml`의 `detection.common_paths` 목록과 연계하여 "서버 자동 탐지" 기능으로 활용하거나, 사용 계획이 없으면 삭제.

---

### 3.3 ✅ `ResourceLimit` — TODO 스텁 → 삭제 완료

**파일**: `src/resource/mod.rs` (52줄)

```rust
pub fn apply(&self, _pid: u32) -> Result<()> {
    // TODO: Use cgroups (Linux) or Job Objects (Windows)
    Ok(())
}
```

**현재 상태**: 구조체와 생성자만 있고 실제 구현이 전혀 없음. `module.toml`에는 `ram` 설정이 존재하지만 Core에서 리소스 제한을 적용하지 않음.

**선택지**:
- **구현**: Windows Job Object API (`winapi::um::jobapi2`) + Linux cgroups v2
- **정직하게 삭제**: 리소스 제한이 현재 불필요하면 파일 삭제 및 Cargo.toml에서 관련 참조 제거

---

### 3.4 ✅ `GlobalConfig` — 에러 처리 개선 완료

**파일**: `src/config/mod.rs`

```rust
pub fn load() -> anyhow::Result<Self> {
    let s = std::fs::read_to_string("config/global.toml").unwrap_or_default();
    let cfg: Self = toml::from_str(&s).unwrap_or(Self {
        ipc_socket: None, servers: None, updater: None,
    });
    Ok(cfg)
}
```

**문제**: 
1. `unwrap_or_default()`로 파일 읽기 실패를 무시 → 설정 파일이 없으면 아무 경고 없이 빈 설정으로 동작
2. TOML 파싱 실패도 무시 → 잘못된 설정을 작성해도 아무 에러 메시지 없음
3. 함수 시그니처가 `Result`를 반환하지만 실제로는 절대 `Err`를 반환하지 않음

**수정 방안**:
```rust
pub fn load() -> anyhow::Result<Self> {
    let path = "config/global.toml";
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let cfg: Self = toml::from_str(&s)
                .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path, e))?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("Config file {} not found, using defaults", path);
            Ok(Self { ipc_socket: None, servers: None, updater: None })
        }
        Err(e) => Err(anyhow::anyhow!("Failed to read {}: {}", path, e)),
    }
}
```

---

### 3.5 ✅ Python 모듈 인터페이스 표준화 — `modules/_template/` 생성 완료

**문제**: `lifecycle.py`의 함수 인터페이스가 암묵적 계약으로만 존재. `FUNCTIONS` 딕셔너리에 어떤 함수가 등록되어야 하는지 문서나 스키마가 없음.

**현재 계약 (암묵적)**:
```python
# 각 lifecycle.py는 다음 중 일부를 구현해야 함:
FUNCTIONS = {
    "start": start,            # (config) → {success, pid?, message}
    "stop": stop,              # (config) → {success, message}
    "status": status,          # (config) → {success, status, ...}
    "command": command,         # (config) → {success, message, data?}
    "validate": validate,       # (config) → {success, issues}
    "get_launch_command": ...,  # (config) → {success, program, args, working_dir}
    "configure": configure,     # (config) → {success, updated_keys}
    "read_properties": ...,     # (config) → {success, properties}
    "accept_eula": ...,         # (config) → {success, message}
    "diagnose_log": ...,        # (config) → {success, issues}
    "list_versions": ...,       # (config) → {success, versions}
    "install_server": ...,      # (config) → {success, install_path}
}
```

**해결 방안**: `modules/_template/` 디렉토리 생성
```
modules/
  _template/
    lifecycle.py     ← 모든 필수/선택 함수의 시그니처와 반환 스키마 정의
    module.toml      ← 최소 필수 필드가 채워진 템플릿
    README.md        ← 새 모듈 작성 가이드
```

**lifecycle.py 템플릿 핵심부**:
```python
"""
saba-chan Module Lifecycle Template

Required functions: validate, get_launch_command, status
Optional functions: start, stop, command, configure,
                    read_properties, accept_eula, diagnose_log,
                    list_versions, install_server, reset_server
                    
All functions receive a dict `config` and must return a dict 
with at minimum {"success": bool, "message": str}.
"""

FUNCTIONS = { ... }  # 등록된 함수만 Daemon이 호출
```

---

### 3.6 ✅ `Supervisor.stop_server()` — `force_kill_pid()` 헬퍼 추출 완료

**파일**: `src/supervisor/mod.rs`

**문제**: `stop_server()` 내부에 Managed 서버 종료와 Non-managed 서버 종료 경로가 있으며, 양쪽 모두 `#[cfg(target_os = "windows")]` 블록에서 거의 동일한 `taskkill /F /PID` 로직을 갖고 있음.

**수정 방안**: 공통 헬퍼 함수 추출
```rust
// src/supervisor/process.rs에 추가
pub fn force_kill_pid(pid: u32) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .status()?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsafe { libc::kill(pid as i32, libc::SIGKILL); }
    }
    Ok(())
}
```

---

## 4. 중간 (P2): 장기적 품질 향상

### 4.1 ✅ `module.toml` 스키마 검증 부재 → serde 강타입 전환 완료

**문제**: `module_loader.rs`가 TOML을 파싱할 때 필수 필드 누락, 타입 불일치 등을 정밀하게 검증하지 않음. 잘못된 `module.toml`을 작성하면 런타임에 `unwrap()` 패닉이나 묵시적 기본값으로 오동작할 수 있음.

**현재 파싱 코드 요약** (`module_loader.rs`):
```rust
let module_config: toml::Value = toml::from_str(&content)?;
// 이후 .get("key").and_then(|v| v.as_str()) 체이닝으로 수동 추출
```

**해결**: `serde`로 강타입 구조체 정의
```rust
#[derive(Deserialize)]
struct ModuleToml {
    metadata: Metadata,
    protocols: Option<Protocols>,
    detection: Option<Detection>,
    config: Option<Config>,
    settings: Option<Vec<SettingsGroup>>,
    commands: Option<Vec<CommandDef>>,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,          // 필수
    display_name: String,  // 필수
    version: String,       // 필수
    #[serde(default)]
    log_pattern: Option<String>,
}
```

이렇게 하면:
- 필수 필드 누락 시 명확한 에러 메시지 (`missing field 'name'`)
- 타입 불일치 시 컴파일 + 런타임 안전성
- AI 에이전트가 새 필드를 추가할 때 구조체만 수정하면 됨

---

### 4.2 ✅ IPC API 토큰 인증 추가 완료

**현재 상태**: `127.0.0.1:57474`에서 아무런 인증 없이 REST API가 노출. 같은 머신의 어떤 프로세스든 서버를 시작/중지/명령 실행 가능.

**위험도**: 현재는 로컬 전용이므로 낮지만, 향후 원격 접근을 추가할 경우 즉시 치명적.

**단기 해결**: 시작 시 랜덤 토큰 생성 → `X-Saba-Token` 헤더로 검증
```rust
// IPCServer 시작 시
let token = uuid::Uuid::new_v4().to_string();
std::fs::write("config/.ipc_token", &token)?;

// 미들웨어
async fn auth_middleware(req: Request, next: Next) -> Response {
    let token = req.headers().get("X-Saba-Token");
    // 검증...
}
```

GUI/CLI/Bot은 `config/.ipc_token` 파일을 읽어서 헤더에 포함.

---

### 4.3 ✅ 에러 처리 — `SupervisorError` 타입 정의 완료

**패턴별 분포**:

| 패턴 | 위치 | 문제 |
|---|---|---|
| `unwrap_or_default()` 침묵 | `config/mod.rs`, `module_loader.rs` | 에러가 삼켜져서 디버깅 불가 |
| `anyhow::Result` 무조건 사용 | Core Daemon 전역 | 에러 종류를 구분할 수 없음 |
| JSON `{"success": false}` | IPC 핸들러 | HTTP 상태 코드가 항상 200 |
| `#[allow(dead_code)]` 남발 | `config`, `resource`, `path_detector`, `state_machine` | 미구현 코드를 경고 없이 방치 |

**개선 방향**:
1. **커스텀 에러 타입 도입** (최소 `supervisor` 모듈):
   ```rust
   #[derive(thiserror::Error, Debug)]
   pub enum SupervisorError {
       #[error("Module '{0}' not found")]
       ModuleNotFound(String),
       #[error("Server '{0}' already running")]
       AlreadyRunning(String),
       #[error("Plugin execution failed: {0}")]
       PluginError(String),
       // ...
   }
   ```
2. **HTTP 상태 코드 활용**: 현재 모든 응답이 `Json(...)` with 200. 에러응답에 적절한 상태 코드 사용:
   - `404` → 인스턴스/모듈 미발견
   - `409` → 이미 실행 중 
   - `500` → 내부 에러

---

### 4.4 ✅ `plugin/mod.rs` — `tokio::process::Command` 비동기 전환 완료

**문제**: `run_plugin()`이 `async fn`이지만 내부에서 `Command::new().output()`으로 **동기 블로킹** 호출을 수행. tokio 런타임에서 스레드 풀 고갈 가능성.

```rust
pub async fn run_plugin(...) -> Result<Value> {
    // ⚠️ 동기 블로킹 호출
    let output = cmd.output()?;
}
```

**수정**:
```rust
pub async fn run_plugin(...) -> Result<Value> {
    let output = tokio::process::Command::new(python_cmd)
        .arg(module_path)
        .arg(function)
        .arg(&config_json)
        .output()
        .await?;  // ✅ 비동기
}
```

---

### 4.5 ✅ CLI `screens.rs` → 9개 파일 분할 완료

**파일**: `saba-chan-cli/src/tui/screens.rs`

**현재**: 모든 TUI 화면(대시보드, 설정, 인스턴스 상세, 콘솔, AI채팅 등)이 한 파일에 존재.

**목표**:
```
src/tui/
  screens/
    mod.rs
    dashboard.rs      ← 메인 대시보드
    instance_detail.rs ← 인스턴스 상세
    settings.rs        ← 앱 설정
    console.rs         ← 콘솔 로그 뷰
    install.rs         ← 서버 설치 마법사
```

---

### 4.6 ✅ Discord Bot `index.js` — 에러 핸들링 개선 완료

**파일**: `discord_bot/index.js` (603줄)

**문제점**:
1. `fetch()` 호출에 대한 timeout이 없음 — Daemon이 응답하지 않으면 봇이 행(hang)
2. 일부 에러 경로에서 Discord interaction reply가 누락 → 사용자에게 "이 상호작용이 실패했습니다" 메시지
3. `process.on('unhandledRejection')` 핸들러 없음

**수정 방안**:
```javascript
// 모든 fetch에 AbortController timeout 적용
const controller = new AbortController();
const timeout = setTimeout(() => controller.abort(), 10000);
try {
    const res = await fetch(url, { signal: controller.signal });
} finally {
    clearTimeout(timeout);
}

// 글로벌 에러 핸들러
process.on('unhandledRejection', (reason) => {
    console.error('[Bot] Unhandled rejection:', reason);
});
```

---

## 5. 낮음 (P3): 나이스 투 해브

### 5.1 ✅ i18n 하드코딩 잔여 → palworld lifecycle.py ~30개 문자열 전환 완료

일부 위치에서 영문 문자열이 직접 사용되고 있음:

| 파일 | 예시 |
|---|---|
| `palworld/lifecycle.py` validate() | `"Server executable path not specified."` |
| `palworld/lifecycle.py` install_server() | `"Palworld dedicated server must be installed via SteamCMD."` |
| `ipc/mod.rs` 일부 에러 | `"Module not found"` |

이들을 `i18n.t(...)` 호출로 교체하여 다국어 지원을 완성해야 함.

### 5.2 ✅ `lifecycle.py` — `key_map` 딕셔너리 중복 → `_PROPERTY_KEY_MAP` 상수 추출 완료

`modules/minecraft/lifecycle.py`의 `configure()` 함수와 `install_server()` 함수에 **동일한** 70줄짜리 `key_map` 딕셔너리가 복사-붙여넣기 되어 있음.

**수정**: 모듈 최상위에 `_KEY_MAP` 상수로 한 번만 정의.

### 5.3 ✅ Python RCON 클라이언트 중복 → 통합 완료

- `extensions/rcon.py` — 공용 `RconClient` 클래스 생성
- `modules/minecraft/lifecycle.py` — 인라인 `_send_rcon_command()` 제거, `extensions.rcon.rcon_command`로 위임
- `modules/palworld/lifecycle.py` — 160줄 `PalworldRconClient` 제거, `extensions.rcon.RconClient` 위임 래퍼로 교체
- `src/protocol/rcon.rs` — Rust RCON 클라이언트 (Core Daemon용, 별도 유지)

### 5.4 ✅ 로그 버퍼 고정 크기 → 설정 가능하게 변경 완료

- `managed_process.rs`: `MAX_LOG_LINES` → `DEFAULT_LOG_BUFFER: usize = 10_000` 상수로 변경
- `LogBuffer::with_capacity(max_size)` 생성자 추가
- `src/config/mod.rs`: `GlobalConfig`에 `log_buffer_size: Option<usize>` 필드 추가
- `config/global.toml`: `# log_buffer_size = 10000` 주석 문서화

### 5.5 ✅ GUI 빌드 최적화 → Vite 코드 스플리팅 적용 완료

- `saba-chan-gui/vite.config.js`에 `manualChunks` 설정 추가
- `vendor-react` (react, react-dom) 및 `vendor-i18n` (i18next, react-i18next) 벤더 청크 분리
- 빌드 출력: `vendor-react` 132KB, `vendor-i18n` 56KB, `index` 296KB (gzip: 각각 43KB, 18KB, 76KB)

---

## 6. 데드 코드 및 스텁 정리

`#[allow(dead_code)]` 어노테이션은 AI 에이전트가 "아직 사용 안 되지만 나중에 쓸 코드"를 남긴 흔적. 이를 정리하여 실제 사용 중인 코드와 미래 코드를 구분해야 함.

### 전수 목록

| 파일 | 항목 | 상태 | 권장 조치 |
|---|---|---|---|
| `src/supervisor/state_machine.rs` | `StateMachine` 전체 | ✅ `#[allow(dead_code)]` 제거, TODO 주석 추가 | 향후 Supervisor 통합 |
| `src/path_detector.rs` | `PathDetector` 전체 | ✅ 삭제 완료 | — |
| `src/resource/mod.rs` | `ResourceLimit` 전체 | ✅ 삭제 완료 | — |
| `src/config/mod.rs` | `get_server()` | ✅ 삭제 완료 | — |
| `src/config/mod.rs` | `ServerInstance`, `ResourceConfig` | ✅ `#[allow(dead_code)]` 제거 완료 | TOML 스키마 타입으로 유지 |
| `src/plugin/mod.rs` | `PluginManager` struct | ✅ struct 제거 완료 | 함수만 유지 |

**일괄 정리 AI 에이전트 지시문**:
> "프로젝트에서 `#[allow(dead_code)]`가 붙은 모든 항목을 찾아서, 실제 호출 위치가 없는 것은 삭제해줘. 단, `state_machine.rs`는 Supervisor 통합을 위해 보존하고, `#[allow(dead_code)]` 대신 `// TODO: integrate with Supervisor` 주석으로 교체해."

---

## 7. 코드 중복 제거

### 7.1 중복 항목 매트릭스

| 중복 코드 | 위치 1 | 위치 2 | 해소 방법 |
|---|---|---|---|
| `key_map` (70줄) | `minecraft/lifecycle.py::configure()` | `minecraft/lifecycle.py::install_server()` | 모듈 상수로 추출 |
| RCON 클라이언트 | `minecraft/lifecycle.py::_send_rcon_command()` | `palworld/lifecycle.py::PalworldRconClient` | ✅ `extensions/rcon.py` 통합 완료 |
| `taskkill /F /PID` | `supervisor/mod.rs::stop_server()` (managed) | `supervisor/mod.rs::stop_server()` (non-managed) | ✅ `process::force_kill_pid()` 추출 완료 |
| `hide_window()` | `src/plugin/mod.rs` | `src/supervisor/managed_process.rs` (유사 패턴) | ✅ `src/utils.rs::apply_creation_flags()` 공용화 완료 |
| `DEFAULT_PROPERTIES` | `minecraft/lifecycle.py` (하드코딩) | `modules/minecraft/server.properties` (참조용 파일) | ✅ `server.properties` 파일에서 로드 + saba-chan 오버라이드 방식으로 전환 |
| UE4 INI 파서 | `palworld/lifecycle.py::_parse_option_settings()` | (현재 1곳이지만 다른 UE 게임 모듈 추가 시 중복될 구조) | ✅ `extensions/ue4_ini.py` 추출 완료 |

---

## 8. 테스트 전략

### 8.1 현재 테스트 커버리지

| 컴포넌트 | 테스트 유형 | 파일 | 줄 수 | 평가 |
|---|---|---|---:|:---:|
| Core Daemon | 유닛 (인라인) | 각 모듈 `#[cfg(test)]` | ~280 | 양호 |
| Core Daemon | 통합 | `tests/daemon_integration.rs` | 350 | 양호 |
| GUI | 없음 | `setupTests.js`만 존재 | 0 | ❌ |
| CLI | 없음 | `test_modules.rs` (스텁) | 미미 | ❌ |
| Discord Bot | 통합 | `test/integration.test.js` | 655 | 양호 |
| Modules | 유닛 | `test_lifecycle.py`, `test_ue4_ini.py` | ~250 | 기본적 |

### 8.2 필요한 테스트 추가 우선순위

1. **`module_loader.rs` — 파싱 퍼즈 테스트**
   - 비정상 module.toml 입력 시 패닉이 아닌 에러 반환 확인
   - 필수 필드 누락 시 명확한 에러 메시지 확인

2. **`lifecycle.py` — 유닛 테스트**
   ```python
   # modules/minecraft/test_lifecycle.py
   def test_validate_no_java():
       result = validate({"java_path": "/nonexistent"})
       assert not result["success"]
       assert any(i["code"] == "JAVA_NOT_FOUND" for i in result["issues"])
   
   def test_key_map_completeness():
       """key_map이 DEFAULT_PROPERTIES의 모든 키를 커버하는지 확인"""
       ...
   ```

3. **IPC 핸들러 — 통합 테스트 확장**
   - 현재 `daemon_integration.rs`가 supervisor 초기화와 모듈 발견만 테스트
   - 인스턴스 CRUD, 서버 시작/중지 사이클, 명령어 실행 경로 추가 필요

4. **GUI — React Testing Library**
   - 최소한 `DynamicSettings` 컴포넌트의 필드 렌더링 테스트
   - SSE 콘솔 스트림 모킹 테스트

### 8.3 ✅ CI/CD 파이프라인 → GitHub Actions 추가 완료

`.github/workflows/ci.yml` 생성 완료. Rust (build + test + clippy), Node.js (Discord Bot), Python (syntax check) 3개 잡.

**권장**: GitHub Actions 워크플로우 추가
```yaml
# .github/workflows/ci.yml
name: CI
on: [push, pull_request]
jobs:
  rust:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace
      - run: cargo clippy --workspace -- -D warnings
  
  node:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - run: cd discord_bot && npm ci && npm test
  
  python:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
      - run: cd modules/minecraft && python -m pytest test_lifecycle.py -v
```

---

## 9. 보안 점검

### 9.1 발견 항목

| # | 항목 | 위험도 | 위치 | 설명 |
|---|---|:---:|---|---|
| S1 | IPC 무인증 | 중 | `src/ipc/mod.rs` | 로컬 API에 인증 없음. 같은 머신의 악의적 프로세스가 서버 제어 가능 |
| S2 | ✅ RCON 비밀번호 평문 저장 | 중 | `instances.json` | `instances.json` 파일 권한 0600 적용 (Unix) |
| S3 | ✅ Admin 비밀번호 자동 생성 로깅 | 낮 | `palworld/lifecycle.py` | `changes` 딕셔너리에 마스킹된 값 반환 (`***auto-generated***`) |
| S4 | ✅ 파이썬 Config 인젝션 | 낮 | `plugin/mod.rs` | `config_json`을 stdin으로 전달하도록 변경 완료 |
| S5 | SO_REUSEADDR 사용 | 정보 | `src/ipc/mod.rs` | 포트 바인딩 재시도 시 사용. 의도적이나, 다른 프로세스가 같은 포트를 하이재킹할 수 있음 |

### 9.2 권장 조치

- **S1**: 토큰 기반 인증 (섹션 4.2 참조)
- **S2**: ✅ `instances.json` 저장 시 Unix에서 파일 퍼미션 0600 적용 (`src/instance/mod.rs`)
- **S4**: ✅ `plugin/mod.rs`에서 config JSON을 커맨드 라인 인자 대신 stdin으로 전달. `minecraft/lifecycle.py`, `palworld/lifecycle.py`, `_template/lifecycle.py` 모두 stdin 방식으로 통일.

---

## 10. 바이브 코딩 특화 가이드라인

AI 에이전트가 이 코드베이스를 효과적으로 유지보수하기 위한 규칙.

### 10.1 파일 크기 상한

> **규칙**: 단일 파일은 **500줄을 초과하지 않도록** 유지한다. 초과 시 즉시 분할한다.

이유: 대부분의 AI 모델의 효과적인 작업 컨텍스트는 파일 1~2개 수준. 500줄을 넘으면 코드를 정확히 수정할 확률이 급격히 떨어짐.

### 10.2 새 기능 추가 체크리스트

새로운 기능(API 엔드포인트, UI 화면, 모듈 등)을 AI에게 지시할 때:

- [ ] 영향받는 파일 목록을 명시
- [ ] 기존 유사 패턴(같은 파일 내 다른 핸들러 등)을 참조로 제시
- [ ] i18n 키 추가를 지시에 포함
- [ ] 테스트 작성을 지시에 포함
- [ ] 500줄 초과 여부를 확인하도록 지시

### 10.3 모듈 추가 체크리스트

새 게임 모듈을 추가할 때:

- [ ] `modules/_template/`를 복사하여 시작
- [ ] `module.toml`의 필수 필드 (`metadata`, `config`, `detection`) 작성
- [ ] `lifecycle.py`의 필수 함수 (`validate`, `get_launch_command`, `status`) 구현
- [ ] `locales/en/*.json`에 번역 키 추가
- [ ] `tests/`에 기본 테스트 추가

### 10.4 `#[allow(dead_code)]` 금지 원칙

> **규칙**: `#[allow(dead_code)]`는 **절대 커밋하지 않는다**.

AI 에이전트가 "나중에 쓸 것"이라며 dead code를 남기는 것은 바이브 코딩의 가장 흔한 안티패턴. 아직 사용하지 않는 코드는 아예 작성하지 않거나, 별도 브랜치에 보관.

### 10.5 의존성 그래프 인식

AI 에이전트에게 크로스 컴포넌트 변경을 지시할 때 반드시 명시해야 할 의존 관계:

```
module.toml (설정 스키마)
    ↓ module_loader.rs가 파싱
    ↓ ipc/mod.rs가 API 응답에 포함
    ↓ GUI App.js가 동적 렌더링
    ↓ CLI screens.rs가 TUI로 렌더링
    ↓ Discord Bot index.js가 슬래시 커맨드 생성

lifecycle.py (런타임 로직)
    ↑ supervisor/mod.rs가 plugin/mod.rs를 통해 호출
    ↑ ipc/ 핸들러가 supervisor 메서드 호출

instances.json (런타임 데이터)
    ↑ instance/mod.rs가 CRUD
    ↑ ipc/ 핸들러가 사용
```

> **예시 지시문**: "Minecraft module.toml에 `custom_jvm_args` 설정 필드를 추가해줘. 이 필드는 module_loader.rs의 `SettingField` 파싱 → GUI의 동적 설정 폼 → CLI의 설정 화면 → lifecycle.py의 `get_launch_command()`에 모두 반영되어야 해."

### 10.6 커밋 메시지 규약

AI 에이전트가 생성하는 커밋은 다음 형식을 따르도록:

```
<type>(<scope>): <subject>

<body>

Types: feat, fix, refactor, test, chore, docs
Scopes: daemon, gui, cli, bot, module-mc, module-pw, updater, i18n
```

---

## 부록: 리팩터링 우선순위 로드맵

```
Phase 1 (즉시, ~3일) — ✅ 완료
├── [P0] ✅ ipc/mod.rs 분할 (1,864줄 → 546줄 + 6개 핸들러 서브모듈)
├── [P0] ✅ managed_process.rs에서 MC 전용 로직 제거 (module.toml log_pattern 기반 제네릭화)
└── [P1] ✅ dead code 일괄 정리 (path_detector, resource, PluginManager, get_server 삭제)

Phase 2 (1주, App.js는 점진적) — 부분 완료
├── [P0] App.js Context/Hook 분리 시작
├── [P1] ✅ StateMachine — #[allow(dead_code)] 제거, TODO 주석 교체
├── [P1] ✅ GlobalConfig 에러 처리 수정 (match 기반 분기)
└── [P1] ✅ lifecycle.py 인터페이스 템플릿 작성 (modules/_template/)

Phase 3 (2주) — 대부분 완료
├── [P2] ✅ module.toml 강타입 스키마 (serde Deserialize 구조체 + parse_module_toml())
├── [P2] ✅ IPC 토큰 인증 (X-Saba-Token 헤더 + auth 미들웨어)
├── [P2] ✅ plugin/mod.rs 비동기 전환 (tokio::process::Command)
├── [P2] ✅ CLI screens.rs 분할 (1,326줄 → 9개 파일)
├── [P2] ✅ 에러 처리 체계화 (SupervisorError thiserror 정의)
└── [P2] ✅ Discord Bot 에러 핸들링 (timeout, 글로벌 핸들러, interaction 안전성)

Phase 4 (지속적) — 거의 완료
├── [P2] ✅ 테스트 커버리지 확대 (module_loader 13개, ue4_ini 9개, lifecycle 7개 테스트 추가)
├── [P2] ✅ CI/CD 구축 (.github/workflows/ci.yml)
├── [P3] ✅ 코드 중복 제거 (DEFAULT_PROPERTIES 파일 로드, UE4 INI 파서 공유 모듈 추출)
├── [P3] ✅ i18n 하드코딩 정리 (palworld lifecycle.py ~30개 문자열 전환)
├── [P3] ✅ 보안 개선 (S2: 파일 권한, S3: 패스워드 마스킹, S4: stdin JSON 전달)
└── [P3] ✅ 리소스 제한 스텁 제거 (섹션 3.3에서 완료)
```

---

*이 문서는 AI 에이전트가 참조할 수 있도록 프로젝트 루트에 보관합니다. 각 Phase 완료 시 해당 항목에 ✅ 체크를 추가하세요.*
