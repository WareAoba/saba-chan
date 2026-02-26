# Rust Daemon Codebase Audit Report

**대상**: `c:\Git\saba-chan\src\` (23개 소스 파일)
**범위**: supervisor/, ipc/, python_env/, node_env/, extension/ 모듈

---

## 1. 코드 중복 (Code Duplication)

### 1.1 `start_server` vs `start_managed_server` — 거의 동일한 ~200줄 함수 2개

- **파일**: `src/supervisor/mod.rs`
- **위치**: `start_server` (L112), `start_managed_server` (L694)

두 함수 모두 아래 로직을 사실상 복사-붙여넣기로 반복합니다:

1. 인스턴스 조회 + 실행 중 여부 확인
2. 모듈 메타데이터 로드 + protocols 추출
3. `check_port_conflicts()` 호출 (L129, L711)
4. 확장 hook `server.pre_start` 디스패치
5. config 병합 (module defaults → instance settings)
6. 환경변수 조립
7. log_follower 또는 managed process 스폰
8. 확장 hook `server.post_start` 디스패치
9. running 상태 업데이트 + broadcast

```rust
// L129 (start_server)
let conflicts = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols));
// L711 (start_managed_server) — 동일 코드
let conflicts = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols));
```

**권장**: `prepare_server_start()` 공통 함수로 config 병합, 포트 충돌 검사, hook 디스패치를 추출. 프로세스 스폰 방식만 enum/trait으로 분기.

### 1.2 `python_env` vs `node_env` — 6개 함수 완전 중복

| 함수 | python_env/mod.rs | node_env/mod.rs |
|---|---|---|
| `download_file()` | L360 | L304 |
| `extract_tar_gz()` | L421 | L363 |
| `resolve_data_dir()` | L446 | L430 |
| `platform_data_dir()` | L466/472/478 | L450/456/462 |
| `is_dir_writable()` | L511 | L545 |
| `dir_size_mb()` | L578 | L596 |

이 6개 함수는 로직이 사실상 동일하며 "python" / "node" 문자열과 디렉토리 이름만 다릅니다.

```rust
// python_env/mod.rs L360
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let response = reqwest::get(url).await?;
    // ... 동일 로직
}
// node_env/mod.rs L304 — 동일 구현
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let response = reqwest::get(url).await?;
    // ... 동일 로직
}
```

**권장**: `crate::utils::portable_env` 공용 모듈로 추출. 구체적인 환경별 설정은 trait 또는 config struct로 주입.

### 1.3 `managed_process.rs` — `spawn` vs `spawn_log_follower` 80% 중복

- **파일**: `src/supervisor/managed_process.rs`
- **위치**: `spawn()` (L149), `spawn_log_follower()` (L305)

두 함수 모두:
- stdout/stderr reader 스폰 (tokio::spawn + BufReader)
- 로그 링 버퍼 + broadcast 채널 전송
- 프로세스 종료 대기 task
- PID 등록

```rust
// spawn() L149 — stdout reader
let stdout_log = log.clone();
let stdout_tx = console_tx.clone();
tokio::spawn(async move {
    let reader = BufReader::new(stdout);
    // ...
});

// spawn_log_follower() L305 — 거의 동일한 stdout reader
let stdout_log = log.clone();
let stdout_tx = console_tx.clone();
tokio::spawn(async move {
    let reader = BufReader::new(stdout);
    // ...
});
```

**권장**: `spawn_with_io_capture()` 공통 헬퍼로 추출.

### 1.4 `current_timestamp()` 함수 2곳 중복

- `src/supervisor/managed_process.rs` L565
- `src/supervisor/process.rs` L228

```rust
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
```

완전히 동일한 함수가 같은 크레이트 내 두 파일에 존재.

**권장**: `crate::utils::current_timestamp()`로 통합.

### 1.5 `list_modules` vs `refresh_modules` — 모듈→ExtensionInfo 매핑 로직 복사

- **파일**: `src/ipc/handlers/server.rs`
- **위치**: `list_modules` (L265), `refresh_modules` (L315)

두 핸들러 모두 `ModuleMetadata` → `ExtensionInfo` 변환 코드를 ~30줄 복사.

**권장**: `fn module_to_extension_info(meta: &ModuleMetadata) -> ExtensionInfo` 헬퍼 추출.

### 1.6 `active_ext_data` 수집 패턴 3회 복사

- **파일**: `src/ipc/handlers/extension.rs`
- **위치**: L85, L143, L373

```rust
let active_ext_data = {
    let sup = state.supervisor.read().await;
    sup.instances.iter().map(|(name, inst)| {
        (name.as_str(), &inst.ext_data)
    }).collect::<Vec<_>>()
    // ...
};
```

동일한 인스턴스 ext_data 수집 코드가 `disable_extension`, `unmount_extension`, `remove_extension`에서 반복.

**권장**: `fn collect_active_ext_data(sup: &Supervisor) -> Vec<...>` 헬퍼.

### 1.7 ProcessTracker의 Mutex lock 보일러플레이트

- **파일**: `src/supervisor/process.rs`
- **위치**: L87, L109, L125, L141, L152, L210 등 6+곳

```rust
let mut locked = self.processes.lock().map_err(|e| {
    ProcessError::LockError(format!("Failed to lock process tracker: {}", e))
})?;
```

동일한 `lock().map_err(...)` 패턴이 모든 메서드에서 반복.

**권장**: `fn lock_processes(&self) -> Result<MutexGuard<...>, ProcessError>` 내부 헬퍼.

### 1.8 포트 충돌 검사 3곳 호출

- **파일**: `src/supervisor/mod.rs`
- **위치**: L129 (start_server), L711 (start_managed_server), L1233 (monitor_processes)

동일한 `crate::validator::check_port_conflicts(...)` 호출 + 동일한 에러 응답 JSON 구성이 3곳에서 반복.

### 1.9 ZIP 압축 해제 로직 3곳 중복

| 위치 | 설명 |
|---|---|
| `extension/mod.rs` L421 `extract_zip_extension()` | 확장 ZIP 압축 해제 |
| `extension/mod.rs` L1139 `install_from_url()` | 다운로드된 확장 ZIP 해제 |
| `supervisor/module_loader.rs` L747 부근 | 모듈 ZIP 해제 |

세 곳 모두 `zip::ZipArchive` → `by_index()` → `enclosed_name()` → `create_dir_all/create/copy` 패턴.

**권장**: `crate::utils::extract_zip(archive_path, dest_dir)` 공용 함수.

### 1.10 `set_config` / `parse_update_config` — if-let 필드 추출 2회 반복

- **파일**: `src/ipc/updates.rs`
- **위치**: `set_config` L326-360, `parse_update_config` L425-431

```rust
// L334-358 (set_config)
if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) { config.enabled = v; }
if let Some(v) = body.get("github_owner").and_then(|v| v.as_str()) { config.github_owner = v.to_string(); }
// ... 9개 필드 반복

// L425-431 (parse_update_config) — 동일 패턴
if let Some(v) = val.get("enabled").and_then(|v| v.as_bool()) { cfg.enabled = v; }
if let Some(v) = val.get("github_owner").and_then(|v| v.as_str()) { cfg.github_owner = v.to_string(); }
```

**권장**: serde 역직렬화 사용 (`#[serde(default)]`로 선택적 필드 처리) 또는 공통 `apply_update_config_fields()` 함수.

---

## 2. 죽은 코드 / 미사용 항목 (Dead Code)

### 2.1 `state_machine.rs` — 모듈 전체가 죽은 코드

- **파일**: `src/supervisor/state_machine.rs` (83줄)
- **위치**: L1 `#![allow(dead_code)]`

```rust
#![allow(dead_code)]
// TODO: Supervisor에 통합 필요
pub enum State { Stopped, Starting, Running, Stopping }
```

파일 전체가 `#![allow(dead_code)]`로 덮어씌워져 있으며, TODO 주석이 통합되지 않았음을 인정. `State` enum은 `process.rs`의 `ProcessStatus`와 의미적으로 중복.

**권장**: 통합하거나 삭제.

### 2.2 `error.rs` — `SupervisorError` 완전 미사용

- **파일**: `src/supervisor/error.rs` (78줄)
- **위치**: L8-9 `#[allow(dead_code)] pub enum SupervisorError`

`SupervisorError`는 axum `IntoResponse`까지 구현되어 있지만, Supervisor의 모든 메서드는 `anyhow::Error` 또는 `Ok(json!({"success": false}))` 패턴을 사용. 이 타입을 실제로 사용하는 코드가 전무합니다.

**권장**: 모든 핸들러를 `SupervisorError` 반환으로 마이그레이션하거나, 파일 삭제.

### 2.3 `process.rs` → `ProcessManager` 빈 구조체

- **파일**: `src/supervisor/process.rs`
- **위치**: L290

```rust
pub struct ProcessManager;
impl Default for ProcessManager { fn default() -> Self { Self } }
impl ProcessManager {
    #[allow(dead_code)]
    pub fn new() -> Self { Self }
    // ... 빈 메서드들
}
```

필드도 실제 로직도 없는 빈 구조체. `ProcessTracker`와 역할이 불명확하게 분리.

### 2.4 `process.rs` — 대량 `#[allow(dead_code)]` 어노테이션

- **파일**: `src/supervisor/process.rs`
- 16개의 `#[allow(dead_code)]` 어노테이션 (L8, L38, L46, L87, L109, L125, L141, L152, L210, L227, L229, L290, L292, L298, L306 등). 파일의 대부분 코드가 실질적으로 사용되지 않음을 암시.

### 2.5 `migration.rs` — 컴파일 불가능한 죽은 코드

- **파일**: `src/supervisor/migration.rs`
- **위치**: L7, L13, L101, L131, L226, L259

```rust
use super::extension_loader;  // L7 — 존재하지 않는 모듈 참조
let ext = self.extension_loader.get_extension(module_name)?;  // L13 — Supervisor에 없는 필드
fn detect_server_files(dir: &Path, metadata: &extension_loader::ExtensionMetadata) -> bool {
    // L101 — 존재하지 않는 타입
```

`extension_loader`는 `module_loader`의 이전 이름으로 추정. 리네이밍 후 migration.rs가 업데이트되지 않아 컴파일 불가. 이 파일이 `mod.rs`에서 `mod migration;`으로 선언되지 않았거나 조건부 컴파일로 숨겨져 있을 가능성이 높음.

**권장**: module_loader와 동기화하거나 삭제.

### 2.6 `python_env.rs` 핸들러 — 미등록 라우트

- **파일**: `src/ipc/handlers/python_env.rs`
- **위치**: L5, L12, L28 — 모든 핸들러에 `#[allow(dead_code)]`

```rust
#[allow(dead_code)]
pub async fn python_env_status() -> impl IntoResponse { ... }
#[allow(dead_code)]
pub async fn python_env_setup() -> impl IntoResponse { ... }
#[allow(dead_code)]
pub async fn python_env_pip_install(...) -> impl IntoResponse { ... }
```

`src/ipc/mod.rs`의 라우트 정의에 python_env 핸들러가 등록되지 않음 (node_env만 L515-516에 등록됨).

---

## 3. 스파게티 패턴 (Spaghetti Code)

### 3.1 God Function: `update_instance_settings` (~340줄)

- **파일**: `src/ipc/handlers/instance.rs`
- **위치**: L428부터 ~L768

하나의 함수에서:
1. JSON body 파싱
2. 모듈 메타데이터 조회
3. 포트 필드별 수동 파싱 (Number/String 분기) × 3 (port, rcon_port, rest_port)
4. `known_fields` 하드코딩된 HashSet
5. managed_start 자동 활성화 로직
6. RCON 자동 설정 로직
7. 확장 필드 기본값 병합
8. 인스턴스 저장

```rust
// L428 — 400줄짜리 단일 함수
pub async fn update_instance_settings(...) -> impl IntoResponse {
    // ... 포트 파싱만 60줄+
    if let Some(port_val) = settings.get("port") {
        if let Some(n) = port_val.as_u64() { ... }
        else if let Some(s) = port_val.as_str() { s.parse::<u16>()... }
    }
    if let Some(rcon_val) = settings.get("rcon_port") { /* 동일 패턴 */ }
    if let Some(rest_val) = settings.get("rest_port") { /* 동일 패턴 */ }
```

**권장**: `parse_port_field()` 헬퍼 추출 + 설정 업데이트 로직을 Supervisor 메서드로 이동 + auto-configure 로직 분리.

### 3.2 God Function: `list_servers` (~240줄)

- **파일**: `src/ipc/handlers/server.rs`
- **위치**: L24부터 ~L264

서버 리스팅에 인스턴스 조회, 프로세스 상태 수집, 모듈 메타데이터 병합, 확장 hook 디스패치, 응답 JSON 조립이 모두 단일 함수에.

### 3.3 God Function: `start_server` / `start_managed_server` (~200줄 each)

- **파일**: `src/supervisor/mod.rs`
- **위치**: L112 / L694

위 §1.1에서 기술한 중복 외에도, 각 함수가 config 병합 → 포트 검사 → hook 디스패치 → 프로세스 스폰 → 상태 업데이트 → 로그 전송 모두를 단일 함수에서 수행.

### 3.4 `dispatch_hook_with_progress` — `on_progress` 콜백 첫 hook에서 break

- **파일**: `src/extension/mod.rs`
- **위치**: L947-1019

```rust
pub async fn dispatch_hook_with_progress<F>(..., on_progress: F) -> ...
where F: Fn(ExtensionProgress) + Send + 'static,
{
    for (ext, binding) in hooks {
        // ...
        let result = crate::plugin::run_plugin_with_progress(
            &module_path, &binding.function, context.clone(), on_progress,  // on_progress move됨
        ).await;
        // ...
        break;  // L1019 — "progress 콜백은 한 번만 소비 가능하므로 첫 번째만"
    }
}
```

`on_progress`가 move 시맨틱이라 첫 번째 확장에서만 호출 가능하고, 나머지는 강제 break. chain-of-responsibility 패턴이 완전히 무력화됨.

**권장**: `Arc<dyn Fn(ExtensionProgress) + Send + Sync>`로 변경하거나, 콜백을 `&dyn Fn(...)` 참조로 전달.

### 3.5 핸들러 보일러플레이트 패턴

- **파일**: `src/ipc/handlers/managed.rs` (전체), `command.rs`, `extension.rs` 등

모든 핸들러가 동일 패턴:
```rust
pub async fn some_handler(State(state): State<IPCServer>, ...) -> impl IntoResponse {
    let sup = state.supervisor.read().await;
    // ... 작업
    match result {
        Ok(val) => Json(json!({"success": true, ...})).into_response(),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})).into_response(),
    }
}
```

**권장**: 매크로 또는 공통 래퍼 함수로 줄일 수 있음.

---

## 4. 타입 오용 (Type Misuse)

### 4.1 과도한 `String` 소유권 — `&str`이면 충분한 곳

전체 코드베이스에서 단기적으로만 사용되는 문자열에도 `.to_string()`, `.clone()` 호출:

```rust
// supervisor/mod.rs — 곳곳
let module_name = module_name.to_string();  // 이미 &str인데 소유권 불필요
let server_name = server_name.to_string();
```

### 4.2 `json!({"success": false, "error": ...})` 반환 vs 타입 안전 에러

- **파일**: `src/supervisor/mod.rs`, `src/ipc/handlers/*.rs` 전반

```rust
// supervisor/mod.rs L135-140
return Ok(json!({
    "success": false,
    "error": "port_conflict",
    "error_code": "port_conflict",
    "conflicts": conflicts,
}));
```

`Result<Value>`를 반환하면서 에러 경우에도 `Ok(json!({success:false}))`를 사용. 이는 `SupervisorError` (error.rs에 정의됨)를 사용해야 하는 곳에서 에러 타입 시스템을 완전히 우회.

**권장**: `SupervisorError` 또는 전용 응답 enum으로 통일.

### 4.3 `Value` (serde_json) 과용

config, instance settings, 확장 데이터 등이 모두 `serde_json::Value`로 처리되어 타입 안전성이 없음:

```rust
// supervisor/mod.rs
pub struct Supervisor {
    pub instances: HashMap<String, Value>,  // 타입 없는 JSON blob
    // ...
}
```

**권장**: 핵심 구조체는 강타입 struct로 정의. `Value`는 외부 경계(API 직렬화)에서만 사용.

### 4.4 `ProcessError::NotFound` — 항상 pid: 0

- **파일**: `src/supervisor/process.rs`

```rust
ProcessError::NotFound { pid: 0 }  // 실제 PID가 아닌 0을 항상 사용
```

**권장**: 실제 PID를 전달하거나, `NotFound`에서 PID 필드 제거.

---

## 5. 에러 처리 (Error Handling)

### 5.1 3가지 에러 패턴 혼재

코드베이스에서 세 가지 서로 다른 에러 처리 패턴이 혼재:

| 패턴 | 사용처 |
|---|---|
| `Err(anyhow::anyhow!(...))` | supervisor/mod.rs, extension/mod.rs |
| `Ok(json!({"success": false, "error": ...}))` | supervisor/mod.rs start_server 등 |
| `SupervisorError` (thiserror) | error.rs에 정의만, 실제 미사용 |

```rust
// 패턴 혼재 예시 — 같은 파일 내
fn start_server(...) -> Result<Value> {
    // 포트 충돌 → Ok(json!({"success": false}))  패턴 A
    return Ok(json!({"success": false, "error": "port_conflict"}));
    // 파일 I/O 에러 → anyhow::Error  패턴 B
    std::fs::read_to_string(&path)?;
}
```

### 5.2 `.unwrap()` / `.unwrap_or_default()` 남용

```rust
// managed_process.rs — stdout 읽기 실패를 무시
let line = line.unwrap_or_default();
// process.rs L228
.unwrap_or_default()  // SystemTime 역행 시 0 반환 (로깅 없음)
```

### 5.3 에러 컨텍스트 부족

```rust
// extension/mod.rs
let file = std::fs::File::open(&zip_path)?;  // 어떤 zip인지 컨텍스트 없음
// vs 좋은 예
let file = std::fs::File::open(&zip_path)
    .with_context(|| format!("Failed to open extension zip: {}", zip_path.display()))?;
```

`.with_context()`를 사용하는 곳도 있지만, 상당수 `?` 전파에는 컨텍스트가 없음.

### 5.4 `to_string_lossy()` 조용한 데이터 손실

```rust
// extension/mod.rs L886
let module_path = module_file.to_string_lossy().to_string();
// python_env, node_env에서도 동일
```

비-UTF8 경로에서 데이터가 조용히 손실됨. 로깅이나 에러 반환 없음.

---

## 6. 구조적 문제 (Structural Issues)

### 6.1 "모듈" vs "확장" 개념 혼란

- `ipc/mod.rs`에서 `ExtensionInfo` 구조체가 module 데이터를 담는 데 사용됨
- `ipc/handlers/server.rs`의 `list_modules`, `refresh_modules`가 `ModuleMetadata` → `ExtensionInfo`로 변환
- `supervisor/module_loader.rs`는 "module"을, `extension/mod.rs`는 "extension"을 다루는데, IPC 계층에서 이 두 개념이 같은 타입(`ExtensionInfo`)으로 표현됨

### 6.2 `migration.rs` — 잘못된 모듈 참조

- **파일**: `src/supervisor/migration.rs` L7

```rust
use super::extension_loader;  // supervisor/extension_loader가 아닌 supervisor/module_loader여야 함
```

이 파일은 리네이밍 이전의 API를 참조하며, 현재 코드와 호환되지 않음. `mod.rs`에서 `mod migration;` 선언을 제거하거나 조건부 컴파일로 숨겼을 가능성 있음.

### 6.3 `PortConflictInfo` vs `PortConflictStopEvent` 이중 정의

- `src/ipc/mod.rs` — `PortConflictInfo` struct
- `src/supervisor/mod.rs` L43 — `PortConflictStopEvent` struct

유사한 정보를 담지만 별개 타입. 포트 충돌 표현이 통일되지 않음.

### 6.4 `Supervisor`가 God Object

- **파일**: `src/supervisor/mod.rs` (1393줄)

`Supervisor` struct가 인스턴스 관리, 프로세스 추적, 모듈 로딩, 확장 관리, 포트 충돌 감지, 마이그레이션, 로그 브로드캐스트를 모두 담당. Single Responsibility 원칙 위반.

**권장**: `InstanceManager`, `ProcessSupervisor`, `PortManager` 등으로 분리.

### 6.5 `extension/mod.rs` — 1645줄 모놀리스

단일 파일에 15+ public 메서드, 8개 struct/enum 정의, ZIP 처리, 레지스트리 통신, hook 디스패치, 상태 영속화, i18n 로드 등이 모두 포함.

**권장**: `extension/manifest.rs`, `extension/hooks.rs`, `extension/registry.rs`, `extension/state.rs` 등으로 분리.

### 6.6 `copy_dir_all` vs `crate::utils::copy_dir_recursive` 중복

- `src/supervisor/module_loader.rs` L1019 — 로컬 `copy_dir_all()` 함수
- `src/supervisor/migration.rs` L165 — `crate::utils::copy_dir_recursive()` 호출

동일 기능의 함수가 유틸리티 모듈과 module_loader에 각각 존재.

---

## 7. 네이밍 문제 (Naming Issues)

### 7.1 `ExtensionInfo` — 모듈과 확장을 모두 표현

- **파일**: `src/ipc/mod.rs`

```rust
pub struct ExtensionInfo {  // 실제로는 ModuleInfo + ExtensionInfo 양쪽에 사용
    pub id: String,
    pub name: String,
    // ...
}
```

### 7.2 `extension_loader` 레거시 이름

- **파일**: `src/supervisor/migration.rs` L7

`module_loader`로 리네이밍된 후에도 이전 이름이 잔존.

### 7.3 `spawn` vs `spawn_log_follower` 이름이 역할을 구분하지 못함

- **파일**: `src/supervisor/managed_process.rs`

`spawn`은 직접 실행, `spawn_log_follower`는 외부 실행기에 의한 프로세스 모니터링인데, 이름만으로는 차이를 알기 어려움. `spawn_managed`, `attach_to_external` 같은 이름이 더 명확.

### 7.4 `ServerInfo` 18+ 필드

- **파일**: `src/ipc/mod.rs`

과도한 필드 수가 struct 이름의 "info" 레벨 추상화와 불일치. 실제로는 서버의 전체 상태 스냅샷.

---

## 8. 하드코딩된 값 (Hardcoded Values)

| 값 | 위치 | 설명 |
|---|---|---|
| `25575` | `handlers/command.rs` L86 | RCON 기본 포트 |
| `8212` | `handlers/command.rs` L86, L223 | REST API 기본 포트 |
| `"127.0.0.1"` | `handlers/command.rs` L104, L223 | RCON/REST 호스트 |
| `3` | `handlers/command.rs` L130 부근 | RCON 재시도 횟수 |
| `500ms` | `handlers/command.rs` L138 | 재시도 딜레이 |
| `90초` | `ipc/mod.rs` (client timeout) | 클라이언트 타임아웃 |
| `30초` | `ipc/auth.rs` | 인증 실패 rate limit |
| `1000` | `managed_process.rs` | 로그 링 버퍼 크기 |
| `"lifecycle.py"` | `supervisor/mod.rs` | 플러그인 진입점 파일명 |
| `"extensions_state.json"` | `extension/mod.rs` | 상태 파일명 |

```rust
// handlers/command.rs L86
Err(_) => (25575, 8212),  // 매직 넘버
// handlers/command.rs L104
let rcon_host = "127.0.0.1".to_string();
// handlers/command.rs L138
tokio::time::sleep(std::time::Duration::from_millis(500)).await;
```

**권장**: `const` 또는 config 파일에서 로드.

---

## 9. 불필요한 변환 (Redundant Conversions)

### 9.1 불필요한 `.to_string()` + `.clone()` 체인

```rust
// 여러 파일에서 반복
let id = ext.manifest.id.clone();  // &str로 충분한 스코프에서
let name = server_name.to_string();  // 바로 format! 매크로에 넣을 수 있음
```

### 9.2 `to_string_lossy().to_string()`

```rust
// extension/mod.rs L886, python_env/mod.rs, node_env/mod.rs 여러 곳
let module_path = module_file.to_string_lossy().to_string();
```

`to_string_lossy()`는 `Cow<str>`을 반환하므로 `.to_string()`은 항상 새 String 할당. `Cow`를 직접 사용하거나 `&str`로 빌려 쓰면 됨.

### 9.3 `serde_json::to_string()` → `parse::<Value>()` 라운드트립

일부 코드에서 JSON Value를 문자열로 직렬화한 후 다시 역직렬화하는 패턴 존재.

### 9.4 `context.clone()` in hook dispatch loop

```rust
// extension/mod.rs L896
let result = crate::plugin::run_plugin_with_timeout(
    &module_path, &binding.function,
    context.clone(),  // 루프의 마지막 반복에서도 불필요하게 clone
    timeout_secs,
).await;
```

마지막 반복에서는 clone 없이 move 가능하지만, 일관되게 clone 사용.

---

## 10. 과잉/과소 엔지니어링 (Over/Under-Engineering)

### 10.1 과잉: `state_machine.rs` — 미사용 상태 머신

83줄의 상태 전이 로직이 설계만 되고 통합되지 않음. `ServerStateMachine`은 어디서도 인스턴스화되지 않음.

### 10.2 과잉: `SupervisorError` — 정교하지만 미사용

HTTP 상태 코드 매핑, `IntoResponse` 구현까지 되어있으나 실제 사용처 없음.

### 10.3 과잉: `ProcessTracker` + `ProcessManager` 이중 추상화

`ProcessManager`는 빈 구조체이고, `ProcessTracker`가 실제 작업을 전담. 불필요한 추상화 계층.

### 10.4 과소: `fetch_registry()` 스텁

- **파일**: `src/extension/mod.rs`

```rust
pub async fn fetch_registry(&self) -> Vec<RegistryExtension> {
    // TODO: 실제 레지스트리 서버 연동
    Vec::new()
}
```

실제 구현 없이 빈 Vec만 반환. 이를 의존하는 `check_updates_against()`, `install_from_url()` 등이 사실상 동작 불가.

### 10.5 과소: SHA256 검증 미구현

- **파일**: `src/extension/mod.rs` L1139

```rust
pub async fn install_from_url(
    &self, ext_id: &str, download_url: &str,
    _expected_sha256: Option<&str>,  // 언더스코어 — 미사용
) -> Result<()> {
    // TODO: sha256 검증 로직 구현
```

다운로드된 확장의 무결성 검증이 구현되지 않음. 보안 위험.

### 10.6 과소: 핸들러에 입력 검증 부재

`update_instance_settings`에서 포트 번호 범위 검증 없음. 서버명 특수문자 검증 없음. 설정 값의 min/max 검증 없음.

### 10.7 과소: `module_loader.rs`의 `discover_modules()` 캐시 전략

```rust
pub fn discover_modules(&self) -> Vec<ModuleMetadata> {
    // RwLock 획득 후 매번 전체 디스크 스캔
}
```

호출마다 전체 디렉토리를 다시 스캔. 캐시 무효화 전략이 없음 (변경 감지 없이 무조건 재스캔).

---

## 요약 통계

| 카테고리 | 이슈 수 | 심각도 |
|---|---|---|
| 코드 중복 | 10 | 🔴 높음 |
| 죽은 코드 | 6 | 🟡 중간 |
| 스파게티 패턴 | 5 | 🔴 높음 |
| 타입 오용 | 4 | 🟡 중간 |
| 에러 처리 | 4 | 🔴 높음 |
| 구조적 문제 | 6 | 🔴 높음 |
| 네이밍 | 4 | 🟢 낮음 |
| 하드코딩 | 10+ | 🟡 중간 |
| 불필요한 변환 | 4 | 🟢 낮음 |
| 과잉/과소 엔지니어링 | 7 | 🟡 중간 |

## 우선 리팩토링 권장사항

1. **`python_env` / `node_env` 공통 모듈 추출** — 가장 명확한 중복 제거 (6개 동일 함수)
2. **`start_server` / `start_managed_server` 공통 로직 추출** — God function 해소 + 중복 제거
3. **에러 처리 통일** — `SupervisorError` 활용하거나 일관된 `Result<T, AppError>` 패턴 도입
4. **죽은 코드 정리** — `state_machine.rs`, `error.rs` (미사용 시), `ProcessManager`, `migration.rs` 수정 or 삭제
5. **`update_instance_settings` 분해** — 400줄 함수를 포트 파싱, auto-config, 확장 필드 병합으로 분리
6. **ZIP 추출 유틸리티 통합** — 3곳의 중복 ZIP 처리를 `crate::utils::extract_zip()`으로
7. **`dispatch_hook_with_progress` 콜백 문제 수정** — `Arc<dyn Fn>` 사용으로 multi-extension progress 지원
