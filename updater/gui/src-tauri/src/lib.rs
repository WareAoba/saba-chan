//! Saba-chan Updater — 통합 업데이터 (GUI + CLI)
//!
//! saba-chan 코어 라이브러리의 UpdateManager를 직접 사용합니다.
//! 데몬 IPC 없이 독립 실행 가능합니다.
//!
//! ## 실행 모드
//! - `saba-chan-updater`              → GUI 모드 (Tauri 윈도우)
//! - `saba-chan-updater --cli <cmd>`   → CLI 모드 (터미널 출력)
//! - `saba-chan-updater --test ...`    → 테스트 모드 (E2E 셀프업데이트)
//!
//! ## v2 아키텍처
//! - 백그라운드 워커: 버전 체크, 다운로드를 비동기로 처리
//! - 포그라운드 적용: GUI/CLI 종료 후 파일 수정
//! - 이벤트 시스템: GUI에 실시간 상태 전달

#[allow(unused_imports)]
use saba_chan_updater_lib::{
    Component, UpdateManager,
    BackgroundWorker, BackgroundTask, WorkerEvent, WorkerStatus,
    DownloadQueue, DownloadRequest, QueueStatus,
    ApplyPhase, ApplyProgress, ApplyPreparation,
    UpdateCompletionMarker,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
#[allow(unused_imports)]
use tauri::{State, AppHandle, Emitter, Manager};
use tokio::sync::RwLock;

pub mod cli;
pub mod config;

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

type ManagerState = Arc<RwLock<UpdateManager>>;

/// 테스트 모드 런치 정보
#[derive(Debug, Clone, Default)]
pub struct TestModeConfig {
    pub enabled: bool,
    pub scenario: Option<String>,
    pub relaunch_cmd: Option<String>,
    pub relaunch_args: Vec<String>,
}

/// --apply 모드 설정 (Tauri managed state)
#[derive(Debug, Clone, Default)]
pub struct ApplyModeConfig {
    pub enabled: bool,
    pub component_args: Vec<String>,
    pub relaunch_cmd: Option<String>,
    pub relaunch_extra: Vec<String>,
}

/// Apply 진행 이벤트 페이로드
#[derive(Debug, Clone, Serialize)]
struct ApplyProgressEvent {
    step: String,
    message: String,
    percent: i32,
    applied: Vec<String>,
}

/// 프론트엔드에 전달하는 컴포넌트 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub key: String,
    pub display_name: String,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub downloaded: bool,
    pub installed: bool,
}

/// 프론트엔드에 전달하는 전체 상태
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdaterState {
    pub checking: bool,
    pub last_check: Option<String>,
    pub components: Vec<ComponentInfo>,
    pub error: Option<String>,
    /// 백그라운드 워커 상태 (v2)
    pub worker_busy: bool,
    pub worker_task: Option<String>,
}

/// 설치 진행 상태
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressInfo {
    pub complete: bool,
    pub current_component: Option<String>,
    pub total: usize,
    pub done: usize,
    pub installed_components: Vec<String>,
    pub errors: Vec<String>,
}

/// 백그라운드 다운로드 큐 상태
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueInfo {
    pub pending: usize,
    pub completed: usize,
    pub failed: usize,
    pub current: Option<String>,
    pub paused: bool,
}

/// 업데이트 완료 후 GUI 재시작 시 표시할 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterUpdateInfo {
    pub updated: bool,
    pub components: Vec<String>,
    pub message: Option<String>,
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn check_updates(manager: State<'_, ManagerState>) -> Result<UpdaterState, String> {
    let mut mgr = manager.write().await;
    mgr.check_for_updates().await.map_err(|e| e.to_string())?;
    Ok(build_state(&mgr))
}

#[tauri::command]
async fn get_status(manager: State<'_, ManagerState>) -> Result<UpdaterState, String> {
    let mgr = manager.read().await;
    Ok(build_state(&mgr))
}

#[tauri::command]
async fn download_all(manager: State<'_, ManagerState>) -> Result<Vec<String>, String> {
    let mut mgr = manager.write().await;
    mgr.download_available_updates()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_component(
    manager: State<'_, ManagerState>,
    key: String,
) -> Result<String, String> {
    let component = Component::from_manifest_key(&key);
    let mut mgr = manager.write().await;
    mgr.download_component(&component)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn apply_updates(manager: State<'_, ManagerState>) -> Result<Vec<String>, String> {
    let mut mgr = manager.write().await;
    mgr.apply_updates().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn full_install(manager: State<'_, ManagerState>) -> Result<ProgressInfo, String> {
    let mut mgr = manager.write().await;
    let progress = mgr.fresh_install(None).await.map_err(|e| e.to_string())?;
    Ok(ProgressInfo {
        complete: progress.complete,
        current_component: progress.current_component,
        total: progress.total,
        done: progress.done,
        installed_components: progress.installed_components,
        errors: progress.errors,
    })
}

#[tauri::command]
async fn install_component(
    manager: State<'_, ManagerState>,
    key: String,
) -> Result<String, String> {
    let component = Component::from_manifest_key(&key);
    let mut mgr = manager.write().await;
    mgr.install_component(&component)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_install_status(manager: State<'_, ManagerState>) -> Result<serde_json::Value, String> {
    let mgr = manager.read().await;
    let status = mgr.get_install_status();
    serde_json::to_value(status).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_install_progress(
    manager: State<'_, ManagerState>,
) -> Result<Option<ProgressInfo>, String> {
    let mgr = manager.read().await;
    Ok(mgr.get_install_progress().map(|p| ProgressInfo {
        complete: p.complete,
        current_component: p.current_component,
        total: p.total,
        done: p.done,
        installed_components: p.installed_components,
        errors: p.errors,
    }))
}

#[tauri::command]
async fn get_config(manager: State<'_, ManagerState>) -> Result<serde_json::Value, String> {
    let mgr = manager.read().await;
    serde_json::to_value(&mgr.config).map_err(|e| e.to_string())
}

/// Mock 서버 URL 설정 (런타임 오버라이드)
#[tauri::command]
async fn set_api_base_url(
    manager: State<'_, ManagerState>,
    url: Option<String>,
) -> Result<String, String> {
    let mut mgr = manager.write().await;
    let msg = match &url {
        Some(u) => {
            mgr.config.api_base_url = Some(u.clone());
            // owner가 비어있으면 mock 서버용 기본값 설정
            if mgr.config.github_owner.is_empty() {
                mgr.config.github_owner = "test-owner".to_string();
            }
            format!("API URL → {}", u)
        }
        None => {
            mgr.config.api_base_url = None;
            "API URL → GitHub (default)".to_string()
        }
    };
    tracing::info!("[Updater] {}", msg);
    Ok(msg)
}

/// 테스트 모드 여부 확인 — 프론트엔드가 시작 시 호출
#[tauri::command]
async fn get_test_mode(test_config: State<'_, TestModeConfig>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "enabled": test_config.enabled,
        "scenario": test_config.scenario,
        "relaunch_cmd": test_config.relaunch_cmd,
    }))
}

// run_scenario 제거됨 — 시나리오 테스트는 프론트엔드(updater.js)에서
// 개별 커맨드(check_updates → download_all → apply_updates)를 단계별로 호출합니다.

/// 업데이트 완료 후 saba-chan GUI 재실행 + 업데이터 종료
#[tauri::command]
async fn relaunch(
    test_config: State<'_, TestModeConfig>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(ref cmd) = test_config.relaunch_cmd {
        tracing::info!("[Test] Relaunching: {} {:?}", cmd, test_config.relaunch_args);

        let mut command = std::process::Command::new(cmd);
        for arg in &test_config.relaunch_args {
            command.arg(arg);
        }
        // --after-update 플래그 추가
        command.arg("--after-update");

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            // DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP
            // WebView2(Chromium)도 Job Object를 사용하므로 BREAKAWAY로 완전 분리
            command.creation_flags(0x00000008 | 0x01000000 | 0x00000200);
        }

        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to relaunch: {}", e))?;

        // 업데이터 종료
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        app_handle.exit(0);
    }
    Ok(())
}

/// 업데이트 완료 마커 확인 — GUI 시작 시 호출
#[tauri::command]
async fn check_after_update() -> Result<AfterUpdateInfo, String> {
    if let Some(marker) = UpdateCompletionMarker::load() {
        // 마커 삭제
        UpdateCompletionMarker::clear().ok();
        
        Ok(AfterUpdateInfo {
            updated: marker.success,
            components: marker.updated_components,
            message: marker.message,
        })
    } else {
        Ok(AfterUpdateInfo {
            updated: false,
            components: Vec::new(),
            message: None,
        })
    }
}

/// 적용 준비 상태 확인 — 적용 전 GUI에서 호출
#[tauri::command]
async fn get_apply_preparation(manager: State<'_, ManagerState>) -> Result<serde_json::Value, String> {
    let mgr = manager.read().await;
    let pending = mgr.get_pending_components();
    
    let prep = serde_json::json!({
        "components": pending.iter().map(|c| c.component.display_name()).collect::<Vec<_>>(),
        "requires_restart": pending.iter().any(|c| matches!(c.component, Component::Gui | Component::Cli)),
        "requires_daemon_restart": pending.iter().any(|c| matches!(c.component, Component::CoreDaemon)),
        "count": pending.len(),
    });
    
    Ok(prep)
}

/// 모듈만 적용 (프로세스 중단 불필요)
#[tauri::command]
async fn apply_modules_only(manager: State<'_, ManagerState>) -> Result<Vec<String>, String> {
    let mut mgr = manager.write().await;
    let mut applied = Vec::new();
    
    let modules: Vec<Component> = mgr
        .get_pending_components()
        .iter()
        .filter(|c| matches!(c.component, Component::Module(_)))
        .map(|c| c.component.clone())
        .collect();
    
    for module in modules {
        match mgr.apply_single_component(&module).await {
            Ok(result) if result.success => {
                applied.push(module.display_name());
            }
            Ok(result) => {
                tracing::warn!("[Apply] Module {} failed: {}", module.display_name(), result.message);
            }
            Err(e) => {
                tracing::error!("[Apply] Module {} error: {}", module.display_name(), e);
            }
        }
    }
    
    Ok(applied)
}

// ═══════════════════════════════════════════════════════
// 헬퍼
// ═══════════════════════════════════════════════════════

fn build_state(mgr: &UpdateManager) -> UpdaterState {
    let status = mgr.get_status();
    UpdaterState {
        checking: status.checking,
        last_check: status.last_check,
        error: status.error,
        components: status
            .components
            .iter()
            .map(|c| ComponentInfo {
                key: c.component.manifest_key(),
                display_name: c.component.display_name(),
                current_version: c.current_version.clone(),
                latest_version: c.latest_version.clone(),
                update_available: c.update_available,
                downloaded: c.downloaded,
                installed: c.installed,
            })
            .collect(),
        worker_busy: false, // TODO: 백그라운드 워커 연동
        worker_task: None,
    }
}

fn resolve_modules_dir() -> String {
    for candidate in &[
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("modules"))),
        Some(PathBuf::from("modules")),
    ] {
        if let Some(p) = candidate {
            if p.exists() {
                return p.to_string_lossy().to_string();
            }
        }
    }
    "modules".to_string()
}

// ═══════════════════════════════════════════════════════
// --apply 모드 (Tauri 윈도우로 진행 표시 + 파일 교체 + 재실행)
// ═══════════════════════════════════════════════════════

/// Apply 모드 정보 조회
#[tauri::command]
async fn get_apply_mode(config: State<'_, ApplyModeConfig>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "enabled": config.enabled,
        "components": config.component_args,
        "relaunch": config.relaunch_cmd.is_some(),
    }))
}

/// Apply 실행 — 매니페스트 로드 → 파일 적용 → 완료 마커 → 재실행
#[tauri::command]
async fn start_apply(
    app: AppHandle,
    apply_config: State<'_, ApplyModeConfig>,
    manager: State<'_, ManagerState>,
) -> Result<Vec<String>, String> {
    // 1. 매니페스트 로드
    app.emit("apply:progress", ApplyProgressEvent {
        step: "manifest".into(),
        message: "매니페스트 로딩 중...".into(),
        percent: 10,
        applied: vec![],
    }).ok();

    let count = {
        let mut mgr = manager.write().await;
        match mgr.load_pending_manifest() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("매니페스트 로드 실패: {}", e);
                app.emit("apply:progress", ApplyProgressEvent {
                    step: "error".into(), message: msg.clone(), percent: 0, applied: vec![],
                }).ok();
                return Err(msg);
            }
        }
    };

    app.emit("apply:progress", ApplyProgressEvent {
        step: "manifest".into(),
        message: format!("{}개 컴포넌트 준비 완료", count),
        percent: 25,
        applied: vec![],
    }).ok();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // 2. 파일 적용
    app.emit("apply:progress", ApplyProgressEvent {
        step: "applying".into(),
        message: "업데이트 파일 적용 중...".into(),
        percent: 50,
        applied: vec![],
    }).ok();

    let mut mgr = manager.write().await;
    // component_args가 있으면 해당 컴포넌트만 적용, 없으면 전체
    let target_keys: Vec<String> = apply_config.component_args.clone();
    let result = if target_keys.is_empty() {
        mgr.apply_updates().await
    } else {
        mgr.apply_components(&target_keys).await
    };
    match result {
        Ok(applied) => {
            // 완료 마커 저장
            if !applied.is_empty() {
                let marker = UpdateCompletionMarker::success(applied.clone());
                let marker = UpdateCompletionMarker {
                    message: Some(format!("{}개 업데이트 적용 완료: {}", applied.len(), applied.join(", "))),
                    ..marker
                };
                marker.save().ok();
            }
            mgr.clear_pending_manifest();

            app.emit("apply:progress", ApplyProgressEvent {
                step: "complete".into(),
                message: if applied.is_empty() {
                    "적용할 업데이트가 없습니다.".into()
                } else {
                    format!("{}개 업데이트 적용 완료!", applied.len())
                },
                percent: 100,
                applied: applied.clone(),
            }).ok();

            drop(mgr);

            // --relaunch가 있을 때만 GUI 재실행 (gui 업데이트 시)
            // 데몬/CLI만 적용 시에는 relaunch 없이 updater가 자동 종료
            let relaunch_cmd = apply_config.relaunch_cmd.clone();
            let relaunch_extra = apply_config.relaunch_extra.clone();
            let app_handle = app.clone();

            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                if let Some(ref cmd) = relaunch_cmd {
                    let extra_refs: Vec<&str> = relaunch_extra.iter().map(|s| s.as_str()).collect();
                    relaunch_process(cmd, &extra_refs);
                }
                // relaunch 여부와 무관하게 updater 프로세스 종료
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                app_handle.exit(0);
            });

            Ok(applied)
        }
        Err(e) => {
            let msg = format!("적용 실패: {}", e);
            app.emit("apply:progress", ApplyProgressEvent {
                step: "error".into(), message: msg.clone(), percent: 0, applied: vec![],
            }).ok();
            Err(msg)
        }
    }
}

/// --apply 모드 진입: 인자 파싱 → Tauri 윈도우로 진행 상황 표시
fn run_apply_mode(args: Vec<String>) {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // --apply 위치 찾기
    let apply_pos = args.iter().position(|a| a == "--apply").unwrap_or(0);
    let after_apply: Vec<String> = args[apply_pos + 1..].to_vec();

    // --install-root <path> 인자 추출
    let install_root_override: Option<String> = after_apply.iter().position(|a| a == "--install-root")
        .and_then(|pos| after_apply.get(pos + 1).cloned());

    // --install-root 와 그 값을 제거한 나머지 인자
    let filtered_args: Vec<String> = {
        let mut result = Vec::new();
        let mut skip_next = false;
        for arg in &after_apply {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "--install-root" {
                skip_next = true;
                continue;
            }
            result.push(arg.clone());
        }
        result
    };

    // --relaunch 위치 찾기
    let relaunch_pos = filtered_args.iter().position(|a| a == "--relaunch");

    // 컴포넌트 목록: --apply 뒤 ~ --relaunch 앞까지 (--install-root 제외)
    let component_args: Vec<String> = match relaunch_pos {
        Some(pos) => filtered_args[..pos].iter().map(|s| s.clone()).collect(),
        None => filtered_args.iter().map(|s| s.clone()).collect(),
    };

    // --relaunch 인자
    let (relaunch_cmd, relaunch_extra): (Option<String>, Vec<String>) = match relaunch_pos {
        Some(pos) => {
            let rest = &filtered_args[pos + 1..];
            if rest.is_empty() {
                (None, Vec::new())
            } else {
                (Some(rest[0].clone()), rest[1..].iter().map(|s| s.clone()).collect())
            }
        }
        None => (None, Vec::new()),
    };

    tracing::info!("[Apply] Install root override: {:?}", install_root_override);
    tracing::info!("[Apply] Components: {:?}", component_args);
    tracing::info!("[Apply] Relaunch: {:?} {:?}", relaunch_cmd, relaunch_extra);

    // GUI 프로세스가 종료될 때까지 잠시 대기
    std::thread::sleep(std::time::Duration::from_millis(500));

    let apply_config = ApplyModeConfig {
        enabled: true,
        component_args,
        relaunch_cmd,
        relaunch_extra,
    };

    // --install-root이 있으면 해당 경로에서 config도 로드 (portable 모드 대응)
    let cfg = if let Some(ref root) = install_root_override {
        let mut c = config::load_config_from_root(root);
        c.install_root = Some(root.clone());
        tracing::info!("[Apply] Config loaded from install_root: {}", root);
        c
    } else {
        config::load_config_for_gui()
    };
    let modules_dir = resolve_modules_dir();
    let manager: ManagerState = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(manager)
        .manage(apply_config)
        .manage(TestModeConfig::default())
        .setup(|app| {
            // apply 모드: 타이틀만 변경 (윈도우 크기는 GUI 모드와 동일)
            if let Some(win) = app.get_webview_window("main") {
                win.set_title("Saba-chan — Updating...").ok();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_apply_mode,
            start_apply,
            get_status,
            get_config,
            get_test_mode,
            check_after_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 프로세스 재실행 헬퍼
fn relaunch_process(cmd: &str, extra_args: &[&str]) {
    tracing::info!("[Apply] Relaunching: {} {:?}", cmd, extra_args);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut command = std::process::Command::new(cmd);
    for arg in extra_args {
        command.arg(arg);
    }
    // --after-update 플래그 추가
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
        .ok();
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    // --cli <command> → CLI 모드 (GUI 윈도우 없이 터미널 출력)
    if let Some(pos) = args.iter().position(|a| a == "--cli") {
        let cli_args: Vec<String> = args[pos + 1..].to_vec();
        cli::run_cli(cli_args);
        return;
    }

    // --silent → 사일런트 모드 (GUI 윈도우 없이 비-셀프 업데이트를 자동 처리)
    // --cli silent --json 과 동일하지만 독립 플래그로도 사용 가능
    if args.iter().any(|a| a == "--silent") {
        let json_flag = if args.iter().any(|a| a == "--json") {
            vec!["silent".to_string(), "--json".to_string()]
        } else {
            vec!["silent".to_string()]
        };
        cli::run_cli(json_flag);
        return;
    }

    // --apply [components...] [--relaunch <exe> [extra args...]]
    // 데몬이 다운로드해둔 파일을 교체하고, 선택적으로 GUI를 재실행합니다.
    // 예: saba-chan-updater --apply gui cli --relaunch "C:\path\gui.exe" --after-update
    if args.iter().any(|a| a == "--apply") {
        run_apply_mode(args);
        return;
    }

    // GUI 모드
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // CLI 인자 파싱: --test --scenario <name> --relaunch <command> [extra args...] --mock-url <url>
    let is_test = args.iter().any(|a| a == "--test");
    let scenario = args.iter().position(|a| a == "--scenario")
        .and_then(|i| args.get(i + 1).cloned());
    let relaunch_cmd = args.iter().position(|a| a == "--relaunch")
        .and_then(|i| args.get(i + 1).cloned());
    // --relaunch 뒤의 모든 추가 인자 수집 (electron에 전달할 인자들)
    let relaunch_args: Vec<String> = args.iter().position(|a| a == "--relaunch")
        .map(|i| args[i + 2..].to_vec())
        .unwrap_or_default();
    // --mock-url <url> → api_base_url 오버라이드
    let mock_url = args.iter().position(|a| a == "--mock-url")
        .and_then(|i| args.get(i + 1).cloned());

    if is_test {
        tracing::info!("[Updater] Test mode enabled, relaunch: {:?}", relaunch_cmd);
    }
    if let Some(ref s) = scenario {
        tracing::info!("[Updater] Scenario: {}", s);
    }
    if let Some(ref url) = mock_url {
        tracing::info!("[Updater] Mock URL override: {}", url);
    }

    let test_config = TestModeConfig {
        enabled: is_test || scenario.is_some(),
        scenario,
        relaunch_cmd,
        relaunch_args,
    };

    let mut cfg = config::load_config_for_gui();
    // --mock-url이 지정되면 api_base_url 오버라이드
    if let Some(ref url) = mock_url {
        cfg.api_base_url = Some(url.clone());
        if cfg.github_owner.is_empty() {
            cfg.github_owner = "test-owner".to_string();
        }
        if cfg.github_repo.is_empty() {
            cfg.github_repo = "saba-chan".to_string();
        }
    }
    let modules_dir = resolve_modules_dir();
    let manager: ManagerState = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(manager)
        .manage(test_config)
        .invoke_handler(tauri::generate_handler![
            check_updates,
            get_status,
            download_all,
            download_component,
            apply_updates,
            apply_modules_only,
            full_install,
            install_component,
            get_install_status,
            get_install_progress,
            get_config,
            set_api_base_url,
            get_test_mode,
            get_apply_preparation,
            check_after_update,
            relaunch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
