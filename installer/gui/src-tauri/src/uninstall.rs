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
//! 5. 바탕화면/시작메뉴 바로가기//! 6. 자기 자신 (cleanup 스크립트를 통해 프로세스 종료 후 삭제)
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
pub async fn do_uninstall(app: &AppHandle, keep_settings: bool) {
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
        // 어떤 언어로 설치했든 제거되도록 모든 로케일 이름 시도
        for name in &["Saba-chan", "사바쨩", "サバちゃん"] {
            let _ = shortcuts::remove_desktop_shortcut(name);
            let _ = shortcuts::remove_start_menu_shortcut(name);
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
    // python-standalone, python-env, node-portable 등 런타임 디렉토리도 포함
    if keep_settings {
        emit("data", "Removing runtime environments (keeping settings)...", 55);
        remove_appdata_dir_keep_settings();
    } else {
        emit("data", "Removing user data and runtime environments...", 55);
        remove_appdata_dir();
    }

    // Step 6: 임시 파일 삭제
    emit("temp", "Cleaning temporary files...", 70);
    remove_temp_files();

    // Step 7: 모듈 디렉토리 삭제 (%APPDATA%/saba-chan/modules)
    // (앞서 appdata 전체를 삭제하므로 포함됨 — 혹시 남아있을 경우 대비)
    if !keep_settings {
        emit("modules", "Cleaning modules directory...", 80);
        remove_modules_dir();
    } else {
        emit("modules", "Keeping module settings...", 80);
    }

    // Step 8: 레지스트리 삭제
    emit("registry", "Removing registry entries...", 90);
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = registry::remove_uninstall_entry() {
            tracing::warn!("Failed to remove registry entry: {}", e);
        }
    }

    // Step 9: 자기 자신 삭제 (cleanup 스크립트)
    emit("self-delete", "Scheduling self-deletion...", 95);
    if let Some(ref dir) = install_dir {
        schedule_self_delete(Some(dir));
    } else {
        schedule_self_delete(None);
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
        for name in &["Saba-chan", "사바쨩", "サバちゃん"] {
            let _ = shortcuts::remove_desktop_shortcut(name);
            let _ = shortcuts::remove_start_menu_shortcut(name);
        }
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
    eprintln!("  Removing user data and runtime environments...");
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

    // 자기 자신 삭제
    eprintln!("  Scheduling self-deletion...");
    {
        let install_location_ref = install_location.as_ref();
        let dir = install_location_ref.map(|l| PathBuf::from(l));
        schedule_self_delete(dir.as_ref());
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

/// 설정 파일만 남기고 나머지 삭제 (바이너리, 런타임, 캐시 등)
fn remove_appdata_dir_keep_settings() {
    #[cfg(target_os = "windows")]
    let saba_dir = std::env::var("APPDATA")
        .ok()
        .map(|a| PathBuf::from(a).join("saba-chan"));
    #[cfg(not(target_os = "windows"))]
    let saba_dir = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("saba-chan"));

    let Some(saba_dir) = saba_dir else { return };
    if !saba_dir.exists() {
        return;
    }

    // 보존할 파일/폴더 패턴 (config, settings, 인스턴스 설정)
    let keep_names: &[&str] = &[
        "settings.json",     // GUI 설정
        "config",            // 글로벌 config 디렉토리
        "instances",         // 인스턴스별 설정 저장
        "global.toml",       // 글로벌 설정
        "bot-config.json",   // 봇 설정
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
                let _ = remove_dir_robust(&path);
            } else {
                tracing::info!("[Uninstall] Removing file: {:?}", path);
                let _ = std::fs::remove_file(&path);
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
        let script_path = std::env::temp_dir().join("saba-chan-cleanup.cmd");

        // 설치 디렉토리 삭제 명령 (있으면)
        let rmdir_line = match install_dir {
            Some(dir) => format!(
                "if exist \"{}\" rmdir /s /q \"{}\"",
                dir.display(),
                dir.display()
            ),
            None => String::new(),
        };

        // 부모 디렉토리도 정리 시도 (비어있으면 삭제)
        let parent_line = match self_exe.parent() {
            Some(parent) => format!(
                "rmdir \"{}\" 2>nul",
                parent.display()
            ),
            None => String::new(),
        };

        let script_display = script_path.display().to_string();
        let script = format!(
            "@echo off\r\n\
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
            self_exe = self_exe.display(),
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
            .args(["/C", &script_path.to_string_lossy()])
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
