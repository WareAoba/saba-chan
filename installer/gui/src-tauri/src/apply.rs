//! 업데이트 적용 (`--apply`)
//!
//! 데몬이 다운로드해둔 업데이트 파일을 적용하고 GUI를 재실행합니다.
//!
//! ## 실행
//! ```text
//! saba-chan-installer --apply [--wait-pid <pid>] --launch-token <t> [--headless] [--relaunch <exe> [extra...]]
//! ```
//!
//! `--headless`(헤드리스 설치에서 데몬이 붙인다)면 창을 띄우지 않고 바로 적용한 뒤 `--relaunch`만 다시 띄운다.
//! 설치된 GUI를 찾아 띄우는 예비 동작은 쓰지 않는다. 종료 코드는 적용이 다 됐으면 0, 아니면 1이다.
//!
//! 보통 데몬이 띄운다(`POST /api/updates/launch-updater`, U7). `--wait-pid`는 데몬 PID로,
//! 데몬이 끝날 때까지(최대 30초) 기다린 뒤 적용한다. 30초 뒤에도 살아 있으면 파일을 바꾸지 않고 `deferred`로 끝낸다.
//! 코어를 바꿨으면 다시 띄운 새 데몬의 `/health`를 지켜보고, 건강하지 않으면 이번 묶음과 데이터 스냅샷을 되돌린 뒤
//! 이전 데몬을 띄운다 (`saba_chan_updater_lib::live_apply`). 결과는 성공·실패 모두
//! 완료 마커(`update-complete.json`)에 남고, 다음 데몬이 `/api/updates/last-result`로 보여준다.
//! 0.1.12 전에는 이 일을 따로 있던 `saba-chan-updater`가 했다. 적용하고 나면 남은 그 파일을 걷는다
//!
//! ## 데이터 소스
//! - `%APPDATA%/saba-chan/updates/apply-targets.json` — 적용 대상 컴포넌트
//! - `%APPDATA%/saba-chan/updates/pending.json` — 다운로드 파일 위치
//! - `%APPDATA%/saba-chan/settings.json` — 언어 설정
//!
//! ## 설계 원칙
//! - install_root는 자기 exe 경로에서 자동 추론 (CLI 인자 불필요)
//! - 적용 대상은 apply-targets.json에서 읽음 (CLI 인자 불필요)
//! - 테마는 CSS `data-theme="auto"` + `prefers-color-scheme` 미디어 쿼리로 자동 처리

use saba_chan_updater_lib::live_apply::{self, LiveApply};
use saba_chan_updater_lib::{UpdateManager, UpdateCompletionMarker};
use std::path::{Path, PathBuf};
#[cfg(feature = "window")]
use std::sync::Arc;
#[cfg(feature = "window")]
use tauri::{AppHandle, Emitter, Manager};
#[cfg(feature = "window")]
use tokio::sync::RwLock;
use saba_chan_updater_lib::constants;

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

#[cfg(feature = "window")]
type ManagerState = Arc<RwLock<UpdateManager>>;

#[cfg(feature = "window")]
/// 재실행 설정 (Tauri managed state)
#[derive(Debug, Clone, Default)]
struct ApplyConfig {
    relaunch_exe: Option<String>,
    relaunch_extra: Vec<String>,
    /// `--wait-pid`로 받은 예전 데몬 (새 데몬과 가리는 데 쓴다)
    wait_pid: Option<u32>,
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드
// ═══════════════════════════════════════════════════════

#[cfg(feature = "window")]
/// Apply 모드 정보 — 프론트엔드 `init()` 에서 호출
/// 항상 `enabled: true`를 반환 (apply 전용 바이너리)
#[tauri::command]
async fn get_apply_mode(config: tauri::State<'_, ApplyConfig>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "enabled": true,
        "components": [],
        "relaunch": config.relaunch_exe.is_some(),
    }))
}

#[cfg(feature = "window")]
/// Apply 실행 — 매니페스트 로드 → 파일 적용 → 완료 마커 → 재실행
#[tauri::command]
async fn start_apply(
    app: AppHandle,
    apply_config: tauri::State<'_, ApplyConfig>,
    manager: tauri::State<'_, ManagerState>,
) -> Result<Vec<String>, String> {
    let (result, live) = {
        let mut mgr = manager.write().await;
        let live = prepare_live_apply(&mut mgr);
        let result = mgr
            .apply_staged(|p| {
                app.emit("apply:progress", p).ok();
            })
            .await;
        (result, live)
    };
    let core_applied = core_was_applied(&result);
    let watch = Watch { live, core_applied, manager: manager.inner().clone() };
    match result {
        Err(msg) => {
            relaunch(&app, &apply_config, false, watch);
            Err(msg)
        }
        Ok((applied, failed)) => {
            after_apply();
            // GUI 재실행 → 업데이터 종료 (하나라도 실패했으면 완료 알림 없이 띄우고 오류 화면을 남긴다)
            relaunch(&app, &apply_config, failed.is_empty(), watch);
            Ok(applied)
        }
    }
}

#[cfg(feature = "window")]
/// 업데이트를 띄운 프로그램(없으면 GUI)을 다시 띄운다. 적용이 실패해도 띄운다:
/// 업데이터가 뜰 때 데몬·GUI는 이미 끝났으니 그대로 두면 아무것도 남지 않는다.
/// `applied`면 `--after-update`를 붙이고(받는 쪽이 완료 알림을 띄운다) 띄운 뒤 업데이터를 닫는다.
/// 실패면 플래그 없이 띄우고, 오류 화면을 닫을 때까지 맨 앞에 남겨 둔다
/// 코어를 바꿨으면 다시 띄운 뒤 새 데몬을 지켜보는 동안 창을 숨기고, 건강하지 않으면 되돌린 뒤 이전 판을 다시 띄운다
fn relaunch(app: &AppHandle, apply_config: &ApplyConfig, applied: bool, watch: Watch) {
    let relaunch_exe = apply_config.relaunch_exe.clone();
    let relaunch_extra = apply_config.relaunch_extra.clone();
    let wait_pid = apply_config.wait_pid;
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        let gui = resolve_gui_exe();
        let has_target = relaunch_exe.is_some() || gui.is_some();
        let since_ms = live_apply::now_ms();
        let launched = relaunch_now(relaunch_exe.clone(), relaunch_extra.clone(), gui.clone(), applied).await;
        let relaunched = launched.is_some();
        if watch.live.is_some() && launched.is_some() {
            if let Some(win) = app_handle.get_webview_window("main") {
                win.hide().ok();
            }
        }
        let Watch { live, core_applied, manager } = watch;
        let manager = manager.read().await;
        let target = Relaunch { exe: relaunch_exe, extra: relaunch_extra, gui };
        watch_new_daemon(live, &manager, core_applied, launched, wait_pid, since_ms, target).await;

        if applied && (relaunched || !has_target) {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            app_handle.exit(0);
        } else if applied {
            // 파일은 바꿨지만 앱(데몬·GUI)이 꺼진 채 남는다: 창을 닫지 않고 직접 실행하라고 알린다
            tracing::error!("[Apply] Update applied but the app could not be relaunched");
            app_handle.emit("apply:relaunch-failed", ()).ok();
            if let Some(win) = app_handle.get_webview_window("main") {
                win.set_always_on_top(true).ok();
                win.set_focus().ok();
            }
        } else if let Some(win) = app_handle.get_webview_window("main") {
            // 다시 띄운 GUI 창이 오류 화면을 덮지 않게, 사용자가 닫을 때까지 맨 앞에 둔다
            win.set_always_on_top(true).ok();
            win.set_focus().ok();
        }
    });
}

#[cfg(feature = "window")]
/// 테마 조회 — settings.json → "auto"
/// CSS `data-theme` + `prefers-color-scheme` 미디어 쿼리로 자동 처리되므로
/// 대부분 "auto"가 반환됨 (향후 GUI가 settings.json에 theme 저장 시 자동 대응)
#[tauri::command]
async fn get_theme() -> Result<String, String> {
    Ok(load_setting("theme").unwrap_or_else(|| "auto".to_string()))
}

// ═══════════════════════════════════════════════════════
// 헬퍼
// ═══════════════════════════════════════════════════════

/// 다시 띄운 프로그램
#[derive(Debug, Clone, PartialEq)]
struct Launched {
    cmd: String,
    pid: u32,
}

/// 다시 띄울 것 (되돌린 뒤 이전 판을 다시 띄울 때도 쓴다)
struct Relaunch {
    exe: Option<String>,
    extra: Vec<String>,
    gui: Option<PathBuf>,
}

/// 다시 띄운다: `--relaunch`로 받은 것, 없으면 `installed_gui`(헤드리스는 `None`을 넘겨 아무것도 띄우지 않는다)
/// 다시 띄웠으면 그 프로그램 (띄울 것이 없거나 모두 실패하면 None)
async fn relaunch_now(relaunch_exe: Option<String>, relaunch_extra: Vec<String>, installed_gui: Option<PathBuf>, applied: bool) -> Option<Launched> {
    let requested = relaunch_exe.is_some();
    let cmd = relaunch_command(relaunch_exe, installed_gui)?;
    // 받은 인자는 받은 프로그램의 것이다. --relaunch 없이 찾은 GUI에는 넘기지 않는다
    let extra: &[String] = if requested { &relaunch_extra } else { &[] };
    wait_for_exe(&cmd).await;
    let after_update = applied && marks_update(&cmd, resolve_install_root_from_exe().as_deref());
    let pid = launch_with_retry(&cmd, extra, after_update).await?;
    Some(Launched { cmd, pid })
}

/// 코어 적용을 지켜볼 준비 (데이터 스냅샷·`.old` 목록·감시 표시). 코어가 바뀌지 않는 묶음이면 None
fn prepare_live_apply(manager: &mut UpdateManager) -> Option<LiveApply> {
    LiveApply::prepare(manager, &constants::resolve_data_dir(), &constants::resolve_instances_dir())
}

fn core_was_applied(result: &Result<(Vec<String>, Vec<String>), String>) -> bool {
    matches!(result, Ok((applied, _)) if applied.iter().any(|k| k == "saba-core"))
}

#[cfg(feature = "window")]
/// 창 업데이터가 적용한 뒤 넘기는 감시 준비
struct Watch {
    live: Option<LiveApply>,
    core_applied: bool,
    manager: ManagerState,
}

/// 새 데몬으로 보는 규칙: 데몬을 직접 띄웠으면 그 PID, GUI·서비스 관리자가 띄우면 다시 띄운 뒤 시작한 데몬
fn health_rule(launched: &Launched, old_pid: Option<u32>, since_ms: u64) -> live_apply::HealthRule {
    if is_daemon_exe(&launched.cmd) {
        live_apply::HealthRule::SpawnedPid(launched.pid)
    } else {
        live_apply::HealthRule::StartedAfter { since_ms, old_pid }
    }
}

fn is_daemon_exe(cmd: &str) -> bool {
    let name = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd);
    name.eq_ignore_ascii_case("saba-core") || name.eq_ignore_ascii_case("saba-core.exe")
}

/// 되돌린 뒤 이전 판을 띄우는 명령. 서비스는 이미 떠 있을 수 있으니(관리자가 새 판을 다시 띄움) 다시 시작한다:
/// `systemctl --user start` → `restart`, `launchctl kickstart` → `kickstart -k`
fn rollback_relaunch(exe: Option<String>, mut extra: Vec<String>) -> (Option<String>, Vec<String>) {
    let name = exe.as_deref().and_then(|e| Path::new(e).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name == "systemctl" {
        extra.iter_mut().filter(|a| *a == "start").for_each(|a| *a = "restart".into());
    } else if name == "launchctl" && extra.first().is_some_and(|a| a == "kickstart") && !extra.iter().any(|a| a == "-k") {
        extra.insert(1, "-k".into());
    }
    (exe, extra)
}

/// 코어를 바꿨으면 새 데몬을 지켜본다. 건강하면 스냅샷과 `.old`를 지우고 true. 건강하지 않으면 새 데몬(과 함께 띄운 GUI)을
/// 끝내고 되돌린 뒤 릴레이에 한 번 알리고 이전 판을 다시 띄운 뒤 false. 지켜볼 것이 없으면 true
async fn watch_new_daemon(
    live: Option<LiveApply>,
    manager: &UpdateManager,
    core_applied: bool,
    launched: Option<Launched>,
    old_pid: Option<u32>,
    since_ms: u64,
    target: Relaunch,
) -> bool {
    let Some(live) = live else { return true };
    let Some(launched) = launched.filter(|_| core_applied) else {
        live.skip();
        return true;
    };
    let rule = health_rule(&launched, old_pid, since_ms);
    tracing::info!("[Apply] Watching the new daemon {} ({:?})", live.target_version, rule);
    let reason = match live_apply::watch(&live_apply::HttpProbe::local(), &live.target_version, &rule, live_apply::WatchLimits::default()).await {
        live_apply::WatchOutcome::Healthy { pid } => {
            tracing::info!("[Apply] The new daemon is healthy (PID {})", pid);
            live.healthy(manager);
            return true;
        }
        live_apply::WatchOutcome::Unhealthy { pid, reason } => {
            // 직접 띄운 사바쨩 프로그램(데몬·GUI)만 끝낸다. 서비스 관리자 명령은 이미 끝났고 그 PID는 다른 프로세스일 수 있다
            if resolve_install_root_from_exe().is_some_and(|root| Path::new(&launched.cmd).starts_with(root)) {
                live_apply::stop_process(launched.pid);
            }
            // GUI·서비스 관리자가 띄운 새 데몬은 PID를 모를 수 있다: 데몬 잠금을 쥔 것까지 끝낸 뒤에만 되돌린다
            let data_dir = constants::resolve_data_dir();
            if !live_apply::stop_new_daemons(pid, old_pid, || live_apply::daemon_lock_holder(&data_dir), live_apply::stop_process) {
                tracing::error!("[Apply] A new daemon still holds the daemon lock; leaving {} in place instead of rolling back", live.target_version);
                live.skip();
                return false;
            }
            reason
        }
    };
    let config = saba_chan_updater_lib::config_file::ConfigFile::default_location();
    // 릴레이는 되돌림을 릴리즈마다 센다 (부분 릴리즈의 코어 판은 릴리즈 판과 다르다)
    let version = live.report_version().to_string();
    live.roll_back(manager, &config, &UpdateCompletionMarker::path(), reason);
    let saved = live_apply::saved_update_config(&config);
    live_apply::report_rolled_back(saved.relay_url.as_deref(), &version, &saved.channel).await;
    let (exe, extra) = rollback_relaunch(target.exe, target.extra);
    if relaunch_now(exe, extra, target.gui, false).await.is_none() {
        tracing::error!("[Apply] The previous version could not be relaunched after the rollback");
    }
    false
}

/// 다시 띄우는 것에 `--after-update`를 붙이는가: 설치 폴더의 사바쨩 프로그램만. 서비스 관리자(`systemctl`) 같은
/// 바깥 프로그램은 그 인자를 모른다. 설치 폴더를 모르면 예전처럼 붙인다
fn marks_update(cmd: &str, install_root: Option<&Path>) -> bool {
    install_root.is_none_or(|root| Path::new(cmd).starts_with(root))
}

#[cfg(feature = "window")]
/// GUI 설정(`config.json`의 `gui`)에서 키 값 읽기
fn load_setting(key: &str) -> Option<String> {
    saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// install_root를 exe 자신의 경로에서 추론
fn resolve_install_root_from_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

#[cfg(feature = "window")]
/// GUI exe 경로 추론 (install_root 바로 안)
fn resolve_gui_exe() -> Option<PathBuf> {
    saba_chan_updater_lib::find_gui_exe(&resolve_install_root_from_exe()?)
}

fn resolve_modules_dir() -> String {
    let p = constants::resolve_modules_dir();
    if !p.exists() { let _ = std::fs::create_dir_all(&p); }
    p.to_string_lossy().to_string()
}

/// 다시 띄울 실행 파일: `--relaunch`로 받은 것, 없으면 설치된 GUI.
/// 받은 것이 GUI인데 업데이트로 비켜났으면(0.1.1의 `Saba-chan.exe` → `saba-chan-gui.exe`) 설치된 GUI
fn relaunch_command(requested: Option<String>, installed_gui: Option<PathBuf>) -> Option<String> {
    let is_gui = |exe: &str| {
        Path::new(exe).file_name().is_some_and(|n| {
            saba_chan_updater_lib::GUI_EXE_NAMES.iter().any(|g| n.eq_ignore_ascii_case(g))
        })
    };
    match (requested, installed_gui) {
        (Some(cmd), Some(gui)) if is_gui(&cmd) && !Path::new(&cmd).exists() => {
            tracing::info!("[Apply] {} was replaced; relaunching {}", cmd, gui.display());
            Some(gui.to_string_lossy().into_owned())
        }
        (Some(cmd), _) => Some(cmd),
        (None, gui) => gui.map(|g| g.to_string_lossy().into_owned()),
    }
}

/// GUI exe가 쓰기 완료될 때까지 대기 (최대 10초)
async fn wait_for_exe(cmd: &str) {
    let path = std::path::Path::new(cmd);
    for attempt in 0..20 {
        if path.exists() && std::fs::File::open(path).is_ok() {
            break;
        }
        tracing::debug!("[Apply] Waiting for exe: {} (attempt {})", cmd, attempt + 1);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// 프로세스 재실행 (최대 3회 재시도). 띄운 프로세스의 PID
async fn launch_with_retry(cmd: &str, extra_args: &[String], after_update: bool) -> Option<u32> {
    for attempt in 0..3 {
        match spawn_detached(cmd, extra_args, after_update) {
            Ok(pid) => {
                tracing::info!("[Apply] Relaunch succeeded (attempt {}, PID {})", attempt + 1, pid);
                return Some(pid);
            }
            Err(e) => {
                tracing::warn!("[Apply] Relaunch attempt {} failed: {}", attempt + 1, e);
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }
    }
    tracing::error!("[Apply] All relaunch attempts failed for: {}", cmd);
    None
}

/// 완전히 분리된 자식 프로세스 생성. 그 PID를 돌려준다
fn spawn_detached(cmd: &str, extra_args: &[String], after_update: bool) -> Result<u32, String> {
    tracing::info!("[Apply] Spawning: {} {:?}", cmd, extra_args);

    let mut command = std::process::Command::new(cmd);
    for arg in extra_args {
        command.arg(arg);
    }
    if after_update {
        command.arg("--after-update");
    }

    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // 업데이터를 띄운 쪽의 Job Object에서 빠져나온다. 탈출을 허용하지 않는 Job이면 (os error 5) 그냥 띄운다
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Err(e) if e.raw_os_error() == Some(5) => {
                tracing::warn!("[Apply] Job breakaway denied; spawning inside the job");
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                command.spawn().map(|c| c.id()).map_err(|e| format!("Failed to spawn: {}", e))
            }
            other => other.map(|c| c.id()).map_err(|e| format!("Failed to spawn: {}", e)),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // 손잡이를 맡겨 둔다: 끝난 자식은 거두기 전까지 좀비로 남아 감시가 살아 있다고 볼 수 있다
        command.spawn().map(live_apply::keep_child).map_err(|e| format!("Failed to spawn: {}", e))
    }
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

/// `--apply ...`로 띄운 실행. 창 없이 적용했으면 그 종료 코드로 끝난다
pub(crate) fn run(args: &[String]) {

    tracing_subscriber::fmt()
        .with_writer({
            // 파일 로거: %TEMP%\saba-updater.log — 프로세스가 GUI 서브시스템이므로 stderr 대신 파일 사용
            let log_path = std::env::temp_dir().join("saba-updater.log");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&log_path)
                .expect("Failed to open log file");
            std::sync::Mutex::new(file)
        })
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_ansi(false)
        .init();

    // 인자 파싱: --apply [--relaunch <exe> [extra...]]
    let apply_pos = args.iter().position(|a| a == "--apply").unwrap_or(0);
    let after_apply = &args[apply_pos + 1..];

    let relaunch_pos = after_apply.iter().position(|a| a == "--relaunch");
    let (relaunch_exe, relaunch_extra) = match relaunch_pos {
        Some(pos) => {
            let rest = &after_apply[pos + 1..];
            if rest.is_empty() {
                (None, Vec::new())
            } else {
                (Some(rest[0].clone()), rest[1..].to_vec())
            }
        }
        None => (None, Vec::new()),
    };

    tracing::info!("[Apply] Relaunch: {:?} {:?}", relaunch_exe, relaunch_extra);

    // 데몬이 끊어 준 실행 표가 있어야 적용한다 (한 번만 쓴다).
    // 표를 모르는 옛 데몬이 띄웠으면(표 인자 자체가 없음) 전환 경로로 받아 준다
    use saba_chan_updater_lib::launch_ticket;
    let token = after_apply
        .iter()
        .position(|a| a == "--launch-token")
        .filter(|&i| relaunch_pos.is_none_or(|r| i < r))
        .and_then(|i| after_apply.get(i + 1))
        .cloned();
    let accepted = match token {
        Some(t) => launch_ticket::redeem(&saba_chan_updater_lib::constants::resolve_staging_dir(), &t),
        None => {
            launch_ticket::installed_core_predates_tickets()
                && launch_ticket::claim_legacy_apply(&saba_chan_updater_lib::constants::resolve_staging_dir())
        }
    };
    if !accepted {
        tracing::error!("[Apply] Missing or stale launch token; refusing to apply");
        eprintln!("이 업데이터는 사바쨩이 업데이트를 적용할 때만 실행됩니다.");
        std::process::exit(1);
    }

    // --relaunch 앞의 인자만 업데이터의 것이다
    // 창 없이 빌드한 업데이터(`window` 기능 없음)는 늘 창 없이 적용한다
    let headless = cfg!(not(feature = "window"))
        || after_apply.iter().take(relaunch_pos.unwrap_or(after_apply.len())).any(|a| a == "--headless");

    // 띄운 쪽(데몬) 종료 대기. PID를 모르면 예전처럼 잠깐만 기다린다.
    // 30초 뒤에도 살아 있으면 파일을 바꾸지 않는다: 받아 둔 것은 그대로 두고, 예전 데몬이 마저 끝나면 다시 띄울 것만 띄운다
    // (예전 데몬이 아직 있을 때 띄우면 새로 띄운 데몬은 단일 인스턴스 잠금에 막혀 끝나고, 그 뒤 예전 데몬도 끝난다)
    let wait_pid = after_apply
        .iter()
        .position(|a| a == "--wait-pid")
        .filter(|&i| relaunch_pos.is_none_or(|r| i < r))
        .and_then(|i| after_apply.get(i + 1))
        .and_then(|p| p.parse::<u32>().ok());
    if !live_apply::wait_for_previous(wait_pid, live_apply::WAIT_PID_TIMEOUT, saba_chan_updater_lib::ProcessChecker::is_pid_running, &UpdateCompletionMarker::path()) {
        std::process::exit(relaunch_unchanged(headless, relaunch_exe, relaunch_extra, wait_pid));
    }

    // install_root: exe 위치에서 자동 추론
    let mut cfg = saba_chan_updater_lib::UpdateConfig::default();
    if let Some(root) = resolve_install_root_from_exe() {
        let root_str = root.to_string_lossy().to_string();
        tracing::info!("[Apply] Install root (from exe): {}", root_str);
        cfg.install_root = Some(root_str);
    }
    let modules_dir = resolve_modules_dir();

    if headless {
        std::process::exit(run_headless(UpdateManager::new(cfg, &modules_dir), relaunch_exe, relaunch_extra, wait_pid));
    }

    #[cfg(feature = "window")]
    {
        let apply_config = ApplyConfig { relaunch_exe, relaunch_extra, wait_pid };

        let manager: ManagerState = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));

        tauri::Builder::default()
            .plugin(tauri_plugin_shell::init())
            .manage(manager)
            .manage(apply_config)
            .setup(|app| {
                let win = crate::main_window(app.handle(), crate::Page::Update, update_title(load_setting("language").as_deref()))?;
                // JS의 appWindow.show()가 블로킹되는 문제를 회피:
                // Rust setup에서 직접 윈도우를 표시하고 포커스 강제 획득
                win.show().ok();
                win.set_always_on_top(true).ok();
                win.set_focus().ok();
                win.request_user_attention(Some(tauri::UserAttentionType::Critical)).ok();
                // always-on-top 해제는 별도 스레드에서 딜레이 후 수행
                let win_clone = win.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    win_clone.set_always_on_top(false).ok();
                });
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                get_apply_mode,
                start_apply,
                crate::get_preferred_language,
                get_theme,
                crate::backdrop::get_window_backdrop,
            ])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}


#[cfg(feature = "window")]
/// 사용자 언어에 맞는 업데이트 창 제목
fn update_title(language: Option<&str>) -> &'static str {
    match language {
        Some("ko") => "사바쨩 — 업데이트중!",
        Some("ja") => "Saba-chan — 更新中...",
        Some("zh-CN") | Some("zh-TW") => "Saba-chan — 更新中...",
        Some("es") => "Saba-chan — Actualizando...",
        Some("pt-BR") => "Saba-chan — Atualizando...",
        Some("ru") => "Saba-chan — Обновление...",
        Some("de") => "Saba-chan — Aktualisierung...",
        Some("fr") => "Saba-chan — Mise à jour...",
        _ => "Saba-chan — Updating...",
    }
}

/// 적용이 끝난 뒤의 정리: 옛 업데이터를 치우고, 명령 실행기(`bin\saba-chan.exe`)가 생겼으면 그 폴더를 사용자 PATH에 넣는다
/// (실행기가 없던 판에서 업데이트한 설치도 `saba-chan`으로 부를 수 있게)
fn after_apply() {
    saba_chan_updater_lib::retire_legacy_updater();
    if let Some(root) = resolve_install_root_from_exe() {
        if let Err(e) = saba_chan_installer_core::user_path::add(&root) {
            tracing::warn!("[Apply] Could not add the saba-chan command to PATH: {}", e);
        }
    }
}

/// 파일을 바꾸지 않고 끝낼 때: 다시 띄울 것만 띄운다 (창 업데이터는 설치된 GUI도 찾는다). 종료 코드 1
/// 예전 데몬이 늦게라도 끝나야 다시 띄운다 (끝나지 않으면 그 데몬이 계속 돈다, `live_apply::may_relaunch_unchanged`)
fn relaunch_unchanged(headless: bool, relaunch_exe: Option<String>, relaunch_extra: Vec<String>, wait_pid: Option<u32>) -> i32 {
    if !live_apply::may_relaunch_unchanged(wait_pid, live_apply::LATE_EXIT_TIMEOUT, saba_chan_updater_lib::ProcessChecker::is_pid_running) {
        return 1;
    }
    #[cfg(feature = "window")]
    let gui = if headless { None } else { resolve_gui_exe() };
    #[cfg(not(feature = "window"))]
    let gui = {
        let _ = headless;
        None
    };
    match tokio::runtime::Runtime::new() {
        Ok(rt) => {
            rt.block_on(relaunch_now(relaunch_exe, relaunch_extra, gui, false));
        }
        Err(e) => tracing::error!("[Apply] Cannot start the runtime: {}", e),
    }
    1
}

/// 창 없이 적용한다 (Tauri를 시작하지 않는다). 결과는 창 업데이터와 같은 완료 마커에 남는다
fn run_headless(mut manager: UpdateManager, relaunch_exe: Option<String>, relaunch_extra: Vec<String>, wait_pid: Option<u32>) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("[Apply] Cannot start the runtime: {}", e);
            UpdateCompletionMarker::failure(format!("Cannot start the updater: {}", e)).save().ok();
            return 1;
        }
    };
    rt.block_on(async {
        let live = prepare_live_apply(&mut manager);
        let result = manager.apply_staged(|p| tracing::info!("[Apply] {} {}% {}", p.step, p.percent, p.message)).await;
        let applied = matches!(&result, Ok((_, failed)) if failed.is_empty());
        let core_applied = core_was_applied(&result);
        if result.is_ok() {
            after_apply();
        }
        // 다시 띄울 것은 데몬이 넘긴 것뿐이다 (헤드리스에는 GUI가 없다)
        if relaunch_exe.is_none() {
            tracing::warn!("[Apply] --headless without --relaunch: nothing is restarted");
        }
        // 데몬을 다시 띄우지 못하면 적용이 성공해도 실패로 끝낸다 (데몬이 내려간 채 남는다)
        let since_ms = live_apply::now_ms();
        let launched = match relaunch_exe {
            Some(_) => relaunch_now(relaunch_exe.clone(), relaunch_extra.clone(), None, applied).await,
            None => None,
        };
        let relaunched = relaunch_exe.is_none() || launched.is_some();
        let target = Relaunch { exe: relaunch_exe, extra: relaunch_extra, gui: None };
        let healthy = watch_new_daemon(live, &manager, core_applied, launched, wait_pid, since_ms, target).await;
        if applied && relaunched && healthy { 0 } else { 1 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // M3: 받은 GUI 경로(0.1.1의 `Saba-chan.exe`)가 업데이트로 비켜났으면 지금 설치된 GUI를 띄운다
    #[test]
    fn a_retired_gui_is_relaunched_as_the_installed_one() {
        let dir = std::env::temp_dir().join(format!("saba-relaunch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let installed = dir.join(saba_chan_updater_lib::GUI_EXE_NAMES[0]);
        std::fs::write(&installed, b"gui").unwrap();
        // Windows는 `Saba-chan.exe`. 다른 OS는 GUI 이름이 하나라 없는 폴더의 같은 이름으로 대신한다
        let old = dir.join("gone").join(saba_chan_updater_lib::GUI_EXE_NAMES.last().unwrap());

        let cmd = relaunch_command(Some(old.to_string_lossy().into_owned()), Some(installed.clone()));
        assert_eq!(cmd.as_deref(), Some(installed.to_string_lossy().as_ref()));
        // 받은 경로가 있으면 그대로, GUI가 아닌 프로그램도 그대로
        let cmd = relaunch_command(Some(installed.to_string_lossy().into_owned()), None);
        assert_eq!(cmd.as_deref(), Some(installed.to_string_lossy().as_ref()));
        let cli = dir.join("saba-chan-cli.exe").to_string_lossy().into_owned();
        assert_eq!(relaunch_command(Some(cli.clone()), Some(installed.clone())), Some(cli));
        assert_eq!(relaunch_command(None, Some(installed.clone())).as_deref(), Some(installed.to_string_lossy().as_ref()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ★ 13절: 데몬을 직접 띄웠으면 그 PID로만, GUI·서비스 관리자가 띄우면 판과 기동 시각으로 새 데몬을 가린다
    #[test]
    fn the_new_daemon_is_told_apart_by_how_it_was_relaunched() {
        let launched = |cmd: &str| Launched { cmd: cmd.into(), pid: 4321 };
        assert_eq!(health_rule(&launched("/opt/saba/saba-core"), Some(10), 99), live_apply::HealthRule::SpawnedPid(4321));
        assert_eq!(health_rule(&launched(r"C:\Saba-chan\saba-core.exe"), Some(10), 99), live_apply::HealthRule::SpawnedPid(4321));
        for other in ["/opt/saba/saba-chan-gui", "/usr/bin/systemctl", "/bin/launchctl"] {
            assert_eq!(
                health_rule(&launched(other), Some(10), 99),
                live_apply::HealthRule::StartedAfter { since_ms: 99, old_pid: Some(10) },
                "{other}"
            );
        }
    }

    /// 되돌린 뒤 서비스는 다시 시작한다 (관리자가 새 판을 이미 다시 띄웠을 수 있다). 그 밖의 명령은 그대로
    #[test]
    fn a_service_is_restarted_after_a_rollback() {
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let (exe, extra) = rollback_relaunch(Some("/usr/bin/systemctl".into()), s(&["--user", "start", "saba-chan.service"]));
        assert_eq!((exe.as_deref(), extra), (Some("/usr/bin/systemctl"), s(&["--user", "restart", "saba-chan.service"])));
        let (_, extra) = rollback_relaunch(Some("/bin/launchctl".into()), s(&["kickstart", "gui/501/online.saba-chan.core"]));
        assert_eq!(extra, s(&["kickstart", "-k", "gui/501/online.saba-chan.core"]));
        let (_, extra) = rollback_relaunch(Some("/opt/saba/saba-core".into()), s(&["--spawned"]));
        assert_eq!(extra, s(&["--spawned"]));
    }

    /// `--after-update`는 사바쨩 프로그램에만 붙인다 (서비스로 돈 데몬은 `systemctl --user start`로 다시 띄운다)
    #[test]
    fn only_saba_chan_programs_are_told_they_follow_an_update() {
        let root = Path::new("/opt/saba");
        assert!(marks_update("/opt/saba/saba-core", Some(root)));
        assert!(!marks_update("/usr/bin/systemctl", Some(root)));
        assert!(marks_update("/usr/bin/systemctl", None), "an unknown install folder keeps the old behaviour");
    }
}
