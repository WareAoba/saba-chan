//! 데몬 수명 — 기동·재사용·종료, 클라이언트 등록·heartbeat, 백그라운드 초기화, 앱 종료 경로

use crate::events::{self, SseEvent};
use crate::{emit_main, settings, tray, AppState};
use serde_json::{json, Value};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// 데몬의 종료 hook 한도(30초, src/main.rs)보다 길게 기다린다: hook이 끝나기 전에 강제 종료하지 않는다 (CLI와 같다)
const STOP_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Default)]
pub struct Lifecycle {
    /// 이 GUI가 띄운 데몬. 강제 종료는 이 PID에만 한다
    child: Mutex<Option<Child>>,
    /// 이 GUI가 띄우지 않았지만 이미 떠 있어 붙은 데몬 (트레이가 꺼짐으로 보이지 않게)
    attached_daemon: AtomicBool,
    /// 이 GUI가 띄운 데몬이 job breakaway를 거절당해 GUI를 띄운 프로그램의 job 안에서 돈다
    daemon_in_parent_job: AtomicBool,
    /// 인터페이스만 종료: 데몬을 남긴다
    keep_daemon_alive: AtomicBool,
    init_started: AtomicBool,
    /// 초기화를 아직 마치지 않았는가. 데몬 단계에서 실패하거나 끊기면 daemon_restart가 성공한 뒤 나머지를 마친다
    init_pending_daemon: AtomicBool,
    ready_sent: AtomicBool,
    quitting: AtomicBool,
    client_id: Mutex<Option<String>>,
    last_status: Mutex<Option<Value>>,
    last_update_key: Mutex<Option<String>>,
    events_stop: Arc<AtomicBool>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
    events_task: Mutex<Option<JoinHandle<()>>>,
    update_task: Mutex<Option<JoinHandle<()>>>,
    /// 다른 인터페이스(CLI의 `gui`)가 이 GUI를 띄우며 고른 서버. 렌더러가 준비되면(app_ready) 넘긴다
    open_server: Mutex<Option<String>>,
    /// 렌더러가 리스너를 달았다 (app_ready를 한 번 받았다). open_server 잠금 안에서만 바꾼다
    renderer_ready: AtomicBool,
    /// 데몬 재시작은 한 번에 하나만. 겹치면 child가 이미 죽은 프로세스를 가리킬 수 있다 (G-U10)
    restart_lock: tokio::sync::Mutex<()>,
}

fn sleep(ms: u64) -> tokio::time::Sleep {
    tokio::time::sleep(Duration::from_millis(ms))
}

fn replace_task(slot: &Mutex<Option<JoinHandle<()>>>, task: Option<JoinHandle<()>>) {
    if let Some(old) = std::mem::replace(&mut *slot.lock().unwrap(), task) {
        old.abort();
    }
}

// ── 상태 표시 ──

/// 렌더러에 초기화 단계 전달. 새로고침한 렌더러에 다시 보내려고 마지막 값을 기억한다
pub fn send_status(app: &AppHandle, step: &str, message: &str) {
    let payload = json!({ "step": step, "message": message, "ts": chrono::Utc::now().timestamp_millis() });
    *app.state::<AppState>().life.last_status.lock().unwrap() = Some(payload.clone());
    emit_main(app, "status:update", payload);
}

fn status_t(app: &AppHandle, step: &str, key: &str) {
    let msg = app.state::<AppState>().t(key, &[]);
    send_status(app, step, &msg);
}

/// `--open-server=<id>`: 다른 인터페이스(CLI의 `gui`)가 고른 서버
pub fn open_server_arg<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    args.into_iter()
        .find_map(|a| a.strip_prefix("--open-server=").map(str::to_string))
        .filter(|id| !id.is_empty())
}

/// 그 서버를 화면에 연다. 렌더러가 아직 준비 전이면 app_ready 때 연다
pub fn open_server(app: &AppHandle, id: String) {
    if let Some(id) = app.state::<AppState>().life.want_server(id) {
        emit_main(app, "app:openServer", json!({ "instanceId": id }));
    }
}

/// 렌더러가 리스너를 단 뒤 부른다. 처음이면 초기화 시작, 다시 부르면(새로고침) 마지막 상태를 다시 보낸다
pub fn app_ready(app: &AppHandle) {
    // 새로 뜬(재로드한) 렌더러는 그 전 닫기 요청을 받지 못했다: 멈춘 것으로 보지 않는다
    crate::window::close_answered();
    let life = &app.state::<AppState>().life;
    if let Some(id) = life.renderer_ready() {
        emit_main(app, "app:openServer", json!({ "instanceId": id }));
    }
    if life.init_started.swap(true, Ordering::SeqCst) {
        if let Some(last) = life.last_status.lock().unwrap().clone() {
            emit_main(app, "status:update", last);
        }
        return;
    }
    status_t(app, "ui", "init.ui_loaded");
    let app = app.clone();
    tauri::async_runtime::spawn(async move { run_background_init(&app).await });
}

async fn run_background_init(app: &AppHandle) {
    app.state::<AppState>().life.begin_init();
    status_t(app, "init", "init.start");
    let up = ensure_daemon(app).await;
    tray::update_menu(app);
    if !up {
        // ready를 보내지 않는다: 렌더러가 다시 시도 / 그대로 열기를 고른다
        status_t(app, "daemon_failed", "daemon.failed_to_start");
        return;
    }
    finish_background_init(app).await;
}

async fn finish_background_init(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.life.claim_init_finish() {
        return;
    }
    // 실제 데이터는 렌더러가 가져온다. 여기서는 로딩 표시만 넘긴다
    status_t(app, "modules", "init.modules");
    status_t(app, "instances", "init.instances");

    register_with_daemon(app).await;
    start_heartbeat(app);
    start_events(app);
    start_update_checker(app);

    status_t(app, "ready", "init.ready");

    // 업데이터가 다시 띄웠으면 첫 ready 뒤 한 번 알린다
    if state.after_update && !state.life.ready_sent.swap(true, Ordering::SeqCst) {
        tracing::info!("[Updater] Detected --after-update flag, notifying renderer");
        emit_main(
            app,
            "updates:completed",
            json!({ "message": state.t("updates.completed", &[]), "timestamp": chrono::Utc::now().to_rfc3339() }),
        );
    }
}

// ── 데몬 기동 ──

pub fn daemon_file_name() -> &'static str {
    if cfg!(windows) {
        "saba-core.exe"
    } else {
        "saba-core"
    }
}

/// 개발: 저장소의 target/release(없으면 debug), 배포: 설치 루트
fn daemon_path(install_root: &Path) -> PathBuf {
    if cfg!(debug_assertions) {
        let release = install_root.join("target").join("release").join(daemon_file_name());
        if release.exists() {
            return release;
        }
        return install_root.join("target").join("debug").join(daemon_file_name());
    }
    install_root.join(daemon_file_name())
}

impl Lifecycle {
    /// 서버를 열라는 요청. 렌더러가 이미 준비됐으면 지금 보낼 id를, 아니면 맡아 두고 None
    fn want_server(&self, id: String) -> Option<String> {
        let mut pending = self.open_server.lock().unwrap();
        if self.renderer_ready.load(Ordering::SeqCst) {
            return Some(id);
        }
        *pending = Some(id);
        None
    }

    /// 렌더러가 준비됐다: 맡아 둔 서버 열기 요청을 꺼낸다
    fn renderer_ready(&self) -> Option<String> {
        let mut pending = self.open_server.lock().unwrap();
        self.renderer_ready.store(true, Ordering::SeqCst);
        pending.take()
    }

    fn shows_running(&self, own_alive: bool) -> bool {
        own_alive || self.attached_daemon.load(Ordering::SeqCst)
    }

    /// heartbeat가 본 데몬 응답 여부를 붙은 데몬 상태에 반영한다. 바뀌었으면 true (트레이를 다시 만든다)
    fn note_reachable(&self, reachable: bool) -> bool {
        self.attached_daemon.swap(reachable, Ordering::SeqCst) != reachable
    }

    /// 초기화 시작. 끝내기 전에 끊기면(렌더러가 멈춤을 알린다) 다시 시도가 나머지를 마친다
    fn begin_init(&self) {
        self.init_pending_daemon.store(true, Ordering::SeqCst);
    }

    fn init_unfinished(&self) -> bool {
        self.init_pending_daemon.load(Ordering::SeqCst)
    }

    /// 나머지 초기화를 맡는다. 원래 초기화와 다시 시도가 겹쳐도 한 번만 true
    fn claim_init_finish(&self) -> bool {
        self.init_pending_daemon.swap(false, Ordering::SeqCst)
    }

    /// 이 GUI가 데몬을 새로 띄웠다: 예전에 붙었던 데몬 표시는 버린다 (이제는 띄운 데몬의 생사만 본다)
    fn own_daemon_started(&self, in_parent_job: bool) {
        self.attached_daemon.store(false, Ordering::SeqCst);
        self.daemon_in_parent_job.store(in_parent_job, Ordering::SeqCst);
    }

    /// 지금 붙어 있는 데몬이 이 GUI가 job 안에 띄운 그 데몬인가 (붙기만 한 데몬이나 이미 끝난 데몬은 아니다)
    pub fn daemon_in_parent_job(&self) -> bool {
        self.daemon_in_parent_job.load(Ordering::SeqCst) && self.child.lock().unwrap().is_some()
    }
}

/// 연결이 거부된 것만 데몬이 없다고 본다. 시간 초과는 바쁜 데몬일 수 있다
fn heartbeat_reached<T>(res: &Result<T, crate::daemon::CallError>) -> bool {
    res.as_ref()
        .err()
        .is_none_or(|e| e.status.is_some() || e.code != "ECONNREFUSED")
}

/// 트레이에 보일 데몬 상태: 이 GUI가 띄운 데몬이 살아 있거나, 이미 떠 있던 데몬에 붙었다
pub fn daemon_running(app: &AppHandle) -> bool {
    let alive = own_daemon_alive(app);
    app.state::<AppState>().life.shows_running(alive)
}

/// 이 GUI가 띄운 데몬이 아직 살아 있는가 (끝났으면 놓는다)
pub fn own_daemon_alive(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let mut guard = state.life.child.lock().unwrap();
    match guard.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(Ok(Some(status))) => {
            tracing::info!("Core Daemon exited with {}", status);
            *guard = None;
            false
        }
        Some(Err(_)) | None => {
            *guard = None;
            false
        }
    }
}

pub fn start_daemon(app: &AppHandle) {
    let state = app.state::<AppState>();
    let root = state.install_root.clone();
    let path = daemon_path(&root);
    tracing::info!(
        "[Daemon] rootDir: {} daemonPath: {} exists: {}",
        root.display(),
        path.display(),
        path.exists()
    );
    if !path.exists() {
        tracing::error!("[Daemon] NOT FOUND: {}", path.display());
        return;
    }
    let s = state.settings();
    // 저장된 포트는 데몬을 새로 띄울 때 적용한다: 띄우는 데몬과 같은 포트로 연결 주소를 맞춘다
    state.refresh_ipc_base();
    let mut cmd = Command::new(&path);
    cmd.arg("--spawned")
        .current_dir(&root)
        .env("RUST_LOG", "info")
        .env("SABA_LANG", settings::language(&s))
        .env("SABA_IPC_PORT", settings::daemon_port(&s).to_string())
        .env("SABA_INSTANCES_PATH", state.data_dir.join("instances"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    detach(&mut cmd, true);
    let mut in_parent_job = false;
    let spawned = cmd.spawn().or_else(|e| {
        // GUI를 띄운 쪽의 job이 breakaway를 막으면 CreateProcess가 거부된다: 그때는 떼지 않고 띄운다.
        // 그러면 GUI를 띄운 프로그램이 꺼질 때 데몬도 함께 꺼질 수 있어 화면에 경고한다 (daemon_status `in_parent_job`)
        if cfg!(windows) {
            tracing::warn!(
                "[Daemon] spawn with job breakaway failed ({}), retrying without: the daemon will run inside the job of the program that launched this app and may stop when that program exits",
                e
            );
            detach(&mut cmd, false);
            in_parent_job = true;
            cmd.spawn()
        } else {
            Err(e)
        }
    });

    match spawned {
        Ok(mut child) => {
            let pid = child.id();
            tracing::info!("[Daemon] spawned with PID: {}", pid);
            // 로그는 GUI가 살아 있는 동안 받고, 그 뒤에는 데몬 버퍼(/api/daemon/console)에 남는다
            if let Some(out) = child.stdout.take() {
                pipe_log(out);
            }
            if let Some(err) = child.stderr.take() {
                pipe_log(err);
            }
            *state.life.child.lock().unwrap() = Some(child);
            state.life.own_daemon_started(in_parent_job);
            watch_daemon_exit(app.clone(), pid);
        }
        Err(e) => tracing::error!("Failed to start Core Daemon: {}", e),
    }
}

/// GUI가 꺼져도 데몬이 살아 있게 떼어 놓는다. 콘솔 창은 띄우지 않는다.
/// `breakaway`: GUI를 띄운 쪽이 kill-on-close job에 넣었어도 데몬은 빠져나온다 (C14)
fn detach(cmd: &mut Command, breakaway: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
        cmd.creation_flags(if breakaway {
            flags | CREATE_BREAKAWAY_FROM_JOB
        } else {
            flags
        });
    }
    #[cfg(not(windows))]
    let _ = breakaway;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 터미널의 Ctrl+C가 데몬까지 가지 않게 프로세스 그룹을 나눈다
        cmd.process_group(0);
    }
}

/// 데몬 출력 → GUI 로그 (tracing WARN/ERROR는 warn으로)
fn pipe_log<R: std::io::Read + Send + 'static>(r: R) {
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(r).lines() {
            let Ok(line) = line else { break };
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            if text
                .split_whitespace()
                .any(|w| w.contains("WARN") || w.contains("ERROR"))
            {
                tracing::warn!("[Daemon] {}", text);
            } else {
                tracing::info!("[Daemon] {}", text);
            }
        }
    });
}

/// 데몬이 끝나면 놓고 트레이를 갱신한다
fn watch_daemon_exit(app: AppHandle, pid: u32) {
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(1000).await;
            let same = app
                .state::<AppState>()
                .life
                .child
                .lock()
                .unwrap()
                .as_ref()
                .map(|c| c.id())
                == Some(pid);
            if !same {
                return;
            }
            if !own_daemon_alive(&app) {
                tray::update_menu(&app);
                return;
            }
        }
    });
}

fn force_kill(child: &mut Child) {
    #[cfg(windows)]
    {
        // /T 없이: 서버는 saba-runner가 쥐고 있어 데몬만 끊고, 다음 기동 때 다시 붙는다
        use saba_chan_updater_lib::process::{hide_console, system_tool};
        let _ = hide_console(&mut Command::new(system_tool("taskkill")))
            .args(["/PID", &child.id().to_string(), "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
    let _ = child.try_wait();
}

/// API로 정상 종료를 요청하고, 시간 안에 안 끝나면 이 GUI가 띄운 PID만 강제 종료한다.
/// 이름 기반 kill은 다른 사용자의 데몬까지 죽일 수 있어 쓰지 않는다
pub async fn stop_daemon(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state.client.post("/api/daemon/shutdown", &json!({}), 2000).await {
        Ok(_) => {
            tracing::info!("Daemon shutdown requested via API");
            state.life.attached_daemon.store(false, Ordering::SeqCst);
        }
        Err(e) => tracing::warn!("Daemon shutdown API failed: {}", e.message),
    }
    if state.life.child.lock().unwrap().is_none() {
        return;
    }
    let start = Instant::now();
    while own_daemon_alive(app) {
        if start.elapsed() >= STOP_TIMEOUT {
            let mut guard = state.life.child.lock().unwrap();
            if let Some(child) = guard.as_mut() {
                tracing::warn!(
                    "Daemon did not exit in {:?}, force killing PID {}",
                    STOP_TIMEOUT,
                    child.id()
                );
                force_kill(child);
            }
            *guard = None;
            break;
        }
        sleep(200).await;
    }
    tracing::info!("Daemon stopped");
}

async fn health_ok(app: &AppHandle, timeout_ms: u64) -> bool {
    app.state::<AppState>().client.get("/health", timeout_ms).await.is_ok()
}

/// 토큰 재로드 후 challenge 응답을 검증한다 (최대 3회, 500ms 간격).
async fn reload_token_until_valid(app: &AppHandle) -> bool {
    let client = app.state::<AppState>().client.clone();
    for retry in 0..3 {
        client.load_token();
        if client.identity_valid(1000).await {
            tracing::info!("Token refreshed and verified");
            return true;
        }
        if let Err(e) = client.get("/health", 500).await {
            tracing::warn!("[Auth] Token verify attempt {} failed: {}", retry + 1, e.message);
        }
        sleep(500).await;
    }
    tracing::warn!("[Auth] Token verification failed after 3 retries, proceeding anyway");
    false
}

/// 데몬이 새로 뜨는 동안 기다린다. 새 토큰을 만들므로 매번 다시 읽고, 토큰이 맞을 때까지 기다린다
/// (/health는 토큰 없이도 답하므로, 응답만 보고 끝내면 렌더러 첫 요청이 옛 토큰으로 401을 받는다)
async fn wait_for_new_daemon(app: &AppHandle) -> bool {
    let client = app.state::<AppState>().client.clone();
    let mut alive = false;
    for _ in 0..20 {
        sleep(500).await;
        client.load_token();
        if client.identity_valid(800).await { return true; }
        alive |= client.get("/health", 500).await.is_ok();
    }
    if alive {
        tracing::warn!("[Auth] New daemon is up but the token file did not match yet");
    }
    false
}

/// 떠 있는 데몬이 있으면 재사용하고, 없으면 띄운다. 데몬이 응답하지 않으면 false
async fn ensure_daemon(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    state.client.load_token();
    status_t(app, "daemon", "daemon.checking");
    match state.client.get("/health", 1000).await {
        Ok(_) => {
            if !state.client.identity_valid(1000).await && !reload_token_until_valid(app).await {
                tracing::info!("Existing daemon detected (stale token). Reloading token...");
                return false;
            }
            tracing::info!("Existing daemon identity verified on IPC port.");
            status_t(app, "daemon", "daemon.existing_running");
            state.life.attached_daemon.store(true, Ordering::SeqCst);
            sync_install_root(app).await;
            true
        }
        // 401 = 예전 데몬이 떠 있지만 토큰이 맞지 않음 (이전 세션 토큰)
        Err(e) if e.status == Some(401) => {
            tracing::info!("Existing daemon detected (auth failed — stale token). Reloading token...");
            if !reload_token_until_valid(app).await { return false; }
            status_t(app, "daemon", "daemon.existing_running");
            state.life.attached_daemon.store(true, Ordering::SeqCst);
            sync_install_root(app).await;
            true
        }
        Err(e) if e.status.is_none() && (e.code == "ECONNREFUSED" || e.code == "ETIMEDOUT") => {
            tracing::info!("No daemon detected, attempting to launch new one...");
            status_t(app, "daemon", "daemon.starting");
            start_daemon(app);
            // 첫 실행은 데이터 폴더 준비로 느릴 수 있다 (최대 10초)
            if wait_for_new_daemon(app).await {
                tracing::info!("Daemon is now running");
                status_t(app, "daemon", "daemon.started");
                sync_install_root(app).await;
                return true;
            }
            tracing::warn!("Daemon did not respond after startup");
            false
        }
        Err(e) => {
            // 데몬은 떠 있지만 다른 문제 — 경고만 하고 진행
            tracing::warn!("Unexpected error checking daemon: {}", e.message);
            let msg = state.t("daemon.check_warning", &[("error", &e.message)]);
            send_status(app, "daemon", &msg);
            state.client.identity_valid(1000).await
        }
    }
}

/// 데몬에 install_root 동기화 (portable 모드에서도 실제 배포 경로를 알린다)
async fn sync_install_root(app: &AppHandle) {
    let state = app.state::<AppState>();
    let root = state.install_root.to_string_lossy().to_string();
    match state
        .client
        .put("/api/updates/config", &json!({ "install_root": root }), 3000)
        .await
    {
        Ok(_) => tracing::info!("[InstallRoot] Synced to daemon: {}", root),
        Err(e) => tracing::warn!("[InstallRoot] Failed to sync: {}", e.message),
    }
}

// ── 클라이언트 등록·heartbeat (데몬이 GUI 생존 여부를 추적) ──

async fn register_with_daemon(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    // PID·설치 폴더: 인스톨러가 이 설치의 GUI를 찾아 끌 수 있게
    let body = json!({ "kind": "gui", "pid": std::process::id(), "installRoot": state.install_root.to_string_lossy() });
    match state.client.post("/api/client/register", &body, 3000).await {
        Ok(v) => {
            let id = v
                .get("client_id")
                .and_then(|i| i.as_str().map(String::from).or_else(|| Some(i.to_string())));
            tracing::info!("[Heartbeat] Registered with daemon as client: {:?}", id);
            *state.life.client_id.lock().unwrap() = id;
            true
        }
        Err(e) => {
            tracing::warn!("[Heartbeat] Failed to register with daemon: {}", e.message);
            false
        }
    }
}

fn start_heartbeat(app: &AppHandle) {
    let app2 = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
        tick.tick().await;
        loop {
            tick.tick().await;
            let state = app2.state::<AppState>();
            let Some(id) = state.life.client_id.lock().unwrap().clone() else {
                // 처음 등록이 실패했으면(데몬이 늦게 뜸 등) 주기마다 다시 시도한다
                register_with_daemon(&app2).await;
                continue;
            };
            let res = state
                .client
                .post(&format!("/api/client/{id}/heartbeat"), &json!({}), 3000)
                .await;
            // 연결이 안 되면 데몬이 없다: 이 GUI 밖에서 꺼진 데몬을 트레이가 실행 중으로 보이지 않게
            if state.life.note_reachable(heartbeat_reached(&res)) {
                tray::update_menu(&app2);
            }
            if let Err(e) = res {
                // 데몬이 재시작되었을 수 있으므로 재등록
                if e.status == Some(404) || e.code == "ECONNREFUSED" {
                    tracing::warn!("[Heartbeat] Lost registration, re-registering...");
                    register_with_daemon(&app2).await;
                }
            }
        }
    });
    replace_task(&app.state::<AppState>().life.heartbeat_task, Some(task));
}

/// 클라이언트 해제. shutdown이면 데몬에 종료도 요청한다
async fn unregister_from_daemon(app: &AppHandle, shutdown: bool) {
    let state = app.state::<AppState>();
    replace_task(&state.life.heartbeat_task, None);
    let Some(id) = state.life.client_id.lock().unwrap().take() else {
        return;
    };
    let qs = if shutdown { "?shutdown=true" } else { "" };
    match state
        .client
        .delete(&format!("/api/client/{id}/unregister{qs}"), 2000)
        .await
    {
        Ok(_) => tracing::info!(
            "[Heartbeat] Unregistered from daemon{}",
            if shutdown { " (with shutdown request)" } else { "" }
        ),
        Err(e) => tracing::warn!("[Heartbeat] Failed to unregister: {}", e.message),
    }
}

// ── 이벤트 스트림·업데이트 알림 ──

fn start_events(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.life.events_stop.store(false, Ordering::SeqCst);
    let (client, stop, app2) = (state.client.clone(), state.life.events_stop.clone(), app.clone());
    let task = tauri::async_runtime::spawn(async move {
        events::run_event_stream(client, stop, move |ev: SseEvent| {
            emit_main(&app2, "events:message", &ev)
        })
        .await;
    });
    replace_task(&state.life.events_task, Some(task));
}

fn start_update_checker(app: &AppHandle) {
    let app2 = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(events::UPDATE_STATUS_POLL);
        loop {
            tick.tick().await;
            check_for_updates(&app2).await;
        }
    });
    replace_task(&app.state::<AppState>().life.update_task, Some(task));
}

/// 데몬이 마지막으로 확인한 결과를 읽어 새 업데이트가 생겼을 때만 알린다
async fn check_for_updates(app: &AppHandle) {
    let state = app.state::<AppState>();
    let data = match state.client.get("/api/updates/status", 5000).await {
        Ok(d) => d,
        Err(e) => return tracing::warn!("[UpdateChecker] Check failed: {}", e.message),
    };
    let Some(notice) = events::update_notice(&data) else {
        if events::updates_cleared(&data) {
            // 받을 것이 없어졌다: 남은 업데이트 알림을 지우고, 다시 생기면 OS 알림도 다시 띄운다
            *state.life.last_update_key.lock().unwrap() = None;
            emit_main(
                app,
                "updates:available",
                json!({ "count": 0, "names": [], "components": [] }),
            );
        }
        return;
    };
    tracing::info!(
        "[UpdateChecker] {} update(s) available: {}",
        notice.names.len(),
        notice.names.join(", ")
    );
    let is_new = state.life.last_update_key.lock().unwrap().as_deref() != Some(notice.key.as_str());
    if is_new {
        let count = notice.names.len().to_string();
        let names = notice.names.join(", ");
        show_notification(
            app,
            &state.t("updates.notification_title", &[]),
            &state.t("updates.notification_body", &[("count", &count), ("names", &names)]),
        );
        *state.life.last_update_key.lock().unwrap() = Some(notice.key.clone());
    } else {
        tracing::info!("[UpdateChecker] Skipping OS notification — same updates already notified");
    }
    // 적용은 사용자가 업데이트 센터에서 고른다
    emit_main(app, "updates:available", notice.payload);
}

pub fn show_notification(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("[Notify] Failed to show notification: {}", e);
    }
}

// ── 재시작 ──

/// main.js daemon:restart
pub async fn daemon_restart(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    let _turn = state.life.restart_lock.lock().await;
    stop_daemon(app).await;
    // 이 GUI가 띄우지 않은 데몬이면 프로세스를 기다릴 수 없으니, 응답이 멎을 때까지 기다린다
    for _ in 0..30 {
        if !health_ok(app, 500).await {
            break;
        }
        sleep(500).await;
    }
    tracing::info!("Starting daemon...");
    state.refresh_ipc_base(); // 포트 변경 반영
    start_daemon(app);
    if !wait_for_new_daemon(app).await {
        state.client.load_token();
        return json!({ "success": false, "error": state.t("daemon.failed_to_start", &[]) });
    }
    // 기동 실패로 멈춘 초기화가 있으면 이어서 끝낸다 (렌더러는 ready를 받고 메인 화면으로 간다)
    if state.life.init_unfinished() {
        sync_install_root(app).await;
        tray::update_menu(app);
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move { finish_background_init(&app2).await });
    }
    tray::update_menu(app);
    json!({ "success": true, "message": "Daemon restarted successfully" })
}

// ── 종료 ──

fn stop_background(state: &AppState) {
    state.life.events_stop.store(true, Ordering::SeqCst);
    replace_task(&state.life.events_task, None);
    replace_task(&state.life.update_task, None);
    replace_task(&state.life.heartbeat_task, None);
}

/// 인터페이스만 종료 — 클라이언트만 해제(shutdown=false)하고 데몬은 그대로 둔다
pub async fn interface_only_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.life.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    tracing::info!("Starting interface-only quit sequence...");
    state.life.keep_daemon_alive.store(true, Ordering::SeqCst);
    unregister_from_daemon(app, false).await;
    stop_background(&state);
    // 자식 핸들만 놓는다 (Child를 버려도 프로세스는 끝나지 않는다)
    state.life.child.lock().unwrap().take();
    tray::remove(app);
    tracing::info!("Interface-only quit sequence completed (daemon kept alive)");
    exit_app(app);
}

/// 완전히 종료 — 클라이언트 해제·데몬 종료 요청 후 데몬이 끝나길 기다린다
pub async fn clean_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.life.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    tracing::info!("Starting clean quit sequence...");
    // 데몬이 종료되면서 관리 중인 봇 프로세스도 함께 정리된다
    unregister_from_daemon(app, true).await;
    stop_background(&state);
    stop_daemon(app).await;
    tray::remove(app);
    tracing::info!("Clean quit sequence completed");
    exit_app(app);
}

/// 이벤트 루프에 종료를 요청한다. 웹뷰가 이미 죽어 요청이 닿지 않으면(Tauri가
/// "failed to exit"만 남기고 돌아온다) 창만 남은 채 멈추므로, 1초 뒤에도 살아 있으면
/// 정리(on_exit)를 직접 하고 프로세스를 끝낸다
fn exit_app(app: &AppHandle) {
    app.exit(0);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(EXIT_WATCHDOG);
        tracing::warn!("Event loop did not exit; forcing process exit");
        on_exit(&app);
        std::process::exit(0);
    });
}

/// 업데이터는 GUI 종료를 기다리지 않고 2초 뒤 새 GUI를 띄우는 경우가 있다. 그보다 먼저 끝나야
/// 새 GUI가 단일 인스턴스에 걸려 그냥 꺼지지 않는다
const EXIT_WATCHDOG: Duration = Duration::from_millis(1000);

pub fn spawn_clean_quit(app: &AppHandle, delay_ms: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(delay_ms).await;
        clean_quit(&app).await;
    });
}

/// 프로세스가 끝나기 직전 (RunEvent::Exit). 위 종료 경로를 거치지 않았으면
/// 기다릴 수 없으니 클라이언트를 짧게 해제하고 자기가 띄운 데몬만 정리한다
pub fn on_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    stop_background(&state);
    if let Some(id) = state.life.client_id.lock().unwrap().take() {
        let client = state.client.clone();
        let _ = tauri::async_runtime::block_on(async move {
            tokio::time::timeout(
                Duration::from_secs(1),
                client.delete(&format!("/api/client/{id}/unregister"), 1000),
            )
            .await
        });
    }
    if !state.life.keep_daemon_alive.load(Ordering::SeqCst) {
        if let Some(mut child) = state.life.child.lock().unwrap().take() {
            if matches!(child.try_wait(), Ok(None)) {
                tracing::info!("Force killing own daemon process at exit");
                force_kill(&mut child);
            }
        }
    }
    tracing::info!("Cleanup completed");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CLI의 `gui`가 붙인 서버 id만 읽는다
    #[test]
    fn the_open_server_argument_is_read() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(open_server_arg(args(&["gui", "--open-server=inst-1"])), Some("inst-1".into()));
        assert_eq!(open_server_arg(args(&["gui", "--open-server="])), None);
        assert_eq!(open_server_arg(args(&["gui", "--after-update"])), None);
    }

    /// 렌더러가 준비되기 전에 온 서버 열기는 맡아 뒀다가 준비될 때 한 번 넘기고, 그 뒤로는 바로 넘긴다
    #[test]
    fn an_open_server_request_waits_for_the_renderer() {
        let life = Lifecycle::default();
        assert_eq!(life.want_server("a".into()), None);
        assert_eq!(life.renderer_ready(), Some("a".into()));
        assert_eq!(life.renderer_ready(), None);
        assert_eq!(life.want_server("b".into()), Some("b".into()));
    }

    #[test]
    fn a_daemon_found_already_running_shows_as_running_until_stopped() {
        let life = Lifecycle::default();
        assert!(!life.shows_running(false));
        life.attached_daemon.store(true, Ordering::SeqCst);
        assert!(
            life.shows_running(false),
            "이 GUI가 띄우지 않은 데몬도 트레이에 실행 중으로 보인다"
        );
        life.attached_daemon.store(false, Ordering::SeqCst);
        assert!(!life.shows_running(false));
        assert!(life.shows_running(true));
    }

    #[test]
    fn heartbeat_reachability_updates_the_attached_daemon() {
        let life = Lifecycle::default();
        life.attached_daemon.store(true, Ordering::SeqCst);
        // 붙은 데몬이 이 GUI 밖에서 꺼지면 heartbeat가 알아채 트레이를 바꾼다
        assert!(life.note_reachable(false));
        assert!(!life.shows_running(false));
        assert!(
            !life.note_reachable(false),
            "바뀐 것이 없으면 트레이를 다시 만들지 않는다"
        );
        assert!(life.note_reachable(true));
        assert!(life.shows_running(false));
    }

    #[test]
    fn only_a_refused_connection_means_the_daemon_is_gone() {
        let err = |status, code| crate::daemon::CallError {
            status,
            body: None,
            message: String::new(),
            code,
        };
        // 바쁜 데몬이 3초 안에 답하지 못한 것은 꺼진 것이 아니다 (트레이가 깜빡이지 않게)
        assert!(heartbeat_reached::<()>(&Err(err(None, "ETIMEDOUT"))));
        assert!(heartbeat_reached::<()>(&Err(err(Some(404), ""))));
        assert!(heartbeat_reached::<()>(&Ok(())));
        assert!(!heartbeat_reached::<()>(&Err(err(None, "ECONNREFUSED"))));
    }

    #[test]
    fn an_init_that_never_finished_is_finished_once_by_a_retry() {
        let life = Lifecycle::default();
        life.begin_init();
        // 초기화가 데몬 단계에서 끊겨도(M11) 다시 시도가 나머지를 한 번만 마친다
        assert!(life.init_unfinished());
        assert!(life.claim_init_finish());
        assert!(!life.init_unfinished());
        assert!(
            !life.claim_init_finish(),
            "다시 시도와 원래 초기화가 겹쳐도 두 번 마치지 않는다"
        );
    }

    #[test]
    fn spawning_our_own_daemon_drops_the_attached_mark() {
        let life = Lifecycle::default();
        life.attached_daemon.store(true, Ordering::SeqCst);
        life.own_daemon_started(false);
        // 띄운 데몬이 죽으면 트레이에 꺼짐으로 보여야 한다
        assert!(!life.shows_running(false));
    }
}
