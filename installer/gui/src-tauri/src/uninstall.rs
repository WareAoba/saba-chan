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
        stop_saba_processes(dir).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Step 3: 바로가기 제거
    emit("shortcuts", "Removing shortcuts...", 20);
    #[cfg(target_os = "windows")]
    {
        // 어떤 언어로 설치했든 이 설치를 가리키는 것만 (다른 폴더의 설치 바로가기는 둔다)
        if let Some(dir) = &install_dir {
            shortcuts::remove_shortcuts_into(dir, shortcuts::ALL_SHORTCUT_NAMES);
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
        stop_saba_processes(dir).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 바로가기 제거
    eprintln!("  Removing shortcuts...");
    #[cfg(target_os = "windows")]
    {
        // 어떤 언어로 설치했든 이 설치를 가리키는 것만 (다른 폴더의 설치 바로가기는 둔다)
        if let Some(dir) = &install_dir {
            shortcuts::remove_shortcuts_into(dir, shortcuts::ALL_SHORTCUT_NAMES);
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
async fn stop_saba_processes(install_dir: &Path) {
    // 데몬을 끄기 전에 데몬에 등록된 이 설치의 GUI를 물어 둔다 (임시 폴더에 풀린 포터블 GUI도 여기 있다)
    let dirs = [install_dir.to_path_buf(), comparable_dir(install_dir)];
    let registered = match DaemonApi::from_config() {
        Some(api) => api.registered_guis(&dirs).await,
        None => Vec::new(),
    };
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
    let unpacked = descendants_of(&launchers, install_dir);
    for (pid, name) in found {
        let tree = TARGETS.iter().any(|(n, t)| *t && n.eq_ignore_ascii_case(&name));
        tracing::info!("[Uninstall] Stopping {} (PID {})", name, pid);
        kill_pid(pid, tree);
    }
    for pid in unpacked {
        tracing::info!("[Uninstall] Stopping unpacked GUI process (PID {})", pid);
        kill_pid(pid, false);
    }
    stop_registered_guis(&registered, "[Uninstall]");
}

/// 데몬 API (설정된 포트·토큰 파일)
struct DaemonApi {
    client: reqwest::Client,
    port: u16,
    token: String,
}

impl DaemonApi {
    fn from_config() -> Option<Self> {
        let port = saba_chan_updater_lib::config_file::ConfigFile::default_location()
            .section("gui")
            .get("ipcPort")
            .and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(saba_chan_updater_lib::constants::DEFAULT_IPC_PORT);
        let token = std::fs::read_to_string(saba_chan_updater_lib::constants::token_file_path()).unwrap_or_default();
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build().ok()?;
        Some(Self { client, port, token: token.trim().to_string() })
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// 데몬에 등록된 GUI 중 이 설치 폴더에서 띄운 것의 PID. 데몬이 없거나 옛 버전이면 비어 있다
    async fn registered_guis(&self, dirs: &[PathBuf]) -> Vec<u32> {
        let resp = self.client.get(self.url("/api/clients")).header("X-Saba-Token", &self.token).send().await;
        match resp {
            Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
                Ok(v) => {
                    let pids = guis_of(&v, dirs);
                    tracing::info!("[Install] GUI clients registered from this install: {:?}", pids);
                    pids
                }
                Err(e) => {
                    tracing::info!("[Install] Cannot read the daemon's client list: {}", e);
                    Vec::new()
                }
            },
            Ok(r) => {
                tracing::info!("[Install] The daemon has no client list ({})", r.status());
                Vec::new()
            }
            Err(e) => {
                tracing::info!("[Install] No daemon to ask for GUI clients ({})", e);
                Vec::new()
            }
        }
    }
}

/// `GET /api/clients` 응답에서 이 설치 폴더(`installRoot`)에서 띄운 GUI의 PID
fn guis_of(clients: &serde_json::Value, dirs: &[PathBuf]) -> Vec<u32> {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let mut wanted: Vec<String> = dirs.iter().map(|d| norm(&d.to_string_lossy())).collect();
    wanted.dedup();
    let mut pids: Vec<u32> = clients
        .get("clients")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("gui"))
        .filter_map(|c| {
            let root = c.get("installRoot")?.as_str()?;
            let roots = [norm(root), norm(&comparable_dir(Path::new(root)).to_string_lossy())];
            let pid = u32::try_from(c.get("pid")?.as_u64()?).ok()?;
            roots.iter().any(|r| wanted.contains(r)).then_some(pid)
        })
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// 데몬에 등록돼 있던 GUI를 끈다. 그 PID가 지금도 GUI 이름의 프로세스일 때만 (PID 재사용 대비)
fn stop_registered_guis(pids: &[u32], tag: &str) {
    if pids.is_empty() {
        return;
    }
    let mut names: Vec<&str> = saba_chan_updater_lib::GUI_EXE_NAMES.iter().map(|n| n.trim_end_matches(".exe")).collect();
    if !names.iter().any(|n| n.eq_ignore_ascii_case("Saba-chan")) {
        names.push("Saba-chan");
    }
    let listing = list_named(&names);
    for &pid in pids {
        if pid == std::process::id() {
            continue;
        }
        if pid_listed(&listing, pid) {
            tracing::info!("{} Closing GUI registered with the daemon (PID {})", tag, pid);
            kill_pid(pid, false);
        } else {
            tracing::info!("{} Registered GUI PID {} is already gone", tag, pid);
        }
    }
}

/// 포터블 실행기가 띄운 GUI 프로세스: `pid|ppid|name|exe` 줄들에서 `roots`의 자손 중 GUI 이름(Electron 도우미도
/// 같은 이름)이고 실행 파일이 `places`(이 설치 폴더, 실행기가 풀어 놓는 임시 폴더) 아래인 것만 고른다.
/// GUI가 띄운 브라우저 같은 남의 프로그램과 그 아래로는 내려가지 않는다. 제거기 자신(`me`)도 뺀다
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // 포터블 실행기는 Windows에만 있다
fn descendants_in(listing: &str, roots: &[u32], me: u32, places: &[&Path]) -> Vec<u32> {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let places: Vec<String> = places.iter().map(|p| norm(&p.to_string_lossy())).collect();
    let ours = |name: &str, exe: &str| {
        let name = name.strip_suffix(".exe").unwrap_or(name);
        let exe = norm(exe);
        saba_chan_updater_lib::GUI_EXE_NAMES
            .iter()
            .chain(["Saba-chan.exe"].iter())
            .any(|g| g.trim_end_matches(".exe").eq_ignore_ascii_case(name))
            && places.iter().any(|p| exe.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with('/')))
    };
    let procs: Vec<(u32, u32, bool)> = listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(4, '|');
            let pid = parts.next()?.trim().parse().ok()?;
            let ppid = parts.next()?.trim().parse().ok()?;
            let name = parts.next()?.trim();
            let exe = parts.next().unwrap_or("").trim();
            Some((pid, ppid, ours(name, exe)))
        })
        .collect();
    let mut found: Vec<u32> = Vec::new();
    let mut frontier: Vec<u32> = roots.to_vec();
    while let Some(parent) = frontier.pop() {
        for &(pid, ppid, is_gui) in &procs {
            if ppid == parent && is_gui && pid != me && pid != parent && !roots.contains(&pid) && !found.contains(&pid) {
                found.push(pid);
                frontier.push(pid);
            }
        }
    }
    found
}

#[cfg(target_os = "windows")]
fn descendants_of(roots: &[u32], install_dir: &Path) -> Vec<u32> {
    if roots.is_empty() {
        return Vec::new();
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)|$($_.ParentProcessId)|$($_.Name)|$($_.ExecutablePath)\" }";
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) => {
            let temp = std::env::temp_dir();
            descendants_in(&String::from_utf8_lossy(&o.stdout), roots, std::process::id(), &[install_dir, &temp])
        }
        Err(e) => {
            tracing::warn!("[Uninstall] Cannot list child processes: {}", e);
            Vec::new()
        }
    }
}

/// 포터블 실행기는 Windows에만 있다
#[cfg(not(target_os = "windows"))]
fn descendants_of(_roots: &[u32], _install_dir: &Path) -> Vec<u32> {
    Vec::new()
}

/// 다시 설치·덮어쓰기 전에 이 설치 폴더의 데몬·GUI·CLI를 끈다. 데몬은 인터페이스가 없어도 남아 있으므로
/// 파일이 잠겨 설치가 실패하지 않게 한다 (os error 32). 데몬은 먼저 `/api/daemon/shutdown`으로 정상 종료를
/// 부탁하고, 20초 안에 안 끝나면 그 PID만 강제 종료한다 (트리째가 아니다: 게임 서버는 러너가 쥐고 있다).
/// 단계마다 로그를 남긴다 (`%TEMP%\saba-chan-installer.log`).
pub async fn stop_daemon_before_install(install_dir: &Path) {
    // 받은 경로와 실제 경로 둘 다로 찾는다 (실행 파일 경로는 띄운 경로 그대로 보인다)
    let dir = comparable_dir(install_dir);
    let dirs = [install_dir.to_path_buf(), dir.clone()];
    tracing::info!("[Install] Checking for running Saba-chan processes in {} (real path {})", install_dir.display(), dir.display());
    let listing = list_named(&["saba-core"]);
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        tracing::info!("[Install] saba-core process: {}", line.trim());
    }
    let mut daemons: Vec<u32> = filter_in(&listing, &dirs, &["saba-core"]).into_iter().map(|(pid, _)| pid).collect();

    let Some(api) = DaemonApi::from_config() else {
        return;
    };
    let port = api.port;
    // 데몬을 끄기 전에 이 설치에서 띄운 GUI를 물어 둔다 (임시 폴더에 풀린 포터블 GUI도 여기 있다)
    let registered = api.registered_guis(&dirs).await;

    // 실행 파일 경로로 못 찾았으면 (경로를 못 읽는 프로세스 등) 설정된 포트의 데몬에게 PID를 묻는다
    if daemons.is_empty() {
        let health = api.client.get(api.url("/health")).header("X-Saba-Token", &api.token).send().await;
        let pid = match health {
            Ok(r) => r.json::<serde_json::Value>().await.ok().and_then(|v| v.get("pid").and_then(|p| p.as_u64())),
            Err(e) => {
                tracing::info!("[Install] No daemon answers on port {} ({})", port, e);
                None
            }
        };
        if let Some(pid) = pid.and_then(|p| u32::try_from(p).ok()) {
            match daemon_owner(&listing, pid, &dirs) {
                Owner::Here => daemons.push(pid),
                Owner::Unknown => {
                    tracing::info!("[Install] Daemon PID {} on port {} has no readable path; stopping it to be safe", pid, port);
                    daemons.push(pid);
                }
                Owner::Elsewhere(exe) => {
                    tracing::info!("[Install] Daemon PID {} on port {} belongs to another install ({}); leaving it", pid, port, exe)
                }
            }
        }
    }

    if daemons.is_empty() {
        tracing::info!("[Install] No daemon from this install is running");
    } else {
        tracing::info!("[Install] A daemon from this install is running (PID {:?}); asking it to shut down", daemons);
        let sent = api
            .client
            .post(api.url("/api/daemon/shutdown"))
            .header("X-Saba-Token", &api.token)
            .json(&serde_json::json!({}))
            .send()
            .await;
        match sent {
            Ok(r) => tracing::info!("[Install] Shutdown request answered {}", r.status()),
            Err(e) => tracing::warn!("[Install] Shutdown request failed: {}", e),
        }
        let alive = |daemons: &[u32]| -> Vec<u32> {
            let listing = list_named(&["saba-core"]);
            daemons.iter().copied().filter(|pid| pid_listed(&listing, *pid)).collect()
        };
        let mut left = daemons.clone();
        for _ in 0..40 {
            left = alive(&daemons);
            if left.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if left.is_empty() {
            tracing::info!("[Install] Daemon stopped");
        }
        for pid in left {
            tracing::warn!("[Install] saba-core (PID {}) did not stop; killing it", pid);
            kill_pid(pid, false);
        }
    }

    // GUI·CLI에는 종료 API가 없다: 이 설치의 것만 끈다. 트리째가 아니다: 데몬이 남아 있으면
    // GUI 아래에 데몬·게임 서버가 있을 수 있다 (Electron 렌더러는 본 프로세스가 끝나면 따라 끝난다)
    // 포터블 실행기(saba-chan-gui.exe)의 자손은 실행기를 끄기 전에 찾아 둔다: 실제 GUI는 임시 폴더에 풀려 돈다
    let gui: Vec<&str> = saba_chan_updater_lib::GUI_EXE_NAMES.iter().map(|n| n.trim_end_matches(".exe")).collect();
    let found = filter_in(&list_named(&gui), &dirs, &gui);
    let launchers: Vec<u32> =
        found.iter().filter(|(_, n)| n.eq_ignore_ascii_case("saba-chan-gui")).map(|(pid, _)| *pid).collect();
    let unpacked = descendants_of(&launchers, install_dir);
    for (pid, name) in found {
        tracing::info!("[Install] Closing {} (PID {})", name, pid);
        kill_pid(pid, false);
    }
    for pid in unpacked {
        tracing::info!("[Install] Closing unpacked GUI process (PID {})", pid);
        kill_pid(pid, false);
    }
    stop_registered_guis(&registered, "[Install]");
    for (pid, name) in filter_in(&list_named(&["saba-chan-cli"]), &dirs, &["saba-chan-cli"]) {
        tracing::info!("[Install] Closing {} (PID {})", name, pid);
        kill_pid(pid, false);
    }
}

/// 비교할 설치 경로: 있으면 실제 경로로 (짧은 이름·`..`·대소문자 차이를 없앤다), Windows의 `\\?\` 접두사는 뗀다
fn comparable_dir(dir: &Path) -> std::path::PathBuf {
    match std::fs::canonicalize(dir) {
        Ok(p) => {
            let s = p.to_string_lossy();
            match s.strip_prefix(r"\\?\UNC\") {
                Some(rest) => std::path::PathBuf::from(format!(r"\\{}", rest)),
                None => std::path::PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)),
            }
        }
        Err(_) => dir.to_path_buf(),
    }
}

#[derive(Debug, PartialEq)]
enum Owner {
    Here,
    Unknown,
    Elsewhere(String),
}

/// `pid|name|exe` 목록에서 그 PID의 실행 파일이 이 설치 폴더 안인지
fn daemon_owner(listing: &str, pid: u32, dirs: &[std::path::PathBuf]) -> Owner {
    if filter_in(listing, dirs, &["saba-core"]).iter().any(|(p, _)| *p == pid) {
        return Owner::Here;
    }
    let exe = listing.lines().find_map(|line| {
        let mut parts = line.trim().splitn(3, '|');
        (parts.next()?.trim().parse::<u32>().ok()? == pid).then(|| parts.nth(1).unwrap_or("").trim().to_string())
    });
    match exe {
        Some(exe) if !exe.is_empty() => Owner::Elsewhere(exe),
        _ => Owner::Unknown,
    }
}

/// 여러 경로 표기 중 하나라도 맞는 프로세스 (PID 중복 없이)
fn filter_in(listing: &str, dirs: &[std::path::PathBuf], names: &[&str]) -> Vec<(u32, String)> {
    let mut found: Vec<(u32, String)> = Vec::new();
    for dir in dirs {
        for hit in filter_processes(listing, dir, names) {
            if !found.iter().any(|(p, _)| *p == hit.0) {
                found.push(hit);
            }
        }
    }
    found
}

fn pid_listed(listing: &str, pid: u32) -> bool {
    listing.lines().any(|l| l.trim().split('|').next().and_then(|p| p.trim().parse::<u32>().ok()) == Some(pid))
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

fn processes_in(install_dir: &Path, names: &[&str]) -> Vec<(u32, String)> {
    filter_processes(&list_named(names), install_dir, names)
}

/// 이름이 `names`인 프로세스 목록 (`pid|name|exe` 줄). 경로를 못 읽으면 exe는 빈칸
#[cfg(target_os = "windows")]
fn list_named(names: &[&str]) -> String {
    let filter = names.iter().map(|n| format!("Name='{}.exe'", n)).collect::<Vec<_>>().join(" OR ");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"{}\" | ForEach-Object {{ \"$($_.ProcessId)|$($_.Name)|$($_.ExecutablePath)\" }}",
        filter
    );
    let out = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new("powershell"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(e) => {
            tracing::warn!("[Uninstall] Cannot list processes: {}", e);
            String::new()
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn list_named(names: &[&str]) -> String {
    let mut listing = String::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else { continue };
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        if names.iter().any(|n| n.eq_ignore_ascii_case(stem)) {
            listing.push_str(&format!("{}|{}|{}\n", pid, name, exe.display()));
        }
    }
    listing
}

#[cfg(target_os = "windows")]
fn kill_pid(pid: u32, tree: bool) {
    let pid = pid.to_string();
    let mut args = vec!["/PID", pid.as_str()];
    if tree {
        args.push("/T");
    }
    args.push("/F");
    let _ = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new("taskkill"))
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
        // 10 = 포터블 실행기, 20 = 임시 폴더의 GUI, 21·22 = GUI 도우미, 30 = 제거기(GUI가 띄움), 31 = 제거기의 자식,
        // 50 = GUI가 링크로 띄운 브라우저, 51 = 그 자식, 60 = 다른 폴더에 풀린 같은 이름의 GUI(다른 설치본)
        let listing = "10|1|saba-chan-gui.exe|C:\\Saba\\saba-chan-gui.exe\n\
                       20|10|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       21|20|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       22|20|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       30|20|saba-chan-installer.exe|C:\\Saba\\saba-chan-installer.exe\n\
                       31|30|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       50|20|chrome.exe|C:\\Program Files\\Google\\chrome.exe\n\
                       51|50|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       60|10|Saba-chan.exe|D:\\Other\\Saba-chan.exe\n";
        let places = [Path::new("C:\\Saba"), Path::new("C:\\Temp")];
        let mut found = descendants_in(listing, &[10], 30, &places);
        found.sort();
        assert_eq!(found, vec![20, 21, 22]);
        assert!(descendants_in(listing, &[], 30, &places).is_empty());
    }

    #[test]
    fn only_guis_registered_from_this_install_are_picked() {
        // 1 = 이 설치(임시 폴더에 풀린 포터블 GUI도 installRoot는 설치 폴더), 2 = 다른 설치본, 3 = CLI, 4 = 옛 GUI(정보 없음)
        let clients = serde_json::json!({"clients": [
            {"id": "a", "kind": "gui", "pid": 20, "installRoot": "C:\\Saba\\"},
            {"id": "b", "kind": "gui", "pid": 60, "installRoot": "D:\\Other"},
            {"id": "c", "kind": "cli", "pid": 70, "installRoot": "C:\\Saba"},
            {"id": "d", "kind": "gui", "pid": null, "installRoot": null},
            {"id": "e", "kind": "gui", "pid": 21, "installRoot": "c:/saba"},
        ]});
        let dirs = [PathBuf::from("C:\\Saba")];
        assert_eq!(guis_of(&clients, &dirs), vec![20, 21]);
        assert!(guis_of(&serde_json::json!({}), &dirs).is_empty());
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
    fn the_daemon_on_the_port_is_stopped_only_when_it_is_ours_or_unknown() {
        let listing = "10|saba-core.exe|C:\\Saba\\saba-core.exe\n20|saba-core.exe|D:\\Other\\saba-core.exe\n30|saba-core.exe|\n";
        let dirs = [std::path::PathBuf::from("c:/saba")];
        assert_eq!(daemon_owner(listing, 10, &dirs), Owner::Here);
        assert_eq!(daemon_owner(listing, 20, &dirs), Owner::Elsewhere("D:\\Other\\saba-core.exe".into()));
        assert_eq!(daemon_owner(listing, 30, &dirs), Owner::Unknown);
        assert_eq!(daemon_owner(listing, 99, &dirs), Owner::Unknown);
        assert!(pid_listed(listing, 20) && !pid_listed(listing, 99));
        let two = [std::path::PathBuf::from("C:\\Saba"), std::path::PathBuf::from("c:/SABA/")];
        assert_eq!(filter_in(listing, &two, &["saba-core"]).len(), 1);
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
