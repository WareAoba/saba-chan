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
//! 5. 바탕화면/시작메뉴 바로가기

use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

use crate::registry;
use crate::shortcuts;

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
pub async fn do_uninstall(app: &AppHandle) {
    use tauri::Emitter;

    let emit = |step: &str, msg: &str, pct: i32| {
        let p = UninstallProgress {
            step: step.to_string(),
            message: msg.to_string(),
            percent: pct,
            complete: false,
            error: None,
        };
        app.emit("uninstall:progress", &p).ok();
    };

    let emit_complete = |msg: &str| {
        let p = UninstallProgress {
            step: "complete".to_string(),
            message: msg.to_string(),
            percent: 100,
            complete: true,
            error: None,
        };
        app.emit("uninstall:progress", &p).ok();
    };

    let _emit_err = |msg: &str| {
        let p = UninstallProgress {
            step: "error".to_string(),
            message: msg.to_string(),
            percent: 0,
            complete: false,
            error: Some(msg.to_string()),
        };
        app.emit("uninstall:progress", &p).ok();
    };

    // Step 1: 설치 경로 확인
    emit("detect", "Detecting install location...", 5);

    let install_location = registry::get_install_location();
    let install_dir = match &install_location {
        Some(loc) if !loc.is_empty() => {
            tracing::info!("[Uninstall] Install location: {}", loc);
            Some(PathBuf::from(loc))
        }
        _ => {
            tracing::warn!("[Uninstall] Install location not found in registry");
            // 기본 경로 시도
            let default = get_default_install_path();
            if PathBuf::from(&default).exists() {
                Some(PathBuf::from(&default))
            } else {
                None
            }
        }
    };

    // Step 2: 프로세스 종료
    emit("stop", "Stopping saba-chan processes...", 10);
    stop_saba_processes();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Step 3: 바로가기 제거
    emit("shortcuts", "Removing shortcuts...", 20);
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = shortcuts::remove_desktop_shortcut("Saba-chan") {
            tracing::warn!("Failed to remove desktop shortcut: {}", e);
        }
        if let Err(e) = shortcuts::remove_start_menu_shortcut("Saba-chan") {
            tracing::warn!("Failed to remove start menu shortcut: {}", e);
        }
    }

    // Step 4: 설치 디렉토리 삭제
    emit("files", "Removing installation files...", 35);
    if let Some(ref dir) = install_dir {
        if dir.exists() {
            match remove_dir_robust(dir) {
                Ok(_) => tracing::info!("[Uninstall] Removed install dir: {:?}", dir),
                Err(e) => {
                    tracing::warn!("[Uninstall] Partial removal of install dir: {}", e);
                }
            }
        }
    }

    // Step 5: 사용자 데이터 삭제 (%APPDATA%/saba-chan)
    emit("data", "Removing user data...", 55);
    remove_appdata_dir();

    // Step 6: 임시 파일 삭제
    emit("temp", "Cleaning temporary files...", 70);
    remove_temp_files();

    // Step 7: 모듈 디렉토리 삭제 (%APPDATA%/saba-chan/modules)
    // (앞서 appdata 전체를 삭제하므로 포함됨 — 혹시 남아있을 경우 대비)
    emit("modules", "Cleaning modules directory...", 80);
    remove_modules_dir();

    // Step 8: 레지스트리 삭제
    emit("registry", "Removing registry entries...", 90);
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = registry::remove_uninstall_entry() {
            tracing::warn!("Failed to remove registry entry: {}", e);
        }
    }

    // 완료
    emit_complete("Saba-chan has been completely removed.");
    tracing::info!("[Uninstall] Complete");
}

/// 사일런트(GUI 없이) 제거
pub async fn do_silent_uninstall() {
    eprintln!("Saba-chan Uninstaller (silent mode)");

    // 프로세스 종료
    eprintln!("  Stopping processes...");
    stop_saba_processes();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 바로가기 제거
    eprintln!("  Removing shortcuts...");
    #[cfg(target_os = "windows")]
    {
        let _ = shortcuts::remove_desktop_shortcut("Saba-chan");
        let _ = shortcuts::remove_start_menu_shortcut("Saba-chan");
    }

    // 설치 디렉토리 삭제
    let install_location = registry::get_install_location();
    if let Some(ref loc) = install_location {
        let dir = PathBuf::from(loc);
        if dir.exists() {
            eprintln!("  Removing install directory: {}", loc);
            let _ = remove_dir_robust(&dir);
        }
    } else {
        let default = get_default_install_path();
        let dir = PathBuf::from(&default);
        if dir.exists() {
            eprintln!("  Removing default install directory: {}", default);
            let _ = remove_dir_robust(&dir);
        }
    }

    // 사용자 데이터
    eprintln!("  Removing user data...");
    remove_appdata_dir();

    // 임시 파일
    eprintln!("  Cleaning temp files...");
    remove_temp_files();

    // 모듈
    eprintln!("  Cleaning modules...");
    remove_modules_dir();

    // 레지스트리
    eprintln!("  Removing registry entries...");
    #[cfg(target_os = "windows")]
    {
        let _ = registry::remove_uninstall_entry();
    }

    eprintln!("[OK] Saba-chan has been completely removed.");
}

// ═══════════════════════════════════════════════════════
// 내부 헬퍼
// ═══════════════════════════════════════════════════════

fn stop_saba_processes() {
    #[cfg(target_os = "windows")]
    {
        let targets = [
            "saba-core",
            "saba-chan-cli",
            "Saba-chan",        // Electron GUI
            "saba-chan-updater",
        ];
        for name in &targets {
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", &format!("{}.exe", name), "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// 디렉토리 강제 삭제 (잠금 파일에 대한 재시도 포함)
fn remove_dir_robust(dir: &PathBuf) -> anyhow::Result<()> {
    // 1차 시도
    if let Ok(_) = std::fs::remove_dir_all(dir) {
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

    Ok(())
}

fn remove_appdata_dir() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let saba_dir = PathBuf::from(appdata).join("saba-chan");
            if saba_dir.exists() {
                tracing::info!("[Uninstall] Removing appdata: {:?}", saba_dir);
                let _ = remove_dir_robust(&saba_dir);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let saba_dir = PathBuf::from(home).join(".config").join("saba-chan");
            if saba_dir.exists() {
                let _ = remove_dir_robust(&saba_dir);
            }
        }
    }
}

fn remove_temp_files() {
    let temp = std::env::temp_dir();

    // saba-chan-installer, saba-chan-updater 등 임시 폴더
    let patterns = ["saba-chan-installer", "saba-chan-updater", "saba-chan-temp"];
    for pattern in &patterns {
        let dir = temp.join(pattern);
        if dir.exists() {
            tracing::info!("[Uninstall] Removing temp: {:?}", dir);
            let _ = remove_dir_robust(&dir);
        }
    }
}

fn remove_modules_dir() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let modules_dir = PathBuf::from(appdata).join("saba-chan").join("modules");
            if modules_dir.exists() {
                tracing::info!("[Uninstall] Removing modules: {:?}", modules_dir);
                let _ = remove_dir_robust(&modules_dir);
            }
        }
    }
}

fn get_default_install_path() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local)
                .join("saba-chan")
                .to_string_lossy()
                .to_string();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("saba-chan")
                .to_string_lossy()
                .to_string();
        }
    }
    "saba-chan".to_string()
}
