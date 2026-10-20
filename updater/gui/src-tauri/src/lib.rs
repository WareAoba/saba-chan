//! Saba-chan Updater — 업데이트 적용 전용 Tauri 앱
//!
//! 데몬이 다운로드해둔 업데이트 파일을 적용하고 GUI를 재실행합니다.
//! CLI 모드, 독립 GUI 모드 없이 **apply 전용**으로 설계되었습니다.
//!
//! ## 실행
//! ```text
//! saba-chan-updater --apply [--wait-pid <pid>] --launch-token <t> [--relaunch <exe> [extra...]]
//! ```
//!
//! 보통 데몬이 띄운다(`POST /api/updates/launch-updater`, U7). `--wait-pid`는 데몬 PID로,
//! 데몬이 끝날 때까지(최대 30초) 기다린 뒤 적용한다. 결과는 성공·실패 모두
//! 완료 마커(`update-complete.json`)에 남고, 다음 데몬이 `/api/updates/last-result`로 보여준다.
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

use saba_chan_updater_lib::{UpdateManager, UpdateCompletionMarker};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use saba_chan_updater_lib::constants;

pub mod config;

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

type ManagerState = Arc<RwLock<UpdateManager>>;

/// 재실행 설정 (Tauri managed state)
#[derive(Debug, Clone, Default)]
struct ApplyConfig {
    relaunch_exe: Option<String>,
    relaunch_extra: Vec<String>,
}

/// Apply 진행 이벤트 페이로드
#[derive(Debug, Clone, Serialize)]
struct ApplyProgressEvent {
    step: String,
    message: String,
    percent: i32,
    applied: Vec<String>,
    /// 실패하거나 건너뛴 키 수. 화면 문구는 GUI가 이 수로 번역하고, message는 진단용 원문이다
    failed: usize,
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드
// ═══════════════════════════════════════════════════════

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

/// Apply 실행 — 매니페스트 로드 → 파일 적용 → 완료 마커 → 재실행
#[tauri::command]
async fn start_apply(
    app: AppHandle,
    apply_config: tauri::State<'_, ApplyConfig>,
    manager: tauri::State<'_, ManagerState>,
) -> Result<Vec<String>, String> {
    // 1. 매니페스트 로드
    emit_progress(&app, "manifest", "Loading manifest...", 10, &[]);

    let count = {
        let mut mgr = manager.write().await;
        mgr.load_pending_manifest()
            .map_err(|e| {
                let msg = format!("Failed to load manifest: {}", e);
                emit_progress_with_failures(&app, "error", &msg, 0, &[], 1);
                UpdateCompletionMarker::failure(msg.clone()).save().ok();
                relaunch(&app, &apply_config, false);
                msg
            })?
    };

    emit_progress(&app, "manifest", &format!("{} components ready", count), 25, &[]);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // 2. apply-targets.json에서 적용 대상 결정
    let mut mgr = manager.write().await;
    let mut target_keys: Vec<String> = mgr
        .load_updater_apply_targets()
        .unwrap_or_default();

    tracing::info!("[Apply] Targets: {:?}", target_keys);

    // 3. 적용
    // apply-targets.json이 없거나 비어있으면 받아 둔 것 전체를 적용 순서대로. 실패하면 남은 것은 건너뛴 것으로 남긴다
    if target_keys.is_empty() {
        target_keys = mgr.pending_keys_in_apply_order();
    }
    let total = target_keys.len();
    let (applied, failed) = mgr.apply_target_keys(&target_keys, |i, key, done| {
        let pct = 30 + ((i as i32) * 60 / std::cmp::max(total as i32, 1));
        emit_progress(&app, "applying", &format!("Applying {} ({}/{})...", key, i + 1, total), pct, done);
    }).await;

    // 4. 완료 마커 저장 — 성공·실패 모두 남겨 "됐는지 안 됐는지" 볼 수 있게 한다
    let marker = UpdateCompletionMarker {
        message: Some(if failed.is_empty() {
            format!("{} updates applied: {}", applied.len(), applied.join(", "))
        } else {
            format!("{} applied, {} failed", applied.len(), failed.len())
        }),
        success: failed.is_empty(),
        failed: failed.clone(),
        // 적용한 릴리즈의 버전·패치노트: 앱이 결과 알림을 누르면 보여 준다 (A5)
        release: mgr.pending_release(),
        ..UpdateCompletionMarker::success(applied.clone())
    };
    marker.save().ok();
    mgr.clear_pending_manifest();

    if failed.is_empty() {
        emit_progress(&app, "complete", &{
            if applied.is_empty() {
                "No updates to apply.".to_string()
            } else {
                format!("{} updates applied!", applied.len())
            }
        }, 100, &applied);
    } else {
        emit_progress_with_failures(&app, "error", &format!("{} applied, {} failed: {}", applied.len(), failed.len(), failed.join("; ")), 0, &applied, failed.len());
    }

    drop(mgr);

    // 5. GUI 재실행 → 업데이터 종료 (하나라도 실패했으면 완료 알림 없이 띄우고 오류 화면을 남긴다)
    relaunch(&app, &apply_config, failed.is_empty());

    Ok(applied)
}

/// 업데이트를 띄운 프로그램(없으면 GUI)을 다시 띄운다. 적용이 실패해도 띄운다:
/// 업데이터가 뜰 때 데몬·GUI는 이미 끝났으니 그대로 두면 아무것도 남지 않는다.
/// `applied`면 `--after-update`를 붙이고(받는 쪽이 완료 알림을 띄운다) 띄운 뒤 업데이터를 닫는다.
/// 실패면 플래그 없이 띄우고 오류 화면을 남겨 둔다
fn relaunch(app: &AppHandle, apply_config: &ApplyConfig, applied: bool) {
    let relaunch_exe = apply_config.relaunch_exe.clone();
    let relaunch_extra = apply_config.relaunch_extra.clone();
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        let requested = relaunch_exe.is_some();
        if let Some(cmd) = relaunch_command(relaunch_exe, resolve_gui_exe()) {
            // 받은 인자는 받은 프로그램의 것이다. --relaunch 없이 찾은 GUI에는 넘기지 않는다
            let extra: &[String] = if requested { &relaunch_extra } else { &[] };
            wait_for_exe(&cmd).await;
            launch_with_retry(&cmd, extra, applied).await;
        }

        if applied {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            app_handle.exit(0);
        }
    });
}

/// 언어 설정 조회 — settings.json → 시스템 로케일 → "en"
#[tauri::command]
async fn get_preferred_language() -> Result<String, String> {
    if let Some(lang) = load_setting("language") {
        if let Some(normalized) = normalize_tag(&lang) {
            return Ok(normalized);
        }
    }
    if let Some(locale) = sys_locale::get_locale() {
        if let Some(normalized) = normalize_tag(&locale) {
            return Ok(normalized);
        }
    }
    Ok("en".to_string())
}

/// 테마 조회 — settings.json → "auto"
/// CSS `data-theme` + `prefers-color-scheme` 미디어 쿼리로 자동 처리되므로
/// 대부분 "auto"가 반환됨 (향후 GUI가 settings.json에 theme 저장 시 자동 대응)
#[tauri::command]
async fn get_theme() -> Result<String, String> {
    Ok(load_setting("theme").unwrap_or_else(|| "auto".to_string()))
}

/// 업데이트 완료 마커 확인 (프론트엔드 호환용)
#[tauri::command]
async fn check_after_update() -> Result<serde_json::Value, String> {
    // 지우지 않는다: 다음 데몬이 /api/updates/last-result로 보여 주고, 클라이언트가 확인하면 확인했다고 적는다
    if let Some(marker) = UpdateCompletionMarker::load().filter(|m| !m.acknowledged) {
        Ok(serde_json::json!({
            "updated": marker.success,
            "components": marker.updated_components,
            "message": marker.message,
        }))
    } else {
        Ok(serde_json::json!({ "updated": false, "components": [], "message": null }))
    }
}

// ═══════════════════════════════════════════════════════
// 헬퍼
// ═══════════════════════════════════════════════════════

fn emit_progress(app: &AppHandle, step: &str, message: &str, percent: i32, applied: &[String]) {
    emit_progress_with_failures(app, step, message, percent, applied, 0);
}

fn emit_progress_with_failures(app: &AppHandle, step: &str, message: &str, percent: i32, applied: &[String], failed: usize) {
    app.emit("apply:progress", ApplyProgressEvent {
        step: step.into(),
        message: message.into(),
        percent,
        applied: applied.to_vec(),
        failed,
    }).ok();
}

/// GUI 설정(`config.json`의 `gui`)에서 키 값 읽기
fn load_setting(key: &str) -> Option<String> {
    saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn normalize_tag(input: &str) -> Option<String> {
    if input.trim().is_empty() { return None; }
    Some(constants::resolve_locale(input))
}

/// install_root를 exe 자신의 경로에서 추론
fn resolve_install_root_from_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

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

/// 프로세스 재실행 (최대 3회 재시도)
async fn launch_with_retry(cmd: &str, extra_args: &[String], after_update: bool) {
    for attempt in 0..3 {
        match spawn_detached(cmd, extra_args, after_update) {
            Ok(()) => {
                tracing::info!("[Apply] Relaunch succeeded (attempt {})", attempt + 1);
                return;
            }
            Err(e) => {
                tracing::warn!("[Apply] Relaunch attempt {} failed: {}", attempt + 1, e);
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }
    }
    tracing::error!("[Apply] All relaunch attempts failed for: {}", cmd);
}

/// 완전히 분리된 자식 프로세스 생성
fn spawn_detached(cmd: &str, extra_args: &[String], after_update: bool) -> Result<(), String> {
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
                command.spawn().map(|_| ()).map_err(|e| format!("Failed to spawn: {}", e))
            }
            other => other.map(|_| ()).map_err(|e| format!("Failed to spawn: {}", e)),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        command.spawn().map(|_| ()).map_err(|e| format!("Failed to spawn: {}", e))
    }
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    // --apply 필수 — 이 바이너리는 apply 전용
    if !args.iter().any(|a| a == "--apply") {
        eprintln!("사바쨩 업데이터 — 업데이트 적용 전용");
        eprintln!();
        eprintln!("사용법: saba-chan-updater --apply [--relaunch <exe> [extra...]]");
        eprintln!();
        eprintln!("이 프로그램은 메인 GUI에서 자동으로 실행됩니다.");
        eprintln!("직접 실행할 필요가 없습니다.");
        std::process::exit(1);
    }

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
    let apply_pos = args.iter().position(|a| a == "--apply").unwrap();
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

    // 띄운 쪽(데몬) 종료 대기. PID를 모르면 예전처럼 잠깐만 기다린다.
    let wait_pid = after_apply
        .iter()
        .position(|a| a == "--wait-pid")
        .and_then(|i| after_apply.get(i + 1))
        .and_then(|p| p.parse::<u32>().ok());
    match wait_pid {
        Some(pid) => {
            for _ in 0..60 {
                if !saba_chan_updater_lib::ProcessChecker::is_pid_running(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            tracing::info!("[Apply] Waited for PID {} to exit", pid);
        }
        None => std::thread::sleep(std::time::Duration::from_millis(500)),
    }

    let apply_config = ApplyConfig { relaunch_exe, relaunch_extra };

    // install_root: exe 위치에서 자동 추론
    let mut cfg = config::load_config_for_gui();
    if let Some(root) = resolve_install_root_from_exe() {
        let root_str = root.to_string_lossy().to_string();
        tracing::info!("[Apply] Install root (from exe): {}", root_str);
        cfg.install_root = Some(root_str);
    }

    let modules_dir = resolve_modules_dir();
    let manager: ManagerState = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(manager)
        .manage(apply_config)
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                // 사용자 언어에 맞는 타이틀
                let title = match load_setting("language").as_deref() {
                    Some("ko") => "사바쨩 — 업데이트중!",
                    Some("ja") => "Saba-chan — 更新中...",
                    Some("zh-CN") | Some("zh-TW") => "Saba-chan — 更新中...",
                    Some("es") => "Saba-chan — Actualizando...",
                    Some("pt-BR") => "Saba-chan — Atualizando...",
                    Some("ru") => "Saba-chan — Обновление...",
                    Some("de") => "Saba-chan — Aktualisierung...",
                    Some("fr") => "Saba-chan — Mise à jour...",
                    _ => "Saba-chan — Updating...",
                };
                win.set_title(title).ok();
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
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_apply_mode,
            start_apply,
            get_preferred_language,
            get_theme,
            check_after_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
}
