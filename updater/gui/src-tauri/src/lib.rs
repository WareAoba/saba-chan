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
    spawned_by: String,
}

#[derive(Debug, Clone)]
struct LaunchArgs {
    relaunch_exe: Option<String>,
    relaunch_extra: Vec<String>,
    spawned_by: String,
    launch_token: String,
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
        "spawned_by": config.spawned_by,
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
    // 데몬이 `save_updater_apply_targets(&needs_updater)`로 기록한 정확한 목록을 로드.
    // 파일이 없거나 읽기 실패 시 **전체 pending 적용으로 폴백하지 않는다** — 오동작의 근원.
    // 데몬과의 계약을 지키지 못한 상태이므로 즉시 오류 반환.
    let target_keys: Vec<String> = match mgr.load_updater_apply_targets() {
        Ok(keys) if !keys.is_empty() => keys,
        Ok(_) => {
            let msg = "apply-targets.json is empty — daemon did not specify any components to apply".to_string();
            emit_progress(&app, "error", &msg, 0, &[]);
            return Err(msg);
        }
        Err(e) => {
            let msg = format!("Failed to load apply-targets.json (daemon may have failed to persist it): {}", e);
            emit_progress(&app, "error", &msg, 0, &[]);
            return Err(msg);
        }
    };

    tracing::info!("[Apply] Targets: {:?}", target_keys);

    // 3. 적용 — 개별 컴포넌트 순차 적용 (진행률 이벤트 발행)
    let mut applied = Vec::new();
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

fn parse_launch_args(args: &[String]) -> Result<LaunchArgs, String> {
    // --apply 필수
    if !args.iter().any(|a| a == "--apply") {
        return Err("missing --apply".to_string());
    }

    // --spawned-by <gui|cli|daemon> 필수
    let spawned_by = if let Some(pos) = args.iter().position(|a| a == "--spawned-by") {
        args.get(pos + 1)
            .cloned()
            .ok_or_else(|| "missing value for --spawned-by".to_string())?
    } else {
        return Err("missing --spawned-by".to_string());
    };

    if !matches!(spawned_by.as_str(), "gui" | "cli" | "daemon") {
        return Err(format!("invalid --spawned-by: {}", spawned_by));
    }

    // --launch-token <token> 필수 — 데몬이 발급한 단일 사용 티켓
    let launch_token = if let Some(pos) = args.iter().position(|a| a == "--launch-token") {
        args.get(pos + 1)
            .cloned()
            .ok_or_else(|| "missing value for --launch-token".to_string())?
    } else {
        return Err("missing --launch-token".to_string());
    };

    if launch_token.trim().is_empty() {
        return Err("empty --launch-token".to_string());
    }

    // 인자 파싱: --apply [--relaunch <exe> [extra...]]
    let apply_pos = args
        .iter()
        .position(|a| a == "--apply")
        .ok_or_else(|| "missing --apply".to_string())?;
    let after_apply = &args[apply_pos + 1..];

    let relaunch_pos = after_apply.iter().position(|a| a == "--relaunch");
    let (relaunch_exe, relaunch_extra) = match relaunch_pos {
        Some(pos) => {
            let rest = &after_apply[pos + 1..];
            if rest.is_empty() {
                (None, Vec::new())
            } else {
                // --relaunch 이후에 다른 알려진 플래그(--spawned-by, --launch-token)가
                // 오면 extra_args로 포함되지 않도록 필터링한다.
                let exe = rest[0].clone();
                let tail = &rest[1..];
                let mut extra = Vec::new();
                let mut i = 0;
                while i < tail.len() {
                    match tail[i].as_str() {
                        "--spawned-by" | "--launch-token" => {
                            // 플래그 + 값 건너뛰기
                            i += 2;
                        }
                        _ => {
                            extra.push(tail[i].clone());
                            i += 1;
                        }
                    }
                }
                (Some(exe), extra)
            }
        }
        None => (None, Vec::new()),
    };

    Ok(LaunchArgs {
        relaunch_exe,
        relaunch_extra,
        spawned_by,
        launch_token,
    })
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

    let launch_args = match parse_launch_args(&args) {
        Ok(v) => v,
        Err(e) => {
        eprintln!("사바쨩 업데이터 — 업데이트 적용 전용");
        eprintln!();
            eprintln!("사용법: saba-chan-updater --apply --spawned-by <gui|cli|daemon> --launch-token <token> [--relaunch <exe> [extra...]]");
        eprintln!();
            eprintln!("이 프로그램은 메인 GUI/CLI/데몬에서 자동으로 실행됩니다.");
            eprintln!("직접 실행하지 마세요. ({})", e);
        std::process::exit(1);
        }
    };

    // 스폰 인증 티켓 검증 — 데몬이 발급하지 않은 실행은 거부
    if let Err(e) = saba_chan_updater_lib::launch_ticket::consume(
        &launch_args.launch_token,
        &launch_args.spawned_by,
    ) {
        eprintln!("사바쨩 업데이터 — 인증 실패");
        eprintln!();
        eprintln!("이 프로그램은 메인 GUI/CLI/데몬에서 자동으로 실행됩니다.");
        eprintln!("직접 실행하지 마세요. ({})", e);
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

    let relaunch_exe = launch_args.relaunch_exe;
    let relaunch_extra = launch_args.relaunch_extra;
    let spawned_by = launch_args.spawned_by;

    tracing::info!(
        "[Apply] Spawned by: {}, relaunch: {:?} {:?}",
        spawned_by,
        relaunch_exe,
        relaunch_extra
    );

    // GUI 프로세스 종료 대기
    std::thread::sleep(std::time::Duration::from_millis(500));

    let apply_config = ApplyConfig {
        relaunch_exe,
        relaunch_extra,
        spawned_by,
    };

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
    use super::parse_launch_args;

    #[test]
    fn parse_launch_args_requires_apply() {
        let args = vec!["saba-chan-updater".to_string()];
        let err = parse_launch_args(&args).unwrap_err();
        assert!(err.contains("missing --apply"));
    }

    #[test]
    fn parse_launch_args_requires_spawned_by() {
        let args = vec!["saba-chan-updater".to_string(), "--apply".to_string()];
        let err = parse_launch_args(&args).unwrap_err();
        assert!(err.contains("missing --spawned-by"));
    }

    #[test]
    fn parse_launch_args_rejects_invalid_spawned_by() {
        let args = vec![
            "saba-chan-updater".to_string(),
            "--apply".to_string(),
            "--spawned-by".to_string(),
            "manual".to_string(),
            "--launch-token".to_string(),
            "tok".to_string(),
        ];
        let err = parse_launch_args(&args).unwrap_err();
        assert!(err.contains("invalid --spawned-by"));
    }

    #[test]
    fn parse_launch_args_requires_launch_token() {
        let args = vec![
            "saba-chan-updater".to_string(),
            "--apply".to_string(),
            "--spawned-by".to_string(),
            "gui".to_string(),
        ];
        let err = parse_launch_args(&args).unwrap_err();
        assert!(err.contains("missing --launch-token"));
    }

    #[test]
    fn parse_launch_args_rejects_empty_launch_token() {
        let args = vec![
            "saba-chan-updater".to_string(),
            "--apply".to_string(),
            "--spawned-by".to_string(),
            "gui".to_string(),
            "--launch-token".to_string(),
            "".to_string(),
        ];
        let err = parse_launch_args(&args).unwrap_err();
        assert!(err.contains("empty --launch-token"));
    }

    #[test]
    fn parse_launch_args_parses_relaunch_and_extra() {
        let args = vec![
            "saba-chan-updater".to_string(),
            "--apply".to_string(),
            "--spawned-by".to_string(),
            "gui".to_string(),
            "--launch-token".to_string(),
            "abc-123".to_string(),
            "--relaunch".to_string(),
            "C:/Saba/saba-chan-gui.exe".to_string(),
            "C:/Saba".to_string(),
        ];

        let parsed = parse_launch_args(&args).expect("parse should succeed");
        assert_eq!(parsed.spawned_by, "gui");
        assert_eq!(parsed.launch_token, "abc-123");
        assert_eq!(parsed.relaunch_exe.as_deref(), Some("C:/Saba/saba-chan-gui.exe"));
        assert_eq!(parsed.relaunch_extra, vec!["C:/Saba".to_string()]);
    }

    #[test]
    fn parse_launch_args_filters_known_flags_from_relaunch_extra() {
        // --relaunch <exe> 뒤에 --spawned-by/--launch-token이 와도 extra에 섞이지 않아야 한다.
        let args = vec![
            "saba-chan-updater".to_string(),
            "--apply".to_string(),
            "--relaunch".to_string(),
            "gui.exe".to_string(),
            "--spawned-by".to_string(),
            "gui".to_string(),
            "--launch-token".to_string(),
            "tok".to_string(),
        ];
        let parsed = parse_launch_args(&args).expect("parse should succeed");
        assert_eq!(parsed.relaunch_exe.as_deref(), Some("gui.exe"));
        assert!(parsed.relaunch_extra.is_empty());
    }
}
