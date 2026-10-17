//! 사바쨩 완전 제거 (Uninstall)
//!
//! 설치 디렉토리, 저장 폴더(%APPDATA%/saba-chan), 임시 폴더, 레지스트리를 모두 삭제합니다.
//! `--uninstall` 인자로 실행됩니다.
//!
//! ## 제거 대상
//! 1. 설치 디렉토리 (레지스트리의 InstallLocation에서 읽음)
//! 2. 사용자 데이터: `%APPDATA%/saba-chan` (settings.json, modules 등)
//! 3. 임시 파일: `%TEMP%/saba-chan-*`
//! 4. 레지스트리: `HKCU\...\Uninstall\Saba-chan`
//! 5. 바탕화면/시작메뉴 바로가기 (Linux: .desktop 항목)
//! 6. 자기 자신 (cleanup 스크립트를 통해 프로세스 종료 후 삭제)
use saba_chan_updater_lib::constants;
use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

#[cfg(target_os = "windows")]
use crate::registry;
#[cfg(target_os = "windows")]
use crate::shortcuts;

/// 제거 전에 종료할 saba-chan 프로세스 이름.
///
/// Windows GUI 는 electron-builder portable 런처(`saba-chan-gui.exe`)가
/// 임시 폴더에 풀린 내부 앱(`Saba-chan.exe`, productName)을 실행하므로 두 이름을 모두 종료한다.
/// 인스톨러 자신(`saba-chan-installer`)은 포함하지 않는다.
#[cfg(target_os = "windows")]
pub(crate) const SABA_PROCESS_NAMES: &[&str] = &[
    "saba-core.exe",
    "saba-chan-cli.exe",
    "saba-chan-gui.exe",
    "Saba-chan.exe",
    "saba-chan-updater.exe",
];
#[cfg(not(target_os = "windows"))]
pub(crate) const SABA_PROCESS_NAMES: &[&str] = &[
    "saba-core",
    "saba-chan-cli",
    "saba-chan-gui",
    "saba-chan-updater",
];

/// 제거 진행 이벤트
#[derive(Debug, Clone, Serialize)]
pub struct UninstallProgress {
    pub step: String,
    pub message: String,
    pub percent: i32,
    pub complete: bool,
    pub error: Option<String>,
}

/// GUI 모드 제거 실행
pub async fn do_uninstall(app: &AppHandle, keep_settings: bool) {
    use tauri::Emitter;

    let emit = |step: &str, message: &str, percent: i32, complete: bool, error: Option<String>| {
        let p = UninstallProgress {
            step: step.to_string(),
            message: message.to_string(),
            percent,
            complete,
            error,
        };
        app.emit("uninstall:progress", &p).ok();
    };

    let failures = run_uninstall(keep_settings, |step, msg, pct| emit(step, msg, pct, false, None)).await;

    if failures.is_empty() {
        emit("complete", "Saba-chan has been completely removed.", 100, true, None);
        tracing::info!("[Uninstall] Complete");
    } else {
        let message = format!(
            "Uninstall completed with {} issue(s): {}",
            failures.len(),
            failures.join(" | ")
        );
        emit("error", &message, 0, false, Some(message.clone()));
        tracing::warn!("[Uninstall] Completed with failures: {}", message);
    }
}

/// 사일런트(GUI 없이) 제거
pub async fn do_silent_uninstall() {
    eprintln!("Saba-chan Uninstaller (silent mode)");
    let failures = run_uninstall(false, |_, msg, _| eprintln!("  {}", msg)).await;
    if failures.is_empty() {
        eprintln!("[OK] Saba-chan has been completely removed.");
    } else {
        eprintln!("[ERROR] Uninstall completed with issues: {}", failures.join(" | "));
    }
}

/// GUI·사일런트 공통 제거 절차. 실패 항목 목록을 반환한다.
/// `report(step, message, percent)` 로 진행 상황을 알린다.
async fn run_uninstall(keep_settings: bool, mut report: impl FnMut(&str, &str, i32)) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();

    report("detect", "Detecting install location...", 5);
    #[cfg(target_os = "windows")]
    let install_location = registry::get_install_location();
    #[cfg(not(target_os = "windows"))]
    let install_location: Option<String> = None;
    let install_dir = match detect_install_dir(install_location.as_deref()) {
        Ok(dir) => dir,
        Err(error) => {
            failures.push(format!("install_dir: {}", error));
            None
        }
    };

    report("stop", "Stopping saba-chan processes...", 10);
    stop_saba_processes();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    report("shortcuts", "Removing shortcuts...", 20);
    remove_shortcuts();

    report("files", "Removing installation files...", 35);
    if let Some(dir) = &install_dir {
        match remove_dir_robust(dir) {
            Ok(_) => tracing::info!("[Uninstall] Removed install dir: {:?}", dir),
            Err(e) => failures.push(format!("install_dir: {}", e)),
        }
    }

    // python-standalone, python-env, node-portable 등 런타임 디렉토리도 포함
    if keep_settings {
        report("data-keep", "Removing runtime environments (keeping settings)...", 55);
        failures.extend(remove_appdata_dir_keep_settings());
    } else {
        report("data", "Removing user data and runtime environments...", 55);
        failures.extend(remove_appdata_dir());
    }

    report("temp", "Cleaning temporary files...", 70);
    failures.extend(remove_temp_files());

    // appdata 삭제에 포함되지만 모듈 경로가 별도로 지정된 경우 대비
    if keep_settings {
        report("modules-keep", "Keeping module settings...", 80);
    } else {
        report("modules", "Cleaning modules directory...", 80);
        failures.extend(remove_modules_dir());
    }

    report("registry", "Removing registry entries...", 90);
    #[cfg(target_os = "windows")]
    if let Err(e) = registry::remove_uninstall_entry() {
        failures.push(format!("registry: {}", e));
    }

    report("self-delete", "Scheduling self-deletion...", 95);
    schedule_self_delete(install_dir.as_ref());

    failures
}

// ═══════════════════════════════════════════════════════
// 내부 헬퍼
// ═══════════════════════════════════════════════════════

/// 어떤 언어로 설치했든 제거되도록 모든 로케일 이름의 바로가기를 지운다.
fn remove_shortcuts() {
    #[cfg(target_os = "windows")]
    for name in shortcuts::ALL_SHORTCUT_NAMES {
        let _ = shortcuts::remove_desktop_shortcut(name);
        let _ = shortcuts::remove_start_menu_shortcut(name);
    }
    #[cfg(not(target_os = "windows"))]
    if let Some(data_dir) = dirs::data_dir() {
        let _ = std::fs::remove_file(data_dir.join("applications/saba-chan.desktop"));
    }
}

fn stop_saba_processes() {
    for name in SABA_PROCESS_NAMES {
        let _ = kill_command(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(target_os = "windows")]
fn kill_command(name: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/IM", name, "/F"]);
    cmd
}

/// `pkill -x` 는 15자로 잘린 프로세스 이름(comm)과 비교하므로 `saba-chan-updater` 같은
/// 긴 이름을 놓친다. 대신 명령줄의 실행 파일 이름(argv[0] basename)만 정확히 맞추는
/// 패턴을 쓴다. `saba-chan` 부분 일치로 인스톨러 자신을 죽이지 않는다.
#[cfg(not(target_os = "windows"))]
fn kill_command(name: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("pkill");
    cmd.args(["-f", &unix_exact_exe_pattern(name)]);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn unix_exact_exe_pattern(name: &str) -> String {
    format!("^([^ ]*/)?{}( |$)", name)
}

/// 디렉토리 강제 삭제 (잠금 파일에 대한 재시도 포함)
fn remove_dir_robust(dir: &PathBuf) -> anyhow::Result<()> {
    if dir.exists() && !dir.is_dir() {
        return Err(anyhow::anyhow!("path is not a directory: {}", dir.display()));
    }

    // 1차 시도
    if std::fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }

    // 재시도 (파일 잠금 해제 대기)
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 파일을 하나씩 삭제 시도
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let _ = remove_dir_robust(&path);
            } else {
                // 읽기전용 해제 후 삭제
                if let Ok(md) = std::fs::metadata(&path) {
                    let mut perms = md.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    let _ = std::fs::set_permissions(&path, perms);
                }
                let _ = std::fs::remove_file(&path);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    if dir.exists() {
        return Err(anyhow::anyhow!("directory still exists after removal attempts: {}", dir.display()));
    }

    Ok(())
}

fn remove_appdata_dir() -> Vec<String> {
    let mut failures = Vec::new();
    let saba_dir = constants::resolve_data_dir();
    if saba_dir.exists() {
        tracing::info!("[Uninstall] Removing appdata: {:?}", saba_dir);
        if let Err(e) = remove_dir_robust(&saba_dir) {
            failures.push(format!("appdata: {}", e));
        }
    }
    failures
}

/// 설정 파일만 남기고 나머지 삭제 (바이너리, 런타임, 캐시 등)
fn remove_appdata_dir_keep_settings() -> Vec<String> {
    let mut failures = Vec::new();
    let saba_dir = constants::resolve_data_dir();
    if !saba_dir.exists() {
        return failures;
    }

    // 보존할 파일/폴더 패턴 (config, settings, 인스턴스 설정)
    let keep_names: &[&str] = &[
        "settings.json",     // GUI 설정
        "instances",         // 인스턴스별 설정 저장
    ];

    // 삭제 대상: 보존 대상을 제외한 모든 파일/폴더
    if let Ok(entries) = std::fs::read_dir(&saba_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if keep_names.iter().any(|k| name_str == *k) {
                tracing::info!("[Uninstall] Keeping: {:?}", entry.path());
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                tracing::info!("[Uninstall] Removing dir: {:?}", path);
                if let Err(e) = remove_dir_robust(&path) {
                    failures.push(format!("keep_settings/remove_dir {}: {}", path.display(), e));
                }
            } else {
                tracing::info!("[Uninstall] Removing file: {:?}", path);
                if let Err(e) = std::fs::remove_file(&path) {
                    failures.push(format!("keep_settings/remove_file {}: {}", path.display(), e));
                }
            }
        }
    }
    failures
}

fn remove_temp_files() -> Vec<String> {
    let mut failures = Vec::new();
    let temp = std::env::temp_dir();

    // saba-chan-installer, saba-chan-updater 등 임시 폴더
    let patterns = ["saba-chan-installer", "saba-chan-updater", "saba-chan-temp"];
    for pattern in &patterns {
        let dir = temp.join(pattern);
        if dir.exists() {
            tracing::info!("[Uninstall] Removing temp: {:?}", dir);
            if let Err(e) = remove_dir_robust(&dir) {
                failures.push(format!("temp {}: {}", dir.display(), e));
            }
        }
    }
    failures
}

fn remove_modules_dir() -> Vec<String> {
    let mut failures = Vec::new();
    let modules_dir = constants::resolve_modules_dir();
    if modules_dir.exists() {
        tracing::info!("[Uninstall] Removing modules: {:?}", modules_dir);
        if let Err(e) = remove_dir_robust(&modules_dir) {
            failures.push(format!("modules: {}", e));
        }
    }
    failures
}

/// 프로세스 종료 후 자기 자신(및 남은 설치 디렉토리)을 삭제하는 cleanup 스크립트를 생성·실행.
///
/// Windows에서는 실행 중인 .exe를 직접 삭제할 수 없으므로,
/// 별도 cmd 스크립트를 spawn하여 프로세스 종료를 대기한 뒤 삭제한다.
fn schedule_self_delete(install_dir: Option<&PathBuf>) {
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;

        let self_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("[Uninstall] Cannot determine own exe path: {}", e);
                return;
            }
        };

        let pid = std::process::id();
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("saba-chan-cleanup.cmd");

        // 설치 디렉토리 삭제 명령 (있으면)
        let rmdir_line = match install_dir {
            Some(dir) => format!(
                "if exist \"{}\" rmdir /s /q \"{}\"",
                escape_batch_path(dir),
                escape_batch_path(dir)
            ),
            None => String::new(),
        };

        // 부모 디렉토리도 정리 시도 (비어있으면 삭제)
        let parent_line = match self_exe.parent() {
            Some(parent) => format!(
                "rmdir \"{}\" 2>nul",
                escape_batch_path(parent)
            ),
            None => String::new(),
        };

        let script_display = escape_batch_path(&script_path);
        let script = format!(
            "@echo off\r\n\
            setlocal DisableDelayedExpansion\r\n\
            :wait\r\n\
            tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\n\
            if not errorlevel 1 (\r\n\
                timeout /t 1 /nobreak >nul\r\n\
                goto wait\r\n\
            )\r\n\
            del /f /q \"{self_exe}\"\r\n\
            {rmdir_line}\r\n\
            {parent_line}\r\n\
            del /f /q \"{script}\"\r\n",
            pid = pid,
            self_exe = escape_batch_path(&self_exe),
            rmdir_line = rmdir_line,
            parent_line = parent_line,
            script = script_display,
        );

        // 스크립트 파일 작성
        match std::fs::File::create(&script_path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(script.as_bytes()) {
                    tracing::warn!("[Uninstall] Failed to write cleanup script: {}", e);
                    return;
                }
            }
            Err(e) => {
                tracing::warn!("[Uninstall] Failed to create cleanup script: {}", e);
                return;
            }
        }

        // CREATE_NO_WINDOW 플래그로 cmd 실행 (콘솔 창 숨김)
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        match std::process::Command::new("cmd")
            .args(["/D", "/C", "saba-chan-cleanup.cmd"])
            .current_dir(temp_dir)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => tracing::info!(
                "[Uninstall] Cleanup script spawned: {:?}",
                script_path
            ),
            Err(e) => tracing::warn!(
                "[Uninstall] Failed to spawn cleanup script: {}",
                e
            ),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Unix: 실행 중인 바이너리도 바로 삭제 가능
        if let Ok(self_exe) = std::env::current_exe() {
            let _ = std::fs::remove_file(&self_exe);
        }
        if let Some(dir) = install_dir {
            let _ = remove_dir_robust(dir);
        }
    }
}

#[cfg(windows)]
fn escape_batch_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}


/// 제거할 설치 디렉토리를 결정한다. 레지스트리 값이 없으면 기본 설치 경로를 쓴다.
/// saba-chan 마커가 없는 디렉토리는 삭제를 거부한다.
fn detect_install_dir(install_location: Option<&str>) -> anyhow::Result<Option<PathBuf>> {
    let dir = install_location
        .filter(|loc| !loc.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::default_install_path);
    crate::validate_elevate_mkdir_path(&dir.to_string_lossy()).map_err(anyhow::Error::msg)?;
    if !dir.exists() {
        return Ok(None);
    }
    if !crate::verify_install_dir_marker(&dir) {
        anyhow::bail!("refusing to delete install dir without saba-chan marker: {}", dir.display());
    }
    Ok(Some(dir))
}


#[cfg(test)]
mod tests {
    use super::remove_dir_robust;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(windows)]
    #[test]
    fn cleanup_script_keeps_percent_paths_literal() {
        let dir = tempfile::tempdir().unwrap();
        let literal = r"C:\review\%SABA_RECOVERY_REVIEW_PATH_PART%\Saba-chan";
        let script = format!("@echo off\r\nsetlocal DisableDelayedExpansion\r\necho {}\r\n", super::escape_batch_path(std::path::Path::new(literal)));
        let script_path = dir.path().join("echo.cmd");
        std::fs::write(&script_path, script).unwrap();
        // NoDefaultCurrentDirectoryInExePath 환경에서도 실행되도록 절대 경로를 쓴다.
        let output = std::process::Command::new("cmd")
            .arg("/D")
            .arg("/C")
            .arg(&script_path)
            .current_dir(dir.path())
            .env("SABA_RECOVERY_REVIEW_PATH_PART", "different-target")
            .output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), literal);
    }

    #[test]
    fn cleanup_target_rejects_unmarked_registry_location() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::detect_install_dir(Some(dir.path().to_str().unwrap())).is_err());
    }

    #[test]
    fn cleanup_target_keeps_validated_directory_for_locked_file_retry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("saba-core.exe"), b"stub").unwrap();
        assert_eq!(super::detect_install_dir(Some(dir.path().to_str().unwrap())).unwrap(), Some(dir.path().to_path_buf()));
    }

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("saba-installer-test-{}-{}", name, ts))
    }

    #[test]
    fn remove_dir_robust_non_directory_path_returns_error() {
        let base = unique_temp_path("file");
        std::fs::create_dir_all(&base).expect("mkdir temp base");
        let file_path = base.join("not_a_dir.txt");
        std::fs::write(&file_path, b"x").expect("write temp file");

        let result = remove_dir_robust(&file_path);
        assert!(result.is_err());

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn remove_dir_robust_removes_directory_tree() {
        let dir_path = unique_temp_path("dir");
        let nested = dir_path.join("nested");
        std::fs::create_dir_all(&nested).expect("mkdirs");
        std::fs::write(nested.join("a.txt"), b"abc").expect("write nested file");

        let result = remove_dir_robust(&dir_path);
        assert!(result.is_ok());
        assert!(!dir_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn process_names_cover_gui_launcher_and_inner_app() {
        let names = super::SABA_PROCESS_NAMES;
        // portable 런처와 Electron 내부 앱 모두 종료해야 설치 폴더가 잠기지 않는다.
        assert!(names.contains(&"saba-chan-gui.exe"));
        assert!(names.contains(&"Saba-chan.exe"));
        assert!(names.contains(&"saba-core.exe"));
        assert!(names.contains(&"saba-chan-updater.exe"));
    }

    #[test]
    fn process_names_exclude_installer_itself() {
        assert!(super::SABA_PROCESS_NAMES
            .iter()
            .all(|n| !n.to_ascii_lowercase().starts_with("saba-chan-installer")));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_kill_pattern_is_anchored_to_exe_name() {
        assert_eq!(
            super::unix_exact_exe_pattern("saba-chan-gui"),
            "^([^ ]*/)?saba-chan-gui( |$)"
        );
    }
}
