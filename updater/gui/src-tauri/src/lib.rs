//! Saba-chan Updater — 업데이트 적용 전용 Tauri 앱
//!
//! 데몬이 다운로드해둔 업데이트 파일을 적용하고 GUI를 재실행합니다.
//! CLI 모드, 독립 GUI 모드 없이 **apply 전용**으로 설계되었습니다.
//!
//! ## 실행
//! ```text
//! saba-chan-updater --apply [--relaunch <exe> [extra...]]
//! ```
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
use std::path::PathBuf;
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
                emit_progress(&app, "error", &msg, 0, &[]);
                msg
            })?
    };

    emit_progress(&app, "manifest", &format!("{} components ready", count), 25, &[]);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // 2. apply-targets.json에서 적용 대상 결정
    let mut mgr = manager.write().await;
    let target_keys: Vec<String> = mgr
        .load_updater_apply_targets()
        .unwrap_or_default();

    tracing::info!("[Apply] Targets: {:?}", target_keys);

    // 3. 적용
    let mut applied = Vec::new();

    if target_keys.is_empty() {
        // apply-targets.json이 없거나 비어있으면 전체 적용
        let total = mgr.get_pending_components().len();
        emit_progress(&app, "applying", &format!("Applying {} components...", total), 50, &[]);

        match mgr.apply_updates().await {
            Ok(a) => applied = a,
            Err(e) => {
                let msg = format!("Apply failed: {}", e);
                emit_progress(&app, "error", &msg, 0, &[]);
                return Err(msg);
            }
        }
    } else {
        // 개별 컴포넌트 순차 적용 (진행률 이벤트 발행)
        let total = target_keys.len();
        for (i, key) in target_keys.iter().enumerate() {
            let pct = 30 + ((i as i32) * 60 / std::cmp::max(total as i32, 1));
            emit_progress(&app, "applying",
                &format!("Applying {} ({}/{})...", key, i + 1, total), pct, &applied);

            match mgr.apply_single_component(
                &saba_chan_updater_lib::Component::from_manifest_key(key),
            ).await {
                Ok(result) if result.success => {
                    tracing::info!("[Apply] {} ✓", key);
                    applied.push(key.clone());
                }
                Ok(result) => {
                    tracing::warn!("[Apply] {} failed: {}", key, result.message);
                }
                Err(e) => {
                    tracing::error!("[Apply] {} error: {}", key, e);
                }
            }
        }
    }

    // 4. 완료 마커 저장
    if !applied.is_empty() {
        let marker = UpdateCompletionMarker::success(applied.clone());
        let marker = UpdateCompletionMarker {
            message: Some(format!("{} updates applied: {}", applied.len(), applied.join(", "))),
            ..marker
        };
        marker.save().ok();
    }
    mgr.clear_pending_manifest();

    emit_progress(&app, "complete", &{
        if applied.is_empty() {
            "No updates to apply.".to_string()
        } else {
            format!("{} updates applied!", applied.len())
        }
    }, 100, &applied);

    drop(mgr);

    // 5. GUI 재실행 → 업데이터 종료
    let relaunch_exe = apply_config.relaunch_exe.clone();
    let relaunch_extra = apply_config.relaunch_extra.clone();
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        if let Some(ref cmd) = relaunch_exe {
            wait_for_exe(cmd).await;
            launch_with_retry(cmd, &relaunch_extra).await;
        } else {
            // --relaunch 미지정 시 GUI exe 자동 추론
            if let Some(gui_path) = resolve_gui_exe() {
                let cmd = gui_path.to_string_lossy().to_string();
                wait_for_exe(&cmd).await;
                launch_with_retry(&cmd, &[]).await;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app_handle.exit(0);
    });

    Ok(applied)
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
    if let Some(marker) = UpdateCompletionMarker::load() {
        UpdateCompletionMarker::clear().ok();
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
    app.emit("apply:progress", ApplyProgressEvent {
        step: step.into(),
        message: message.into(),
        percent,
        applied: applied.to_vec(),
    }).ok();
}

/// settings.json에서 키 값 읽기
fn load_setting(key: &str) -> Option<String> {
    let path = constants::resolve_settings_path();
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn normalize_tag(input: &str) -> Option<String> {
    if input.trim().is_empty() { return None; }
    Some(constants::resolve_locale(input))
}

/// install_root를 exe 자신의 경로에서 추론
fn resolve_install_root_from_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// GUI exe 경로 추론 (install_root/saba-chan-gui.exe)
fn resolve_gui_exe() -> Option<PathBuf> {
    let root = resolve_install_root_from_exe()?;
    let name = if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" };
    let path = root.join(name);
    if path.exists() { Some(path) } else { None }
}

fn resolve_modules_dir() -> String {
    let p = constants::resolve_modules_dir();
    if !p.exists() { let _ = std::fs::create_dir_all(&p); }
    p.to_string_lossy().to_string()
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
async fn launch_with_retry(cmd: &str, extra_args: &[String]) {
    for attempt in 0..3 {
        match spawn_detached(cmd, extra_args) {
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
fn spawn_detached(cmd: &str, extra_args: &[String]) -> Result<(), String> {
    tracing::info!("[Apply] Spawning: {} {:?}", cmd, extra_args);

    let mut command = std::process::Command::new(cmd);
    for arg in extra_args {
        command.arg(arg);
    }
    command.arg("--after-update");

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x00000008 | 0x01000000 | 0x00000200);
    }

    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn: {}", e))?;

    Ok(())
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

    // GUI 프로세스 종료 대기
    std::thread::sleep(std::time::Duration::from_millis(500));

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
