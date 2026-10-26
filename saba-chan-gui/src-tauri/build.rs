use std::path::Path;

/// 렌더러가 부를 수 있는 명령. capabilities/default.json의 allow-* 권한과 짝을 맞춘다
const COMMANDS: &[&str] = &[
    "daemon_request",
    "app_ready",
    "daemon_status",
    "daemon_restart",
    "app_close_response",
    "updater_launch_apply",
    "launch_uninstaller",
    "settings_local_load",
    "settings_local_save",
    "settings_get_path",
    "refresh_ipc_base",
    "language_get",
    "language_set",
    "dialog_open_file",
    "dialog_open_folder",
    "dialog_save_file",
    "shell_open_external",
    "shell_open_path",
    "logs_get_path",
    "logs_open_folder",
    "app_get_version",
    "notify",
    "console_popout",
    "console_popout_list",
    "console_focus_popout",
    "console_popin",
    "console_toggle_always_on_top",
    "window_minimize",
    "window_maximize",
    "window_close",
    "sandbox_consent_allow",
    "tray_action",
    "tray_popup_fit",
];

/// 자리표시 index.html에만 있는 문구
const PLACEHOLDER_MARK: &str = "프런트가 빌드되지 않았습니다";
const ALLOW_PLACEHOLDER: &str = "SABA_ALLOW_PLACEHOLDER_GUI";

fn main() {
    // 프런트를 빌드하지 않아도 cargo test --workspace가 돌게 자리표시 파일을 둔다 (있으면 건드리지 않는다)
    let index = Path::new("../build/index.html");
    println!("cargo:rerun-if-changed=../build/index.html");
    let placeholder = !index.exists() || std::fs::read_to_string(index).is_ok_and(|s| s.contains(PLACEHOLDER_MARK));
    // 화면 없는 GUI가 릴리스로 나가지 않게 release는 멈춘다. 용량 측정처럼 일부러 빌드할 때만 환경 변수로 넘긴다
    println!("cargo:rerun-if-env-changed={ALLOW_PLACEHOLDER}");
    if placeholder && std::env::var("PROFILE").as_deref() == Ok("release") {
        if std::env::var_os(ALLOW_PLACEHOLDER).is_none() {
            panic!(
                "프런트가 빌드되지 않았습니다. saba-chan-gui에서 npm run build를 먼저 실행하세요 \
                 (안내 화면만 든 GUI를 일부러 만들려면 {ALLOW_PLACEHOLDER}=1)"
            );
        }
        println!("cargo:warning=프런트 없이 안내 화면만 든 GUI를 만듭니다 ({ALLOW_PLACEHOLDER})");
    }
    if !index.exists() {
        let _ = std::fs::create_dir_all("../build");
        let _ = std::fs::write(
            index,
            "<!doctype html><meta charset=\"utf-8\"><title>Saba-chan</title>\
             <p>프런트가 빌드되지 않았습니다. saba-chan-gui에서 <code>npm run build</code>를 실행하세요.</p>\n",
        );
    }
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build");
}
