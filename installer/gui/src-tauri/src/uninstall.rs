//! 사바쨩 완전 제거 (Uninstall)
//!
//! 설치 디렉토리, 저장 폴더(%APPDATA%/saba-chan), 임시 폴더, 레지스트리를 모두 삭제합니다.
//! `--uninstall` 인자로 실행됩니다.
//!
//! ## 제거 대상
//! 1. 설치 디렉토리: 제거기 자신이 든 폴더 (설치기가 `saba-chan-installer.exe`를 설치 폴더에 복사해 둔다).
//!    레지스트리 값은 다른 설치본을 가리킬 수 있어 쓰지 않는다
//! 2. 사용자 데이터: `%APPDATA%/saba-chan` (settings.json, modules 등)
//! 3. 임시 파일: `%TEMP%/saba-chan-*`
//! 4. 레지스트리: 이 설치 폴더의 `HKCU\...\Uninstall\Saba-chan*` 항목만
//! 5. 바탕화면/시작메뉴 바로가기
//! 6. 자기 자신 (cleanup 스크립트를 통해 프로세스 종료 후 삭제)
use saba_chan_updater_lib::constants;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[cfg(target_os = "windows")]
use crate::registry;
#[cfg(target_os = "windows")]
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

    let install_dir = target_dir();
    match &install_dir {
        Some(dir) => tracing::info!("[Uninstall] Install location: {}", dir.display()),
        None => tracing::warn!("[Uninstall] Not running from an install folder; install files are left alone"),
    }

    // 사용자 데이터는 설치본들이 함께 쓴다: 다른 설치본이 남아 있으면 지우지 않는다
    let last_install = saba_chan_updater_lib::uninstall_entry::is_last_install(
        &saba_chan_updater_lib::uninstall_entry::entries(),
        install_dir.as_deref(),
    );

    // Step 2: 프로세스 종료
    emit("stop", "Stopping saba-chan processes...", 10);
    if let Some(dir) = &install_dir {
        stop_saba_processes(dir);
    }
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
    if !last_install {
        emit("data-shared", "Keeping user data: another Saba-chan install still uses it", 55);
    } else if keep_settings {
        emit("data-keep", "Removing runtime environments (keeping settings)...", 55);
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
    if last_install && !keep_settings {
        emit("modules", "Cleaning modules directory...", 80);
        remove_modules_dir();
    } else {
        emit("modules-keep", "Keeping module settings...", 80);
    }

    // Step 8: 레지스트리 삭제
    emit("registry", "Removing registry entries...", 90);
    #[cfg(target_os = "windows")]
    if let Some(dir) = &install_dir {
        if let Err(e) = registry::remove_uninstall_entry(dir) {
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
    emit_complete(if last_install {
        "Saba-chan has been completely removed."
    } else {
        "This Saba-chan install was removed. Your settings and servers were kept because another Saba-chan install still uses them."
    });
    tracing::info!("[Uninstall] Complete");
}

/// 사일런트(GUI 없이) 제거
pub async fn do_silent_uninstall() {
    eprintln!("Saba-chan Uninstaller (silent mode)");
    let install_dir = target_dir();

    // 프로세스 종료
    eprintln!("  Stopping processes...");
    if let Some(dir) = &install_dir {
        stop_saba_processes(dir);
    }
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
    #[cfg(not(target_os = "windows"))]
    {
        // Linux: .desktop 파일 제거
        if let Some(data_dir) = dirs::data_dir() {
            let desktop_file = data_dir.join("applications/saba-chan.desktop");
            let _ = std::fs::remove_file(desktop_file);
        }
    }

    // 설치 디렉토리 삭제
    if let Some(dir) = &install_dir {
        eprintln!("  Removing install directory: {}", dir.display());
        let _ = remove_dir_robust(dir);
    }

    // 사용자 데이터: 다른 설치본이 남아 있으면 그대로 둔다
    let last_install = saba_chan_updater_lib::uninstall_entry::is_last_install(
        &saba_chan_updater_lib::uninstall_entry::entries(),
        install_dir.as_deref(),
    );
    if last_install {
        eprintln!("  Removing user data and runtime environments...");
        remove_appdata_dir();
    } else {
        eprintln!("  Keeping user data: another Saba-chan install still uses it");
    }

    // 임시 파일
    eprintln!("  Cleaning temp files...");
    remove_temp_files();

    // 모듈
    if last_install {
        eprintln!("  Cleaning modules...");
        remove_modules_dir();
    }

    // 레지스트리
    eprintln!("  Removing registry entries...");
    #[cfg(target_os = "windows")]
    if let Some(dir) = &install_dir {
        let _ = registry::remove_uninstall_entry(dir);
    }

    // 자기 자신 삭제
    eprintln!("  Scheduling self-deletion...");
    {
        schedule_self_delete(install_dir.as_ref());
    }

    if last_install {
        eprintln!("[OK] Saba-chan has been completely removed.");
    } else {
        eprintln!("[OK] This install was removed; settings and servers were kept for the other install.");
    }
}

// ═══════════════════════════════════════════════════════
// 내부 헬퍼
// ═══════════════════════════════════════════════════════

/// 제거할 설치 폴더: 제거기 자신이 든 폴더가 사바쨩 설치 폴더일 때만. 화면에 보이는 경로도 이것이다
pub fn target_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    looks_like_install_dir(&dir).then_some(dir)
}

/// 이 설치 폴더의 실행 파일로 떠 있는 사바쨩 프로세스만 끝낸다 (다른 설치본의 데몬·서버는 그대로)
fn stop_saba_processes(install_dir: &Path) {
    // saba-runner가 게임 서버를 잡(Job Object)에 묶고 있다: 데몬·러너는 트리째 끝내 서버도 끝낸다.
    // GUI·CLI는 트리 없이: GUI가 이 제거기를 자식으로 띄우므로 트리째 끝내면 제거기도 죽는다.
    const TARGETS: [(&str, bool); 6] = [
        ("saba-core", true),
        ("saba-runner", true),
        ("saba-chan-cli", false),
        ("saba-chan-gui", false),
        ("Saba-chan", false), // 0.1.1 Electron GUI
        ("saba-chan-updater", false),
    ];
    let found = processes_in(install_dir, &TARGETS.map(|(n, _)| n));
    // 포터블 GUI(saba-chan-gui.exe)는 실행기일 뿐이고 실제 GUI는 임시 폴더에 풀려 그 자식으로 돈다.
    // 실행기의 자손을 먼저 찾아 둔다 (실행기를 끝내면 부모 관계로 찾을 수 없다)
    let launchers: Vec<u32> =
        found.iter().filter(|(_, n)| n.eq_ignore_ascii_case("saba-chan-gui")).map(|(pid, _)| *pid).collect();
    let unpacked = descendants_of(&launchers);
    for (pid, name) in found {
        let tree = TARGETS.iter().any(|(n, t)| *t && n.eq_ignore_ascii_case(&name));
        tracing::info!("[Uninstall] Stopping {} (PID {})", name, pid);
        kill_pid(pid, tree);
    }
    for pid in unpacked {
        tracing::info!("[Uninstall] Stopping unpacked GUI process (PID {})", pid);
        kill_pid(pid, false);
    }
}

/// `pid|ppid` 줄들에서 `roots`의 자손을 고른다. 제거기 자신(`me`)과 그 아래는 뺀다:
/// 제거기는 GUI가 띄우므로 GUI의 자손이다
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // 포터블 실행기는 Windows에만 있다
fn descendants_in(listing: &str, roots: &[u32], me: u32) -> Vec<u32> {
    let pairs: Vec<(u32, u32)> = listing
        .lines()
        .filter_map(|line| {
            let (pid, ppid) = line.trim().split_once('|')?;
            Some((pid.trim().parse().ok()?, ppid.trim().parse().ok()?))
        })
        .collect();
    let mut found: Vec<u32> = Vec::new();
    let mut frontier: Vec<u32> = roots.to_vec();
    while let Some(parent) = frontier.pop() {
        for &(pid, ppid) in &pairs {
            if ppid == parent && pid != me && pid != parent && !roots.contains(&pid) && !found.contains(&pid) {
                found.push(pid);
                frontier.push(pid);
            }
        }
    }
    found
}

#[cfg(target_os = "windows")]
fn descendants_of(roots: &[u32]) -> Vec<u32> {
    if roots.is_empty() {
        return Vec::new();
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)|$($_.ParentProcessId)\" }";
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) => descendants_in(&String::from_utf8_lossy(&o.stdout), roots, std::process::id()),
        Err(e) => {
            tracing::warn!("[Uninstall] Cannot list child processes: {}", e);
            Vec::new()
        }
    }
}

/// 포터블 실행기는 Windows에만 있다
#[cfg(not(target_os = "windows"))]
fn descendants_of(_roots: &[u32]) -> Vec<u32> {
    Vec::new()
}

/// 다시 설치·덮어쓰기 전에 이 설치 폴더의 데몬을 끈다. 데몬은 인터페이스가 없어도 남아 있으므로
/// 파일이 잠겨 설치가 실패하지 않게 한다. 먼저 `/api/daemon/shutdown`으로 정상 종료를 부탁하고,
/// 20초 안에 안 끝나면 그 PID만 강제 종료한다 (트리째가 아니다: 게임 서버는 러너가 쥐고 있다)
pub async fn stop_daemon_before_install(install_dir: &Path) {
    let running = || processes_in(install_dir, &["saba-core"]);
    if running().is_empty() {
        return;
    }
    tracing::info!("[Install] A daemon from this install is running; asking it to shut down");
    let port = saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get("ipcPort")
        .and_then(|v| v.as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(saba_chan_updater_lib::constants::DEFAULT_IPC_PORT);
    let token = std::fs::read_to_string(saba_chan_updater_lib::constants::token_file_path()).unwrap_or_default();
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build();
    if let Ok(client) = client {
        let sent = client
            .post(format!("http://127.0.0.1:{}/api/daemon/shutdown", port))
            .header("X-Saba-Token", token.trim())
            .json(&serde_json::json!({}))
            .send()
            .await;
        if let Err(e) = sent {
            tracing::warn!("[Install] Shutdown request failed: {}", e);
        }
    }
    for _ in 0..40 {
        if running().is_empty() {
            tracing::info!("[Install] Daemon stopped");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    for (pid, name) in running() {
        tracing::warn!("[Install] {} (PID {}) did not stop; killing it", name, pid);
        kill_pid(pid, false);
    }
}

/// `pid|name|exe` 줄들에서 `install_dir` 안의 실행 파일만 고른다. name은 확장자 뗀 이름
fn filter_processes(listing: &str, install_dir: &Path, names: &[&str]) -> Vec<(u32, String)> {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let root = norm(&install_dir.to_string_lossy());
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, '|');
            let pid = parts.next()?.trim().parse::<u32>().ok()?;
            let name = parts.next()?.trim();
            let name = name.strip_suffix(".exe").unwrap_or(name).to_string();
            let exe = norm(parts.next()?.trim());
            let inside = exe.strip_prefix(&root).is_some_and(|rest| rest.starts_with('/'));
            (inside && names.iter().any(|n| n.eq_ignore_ascii_case(&name))).then_some((pid, name))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn processes_in(install_dir: &Path, names: &[&str]) -> Vec<(u32, String)> {
    let filter = names.iter().map(|n| format!("Name='{}.exe'", n)).collect::<Vec<_>>().join(" OR ");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"{}\" | ForEach-Object {{ \"$($_.ProcessId)|$($_.Name)|$($_.ExecutablePath)\" }}",
        filter
    );
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => filter_processes(&String::from_utf8_lossy(&o.stdout), install_dir, names),
        Err(e) => {
            tracing::warn!("[Uninstall] Cannot list processes: {}", e);
            Vec::new()
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn processes_in(install_dir: &Path, names: &[&str]) -> Vec<(u32, String)> {
    let mut listing = String::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else { continue };
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        listing.push_str(&format!("{}|{}|{}\n", pid, name, exe.display()));
    }
    filter_processes(&listing, install_dir, names)
}

#[cfg(target_os = "windows")]
fn kill_pid(pid: u32, tree: bool) {
    let pid = pid.to_string();
    let mut args = vec!["/PID", pid.as_str()];
    if tree {
        args.push("/T");
    }
    args.push("/F");
    let _ = std::process::Command::new("taskkill")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(target_os = "windows"))]
fn kill_pid(pid: u32, tree: bool) {
    // 게임 서버는 러너·데몬의 자식이고 그 아래 손자가 있을 수 있다: 자식 트리를 맨 아래부터 끝낸다
    let script = if tree {
        "kt() { local c; for c in $(pgrep -P \"$1\"); do kt \"$c\"; done; kill -KILL \"$1\" 2>/dev/null; }; kt \"$0\""
    } else {
        "kill -KILL \"$0\" 2>/dev/null"
    };
    let _ = std::process::Command::new("sh")
        .args(["-c", script, &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 사바쨩이 설치한 폴더인지: 바로 안에 데몬이나 GUI 실행 파일이 있어야 한다
fn looks_like_install_dir(dir: &Path) -> bool {
    let core = format!("saba-core{}", std::env::consts::EXE_SUFFIX);
    dir.join(core).is_file() || saba_chan_updater_lib::find_gui_exe(dir).is_some()
}

/// 디렉토리 강제 삭제 (잠금 파일에 대한 재시도 포함)
fn remove_dir_robust(dir: &PathBuf) -> anyhow::Result<()> {
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

    Ok(())
}

fn remove_appdata_dir() {
    let saba_dir = constants::resolve_data_dir();
    if saba_dir.exists() {
        tracing::info!("[Uninstall] Removing appdata: {:?}", saba_dir);
        let _ = remove_dir_robust(&saba_dir);
    }
}

/// 설정 파일만 남기고 나머지 삭제 (바이너리, 런타임, 캐시 등)
fn remove_appdata_dir_keep_settings() {
    let saba_dir = constants::resolve_data_dir();
    if !saba_dir.exists() {
        return;
    }

    // 보존할 파일/폴더 패턴 (config, settings, 인스턴스 설정)
    let keep_names: &[&str] = &[
        "config.json",       // 모든 사용자 설정 (config_file)
        "config.json.bak",   // 직전 설정 백업
        "instances",         // 인스턴스별 설정 저장
        // 아직 이관되지 않은 예전 설정 파일 (다음 실행 때 config.json으로 이관)
        "settings.json",
        "bot-config.json",
        "cli-settings.json",
        "update-config.json",
        "boot-config.json",
        "extensions_state.json",
        "extensionConfig.json",
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
    let modules_dir = constants::resolve_modules_dir();
    if modules_dir.exists() {
        tracing::info!("[Uninstall] Removing modules: {:?}", modules_dir);
        let _ = remove_dir_robust(&modules_dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unpacked_gui_under_the_portable_launcher_is_found_but_not_the_uninstaller() {
        // 10 = 포터블 실행기, 20 = 임시 폴더의 GUI, 21·22 = GUI 도우미, 30 = 제거기(GUI가 띄움), 31 = 제거기의 자식
        let listing = "10|1\n20|10\n21|20\n22|20\n30|20\n31|30\n40|1\n41|40\n";
        let mut found = descendants_in(listing, &[10], 30);
        found.sort();
        assert_eq!(found, vec![20, 21, 22]);
        assert!(descendants_in(listing, &[], 30).is_empty());
    }

    #[test]
    fn only_a_folder_with_saba_binaries_counts_as_the_install_dir() {
        let dir = std::env::temp_dir().join(format!("saba-uninstall-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!looks_like_install_dir(&dir));
        let ext = if cfg!(windows) { ".exe" } else { "" };
        std::fs::write(dir.join(format!("saba-core{}", ext)), b"").unwrap();
        assert!(looks_like_install_dir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_processes_running_from_this_install_are_stopped() {
        let listing = "10|saba-core.exe|C:\\Saba\\saba-core.exe\n\
                       11|saba-core.exe|C:\\Program Files\\Saba-chan\\saba-core.exe\n\
                       12|saba-runner.exe|c:\\saba\\saba-runner.exe\n\
                       13|saba-core.exe|C:\\Saba-old\\saba-core.exe\n\
                       14|notepad.exe|C:\\Saba\\notepad.exe\n";
        let found = filter_processes(listing, Path::new("C:\\Saba"), &["saba-core", "saba-runner"]);
        assert_eq!(found, vec![(10, "saba-core".to_string()), (12, "saba-runner".to_string())]);
    }
}
