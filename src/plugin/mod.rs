use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::io::{AsyncWriteExt, BufReader};
use crate::utils::apply_creation_flags;

mod loopback;
pub(crate) mod host;

/// 데몬 IPC 포트. 모듈이 데몬 API를 역호출할 때 쓴다(main이 기동 시 한 번 설정).
static IPC_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

pub fn set_ipc_port(port: u16) {
    let _ = IPC_PORT.set(port);
}

/// 데몬 API 포트. HOST·LOOPBACK 중계는 이 포트로 잇지 않는다
pub(crate) fn ipc_port() -> u16 {
    *IPC_PORT.get().unwrap_or(&saba_chan_updater_lib::constants::DEFAULT_IPC_PORT)
}

/// Python 실행마다 넣는 데몬 연결 정보.
/// - `SABA_IPC_PORT`, `SABA_IPC_URL`: 데몬 API 주소
/// - `SABA_TOKEN_PATH`: `X-Saba-Token` 값이 든 파일 (토큰 자체는 env에 넣지 않는다)
/// - `SABA_LANG`: 요청이 고른 언어(Accept-Language), 없으면 지금 GUI 언어(고르기 전에는 OS 언어). 언어를 바꿔도 데몬은 재시작하지 않으므로 매번 읽는다.
/// - `TEMP`·`TMP`·`TMPDIR`·`PIP_CACHE_DIR`: 데이터 폴더 안 (제거기가 지운다. 시스템 임시 폴더·사용자 pip 캐시에 흔적을 남기지 않는다).
///   격리 실행은 그 뒤에 자기 임시 폴더로 덮는다
fn daemon_env() -> Vec<(&'static str, String)> {
    let port = ipc_port();
    let mut env = vec![
        ("SABA_IPC_PORT", port.to_string()),
        ("SABA_IPC_URL", format!("http://127.0.0.1:{}", port)),
        (
            "SABA_TOKEN_PATH",
            saba_chan_updater_lib::constants::token_file_path().to_string_lossy().into_owned(),
        ),
    ];
    let lang = crate::audit::current().lang.unwrap_or_else(|| {
        saba_chan_updater_lib::config_file::ConfigFile::default_location()
            .section("gui")
            .get("language")
            .and_then(|v| v.as_str())
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            // 언어를 고르기 전에는 GUI처럼 OS 언어를 따른다
            .unwrap_or_else(crate::chat::i18n::os_language)
    });
    env.push(("SABA_LANG", lang));
    let temp = saba_chan_updater_lib::constants::resolve_temp_dir();
    if let Err(e) = std::fs::create_dir_all(&temp) {
        tracing::warn!("Cannot create {}: {}", temp.display(), e);
    }
    let temp = temp.to_string_lossy().into_owned();
    env.extend([("TEMP", temp.clone()), ("TMP", temp.clone()), ("TMPDIR", temp)]);
    env.push(("PIP_CACHE_DIR", saba_chan_updater_lib::constants::resolve_pip_cache_dir().to_string_lossy().into_owned()));
    env
}

/// 데몬 환경에서 물려받지 않는 변수 이름 (M12 실행 시 제한): 이름에 이 말이 든 것.
/// 모듈·익스텐션 코드는 데몬이 넣어 주는 값(`daemon_env`)과 평범한 시스템 변수만 본다
const SECRET_ENV_MARKERS: &[&str] = &["TOKEN", "SECRET", "PASSWORD", "PASSWD", "API_KEY", "APIKEY", "PRIVATE_KEY", "CREDENTIAL", "ACCESS_KEY"];

fn is_secret_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper != "SABA_TOKEN_PATH" && SECRET_ENV_MARKERS.iter().any(|m| upper.contains(m))
}

/// 시간 초과한 Python과 그것이 띄운 프로세스를 함께 끝낸다.
/// Unix는 자기 프로세스 그룹으로 띄우므로 그룹째, Windows는 `taskkill /T`
pub(crate) fn kill_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        nix::libc::kill(-(pid as i32), nix::libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new(saba_chan_updater_lib::process::system_tool("taskkill"));
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = cmd.output();
    }
}

/// Progress info emitted by Python extensions via stderr "PROGRESS:{json}" lines
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExtensionProgress {
    pub percent: Option<u8>,
    pub message: Option<String>,
    /// 현재 단계 인덱스 (0-based)
    pub step: Option<u8>,
    /// 전체 단계 수
    pub total: Option<u8>,
    /// 현재 단계 식별자 (예: "checking_engine")
    pub label: Option<String>,
}

/// extensions/ 디렉토리 경로를 해석합니다.
///
/// 고정 경로: `%APPDATA%/saba-chan/extensions` (Windows)
///            `~/.config/saba-chan/extensions` (Linux/macOS)
///
/// `SABA_EXTENSIONS_DIR` 환경 변수가 설정되면 해당 경로를 우선 사용합니다 (테스트/개발용).
///
/// 디렉토리가 존재하지 않으면 생성을 시도합니다.
pub fn resolve_extensions_dir() -> std::path::PathBuf {
    let candidate = saba_chan_updater_lib::constants::resolve_extensions_dir();
    if !candidate.exists() {
        let _ = std::fs::create_dir_all(&candidate);
    }
    candidate
}

/// modules/ 디렉토리 경로를 해석합니다.
///
/// 고정 경로: `%APPDATA%/saba-chan/modules` (Windows)
///            `~/.config/saba-chan/modules` (Linux/macOS)
///
/// `SABA_MODULES_PATH` 환경 변수가 설정되면 해당 경로를 우선 사용합니다 (테스트/개발용).
///
/// 디렉토리가 존재하지 않으면 생성을 시도합니다.
pub fn resolve_modules_dir() -> std::path::PathBuf {
    let candidate = saba_chan_updater_lib::constants::resolve_modules_dir();
    if !candidate.exists() {
        let _ = std::fs::create_dir_all(&candidate);
    }
    candidate
}

/// import 가드(`sitecustomize.py`)를 둔 폴더. 데이터 폴더에 한 번 쓰고, 내용이 바뀌었을 때만 다시 쓴다.
fn import_guard_dir() -> Option<std::path::PathBuf> {
    const GUARD: &str = include_str!("sitecustomize.py");
    let dir = saba_chan_updater_lib::constants::resolve_data_dir().join("python-guard");
    let file = dir.join("sitecustomize.py");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(GUARD) {
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, GUARD)) {
            tracing::warn!("Cannot write the extension import guard to {}: {}", file.display(), e);
            return None;
        }
    }
    Some(dir)
}

/// 데몬에 넣어 둔 공용 Python 유틸리티. 모듈 폴더에 이것과 다른 판이 있으면 덮어쓴다.
/// 설치·업데이트 경로와 상관없이(개발 빌드를 복사만 해도) 모듈이 데몬과 같은 판을 쓴다 (C7)
const SHARED_MODULES: [(&str, &str); 3] = [
    ("i18n.py", include_str!("../../modules/i18n.py")),
    ("daemon_rcon.py", include_str!("../../modules/daemon_rcon.py")),
    // 모듈 코드 검사기 (M12): 적합성 키트가 `--shared-libs`에서 찾는다
    ("saba_audit.py", crate::module_audit::SCRIPT),
];

/// 공용 유틸리티를 `dir`에 맞춰 둔다. 내용이 같으면 쓰지 않는다
pub fn ensure_shared_modules(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, content) in SHARED_MODULES {
        let file = dir.join(name);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(content) {
            std::fs::write(&file, content)?;
            tracing::info!("Shared module refreshed: {}", file.display());
        }
    }
    Ok(())
}

/// 기본 플러그인 타임아웃 (초)
pub const DEFAULT_PLUGIN_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
pub struct ModuleCallError {
    response: Value,
}

impl ModuleCallError {
    pub fn from_response(response: Value) -> Self {
        Self { response }
    }
}

impl std::fmt::Display for ModuleCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.response.get("error").or_else(|| self.response.get("message")).and_then(Value::as_str).unwrap_or("Module call failed"))
    }
}

impl std::error::Error for ModuleCallError {}

pub fn error_response(error: &anyhow::Error) -> Value {
    error.downcast_ref::<ModuleCallError>().map_or_else(
        || serde_json::json!({"success": false, "error": error.to_string()}),
        |error| error.response.clone(),
    )
}

const INSTALL_SLOTS: usize = 2;
const GENERAL_SLOTS: usize = 8;
const STDOUT_LIMIT: usize = 4 * 1024 * 1024;
const STDERR_LINES: usize = 200;
const STDERR_BYTES: usize = 64 * 1024;

tokio::task_local! {
    static CALL_DEADLINE: CallDeadline;
    static HOST_CONTEXT: Option<host::Context>;
}

#[derive(Clone, Default)]
struct IsolationTime {
    elapsed: std::time::Duration,
    started: Option<tokio::time::Instant>,
    waiting: usize,
}

impl IsolationTime {
    fn elapsed(&self) -> std::time::Duration {
        self.elapsed + self.started.map_or(std::time::Duration::ZERO, |started| started.elapsed())
    }
}

#[derive(Clone)]
struct CallDeadline {
    at: tokio::time::Instant,
    isolation: tokio::sync::watch::Sender<IsolationTime>,
}

struct IsolationWait(tokio::sync::watch::Sender<IsolationTime>);

impl Drop for IsolationWait {
    fn drop(&mut self) {
        self.0.send_modify(|time| {
            time.waiting -= 1;
            if time.waiting == 0 {
                time.elapsed += time.started.take().expect("isolation wait started").elapsed();
            }
        });
    }
}

impl CallDeadline {
    fn new(at: tokio::time::Instant) -> Self {
        Self { at, isolation: tokio::sync::watch::channel(IsolationTime::default()).0 }
    }

    fn until(&self, at: tokio::time::Instant) -> Self {
        Self { at: self.at.min(at - self.isolation.borrow().elapsed()), isolation: self.isolation.clone() }
    }

    fn remaining(&self) -> std::time::Duration {
        (self.at + self.isolation.borrow().elapsed()).saturating_duration_since(tokio::time::Instant::now())
    }

    fn pause_isolation(&self) -> IsolationWait {
        self.isolation.send_modify(|time| {
            if time.waiting == 0 { time.started = Some(tokio::time::Instant::now()); }
            time.waiting += 1;
        });
        IsolationWait(self.isolation.clone())
    }

    /// 격리 준비(프로필·폴더 권한)를 기다리는 동안은 기한에서 뺀다
    async fn paused<T>(&self, work: impl std::future::Future<Output = T>) -> T {
        let _waiting = self.pause_isolation();
        work.await
    }

    async fn follow_isolation<T>(&self, source: &Self, work: impl std::future::Future<Output = T>) -> T {
        if self.isolation.same_channel(&source.isolation) { return work.await; }
        let source_start = source.isolation.borrow().elapsed();
        let own_start = self.isolation.borrow().elapsed();
        let mut isolation = source.isolation.subscribe();
        let mut waiting = None;
        tokio::pin!(work);
        loop {
            if isolation.borrow_and_update().waiting > 0 {
                if waiting.is_none() { waiting = Some(self.pause_isolation()); }
            } else {
                waiting = None;
            }
            tokio::select! {
                biased;
                _ = isolation.changed() => {},
                result = &mut work => {
                    drop(waiting);
                    let excluded = source.isolation.borrow().elapsed().saturating_sub(source_start);
                    let followed = self.isolation.borrow().elapsed().saturating_sub(own_start);
                    // watch는 중간 상태를 합칠 수 있으므로 다음 실행 전에 누적 제외 시간을 맞춘다.
                    self.isolation.send_modify(|time| time.elapsed = (time.elapsed + excluded).saturating_sub(followed));
                    return result;
                },
            }
        }
    }

    async fn timeout<T>(&self, work: impl std::future::Future<Output = T>) -> std::result::Result<T, ()> {
        let mut isolation = self.isolation.subscribe();
        tokio::pin!(work);
        loop {
            let at = {
                let time = isolation.borrow_and_update();
                (time.waiting == 0).then_some(self.at + time.elapsed)
            };
            let timer = async {
                if let Some(at) = at { tokio::time::sleep_until(at).await; }
                else { std::future::pending::<()>().await; }
            };
            tokio::select! {
                biased;
                _ = isolation.changed() => {},
                _ = timer => {
                    let time = isolation.borrow();
                    if time.waiting == 0 && tokio::time::Instant::now() >= self.at + time.elapsed { return Err(()); }
                },
                result = &mut work => return Ok(result),
            }
        }
    }
}

pub(crate) async fn with_instance<T>(context: Option<host::Context>, work: impl std::future::Future<Output = T>) -> T {
    HOST_CONTEXT.scope(context, work).await
}

struct CallLanes {
    install: tokio::sync::Semaphore,
    general: tokio::sync::Semaphore,
}

struct CallSlot<'a> {
    _permit: Option<tokio::sync::SemaphorePermit<'a>>,
}

impl CallLanes {
    const fn new() -> Self {
        Self { install: tokio::sync::Semaphore::const_new(INSTALL_SLOTS), general: tokio::sync::Semaphore::const_new(GENERAL_SLOTS) }
    }

    async fn acquire(&self, function: &str, deadline: tokio::time::Instant, skip_busy: bool) -> std::result::Result<Option<CallSlot<'_>>, tokio::time::error::Elapsed> {
        // 같은 작업의 내부 조회·재진입은 바깥 호출의 자리를 빌린다
        if CALL_DEADLINE.try_with(|_| ()).is_ok() {
            return Ok(Some(CallSlot { _permit: None }));
        }
        let lane = if function == "install_server" { &self.install } else { &self.general };
        let permit = if skip_busy {
            let Ok(permit) = lane.try_acquire() else { return Ok(None) };
            permit
        } else {
            tokio::time::timeout_at(deadline, lane.acquire()).await?.expect("plugin lanes stay open")
        };
        Ok(Some(CallSlot { _permit: Some(permit) }))
    }
}

#[cfg(not(test))]
fn call_lanes() -> &'static CallLanes {
    static CALL_LANES: CallLanes = CallLanes::new();
    &CALL_LANES
}

#[cfg(test)]
fn call_lanes() -> std::sync::Arc<CallLanes> {
    // 각 테스트 런타임은 독립 데몬이다. 같은 런타임의 작업은 같은 상한을 쓴다.
    static LANES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<tokio::runtime::Id, std::sync::Arc<CallLanes>>>> = std::sync::LazyLock::new(Default::default);
    LANES.lock().unwrap_or_else(|e| e.into_inner())
        .entry(tokio::runtime::Handle::current().id()).or_insert_with(|| std::sync::Arc::new(CallLanes::new())).clone()
}

fn module_name(entry: &str) -> String {
    let path = std::path::Path::new(entry);
    for dir in path.parent().into_iter().flat_map(std::path::Path::ancestors) {
        if let Some(name) = std::fs::read_to_string(dir.join("module.toml")).ok().and_then(|text| text.parse::<toml::Table>().ok())
            .and_then(|value| value.get("module")?.get("name")?.as_str().map(str::to_string)) {
            return name;
        }
        if let Some(id) = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.get("id")?.as_str().map(str::to_string)) {
            return id;
        }
    }
    path.parent().and_then(std::path::Path::file_name).unwrap_or_default().to_string_lossy().into_owned()
}

fn call_error(entry: &str, function: &str, code: &str, phase: &str, error: impl std::fmt::Display, stderr_tail: Vec<String>) -> anyhow::Error {
    ModuleCallError::from_response(serde_json::json!({
        "success": false, "error_code": code, "error": error.to_string(),
        "module": module_name(entry), "function": function, "phase": phase, "stderr_tail": stderr_tail,
    })).into()
}

fn sandbox_failure(entry: &str, function: &str, component: &str, blocked: &crate::sandbox::Blocked) -> Value {
    let message = crate::sandbox::message(blocked);
    let mut value = error_response(&call_error(entry, function, "module_execution_failed", "prepare", &message, Vec::new()));
    if let Some(fields) = blocked.to_json(message).as_object() {
        value.as_object_mut().expect("call error object").extend(fields.clone());
    }
    value["component"] = Value::String(component.to_string());
    value
}

async fn prepare<T>(entry: &str, function: &str, deadline: CallDeadline, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    match deadline.timeout(future).await {
        Ok(result) => result.map_err(|error| call_error(entry, function, "module_execution_failed", "prepare", error, Vec::new())),
        Err(_) => Err(call_error(entry, function, "module_timeout", "prepare", format!("{}.{} preparation did not finish before the deadline", module_name(entry), function), Vec::new())),
    }
}

fn decorate_result(entry: &str, function: &str, mut result: Value, stderr: &str) -> Value {
    if result.get("success").and_then(Value::as_bool) == Some(false) {
        result["module"] = Value::String(module_name(entry));
        result["function"] = Value::String(function.to_string());
        if result.get("error_code").and_then(Value::as_str) == Some("module_exception") {
            result["stderr_tail"] = serde_json::json!(stderr_tail(stderr));
        }
    }
    result
}

fn stderr_tail(stderr: &str) -> Vec<String> {
    let lines: Vec<_> = stderr.lines().collect();
    lines.iter().skip(lines.len().saturating_sub(20)).map(|line| (*line).to_string()).collect()
}

async fn in_call<T>(entry: &str, function: &str, timeout_secs: u64, skip_busy: bool, work: impl std::future::Future<Output = Result<T>>) -> Result<Option<T>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let call_deadline = CALL_DEADLINE.try_with(|outer| outer.until(deadline)).unwrap_or_else(|_| CallDeadline::new(deadline));
    let lanes = call_lanes();
    let slot = lanes.acquire(function, deadline, skip_busy).await
        .map_err(|_| call_error(entry, function, "module_timeout", "queue", format!("{}.{} did not obtain a slot in {} s", module_name(entry), function, timeout_secs), Vec::new()))?;
    let Some(_slot) = slot else { return Ok(None) };
    CALL_DEADLINE.scope(call_deadline, work).await.map(Some)
}

/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
/// Called by Supervisor for module lifecycle management
/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
pub async fn run_plugin(module_path: &str, function: &str, config: Value) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, DEFAULT_PLUGIN_TIMEOUT_SECS, None).await
}

/// 데몬 락 안에서 만들고 락을 놓은 뒤 실행하는 모듈 호출 (C12).
/// Python은 몇 분까지 걸릴 수 있어서, 락을 쥔 채 기다리면 모니터와 다른 요청이 모두 멈춘다.
pub struct PluginCall {
    entry: String,
    function: String,
    config: Value,
    /// 데몬이 추적하는 그 인스턴스의 서버 PID: 격리 실행의 루프백 중계는 이 프로세스(트리)가 듣는 포트로만 잇는다
    server: Option<u32>,
    timeout_secs: u64,
    host: Option<host::Context>,
}

impl PluginCall {
    pub fn new(entry: impl Into<String>, function: &str, config: Value) -> Self {
        Self { entry: entry.into(), function: function.to_string(), config, server: None, timeout_secs: DEFAULT_PLUGIN_TIMEOUT_SECS, host: None }
    }

    pub(crate) fn in_instance(mut self, host: Option<host::Context>) -> Self {
        self.host = host;
        self
    }

    pub fn serving(mut self, server: Option<u32>) -> Self {
        self.server = server;
        self
    }

    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    #[cfg(test)]
    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Value {
        &mut self.config
    }

    #[cfg(test)]
    pub fn server(&self) -> Option<u32> {
        self.server
    }

    pub async fn run(self) -> Result<Value> {
        HOST_CONTEXT.scope(self.host, run_plugin_inner(&self.entry, &self.function, self.config, None, self.timeout_secs, self.server)).await
    }

    pub async fn run_with_progress(self, callback: Box<dyn Fn(ExtensionProgress) + Send>) -> Result<Value> {
        HOST_CONTEXT.scope(self.host, run_plugin_inner(&self.entry, &self.function, self.config, Some(callback), self.timeout_secs, self.server)).await
    }

    pub async fn try_run(self) -> Result<Option<Value>> {
        HOST_CONTEXT.scope(self.host, in_call(&self.entry, &self.function, self.timeout_secs, true,
            run_requested_plugin(&self.entry, &self.function, self.config, None, self.timeout_secs, self.server))).await
    }
}

/// Like `run_plugin` but with a custom timeout (seconds).
/// 서버 설치·업데이트(`install_server`) 한도: 대용량 다운로드를 허용한다
pub const INSTALL_TIMEOUT_SECS: u64 = 1800;

pub async fn run_plugin_with_timeout(module_path: &str, function: &str, config: Value, timeout_secs: u64) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, timeout_secs, None).await
}

type Writer = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;
type Reader = Box<dyn tokio::io::AsyncRead + Unpin + Send>;

/// 띄운 Python: 평소대로 띄운 것과 AppContainer 안에서 띄운 것 (실행마다 하나라 크기는 상관없다)
#[allow(clippy::large_enum_variant)]
enum Process {
    /// 평소대로 띄운 것. Windows는 Job Object에 넣어 둔다: 부모가 먼저 끝나 `taskkill /T`가 못 찾는 손자도 끝낼 수 있게
    Plain {
        child: tokio::process::Child,
        #[cfg(windows)]
        job: Option<std::os::windows::io::OwnedHandle>,
    },
    #[cfg(windows)]
    Contained(crate::sandbox::appcontainer::Contained),
}

impl Process {
    fn kill_now(&mut self, pid: Option<u32>) {
        match self {
            Process::Plain {
                child,
                #[cfg(windows)]
                job,
            } => {
                #[cfg(windows)]
                if let Some(job) = job { windows_job::terminate(job); }
                if let Some(pid) = pid.or(child.id()) { kill_tree(pid); }
                let _ = child.start_kill();
            }
            #[cfg(windows)]
            Process::Contained(child) => child.kill(),
        }
    }

    fn pipes(&mut self) -> (Option<Writer>, Option<Reader>, Option<Reader>) {
        match self {
            Process::Plain { child, .. } => (
                child.stdin.take().map(|p| Box::new(p) as Writer),
                child.stdout.take().map(|p| Box::new(p) as Reader),
                child.stderr.take().map(|p| Box::new(p) as Reader),
            ),
            #[cfg(windows)]
            Process::Contained(c) => (
                c.stdin.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Writer),
                c.stdout.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Reader),
                c.stderr.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Reader),
            ),
        }
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self {
            Process::Plain { child, .. } => child.wait().await,
            #[cfg(windows)]
            Process::Contained(c) => {
                use std::os::windows::process::ExitStatusExt;
                let handle = c.wait_handle();
                let code = tokio::task::spawn_blocking(move || crate::sandbox::appcontainer::wait(handle))
                    .await
                    .map_err(std::io::Error::other)??;
                Ok(std::process::ExitStatus::from_raw(code))
            }
        }
    }

    /// 그것이 띄운 프로세스까지 끝낸다. `pid`는 띄울 때 받아 둔 것 (이미 거둔 뒤에는 `child.id()`가 없다)
    async fn kill(&mut self, pid: Option<u32>) {
        match self {
            Process::Plain {
                child,
                #[cfg(windows)]
                job,
            } => {
                #[cfg(windows)]
                if let Some(job) = job {
                    windows_job::terminate(job);
                }
                if let Some(pid) = pid.or(child.id()) {
                    kill_tree(pid);
                }
                let _ = child.kill().await;
            }
            #[cfg(windows)]
            Process::Contained(c) => {
                c.kill();
                // 핸들을 닫기 전에 끝난 것을 확인한다
                let handle = c.wait_handle();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::task::spawn_blocking(move || crate::sandbox::appcontainer::wait(handle)),
                )
                .await;
            }
        }
    }
}

fn plain(cmd: &mut Command) -> std::io::Result<Process> {
    #[cfg(windows)]
    let (child, job) = windows_job::spawn(cmd)?;
    #[cfg(not(windows))]
    let child = cmd.spawn()?;
    Ok(Process::Plain {
        #[cfg(windows)]
        job: Some(job),
        child,
    })
}

/// 환경 준비가 취소되어도 다운로드·venv 생성 프로세스와 그 자식은 남기지 않는다
pub(crate) async fn preparation_output(cmd: &mut Command) -> std::io::Result<std::process::Output> {
    cmd.kill_on_drop(true).stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    let (child, job) = windows_job::spawn(cmd)?;
    #[cfg(not(windows))]
    let child = cmd.spawn()?;
    struct Cleanup {
        pid: Option<u32>,
        #[cfg(windows)]
        job: Option<std::os::windows::io::OwnedHandle>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(pid) = self.pid {
                #[cfg(windows)]
                if let Some(job) = &self.job { windows_job::terminate(job); }
                kill_tree(pid);
            }
        }
    }
    let mut cleanup = Cleanup {
        pid: child.id(),
        #[cfg(windows)]
        job: Some(job),
    };
    let output = child.wait_with_output().await;
    if output.is_ok() { cleanup.pid = None; }
    output
}

/// 평소대로 띄운 Python을 넣는 Job Object (Windows).
/// 끝낼 때만 쓴다: 닫아도 안의 프로세스는 남는다(모듈 `start`가 띄우고 PID만 돌려주는 서버가 있다).
/// Job 탈출을 요청하는 프로세스는 막지 않는다 (예전처럼 띄울 수 있게)
#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME};

    // 실행기가 자식을 먼저 띄우면 Job 등록만으로는 이미 나온 자식을 포함하지 못한다.
    pub(super) fn spawn(cmd: &mut tokio::process::Command) -> std::io::Result<(tokio::process::Child, OwnedHandle)> {
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = cmd.spawn()?;
        let ready = contain(&child).and_then(|job| {
            let pid = child.id().ok_or_else(|| std::io::Error::other("Plugin process has no PID"))?;
            resume(pid)?;
            Ok(job)
        });
        match ready {
            Ok(job) => Ok((child, job)),
            Err(error) => {
                let _ = child.start_kill();
                Err(error)
            }
        }
    }

    fn contain(child: &tokio::process::Child) -> std::io::Result<OwnedHandle> {
        let process = child.raw_handle().ok_or_else(|| std::io::Error::other("Plugin process has no handle"))? as HANDLE;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = OwnedHandle::from_raw_handle(job as RawHandle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            let ok = SetInformationJobObject(
                job.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                &mut limits as *mut _ as *mut _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
                && AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process) != 0;
            if !ok {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }
    }

    fn resume(pid: u32) -> std::io::Result<()> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let snapshot = OwnedHandle::from_raw_handle(snapshot as RawHandle);
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut found = Thread32First(snapshot.as_raw_handle() as HANDLE, &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if thread.is_null() {
                        return Err(std::io::Error::last_os_error());
                    }
                    let thread = OwnedHandle::from_raw_handle(thread as RawHandle);
                    if ResumeThread(thread.as_raw_handle() as HANDLE) == u32::MAX {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(());
                }
                found = Thread32Next(snapshot.as_raw_handle() as HANDLE, &mut entry);
            }
            Err(std::io::Error::other("Plugin process has no initial thread"))
        }
    }

    pub(super) fn terminate(job: &OwnedHandle) {
        unsafe { TerminateJobObject(job.as_raw_handle() as HANDLE, 1) };
    }
}

/// 이 실행이 import할 수 있는 익스텐션과 확인되지 않아 뺀 익스텐션. 격리 없이 도는 실행(확인된 모듈 등)이
/// 확인되지 않은 익스텐션을 import하면 그 코드가 격리·검사 없이 그 실행의 권한(데몬 토큰 경로 등)으로 돈다.
/// `consented`는 사용자가 격리 없이 돌리기로 한 구성요소의 키(`ext-<id>`)다: 그 익스텐션은 이미 이 실행의 코드이므로 자기 패키지를 import할 수 있다
fn importable_extensions(enabled: Vec<String>, isolated: bool, unverified: impl Fn(&str) -> bool, consented: Option<&str>) -> (Vec<String>, Vec<String>) {
    if isolated {
        return (enabled, Vec::new());
    }
    let own = consented.and_then(|key| key.strip_prefix("ext-")).map(|id| id.replace('_', "-"));
    enabled.into_iter().partition(|id| !unverified(id) || own.as_deref() == Some(id.replace('_', "-").as_str()))
}

/// 이 실행이 import할 수 있는 익스텐션을 Python 가드(`sitecustomize.py`)에 알린다. 확인되지 않아 뺀 것을 돌려준다
fn set_extension_env(cmd: &mut tokio::process::Command, enabled: Vec<String>, isolated: bool, consented: Option<&str>) -> Vec<String> {
    let (importable, unverified) = importable_extensions(enabled, isolated, |id| {
        crate::sandbox::python_isolation_required(crate::sandbox::extension_trust(id))
            && !crate::sandbox::extension_unisolated_allowed(id)
    }, consented);
    cmd.env("SABA_ENABLED_EXTENSIONS", importable.join(","));
    // 가드가 "켜져 있지 않음" 대신 "확인되지 않아 쓸 수 없음"이라고 알리게
    cmd.env("SABA_UNVERIFIED_EXTENSIONS", unverified.join(","));
    unverified
}

/// 빠진 익스텐션이 모두 검증되지 않아 차단되었으면 설치 안내 대신 검증 실패를 알린다.
fn explain_unverified_extensions(result: &mut Value, unverified: &[String]) {
    if result.get("error_code").and_then(Value::as_str) != Some("extension_required") || unverified.is_empty() {
        return;
    }
    let same = |a: &str, b: &str| a.replace('_', "-").eq_ignore_ascii_case(&b.replace('_', "-"));
    let missing: Vec<String> = result.get("missing_extensions").and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str).map(str::to_string).collect();
    let blocked: Vec<&String> = unverified.iter().filter(|u| missing.iter().any(|m| same(m, u))).collect();
    if missing.is_empty() || blocked.len() < missing.len() {
        return;
    }
    result["error_code"] = serde_json::json!("extension_not_verified");
    result["unverified_extensions"] = serde_json::json!(blocked);
    result["message"] = serde_json::json!(format!(
        "Extension(s) {} changed and could not be verified. Reinstall them or restore the original files.",
        blocked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    ));
}

/// 모듈이 구현하지 않아도 되는 lifecycle 함수 (MK1). 모듈이 `functions`로 밝힌 목록에 없으면 데몬은 Python을 띄우지 않고
/// `not_supported`로 답하고, 클라이언트는 `GET /api/modules`의 `functions`를 보고 그 기능을 숨긴다
pub const OPTIONAL_MODULE_FUNCTIONS: &[&str] = &[
    "reset_server", "list_versions", "get_version_details", "get_installed_version", "diagnose_log",
    "read_properties", "reset_properties", "configure", "import_settings", "install_server", "validate", "config_files",
];

/// 예약 함수: 모듈이 구현한 lifecycle 함수 이름(자신 제외)을 `{"success": true, "functions": [...]}`로 답한다
pub const FUNCTIONS_QUERY: &str = "functions";
const FUNCTIONS_TIMEOUT_SECS: u64 = 15;

/// 목록을 묻다 실패한(시간 초과·격리 차단 등) 결과는 이만큼만 기억한다: 일시적인 실패가 기능을 계속 가리지 않게
const FUNCTIONS_RETRY_SECS: u64 = 60;
/// `GET /api/modules`가 캐시에 없는 목록을 기다리는 한도. 넘기면 null로 답하고 묻기는 뒤에서 마저 한다
pub const FUNCTIONS_LIST_WAIT_MS: u64 = 2000;

/// `functions`에 대한 모듈의 답
#[derive(Debug, Clone, PartialEq)]
enum FunctionsAnswer {
    Listed(Vec<String>),
    /// `functions`를 모르는 옛 모듈("Unknown function"): 진입 파일이 바뀔 때까지 다시 묻지 않는다
    Unknown,
    /// 실패·모양이 다른 답: 잠시 뒤 다시 묻는다
    Failed,
}

impl FunctionsAnswer {
    fn list(&self) -> Option<Vec<String>> {
        match self {
            Self::Listed(list) => Some(list.clone()),
            _ => None,
        }
    }
}

/// 진입 파일 경로 → (그때의 수정 시각, 답, 답한 때). 모듈을 업데이트해 진입 파일이 바뀌면 다시 묻는다
type FunctionCache = std::collections::HashMap<String, (Option<std::time::SystemTime>, FunctionsAnswer, std::time::Instant)>;
static MODULE_FUNCTIONS: std::sync::LazyLock<std::sync::Mutex<FunctionCache>> = std::sync::LazyLock::new(Default::default);
/// 진입 파일마다 하나: 동시에 온 요청이 Python을 따로 띄우지 않고 먼저 물은 답을 기다린다
struct FunctionQuery {
    asking: tokio::sync::Mutex<()>,
    isolation: tokio::sync::watch::Sender<IsolationTime>,
}

impl Default for FunctionQuery {
    fn default() -> Self {
        Self { asking: tokio::sync::Mutex::new(()), isolation: tokio::sync::watch::channel(IsolationTime::default()).0 }
    }
}

type FunctionLocks = std::collections::HashMap<String, std::sync::Arc<FunctionQuery>>;
static FUNCTIONS_ASKING: std::sync::LazyLock<std::sync::Mutex<FunctionLocks>> = std::sync::LazyLock::new(Default::default);

/// `functions` 응답을 읽는다. 목록은 성공 응답의 문자열 배열에서만 읽는다
fn functions_answer(result: Result<Value>) -> FunctionsAnswer {
    let Ok(value) = result else { return FunctionsAnswer::Failed };
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        let unknown = value.get("message").and_then(Value::as_str).is_some_and(|m| m.starts_with("Unknown function"));
        return if unknown { FunctionsAnswer::Unknown } else { FunctionsAnswer::Failed };
    }
    let list: Option<Vec<String>> = value.get("functions").and_then(Value::as_array)
        .and_then(|items| items.iter().map(|f| f.as_str().map(str::to_string)).collect());
    list.map_or(FunctionsAnswer::Failed, FunctionsAnswer::Listed)
}

/// 캐시에 남은 답이 아직 쓸 만한가: 진입 파일이 그대로이고, 실패면 다시 물을 때가 지나지 않았다
fn still_valid(cached: &(Option<std::time::SystemTime>, FunctionsAnswer, std::time::Instant), modified: Option<std::time::SystemTime>) -> bool {
    cached.0 == modified
        && (cached.1 != FunctionsAnswer::Failed || cached.2.elapsed() < std::time::Duration::from_secs(FUNCTIONS_RETRY_SECS))
}

/// 캐시에 있는 목록 (Some(None)은 "물었지만 모름")
fn cached_functions(entry: &str) -> Option<Option<Vec<String>>> {
    let modified = std::fs::metadata(entry).and_then(|m| m.modified()).ok();
    let cache = MODULE_FUNCTIONS.lock().unwrap_or_else(|e| e.into_inner());
    cache.get(entry).filter(|c| still_valid(c, modified)).map(|c| c.1.list())
}

/// 모듈이 구현한 lifecycle 함수 목록. 진입 파일마다 `functions`를 한 번 불러 두고(동시 요청은 그 답을 함께 기다린다), 모르면 None
pub async fn module_functions(entry: &str) -> Option<Vec<String>> {
    if let Some(list) = cached_functions(entry) {
        return list;
    }
    in_call(entry, FUNCTIONS_QUERY, FUNCTIONS_TIMEOUT_SECS, false, query_functions(entry)).await.ok().flatten().flatten()
}

async fn query_functions(entry: &str) -> Result<Option<Vec<String>>> {
    if let Some(list) = cached_functions(entry) {
        return Ok(list);
    }
    let lock = FUNCTIONS_ASKING.lock().unwrap_or_else(|e| e.into_inner()).entry(entry.to_string()).or_default().clone();
    let outer = CALL_DEADLINE.with(Clone::clone);
    let deadline = CallDeadline {
        at: tokio::time::Instant::now() + outer.remaining().min(std::time::Duration::from_secs(FUNCTIONS_TIMEOUT_SECS)) - lock.isolation.borrow().elapsed(),
        isolation: lock.isolation.clone(),
    };
    outer.follow_isolation(&deadline, CALL_DEADLINE.scope(deadline.clone(), async {
        let _asking = deadline.timeout(lock.asking.lock()).await
            .map_err(|_| call_error(entry, FUNCTIONS_QUERY, "module_timeout", "run", "Function list query did not finish before the deadline", Vec::new()))?;
        if let Some(list) = cached_functions(entry) {
            return Ok(list);
        }
        let modified = std::fs::metadata(entry).and_then(|m| m.modified()).ok();
        let answer = functions_answer(Box::pin(spawn_plugin(entry, FUNCTIONS_QUERY, serde_json::json!({}), None, FUNCTIONS_TIMEOUT_SECS, None)).await);
        let list = answer.list();
        MODULE_FUNCTIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(entry.to_string(), (modified, answer, std::time::Instant::now()));
        Ok(list)
    })).await
}

/// 목록 응답용: 캐시에 없으면 뒤에서 묻기 시작하고 `FUNCTIONS_LIST_WAIT_MS`까지만 기다린다
pub async fn module_functions_for_list(entry: String) -> Option<Vec<String>> {
    if let Some(list) = cached_functions(&entry) {
        return list;
    }
    let asking = tokio::spawn(async move { module_functions(&entry).await });
    tokio::time::timeout(std::time::Duration::from_millis(FUNCTIONS_LIST_WAIT_MS), asking).await.ok()?.ok()?
}

/// 데몬 시작·모듈 새로고침 때 목록을 미리 물어 둔다 (목록 응답이 기다리지 않게)
pub fn prefetch_module_functions(entries: Vec<String>) {
    for entry in entries {
        tokio::spawn(async move { module_functions(&entry).await });
    }
}

/// 모듈이 밝힌 목록에 없는 선택 함수에 대한 답. 목록을 모르거나 필수 함수면 None(그대로 부른다)
fn unsupported_reply(function: &str, known: Option<&[String]>) -> Option<Value> {
    let known = known?;
    if !OPTIONAL_MODULE_FUNCTIONS.contains(&function) || known.iter().any(|f| f == function) {
        return None;
    }
    Some(serde_json::json!({
        "success": false,
        "error_code": "not_supported",
        "function": function,
        "message": format!("This module does not implement '{}'", function),
    }))
}

async fn run_plugin_inner(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
    server: Option<u32>,
) -> Result<Value> {
    in_call(module_path, function, timeout_secs, false,
        run_requested_plugin(module_path, function, config, on_progress, timeout_secs, server)).await.map(Option::unwrap)
}

async fn run_requested_plugin(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
    server: Option<u32>,
) -> Result<Value> {
    // 모듈(옆에 module.toml이 있는 진입 파일)만: 익스텐션 hook 이름은 이 표와 상관없다
    let is_module = std::path::Path::new(module_path).parent().is_some_and(|d| d.join("module.toml").is_file());
    if is_module && OPTIONAL_MODULE_FUNCTIONS.contains(&function) {
        if let Some(reply) = unsupported_reply(function, query_functions(module_path).await.unwrap_or(None).as_deref()) {
            tracing::debug!("Module {} does not implement {}", module_path, function);
            return Ok(decorate_result(module_path, function, reply, ""));
        }
    }
    Box::pin(spawn_plugin(module_path, function, config, on_progress, timeout_secs, server)).await
}

async fn spawn_plugin(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
    server: Option<u32>,
) -> Result<Value> {
    tracing::debug!("Executing plugin: {} -> {}", module_path, function);

    let deadline = CALL_DEADLINE.with(Clone::clone);
    if !std::path::Path::new(module_path).is_file() {
        return Err(call_error(module_path, function, "module_execution_failed", "prepare", "Module entry file does not exist", Vec::new()));
    }
    let config_json = serde_json::to_string(&config).map_err(|error| call_error(module_path, function, "module_execution_failed", "prepare", error, Vec::new()))?;
    let python_exe = prepare(module_path, function, deadline.clone(), crate::python_env::get_python_path()).await?;
    tracing::debug!("Using Python: {}", python_exe.display());

    let mut cmd = Command::new(&python_exe);
    // 실행 시 제한 (M12): 비밀값처럼 보이는 변수는 물려주지 않고, 작업 폴더는 모듈·익스텐션 폴더로 고정
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(is_secret_env) {
            cmd.env_remove(&name);
        }
    }
    if let Some(dir) = std::path::Path::new(module_path).parent().filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    // 바이트코드 캐시는 데이터 폴더에만: 모듈 폴더의 __pycache__에 넣어 둔 .pyc가 소스 대신 실행되지 않게
    cmd.env("PYTHONPYCACHEPREFIX", saba_chan_updater_lib::constants::resolve_data_dir().join("pycache"));
    cmd.arg(module_path)
        .arg(function)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .envs(daemon_env());

    // 확인되지 않은 구성요소: AppContainer 안에서 돌린다 (Windows). 토큰 파일 경로는 주지 않는다
    let isolation = crate::sandbox::isolation_for(std::path::Path::new(module_path), &config, &python_exe);

    // PYTHONPATH 경로 리스트 구분자 (Windows: ';', Unix: ':')
    #[cfg(windows)]
    const PYPATH_SEP: char = ';';
    #[cfg(not(windows))]
    const PYPATH_SEP: char = ':';
    let mut paths: Vec<String> = Vec::new();
    // 활성화된 익스텐션 목록을 Python에 전달 (비활성 익스텐션 import 방지용)
    let mut enabled_extensions: Option<Vec<String>> = None;
    let mut unverified_extensions = Vec::new();
    let extensions_dir = resolve_extensions_dir();
    if extensions_dir.is_dir() {
        let enabled = saba_chan_updater_lib::config_file::ConfigFile::default_location()
            .section("extensions")
            .get("enabled")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok());
        // 목록이 없으면 활성 익스텐션도 없다 (ExtensionManager와 같은 규칙)
        enabled_extensions = Some(enabled.unwrap_or_default());
        unverified_extensions = set_extension_env(&mut cmd, enabled_extensions.clone().unwrap_or_default(), isolation.is_some(), None);

        let mut pypath = extensions_dir.parent()
            .unwrap_or(&extensions_dir)
            .to_string_lossy()
            .into_owned();
        // 활성 익스텐션만 import되게 하는 가드 (M6, sitecustomize.py)
        if let Some(guard) = import_guard_dir() {
            pypath = format!("{}{}{}", guard.display(), PYPATH_SEP, pypath);
        }
        paths.push(pypath);
    }
    // 공용 모듈 유틸리티 경로 (daemon_rcon.py, i18n.py 등). 익스텐션 폴더가 없어도 필요하다
    static SHARED: std::sync::Once = std::sync::Once::new();
    let modules_lib_dir = saba_chan_updater_lib::constants::resolve_data_dir().join("python-shared");
    SHARED.call_once(|| {
        if let Err(e) = ensure_shared_modules(&modules_lib_dir) {
            tracing::warn!("Cannot write shared Python modules to {}: {}", modules_lib_dir.display(), e);
        }
    });
    if modules_lib_dir.is_dir() {
        paths.push(modules_lib_dir.display().to_string());
    }
    if !paths.is_empty() {
        if let Ok(existing) = std::env::var("PYTHONPATH") {
            paths.push(existing);
        }
        cmd.env("PYTHONPATH", paths.join(&PYPATH_SEP.to_string()));
    }

    // 데몬이 찾은 Java 런타임 (`[{java, home, version, major}]`, JAVA_HOME·PATH·설치 위치 순). 모듈은 필요한 주 버전을 여기서 고른다
    if let Ok(runtimes) = serde_json::to_string(&crate::sandbox::java_runtimes()) {
        cmd.env("SABA_JAVA_RUNTIMES", runtimes);
    }

    // 격리 실행은 127.0.0.1에 닿지 못한다: 자기 인스턴스 포트로 가는 요청은 데몬이 대신 보낸다 (loopback.rs)
    let mut loopback = isolation.as_ref().and_then(|iso| {
        let dir = iso.temp.join("loopback");
        std::fs::create_dir_all(&dir).map_err(|e| tracing::warn!("[Loopback] cannot create {}: {}", dir.display(), e)).ok()?;
        cmd.env("SABA_LOOPBACK_DIR", &dir);
        Some(loopback::Proxy { dir, ports: loopback::instance_ports(&config), server })
    });
    if let Some(iso) = &isolation {
        cmd.env("PYTHONPYCACHEPREFIX", &iso.pycache)
            .env("TEMP", &iso.temp)
            .env("TMP", &iso.temp)
            .env_remove("SABA_TOKEN_PATH");
        if let Some(data) = &iso.data {
            cmd.env("SABA_EXTENSION_DATA_DIR", data);
        }
    }
    // import된 익스텐션(SteamCMD 등)이 받은 파일을 둘 곳: `<뿌리>/<익스텐션 id>`
    let extension_root = match &isolation {
        Some(iso) => iso.extension_root.clone(),
        None => crate::sandbox::extension_data_root(&saba_chan_updater_lib::constants::resolve_data_dir(), None),
    };
    cmd.env("SABA_EXTENSION_DATA_ROOT", extension_root);

    let host_root = isolation.as_ref().map(|iso| iso.temp.clone())
        .unwrap_or_else(|| saba_chan_updater_lib::constants::resolve_data_dir().join("host-temp"));
    std::fs::create_dir_all(&host_root).map_err(|error| call_error(module_path, function, "module_execution_failed", "prepare", error, Vec::new()))?;
    let host_dir = tempfile::Builder::new().prefix("host-").tempdir_in(&host_root)
        .map_err(|error| call_error(module_path, function, "module_execution_failed", "prepare", error, Vec::new()))?;
    cmd.env("SABA_HOST_DIR", host_dir.path());
    // 격리된 구성요소는 HOST에서 자기 인스턴스의 REST만 쓴다 (host.rs `sandboxed`)
    let sandboxed = isolation.as_ref()
        .filter(|iso| !crate::sandbox::unisolated_allowed(&iso.key, &iso.dir))
        .map(|_| std::path::PathBuf::from(module_path));
    let host_context = HOST_CONTEXT.try_with(Clone::clone).ok().flatten();
    let host_proxy = host::Proxy { dir: host_dir.path().into(), context: host_context, server, sandboxed };

    apply_creation_flags(&mut cmd);

    if deadline.remaining().is_zero() {
        return Err(call_error(module_path, function, "module_timeout", "prepare", "Process preparation exceeded the deadline", Vec::new()));
    }
    let mut child = match &isolation {
        None => plain(&mut cmd).map_err(|error| call_error(module_path, function, "module_execution_failed", "run", error, Vec::new()))?,
        Some(iso) => match deadline.paused(crate::sandbox::spawn_isolated(cmd.as_std(), iso)).await {
            Ok(crate::sandbox::Spawned::Unisolated) => {
                // 사용자가 격리 없이 돌리기로 한 것은 이 구성요소뿐이다: 확인되지 않은 다른 익스텐션은 import하지 못하게
                if let Some(enabled) = enabled_extensions {
                    unverified_extensions = set_extension_env(&mut cmd, enabled, false, Some(&iso.key));
                }
                // 격리 없이 돌면 루프백에 직접 닿는다
                cmd.env_remove("SABA_LOOPBACK_DIR");
                loopback = None;
                plain(&mut cmd).map_err(|error| call_error(module_path, function, "module_execution_failed", "run", error, Vec::new()))?
            }
            #[cfg(windows)]
            Ok(crate::sandbox::Spawned::Contained(contained)) => Process::Contained(contained),
            Err(blocked) => {
                return Ok(sandbox_failure(module_path, function, &iso.key, &blocked));
            }
        },
    };
    let status = drive(&mut child, config_json.as_bytes(), on_progress, loopback, Some(host_proxy), deadline.remaining()).await;

    match status {
        Ok(Ok((exit_status, stdout_str, stderr_str))) => {
            if !exit_status.success() {
                tracing::error!("Plugin failed (exit {:?}):\n  stderr: {}\n  stdout: {}", exit_status.code(), stderr_str, stdout_str.trim());
                return Err(call_error(module_path, function, "module_crashed", "run", format!("{}.{} exited with {:?}", module_name(module_path), function, exit_status.code()), stderr_tail(&stderr_str)));
            }

            match parse_plugin_output(&stdout_str) {
                Ok(mut result) => {
                    tracing::debug!("Plugin result (raw): {}", stdout_str.trim());
                    explain_unverified_extensions(&mut result, &unverified_extensions);
                    Ok(decorate_result(module_path, function, result, &stderr_str))
                }
                Err(e) => {
                    tracing::error!("Failed to parse plugin JSON: {} | stdout: {}", e, stdout_str);
                    Err(call_error(module_path, function, "module_bad_output", "run", format!("Invalid JSON from {}.{}: {}", module_name(module_path), function, e), stderr_tail(&stderr_str)))
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Plugin process error: {}", e);
            let code = if e.error.get_ref().is_some_and(|error| error.is::<OutputTooLarge>()) { "module_output_too_large" } else { "module_execution_failed" };
            Err(call_error(module_path, function, code, "run", &e.error, e.stderr_tail))
        }
        Err(error) => {
            // 타임아웃 — drive가 프로세스 트리를 이미 끝냈다
            tracing::warn!(
                "Plugin timed out after {}s: {} -> {} — killed process",
                timeout_secs, module_path, function
            );
            Err(call_error(module_path, function, "module_timeout", "run", format!("{}.{} did not finish in {} s", module_name(module_path), function, timeout_secs), error.stderr_tail))
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Plugin stdout exceeds 4 MiB")]
struct OutputTooLarge;

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
struct RunFailure {
    error: std::io::Error,
    stderr_tail: Vec<String>,
}

#[derive(Debug)]
struct RunTimeout {
    stderr_tail: Vec<String>,
}

type Collected = std::result::Result<(std::process::ExitStatus, String, String), RunFailure>;

#[derive(Default)]
struct StderrLog {
    lines: std::collections::VecDeque<String>,
    bytes: usize,
}

impl StderrLog {
    fn push(&mut self, mut line: String) {
        if line.len() >= STDERR_BYTES {
            let mut start = line.len() - (STDERR_BYTES - 1);
            while !line.is_char_boundary(start) { start += 1; }
            line.drain(..start);
        }
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.lines.len() > STDERR_LINES || self.bytes > STDERR_BYTES {
            if let Some(line) = self.lines.pop_front() {
                self.bytes -= line.len() + 1;
            }
        }
    }

    fn tail(&self) -> Vec<String> {
        self.lines.iter().skip(self.lines.len().saturating_sub(20)).cloned().collect()
    }

    fn text(&self) -> String {
        self.lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n")
    }
}

async fn next_stderr_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R, lines: &mut saba_runner::Lines) -> std::io::Result<Option<(String, bool)>> {
    use tokio::io::AsyncBufReadExt;
    loop {
        if let Some(line) = lines.next_line() {
            let ended = lines.ends_line(&line);
            return Ok(Some((lines.decode(&line), ended)));
        }
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(lines.finish().map(|line| (lines.decode(&line), true)));
        }
        let count = chunk.len();
        lines.push(chunk);
        reader.consume(count);
    }
}

/// 입력 쓰기부터 종료 대기, stdout·stderr 수집까지 한 기한 안에 한다.
/// 기한을 넘기거나 입출력이 실패하면 프로세스 트리를 끝내고 읽기 작업을 멈춘다.
/// (손자 프로세스가 파이프를 물려받아 쥐고 있으면 부모가 끝나도 출력이 닫히지 않는다)
async fn drive(
    child: &mut Process,
    input: &[u8],
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    loopback: Option<loopback::Proxy>,
    host: Option<host::Proxy>,
    timeout: std::time::Duration,
) -> std::result::Result<Collected, RunTimeout> {
    let pid = match child {
        Process::Plain { child, .. } => child.id(),
        #[cfg(windows)]
        Process::Contained(_) => None,
    };
    let (stdin_pipe, stdout_pipe, stderr_pipe) = child.pipes();

    let stderr = std::sync::Arc::new(std::sync::Mutex::new(StderrLog::default()));
    let captured = stderr.clone();
    let deadline = CALL_DEADLINE.try_with(Clone::clone).ok();
    let run_deadline = deadline.clone().unwrap_or_else(|| CallDeadline::new(tokio::time::Instant::now() + timeout));
    let stderr_handle = tokio::spawn(async move {
        if let Some(pipe) = stderr_pipe {
            let mut reader = BufReader::new(pipe);
            let mut lines = saba_runner::Lines::default();
            let mut relayed = 0;
            let mut requests = tokio::task::JoinSet::new();
            let mut hide_continuation = false;
            // UTF-8이 아닌 줄에서 멈추면 stderr 파이프가 차서 모듈이 시간 초과까지 멈춘다
            while let Some((line, ended)) = next_stderr_line(&mut reader, &mut lines).await? {
                if hide_continuation {
                    hide_continuation = !ended;
                    continue;
                }
                if let Some(request) = line.strip_prefix(host::PREFIX) {
                    hide_continuation = !ended;
                    match &host {
                        Some(proxy) if relayed < loopback::MAX_REQUESTS => {
                            relayed += 1;
                            let (proxy, request) = (proxy.clone(), request.to_string());
                            let deadline = deadline.clone();
                            requests.spawn(async move {
                                let serve = host::serve(&proxy, &request);
                                if let Some(deadline) = deadline { CALL_DEADLINE.scope(deadline, serve).await; }
                                else { serve.await; }
                            });
                        }
                        Some(proxy) => host::refuse(proxy, request, "too_many_requests"),
                        None => tracing::warn!("[Host] request without a channel ignored"),
                    }
                    continue;
                }
                // 중계 요청은 로그에 남기지 않는다 (인증 머리글이 들어 있을 수 있다)
                if let Some(request) = line.strip_prefix(loopback::PREFIX) {
                    hide_continuation = !ended;
                    match &loopback {
                        Some(proxy) if relayed < loopback::MAX_REQUESTS => {
                            relayed += 1;
                            let (proxy, request) = (proxy.clone(), request.to_string());
                            let deadline = deadline.clone();
                            requests.spawn(async move {
                                let serve = loopback::serve(&proxy, &request);
                                if let Some(deadline) = deadline {
                                    CALL_DEADLINE.scope(deadline, serve).await;
                                } else {
                                    serve.await;
                                }
                            });
                        }
                        Some(proxy) => {
                            tracing::warn!("[Loopback] request limit of {} reached", loopback::MAX_REQUESTS);
                            loopback::refuse(proxy, request, "too_many_requests");
                        }
                        None => tracing::warn!("[Loopback] request from an unisolated run ignored"),
                    }
                    continue;
                }
                if let Some(json_str) = line.strip_prefix("PROGRESS:") {
                    if let Ok(prog) = serde_json::from_str::<ExtensionProgress>(json_str) {
                        if let Some(ref cb) = on_progress {
                            cb(prog);
                        }
                    }
                    continue;
                } else {
                    tracing::debug!("Plugin stderr: {}", line);
                }
                captured.lock().unwrap_or_else(|e| e.into_inner()).push(line);
            }
        }
        Ok::<_, std::io::Error>(())
    });

    let stdout_handle = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let mut reader = tokio::io::AsyncReadExt::take(pipe, STDOUT_LIMIT as u64 + 1);
            tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await?;
            if buf.len() > STDOUT_LIMIT {
                return Err(std::io::Error::other(OutputTooLarge));
            }
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    });

    let readers = [stderr_handle.abort_handle(), stdout_handle.abort_handle()];

    struct Running<'a> {
        child: &'a mut Process,
        pid: Option<u32>,
        readers: [tokio::task::AbortHandle; 2],
        finished: bool,
    }
    impl Drop for Running<'_> {
        fn drop(&mut self) {
            if !self.finished {
                self.child.kill_now(self.pid);
                for reader in &self.readers { reader.abort(); }
            }
        }
    }
    let mut running = Running { child, pid, readers, finished: false };

    let work = async {
        // 출력 상한은 프로세스 종료를 기다리지 않고 바로 끊는다
        let (exit_status, stdout_str, ()) = tokio::try_join!(
            async {
                if let Some(mut stdin) = stdin_pipe {
                    stdin.write_all(input).await?;
                    stdin.shutdown().await?;
                }
                running.child.wait().await
            },
            async { stdout_handle.await.map_err(std::io::Error::other)? },
            async { stderr_handle.await.map_err(std::io::Error::other)? },
        )?;
        let stderr_str = stderr.lock().unwrap_or_else(|e| e.into_inner()).text();
        Ok((exit_status, stdout_str, stderr_str))
    };
    let result = run_deadline.timeout(work).await;
    if !matches!(result, Ok(Ok(_))) {
        running.child.kill(pid).await;
        for reader in &running.readers {
            reader.abort();
        }
    }
    running.finished = true;
    let stderr_tail = stderr.lock().unwrap_or_else(|e| e.into_inner()).tail();
    match result {
        Ok(Ok(collected)) => Ok(Ok(collected)),
        Ok(Err(error)) => Ok(Err(RunFailure { error, stderr_tail })),
        Err(_) => Err(RunTimeout { stderr_tail }),
    }
}

// detect_python_command()는 python_env::get_python_path()로 대체됨.
// 시스템 Python 탐지는 python_env 모듈에서 venv 부트스트랩 시 수행합니다.

/// 결과는 stdout 전체 또는 마지막 줄의 JSON (앞에 찍힌 print는 무시)
fn parse_plugin_output(stdout: &str) -> serde_json::Result<Value> {
    serde_json::from_str::<Value>(stdout).or_else(|e| {
        stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l.trim()))
            .unwrap_or(Err(e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_call_futures_fit_worker_stacks() {
        fn size<F: std::future::Future>(_: impl FnOnce() -> F) -> usize {
            std::mem::size_of::<F>()
        }

        let sizes = [
            ("run_plugin", size(|| run_plugin("game/lifecycle.py", "status", Value::Null))),
            ("PluginCall::run", size(|| PluginCall::new("game/lifecycle.py", "status", Value::Null).run())),
            ("PluginCall::run_with_progress", size(|| PluginCall::new("game/lifecycle.py", "status", Value::Null).run_with_progress(Box::new(|_| {})))),
            ("PluginCall::try_run", size(|| PluginCall::new("game/lifecycle.py", "status", Value::Null).try_run())),
            ("module_functions", size(|| module_functions("game/lifecycle.py"))),
        ];
        eprintln!("module-call future sizes: {sizes:?}; spawn_plugin={}, query_functions={}, run_requested_plugin={}",
            size(|| spawn_plugin("game/lifecycle.py", "status", Value::Null, None, 120, None)),
            size(|| query_functions("game/lifecycle.py")),
            size(|| run_requested_plugin("game/lifecycle.py", "status", Value::Null, None, 120, None)));
        for (entry, bytes) in sizes {
            assert!(bytes <= 16 * 1024, "{entry} future uses {bytes} bytes; keep module calls small on 2 MiB worker stacks");
        }
    }

    #[test]
    fn a_blocked_unverified_extension_is_reported_as_unverified() {
        let unverified = vec!["ue4-ini".to_string()];
        let mut blocked = json!({"success": false, "error_code": "extension_required", "missing_extensions": ["ue4_ini"], "message": "install it"});
        explain_unverified_extensions(&mut blocked, &unverified);
        let blocked = decorate_result("game/lifecycle.py", "start", blocked, "");
        assert_eq!(blocked["error_code"], "extension_not_verified");
        assert_eq!(blocked["unverified_extensions"], json!(["ue4-ini"]));
        assert_eq!(blocked["module"], "game");
        assert_eq!(blocked["function"], "start");
        let mut missing = json!({"error_code": "extension_required", "missing_extensions": ["ue4-ini", "steamcmd"]});
        explain_unverified_extensions(&mut missing, &unverified);
        assert_eq!(missing["error_code"], "extension_required");
    }

    async fn python_process(script: &str) -> Process {
        let python = crate::python_env::detect_system_python().await.expect("test Python");
        let mut cmd = Command::new(python);
        cmd.args(["-c", script]).stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        cmd.process_group(0);
        apply_creation_flags(&mut cmd);
        plain(&mut cmd).unwrap()
    }

    #[tokio::test]
    async fn stdout_larger_than_four_mib_stops_the_run() {
        let mut child = python_process("import sys; sys.stdin.read(); sys.stdout.write('x' * (4 * 1024 * 1024 + 1)); sys.stdout.flush()").await;
        let result = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await;
        assert!(matches!(result, Ok(Err(_))), "stdout beyond the cap must fail");
    }

    #[tokio::test]
    async fn stdout_at_the_four_mib_boundary_is_allowed() {
        let mut child = python_process("import sys; sys.stdin.read(); sys.stdout.write('x' * (4 * 1024 * 1024)); sys.stdout.flush()").await;
        let (_, stdout, _) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
        assert_eq!(stdout.len(), STDOUT_LIMIT);
    }

    #[tokio::test]
    async fn a_timed_out_run_preserves_its_last_twenty_stderr_lines() {
        let mut child = python_process("import sys, time; sys.stdin.read(); [print(i, file=sys.stderr, flush=True) for i in range(30)]; time.sleep(60)").await;
        let failure = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_millis(500)).await.unwrap_err();
        assert_eq!(failure.stderr_tail.len(), 20);
        assert_eq!(failure.stderr_tail.first().unwrap(), "10");
        assert_eq!(failure.stderr_tail.last().unwrap(), "29");
    }

    #[tokio::test]
    async fn stderr_keeps_only_the_last_two_hundred_lines_and_sixty_four_kib() {
        let mut child = python_process("import sys; sys.stdin.read(); [print(str(i) + ':' + 'x' * 1000, file=sys.stderr) for i in range(300)]; print('{}')").await;
        let (_, _, stderr) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
        assert!(stderr.len() <= 64 * 1024, "retained {} bytes", stderr.len());
        assert!(stderr.lines().count() <= 200);
        assert!(stderr.lines().last().unwrap().starts_with("299:"));
        assert!(!stderr.lines().any(|l| l.starts_with("0:")));
    }

    #[tokio::test]
    async fn short_stderr_lines_keep_only_the_last_two_hundred() {
        let mut child = python_process("import sys; sys.stdin.read(); [print(i, file=sys.stderr) for i in range(300)]; print('{}')").await;
        let (_, _, stderr) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
        assert_eq!(stderr.lines().count(), 200);
        assert_eq!(stderr.lines().next(), Some("100"));
        assert_eq!(stderr.lines().last(), Some("299"));
    }

    #[tokio::test]
    async fn host_request_fragments_never_enter_stderr_diagnostics() {
        for script in [
            "import sys; sys.stdin.read(); print('HOST:' + 'sensitive' * 20000, file=sys.stderr); print('visible', file=sys.stderr); print('{}')",
            "import sys; sys.stdin.read(); sys.stderr.buffer.write(('HOST:' + 'x'*32762 + '\\u0a41' + 'sensitive-tail\\nvisible\\n').encode('utf-16-le')); print('{}')",
        ] {
            let mut child = python_process(script).await;
            let (_, _, stderr) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
            assert!(stderr == "visible", "HOST fragments were retained ({} bytes)", stderr.len());
        }
    }

    #[tokio::test]
    async fn host_request_budget_refuses_the_sixty_fifth_request() {
        let dir = tempfile::tempdir().unwrap();
        let script = format!(r#"import json, sys, time
from pathlib import Path
sys.path.insert(0, {:?})
from daemon_rcon import _read_reply
sys.stdin.read()
directory = Path({:?})
codes = []
for number in range(65):
    print('HOST:' + json.dumps({{'id': str(number), 'op': 'config.set', 'args': {{}}}}), file=sys.stderr, flush=True)
for number in range(65):
    path = directory / (str(number) + '.json')
    deadline = time.monotonic() + 10
    codes.append(_read_reply(path, deadline, 'host reply timed out', 1024 * 1024)['error_code'])
print(json.dumps(codes))
"#, std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("modules").to_string_lossy(), dir.path().to_string_lossy());
        let mut child = python_process(&script).await;
        let proxy = host::Proxy { dir: dir.path().into(), context: None, server: None, sandboxed: None };
        let (status, stdout, stderr) = drive(&mut child, b"{}", None, None, Some(proxy), std::time::Duration::from_secs(15)).await.unwrap().unwrap();
        assert!(status.success(), "{stderr}");
        let codes = parse_plugin_output(&stdout).unwrap();
        assert!(codes.as_array().unwrap()[..64].iter().all(|code| code == "op_not_allowed"));
        assert_eq!(codes[64], "too_many_requests");
        assert!(stderr.is_empty());
    }

    #[tokio::test]
    async fn stderr_read_errors_are_not_treated_as_a_clean_eof() {
        struct Broken;
        impl tokio::io::AsyncRead for Broken {
            fn poll_read(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>, _: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::Error::other("broken stderr pipe")))
            }
        }
        let mut reader = BufReader::new(Broken);
        let mut lines = saba_runner::Lines::default();
        assert!(next_stderr_line(&mut reader, &mut lines).await.is_err());
    }

    #[test]
    fn module_failure_fields_survive_anyhow_context() {
        let value = serde_json::json!({"success": false, "module": "game", "function": "start", "error_code": "module_exception", "message": "failed", "path": "world", "stderr_tail": ["detail"]});
        let error = anyhow::Error::new(ModuleCallError::from_response(value.clone())).context("starting server");
        assert_eq!(error_response(&error), value);
    }

    #[test]
    fn isolation_preparation_failures_keep_common_error_fields_and_the_consent_action() {
        let blocked = crate::sandbox::Blocked { code: "sandbox_unavailable", file: None, detail: Some("permission denied".into()) };
        let value = sandbox_failure("game/lifecycle.py", "start", "module-game", &blocked);
        assert_eq!(value["success"], false);
        assert_eq!(value["error_code"], if crate::sandbox::ENFORCED { "module_execution_failed" } else { "isolation_unsupported" });
        assert_eq!(value["phase"], "prepare");
        assert_eq!(value["module"], "game");
        assert_eq!(value["function"], "start");
        assert_eq!(value["action_required"], "sandbox_unavailable");
        assert_eq!(value["component"], "module-game");
        assert_eq!(value["stderr_tail"], serde_json::json!([]));
        assert!(value["error"].as_str().unwrap().contains(if crate::sandbox::ENFORCED { "permission denied" } else { "operating system" }));
    }

    #[tokio::test]
    async fn install_and_general_slots_have_separate_limits() {
        let lanes = CallLanes::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(50);
        let mut installs = Vec::new();
        let mut general = Vec::new();
        for _ in 0..2 {
            installs.push(lanes.acquire("install_server", deadline, true).await.unwrap().unwrap());
        }
        assert!(lanes.acquire("install_server", deadline, true).await.unwrap().is_none());
        for function in ["stop", "start", "status", "command", "functions", "hook", "validate", "players"] {
            general.push(lanes.acquire(function, deadline, true).await.unwrap().unwrap());
        }
        assert!(lanes.acquire("functions", deadline, true).await.unwrap().is_none());
        assert!(lanes.acquire("status", deadline, false).await.is_err(), "queue consumes the deadline");
    }

    #[test]
    fn separate_test_runtimes_do_not_share_call_slots() {
        let first = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let second = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let lanes = first.block_on(async { call_lanes() });
        let _slots = first.block_on(lanes.general.acquire_many(GENERAL_SLOTS as u32)).unwrap();
        second.block_on(async {
            assert!(in_call("game/lifecycle.py", "status", 1, true, async { Ok(()) }).await.unwrap().is_some(),
                "a different test runtime represents an independent daemon");
        });
    }

    #[tokio::test]
    async fn a_reentrant_call_borrows_the_outer_slot_when_all_slots_are_taken() {
        let lanes = CallLanes::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let _all = lanes.general.acquire_many(8).await.unwrap();
        CALL_DEADLINE.scope(CallDeadline::new(deadline), async {
            assert!(lanes.acquire("get_credentials", deadline, true).await.unwrap().is_some());
        }).await;
    }

    #[tokio::test]
    async fn an_internal_query_cannot_extend_the_outer_deadline() {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
        let inherited = CALL_DEADLINE.scope(CallDeadline::new(deadline), in_call("game/lifecycle.py", "functions", 60, false, async {
            Ok(CALL_DEADLINE.with(|deadline| deadline.at))
        })).await.unwrap().unwrap();
        assert_eq!(inherited, deadline);
    }

    #[tokio::test]
    async fn an_http_credentials_callback_does_not_wait_for_a_full_general_lane() {
        use axum::{extract::{Path, State}, http::StatusCode, Json};
        use crate::ipc::{handlers::command, internal::into_internal, IPCServer};
        let dir = tempfile::tempdir().unwrap();
        let modules = dir.path().join("modules");
        let module = modules.join("credentials-boundary");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("module.toml"), "[module]\nschema_version = 1\nname = \"credentials-boundary\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(module.join("lifecycle.py"), "raise AssertionError('a saturated credentials callback must not run Python')\n").unwrap();
        let mut supervisor = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(), dir.path().join("instances").to_str().unwrap(),
        );
        supervisor.module_loader.discover_modules().unwrap();
        let instance = crate::instance::ServerInstance::new("credentials", "credentials-boundary");
        let id = instance.id.clone();
        supervisor.instance_store.add(instance).unwrap();
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(supervisor)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let lanes = call_lanes();
        let _slots = lanes.general.acquire_many(GENERAL_SLOTS as u32).await.unwrap();
        // HTTP 요청 작업은 바깥 모듈의 task-local을 물려받지 않는다
        let mut callback = tokio::spawn(async move {
            into_internal(command::execute_rest_command(Path(id), State(state), Json(serde_json::json!({"endpoint": "info"}))).await).await
        });
        let response = tokio::time::timeout(std::time::Duration::from_secs(1), &mut callback).await;
        if response.is_err() {
            callback.abort();
            let _ = callback.await;
        }
        let response = response.expect("a cold credentials callback must fail immediately instead of deadlocking").unwrap();
        assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.body["error_code"], "module_execution_failed");
        assert_eq!(response.body["phase"], "queue");
        assert_eq!(response.body["module"], "credentials-boundary");
        assert_eq!(response.body["function"], "get_credentials");
    }

    #[tokio::test]
    async fn environment_preparation_shares_the_call_deadline() {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(20);
        let result = prepare("game/lifecycle.py", "start", CallDeadline::new(deadline), async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(())
        }).await;
        let error = error_response(&result.unwrap_err());
        assert_eq!(error["error_code"], "module_timeout");
        assert_eq!(error["module"], "game");
        assert_eq!(error["function"], "start");
        assert_eq!(error["phase"], "prepare");
    }

    #[tokio::test]
    async fn isolation_preparation_keeps_its_slot_past_the_call_deadline() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("lifecycle.py");
        std::fs::write(&entry, "import sys; sys.stdin.read(); print('{\"success\":true}')\n").unwrap();
        let roots = crate::sandbox::LaunchRoots { dirs: Vec::new(), runtimes: Vec::new() };
        let (_, gate) = crate::sandbox::guard_launch_for(
            || saba_chan_updater_lib::integrity::Trust::Verified, "java", &[], std::path::Path::new("."), roots, &mut Vec::new(),
        ).await.unwrap();
        let lanes = call_lanes();
        let (entered, observing) = tokio::sync::oneshot::channel();
        let mut call = tokio::spawn(async move {
            let entry = entry.to_str().unwrap();
            in_call(entry, "status", 1, false, async {
                assert!(entered.send(CALL_DEADLINE.with(|deadline| deadline.isolation.subscribe())).is_ok());
                run_requested_plugin(entry, "status", json!({}), None, 1, None).await
            }).await.map(Option::unwrap)
        });
        let mut isolation = observing.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while isolation.borrow_and_update().waiting == 0 {
                isolation.changed().await.expect("the call reaches isolation preparation");
            }
        }).await.expect("isolation preparation starts");
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let waiting = !call.is_finished();
        let free_slots = lanes.general.available_permits();
        drop(gate);
        // 전역 LaunchGate 대기와 OS 권한 준비는 실행 완료 감시 시간에서도 제외한다.
        let prepared = tokio::time::timeout(std::time::Duration::from_secs(DEFAULT_PLUGIN_TIMEOUT_SECS), async {
            while isolation.borrow_and_update().waiting > 0 {
                if isolation.changed().await.is_err() { break; }
            }
        }).await;
        if prepared.is_err() { call.abort(); }
        prepared.expect("isolation preparation finishes");
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), &mut call).await;
        if result.is_err() { call.abort(); }
        assert!(waiting, "isolation preparation must outlast the execution budget");
        assert_eq!(free_slots, GENERAL_SLOTS - 1, "the isolation wait keeps its slot");
        let result = result.expect("execution finishes after isolation preparation").unwrap().unwrap();
        assert_ne!(result["error_code"], "module_timeout", "{result}");
    }

    #[tokio::test]
    async fn concurrent_function_queries_exclude_the_shared_isolation_preparation() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("lifecycle.py");
        std::fs::write(&entry, "import sys; sys.stdin.read(); print('{\"success\":true,\"functions\":[\"validate\"]}')\n").unwrap();
        let roots = crate::sandbox::LaunchRoots { dirs: Vec::new(), runtimes: Vec::new() };
        let (_, gate) = crate::sandbox::guard_launch_for(
            || saba_chan_updater_lib::integrity::Trust::Verified, "java", &[], std::path::Path::new("."), roots, &mut Vec::new(),
        ).await.unwrap();
        let calls: Vec<_> = (0..3).map(|_| {
            let entry = entry.clone();
            tokio::spawn(async move {
                let entry = entry.to_str().unwrap();
                in_call(entry, FUNCTIONS_QUERY, 1, false, query_functions(entry)).await
            })
        }).collect();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        drop(gate);
        for call in calls {
            let result = call.await.unwrap();
            assert!(result.is_ok(), "shared isolation preparation consumed a caller's budget: {result:?}");
        }
    }

    #[tokio::test]
    async fn a_reentrant_isolation_wait_pauses_the_running_parent() {
        let deadline = CallDeadline::new(tokio::time::Instant::now() + std::time::Duration::from_millis(100));
        let parent = deadline.clone();
        let result = CALL_DEADLINE.scope(deadline, parent.timeout(async {
            in_call("game/lifecycle.py", "get_credentials", 120, false, async {
                let nested = CALL_DEADLINE.with(Clone::clone);
                let _waiting = nested.pause_isolation();
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(())
            }).await
        })).await;
        assert!(result.unwrap().unwrap().is_some(), "the parent keeps its execution budget during reentry");
    }

    #[tokio::test]
    async fn overlapping_isolation_waits_preserve_only_their_union() {
        let deadline = CallDeadline::new(tokio::time::Instant::now() + std::time::Duration::from_millis(60));
        let first = deadline.pause_isolation();
        let second = deadline.pause_isolation();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(first);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(second);
        assert!(deadline.timeout(tokio::time::sleep(std::time::Duration::from_millis(120))).await.is_err(),
            "overlapping preparation must not grant extra execution time");
    }

    #[tokio::test]
    async fn a_shared_query_preserves_preparation_time_between_observer_polls() {
        let deadline = CallDeadline::new(tokio::time::Instant::now() + std::time::Duration::from_millis(100));
        let query = CallDeadline::new(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
        deadline.follow_isolation(&query, async {
            let _waiting = query.pause_isolation();
            std::thread::sleep(std::time::Duration::from_millis(150));
        }).await;
        assert!(prepare("game/lifecycle.py", "status", deadline, async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok(())
        }).await.is_ok(),
            "coalesced watch updates must preserve the caller's next execution budget");
    }

    #[test]
    fn a_modules_own_failure_only_gains_context_except_for_exception_tail() {
        let failure = serde_json::json!({"success": false, "error_code": "extension_required", "message": "keep me", "path": "world"});
        let mut expected = failure.clone();
        expected["module"] = serde_json::json!("game");
        expected["function"] = serde_json::json!("install_server");
        assert_eq!(decorate_result("game/lifecycle.py", "install_server", failure, "detail"), expected);
        let exception = decorate_result("game/lifecycle.py", "start", serde_json::json!({"success": false, "error_code": "module_exception"}), &(0..30).map(|i| i.to_string()).collect::<Vec<_>>().join("\n"));
        assert_eq!(exception["stderr_tail"].as_array().unwrap().len(), 20);
        assert_eq!(exception["stderr_tail"][0], "10");
    }

    #[test]
    fn nested_extension_calls_use_the_manifest_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), r#"{"id":"extension-id"}"#).unwrap();
        let entry = dir.path().join("python_modules/nested/hook.py");
        assert_eq!(module_name(entry.to_str().unwrap()), "extension-id");
    }

    #[tokio::test]
    async fn cancelled_environment_commands_kill_their_process_tree() {
        let python = crate::python_env::detect_system_python().await.expect("test Python");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("processes.json");
        let mut cmd = Command::new(&python);
        cmd.args(["-c", "import json, os, subprocess, sys, time; child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); open(sys.argv[1], 'w').write(json.dumps([os.getpid(), child.pid])); time.sleep(60)"])
            .arg(&path);
        apply_creation_flags(&mut cmd);
        let pending = tokio::spawn(async move { preparation_output(&mut cmd).await });
        let pids: Vec<u32> = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(pids) = serde_json::from_str(&text) { break pids; }
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("the environment command started");
        pending.abort();
        let _ = pending.await;
        for _ in 0..100 {
            if pids.iter().all(|pid| !crate::supervisor::process::is_process_alive(*pid)) { return; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("preparation child survived: {pids:?}");
    }

    #[tokio::test]
    async fn cancelled_module_calls_kill_their_process_tree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("processes.json");
        let script = format!("import json, os, subprocess, sys, time; sys.stdin.read(); child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); open({:?}, 'w').write(json.dumps([os.getpid(), child.pid])); time.sleep(60)", path.to_string_lossy());
        let mut child = python_process(&script).await;
        let pending = tokio::spawn(async move { drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(120)).await });
        let pids: Vec<u32> = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(pids) = serde_json::from_str(&text) { break pids; }
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("the module started");
        pending.abort();
        let _ = pending.await;
        for _ in 0..50 {
            if pids.iter().all(|pid| !crate::supervisor::process::is_process_alive(*pid)) { return; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for pid in &pids { kill_tree(*pid); }
        panic!("cancelled module child survived: {pids:?}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_module_may_request_a_breakaway_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.pid");
        let script = format!("import subprocess, sys, time\nsys.stdin.read()\ntry:\n child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(3)'], creationflags=subprocess.CREATE_BREAKAWAY_FROM_JOB)\nexcept OSError:\n print('{{}}')\nelse:\n open({:?}, 'w').write(str(child.pid))\n time.sleep(3)\n", path.to_string_lossy());
        let mut child = python_process(&script).await;
        let result = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_millis(500)).await;
        if let Ok(Ok((status, _, stderr))) = &result { assert!(status.success(), "{stderr}"); }
        if let Ok(pid) = std::fs::read_to_string(path) {
            let pid = pid.trim().parse().unwrap();
            let alive = crate::supervisor::process::is_process_alive(pid);
            kill_tree(pid);
            assert!(alive, "the explicitly requested breakaway server must survive the module job");
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_successful_start_leaves_its_server_running() {
        let mut child = python_process("import json, subprocess, sys; sys.stdin.read(); server = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(3)'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); print(json.dumps({'pid': server.pid}))").await;
        let (status, stdout, stderr) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
        assert!(status.success(), "{stderr}");
        let pid = parse_plugin_output(&stdout).unwrap()["pid"].as_u64().unwrap() as u32;
        assert!(crate::supervisor::process::is_process_alive(pid), "successful start must preserve its server");
        child.kill(None).await;
    }

    /// 모듈·익스텐션 문구도 그 요청의 언어를 따른다 (CLI `--lang`, Accept-Language)
    #[tokio::test]
    async fn python_gets_the_request_language() {
        let origin = crate::audit::Origin { lang: Some("ko".into()), ..crate::audit::Origin::new("cli") };
        let env = crate::audit::with_origin(origin, async { super::daemon_env() }).await;
        assert!(env.iter().any(|(k, v)| *k == "SABA_LANG" && v == "ko"), "{env:?}");
    }

    /// 모듈·익스텐션의 임시 파일과 pip 캐시는 데이터 폴더 안에 둔다 (제거기가 지운다)
    #[test]
    fn python_temp_files_and_pip_cache_stay_in_the_data_folder() {
        let env = super::daemon_env();
        let data = saba_chan_updater_lib::constants::resolve_data_dir();
        for key in ["TEMP", "TMP", "TMPDIR", "PIP_CACHE_DIR"] {
            let value = env.iter().find(|(k, _)| *k == key).map(|(_, v)| std::path::PathBuf::from(v));
            assert!(value.as_deref().is_some_and(|v| v.starts_with(&data)), "{key}: {value:?}");
        }
    }

    /// 격리 없이 도는 실행은 확인된 익스텐션만 import한다. 격리된 실행은 활성 익스텐션을 모두 쓴다(같은 격리 안에서 돈다)
    #[test]
    fn an_unisolated_run_imports_only_verified_extensions() {
        let enabled = vec!["steamcmd".to_string(), "ue4-ini".to_string()];
        let unverified = |id: &str| id == "ue4-ini";
        assert_eq!(importable_extensions(enabled.clone(), false, unverified, None), (vec!["steamcmd".to_string()], vec!["ue4-ini".to_string()]));
        assert_eq!(importable_extensions(enabled.clone(), true, unverified, None), (enabled, vec![]));
    }

    /// 사용자가 격리 없이 돌리기로 한 익스텐션은 자기 패키지(`extensions.<자기 id>`)를 import할 수 있다. 다른 미확인 익스텐션은 그대로 막는다
    #[test]
    fn an_extension_run_unisolated_by_consent_imports_itself() {
        let enabled = vec!["steamcmd".to_string(), "ue4-ini".to_string(), "other".to_string()];
        let unverified = |id: &str| id != "steamcmd";
        assert_eq!(
            importable_extensions(enabled.clone(), false, unverified, Some("ext-ue4_ini")),
            (vec!["steamcmd".to_string(), "ue4-ini".to_string()], vec!["other".to_string()])
        );
        assert_eq!(importable_extensions(enabled, false, unverified, Some("module-ue4-ini")).1, vec!["ue4-ini".to_string(), "other".to_string()], "a module key is not an extension");
    }

    #[test]
    fn unisolated_imports_require_each_extensions_current_fingerprint() {
        let root = resolve_extensions_dir();
        let first = tempfile::Builder::new().prefix("consent_first_").tempdir_in(&root).unwrap();
        let second = tempfile::Builder::new().prefix("consent_second_").tempdir_in(&root).unwrap();
        let id = |dir: &tempfile::TempDir| dir.path().file_name().unwrap().to_string_lossy().into_owned();
        let (first_id, second_id) = (id(&first), id(&second));
        for dir in [&first, &second] { std::fs::write(dir.path().join("run.py"), "original").unwrap(); }
        crate::sandbox::set_consent(crate::sandbox::Consent::Unisolated, &format!("ext-{first_id}"), true).unwrap();
        let enabled = vec![first_id.clone(), second_id.clone()];
        let mut command = Command::new("python");
        assert_eq!(set_extension_env(&mut command, enabled.clone(), false, Some("module-another")), vec![second_id.clone()]);
        let allowed = command.as_std().get_envs().find(|(key, _)| *key == "SABA_ENABLED_EXTENSIONS").unwrap().1.unwrap();
        assert_eq!(allowed, std::ffi::OsStr::new(&first_id));
        std::fs::write(first.path().join("run.py"), "changed").unwrap();
        assert_eq!(set_extension_env(&mut command, enabled.clone(), false, Some("module-another")), enabled);
    }
    use serde_json::json;

    #[test]
    fn secret_looking_variables_are_not_passed_to_python() {
        for name in ["DISCORD_TOKEN", "openai_api_key", "AWS_SECRET_ACCESS_KEY", "DB_PASSWORD", "GH_APIKEY", "GOOGLE_CREDENTIALS"] {
            assert!(is_secret_env(name), "{name}");
        }
        for name in ["SABA_TOKEN_PATH", "PATH", "APPDATA", "JAVA_HOME", "SYSTEMROOT", "KEYBOARD_LAYOUT"] {
            assert!(!is_secret_env(name), "{name}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_tree_is_killed_with_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!("sleep 30 & echo $! > {}; wait", pidfile.display())).process_group(0);
        let mut child = cmd.spawn().unwrap();
        let mut grandchild = None;
        for _ in 0..50 {
            if let Some(pid) = std::fs::read_to_string(&pidfile).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
                grandchild = Some(pid);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let grandchild = grandchild.expect("child started");
        kill_tree(child.id().unwrap());
        let _ = child.wait().await;
        for _ in 0..50 {
            // 거둘 부모가 없으면 좀비로 남으니 상태도 본다
            let zombie = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            if zombie || !crate::supervisor::process::is_process_alive(grandchild) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived");
    }

    /// 격리된 모듈(127.0.0.1에 못 닿음)이 공용 도우미 `loopback_http`로 자기 서버에 물으면 데몬이 대신 보내고 답을 돌려준다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_sandboxed_module_reaches_its_own_server_through_the_daemon() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut got = vec![0u8; 4096];
            let n = sock.read(&mut got).await.unwrap();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 22\r\n\r\n{\"currentplayernum\":3}").await.unwrap();
            String::from_utf8_lossy(&got[..n]).to_string()
        });
        let relay = tempfile::tempdir().unwrap();
        let script = format!(
            "import json, sys; sys.path.insert(0, {:?}); import daemon_rcon; sys.stdin.read(); \
             status, body = daemon_rcon.loopback_http({port}, 'GET', '/v1/api/metrics', headers={{'Authorization': 'Basic eA=='}}); \
             print(json.dumps({{'status': status, 'body': json.loads(body)}}))",
            concat!(env!("CARGO_MANIFEST_DIR"), "/modules"),
        );
        let mut cmd = Command::new("python3");
        cmd.arg("-c").arg(script).env("SABA_LOOPBACK_DIR", relay.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = plain(&mut cmd).unwrap();
        let proxy = loopback::Proxy { dir: relay.path().to_path_buf(), ports: vec![port], server: Some(std::process::id()) };
        let (status, stdout, stderr) = drive(&mut child, b"{}", None, Some(proxy), None, std::time::Duration::from_secs(20)).await.unwrap().unwrap();
        assert!(status.success(), "{stderr}");
        assert_eq!(parse_plugin_output(&stdout).unwrap(), json!({ "status": 200, "body": { "currentplayernum": 3 } }));
        assert!(server.await.unwrap().starts_with("GET /v1/api/metrics HTTP/1.1\r\n"));
        assert!(!stderr.contains("LOOPBACK:"), "relay requests (they carry auth headers) are not logged");
        assert_eq!(std::fs::read_dir(relay.path()).unwrap().count(), 0, "the module removes the reply it read");
    }

    /// stderr의 UTF-8이 아닌 줄 하나로 읽기가 멈추지 않는다: 멈추면 모듈이 다음 stderr 쓰기에서 끊기거나 파이프가 차서 멈춘다 (D-S7)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stderr_line_that_is_not_utf8_does_not_cut_off_the_run() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(r"cat > /dev/null; printf 'bad \377\n' >&2; head -c 300000 /dev/zero | tr '\0' 'x' | fold -w 100 >&2; echo '{}'")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        let mut child = plain(&mut cmd).unwrap();
        let (status, stdout, stderr) = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(10)).await.unwrap().unwrap();
        assert!(status.success(), "the module was not cut off: {status:?}");
        assert_eq!(stdout.trim(), "{}");
        assert!(stderr.contains("xxxx"), "later stderr lines are still read");
    }

    /// 부모가 끝나도 자식이 출력 파이프를 쥐고 있으면, 출력 수집까지 기한 안에 끊고 자식도 끝낸다
    #[cfg(unix)]
    #[tokio::test]
    async fn the_deadline_covers_output_collection_after_the_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            // 입력을 다 읽고 끝나야 한다: 먼저 끝나면 입력 쓰기가 끊긴 파이프로 실패해 기한까지 가지 않는다
            .arg(format!("cat > /dev/null; sleep 30 & echo $! > {}; echo '{{}}'", pidfile.display()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        let mut child = plain(&mut cmd).unwrap();
        let started = std::time::Instant::now();
        let result = drive(&mut child, b"{}", None, None, None, std::time::Duration::from_secs(1)).await;
        assert!(result.is_err(), "timed out instead of waiting for the held pipe");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let grandchild: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        for _ in 0..50 {
            let zombie = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            if zombie || !crate::supervisor::process::is_process_alive(grandchild) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived");
    }

    /// 부모가 먼저 끝나 트리에서 떨어진 손자도 시간 초과 때 같이 끝낸다 (Job Object)
    #[cfg(windows)]
    #[tokio::test]
    async fn a_timed_out_run_kills_children_even_after_the_parent_exited() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let script = format!(
            "$p = Start-Process ping -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden; Set-Content -Encoding ascii -Path '{}' -Value $p.Id",
            pidfile.display()
        );
        let mut cmd = Command::new(saba_chan_updater_lib::process::system_tool("powershell"));
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let mut child = plain(&mut cmd).unwrap();
        let pid = match &child {
            Process::Plain { child, .. } => child.id(),
            Process::Contained(_) => None,
        };
        let _ = child.wait().await;
        let grandchild: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        assert!(crate::supervisor::process::is_process_alive(grandchild), "grandchild should still run before the kill");
        child.kill(pid).await;
        for _ in 0..100 {
            if !crate::supervisor::process::is_process_alive(grandchild) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("grandchild {grandchild} survived");
    }

    #[test]
    fn shared_modules_are_written_and_refreshed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules");
        ensure_shared_modules(&dir).unwrap();
        std::fs::write(dir.join("i18n.py"), "old").unwrap();
        ensure_shared_modules(&dir).unwrap();
        for (name, content) in SHARED_MODULES {
            assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), content);
        }
    }

    /// 가드는 활성 목록에 없는 `extensions.<이름>` import만 막는다 (`-`와 `_`는 같다)
    #[tokio::test]
    async fn import_guard_blocks_only_disabled_extensions() {
        // PATH의 첫 python이 Windows 스토어 스텁일 수 있으므로 데몬과 같은 탐지로 실제 인터프리터를 고른다
        let Ok(python) = crate::python_env::detect_system_python().await else {
            eprintln!("SKIP: Python >= 3.10 not found (python, python3, py)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let guard = tmp.path().join("guard");
        std::fs::create_dir_all(&guard).unwrap();
        std::fs::write(guard.join("sitecustomize.py"), include_str!("sitecustomize.py")).unwrap();
        for name in ["steamcmd", "ue4_ini"] {
            let dir = tmp.path().join("root").join("extensions").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("__init__.py"), "").unwrap();
        }
        let sep = if cfg!(windows) { ";" } else { ":" };
        let pypath = format!("{}{}{}", guard.display(), sep, tmp.path().join("root").display());
        let probe = "import importlib\nfor m in ('steamcmd', 'ue4_ini'):\n    try:\n        importlib.import_module('extensions.' + m); print(m, 'ok')\n    except ImportError:\n        print(m, 'blocked')";
        let out = std::process::Command::new(&python)
            .args(["-c", probe])
            .env("PYTHONPATH", pypath)
            .env("SABA_ENABLED_EXTENSIONS", "ue4-ini")
            .output()
            .expect("detected python must run");
        assert_eq!(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n").trim(), "steamcmd blocked\nue4_ini ok");
    }

    #[tokio::test]
    async fn pythonpath_entries_cannot_shadow_standard_library_modules() {
        let Ok(python) = crate::python_env::detect_system_python().await else { return };
        let tmp = tempfile::tempdir().unwrap();
        let guard = tmp.path().join("guard");
        let root = tmp.path().join("extensions");
        std::fs::create_dir_all(&guard).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(guard.join("sitecustomize.py"), include_str!("sitecustomize.py")).unwrap();
        std::fs::write(root.join("json.py"), "raise RuntimeError('shadowed')").unwrap();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let pypath = format!("{}{}{}", guard.display(), sep, root.display());
        let out = std::process::Command::new(&python)
            .args(["-c", "import json, sys; print(not sys.argv[1] in json.__file__)"])
            .arg(root.to_string_lossy().as_ref())
            .env("PYTHONPATH", pypath)
            .output()
            .expect("detected python must run");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "True");
    }

    #[test]
    fn plugin_result_is_whole_stdout_or_last_line() {
        assert_eq!(parse_plugin_output("{\n  \"ok\": 1\n}\n").unwrap(), json!({"ok": 1}));
        assert_eq!(parse_plugin_output("debug print\n{\"ok\": 2}\n\n").unwrap(), json!({"ok": 2}));
        assert!(parse_plugin_output("{\"ok\": 3}\nnot json\n").is_err());
    }

    /// 모듈이 `functions`로 밝힌 목록에 없는 선택 함수는 Python을 띄우지 않고 not_supported로 답한다 (MK1).
    /// 목록을 모르는 옛 모듈과 필수 함수는 그대로 부른다
    #[test]
    fn optional_functions_a_module_does_not_list_are_refused_without_running_it() {
        let known = vec!["validate".to_string(), "list_versions".to_string()];
        let refused = unsupported_reply("reset_server", Some(&known)).expect("reset_server is not listed");
        assert_eq!(refused["success"], false);
        assert_eq!(refused["error_code"], "not_supported");
        assert_eq!(refused["function"], "reset_server");
        assert!(unsupported_reply("list_versions", Some(&known)).is_none(), "listed");
        assert!(unsupported_reply("reset_server", None).is_none(), "an old module without the list is asked anyway");
        assert!(unsupported_reply("start", Some(&known)).is_none(), "required functions are never refused");
        assert!(unsupported_reply("functions", Some(&known)).is_none());
    }

    /// 목록은 성공 응답의 문자열 배열에서만 읽는다. `functions`를 모르는 옛 모듈은 Unknown(다시 묻지 않음), 실패·모양이 다른 답은 Failed
    #[test]
    fn the_function_list_is_read_only_from_a_successful_reply() {
        assert_eq!(
            functions_answer(Ok(json!({ "success": true, "functions": ["validate", "diagnose_log"] }))),
            FunctionsAnswer::Listed(vec!["validate".to_string(), "diagnose_log".to_string()])
        );
        assert_eq!(functions_answer(Ok(json!({ "success": false, "message": "Unknown function: functions" }))), FunctionsAnswer::Unknown);
        assert_eq!(functions_answer(Ok(json!({ "success": false, "error_code": "sandbox_consent_required" }))), FunctionsAnswer::Failed);
        assert_eq!(functions_answer(Ok(json!({ "success": true, "functions": "validate" }))), FunctionsAnswer::Failed);
        assert_eq!(functions_answer(Ok(json!({ "success": true, "functions": ["validate", 3] }))), FunctionsAnswer::Failed);
        assert_eq!(functions_answer(Err(anyhow::anyhow!("timed out"))), FunctionsAnswer::Failed);
    }

    /// 답은 진입 파일이 바뀌면 버린다. 실패는 잠시만 기억해 일시적인 실패가 기능을 계속 가리지 않게 한다
    #[test]
    fn a_failed_answer_is_retried_and_a_changed_entry_is_asked_again() {
        let t = Some(std::time::SystemTime::UNIX_EPOCH);
        let now = std::time::Instant::now();
        let old = now.checked_sub(std::time::Duration::from_secs(FUNCTIONS_RETRY_SECS + 1)).unwrap();
        assert!(still_valid(&(t, FunctionsAnswer::Unknown, old), t), "an old module's answer stays");
        assert!(still_valid(&(t, FunctionsAnswer::Listed(vec![]), old), t));
        assert!(still_valid(&(t, FunctionsAnswer::Failed, now), t));
        assert!(!still_valid(&(t, FunctionsAnswer::Failed, old), t), "a failure is asked again later");
        assert!(!still_valid(&(t, FunctionsAnswer::Listed(vec![]), now), Some(std::time::SystemTime::now())), "an updated module is asked again");
    }

    #[tokio::test]
    async fn failed_function_queries_do_not_prevent_the_requested_function() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        for query in ["raise RuntimeError('functions query failed')", "time.sleep(60); print(json.dumps({'success': True, 'functions': []}))"] {
            let dir = tempfile::tempdir().unwrap();
            let entry = dir.path().join("lifecycle.py");
            std::fs::write(dir.path().join("module.toml"), "[module]\nname = 'query-fallback'\n").unwrap();
            std::fs::write(&entry, format!(
                "import json, sys, time\nsys.stdin.read()\n\
                 if sys.argv[1] == 'functions':\n    {query}\n\
                 elif sys.argv[1] == 'validate':\n    print(json.dumps({{'success': True, 'called': sys.argv[1]}}))\n\
                 else:\n    raise RuntimeError('requested function failed')\n"
            )).unwrap();
            crate::sandbox::consent_test_component(&entry);
            let entry = entry.to_str().unwrap();

            let result = run_plugin(entry, "validate", json!({})).await
                .expect("a failed functions query must not prevent validate");
            assert_eq!(result, json!({"success": true, "called": "validate"}));
            assert_eq!(cached_functions(entry), Some(None), "the failed query is cached as an unknown list");

            let error = error_response(&run_plugin(entry, "reset_server", json!({})).await.unwrap_err());
            assert_eq!(error["error_code"], "module_crashed");
            assert_eq!(error["module"], "query-fallback");
            assert_eq!(error["function"], "reset_server");
            assert_eq!(error["phase"], "run");
            assert!(error["stderr_tail"].as_array().unwrap().iter()
                .any(|line| line.as_str().unwrap().contains("requested function failed")), "{error}");
        }
    }

    /// 동시에 온 요청은 Python을 한 번만 띄운다. 목록 응답은 느린 모듈을 짧게만 기다리고, 묻기는 뒤에서 마저 해 캐시에 남긴다
    #[tokio::test]
    async fn concurrent_asks_share_one_run_and_the_list_does_not_wait_for_a_slow_module() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        // Windows에서 확인되지 않은 모듈은 격리되어 모듈 폴더에 쓸 수 없다: 실행 횟수는 답에 실은 pid로 센다
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("lifecycle.py");
        std::fs::write(
            &entry,
            "import json, os, sys, time\ntime.sleep(float(open('delay.txt').read()))\n\
             print(json.dumps({'success': True, 'functions': ['validate', str(os.getpid())]}))\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("delay.txt"), "0.5").unwrap();
        crate::sandbox::consent_test_component(&entry);
        let entry = entry.to_string_lossy().into_owned();
        let first = std::time::Instant::now();
        let asks: Vec<_> = (0..3).map(|_| {
            let entry = entry.clone();
            tokio::spawn(async move { module_functions(&entry).await })
        }).collect();
        let mut answers = Vec::new();
        for ask in asks {
            answers.push(ask.await.unwrap().expect("the shared function query returned a list"));
        }
        // 이 러너에서 Python 한 번을 띄우는 데 드는 시간 (느린 Windows 러너에서는 수 초)
        let startup = first.elapsed().saturating_sub(std::time::Duration::from_millis(500));
        assert_eq!(answers[0][0], "validate");
        // 동시에 돈 실행이라면 pid가 서로 다르다
        assert!(answers.iter().all(|a| a == &answers[0]), "one run for concurrent asks: {answers:?}");

        // 모듈이 바뀌었고 느리다: 목록은 기다리지 않고 null, 뒤에서 마친 답은 캐시에 남는다
        let delay = 4.0;
        std::fs::write(dir.path().join("delay.txt"), delay.to_string()).unwrap();
        let text = std::fs::read_to_string(&entry).unwrap();
        std::fs::write(&entry, format!("{text}# changed\n")).unwrap();
        // 수정 시각 해상도가 거친 파일 시스템에서도 바뀐 것으로 보이게
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&entry).unwrap().set_modified(later).unwrap();
        crate::sandbox::consent_test_component(std::path::Path::new(&entry));
        let started = std::time::Instant::now();
        assert_eq!(module_functions_for_list(entry.clone()).await, None);
        assert!(started.elapsed() < std::time::Duration::from_millis(FUNCTIONS_LIST_WAIT_MS + 1500), "{:?}", started.elapsed());
        let shared = module_functions(&entry).await.expect("the background ask finishes");
        assert_ne!(shared, answers[0], "the changed module was asked again");
        // 새로 실행했다면 목록이 기다린 시간(FUNCTIONS_LIST_WAIT_MS)에 띄우는 시간과 지연 전체가 더해진다.
        // 기준은 그 차이의 절반이다: 같은 실행이면 띄우는 시간 + 지연, 새 실행이면 그보다 2초 더
        let limit = startup + std::time::Duration::from_secs_f64(delay) + std::time::Duration::from_millis(FUNCTIONS_LIST_WAIT_MS / 2);
        assert!(started.elapsed() < limit, "the background ask is shared, not run again: {:?}", started.elapsed());
    }

    /// 공용 진입점 `run_lifecycle`은 표에 `functions`가 없으면 표의 키(자신 제외)로 답한다.
    /// 데몬이 모듈을 돌리는 그 Python으로 돌린다 (PATH의 `python3`는 Windows에서 스토어 안내 스텁일 수 있다)
    #[tokio::test]
    async fn run_lifecycle_answers_functions_with_its_table() {
        let script = format!(
            "import sys; sys.path.insert(0, {:?}); sys.argv = ['lifecycle.py', 'functions']; import daemon_rcon; \
             daemon_rcon.run_lifecycle({{'validate': lambda c: {{}}, 'reset_server': lambda c: {{}}}})",
            concat!(env!("CARGO_MANIFEST_DIR"), "/modules"),
        );
        let python = crate::python_env::get_python_path().await.unwrap();
        let out = std::process::Command::new(python).arg("-c").arg(script).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(
            parse_plugin_output(&String::from_utf8_lossy(&out.stdout)).unwrap(),
            json!({ "success": true, "functions": ["validate", "reset_server"] })
        );
    }

    #[tokio::test]
    async fn test_run_plugin_with_invalid_path() {
        // venv가 없는 환경에서는 get_python_path부터 실패할 수 있음
        let result = run_plugin(
            "nonexistent/module.py",
            "test_function",
            json!({"test": "data"})
        ).await;

        assert!(result.is_err(), "{result:?}");
    }
}
