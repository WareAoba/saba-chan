# Extension System Migration Blueprint

> **이 문서는 AI 에이전트가 실행할 작업 지시서입니다.**
> 순서대로 Phase를 실행하고, 각 작업마다 명시된 검증 명령을 반드시 수행하세요.

---

## AI 에이전트를 위한 사전 안내

### 이 문서의 읽는 법

1. **Phase 순서 엄수** — Phase 번호 순서대로 진행. 건너뛰기 금지.
2. **각 Task는 원자적** — 하나의 Task를 완료하고 검증한 후 다음으로.
3. **`검증`은 반드시 실행** — `cargo build`, `grep` 등 명시된 명령을 실행하여 통과 확인.
4. **`완료 기준`을 만족해야 다음 Task로** — 달성하지 못하면 해당 Task를 디버그.
5. **기존 코드를 먼저 읽어라** — 각 Task의 `대상 파일`을 반드시 read_file로 확인 후 수정.

### 컨텍스트 로딩 순서

새 세션을 시작하면 이 순서로 파일을 읽어 전체 아키텍처를 이해하세요:

```
1. 이 문서 전체
2. src/main.rs                         — 데몬 진입점, mod 선언
3. src/ipc/mod.rs                      — AppState(IPCServer), ServerInfo, 라우터
4. src/supervisor/mod.rs               — Supervisor 구조체, start/stop/status
5. src/instance/mod.rs                 — ServerInstance 구조체
6. src/supervisor/module_loader.rs     — ModuleMetadata, DockerExtensionConfig
7. src/docker/mod.rs                   — DockerComposeManager (삭제 대상)
8. src/plugin/mod.rs                   — run_plugin (Python 호출 인터페이스)
9. src/ipc/handlers/instance.rs        — 인스턴스 CRUD, docker_provision()
10. src/ipc/handlers/server.rs         — list_servers, Docker 통계
11. extensions/docker_engine.py        — 포터블 Docker Engine (Python)
12. saba-chan-gui/src/components/ServerCard.js
13. saba-chan-gui/src/components/ServerSettingsModal.js
14. saba-chan-gui/src/components/MemoryGauge.js
15. saba-chan-gui/src/components/Modals/AddServerModal.js
16. saba-chan-gui/src/hooks/useServerSettings.js
17. saba-chan-gui/src/hooks/useServerActions.js
```

### 워크스페이스 정보

- OS: Windows
- 경로: `c:\Git\saba-chan\`
- Rust 빌드: `cargo build` (워크스페이스 루트에서)
- GUI 빌드: `cd saba-chan-gui && npm run build`
- 기존 유닛 테스트 75개: `cargo test`
- IPC 포트: 57474 (HTTP, axum 기반)

---

## 0. 현재 Docker 기능 완전 목록

> 이 목록의 **모든 항목**이 익스텐션 시스템을 통해 동일하게 동작해야 한다.
> 하나라도 빠지면 마이그레이션 실패.

### 0.1 Rust 데몬 기능 (src/docker/mod.rs — 930줄)

| # | 함수/구조체 | 하는 일 | 호출 위치 |
|---|-----------|--------|----------|
| D1 | `WSL2_MODE` + `set_wsl2_mode()` / `is_wsl2_mode()` | 전역 WSL2 모드 플래그 관리 | supervisor, ipc handlers |
| D2 | `DockerComposeManager::new()` | 인스턴스 디렉토리로 매니저 생성 | supervisor, ipc handlers |
| D3 | `DockerComposeManager::has_compose_file()` | docker-compose.yml 존재 확인 | supervisor, ipc handlers, main.rs |
| D4 | `DockerComposeManager::start()` | `docker compose up -d` | supervisor `start_server`, `start_managed_server` |
| D5 | `DockerComposeManager::stop()` | `docker compose stop` | supervisor `stop_server` |
| D6 | `DockerComposeManager::down()` | `docker compose down` | instance delete, daemon shutdown |
| D7 | `DockerComposeManager::status()` | `docker compose ps --format json` | supervisor `get_server_status`, list_servers |
| D8 | `DockerComposeManager::server_process_running()` | `docker top`으로 프로세스 패턴 매치 | supervisor status, list_servers |
| D9 | `DockerComposeManager::logs()` | `docker compose logs` | (현재 미사용이지만 존재) |
| D10 | `DockerComposeManager::restart()` | `docker compose restart` | (현재 미사용이지만 존재) |
| D11 | `docker_container_stats()` | `docker stats --no-stream --format json` | list_servers (리소스 게이지용) |
| D12 | `is_docker_available()` | docker CLI 존재 확인 | docker_provision |
| D13 | `is_docker_daemon_running()` | dockerd 실행 확인 | docker_provision |
| D14 | `docker_status_detail()` | Docker 가용성 상세 정보 | (현재 미사용) |
| D15 | `ensure_docker_engine()` | 포터블 Docker 확보 + 시작 (Python 호출) | supervisor start, list_servers |
| D16 | `ensure_docker_engine_with_progress()` | 위와 동일 + 진행률 콜백 | docker_provision step 1 |
| D17 | `docker_engine_info()` | Docker 버전 정보 (Python 호출) | (현재 미사용) |
| D18 | `ComposeTemplateContext` | compose 템플릿 변수 구조체 | docker_provision step 3 |
| D19 | `generate_compose_yaml()` | docker-compose.yml YAML 문자열 생성 | provision, settings_changed |
| D20 | `provision_compose_file()` | 인스턴스 디렉토리에 YAML 파일 기록 | provision, settings_changed |
| D21 | `DockerComposeConfig` | compose 설정 구조체 | DockerComposeManager |
| D22 | 유닛 테스트 (~130줄) | generate_compose_yaml 출력 검증 | cargo test |

### 0.2 IPC/Supervisor 통합 (Mounting Points)

| # | 위치 | 하는 일 |
|---|------|--------|
| M1 | `main.rs` L196-209 | shutdown 시 모든 Docker 인스턴스 `docker compose down` |
| M2 | `supervisor/mod.rs` start_server (L87-113) | Docker일 때 compose up 으로 분기 |
| M3 | `supervisor/mod.rs` stop_server (L207-223) | Docker일 때 compose stop으로 분기 |
| M4 | `supervisor/mod.rs` get_server_status (L347-401) | Docker일 때 compose ps + docker top으로 분기 |
| M5 | `supervisor/mod.rs` start_managed_server (L545-571) | Docker일 때 compose up으로 분기 |
| M6 | `supervisor/mod.rs` validate_settings (L729-731) | use_docker 플래그를 Python에 전달 |
| M7 | `ipc/handlers/instance.rs` create_instance (L86-175) | Docker 프로비저닝 백그라운드 spawn |
| M8 | `ipc/handlers/instance.rs` docker_provision (L193-382) | Docker 엔진 확인 → SteamCMD → compose 생성 (3단계) |
| M9 | `ipc/handlers/instance.rs` delete_instance (L420-427) | Docker compose down |
| M10 | `ipc/handlers/instance.rs` update_settings (L489-775) | docker_cpu/memory_limit 처리 + compose 재생성 |
| M11 | `ipc/handlers/server.rs` list_servers (L21-150) | Docker 상태/프로세스/통계 수집 + ServerInfo 매핑 |
| M12 | `ipc/mod.rs` DockerStatsCache (L348-395) | 5초 TTL 통계 캐시 |
| M13 | `ipc/mod.rs` AppState (L415-433) | docker_stats_cache 필드 |
| M14 | `supervisor/migration.rs` | v1 인스턴스 마이그레이션 시 Docker 처리 |

### 0.3 데이터 모델

| # | 구조체 | Docker 전용 필드 |
|---|--------|----------------|
| F1 | `ServerInstance` | `use_docker: bool`, `docker_cpu_limit: Option<f64>`, `docker_memory_limit: Option<String>` |
| F2 | `ServerInfo` | `use_docker`, `docker_memory_usage`, `docker_memory_percent`, `docker_cpu_percent`, `docker_cpu_limit`, `docker_memory_limit` |
| F3 | `ModuleMetadata` | `docker: Option<DockerExtensionConfig>`, `docker_process_patterns: Vec<String>` |
| F4 | `DockerExtensionConfig` | image, working_dir, restart, command, entrypoint, user, ports, volumes, environment, dockerfile, extra_options, cpu_limit, memory_limit |
| F5 | `DockerSectionToml` | TOML 역직렬화용 |

### 0.4 GUI 컴포넌트

| # | 파일 | Docker 기능 |
|---|------|-----------|
| G1 | `ServerCard.js` L86-88 | Docker 배지 (아이콘) |
| G2 | `ServerCard.js` L102-104 | 미니 MemoryGauge (헤더) |
| G3 | `ServerCard.js` L146 | 프로비저닝 라벨 "docker_engine" |
| G4 | `ServerCard.js` L190-201 | 확장 영역: MemoryGauge + CPU 레이블 |
| G5 | `MemoryGauge.js` 전체 | 아날로그 게이지 SVG (~250줄) |
| G6 | `ServerSettingsModal.js` L231-285 | DockerTab (CPU/메모리 제한 UI) |
| G7 | `ServerSettingsModal.js` L460-507 | Docker 탭 버튼 + 조건부 렌더링 |
| G8 | `AddServerModal.js` L24,50,59,127-147 | Docker 격리 토글 |
| G9 | `Icon.js` L348 | `dockerL` SVG 아이콘 |
| G10 | `useServerSettings.js` L107-121, L387-397 | Docker 설정 초기화/저장 |
| G11 | `useServerActions.js` L417-448 | use_docker를 create payload에 포함 |
| G12 | `App.css` | `.docker-badge`, `.docker-stats-row`, `.memory-gauge-compact`, `.docker-cpu-label`, `.as-docker-row` |
| G13 | `index.js` L16 | MemoryGauge re-export |

### 0.5 Python/Module/i18n

| # | 파일 | 내용 |
|---|------|------|
| P1 | `extensions/docker_engine.py` | 포터블 Docker Engine 관리 (~830줄) |
| P2 | `saba-chan-gui/extensions/docker_engine.py` | 위의 GUI 번들용 복사본 |
| P3 | `modules/minecraft/module.toml` [docker] | image, ports, volumes, environment |
| P4 | `modules/palworld/module.toml` [docker] | image, entrypoint, user, ports, volumes |
| P5 | `modules/palworld/lifecycle.py` L795-878 | use_docker 분기 (경로 변환, 실행파일 체크 스킵) |
| P6 | `locales/en/gui.json` | docker_* i18n 키 20+개 |
| P7 | `locales/ko/gui.json` | 동일 |
| P8 | `locales/ja/gui.json` | 13+개 |

---

## 1. 목표 아키텍처

### 1.1 핵심 원칙

1. **제로 Docker in 메인 코드** — `src/` 안에 "docker" 문자열이 extension 관련 일반 코드 외에는 없어야 함
2. **범용 익스텐션 시스템** — Docker는 첫 번째 익스텐션일 뿐. 향후 모든 컴포넌트(데몬/GUI/CLI/Discord봇)에 대해 같은 패턴으로 확장
3. **다리 먼저, 제거 나중** — 기존 Docker 코드에 Hook 다리를 먼저 놓고, 익스텐션으로 우회 동작을 검증한 후, 기존 코드를 제거
4. **런타임 동적 토글** — 데몬 재시작 없이 익스텐션 on/off
5. **Python 위임** — Docker 로직은 전부 Python으로 이전, `run_plugin`으로 호출

### 1.2 최종 디렉토리 구조

```
extensions/
├── docker/                           ← Docker 익스텐션 패키지
│   ├── manifest.json                 ← 익스텐션 메타/훅/슬롯 선언
│   ├── docker_engine.py              ← 포터블 Docker Engine 관리 (이동)
│   ├── compose_manager.py            ← DockerComposeManager Python 포팅 (신규)
│   ├── gui/
│   │   ├── package.json
│   │   ├── vite.config.js
│   │   ├── src/
│   │   │   ├── index.js              ← 슬롯 컴포넌트 export
│   │   │   ├── DockerBadge.js
│   │   │   ├── DockerMiniGauge.js
│   │   │   ├── DockerStatsRow.js
│   │   │   ├── MemoryGauge.js        ← 아날로그 게이지 (이동)
│   │   │   ├── DockerTab.js          ← 설정 탭 (이동)
│   │   │   ├── DockerToggle.js       ← AddServer 토글 (이동)
│   │   │   └── docker.css
│   │   └── dist/
│   │       └── docker-gui.umd.js     ← 빌드된 UMD 번들
│   └── i18n/
│       ├── en.json
│       ├── ko.json
│       └── ja.json
├── __init__.py                       ← 기존 유지
├── steamcmd.py                       ← 기존 유지
├── rcon.py                           ← 기존 유지
└── ue4_ini.py                        ← 기존 유지
```

### 1.3 Hook 시스템 — 범용 설계

Hook은 **어디서나** 사용할 수 있도록 계층적으로 설계한다:

```
hook namespace:
  daemon.*          ← 데몬 수명주기
  server.*          ← 서버 인스턴스 수명주기
  gui.*             ← GUI 확장 포인트 (슬롯 시스템)
  cli.*             ← CLI 확장 포인트
  bot.*             ← Discord 봇 확장 포인트
  module.*          ← 모듈 설정 파싱 확장
```

#### 1.3.1 전체 Hook 목록

| Hook | 트리거 시점 | 인자 | 반환 | Docker가 사용? |
|------|-----------|------|------|:---:|
| **daemon.startup** | 데몬 초기화 직후 | `{}` | status | ✓ (ensure_docker_engine) |
| **daemon.shutdown** | 종료 시그널 수신 | `{instances: [...]}` | ok | ✓ (compose down 루프) |
| **daemon.tick** | 주기적 유지보수(60s) | `{}` | — | (향후) |
| **server.pre_create** | 인스턴스 생성 직전 | `{instance, module_config}` | 수정된 instance | ✓ (working_dir 설정) |
| **server.post_create** | 인스턴스 생성 직후 | `{instance, instance_dir, module_config}` | — | ✓ (프로비저닝) |
| **server.pre_start** | 서버 시작 직전 | `{instance_id, instance_dir, ext_data}` | `{handled, ...}` | ✓ (compose up) |
| **server.post_stop** | 서버 정지 직후 | `{instance_id, instance_dir, ext_data}` | `{handled, ...}` | ✓ (compose stop) |
| **server.status** | 상태 조회 | `{instance_id, instance_dir, ext_data, process_patterns}` | `{handled, status, ...}` | ✓ (compose ps + docker top) |
| **server.stats** | 리소스 사용량 조회 | `{instance_id, instance_dir, ext_data}` | `{docker_memory_*, ...}` | ✓ (docker stats) |
| **server.pre_delete** | 인스턴스 삭제 직전 | `{instance_id, instance_dir}` | — | ✓ (compose down) |
| **server.settings_changed** | 설정 변경 후 | `{instance_id, instance_dir, ext_data, changed}` | — | ✓ (compose 재생성) |
| **server.list_enrich** | list_servers 응답 보강 | `{instance_id, ext_data, status}` | 추가 필드 | ✓ (docker 필드) |
| **server.logs** | 로그 조회 | `{instance_id, instance_dir, lines}` | `{logs}` | ✓ (compose logs) |
| **module.parse_config** | module.toml 파싱 시 | `{toml_section}` | `{config}` | ✓ ([docker] 섹션) |
| **gui.slots** | (GUI 전용 — manifest.json에 선언) | — | — | ✓ |
| **cli.commands** | CLI 커맨드 등록 | `{command_defs}` | — | (향후) |
| **bot.commands** | Discord 봇 커맨드 등록 | `{command_defs}` | — | (향후) |

#### 1.3.2 Hook 실행 규칙

1. **condition 평가**: `"condition": "instance.ext_data.docker_enabled"` — 인스턴스의 extension_data에서 키를 확인, true일 때만 호출
2. **handled 패턴**: hook이 `{"handled": true}`를 반환하면, 호출 측의 기본 동작을 스킵 (예: Docker start가 handled=true → Native start 스킵)
3. **다중 익스텐션**: 같은 hook에 여러 익스텐션이 등록되면 순서대로 실행. 첫 번째 handled=true가 나오면 나머지는 스킵 (chain-of-responsibility)
4. **에러 전파**: Python 프로세스가 비정상 종료하면 hook 실패로 간주, 에러 로깅 후 기본 동작 진행 (graceful degradation)
5. **비동기 hook**: `server.post_create`처럼 장시간 실행되는 hook은 별도 tokio::spawn으로 백그라운드 실행. 진행률은 ProvisionTracker로 관리

#### 1.3.3 조건(condition) 평가 규칙

condition 문자열은 간단한 dotpath로 인스턴스 extension_data를 체크한다:

- `"instance.ext_data.docker_enabled"` → `instance.extension_data["docker_enabled"] == true`
- 조건이 없으면(`null`) 무조건 실행
- 복합 조건 불필요 — 단순 boolean 체크면 충분

### 1.4 데이터 모델 변경

#### ServerInstance (기존 → 신규)

```rust
// 기존: Docker 전용 필드
pub use_docker: bool,
pub docker_cpu_limit: Option<f64>,
pub docker_memory_limit: Option<String>,

// 신규: 범용 확장 데이터
/// 익스텐션이 저장하는 키-값 데이터.
/// 예: {"docker_enabled": true, "docker_cpu_limit": 2.0, "docker_memory_limit": "4g"}
#[serde(default)]
pub extension_data: HashMap<String, serde_json::Value>,
```

마이그레이션 규칙 (`use_docker` → `extension_data`):
```
use_docker: true      → extension_data["docker_enabled"] = true
docker_cpu_limit: 2.0 → extension_data["docker_cpu_limit"] = 2.0
docker_memory_limit: "4g" → extension_data["docker_memory_limit"] = "4g"
```

#### ServerInfo (기존 → 신규)

```rust
// 기존 Docker 필드 전부 제거, 대신:
#[serde(default)]
pub extension_data: HashMap<String, serde_json::Value>,
```

GUI는 `server.extension_data.docker_enabled`, `server.extension_data.docker_memory_percent` 등으로 접근.

---

## 2. Extension Manifest 포맷

`extensions/docker/manifest.json`:

```json
{
  "id": "docker",
  "name": "Docker Isolation",
  "version": "1.0.0",
  "description": "Docker 컨테이너를 사용한 게임 서버 격리 실행",
  "author": "saba-chan",
  "min_app_version": "0.2.0",
  "dependencies": [],

  "python_modules": {
    "docker_engine": "docker_engine.py",
    "compose_manager": "compose_manager.py"
  },

  "hooks": {
    "daemon.startup": {
      "module": "docker_engine",
      "function": "ensure",
      "condition": null
    },
    "daemon.shutdown": {
      "module": "compose_manager",
      "function": "shutdown_all",
      "condition": null
    },
    "server.pre_create": {
      "module": "compose_manager",
      "function": "pre_create",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.post_create": {
      "module": "compose_manager",
      "function": "provision",
      "condition": "instance.ext_data.docker_enabled",
      "async": true
    },
    "server.pre_start": {
      "module": "compose_manager",
      "function": "start",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.post_stop": {
      "module": "compose_manager",
      "function": "stop",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.status": {
      "module": "compose_manager",
      "function": "status",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.stats": {
      "module": "compose_manager",
      "function": "container_stats",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.pre_delete": {
      "module": "compose_manager",
      "function": "cleanup",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.settings_changed": {
      "module": "compose_manager",
      "function": "regenerate_compose",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.list_enrich": {
      "module": "compose_manager",
      "function": "enrich_server_info",
      "condition": "instance.ext_data.docker_enabled"
    },
    "server.logs": {
      "module": "compose_manager",
      "function": "get_logs",
      "condition": "instance.ext_data.docker_enabled"
    }
  },

  "gui": {
    "bundle": "gui/dist/docker-gui.umd.js",
    "styles": "gui/dist/style.css",
    "slots": {
      "ServerCard.badge": "DockerBadge",
      "ServerCard.headerGauge": "DockerMiniGauge",
      "ServerCard.expandedStats": "DockerStatsRow",
      "ServerSettings.tab": "DockerTab",
      "AddServer.options": "DockerToggle"
    }
  },

  "module_config_section": "docker",

  "instance_fields": {
    "docker_enabled": { "type": "boolean", "default": false },
    "docker_cpu_limit": { "type": "number", "optional": true },
    "docker_memory_limit": { "type": "string", "optional": true }
  },

  "i18n_dir": "i18n/"
}
```

---

## 3. Phase 계획 개요

```
Phase 1: Extension Infrastructure    ← ExtensionManager + API 엔드포인트 (기존 코드 미변경)
Phase 2: Docker Python Extension     ← compose_manager.py 작성 (Rust 코드 미변경)
Phase 3: Bridge Installation         ← 기존 코드에 Hook 다리 추가 (기존 Docker 코드 유지, 양쪽 경로 공존)
Phase 4: Bridge Validation           ← 환경변수로 Hook 경로 활성화, Docker 기능 전체 검증
Phase 5: Legacy Removal              ← 검증 통과 후 기존 Docker 코드 완전 삭제
Phase 6: GUI Extension System        ← 슬롯 시스템 + Docker GUI 익스텐션
Phase 7: Cleanup & Verification      ← dead code 제거, 최종 테스트
```

핵심 안전장치: **Phase 3-4에서 기존 코드와 익스텐션 코드가 나란히 존재**하여, 어느 시점에서든 환경변수 하나로 롤백 가능.

---

## Phase 1: Extension Infrastructure

> 기존 코드를 전혀 건드리지 않고, 익스텐션 시스템의 뼈대를 구축한다.

### Task 1.1: `src/extension/mod.rs` 생성

**파일**: `src/extension/mod.rs` (신규)

아래 구조체와 함수를 구현하라:

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// 익스텐션 매니페스트 — manifest.json을 역직렬화한 것
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub python_modules: HashMap<String, String>,   // name → relative path
    #[serde(default)]
    pub hooks: HashMap<String, HookBinding>,       // hook_name → binding
    #[serde(default)]
    pub gui: Option<GuiManifest>,
    #[serde(default)]
    pub module_config_section: Option<String>,      // "docker" 등
    #[serde(default)]
    pub instance_fields: HashMap<String, FieldDef>,
    #[serde(default)]
    pub i18n_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub module: String,           // python_modules의 키
    pub function: String,         // Python 함수명
    #[serde(default)]
    pub condition: Option<String>, // "instance.ext_data.docker_enabled"
    #[serde(default)]
    pub r#async: Option<bool>,    // true면 tokio::spawn으로 백그라운드 실행
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiManifest {
    pub bundle: String,
    #[serde(default)]
    pub styles: Option<String>,
    #[serde(default)]
    pub slots: HashMap<String, String>,  // slot_id → component_name
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub optional: Option<bool>,
}

/// 발견된 익스텐션 정보 (manifest + 디렉토리 경로)
#[derive(Debug, Clone)]
pub struct DiscoveredExtension {
    pub manifest: ExtensionManifest,
    pub dir: PathBuf,
}

pub struct ExtensionManager {
    extensions_dir: PathBuf,
    discovered: HashMap<String, DiscoveredExtension>,
    enabled: HashSet<String>,
    state_path: PathBuf,
}
```

구현해야 할 메서드:

| 메서드 | 역할 | 비고 |
|--------|------|------|
| `new(extensions_dir: &str) -> Self` | 생성 + load_state() | state_path: `%APPDATA%/saba-chan/extensions_state.json` |
| `discover() -> Result<Vec<String>>` | extensions/ 서브디렉토리 스캔, manifest.json 파싱 | 잘못된 매니페스트는 warn 로그 후 스킵 |
| `enable(ext_id: &str) -> Result<()>` | 활성화 + save_state | discovered에 없으면 에러 |
| `disable(ext_id: &str) -> Result<()>` | 비활성화 + save_state | |
| `is_enabled(ext_id: &str) -> bool` | 활성 여부 | |
| `list() -> Vec<ExtensionListItem>` | 발견된 전체 목록 (활성 상태 포함) | API 응답용 |
| `hooks_for(hook_name: &str) -> Vec<(&DiscoveredExtension, &HookBinding)>` | 해당 hook에 바인딩된 활성 익스텐션 | |
| `evaluate_condition(condition: &str, ext_data: &HashMap<String, Value>) -> bool` | 조건 평가 | `instance.ext_data.<key>` 패턴 |
| `dispatch_hook(hook_name: &str, context: Value) -> Vec<(String, Result<Value>)>` | 조건 평가 + run_plugin 호출 + handled 체크 | async |
| `dispatch_hook_with_progress(hook_name, context, on_progress)` | 위와 동일 + 진행률 콜백 | run_plugin_with_progress 사용 |
| `should_parse_config_section(section: &str) -> bool` | 해당 섹션의 익스텐션이 활성화? | module.toml 파싱 제어 |
| `all_instance_fields() -> HashMap<String, FieldDef>` | 활성 익스텐션의 instance_fields 합산 | |
| `gui_manifests() -> Vec<(&str, &GuiManifest)>` | 활성 익스텐션의 GUI 매니페스트 | |
| `extension_file_path(ext_id, relative) -> Option<PathBuf>` | 파일 절대 경로 | GUI 번들 서빙용 |
| `load_i18n(ext_id, locale) -> Option<Value>` | i18n JSON 로드 | |
| `save_state() / load_state()` | enabled 목록 영속화 | JSON: `["docker"]` |

**주의**: `dispatch_hook()` 내에서 `crate::plugin::run_plugin()`을 호출한다. Python 모듈 경로는 `ext.dir.join(filename)`으로 절대 경로를 만든다.

**검증**:
```powershell
# main.rs에 `mod extension;` 추가, lib.rs에 `pub mod extension;` 추가 후:
cargo build 2>&1 | Select-String "error"
# → 0 errors
```

**완료 기준**: `cargo build` 성공, 경고만 허용 (unused 경고 OK)

---

### Task 1.2: IPC 엔드포인트 추가

**신규 파일**: `src/ipc/handlers/extension.rs`

구현할 핸들러:

```
GET    /api/extensions                    → list_extensions()
POST   /api/extensions/:id/enable         → enable_extension()
POST   /api/extensions/:id/disable        → disable_extension()
GET    /api/extensions/:id/gui            → serve_gui_bundle()  (static file)
GET    /api/extensions/:id/gui/styles     → serve_gui_styles()  (static file)
GET    /api/extensions/:id/i18n/:locale   → serve_i18n()        (JSON)
```

**수정 파일**: `src/ipc/mod.rs`
1. `IPCServer`(AppState) 구조체에 `pub extension_manager: Arc<RwLock<crate::extension::ExtensionManager>>` 추가
2. `IPCServer::new()`에서:
   - `ExtensionManager::new(extensions_dir)` 생성
   - `discover()` 호출
   - AppState에 값 설정
3. `start()` 메서드의 라우터에 위 6개 라우트 추가

**수정 파일**: `src/ipc/handlers/mod.rs`
- `pub mod extension;` 추가

**수정 파일**: `src/main.rs`
- `mod extension;` 추가 (모듈 선언)

**수정 파일**: `src/lib.rs`
- `pub mod extension;` 추가

**검증**:
```powershell
cargo build 2>&1 | Select-String "error"
cargo test 2>&1 | Select-String "FAILED"
# 기존 75개 테스트 전체 통과
```

**완료 기준**: 빌드 성공, 기존 테스트 전체 통과

---

## Phase 2: Docker Python Extension 생성

> 기존 Rust Docker 코드를 참조하여 Python 스크립트를 만든다.
> 기존 코드는 변경하지 않으며, 새 파일만 생성한다.

### Task 2.1: 디렉토리 구조 생성

```powershell
New-Item -ItemType Directory -Path "extensions/docker/gui/src" -Force
New-Item -ItemType Directory -Path "extensions/docker/i18n" -Force
```

### Task 2.2: `extensions/docker/manifest.json` 생성

위 섹션 2의 전체 JSON을 그대로 사용.

### Task 2.3: `extensions/docker/compose_manager.py` 생성

**핵심**: `src/docker/mod.rs`의 모든 함수를 Python으로 포팅.

**반드시 `src/docker/mod.rs` 전체를 읽은 뒤 작성할 것.**

포팅 대상 함수 매핑 (이 목록을 전부 구현해야 함):

| Rust 함수 (src/docker/mod.rs) | Python 함수 (compose_manager.py) | Hook | 기능 ID |
|------|------|------|:---:|
| `DockerComposeManager::start()` | `start(config)` | server.pre_start | D4 |
| `DockerComposeManager::stop()` | `stop(config)` | server.post_stop | D5 |
| `DockerComposeManager::down()` | `cleanup(config)` | server.pre_delete | D6 |
| `DockerComposeManager::status()` + `server_process_running()` | `status(config)` | server.status | D7,D8 |
| `docker_container_stats()` | `container_stats(config)` | server.stats | D11 |
| `generate_compose_yaml()` + `provision_compose_file()` + docker_provision 3단계 | `provision(config)` | server.post_create | D18-D20,M8 |
| `provision_compose_file()` (설정 변경 시) | `regenerate_compose(config)` | server.settings_changed | D19,D20 |
| (shutdown 루프) | `shutdown_all(config)` | daemon.shutdown | M1 |
| (list_servers 보강) | `enrich_server_info(config)` | server.list_enrich | M11 |
| `DockerComposeManager::logs()` | `get_logs(config)` | server.logs | D9 |
| (인스턴스 생성 전처리) | `pre_create(config)` | server.pre_create | M7 |

**각 Python 함수의 인자/반환 규약**:

```python
def start(config: dict) -> dict:
    """
    config 키:
      instance_id: str
      instance_dir: str (절대 경로)
      extension_data: dict (docker_enabled, docker_cpu_limit 등)
      process_patterns: list[str]  (module.toml의 docker_process_patterns)
      module_config: dict          (module.toml [docker] 섹션, provision/regenerate에서)
      # 추가 키는 hook마다 다름

    반환:
      {"handled": True/False, "success": True/False, ...}
      handled=True → Rust 쪽 기본 동작 스킵
      handled=False → Rust 쪽 기본 동작 실행
    """
```

**generate_compose_yaml 포팅 시 반드시 확인할 사항** (D19):
1. `src/docker/mod.rs`의 `generate_compose_yaml()` 함수를 read_file로 **전체 읽기**
2. 템플릿 변수 치환: `{port}`, `{rcon_port}`, `{rest_port}`, `{rest_password}`, `{instance_dir}`
3. 볼륨 마운트의 상대/절대 경로 처리
4. WSL2 모드에서의 Windows→Linux 경로 변환
5. `deploy.resources.limits` (cpu_limit, memory_limit) — per-instance override 적용
6. 컨테이너 이름 규칙: `saba-{module}-{instance_id[:8]}`
7. entrypoint, user, command 등 Optional 필드 처리
8. environment 변수의 템플릿 치환
9. **Rust 유닛 테스트 (D22)에서 기대하는 YAML 출력과 동일한 결과를 생성해야 함**

**provision 포팅 시 반드시 확인할 사항** (M8):
1. `src/ipc/handlers/instance.rs`의 `docker_provision()` (L193-382) 전체 읽기
2. 3단계 파이프라인:
   - Step 0: Docker 엔진 확인 (`docker_engine.py ensure` 호출)
   - Step 1: SteamCMD 설치 (`extensions/steamcmd.py install` 호출)
   - Step 2: docker-compose.yml 생성
3. 진행률 보고: `PROGRESS:{"percent": N, "message": "..."}` stdout으로 출력 (기존 run_plugin 프로토콜)
4. WSL2 모드에서 SteamCMD platform을 "linux"로 강제

**status 포팅 시 반드시 확인할 사항** (D7, D8):
1. `docker compose ps --format json` 실행
2. 컨테이너가 running이면 `docker top <container> -eo args`로 프로세스 패턴 매치
3. 상태 결정:
   - 컨테이너 running + 서버 프로세스 있음 → "running"
   - 컨테이너 running + 서버 프로세스 없음 → "starting"
   - 컨테이너 정지 → "stopped"

**WSL2 모드 처리 (D1)**:
- Docker CLI prefix: WSL2면 `["wsl", "-d", "saba-docker", "--", "docker"]`, 네이티브면 `["docker"]`
- `docker_engine.py`의 `ensure()` 반환값에서 WSL2 여부를 판단하여 모듈 전역 변수에 저장
- 또는 config에 `wsl2_mode: bool`을 전달받음

**검증**:
```powershell
python -m py_compile extensions/docker/compose_manager.py
# 구문 에러 0건
```

### Task 2.4: `extensions/docker/docker_engine.py` 복사

```powershell
Copy-Item extensions/docker_engine.py extensions/docker/docker_engine.py
# 기존 파일은 아직 삭제하지 않음 (Phase 7에서 삭제)
```

### Task 2.5: i18n 파일 추출

`locales/en/gui.json`, `locales/ko/gui.json`, `locales/ja/gui.json`에서 `docker_`로 시작하는 키만 추출하여 별도 파일로 생성.

**아직 원본에서 제거하지 않음** — Phase 6에서 제거.

**검증**:
```powershell
python -c "import json; json.load(open('extensions/docker/i18n/en.json', encoding='utf-8'))"
python -c "import json; json.load(open('extensions/docker/i18n/ko.json', encoding='utf-8'))"
python -c "import json; json.load(open('extensions/docker/i18n/ja.json', encoding='utf-8'))"
# 에러 0건
```

---

## Phase 3: Bridge Installation (다리 설치)

> **핵심 원칙**: 기존 Docker 코드를 건드리지 않고, hook 디스패치 호출을 **추가**한다.
> 환경변수 `SABA_EXT_HOOKS=1`일 때만 hook 경로를 실행하고,
> 그렇지 않으면 기존 코드가 그대로 동작한다.

### Task 3.0: Feature Gate 헬퍼

**파일**: `src/extension/mod.rs`에 추가

```rust
/// 환경변수 SABA_EXT_HOOKS=1이면 true → hook 경로 사용
/// 아니면 false → 기존 직접 호출 경로 사용 (폴백)
pub fn use_extension_hooks() -> bool {
    std::env::var("SABA_EXT_HOOKS").map(|v| v == "1").unwrap_or(false)
}
```

### Task 3.1: ServerInstance에 extension_data 필드 추가

**파일**: `src/instance/mod.rs`

**기존 Docker 필드는 유지한 채** 새 필드만 추가:

```rust
pub struct ServerInstance {
    // ... 기존 필드 모두 유지 (use_docker, docker_cpu_limit, docker_memory_limit 포함) ...

    /// 범용 익스텐션 확장 데이터
    #[serde(default)]
    pub extension_data: HashMap<String, serde_json::Value>,
}
```

`Default` impl에 `extension_data: HashMap::new()` 추가.
`new()` 함수에서도 `extension_data: HashMap::new()` 초기화.

**검증**:
```powershell
cargo build 2>&1 | Select-String "error"
cargo test 2>&1 | Select-String "FAILED"
```

### Task 3.2: ServerInfo에 extension_data 필드 추가

**파일**: `src/ipc/mod.rs`

```rust
pub struct ServerInfo {
    // ... 기존 Docker 필드 모두 유지 ...

    /// 익스텐션 확장 데이터 (범용)
    #[serde(default)]
    pub extension_data: HashMap<String, serde_json::Value>,
}
```

### Task 3.3: Supervisor에 ExtensionManager 연결

**파일**: `src/supervisor/mod.rs`

```rust
pub struct Supervisor {
    // ... 기존 필드 ...
    /// 익스텐션 매니저 (Phase 3에서 optional로 추가)
    pub extension_manager: Option<Arc<RwLock<crate::extension::ExtensionManager>>>,
}
```

`new()`에서 `extension_manager: None` 초기화.
`main.rs`에서 Supervisor 생성 후 `sup.extension_manager = Some(ext_mgr.clone())` 설정.

### Task 3.4: Hook 다리 설치 — 패턴 설명

모든 다리는 이 패턴을 따른다:

```rust
// ── Extension hook: {hook_name} ──
if crate::extension::use_extension_hooks() {
    if let Some(ref ext_mgr) = self.extension_manager {
        let results = ext_mgr.read().await
            .dispatch_hook("{hook_name}", json!({
                "instance_id": instance.id.clone(),
                "instance_dir": instance_dir.to_string_lossy(),
                "extension_data": instance.extension_data.clone(),
                // ... hook별 추가 데이터 ...
            })).await;

        for (_ext_id, result) in &results {
            if let Ok(val) = result {
                if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                    return Ok(val.clone());  // 기존 코드 건너뜀
                }
            }
        }
    }
}
// ── 기존 코드 (그대로 유지) ──
if instance.use_docker { ... }
```

### Task 3.5: 실제 다리 설치 위치

아래 표의 **모든 위치**에 Task 3.4의 패턴으로 다리를 설치한다:

| 파일 | 함수 | Hook | 기존 Docker 코드 위치 | 추가 context 키 |
|------|------|------|---------------------|----------------|
| `supervisor/mod.rs` | `start_server()` | `server.pre_start` | L87-113 | — |
| `supervisor/mod.rs` | `stop_server()` | `server.post_stop` | L207-223 | — |
| `supervisor/mod.rs` | `get_server_status()` | `server.status` | L347-401 | `process_patterns` |
| `supervisor/mod.rs` | `start_managed_server()` | `server.pre_start` | L545-571 | — |
| `ipc/handlers/instance.rs` | `create_instance()` | `server.pre_create` + `server.post_create` | L86-175 | `module_config` |
| `ipc/handlers/instance.rs` | `delete_instance()` | `server.pre_delete` | L420-427 | — |
| `ipc/handlers/instance.rs` | `update_instance_settings()` | `server.settings_changed` | L489-775 | `changed_keys` |
| `ipc/handlers/server.rs` | `list_servers()` | `server.list_enrich` + `server.stats` | L21-150 | `status` |
| `main.rs` | shutdown 핸들러 | `daemon.shutdown` | L196-209 | `instances` |

**주의 — list_servers()는 특별한 처리 필요**:
- 기존: 각 인스턴스에 대해 Docker 상태/통계를 수집하여 `ServerInfo`의 Docker 필드에 매핑
- Hook: `server.list_enrich`와 `server.stats` 결과를 `ServerInfo.extension_data`에 병합
- **기존 Docker 필드도 호환을 위해 채워야 함** (Phase 5까지는 둘 다 유지)

### Task 3.6: Extension data 동기화 코드

인스턴스 생성 시 `use_docker: true`이면 `extension_data["docker_enabled"] = true`도 설정하는 동기화 코드를 `create_instance`에 추가:

```rust
// use_docker → extension_data 동기화 (Phase 3 호환 레이어)
if instance.use_docker {
    instance.extension_data.insert(
        "docker_enabled".to_string(),
        serde_json::Value::Bool(true),
    );
}
```

### Task 3.7: 빌드 & 테스트

```powershell
cargo build 2>&1 | Select-String "error"
cargo test 2>&1 | Select-String "FAILED"
# 기존 75개 테스트 전체 통과 (SABA_EXT_HOOKS 미설정 → 기존 경로)
```

**완료 기준**:
1. `cargo build` 성공
2. 기존 테스트 전체 통과
3. `SABA_EXT_HOOKS` 미설정 시 기존과 100% 동일하게 동작
4. `SABA_EXT_HOOKS=1`로 설정 시 hook 시도 → Docker 익스텐션 활성화 안 되어 있으면 handled=false → 기존 코드 폴백

---

## Phase 4: Bridge Validation (다리 검증)

> Docker 익스텐션을 활성화하고, Hook 경로로 모든 Docker 기능이 동작하는지 검증.

### Task 4.1: Docker 익스텐션 활성화 + 기능 테스트

```powershell
$env:SABA_EXT_HOOKS = "1"
# 데몬 실행 후:
# 1. curl http://localhost:57474/api/extensions → docker 목록에 나타남
# 2. curl -X POST http://localhost:57474/api/extensions/docker/enable → 활성화
# 3. Docker 인스턴스 생성/시작/정지/삭제 전체 흐름 테스트
```

### Task 4.2: 기능 완전성 체크리스트

**아래 T1~T16 모든 항목을 Hook 경로(`SABA_EXT_HOOKS=1`)로 검증:**

| # | 기능 | 테스트 방법 | 참조 ID |
|---|------|-----------|:---:|
| T1 | Docker 엔진 자동 확인/설치 | 데몬 시작 → daemon.startup hook | D15,D16 |
| T2 | 인스턴스 생성 + working_dir 설정 | POST /api/instances → pre_create hook | M7 |
| T3 | Docker 프로비저닝 (엔진→SteamCMD→compose) | post_create hook → provision() | M8 |
| T4 | 프로비저닝 진행률 | GET /api/provision-progress/:name | M8 |
| T5 | docker-compose.yml 생성 확인 | 인스턴스 디렉토리에 파일 존재 | D18-D20 |
| T6 | compose 템플릿 변수 치환 | YAML 내 포트/경로 값 확인 | D19 |
| T7 | Docker compose up (시작) | POST /api/server/:name/start → pre_start | D4,M2,M5 |
| T8 | Docker compose stop (정지) | POST /api/server/:name/stop → post_stop | D5,M3 |
| T9 | 컨테이너 상태 + 프로세스 감지 | GET /api/server/:name/status → status hook | D7,D8,M4 |
| T10 | Docker stats (메모리/CPU) | GET /api/servers → stats/list_enrich hooks | D11,M11 |
| T11 | 리소스 제한 변경 | PATCH /api/instance/:id → settings_changed | M10 |
| T12 | 설정 변경 → compose 재생성 | YAML 파일 업데이트 확인 | D19,D20 |
| T13 | Docker compose down (삭제) | DELETE /api/instance/:id → pre_delete | D6,M9 |
| T14 | Daemon shutdown 전체 정리 | Ctrl+C → shutdown hook | M1 |
| T15 | WSL2 모드 감지 | WSL 존재 시 WSL2 docker 사용 | D1 |
| T16 | 프로세스 패턴 매칭 | docker top + process_patterns | D8 |

### Task 4.3: Regression Test (롤백 확인)

```powershell
# 환경변수 OFF → 기존 경로로 T1~T16 동일하게 통과하는지 확인
$env:SABA_EXT_HOOKS = ""
```

**완료 기준**: T1~T16 모두 Hook 경로로 동작 확인, 환경변수 OFF 시 기존 경로도 정상

---

## Phase 5: Legacy Removal (기존 코드 제거)

> Phase 4 검증 통과 후, 이제 기존 Docker 코드를 안전하게 제거한다.

### Task 5.1: Feature Gate 제거

`if crate::extension::use_extension_hooks()` 분기를 모두 제거하고, Hook 경로만 남긴다.

```powershell
Select-String -Path "src\**\*.rs" -Pattern "use_extension_hooks" -Recurse
# 모든 매칭을 찾아서 Hook 전용으로 전환
```

### Task 5.2: 기존 Docker 분기 코드 제거

| 파일 | 제거 대상 |
|------|----------|
| `src/supervisor/mod.rs` | start_server/stop_server/get_server_status/start_managed_server 의 `if instance.use_docker { ... }` 블록 |
| `src/ipc/handlers/instance.rs` | `docker_provision()` 전체 함수, create_instance의 Docker 분기, delete_instance의 compose down, update_settings의 Docker 필드 처리 |
| `src/ipc/handlers/server.rs` | list_servers의 Docker 상태/통계 직접 수집 코드 |
| `src/main.rs` | shutdown의 Docker 정리 루프, `mod docker;` 선언 |
| `src/lib.rs` | `pub mod docker;` 선언 |

### Task 5.3: `src/docker/` 디렉토리 삭제

```powershell
Remove-Item src/docker -Recurse -Force
```

### Task 5.4: Docker 전용 데이터 필드 제거

**`src/instance/mod.rs`**:
- `use_docker`, `docker_cpu_limit`, `docker_memory_limit` 필드 제거
- `extension_data`만 남김
- 마이그레이션 함수 추가 (기존 instances.json 호환):

```rust
pub fn migrate_legacy_docker_fields(&mut self) {
    // 기존 JSON에 use_docker: true가 있으면 extension_data로 변환
}
```

`InstanceStore::load()` 시 각 인스턴스에 `migrate_legacy_docker_fields()` 호출.

**`src/ipc/mod.rs`**:
- `ServerInfo`에서 Docker 전용 필드 제거: `use_docker`, `docker_memory_usage`, `docker_memory_percent`, `docker_cpu_percent`, `docker_cpu_limit`, `docker_memory_limit`
- `extension_data`만 남김
- `DockerStatsCache` 구조체 전체 삭제
- `AppState`에서 `docker_stats_cache` 필드 제거

### Task 5.5: `src/supervisor/module_loader.rs` 일반화

- `DockerExtensionConfig`, `DockerSectionToml` 삭제
- `ModuleMetadata.docker` → 제거
- `extension_configs: HashMap<String, serde_json::Value>` 추가
- TOML 파싱 시 ExtensionManager가 활성화한 섹션만 `serde_json::Value`로 저장

### Task 5.6: Python lifecycle 정리

- `modules/palworld/lifecycle.py`에서 `use_docker` 분기 제거 (L795-878)

### Task 5.7: 빌드 & 검증

```powershell
cargo build 2>&1 | Select-String "error"
cargo test 2>&1 | Select-String "FAILED"

# src/ 내에 docker 직접 참조 없어야 함
Select-String -Path "src\**\*.rs" -Pattern "\bdocker\b" -Recurse |
  Where-Object { $_.Line -notmatch "extension|config_section|ext_data|docker_enabled|// " }
# → 0건
```

---

## Phase 6: GUI Extension System

> GUI에 범용 슬롯 시스템을 만들고, Docker GUI 컴포넌트를 익스텐션 패키지로 이동.

### Task 6.1: `ExtensionContext.js` 생성

**파일**: `saba-chan-gui/src/contexts/ExtensionContext.js`

역할:
1. `/api/extensions` 호출 → 활성 익스텐션 목록
2. 활성 익스텐션의 GUI 번들을 `<script>` tag로 동적 로드
3. 이름 규칙: `window.SabaExt{Id}` (예: `window.SabaExtDocker`)
4. 슬롯 레지스트리 관리: `slotId → [Component, ...]`
5. i18n 로드 + 기존 i18n에 병합

```javascript
export const useExtensions = () => useContext(ExtensionContext);
// 제공 값: { extensions, enabledExtensions, slots, toggleExtension }
```

### Task 6.2: `ExtensionSlot.js` 생성

**파일**: `saba-chan-gui/src/components/ExtensionSlot.js`

```javascript
export default function ExtensionSlot({ slotId, ...props }) {
  const { slots } = useExtensions();
  const components = slots[slotId] || [];
  if (components.length === 0) return null;
  return <>{components.map((Comp, i) => <Comp key={`${slotId}-${i}`} {...props} />)}</>;
}
```

### Task 6.3: `App.js`에 `<ExtensionProvider>` 래핑

### Task 6.4: Docker GUI 컴포넌트 이동 & 추출

| 원본 | 이동/추출 위치 | 작업 |
|------|--------------|------|
| `MemoryGauge.js` | `extensions/docker/gui/src/MemoryGauge.js` | 전체 이동 |
| `ServerSettingsModal.js` L231-285 | `extensions/docker/gui/src/DockerTab.js` | 추출 (새 컴포넌트로) |
| `ServerCard.js` L86-88 | `extensions/docker/gui/src/DockerBadge.js` | 추출 |
| `ServerCard.js` L102-104 | `extensions/docker/gui/src/DockerMiniGauge.js` | 추출 |
| `ServerCard.js` L190-201 | `extensions/docker/gui/src/DockerStatsRow.js` | 추출 |
| `AddServerModal.js` L127-147 | `extensions/docker/gui/src/DockerToggle.js` | 추출 |
| `Icon.js` L348 dockerL | 익스텐션 번들에 인라인 | SVG 이동 |
| `App.css` docker 클래스들 | `extensions/docker/gui/src/docker.css` | 추출 |

**각 익스텐션 컴포넌트의 props 인터페이스**:

| 슬롯 | Props | 설명 |
|------|-------|------|
| `ServerCard.badge` | `{ server }` | server.extension_data에서 docker_enabled 확인 |
| `ServerCard.headerGauge` | `{ server }` | server.extension_data.docker_memory_percent |
| `ServerCard.expandedStats` | `{ server, t }` | 게이지 + CPU |
| `ServerSettings.tab` | `{ server, activeTab, setActiveTab, settings, onSettingsChange, t }` | 탭 버튼 + 내용 |
| `AddServer.options` | `{ options, onOptionsChange }` | Docker 토글 체크박스 |

### Task 6.5: 호스트 컴포넌트에서 Docker 코드 → ExtensionSlot

**ServerCard.js**: Docker 배지/게이지/stats를 `<ExtensionSlot />` 3개로 교체
**ServerSettingsModal.js**: DockerTab 관련 코드를 `<ExtensionSlot />` 1개로 교체
**AddServerModal.js**: Docker 토글을 `<ExtensionSlot />` 1개로 교체
**useServerSettings.js**: Docker 필드 → extension_data 기반
**useServerActions.js**: use_docker → extension_data
**index.js**: MemoryGauge export 제거

### Task 6.6: Docker CSS 제거 (App.css)

`.docker-badge`, `.docker-stats-row`, `.memory-gauge-compact`, `.docker-cpu-label`, `.as-docker-row` 제거

### Task 6.7: i18n 분리

`locales/en/gui.json`, `ko/gui.json`, `ja/gui.json`에서 `docker_*` 키 제거.

### Task 6.8: 익스텐션 GUI 빌드

**파일 생성**:
- `extensions/docker/gui/package.json` (react, vite 의존)
- `extensions/docker/gui/vite.config.js` (UMD 라이브러리 모드, react/react-dom external)
- `extensions/docker/gui/src/index.js` (모든 컴포넌트 named export)

```powershell
cd extensions/docker/gui
npm install
npm run build
# → dist/docker-gui.umd.js + dist/style.css
```

### Task 6.9: 빌드 & 테스트

```powershell
cd saba-chan-gui
npm run build
# 빌드 성공 (Docker import 없이)
```

**검증**:
```powershell
Select-String -Path "saba-chan-gui\src\**\*.js" -Pattern "docker|Docker|MemoryGauge|DockerTab" -Recurse |
  Where-Object { $_.Line -notmatch "ExtensionSlot|extension|ext_data" }
# → 0건
```

---

## Phase 7: Cleanup & Final Verification

### Task 7.1: Dead code 제거

```powershell
cargo build 2>&1 | Select-String "warning.*unused"
# 모든 unused warning 해결
```

### Task 7.2: 레거시 파일 삭제

```powershell
Remove-Item extensions/docker_engine.py              # 루트 레거시
Remove-Item saba-chan-gui/extensions/docker_engine.py # GUI 번들 복사본
```

### Task 7.3: 최종 Docker 비활성 테스트

익스텐션 비활성 상태에서 아래 **모두 확인**:

| 확인 항목 | 예상 결과 |
|----------|----------|
| 서버 생성 (Native) | 정상 |
| 서버 시작/정지/상태 (Native) | 정상 |
| 설정 모달 | Docker 탭 없음 |
| AddServer 모달 | Docker 토글 없음 |
| ServerCard | Docker 배지/게이지 없음 |
| `GET /api/servers` | extension_data 비어있음 |

### Task 7.4: 최종 Docker 활성 테스트

익스텐션 활성 상태에서 **T1~T16 전체 재검증** + GUI 확인:

| 추가 확인 항목 | 예상 결과 |
|--------------|----------|
| AddServer에 Docker 토글 등장 | ✓ |
| ServerCard Docker 배지 표시 | ✓ |
| MemoryGauge (미니 44px / 풀 130px) | 정상 렌더링 |
| Docker 설정 탭 (CPU/메모리) | 변경 가능 |
| 기존 instances.json 마이그레이션 | use_docker → extension_data 자동 변환 |

### Task 7.5: grep 최종 점검

```powershell
# 1. Rust src/ 내 docker 직접 참조 0건
Select-String -Path "src\**\*.rs" -Pattern "\bdocker\b" -Recurse |
  Where-Object { $_.Line -notmatch "extension|config_section|ext_data|docker_enabled" }

# 2. GUI src/ 내 Docker 직접 참조 0건
Select-String -Path "saba-chan-gui\src\**\*.js" -Pattern "\bdocker\b|\bDocker\b|MemoryGauge|DockerTab" -Recurse |
  Where-Object { $_.Line -notmatch "ExtensionSlot|extension|ext_data" }

# 3. locales에서 docker 키 0건
Select-String -Path "locales\**\gui.json" -Pattern "docker_" -Recurse

# 4. App.css Docker 클래스 0건
Select-String -Path "saba-chan-gui\src\App.css" -Pattern "docker-badge|docker-stats|docker-cpu|memory-gauge|as-docker-row"

# 모두 0건이면 마이그레이션 완료
```

---

## 부록 A: 파일 변경 매트릭스

| 파일 | 작업 | Phase |
|------|------|:---:|
| `src/extension/mod.rs` | **신규** | 1 |
| `src/ipc/handlers/extension.rs` | **신규** | 1 |
| `src/main.rs` | `mod extension;` 추가 → `mod docker;` 제거 | 1→5 |
| `src/lib.rs` | `pub mod extension;` 추가 → `pub mod docker;` 제거 | 1→5 |
| `src/instance/mod.rs` | extension_data 추가 → Docker 필드 제거 | 3→5 |
| `src/ipc/mod.rs` | extension_data/ExtMgr 추가, Docker 필드/캐시 제거 | 1,3→5 |
| `src/supervisor/mod.rs` | ExtMgr 필드 + Hook 다리 → Docker 분기 제거 | 3→5 |
| `src/ipc/handlers/instance.rs` | Hook 다리 → docker_provision/Docker분기 제거 | 3→5 |
| `src/ipc/handlers/server.rs` | Hook 다리 → Docker 통계 직접 수집 제거 | 3→5 |
| `src/docker/mod.rs` | **삭제** (930줄) | 5 |
| `src/supervisor/module_loader.rs` | DockerExtensionConfig 삭제 → 일반화 | 5 |
| `src/supervisor/migration.rs` | Docker 프로비저닝 → hook 호출 | 5 |
| `extensions/docker/manifest.json` | **신규** | 2 |
| `extensions/docker/compose_manager.py` | **신규** (~600줄) | 2 |
| `extensions/docker/docker_engine.py` | **복사** (→ Phase 7에서 루트 삭제) | 2 |
| `extensions/docker/i18n/*.json` | **신규** (추출) | 2 |
| `extensions/docker_engine.py` | **삭제** | 7 |
| `saba-chan-gui/extensions/docker_engine.py` | **삭제** | 7 |
| `saba-chan-gui/src/contexts/ExtensionContext.js` | **신규** | 6 |
| `saba-chan-gui/src/components/ExtensionSlot.js` | **신규** | 6 |
| `saba-chan-gui/src/App.js` | ExtensionProvider 래핑 | 6 |
| `saba-chan-gui/src/components/MemoryGauge.js` | → 익스텐션 이동 | 6 |
| `saba-chan-gui/src/components/ServerCard.js` | Docker → ExtensionSlot | 6 |
| `saba-chan-gui/src/components/ServerSettingsModal.js` | DockerTab → ExtensionSlot | 6 |
| `saba-chan-gui/src/components/Modals/AddServerModal.js` | Docker토글 → ExtensionSlot | 6 |
| `saba-chan-gui/src/components/Icon.js` | dockerL → 익스텐션 번들 | 6 |
| `saba-chan-gui/src/components/index.js` | MemoryGauge export 제거 | 6 |
| `saba-chan-gui/src/hooks/useServerSettings.js` | extension_data 기반 | 6 |
| `saba-chan-gui/src/hooks/useServerActions.js` | extension_data 기반 | 6 |
| `saba-chan-gui/src/App.css` | Docker CSS 제거 | 6 |
| `locales/*/gui.json` | docker_* 키 제거 | 6 |
| `extensions/docker/gui/*` | **신규** (빌드 설정 + 6 컴포넌트) | 6 |
| `modules/palworld/lifecycle.py` | use_docker 분기 제거 | 5 |

---

## 부록 B: 범용 확장성 설계

### B.1 새 익스텐션 추가 방법 (기존 코드 변경 0줄)

1. `extensions/<name>/manifest.json` 작성 — hooks, gui.slots, instance_fields 선언
2. Python 모듈 구현 — hook 함수 작성
3. (선택) GUI 컴포넌트 — UMD 번들 빌드

**메인 코드 변경 없이 완결.**

### B.2 CLI 확장 (향후)

```json
"cli": {
  "commands": {
    "docker-status": { "module": "compose_manager", "function": "cli_status" }
  }
}
```

`saba-chan-cli`가 `/api/extensions`에서 커맨드 목록 받아 동적 등록.

### B.3 Discord 봇 확장 (향후)

```json
"bot": {
  "commands": {
    "!docker": { "module": "compose_manager", "function": "bot_handler" }
  }
}
```

`discord_bot/index.js`가 `/api/extensions` 조회 후 커맨드 동적 등록.

### B.4 익스텐션 간 의존성 (향후)

```json
{ "dependencies": ["docker"] }
```

`ExtensionManager.enable()` 시 의존성 체크. 현재는 미구현이나 manifest 스키마에 필드는 예약됨.

---

## 부록 C: 예상 작업량

| Phase | 신규(줄) | 수정(줄) | 삭제(줄) | 세션 |
|-------|---------|---------|---------|:---:|
| 1. Infrastructure | ~500 | ~80 | 0 | 1 |
| 2. Docker Python | ~800 | 0 | 0 | 1 |
| 3. Bridge Install | ~300 | ~200 | 0 | 1-2 |
| 4. Bridge Validate | 0 | 0 | 0 | (테스트) |
| 5. Legacy Removal | ~50 | ~100 | ~1400 | 1 |
| 6. GUI Extension | ~600 | ~300 | ~300 | 1-2 |
| 7. Cleanup | 0 | ~50 | ~100 | 0.5 |
| **합계** | **~2250** | **~730** | **~1800** | **4-6** |

---

> **이 문서로 작업을 시작하는 AI 에이전트에게:**
>
> 1. Phase 순서를 절대 건너뛰지 마세요.
> 2. 각 Task 후 명시된 검증 명령을 실행하세요.
> 3. Phase 3-4가 핵심입니다 — **다리를 먼저 놓고, 검증하고, 그 다음에 제거합니다.**
> 4. `compose_manager.py` 작성 시 반드시 `src/docker/mod.rs`의 `generate_compose_yaml()`을 **전부 읽고** 정확히 재현하세요. 이게 제일 중요합니다.
> 5. GUI 슬롯의 props 인터페이스를 잘 설계하세요 — 익스텐션 컴포넌트가 필요한 데이터를 모두 받을 수 있어야 합니다.
> 6. 섹션 0(기능 완전 목록)의 모든 항목(D1-D22, M1-M14, F1-F5, G1-G13, P1-P8)이 마이그레이션 후에도 동작해야 합니다. 하나라도 빠지면 실패입니다.
