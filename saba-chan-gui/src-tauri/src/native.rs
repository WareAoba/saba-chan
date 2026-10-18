//! 렌더러가 부르는 Rust 명령 (계약 §2). 결과 모양은 main.js 각 핸들러와 같다

use crate::daemon::{is_allowed_request, parse_method};
use crate::{lifecycle, settings, tray, window, AppState};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

// ── 데몬 ──

/// 데몬 HTTP 요청. 성공·HTTP 오류 모두 {status, data}, 연결 실패 {status:0, error, code}
#[tauri::command(rename_all = "camelCase")]
pub async fn daemon_request(
    app: AppHandle,
    method: String,
    path: String,
    body: Option<Value>,
    timeout_ms: Option<u64>,
    response_type: Option<String>,
) -> Value {
    let Some(method) = parse_method(&method).filter(|m| is_allowed_request(m, &path)) else {
        return json!({ "status": 0, "error": format!("Invalid daemon request: {path}"), "code": "EINVAL" });
    };
    let client = app.state::<AppState>().client.clone();
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(5000));
    match client.send(method, &path, body.as_ref(), timeout).await {
        Ok(res) => {
            let data = match response_type.as_deref() {
                Some("base64") => Value::String(base64::engine::general_purpose::STANDARD.encode(&res.body)),
                Some("text") => Value::String(String::from_utf8_lossy(&res.body).into_owned()),
                _ => res.data(),
            };
            json!({ "status": res.status, "data": data })
        }
        Err(e) => json!({ "status": 0, "error": e.message, "code": e.code }),
    }
}

/// 동의 종류 → (데몬 경로 머리, 확인 창 문구 키). 렌더러는 이 경로에 직접 POST할 수 없다 (daemon::is_allowed_request)
fn consent_kind(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "unisolated" => Some(("/api/sandbox/unisolated", "consent.unisolated")),
        "extension-gui" => Some(("/api/sandbox/extension-gui", "consent.extension_gui")),
        _ => None,
    }
}

/// 구성요소 이름은 `module-<이름>`·`ext-<id>` 꼴만 받는다 (경로·창 문구에 그대로 넣는다)
fn consent_path(kind: &str, component: &str) -> Option<(String, &'static str)> {
    let (base, key) = consent_kind(kind)?;
    let ok = !component.is_empty()
        && component.len() <= 128
        && component.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && component != "."
        && component != "..";
    ok.then(|| (format!("{base}/{component}"), key))
}

/// 확인되지 않은 구성요소 동의: 네이티브 확인 창에서 사용자가 허락했을 때만 데몬에 기록한다.
/// 취소하면 {success:false, error_code:"canceled"}
#[tauri::command]
pub async fn sandbox_consent_allow(app: AppHandle, window: WebviewWindow, kind: String, component: String) -> Value {
    let Some((path, key)) = consent_path(&kind, &component) else {
        return json!({ "success": false, "error": "Invalid consent request", "error_code": "invalid_request" });
    };
    let state = app.state::<AppState>();
    let vars = [("component", component.as_str())];
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(state.t(&format!("{key}.message"), &vars))
        .title(state.t(&format!("{key}.title"), &vars))
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            state.t("consent.allow", &[]),
            state.t("cancel", &[]),
        ))
        .parent(&window)
        .show(move |ok| {
            let _ = tx.send(ok);
        });
    if !rx.await.unwrap_or(false) {
        return json!({ "success": false, "error_code": "canceled" });
    }
    match state.client.post(&path, &json!({}), 10000).await {
        Ok(data) => data,
        Err(e) => e.error_body(json!({ "success": false })),
    }
}

#[tauri::command]
pub fn app_ready(app: AppHandle) {
    lifecycle::app_ready(&app);
}

#[tauri::command]
pub async fn daemon_status(app: AppHandle) -> Value {
    match app.state::<AppState>().client.get("/health", 1000).await {
        Ok(_) => json!({ "running": true, "message": "Daemon is running" }),
        Err(e) => json!({ "running": false, "message": format!("Daemon not responding: {}", e.message) }),
    }
}

#[tauri::command]
pub async fn daemon_restart(app: AppHandle) -> Value {
    lifecycle::daemon_restart(&app).await
}

/// React 종료 확인 창의 답
#[tauri::command]
pub fn app_close_response(app: AppHandle, choice: String) {
    match choice.as_str() {
        // 인터페이스만 종료 — GUI 프로세스 종료, 데몬 유지
        "exit-interface" => {
            tauri::async_runtime::spawn(async move { lifecycle::interface_only_quit(&app).await });
        }
        "quit" => {
            if let Some(w) = app.get_webview_window(window::MAIN) {
                let _ = w.hide();
            }
            tauri::async_runtime::spawn(async move { lifecycle::clean_quit(&app).await });
        }
        _ => {}
    }
}

/// 업데이트 적용 — 업데이터는 데몬이 띄운다 (U7). GUI는 다시 띄울 자기 경로만 알려 주고 종료한다
#[tauri::command]
pub async fn updater_launch_apply(app: AppHandle) -> Value {
    let relaunch: Vec<String> = std::env::current_exe()
        .map(|p| vec![p.to_string_lossy().into_owned()])
        .unwrap_or_default();
    let client = app.state::<AppState>().client.clone();
    match client
        .post("/api/updates/launch-updater", &json!({ "relaunch": relaunch }), 10000)
        .await
    {
        Ok(data) if data["ok"].as_bool() == Some(true) => {
            tracing::info!("[Updater] Daemon launched the updater; quitting GUI");
            lifecycle::spawn_clean_quit(&app, 500);
            json!({ "ok": true })
        }
        Ok(data) => {
            let error = data.get("error").filter(|e| e.as_str().is_some_and(|s| !s.is_empty()));
            json!({ "ok": false, "error": error.cloned().unwrap_or_else(|| json!("Failed to launch updater")) })
        }
        Err(e) => e.error_body(json!({ "ok": false })),
    }
}

// ── 제거 ──

const INSTALLER_NAMES: &[&str] = &[
    "saba-chan-installer.exe",
    "Saba-chan Installer.exe",
    "saba-chan-installer",
];

fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|d| INSTALLER_NAMES.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

#[cfg(windows)]
fn registry_install_location() -> Option<PathBuf> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan")
        .ok()?;
    let loc: String = key.get_value("InstallLocation").ok()?;
    Some(PathBuf::from(loc.trim()))
}

#[cfg(not(windows))]
fn registry_install_location() -> Option<PathBuf> {
    None
}

/// 설치 루트·하위·상위·exe 폴더, 없으면 레지스트리 InstallLocation에서 인스톨러를 찾는다
fn find_installer(install_root: &Path) -> Option<PathBuf> {
    let mut dirs = vec![
        install_root.to_path_buf(),
        install_root.join("saba-chan-gui"),
        install_root.join(".."),
    ];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir);
    }
    find_in(&dirs).or_else(|| registry_install_location().and_then(|d| find_in(&[d])))
}

/// 인스톨러를 --uninstall로 분리 실행하고 GUI를 끝낸다 (cleanQuit이 데몬도 끈다)
#[tauri::command]
pub fn launch_uninstaller(app: AppHandle) -> Value {
    let state = app.state::<AppState>();
    let Some(installer) = find_installer(&state.install_root) else {
        return json!({ "success": false, "error": "Installer not found" });
    };
    tracing::info!("[Uninstall] Launching installer: {}", installer.display());
    let mut cmd = std::process::Command::new(&installer);
    cmd.arg("--uninstall")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(_) => {
            lifecycle::spawn_clean_quit(&app, 500);
            json!({ "success": true })
        }
        Err(e) => {
            tracing::error!("[Uninstall] Failed to launch installer: {}", e);
            json!({ "success": false, "error": e.to_string() })
        }
    }
}

// ── 설정·언어 ──

/// 데몬이 응답하면 API로만 저장한다(단일 원천). 데몬이 아예 응답하지 않을 때만 파일에 쓴다
pub async fn save_settings(state: &AppState, s: &Value) -> bool {
    match state.client.put("/api/config/gui", s, 5000).await {
        Ok(_) => return true,
        Err(e) if e.status.is_some() => {
            tracing::error!("[saveSettings] Daemon rejected settings: {}", e.message);
            return false;
        }
        Err(_) => {}
    }
    save_local(state, s)
}

fn save_local(state: &AppState, s: &Value) -> bool {
    match settings::write_gui_section(&state.data_dir, s) {
        Ok(()) => {
            tracing::info!("[saveSettings] Daemon not reachable: saved locally");
            true
        }
        Err(e) => {
            tracing::error!("[saveSettings] Local save failed: {}", e);
            false
        }
    }
}

#[tauri::command]
pub fn settings_local_load(app: AppHandle) -> Value {
    app.state::<AppState>().settings()
}

/// 데몬이 응답하지 않을 때만 JS가 부른다. 로컬 설정에 덮어 쓰고 다른 네임스페이스는 보존
#[tauri::command]
pub fn settings_local_save(app: AppHandle, settings: Value) -> bool {
    let state = app.state::<AppState>();
    let merged = settings::merge(state.settings(), &settings);
    let ok = save_local(&state, &merged);
    state.refresh_ipc_base();
    ok
}

#[tauri::command]
pub fn settings_get_path(app: AppHandle) -> String {
    settings::config_path(&app.state::<AppState>().data_dir)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub fn refresh_ipc_base(app: AppHandle) {
    app.state::<AppState>().refresh_ipc_base();
}

#[tauri::command]
pub fn language_get(app: AppHandle) -> String {
    settings::language(&app.state::<AppState>().settings())
}

/// 저장은 데몬 API(실패 시 로컬). 트레이 문구도 새 언어로.
/// 데몬은 재시작하지 않는다 — 실행 중인 서버가 모두 끝나기 때문
#[tauri::command]
pub async fn language_set(app: AppHandle, language: String) -> Value {
    let state = app.state::<AppState>();
    let mut s = state.settings();
    s["language"] = Value::String(language.clone());
    let success = save_settings(&state, &s).await;
    state.reload_i18n(&language);
    tray::update_menu(&app);
    json!({ "success": success, "language": language })
}

// ── 대화상자 ──

#[derive(Deserialize)]
pub struct FileFilter {
    name: String,
    extensions: Vec<String>,
}

fn default_filters() -> Vec<FileFilter> {
    let f = |name: &str, ext: &str| FileFilter {
        name: name.into(),
        extensions: vec![ext.into()],
    };
    if cfg!(windows) {
        vec![f("Executable Files", "exe"), f("All Files", "*")]
    } else if cfg!(target_os = "macos") {
        vec![f("Applications", "app"), f("All Files", "*")]
    } else {
        vec![f("All Files", "*")]
    }
}

fn with_filters<R: tauri::Runtime>(
    mut b: tauri_plugin_dialog::FileDialogBuilder<R>,
    filters: &[FileFilter],
) -> tauri_plugin_dialog::FileDialogBuilder<R> {
    for f in filters {
        // 리눅스(GTK)는 "*"를 "*.*"로 바꿔 확장자 없는 파일을 숨긴다. 필터가 없으면 모든 파일이 보인다
        if cfg!(target_os = "linux") && f.extensions.iter().all(|e| e == "*") {
            continue;
        }
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        b = b.add_filter(&f.name, &exts);
    }
    b
}

type PathReply = tokio::sync::oneshot::Sender<Option<String>>;

fn reply(tx: PathReply) -> impl FnOnce(Option<tauri_plugin_dialog::FilePath>) + Send + 'static {
    move |p| {
        let _ = tx.send(p.map(|p| p.to_string()));
    }
}

#[tauri::command]
pub async fn dialog_open_file(app: AppHandle, filters: Option<Vec<FileFilter>>) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let filters = filters.unwrap_or_else(default_filters);
    with_filters(app.dialog().file(), &filters).pick_file(reply(tx));
    rx.await.ok().flatten()
}

#[tauri::command]
pub async fn dialog_open_folder(app: AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(reply(tx));
    rx.await.ok().flatten()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn dialog_save_file(
    app: AppHandle,
    default_path: Option<String>,
    filters: Option<Vec<FileFilter>>,
) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut b = with_filters(app.dialog().file(), &filters.unwrap_or_default());
    if let Some(dp) = default_path.filter(|p| !p.is_empty()) {
        let dp = PathBuf::from(dp);
        if let Some(dir) = dp.parent().filter(|d| d.is_dir()) {
            b = b.set_directory(dir);
        }
        if let Some(name) = dp.file_name() {
            b = b.set_file_name(name.to_string_lossy());
        }
    }
    b.save_file(reply(tx));
    rx.await.ok().flatten()
}

// ── 셸·로그·버전·알림 ──

#[tauri::command]
pub fn shell_open_external(app: AppHandle, url: Option<String>) -> Value {
    let state = app.state::<AppState>();
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return json!({ "error": state.t("errors.url_missing", &[]) });
    };
    if !window::is_web_url(&url) {
        return json!({ "error": state.t("errors.url_not_web", &[]) });
    }
    match app.opener().open_url(url, None::<&str>) {
        Ok(()) => json!({ "success": true }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

#[tauri::command]
pub fn shell_open_path(app: AppHandle, path: Option<String>) -> Value {
    let state = app.state::<AppState>();
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return json!({ "error": state.t("errors.path_missing", &[]) });
    };
    // 폴더만 연다: 파일 경로를 셸에 넘기면 그 파일이 실행된다
    if !is_folder(&path) {
        return json!({ "error": state.t("errors.path_not_folder", &[]) });
    }
    match app.opener().open_path(path, None::<&str>) {
        Ok(()) => json!({ "success": true }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn is_folder(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

#[tauri::command]
pub fn logs_get_path(app: AppHandle) -> String {
    let state = app.state::<AppState>();
    match &state.log_path {
        Some(p) => p.to_string_lossy().into_owned(),
        None => state.t("errors.no_log_file", &[]),
    }
}

#[tauri::command]
pub fn logs_open_folder(app: AppHandle) -> Value {
    let state = app.state::<AppState>();
    let dir = state.data_dir.join("logs");
    if !dir.exists() {
        return json!({ "error": state.t("errors.no_log_folder", &[]) });
    }
    let _ = app.opener().open_path(dir.to_string_lossy(), None::<&str>);
    json!({ "success": true })
}

#[tauri::command]
pub fn app_get_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// OS 알림 (렌더러의 포트 충돌 알림 등)
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String) {
    lifecycle::show_notification(&app, &title, &body);
}

// ── 콘솔 팝아웃 ──

/// 창을 만드는 명령은 async여야 한다: 동기 명령은 메인 스레드에서 돌아서, Windows에서는 창 생성이 끝나지 않고 GUI가 멈춘다
#[tauri::command(rename_all = "camelCase")]
pub async fn console_popout(app: AppHandle, instance_id: String, server_name: Option<String>) -> Value {
    window::open_popout(&app, &instance_id, server_name.as_deref().unwrap_or(""))
        .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

#[tauri::command]
pub fn console_popout_list(app: AppHandle) -> Vec<String> {
    window::popout_list(&app)
}

#[tauri::command(rename_all = "camelCase")]
pub fn console_focus_popout(app: AppHandle, instance_id: String) -> Value {
    window::focus_popout(&app, &instance_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn console_popin(app: AppHandle, instance_id: String) -> Value {
    window::popin(&app, &instance_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn console_toggle_always_on_top(app: AppHandle, instance_id: String, pinned: bool) -> Value {
    window::toggle_always_on_top(&app, &instance_id, pinned)
}

// ── 창 제어 (호출한 창에 적용) ──

#[tauri::command]
pub fn window_minimize(window: WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
pub fn window_maximize(window: WebviewWindow) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

/// 메인 창이면 닫기 요청이 가로채여 app:closeRequest가 된다
#[tauri::command]
pub fn window_close(window: WebviewWindow) {
    let _ = window.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_path_takes_only_known_kinds_and_plain_names() {
        assert_eq!(
            consent_path("extension-gui", "ext-docker"),
            Some(("/api/sandbox/extension-gui/ext-docker".into(), "consent.extension_gui"))
        );
        assert_eq!(consent_path("unisolated", "module-palworld").unwrap().0, "/api/sandbox/unisolated/module-palworld");
        for (kind, component) in [
            ("other", "ext-x"),
            ("unisolated", ""),
            ("unisolated", ".."),
            ("unisolated", "a/b"),
            ("unisolated", "a%2fb"),
            ("extension-gui", "ext-x?y=1"),
            ("extension-gui", "ext x"),
        ] {
            assert!(consent_path(kind, component).is_none(), "{kind} {component:?}");
        }
    }

    #[test]
    fn open_path_accepts_only_folders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.bat");
        std::fs::write(&file, "echo").unwrap();
        assert!(is_folder(dir.path().to_str().unwrap()));
        assert!(!is_folder(file.to_str().unwrap()));
        assert!(!is_folder(dir.path().join("missing").to_str().unwrap()));
    }
}
