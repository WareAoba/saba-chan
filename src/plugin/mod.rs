use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use crate::utils::apply_creation_flags;

mod loopback;

/// 데몬 IPC 포트. 모듈이 데몬 API를 역호출할 때 쓴다(main이 기동 시 한 번 설정).
static IPC_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

pub fn set_ipc_port(port: u16) {
    let _ = IPC_PORT.set(port);
}

/// Python 실행마다 넣는 데몬 연결 정보.
/// - `SABA_IPC_PORT`, `SABA_IPC_URL`: 데몬 API 주소
/// - `SABA_TOKEN_PATH`: `X-Saba-Token` 값이 든 파일 (토큰 자체는 env에 넣지 않는다)
/// - `SABA_LANG`: 지금 GUI 언어(고르기 전에는 OS 언어). 언어를 바꿔도 데몬은 재시작하지 않으므로 매번 읽는다.
fn daemon_env() -> Vec<(&'static str, String)> {
    let port = *IPC_PORT.get().unwrap_or(&saba_chan_updater_lib::constants::DEFAULT_IPC_PORT);
    let mut env = vec![
        ("SABA_IPC_PORT", port.to_string()),
        ("SABA_IPC_URL", format!("http://127.0.0.1:{}", port)),
        (
            "SABA_TOKEN_PATH",
            saba_chan_updater_lib::constants::token_file_path().to_string_lossy().into_owned(),
        ),
    ];
    let lang = saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get("language")
        .and_then(|v| v.as_str())
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        // 언어를 고르기 전에는 GUI처럼 OS 언어를 따른다
        .unwrap_or_else(crate::chat::i18n::os_language);
    env.push(("SABA_LANG", lang));
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
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        nix::libc::kill(-(pid as i32), nix::libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
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
    pub entry: String,
    pub function: String,
    pub config: Value,
    /// 데몬이 추적하는 그 인스턴스의 서버 PID: 격리 실행의 루프백 중계는 이 프로세스(트리)가 듣는 포트로만 잇는다
    pub server: Option<u32>,
}

impl PluginCall {
    pub fn new(entry: impl Into<String>, function: &str, config: Value) -> Self {
        Self { entry: entry.into(), function: function.to_string(), config, server: None }
    }

    pub fn serving(mut self, server: Option<u32>) -> Self {
        self.server = server;
        self
    }

    pub async fn run(self) -> Result<Value> {
        run_plugin_inner(&self.entry, &self.function, self.config, None, DEFAULT_PLUGIN_TIMEOUT_SECS, self.server).await
    }
}

/// Like `run_plugin` but with a custom timeout (seconds).
/// 서버 설치·업데이트(`install_server`) 한도: 대용량 다운로드를 허용한다
pub const INSTALL_TIMEOUT_SECS: u64 = 1800;

pub async fn run_plugin_with_timeout(module_path: &str, function: &str, config: Value, timeout_secs: u64) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, timeout_secs, None).await
}

/// Like `run_plugin_with_progress` but with a custom timeout.
/// SteamCMD 다운로드 등 장시간 작업에 사용합니다.
pub async fn run_plugin_with_progress_and_timeout<F>(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: F,
    timeout_secs: u64,
) -> Result<Value>
where
    F: Fn(ExtensionProgress) + Send + 'static,
{
    run_plugin_inner(module_path, function, config, Some(Box::new(on_progress)), timeout_secs, None).await
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

fn plain(child: tokio::process::Child) -> Process {
    Process::Plain {
        #[cfg(windows)]
        job: windows_job::contain(&child),
        child,
    }
}

/// 평소대로 띄운 Python을 넣는 Job Object (Windows).
/// 끝낼 때만 쓴다: 닫아도 안의 프로세스는 남는다(모듈 `start`가 띄우고 PID만 돌려주는 서버가 있다).
/// Job 탈출을 요청하는 프로세스는 막지 않는다 (예전처럼 띄울 수 있게)
#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject};
    use winapi::um::winnt::{JobObjectExtendedLimitInformation, HANDLE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK};

    /// 띄운 직후에 넣는다. 그 전에 띄운 손자는 없다고 본다 (Python이 시작하는 데 걸리는 시간보다 짧다).
    /// 실패하면 None: 예전처럼 `taskkill /T`만 쓴다
    pub(super) fn contain(child: &tokio::process::Child) -> Option<OwnedHandle> {
        let process = child.raw_handle()? as HANDLE;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                tracing::warn!("Plugin job object: {}", std::io::Error::last_os_error());
                return None;
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
                tracing::warn!("Plugin job object: {}", std::io::Error::last_os_error());
                return None;
            }
            Some(job)
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
        crate::sandbox::ENFORCED && crate::sandbox::restricted(crate::sandbox::extension_trust(id))
    }, consented);
    cmd.env("SABA_ENABLED_EXTENSIONS", importable.join(","));
    // 가드가 "켜져 있지 않음" 대신 "확인되지 않아 쓸 수 없음"이라고 알리게
    cmd.env("SABA_UNVERIFIED_EXTENSIONS", unverified.join(","));
    unverified
}

/// 모듈이 import 실패를 삼키고 `extension_required`로 답해도(팰월드는 맨 위에서 ue4_ini import 실패를 받아 둔다),
/// 빠진 익스텐션이 모두 확인되지 않아 막은 것이면 설치·활성화 안내 대신 `extension_not_verified`로 바꾼다
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

async fn run_plugin_inner(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
    server: Option<u32>,
) -> Result<Value> {
    tracing::debug!("Executing plugin: {} -> {}", module_path, function);

    let config_json = serde_json::to_string(&config)?;
    let python_exe = crate::python_env::get_python_path().await?;
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
    let modules_lib_dir = resolve_modules_dir();
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

    apply_creation_flags(&mut cmd);

    let mut child = match &isolation {
        None => plain(cmd.spawn()?),
        Some(iso) => match crate::sandbox::spawn_isolated(cmd.as_std(), iso).await {
            Ok(crate::sandbox::Spawned::Unisolated) => {
                // 사용자가 격리 없이 돌리기로 한 것은 이 구성요소뿐이다: 확인되지 않은 다른 익스텐션은 import하지 못하게
                if let Some(enabled) = enabled_extensions {
                    unverified_extensions = set_extension_env(&mut cmd, enabled, false, Some(&iso.key));
                }
                // 격리 없이 돌면 루프백에 직접 닿는다
                cmd.env_remove("SABA_LOOPBACK_DIR");
                loopback = None;
                plain(cmd.spawn()?)
            }
            #[cfg(windows)]
            Ok(crate::sandbox::Spawned::Contained(contained)) => Process::Contained(contained),
            Err(blocked) => {
                let mut value = blocked.to_json(crate::sandbox::message(&blocked));
                value["component"] = Value::String(iso.key.clone());
                return Ok(value);
            }
        },
    };
    let status = drive(&mut child, config_json.as_bytes(), on_progress, loopback, std::time::Duration::from_secs(timeout_secs)).await;

    match status {
        Ok(Ok((exit_status, stdout_str, stderr_str))) => {
            if !exit_status.success() {
                tracing::error!("Plugin failed (exit {:?}):\n  stderr: {}\n  stdout: {}", exit_status.code(), stderr_str, stdout_str.trim());
                return Err(anyhow::anyhow!("Plugin execution failed (exit {:?}):\nstderr: {}\nstdout: {}", exit_status.code(), stderr_str, stdout_str.trim()));
            }

            match parse_plugin_output(&stdout_str) {
                Ok(mut result) => {
                    tracing::debug!("Plugin result (raw): {}", stdout_str.trim());
                    explain_unverified_extensions(&mut result, &unverified_extensions);
                    Ok(result)
                }
                Err(e) => {
                    tracing::error!("Failed to parse plugin JSON: {} | stdout: {}", e, stdout_str);
                    Err(anyhow::anyhow!("Invalid JSON from plugin: {}\nOutput: {}", e, stdout_str))
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Plugin process error: {}", e);
            Err(anyhow::anyhow!("Plugin process error: {}", e))
        }
        Err(_) => {
            // 타임아웃 — drive가 프로세스 트리를 이미 끝냈다
            tracing::warn!(
                "Plugin timed out after {}s: {} -> {} — killed process",
                timeout_secs, module_path, function
            );
            Err(anyhow::anyhow!("Plugin timed out after {}s", timeout_secs))
        }
    }
}

type Collected = std::io::Result<(std::process::ExitStatus, String, String)>;

/// 입력 쓰기부터 종료 대기, stdout·stderr 수집까지 한 기한 안에 한다.
/// 기한을 넘기거나 입출력이 실패하면 프로세스 트리를 끝내고 읽기 작업을 멈춘다.
/// (손자 프로세스가 파이프를 물려받아 쥐고 있으면 부모가 끝나도 출력이 닫히지 않는다)
async fn drive(
    child: &mut Process,
    input: &[u8],
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    loopback: Option<loopback::Proxy>,
    timeout: std::time::Duration,
) -> std::result::Result<Collected, tokio::time::error::Elapsed> {
    let pid = match child {
        Process::Plain { child, .. } => child.id(),
        #[cfg(windows)]
        Process::Contained(_) => None,
    };
    let (stdin_pipe, stdout_pipe, stderr_pipe) = child.pipes();

    // Stream stderr line-by-line to capture PROGRESS: events in real-time
    let stderr_handle = tokio::spawn(async move {
        let mut log_lines = Vec::new();
        if let Some(pipe) = stderr_pipe {
            let mut reader = BufReader::new(pipe).lines();
            let mut relayed = 0;
            while let Ok(Some(line)) = reader.next_line().await {
                // 중계 요청은 로그에 남기지 않는다 (인증 머리글이 들어 있을 수 있다)
                if let Some(request) = line.strip_prefix(loopback::PREFIX) {
                    match &loopback {
                        Some(proxy) if relayed < loopback::MAX_REQUESTS => {
                            relayed += 1;
                            let (proxy, request) = (proxy.clone(), request.to_string());
                            tokio::spawn(async move { loopback::serve(&proxy, &request).await });
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
                } else {
                    tracing::debug!("Plugin stderr: {}", line);
                }
                log_lines.push(line);
            }
        }
        log_lines.join("\n")
    });

    // Read all of stdout (final JSON result)
    let stdout_handle = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
        }
        String::from_utf8_lossy(&buf).to_string()
    });

    let readers = [stderr_handle.abort_handle(), stdout_handle.abort_handle()];

    let work = async {
        if let Some(mut stdin) = stdin_pipe {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
        }
        let exit_status = child.wait().await?;
        let stderr_str = stderr_handle.await.unwrap_or_default();
        let stdout_str = stdout_handle.await.unwrap_or_default();
        Ok((exit_status, stdout_str, stderr_str))
    };
    let result = tokio::time::timeout(timeout, work).await;
    if !matches!(result, Ok(Ok(_))) {
        child.kill(pid).await;
        for reader in readers {
            reader.abort();
        }
    }
    result
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
        let mut child = plain(cmd.spawn().unwrap());
        let proxy = loopback::Proxy { dir: relay.path().to_path_buf(), ports: vec![port], server: Some(std::process::id()) };
        let (status, stdout, stderr) = drive(&mut child, b"{}", None, Some(proxy), std::time::Duration::from_secs(20)).await.unwrap().unwrap();
        assert!(status.success(), "{stderr}");
        assert_eq!(parse_plugin_output(&stdout).unwrap(), json!({ "status": 200, "body": { "currentplayernum": 3 } }));
        assert!(server.await.unwrap().starts_with("GET /v1/api/metrics HTTP/1.1\r\n"));
        assert!(!stderr.contains("LOOPBACK:"), "relay requests (they carry auth headers) are not logged");
        assert_eq!(std::fs::read_dir(relay.path()).unwrap().count(), 0, "the module removes the reply it read");
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
        let mut child = plain(cmd.spawn().unwrap());
        let started = std::time::Instant::now();
        let result = drive(&mut child, b"{}", None, None, std::time::Duration::from_secs(1)).await;
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
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let mut child = plain(cmd.spawn().unwrap());
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

    // PC 실사용: 바뀐 ue4-ini를 확인된 팰월드가 import하지 못했는데, 모듈이 실패를 삼켜 "설치하고 활성화하세요"가 떴다
    #[test]
    fn a_blocked_unverified_extension_is_reported_as_unverified() {
        let unverified = vec!["ue4-ini".to_string()];
        let mut blocked = json!({"success": false, "error_code": "extension_required", "missing_extensions": ["ue4_ini"], "message": "install it"});
        explain_unverified_extensions(&mut blocked, &unverified);
        assert_eq!(blocked["error_code"], "extension_not_verified");
        assert_eq!(blocked["unverified_extensions"], json!(["ue4-ini"]));

        // 정말 빠진 것이 섞여 있으면 설치 안내가 맞다
        let mut missing = json!({"error_code": "extension_required", "missing_extensions": ["ue4-ini", "steamcmd"]});
        explain_unverified_extensions(&mut missing, &unverified);
        assert_eq!(missing["error_code"], "extension_required");
    }

    #[test]
    fn plugin_result_is_whole_stdout_or_last_line() {
        assert_eq!(parse_plugin_output("{\n  \"ok\": 1\n}\n").unwrap(), json!({"ok": 1}));
        assert_eq!(parse_plugin_output("debug print\n{\"ok\": 2}\n\n").unwrap(), json!({"ok": 2}));
        assert!(parse_plugin_output("{\"ok\": 3}\nnot json\n").is_err());
    }

    #[tokio::test]
    async fn test_run_plugin_with_invalid_path() {
        // venv가 없는 환경에서는 get_python_path부터 실패할 수 있음
        let result = run_plugin(
            "nonexistent/module.py",
            "test_function",
            json!({"test": "data"})
        ).await;

        assert!(result.is_err());
    }
}
