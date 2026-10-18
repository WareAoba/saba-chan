//! Saba-chan GUI 셸 (Tauri) — Electron main.js를 옮긴 것
//!
//! 화면(React)은 그대로 두고, 네이티브 일(창·트레이·데몬 기동·토큰·이벤트 스트림)만 여기서 한다.
//! 데몬 API 응답 가공은 JS 브리지(src/api)가 맡는다.
//!
//! ## 인자
//! - `--spawned-by-daemon` 데몬 boot_selector가 붙인다 (무시)
//! - `--after-update`      업데이터가 다시 띄웠다 → 첫 ready 뒤 `updates:completed`

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
use tauri::{AppHandle, Emitter, RunEvent};

pub struct AppState {
    pub client: Arc<DaemonClient>,
    pub data_dir: PathBuf,
    pub install_root: PathBuf,
    pub log_path: Option<PathBuf>,
    pub after_update: bool,
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
        i18n: RwLock::new(i18n),
        life: lifecycle::Lifecycle::default(),
    };

    let app = tauri::Builder::default()
        // 두 번째 실행은 창을 새로 띄우지 않고 떠 있는 창을 앞으로 가져온다.
        // 여러 GUI가 한 데몬을 함께 쓰다 한쪽의 완전히 종료가 다른 쪽의 데몬을 꺼뜨리던 것을 막는다
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tracing::info!("Second instance launched; focusing the existing window");
            window::show_main(app);
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
        ])
        .setup(|app| {
            let handle = app.handle();
            window::create_main_window(handle)?;
            if let Err(e) = tray::create(handle) {
                tracing::warn!("[Tray] Failed to create tray: {}", e);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building saba-chan-gui");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            lifecycle::on_exit(app);
        }
    });
}
