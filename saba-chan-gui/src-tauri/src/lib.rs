//! Saba-chan GUI 셸 (Tauri) — Electron main.js를 옮긴 것
//!
//! 화면(React)은 그대로 두고, 네이티브 일(창·트레이·데몬 기동·토큰·이벤트 스트림)만 여기서 한다.
//! 데몬 API 응답 가공은 JS 브리지(src/api)가 맡는다.
//!
//! ## 인자
//! - `--spawned-by-daemon` 데몬 boot_selector가 붙인다 (무시)
//! - `--after-update`      업데이터가 다시 띄웠다 → 첫 ready 뒤 `updates:completed`
//! - `--open-server=<id>`  CLI의 `gui`가 띄웠다 → 그 서버를 연다 (`app:openServer`). 이미 떠 있으면 그 창이 받는다
//! - `--tray`              데몬이 띄운 상주 GUI: 창 없이 트레이만 든다 (`tray_host.rs`). 이미 떠 있으면 아무것도 하지 않는다
//!
//! 창을 모두 닫아도 프로세스는 트레이로 남는다. 끝나는 것은 완전히 종료·업데이트·데몬이 밖에서 꺼졌을 때뿐이다.

pub mod daemon;
pub mod events;
pub mod i18n;
pub mod lifecycle;
pub mod logging;
pub mod native;
pub mod settings;
pub mod tray;
pub mod window;

use daemon::DaemonClient;
use i18n::I18n;
use saba_chan_updater_lib::constants;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

pub struct AppState {
    pub client: Arc<DaemonClient>,
    pub data_dir: PathBuf,
    pub install_root: PathBuf,
    pub log_path: Option<PathBuf>,
    pub after_update: bool,
    /// `--tray`로 떴다: 처음부터 창을 열지 않는다
    pub tray_start: bool,
    pub i18n: RwLock<I18n>,
    pub life: lifecycle::Lifecycle,
}

impl AppState {
    pub fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.i18n.read().unwrap().t(key, vars)
    }

    /// 로컬 config.json의 gui 섹션 + 기본값
    pub fn settings(&self) -> Value {
        settings::load_settings(&self.data_dir)
    }

    /// 설정 포트를 다시 읽는다 (IPC_BASE 환경변수가 있으면 그것)
    pub fn refresh_ipc_base(&self) {
        self.client.set_base(settings::ipc_base(&self.settings()));
    }

    pub fn reload_i18n(&self, lang: &str) {
        *self.i18n.write().unwrap() = I18n::load(lang, Some(&self.install_root.join("locales")));
    }
}

/// 이벤트는 메인 창에만 보낸다
pub fn emit_main<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if let Err(e) = app.emit_to(window::MAIN, event, payload) {
        tracing::debug!("emit {} failed: {}", event, e);
    }
}

/// 설치 루트: 개발(debug) = 저장소 루트, 배포 = exe 폴더
fn install_root() -> PathBuf {
    if cfg!(debug_assertions) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        return repo.canonicalize().unwrap_or(repo);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn run() {
    let data_dir = constants::resolve_data_dir();
    let log_path = logging::init(&data_dir);
    let install_root = install_root();
    tracing::info!(
        "Saba-chan starting... version {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    tracing::info!(
        "installRoot: {} dataDir: {}",
        install_root.display(),
        data_dir.display()
    );

    // 설정을 데몬 기동 전에 읽는다
    let s = settings::load_settings(&data_dir);
    let client = Arc::new(DaemonClient::new(settings::ipc_base(&s), constants::token_file_path()));
    let i18n = I18n::load(&settings::language(&s), Some(&install_root.join("locales")));
    let state = AppState {
        client,
        data_dir,
        install_root,
        log_path,
        after_update: std::env::args().any(|a| a == "--after-update"),
        tray_start: lifecycle::tray_arg(std::env::args()),
        i18n: RwLock::new(i18n),
        life: lifecycle::Lifecycle::default(),
    };

    let app = tauri::Builder::default()
        // 두 번째 실행은 창을 새로 띄우지 않고 떠 있는 창을 앞으로 가져온다.
        // 여러 GUI가 한 데몬을 함께 쓰다 한쪽의 완전히 종료가 다른 쪽의 데몬을 꺼뜨리던 것을 막는다
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // 데몬이 상주 GUI를 다시 띄우려 했다: 이 프로세스가 이미 트레이를 들고 있다
            if lifecycle::tray_arg(args.iter().cloned()) {
                return;
            }
            tracing::info!("Second instance launched; focusing the existing window");
            window::show_main(app);
            if let Some(id) = lifecycle::open_server_arg(args) {
                lifecycle::open_server(app, id);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            native::daemon_request,
            native::app_ready,
            native::daemon_status,
            native::daemon_restart,
            native::app_close_response,
            native::updater_launch_apply,
            native::launch_uninstaller,
            native::settings_local_load,
            native::settings_local_save,
            native::settings_get_path,
            native::refresh_ipc_base,
            native::language_get,
            native::language_set,
            native::dialog_open_file,
            native::dialog_open_folder,
            native::dialog_save_file,
            native::shell_open_external,
            native::shell_open_path,
            native::logs_get_path,
            native::logs_open_folder,
            native::app_get_version,
            native::notify,
            native::console_popout,
            native::console_popout_list,
            native::console_focus_popout,
            native::console_popin,
            native::console_toggle_always_on_top,
            native::window_minimize,
            native::window_maximize,
            native::window_close,
            native::sandbox_consent_allow,
            native::tray_action,
            native::tray_popup_fit,
        ])
        .setup(|app| {
            let handle = app.handle();
            let tray_start = handle.state::<AppState>().tray_start;
            if !tray_start {
                window::create_main_window(handle)?;
            }
            if let Err(e) = tray::create(handle) {
                tracing::warn!("[Tray] Failed to create tray: {}", e);
            }
            if tray_start {
                // 렌더러가 없으니 app_ready를 기다리지 않고 데몬 연결·등록을 시작한다
                lifecycle::begin_init(handle);
            }
            if let Some(id) = lifecycle::open_server_arg(std::env::args()) {
                lifecycle::open_server(handle, id);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building saba-chan-gui");

    app.run(|app, event| match event {
        // 마지막 창이 닫혀도(code 없음) 끝나지 않고 트레이로 남는다. app.exit()는 code가 있다
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => lifecycle::on_exit(app),
        _ => {}
    });
}
