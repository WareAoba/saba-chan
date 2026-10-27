//! 사바쨩 완전 제거 (Uninstall)
//!
//! 설치 디렉토리, 저장 폴더(%APPDATA%/saba-chan), 임시 폴더, 레지스트리를 모두 삭제합니다.
//! `--uninstall` 인자로 실행됩니다. 남길 것은 [`keep_from_args`]가 정한다 (기본은 설정·인스턴스 모두 남김).
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

pub use saba_chan_updater_lib::uninstall_entry::Keep;

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
    /// 완료 때 지우지 못하고 남은 항목 (비어 있으면 다 지웠다)
    pub left: Vec<String>,
}

/// 이 설치본이 마지막인지. 제거 항목(레지스트리)과, 항목 없이 설치된 폴더 목록(CLI `install --dir`)을 함께 본다.
/// 지우는 폴더는 목록에서 뺀다
/// `~/.local/bin/saba-chan-cli`가 이 설치 폴더의 CLI를 가리키는 링크면 지운다 (다른 설치를 가리키면 둔다)
#[cfg(unix)]
fn remove_cli_link(home: &Path, install_dir: &Path) -> bool {
    let link = home.join(".local").join("bin").join("saba-chan-cli");
    let Ok(target) = std::fs::read_link(&link) else { return false };
    if !target.parent().is_some_and(|p| saba_chan_updater_lib::same_path(p, install_dir)) {
        return false;
    }
    std::fs::remove_file(&link).is_ok()
}

fn last_install(install_dir: Option<&Path>) -> bool {
    use saba_chan_updater_lib::uninstall_entry as entry;
    let Some(dir) = install_dir else { return false };
    let file = entry::installs_file();
    if let Err(e) = entry::forget_install(&file, dir) {
        tracing::warn!("[Uninstall] Could not update the install list: {}", e);
    }
    entry::is_last_install(&entry::entries(), Some(dir)) && !entry::other_listed_install(&file, dir)
}

/// 제거 인자에서 남길 것. 지우려면 `--remove-settings`·`--remove-instances`를 붙인다
pub fn keep_from_args(args: &[String]) -> Keep {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    Keep { settings: !has("--remove-settings"), instances: !has("--remove-instances") }
}

/// 제거 실행 (창·명령창 공통). 단계마다 `on_progress`로 알리고, 지우지 못하고 남은 항목을 돌려준다
pub async fn do_uninstall(keep: Keep, on_progress: impl Fn(&UninstallProgress)) -> Vec<PathBuf> {
    let emit = |step: &str, msg: &str, pct: i32| {
        let p = UninstallProgress {
            step: step.to_string(),
            message: msg.to_string(),
            percent: pct,
            complete: false,
            error: None,
            left: Vec::new(),
        };
        on_progress(&p);
    };

    let emit_complete = |msg: &str, left: &[PathBuf]| {
        let p = UninstallProgress {
            step: "complete".to_string(),
            message: msg.to_string(),
            percent: 100,
            complete: true,
            error: None,
            left: left.iter().map(|p| p.display().to_string()).collect(),
        };
        on_progress(&p);
    };

    let _emit_err = |msg: &str| {
        let p = UninstallProgress {
            step: "error".to_string(),
            message: msg.to_string(),
            percent: 0,
            complete: false,
            error: Some(msg.to_string()),
            left: Vec::new(),
        };
        on_progress(&p);
    };

    // Step 1: 설치 경로 확인
    emit("detect", "Detecting install location...", 5);

    let install_dir = target_dir();
    match &install_dir {
        Some(dir) => tracing::info!("[Uninstall] Install location: {}", dir.display()),
        None => tracing::warn!("[Uninstall] Not running from an install folder; install files are left alone"),
    }

    // 사용자 데이터는 설치본들이 함께 쓴다: 다른 설치본이 남아 있으면 지우지 않는다
    let last_install = last_install(install_dir.as_deref());

    // Step 2: 프로세스 종료
    emit("stop", "Stopping saba-chan processes...", 10);
    if let Some(dir) = &install_dir {
        stop_saba_processes(dir).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 격리 실행의 프로필과 폴더 권한·표시를 데몬 실행 파일이 거둔다 (설치 폴더를 지우기 전에. 남기는 서버 폴더도)
    let mut left = Vec::new();
    if let Some(dir) = &install_dir {
        emit("sandbox", "Removing isolation profiles and folder permissions...", 15);
        left.extend(reset_sandbox(dir));
    }

    // Step 3: 바로가기 제거
    emit("shortcuts", "Removing shortcuts...", 20);
    #[cfg(target_os = "windows")]
    {
        // 어떤 언어로 설치했든 이 설치를 가리키는 것만 (다른 폴더의 설치 바로가기는 둔다)
        if let Some(dir) = &install_dir {
            shortcuts::remove_shortcuts_into(dir, shortcuts::ALL_SHORTCUT_NAMES);
        }
    }
    // 로그인할 때 자동 실행: 이 설치를 가리키는 것만 (다른 설치본을 켜 둔 것은 둔다)
    if let Some(dir) = &install_dir {
        match saba_chan_updater_lib::autostart::remove_if_in(dir) {
            // 지운 서비스 단위를 systemd가 잊게 한다 (리눅스 화면 없는 서버, 없으면 아무 일도 없다)
            #[cfg(target_os = "linux")]
            Ok(true) => {
                let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("[Uninstall] Failed to remove the sign-in autostart entry: {}", e),
        }
        // install.sh가 만든 CLI 링크 (이 설치를 가리킬 때만)
        #[cfg(unix)]
        if let Some(home) = std::env::var_os("HOME") {
            remove_cli_link(Path::new(&home), dir);
        }
    }

    // Step 4: 설치 디렉토리 삭제
    emit("files", "Removing installation files...", 35);
    let dedicated = install_dir.as_deref().is_some_and(is_dedicated);
    if let Some(ref dir) = install_dir {
        if dir.exists() {
            left.extend(remove_install_dir(dir, dedicated));
            if left.is_empty() {
                tracing::info!("[Uninstall] Removed install dir: {:?}", dir);
            }
        }
    }

    // Step 5: 사용자 데이터 (%APPDATA%/saba-chan). 런타임·캐시는 늘 지우고, 설정·인스턴스는 고른 대로 남긴다
    if !last_install {
        emit("data-shared", "Keeping user data: another Saba-chan install still uses it", 55);
    } else {
        if keep == Keep::NOTHING {
            emit("data", "Removing user data and runtime environments...", 55);
        } else {
            emit("data-keep", "Removing runtime environments (keeping what you chose)...", 55);
        }
        // 데이터 폴더 밖으로 옮긴 모듈·익스텐션 폴더 (설정 파일을 지우기 전에 위치를 읽는다)
        left.extend(remove_moved_component_dirs());
        left.extend(remove_appdata_entries(keep));
    }

    // Step 6: 데이터 폴더 밖의 흔적 (임시 파일·기록, 창 앱 데이터, 리눅스 캐시·바로가기). 무엇을 남기든 지운다.
    // 이 창 제거기의 WebView 폴더는 끝날 때까지 잠겨 있어 정리 스크립트가 지운다
    emit("temp", "Cleaning temporary files...", 70);
    let roots = crate::traces::Roots::current();
    let running: Option<Vec<u32>> = list_all().map(|l| proc_rows(&l).iter().map(|r| r.pid).collect());
    left.extend(crate::traces::remove(&roots, running.as_deref(), cfg!(windows)));

    // Step 8: 레지스트리 삭제
    emit("registry", "Removing registry entries...", 90);
    #[cfg(target_os = "windows")]
    if let Some(dir) = &install_dir {
        if let Err(e) = registry::remove_uninstall_entry(dir) {
            tracing::warn!("Failed to remove registry entry: {}", e);
        }
    }
    // 알림 앱 등록은 설치본들이 함께 쓴다
    if last_install {
        if let Err(e) = saba_chan_updater_lib::notify_app_id::remove(saba_chan_updater_lib::notify_app_id::GUI_APP_ID) {
            tracing::warn!("Failed to remove notification app id: {}", e);
        }
    }

    // Step 9: 자기 자신 삭제 (cleanup 스크립트)
    emit("self-delete", "Scheduling self-deletion...", 95);
    schedule_self_delete(install_dir.as_ref(), dedicated);

    // 완료
    emit_complete(if install_dir.is_none() {
        "No Saba-chan install was found next to this uninstaller, so your settings and servers were kept."
    } else if !left.is_empty() {
        "Saba-chan was removed, but some files could not be deleted."
    } else if !last_install {
        "This Saba-chan install was removed. Your settings and servers were kept because another Saba-chan install still uses them."
    } else {
        kept_message(keep)
    }, &left);
    tracing::info!("[Uninstall] Complete");
    left
}

/// 그 설치의 `saba-core sandbox reset`을 돌린다 (Windows). 거두지 못한 것을 `경로: 이유` 줄로 돌려준다.
/// 큰 서버 폴더는 표시를 덮는 데 시간이 걸려 오래 기다린다
fn reset_sandbox(install_dir: &Path) -> Vec<PathBuf> {
    let core = install_dir.join(saba_chan_updater_lib::uninstall_entry::CORE_EXE);
    if !cfg!(windows) || !core.is_file() {
        return Vec::new();
    }
    let mut cmd = std::process::Command::new(&core);
    cmd.args(["sandbox", "reset"]);
    saba_chan_updater_lib::process::hide_console(&mut cmd);
    let Some(out) = output_within(&mut cmd, std::time::Duration::from_secs(600)) else {
        tracing::warn!("[Uninstall] sandbox reset did not finish");
        return vec![PathBuf::from(format!("{}: sandbox reset did not finish", core.display()))];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    tracing::info!("[Uninstall] sandbox reset: {}", text.trim());
    sandbox_failures(&text, out.status.success(), &core)
}

/// `sandbox reset` 출력의 실패 줄. 실패로 끝났는데 실패 줄이 없으면(명령을 모르는 옛 코어 등) 그 사실을 한 줄로
fn sandbox_failures(text: &str, succeeded: bool, core: &Path) -> Vec<PathBuf> {
    let mut failed: Vec<PathBuf> = text.lines().filter_map(|l| l.strip_prefix("FAIL  ")).map(PathBuf::from).collect();
    if !succeeded && failed.is_empty() {
        failed.push(PathBuf::from(format!("{}: sandbox reset failed", core.display())));
    }
    failed
}

/// 마지막 설치본을 다 지운 뒤 남긴 것을 알리는 문구
fn kept_message(keep: Keep) -> &'static str {
    match (keep.settings, keep.instances) {
        (false, false) => "Saba-chan has been completely removed.",
        (true, false) => "Saba-chan has been removed. Your settings were kept.",
        (false, true) => "Saba-chan has been removed. Your servers were kept.",
        (true, true) => "Saba-chan has been removed. Your settings and servers were kept.",
    }
}

/// 명령창 제거. 창과 같은 단계를 밟고 진행을 표준 오류로 적는다. 지우지 못한 항목이 남으면 1 (프로세스 종료 코드)
pub async fn do_silent_uninstall(keep: Keep) -> i32 {
    eprintln!("Saba-chan Uninstaller (console)");
    let left = do_uninstall(keep, |p| {
        if p.complete {
            for path in &p.left {
                eprintln!("  Could not remove {}", path);
            }
            let tag = if p.left.is_empty() { "[OK]" } else { "[WARN]" };
            eprintln!("{} {}", tag, p.message);
        } else {
            eprintln!("  {}", p.message);
        }
    })
    .await;
    i32::from(!left.is_empty())
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

/// 끈 데몬을 기다리는 상한, 강제 종료한 프로세스를 기다리는 상한
const DAEMON_EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const KILL_EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// 프로세스 목록 조회(PowerShell)·taskkill 한 번의 상한. WMI가 멈춰도 제거기는 멈추지 않는다
const COMMAND_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// 이 설치 폴더의 실행 파일로 떠 있는 사바쨩 프로세스만 끝낸다 (다른 설치본의 데몬·서버는 그대로).
/// 끝까지 끝나지 않은 PID를 돌려준다
async fn stop_saba_processes(install_dir: &Path) -> Vec<u32> {
    // 데몬을 끄기 전에 데몬에 등록된 이 설치의 GUI를 물어 둔다 (임시 폴더에 풀린 포터블 GUI도 여기 있다)
    let dirs = [install_dir.to_path_buf(), comparable_dir(install_dir)];
    let delay = std::time::Duration::from_millis(500);
    let api = DaemonApi::from_config();
    let mut registered = Vec::new();
    let mut orphans: Vec<Found> = Vec::new();
    if let Some(api) = &api {
        // 프로세스를 끊기 전에 이 설치의 데몬에게 켜진 서버를 정상 종료시킨다 (강제 종료는 저장 없이 끈다)
        let ours = api.daemon_is_in(&dirs).await;
        if ours.is_some() {
            api.stop_running_servers().await;
        }
        registered = api.registered_guis(&dirs).await;
        // 데몬도 정상 종료시킨다: 강제로 끊으면 끝나는 사이에 설정·토큰 파일을 다시 쓸 수 있다
        if let Some(health_pid) = ours {
            let mut daemons: Vec<(u32, String)> = processes_in(install_dir, &["saba-core"]);
            if daemons.is_empty() {
                daemons.push((health_pid, "saba-core".to_string()));
            }
            let pids: Vec<u32> = daemons.iter().map(|(pid, _)| *pid).collect();
            // 러너 없이 띄운 서버는 데몬의 자식이다: 데몬이 먼저 끝나면 트리 종료가 닿지 않으므로 지금 찾아 둔다.
            // 목록을 못 읽으면 자식을 모르므로 정상 종료 대신 예전처럼 트리째 끊는다
            match list_all() {
                Some(listing) => {
                    orphans = tree_of(&listing, &pids, std::process::id());
                    let daemons = identify(Some(&listing), &daemons);
                    if api.shutdown("[Uninstall]").await {
                        let left = wait_for_exit(&daemons, list_all, DAEMON_EXIT_WAIT, delay).await;
                        if !left.is_empty() {
                            tracing::warn!("[Uninstall] saba-core {:?} did not stop after the shutdown request", left);
                        }
                    }
                }
                None => {
                    tracing::warn!("[Uninstall] Cannot list the daemon's children; stopping the daemon {:?} with its tree", pids);
                    for pid in pids {
                        kill_pid(pid, true);
                    }
                }
            }
        }
    }
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
    // 명령창 제거(`saba-chan-cli uninstall`)는 이 설치의 CLI 자신이 돌린다: 자기는 끄지 않는다
    let me = std::process::id();
    let found: Vec<(u32, String)> = processes_in(install_dir, &TARGETS.map(|(n, _)| n)).into_iter().filter(|(pid, _)| *pid != me).collect();
    // 포터블 GUI(saba-chan-gui.exe)는 실행기일 뿐이고 실제 GUI는 임시 폴더에 풀려 그 자식으로 돈다.
    // 실행기의 자손을 먼저 찾아 둔다 (실행기를 끝내면 부모 관계로 찾을 수 없다)
    let launchers: Vec<u32> =
        found.iter().filter(|(_, n)| n.eq_ignore_ascii_case("saba-chan-gui")).map(|(pid, _)| *pid).collect();
    let unpacked = descendants_of(&launchers, install_dir);
    // 끈 프로세스를 PID·이름·시작 시각으로 적어 둔다: 끝나기를 기다리는 사이에 PID가 다른 프로그램에 다시 쓰일 수 있다
    let before = list_all();
    let mut stopped: Vec<(u32, bool)> = Vec::new();
    let mut names: Vec<(u32, String)> = Vec::new();
    for (pid, name) in found {
        names.push((pid, name.clone()));
        let tree = TARGETS.iter().any(|(n, t)| *t && n.eq_ignore_ascii_case(&name));
        tracing::info!("[Uninstall] Stopping {} (PID {})", name, pid);
        kill_pid(pid, tree);
        stopped.push((pid, tree));
    }
    // 데몬 아래에 남은 서버: 같은 PID가 아직 같은 이름일 때만 (PID 재사용 대비)
    let orphans = if orphans.is_empty() { Vec::new() } else { list_all().map(|l| still_listed(&l, &orphans)).unwrap_or_default() };
    for (pid, name, _) in orphans {
        if stopped.iter().any(|(p, _)| *p == pid) {
            continue;
        }
        tracing::info!("[Uninstall] Stopping {} left under the daemon (PID {})", name, pid);
        kill_pid(pid, true);
        stopped.push((pid, true));
        names.push((pid, name));
    }
    for pid in unpacked {
        tracing::info!("[Uninstall] Stopping unpacked GUI process (PID {})", pid);
        kill_pid(pid, false);
        stopped.push((pid, false));
        names.push((pid, String::new()));
    }
    for pid in stop_registered_guis(&registered, "[Uninstall]") {
        if !stopped.iter().any(|(p, _)| *p == pid) {
            stopped.push((pid, false));
            names.push((pid, String::new()));
        }
    }

    // 강제 종료는 바로 끝나지 않는다. 끝나기 전에 데이터를 지우면 그 프로세스가 파일을 다시 쓴다.
    // 끝날 때까지 기다리고, 아직 같은 프로세스로 남은 것만 한 번 더 끊는다
    let procs = identify(before.as_deref(), &names);
    let mut left = wait_for_exit(&procs, list_all, KILL_EXIT_WAIT, delay).await;
    if !left.is_empty() {
        let again = list_all().map(|l| still_listed(&l, &left)).unwrap_or_default();
        for (pid, tree) in stopped.iter().filter(|(pid, _)| again.iter().any(|(p, ..)| p == pid)) {
            tracing::warn!("[Uninstall] PID {} is still running; stopping it again", pid);
            kill_pid(*pid, *tree);
        }
        left = wait_for_exit(&left, list_all, KILL_EXIT_WAIT, delay).await;
    }
    let left: Vec<u32> = left.into_iter().map(|(pid, ..)| pid).collect();
    if left.is_empty() {
        tracing::info!("[Uninstall] All stopped processes have exited");
    } else {
        tracing::warn!("[Uninstall] Still running after stop: {:?}", left);
    }
    left
}

/// `procs` 중 `within` 동안 `list`(`list_all` 줄)에 같은 프로세스로 남아 있는 것. 다 끝나면 바로 돌아온다.
/// 목록을 못 읽은 회차(`None`)는 아무것도 끝나지 않은 것으로 센다
async fn wait_for_exit(
    procs: &[Found],
    mut list: impl FnMut() -> Option<String>,
    within: std::time::Duration,
    delay: std::time::Duration,
) -> Vec<Found> {
    let deadline = std::time::Instant::now() + within;
    let mut left = procs.to_vec();
    while !left.is_empty() {
        if let Some(listing) = list() {
            left = still_listed(&listing, &left);
        }
        if left.is_empty() || std::time::Instant::now() + delay >= deadline {
            break;
        }
        tokio::time::sleep(delay).await;
    }
    left
}

/// `list_all`의 한 줄: `pid|ppid|start|name|exe`. start는 시작 시각(모르면 0)
struct ProcRow<'a> {
    pid: u32,
    ppid: u32,
    start: u64,
    name: &'a str,
    exe: &'a str,
}

fn proc_rows(listing: &str) -> Vec<ProcRow<'_>> {
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(5, '|');
            let pid = parts.next()?.trim().parse().ok()?;
            let ppid = parts.next()?.trim().parse().ok()?;
            let start = parts.next()?.trim().parse().unwrap_or(0);
            let name = parts.next()?.trim();
            Some(ProcRow { pid, ppid, start, name, exe: parts.next().unwrap_or("").trim() })
        })
        .collect()
}

/// 데몬 아래에서 찾아 둔 프로세스 (PID, 이름, 시작 시각)
type Found = (u32, String, u64);

/// `roots`의 자손 전부. 제거기 자신(`me`)과 그 아래, 그 위(제거기를 띄운 프로세스)는 뺀다.
/// 부모보다 먼저 시작한 프로세스는 자식으로 치지 않는다: 부모 PID는 이미 끝난 프로세스의 것이 다시 쓰였을 수 있다
fn tree_of(listing: &str, roots: &[u32], me: u32) -> Vec<Found> {
    let procs = proc_rows(listing);
    let mut mine = vec![me];
    let mut cur = me;
    while let Some(row) = procs.iter().find(|r| r.pid == cur) {
        if row.ppid == 0 || mine.contains(&row.ppid) {
            break;
        }
        mine.push(row.ppid);
        cur = row.ppid;
    }
    let mut found: Vec<Found> = Vec::new();
    let mut frontier: Vec<(u32, u64)> =
        procs.iter().filter(|r| roots.contains(&r.pid)).map(|r| (r.pid, r.start)).collect();
    while let Some((parent, since)) = frontier.pop() {
        for r in &procs {
            let child = r.ppid == parent && r.pid != parent && since > 0 && r.start >= since;
            if child && !mine.contains(&r.pid) && !roots.contains(&r.pid) && !found.iter().any(|(p, ..)| *p == r.pid) {
                found.push((r.pid, r.name.to_string(), r.start));
                frontier.push((r.pid, r.start));
            }
        }
    }
    found
}

/// `procs` 중 `listing`에 같은 PID·이름·시작 시각으로 아직 있는 것 (PID가 다른 프로그램에 다시 쓰였으면 뺀다).
/// 이름이 빈칸이거나 시작 시각이 0이면 (적어 둘 때 몰랐으면) 그 항목은 비교하지 않는다
fn still_listed(listing: &str, procs: &[Found]) -> Vec<Found> {
    let stem = |n: &str| n.strip_suffix(".exe").unwrap_or(n).to_ascii_lowercase();
    let rows = proc_rows(listing);
    procs
        .iter()
        .filter(|(pid, name, start)| {
            rows.iter().any(|r| {
                r.pid == *pid && (*start == 0 || r.start == *start) && (name.is_empty() || stem(r.name) == stem(name))
            })
        })
        .cloned()
        .collect()
}

/// 끌 프로세스(PID, 이름)를 `listing`(`list_all`)에서 찾아 시작 시각까지 적는다. 목록에 없으면 시작 시각 0
fn identify(listing: Option<&str>, procs: &[(u32, String)]) -> Vec<Found> {
    let rows = listing.map(proc_rows).unwrap_or_default();
    procs
        .iter()
        .map(|(pid, name)| match rows.iter().find(|r| r.pid == *pid) {
            Some(r) => (*pid, r.name.to_string(), r.start),
            None => (*pid, name.clone(), 0),
        })
        .collect()
}

/// 명령을 돌려 출력을 받는다. `limit` 안에 안 끝나면 끊고 None
fn output_within(cmd: &mut std::process::Command, limit: std::time::Duration) -> Option<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| tracing::warn!("[Uninstall] Cannot run {:?}: {}", cmd.get_program(), e))
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                tracing::warn!("[Uninstall] {:?} did not finish in {:?}; giving up on it", cmd.get_program(), limit);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let stdout = reader.join().unwrap_or_default();
    Some(std::process::Output { status, stdout, stderr: Vec::new() })
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

    /// 설정된 포트에 답하는 데몬의 PID. 답이 없거나 PID를 모르면 None
    async fn health_pid(&self) -> Option<u32> {
        let health = self.client.get(self.url("/health")).header("X-Saba-Token", &self.token).send().await;
        let r = health.map_err(|e| tracing::info!("No daemon answers on port {} ({})", self.port, e)).ok()?;
        let pid = r.json::<serde_json::Value>().await.ok().and_then(|v| v.get("pid").and_then(|p| p.as_u64()));
        pid.and_then(|p| u32::try_from(p).ok())
    }

    /// 설정된 포트의 데몬이 이 설치 폴더의 것이면 그 PID (경로를 모르면 아니라고 본다)
    async fn daemon_is_in(&self, dirs: &[PathBuf]) -> Option<u32> {
        self.health_pid()
            .await
            .filter(|pid| daemon_owner(&list_named(&["saba-core"]).unwrap_or_default(), *pid, dirs) == Owner::Here)
    }

    /// 데몬에 정상 종료를 요청한다. 받아들였는지만 돌려준다 (끝났는지는 부른 쪽이 프로세스로 확인한다)
    async fn shutdown(&self, tag: &str) -> bool {
        let sent = self
            .client
            .post(self.url("/api/daemon/shutdown"))
            .header("X-Saba-Token", &self.token)
            .json(&serde_json::json!({}))
            .send()
            .await;
        match sent {
            Ok(r) => {
                tracing::info!("{} Shutdown request answered {}", tag, r.status());
                r.status().is_success()
            }
            Err(e) => {
                tracing::warn!("{} Shutdown request failed: {}", tag, e);
                false
            }
        }
    }

    /// 꺼져 있지 않은 서버마다 정상 종료를 요청하고 끝날 때까지 기다린다 (서버마다 최대 90초)
    async fn stop_running_servers(&self) {
        let list = self.client.get(self.url("/api/servers")).header("X-Saba-Token", &self.token).send().await;
        let Ok(r) = list else { return };
        let Ok(v) = r.json::<serde_json::Value>().await else { return };
        let ids: Vec<String> = v
            .get("servers")
            .and_then(|s| s.as_array())
            .into_iter()
            .flatten()
            .filter(|s| s.get("status").and_then(|st| st.as_str()).is_some_and(|st| st != "stopped"))
            .filter_map(|s| s.get("id").and_then(|id| id.as_str()).map(String::from))
            .collect();
        let stops = ids.iter().map(|id| {
            tracing::info!("[Uninstall] Stopping server {} before removal", id);
            self.client
                .post(self.url(&format!("/api/instance/{}/stop", id)))
                .header("X-Saba-Token", &self.token)
                .timeout(std::time::Duration::from_secs(90))
                .json(&serde_json::json!({}))
                .send()
        });
        for (id, r) in ids.iter().zip(futures_util::future::join_all(stops).await) {
            match r {
                Ok(r) => tracing::info!("[Uninstall] Stop {} answered {}", id, r.status()),
                Err(e) => tracing::warn!("[Uninstall] Stop {} failed: {}", id, e),
            }
        }
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

/// 데몬에 등록돼 있던 GUI를 끈다. 그 PID가 지금도 GUI 이름의 프로세스일 때만 (PID 재사용 대비). 끈 PID를 돌려준다
fn stop_registered_guis(pids: &[u32], tag: &str) -> Vec<u32> {
    let mut stopped = Vec::new();
    if pids.is_empty() {
        return stopped;
    }
    let mut names: Vec<&str> = saba_chan_updater_lib::GUI_EXE_NAMES.iter().map(|n| n.trim_end_matches(".exe")).collect();
    if !names.iter().any(|n| n.eq_ignore_ascii_case("Saba-chan")) {
        names.push("Saba-chan");
    }
    let listing = list_named(&names).unwrap_or_default();
    for &pid in pids {
        if pid == std::process::id() {
            continue;
        }
        if pid_listed(&listing, pid) {
            tracing::info!("{} Closing GUI registered with the daemon (PID {})", tag, pid);
            kill_pid(pid, false);
            stopped.push(pid);
        } else {
            tracing::info!("{} Registered GUI PID {} is already gone", tag, pid);
        }
    }
    stopped
}

/// 포터블 실행기가 띄운 GUI 프로세스: `list_all` 줄들에서 `roots`의 자손 중 GUI 이름(Electron 도우미도
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
    let procs: Vec<(u32, u32, bool)> = proc_rows(listing).into_iter().map(|r| (r.pid, r.ppid, ours(r.name, r.exe))).collect();
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
    match list_all() {
        Some(listing) => descendants_in(&listing, roots, std::process::id(), &[install_dir, &std::env::temp_dir()]),
        None => {
            tracing::warn!("[Uninstall] Cannot list child processes");
            Vec::new()
        }
    }
}

/// 포터블 실행기는 Windows에만 있다
/// 종료 API는 포트에 답하는 데몬에게 간다: 그 데몬이 이 설치에서 찾은 데몬일 때만 부탁한다
fn asks_to_shut_down(daemons: &[u32], answering: Option<u32>) -> bool {
    answering.is_some_and(|pid| daemons.contains(&pid))
}

#[cfg(not(target_os = "windows"))]
fn descendants_of(_roots: &[u32], _install_dir: &Path) -> Vec<u32> {
    Vec::new()
}

/// 다시 설치·덮어쓰기 전에 이 설치 폴더의 데몬·GUI·CLI를 끈다. 데몬은 인터페이스가 없어도 남아 있으므로
/// 파일이 잠겨 설치가 실패하지 않게 한다 (os error 32). 데몬은 먼저 `/api/daemon/shutdown`으로 정상 종료를
/// 부탁하고, 20초 안에 안 끝나면 그 PID만 강제 종료한다 (트리째가 아니다: 게임 서버는 러너가 쥐고 있다).
/// 단계마다 로그를 남긴다 (`%TEMP%\saba-chan-installer.log`).
/// 포트에 답하는 데몬이 어느 설치의 것인지 모르면 끄지 않고, 사용자가 먼저 끄도록 이유를 돌려준다
pub async fn stop_daemon_before_install(install_dir: &Path) -> Result<(), String> {
    // 받은 경로와 실제 경로 둘 다로 찾는다 (실행 파일 경로는 띄운 경로 그대로 보인다)
    let dir = comparable_dir(install_dir);
    let dirs = [install_dir.to_path_buf(), dir.clone()];
    tracing::info!("[Install] Checking for running Saba-chan processes in {} (real path {})", install_dir.display(), dir.display());
    let listing = list_named(&["saba-core"]).unwrap_or_default();
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        tracing::info!("[Install] saba-core process: {}", line.trim());
    }
    let mut daemons: Vec<u32> = filter_in(&listing, &dirs, &["saba-core"]).into_iter().map(|(pid, _)| pid).collect();

    let Some(api) = DaemonApi::from_config() else {
        return Ok(());
    };
    let port = api.port;
    // 데몬을 끄기 전에 이 설치에서 띄운 GUI를 물어 둔다 (임시 폴더에 풀린 포터블 GUI도 여기 있다)
    let registered = api.registered_guis(&dirs).await;

    // 실행 파일 경로로 못 찾았으면 (경로를 못 읽는 프로세스 등) 설정된 포트의 데몬에게 PID를 묻는다
    if daemons.is_empty() {
        if let Some(pid) = api.health_pid().await {
            let owner = daemon_owner(&listing, pid, &dirs);
            tracing::info!("[Install] Daemon PID {} on port {}: {:?}", pid, port, owner);
            daemons.extend(port_daemon_to_stop(owner, pid)?);
        }
    }

    if daemons.is_empty() {
        tracing::info!("[Install] No daemon from this install is running");
    } else {
        tracing::info!("[Install] A daemon from this install is running (PID {:?}); asking it to shut down", daemons);
        // 설정된 포트(이 설치기의 데이터 폴더 기준)에 다른 설치의 데몬이 답할 수 있다: 찾은 데몬이 답할 때만 부탁한다
        let answering = api.health_pid().await;
        if !asks_to_shut_down(&daemons, answering) {
            tracing::info!("[Install] Port {} is answered by PID {:?}, not this install's daemon; not asking it to shut down", port, answering);
        }
        let left = if asks_to_shut_down(&daemons, answering) && api.shutdown("[Install]").await {
            let named: Vec<(u32, String)> = daemons.iter().map(|pid| (*pid, "saba-core".to_string())).collect();
            let procs = identify(list_all().as_deref(), &named);
            let left = wait_for_exit(&procs, list_all, DAEMON_EXIT_WAIT, std::time::Duration::from_millis(500)).await;
            // 기다리는 사이에 PID가 다른 프로그램에 다시 쓰였을 수 있다: 아직 같은 프로세스인 것만 끊는다
            let left = if left.is_empty() { left } else { list_all().map(|l| still_listed(&l, &left)).unwrap_or_default() };
            left.into_iter().map(|(pid, ..)| pid).collect()
        } else {
            daemons
        };
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
    let found = filter_in(&list_named(&gui).unwrap_or_default(), &dirs, &gui);
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
    for (pid, name) in filter_in(&list_named(&["saba-chan-cli"]).unwrap_or_default(), &dirs, &["saba-chan-cli"]) {
        tracing::info!("[Install] Closing {} (PID {})", name, pid);
        kill_pid(pid, false);
    }
    Ok(())
}

/// 포트에 답하는 데몬 중 설치기가 끌 것: 이 설치의 것만. 다른 설치의 것은 둔다.
/// 어느 설치의 것인지 모르면(실행 파일 경로를 못 읽음) 끄지 않고 사용자가 먼저 끄도록 이유를 돌려준다
fn port_daemon_to_stop(owner: Owner, pid: u32) -> Result<Option<u32>, String> {
    match owner {
        Owner::Here => Ok(Some(pid)),
        Owner::Elsewhere(_) => Ok(None),
        Owner::Unknown => Err(format!(
            "A Saba-chan daemon (PID {}) is running and this installer cannot tell which install it belongs to. Quit Saba-chan first, then try again.",
            pid
        )),
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
    filter_processes(&list_named(names).unwrap_or_default(), install_dir, names)
}

/// 이름이 `names`인 프로세스 목록 (`pid|name|exe` 줄). 경로를 못 읽으면 exe는 빈칸. 목록을 못 읽으면 None
#[cfg(target_os = "windows")]
fn list_named(names: &[&str]) -> Option<String> {
    let filter = names.iter().map(|n| format!("Name='{}.exe'", n)).collect::<Vec<_>>().join(" OR ");
    powershell_lines(&format!(
        "Get-CimInstance Win32_Process -Filter \"{}\" | ForEach-Object {{ \"$($_.ProcessId)|$($_.Name)|$($_.ExecutablePath)\" }}",
        filter
    ))
}

/// 모든 프로세스 목록 (`pid|ppid|start|name|exe` 줄, start는 생성 시각 FILETIME). 목록을 못 읽으면 None
#[cfg(target_os = "windows")]
fn list_all() -> Option<String> {
    powershell_lines(
        "Get-CimInstance Win32_Process | ForEach-Object { $s = if ($_.CreationDate) { $_.CreationDate.ToFileTimeUtc() } else { 0 }; \"$($_.ProcessId)|$($_.ParentProcessId)|$s|$($_.Name)|$($_.ExecutablePath)\" }",
    )
}

#[cfg(target_os = "windows")]
fn powershell_lines(script: &str) -> Option<String> {
    let mut cmd = std::process::Command::new(saba_chan_updater_lib::process::system_tool("powershell"));
    saba_chan_updater_lib::process::hide_console(&mut cmd).args(["-NoProfile", "-NonInteractive", "-Command", script]);
    match output_within(&mut cmd, COMMAND_LIMIT) {
        Some(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
        Some(o) => {
            tracing::warn!("[Uninstall] Cannot list processes (exit {:?})", o.status.code());
            None
        }
        None => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn list_named(names: &[&str]) -> Option<String> {
    let mut listing = String::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else { continue };
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        if names.iter().any(|n| n.eq_ignore_ascii_case(stem)) {
            listing.push_str(&format!("{}|{}|{}\n", pid, name, exe.display()));
        }
    }
    Some(listing)
}

#[cfg(not(target_os = "windows"))]
fn list_all() -> Option<String> {
    let mut listing = String::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // `pid (comm) state ppid ... starttime(22번째)`: comm에 공백·괄호가 있을 수 있어 마지막 `)` 뒤를 읽는다
        let Some((head, rest)) = stat.rsplit_once(')') else { continue };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ppid = fields.get(1).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
        let start = fields.get(19).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
        let comm = head.split_once('(').map(|(_, c)| c).unwrap_or("");
        let exe = std::fs::read_link(entry.path().join("exe")).ok();
        let name = exe.as_ref().and_then(|e| e.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| comm.to_string());
        let exe = exe.map(|e| e.display().to_string()).unwrap_or_default();
        listing.push_str(&format!("{}|{}|{}|{}|{}\n", pid, ppid, start, name, exe));
    }
    Some(listing)
}

#[cfg(target_os = "windows")]
fn kill_pid(pid: u32, tree: bool) {
    let pid = pid.to_string();
    let mut args = vec!["/PID", pid.as_str()];
    if tree {
        args.push("/T");
    }
    args.push("/F");
    let mut cmd = std::process::Command::new(saba_chan_updater_lib::process::system_tool("taskkill"));
    saba_chan_updater_lib::process::hide_console(&mut cmd).args(&args);
    let _ = output_within(&mut cmd, COMMAND_LIMIT);
}

#[cfg(not(target_os = "windows"))]
fn kill_pid(pid: u32, tree: bool) {
    // 게임 서버는 러너·데몬의 자식이고 그 아래 손자가 있을 수 있다: 자식 트리를 맨 아래부터 끝낸다
    let script = if tree {
        "kt() { local c; for c in $(pgrep -P \"$1\"); do kt \"$c\"; done; kill -KILL \"$1\" 2>/dev/null; }; kt \"$0\""
    } else {
        "kill -KILL \"$0\" 2>/dev/null"
    };
    let _ = output_within(std::process::Command::new("sh").args(["-c", script, &pid.to_string()]), COMMAND_LIMIT);
}

/// 설치기가 새로 만들었거나 빈 폴더에 설치했다는 표시. 이 파일이 있을 때만 설치 폴더를 통째로 지운다
pub const INSTALL_MARKER: &str = ".saba-chan-install";

/// 표시가 없는 설치(0.1.4 이전, 또는 사용자 파일이 있던 폴더)에서 지울 바로 안의 항목 (확장자·`.old` 제외한 이름)
const INSTALLED_ENTRIES: &[&str] = &[
    "saba-core",
    "saba-runner",
    "saba-chan-cli",
    "saba-chan-gui",
    "Saba-chan", // 0.1.1 Electron GUI
    "saba-chan-updater",
    "saba-chan-installer",
    "locales",
    "i18n.py",
    "daemon_rcon.py",
    "common_locales.json",
    INSTALL_MARKER,
];

fn is_dedicated(dir: &Path) -> bool {
    dir.join(INSTALL_MARKER).is_file()
}

fn is_installed_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // 업데이터의 백업은 `.old`, 그 이름이 잡혀 있으면 `.old.<번호>`다
    let lower = match lower.rsplit_once('.') {
        Some((head, n)) if head.ends_with(".old") && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => head.to_string(),
        _ => lower,
    };
    let base = lower.strip_suffix(".old").unwrap_or(&lower);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    INSTALLED_ENTRIES.iter().any(|e| e.eq_ignore_ascii_case(base))
}

/// 설치 폴더 정리: 전용 폴더면 통째로, 아니면 사바쨩이 설치한 항목만 지우고 폴더는 비었을 때만 지운다
/// 지우지 못하고 남은 항목을 돌려준다 (실행 중인 제거기 자신은 정리 스크립트가 지우므로 빼고)
fn remove_install_dir(dir: &PathBuf, dedicated: bool) -> Vec<PathBuf> {
    if dedicated {
        let _ = remove_dir_robust(dir);
    } else {
        tracing::info!("[Uninstall] {:?} is not a dedicated install folder: removing Saba-chan files only", dir);
        for path in std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()) {
            if !is_ours(&path) {
                continue;
            }
            if path.is_dir() {
                let _ = remove_dir_robust(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
        let _ = std::fs::remove_dir(dir);
    }
    let left = install_leftovers(dir, dedicated, std::env::current_exe().ok().as_deref());
    for path in &left {
        tracing::warn!("[Uninstall] Could not remove {:?}", path);
    }
    left
}

/// 설치 폴더 바로 안의 사바쨩 항목인지. 폴더는 locales만 사바쨩 것이다 (실행 파일 이름과 같은 사용자 폴더는 남긴다)
fn is_ours(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if path.is_dir() { name.eq_ignore_ascii_case("locales") } else { is_installed_entry(&name) }
}

/// 설치 폴더에 아직 남은 지울 항목: 전용 폴더면 전부, 아니면 사바쨩 항목만. `self_exe`는 뺀다
fn install_leftovers(dir: &Path, dedicated: bool, self_exe: Option<&Path>) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| dedicated || is_ours(p))
        .filter(|p| !self_exe.is_some_and(|me| same_name(me, p)))
        .collect()
}

/// 같은 파일 이름인지 (Windows 파일 이름은 대소문자를 가리지 않는다. 실행할 때 적은 이름이 그대로 `current_exe`에 남는다)
fn same_name(a: &Path, b: &Path) -> bool {
    a.parent() == b.parent()
        && a.file_name().zip(b.file_name()).is_some_and(|(x, y)| x.to_string_lossy().eq_ignore_ascii_case(&y.to_string_lossy()))
}

/// 사바쨩이 설치한 폴더인지: 바로 안에 데몬이나 GUI 실행 파일이 있어야 한다
pub fn looks_like_install_dir(dir: &Path) -> bool {
    let core = format!("saba-core{}", std::env::consts::EXE_SUFFIX);
    dir.join(core).is_file() || saba_chan_updater_lib::find_gui_exe(dir).is_some()
}

/// 디렉토리 강제 삭제 (잠금 파일에 대한 재시도 포함)
pub(crate) fn remove_dir_robust(dir: &PathBuf) -> anyhow::Result<()> {
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

/// 제거기가 지울 사용자 데이터 폴더. 환경 변수로 다른 폴더를 가리키게 했으면 None
/// (개발 체크아웃 같은 남의 폴더일 수 있다)
fn owned_data_dir() -> Option<PathBuf> {
    let overridden = ["SABA_DATA_DIR", "SABA_MODULES_PATH", "SABA_EXTENSIONS_DIR"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    if overridden {
        tracing::warn!("[Uninstall] Data folder is overridden by environment; leaving user data alone");
        return None;
    }
    Some(constants::resolve_data_dir())
}

/// 사용자가 데이터 폴더 밖으로 옮긴 모듈·익스텐션 폴더를 지운다 (모듈·익스텐션은 무엇을 남기든 지우는 런타임 항목이다).
/// 데이터 폴더를 환경 변수로 바꿨으면 건드리지 않는다. 지우지 못한 것을 돌려준다
fn remove_moved_component_dirs() -> Vec<PathBuf> {
    use saba_chan_updater_lib::component_dirs::{resolve, Kind};
    if owned_data_dir().is_none() {
        return Vec::new();
    }
    remove_moved_dirs(&Kind::ALL.map(resolve))
}

/// 설정으로 고른 폴더(`Source::Config`) 중 사바쨩 표시가 있는 것만 지우고, 그래서 빈 상위 폴더도 지운다.
/// 표시가 없는 폴더(손으로 고친 설정이 가리키는 남의 폴더일 수 있다)는 둔다
fn remove_moved_dirs(dirs: &[(PathBuf, saba_chan_updater_lib::component_dirs::Source)]) -> Vec<PathBuf> {
    use saba_chan_updater_lib::component_dirs::{is_owned, Source};
    let mut left = Vec::new();
    for (dir, source) in dirs {
        if *source != Source::Config {
            continue;
        }
        // 끊긴 옮기기가 남긴 복사 중 폴더
        let staging = saba_chan_updater_lib::component_dirs::staging_dir(dir);
        if saba_chan_updater_lib::component_dirs::is_staging(&staging) {
            let _ = remove_dir_robust(&staging);
        }
        if !dir.exists() {
            continue;
        }
        if !is_owned(dir) {
            tracing::info!("[Uninstall] Keeping {:?}: not marked as made by Saba-chan", dir);
            continue;
        }
        tracing::info!("[Uninstall] Removing moved folder: {:?}", dir);
        let _ = remove_dir_robust(dir);
        if dir.exists() {
            left.push(dir.clone());
        } else if let Some(parent) = dir.parent() {
            // 모듈·익스텐션을 두려고 고른 폴더가 비었으면 같이 지운다 (다른 파일이 있으면 실패하고 남는다)
            let _ = std::fs::remove_dir(parent);
        }
    }
    left
}

/// 사용자 데이터 폴더에서 `keep`이 남기지 않는 항목을 지운다 (런타임·캐시는 늘 지운다).
/// 앱이 만들지 않는 항목(업데이트 서명 키 등)은 어느 쪽이든 남는다
/// 지우지 못한 항목을 돌려준다 (몇 번 다시 시도한 뒤에도 남은 것)
fn remove_appdata_entries(keep: Keep) -> Vec<PathBuf> {
    let Some(saba_dir) = owned_data_dir() else { return Vec::new() };
    let removable = |path: &Path| {
        path.file_name()
            .is_some_and(|n| saba_chan_updater_lib::uninstall_entry::removable_data_entry(&n.to_string_lossy(), keep))
    };
    let pending = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| removable(p)).collect()
    };
    for path in std::fs::read_dir(&saba_dir).into_iter().flatten().flatten().map(|e| e.path()) {
        if !removable(&path) {
            tracing::info!("[Uninstall] Keeping: {:?}", path);
        }
    }
    // 잠긴 파일, 아직 끝나는 중인 프로세스가 다시 쓴 파일은 잠시 뒤 다시 지운다
    let mut left = pending(&saba_dir);
    for attempt in 0..3 {
        for path in &left {
            if path.is_dir() {
                tracing::info!("[Uninstall] Removing dir: {:?}", path);
                let _ = remove_dir_robust(path);
            } else {
                tracing::info!("[Uninstall] Removing file: {:?}", path);
                let _ = std::fs::remove_file(path);
            }
        }
        left = pending(&saba_dir);
        if left.is_empty() {
            break;
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    for path in &left {
        tracing::warn!("[Uninstall] Could not remove {:?}", path);
    }
    // 남긴 것이 없으면 폴더도 지운다
    let _ = std::fs::remove_dir(&saba_dir);
    left
}

/// 정리 스크립트를 띄우는 cmd 인자 (스크립트 경로 앞). `/D` AutoRun 끄기, `/V:OFF` 경로의 `!`를 펼치지 않기
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const CLEANUP_CMD_ARGS: [&str; 3] = ["/D", "/V:OFF", "/C"];

/// 제거 후 정리 배치 스크립트. 경로는 배치 파일 안에서도 `%VAR%`가 펼쳐지므로 `%`를 `%%`로 적는다
/// (펼쳐지면 `rmdir /s /q`가 엉뚱한 폴더를 지울 수 있다).
/// 파일은 UTF-8로 쓴다: cmd는 줄마다 지금 코드 페이지로 읽으므로 경로보다 먼저 `chcp 65001`로 바꾼다
/// (그대로면 CP949로 읽어 한글 경로의 del·rmdir가 조용히 실패한다).
/// 마지막 줄은 배치 문맥을 먼저 끝내고(`(goto)`) 자기 자신을 지운다. 그냥 지우면 cmd가 다음 줄을 읽으려다
/// "The batch file cannot be found."로 종료 코드 1을 낸다
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn cleanup_script(
    pid: u32,
    self_exe: &str,
    install_dir: Option<&str>,
    dedicated: bool,
    parent: Option<&str>,
    held: &[(String, bool)],
) -> String {
    let q = |p: &str| p.replace('%', "%%");
    // 제거기가 끝나야 풀리는 흔적 (`traces::held_by_self`: WebView 데이터 폴더, 기록 파일). `(경로, 폴더인가)`
    let extra_lines: String = held
        .iter()
        .map(|(p, dir)| match dir {
            true => format!("if exist \"{0}\" rmdir /s /q \"{0}\"\r\n", q(p)),
            false => format!("del /f /q \"{}\" 2>nul\r\n", q(p)),
        })
        .collect();
    // 전용 폴더가 아니면 비었을 때만 지운다 (사용자 파일이 남아 있을 수 있다)
    let rmdir_line = match install_dir {
        Some(dir) if dedicated => format!("if exist \"{0}\" rmdir /s /q \"{0}\"", q(dir)),
        Some(dir) => format!("rmdir \"{}\" 2>nul", q(dir)),
        None => String::new(),
    };
    // 부모 디렉토리도 정리 시도 (비어있으면 삭제)
    let parent_line = match parent {
        Some(parent) => format!("rmdir \"{}\" 2>nul", q(parent)),
        None => String::new(),
    };
    format!(
        "@echo off\r\n\
        setlocal DisableDelayedExpansion\r\n\
        chcp 65001 >nul\r\n\
        :wait\r\n\
        tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\n\
        if not errorlevel 1 (\r\n\
            timeout /t 1 /nobreak >nul\r\n\
            goto wait\r\n\
        )\r\n\
        del /f /q \"{self_exe}\"\r\n\
        {rmdir_line}\r\n\
        {parent_line}\r\n\
        {extra_lines}\
        (goto) 2>nul & del /f /q \"%~f0\"\r\n",
        self_exe = q(self_exe),
    )
}

/// 프로세스 종료 후 자기 자신(및 남은 설치 디렉토리)을 삭제하는 cleanup 스크립트를 생성·실행.
///
/// Windows에서는 실행 중인 .exe를 직접 삭제할 수 없으므로,
/// 별도 cmd 스크립트를 spawn하여 프로세스 종료를 대기한 뒤 삭제한다.
fn schedule_self_delete(install_dir: Option<&PathBuf>, dedicated: bool) {
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
        // cmd 명령줄에서는 `%`를 이스케이프할 수 없다. 이런 임시 폴더면 남은 파일 정리를 건너뛴다
        if script_path.to_string_lossy().contains('%') {
            tracing::warn!("[Uninstall] Temp path contains '%', skipping cleanup script: {:?}", script_path);
            return;
        }

        let script = cleanup_script(
            pid,
            &self_exe.to_string_lossy(),
            install_dir.map(|d| d.to_string_lossy()).as_deref(),
            dedicated,
            self_exe.parent().map(|p| p.to_string_lossy()).as_deref(),
            &crate::traces::held_by_self(&crate::traces::Roots::current())
                .iter()
                .map(|p| (p.to_string_lossy().into_owned(), p.is_dir()))
                .collect::<Vec<_>>(),
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

        match std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd"))
            .args(CLEANUP_CMD_ARGS)
            .arg(&script_path)
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
            if dedicated {
                let _ = remove_dir_robust(dir);
            } else {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn moved_component_folders_are_removed_only_when_marked() {
        use saba_chan_updater_lib::component_dirs::{mark_owned, Source};
        let tmp = tempfile::tempdir().unwrap();
        let chosen = tmp.path().join("Games").join("Saba");
        let (modules, extensions) = (chosen.join("modules"), chosen.join("extensions"));
        std::fs::create_dir_all(modules.join("minecraft")).unwrap();
        std::fs::write(modules.join("minecraft").join("module.toml"), "").unwrap();
        mark_owned(&modules).unwrap();
        std::fs::create_dir_all(&extensions).unwrap();
        mark_owned(&extensions).unwrap();
        let foreign = tmp.path().join("Other").join("modules");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("mine.txt"), "user").unwrap();
        let staging = saba_chan_updater_lib::component_dirs::staging_dir(&extensions);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join(".saba-chan-moving"), "").unwrap();
        // Codex 리뷰 #544 P1: 표시 없는 같은 이름 폴더는 사용자 것일 수 있어 남긴다
        let unmarked = saba_chan_updater_lib::component_dirs::staging_dir(&foreign);
        std::fs::create_dir_all(&unmarked).unwrap();
        std::fs::write(unmarked.join("mine.txt"), "user").unwrap();
        let default = tmp.path().join("data").join("modules");
        std::fs::create_dir_all(&default).unwrap();
        mark_owned(&default).unwrap();

        let left = remove_moved_dirs(&[
            (modules.clone(), Source::Config),
            (extensions.clone(), Source::Config),
            (foreign.clone(), Source::Config),
            (default.clone(), Source::Default),
        ]);
        assert!(left.is_empty(), "{:?}", left);
        assert!(!chosen.exists(), "the emptied chosen folder goes too");
        assert!(foreign.join("mine.txt").is_file(), "an unmarked folder is kept");
        assert!(unmarked.join("mine.txt").is_file(), "an unmarked folder with the staging name is kept");
        assert!(default.is_dir(), "the data folder entries are left to remove_appdata_entries");
    }

    // M15: 끈 프로세스가 실제로 끝날 때까지 기다린다. 끝나기 전에 사용자 데이터를 지우면
    // 아직 살아 있던 데몬이 설정·토큰 파일을 다시 쓴다
    #[test]
    fn sandbox_reset_failures_are_reported_as_left_items() {
        let core = std::path::Path::new("C:\\Saba\\saba-core.exe");
        let out = "ok    removed 2 isolation profile(s)\n";
        assert!(sandbox_failures(out, true, core).is_empty());
        let out = "FAIL  D:\\Games\\pal: write ACL of D:\\Games\\pal: Access is denied.\nFAIL  saba-chan.untrusted.x: gone\n";
        assert_eq!(sandbox_failures(out, false, core).len(), 2);
        // Codex 리뷰 #539: 실패로 끝났는데 실패 줄이 없으면(명령을 모르는 옛 코어 등) 성공으로 보았다
        assert_eq!(sandbox_failures("usage: saba-core ...\n", false, core).len(), 1);
    }

    /// install.sh가 만든 CLI 링크는 이 설치를 가리킬 때만 지운다
    #[cfg(unix)]
    #[test]
    fn the_cli_link_is_removed_only_when_it_points_into_this_install() {
        let home = tempfile::tempdir().unwrap();
        let ours = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(other.path().join("saba-chan-cli"), bin.join("saba-chan-cli")).unwrap();
        assert!(!remove_cli_link(home.path(), ours.path()));
        assert!(bin.join("saba-chan-cli").symlink_metadata().is_ok());
        std::fs::remove_file(bin.join("saba-chan-cli")).unwrap();
        std::os::unix::fs::symlink(ours.path().join("saba-chan-cli"), bin.join("saba-chan-cli")).unwrap();
        assert!(remove_cli_link(home.path(), ours.path()));
        assert!(bin.join("saba-chan-cli").symlink_metadata().is_err());
    }

    /// 아무것도 정하지 않으면 설정과 인스턴스를 둘 다 남기고, 지울 것만 따로 고른다
    #[test]
    fn uninstall_keeps_settings_and_instances_unless_told_to_remove_each() {
        let args = |a: &[&str]| keep_from_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(args(&["--uninstall"]), Keep::ALL);
        assert_eq!(args(&["--uninstall", "--silent"]), Keep::ALL);
        assert_eq!(args(&["--uninstall", "--remove-settings"]), Keep { settings: false, instances: true });
        assert_eq!(args(&["--uninstall", "--remove-instances"]), Keep { settings: true, instances: false });
        assert_eq!(args(&["--uninstall", "--remove-instances", "--remove-settings"]), Keep::NOTHING);
    }

    /// 어느 설치의 것인지 모르는 데몬은 끄지 않고 사용자에게 먼저 끄라고 안내한다
    #[test]
    fn a_daemon_of_unknown_install_is_left_running_and_explained() {
        assert_eq!(port_daemon_to_stop(Owner::Here, 7), Ok(Some(7)));
        assert_eq!(
            port_daemon_to_stop(Owner::Elsewhere("D:/other/saba-core.exe".into()), 7),
            Ok(None)
        );
        let why = port_daemon_to_stop(Owner::Unknown, 7).unwrap_err();
        assert!(why.contains("PID 7") && why.contains("Quit Saba-chan first"), "{}", why);
    }

    /// 설정된 포트에 다른 설치의 데몬이 답하면 그 데몬에 종료를 부탁하지 않는다 (찾은 PID만 끝낸다)
    #[test]
    fn only_this_installs_daemon_is_asked_to_shut_down() {
        assert!(asks_to_shut_down(&[100], Some(100)));
        assert!(!asks_to_shut_down(&[100], Some(200)), "another install's daemon answers on the port");
        assert!(!asks_to_shut_down(&[100], None));
    }

    #[tokio::test]
    async fn stopped_processes_are_waited_for_until_they_are_gone() {
        let mut calls = 0;
        let procs = [(10, "saba-core.exe".to_string(), 5), (20, "saba-runner.exe".to_string(), 5)];
        let left = wait_for_exit(&procs, || {
            calls += 1;
            Some(if calls < 3 { "10|1|5|saba-core.exe|C:/x/saba-core.exe\n20|1|5|saba-runner.exe|C:/x/saba-runner.exe".into() } else { String::new() })
        }, Duration::from_secs(5), Duration::ZERO)
        .await;
        assert!(left.is_empty());
        assert_eq!(calls, 3);

        let started = Instant::now();
        let left = wait_for_exit(&procs, || Some("20|1|5|saba-runner.exe|C:/x/saba-runner.exe".to_string()), Duration::from_millis(50), Duration::from_millis(5)).await;
        assert_eq!(left, vec![procs[1].clone()], "one still running until the deadline");

        // 리뷰 288: 기다리는 사이 PID가 다른 프로그램에 다시 쓰였으면 끝난 것으로 본다 (다시 끊지 않는다)
        let reused = wait_for_exit(&procs[1..], || Some("20|1|9|explorer.exe|C:/Windows/explorer.exe".to_string()), Duration::from_secs(5), Duration::ZERO).await;
        assert!(reused.is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    // 리뷰 288: 프로세스 목록을 못 읽은 회차는 끝났다고 보지 않는다 (그러면 살아 있는 데몬 옆에서 데이터를 지운다)
    #[tokio::test]
    async fn a_round_without_a_process_list_counts_as_still_running() {
        let procs = [(10, "saba-core.exe".to_string(), 5)];
        let left = wait_for_exit(&procs, || None, Duration::from_millis(30), Duration::from_millis(5)).await;
        assert_eq!(left, procs.to_vec());
    }

    // 리뷰 288: 목록 조회(PowerShell)·taskkill이 멈춰도 제거기가 함께 멈추지 않는다
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_command_that_hangs_is_given_up_on() {
        let started = Instant::now();
        assert!(output_within(std::process::Command::new("sleep").arg("5"), Duration::from_millis(200)).is_none());
        assert!(started.elapsed() < Duration::from_secs(3));
        let out = output_within(std::process::Command::new("echo").arg("hi"), Duration::from_secs(5)).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    }

    // 리뷰 288: 러너 없이 띄운 서버는 데몬의 자식이다. 데몬이 정상 종료하면 트리 종료가 닿지 않으므로
    // 끄기 전에 데몬 아래를 찾아 둔다. 제거기 자신과 그 위(제거기를 띄운 GUI)는 뺀다
    #[test]
    fn the_daemons_children_are_found_before_it_shuts_down() {
        // 100 = 데몬, 110 = 러너 없이 뜬 서버, 111 = 그 자식, 120 = 러너, 121 = 러너의 서버,
        // 200 = 데몬이 띄운 GUI, 300 = 제거기(200이 띄움), 301 = 제거기의 자식, 400 = 남의 프로세스,
        // 500 = 데몬보다 먼저 떠 있던 프로세스(끝난 부모의 PID 100이 데몬에 다시 쓰였다)
        let listing = "100|1|50|saba-core.exe|C:\\Saba\\saba-core.exe\n\
                       110|100|60|java.exe|C:\\Java\\java.exe\n\
                       111|110|61|conhost.exe|C:\\Windows\\conhost.exe\n\
                       120|100|52|saba-runner.exe|C:\\Saba\\saba-runner.exe\n\
                       121|120|53|PalServer.exe|C:\\Games\\PalServer.exe\n\
                       200|100|70|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       300|200|80|saba-chan-installer.exe|C:\\Saba\\saba-chan-installer.exe\n\
                       301|300|81|cmd.exe|C:\\Windows\\cmd.exe\n\
                       400|1|10|notepad.exe|C:\\Windows\\notepad.exe\n\
                       500|100|20|explorer.exe|C:\\Windows\\explorer.exe\n";
        let mut found = tree_of(listing, &[100], 300);
        found.sort();
        let pids: Vec<u32> = found.iter().map(|(p, ..)| *p).collect();
        assert_eq!(pids, vec![110, 111, 120, 121]);

        // 나중에 끌 때는 같은 PID가 같은 프로세스일 때만 (PID가 다른 프로그램에 다시 쓰였으면 둔다)
        let later = "110|100|60|java.exe|C:\\Java\\java.exe\n\
                     111|5|99|conhost.exe|C:\\Windows\\conhost.exe\n\
                     121|5|99|explorer.exe|C:\\Windows\\explorer.exe\n";
        assert_eq!(still_listed(later, &found), vec![(110, "java.exe".to_string(), 60)]);

        // 끌 때 적는 정보: 목록에 있으면 그 이름·시작 시각, 없으면(목록 실패) 아는 이름에 시작 시각 0
        let named = [(110, String::new()), (999, "saba-core".to_string())];
        assert_eq!(identify(Some(listing), &named), vec![(110, "java.exe".to_string(), 60), (999, "saba-core".to_string(), 0)]);
        assert_eq!(still_listed("999|1|42|saba-core.exe|C:\\Saba\\saba-core.exe\n", &identify(None, &named[1..])).len(), 1);
    }

    // 실제 프로세스 목록으로: 데몬 자리의 셸 아래 자식을 찾고, 끈 뒤 끝날 때까지 기다린다
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn a_real_child_is_found_stopped_and_waited_for() {
        let mut sh = std::process::Command::new("sh").args(["-c", "sleep 30; true"]).spawn().unwrap();
        let root = sh.id();
        let mut kids = Vec::new();
        for _ in 0..50 {
            kids = tree_of(&list_all().unwrap(), &[root], std::process::id());
            if !kids.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(kids.iter().any(|(_, name, _)| name == "sleep"), "{:?}", kids);
        kill_pid(root, true);
        let _ = sh.wait();
        let left = wait_for_exit(&kids, list_all, Duration::from_secs(5), Duration::from_millis(50)).await;
        assert!(left.is_empty(), "{:?}", left);
    }

    #[test]
    fn the_cleanup_script_keeps_percent_signs_in_paths_literal() {
        let s = cleanup_script(
            7,
            "C:\\%TEMP%\\Saba-chan\\saba-chan-installer.exe",
            Some("C:\\%TEMP%\\Saba-chan"),
            true,
            Some("C:\\%TEMP%"),
            &[
                ("C:\\Users\\a\\AppData\\Local\\com.saba-chan.installer".to_string(), true),
                ("C:\\%TEMP%\\saba-chan-installer.log".to_string(), false),
            ],
        );
        assert!(s.contains("del /f /q \"C:\\%%TEMP%%\\saba-chan-installer.log\" 2>nul"), "{s}");
        assert!(s.contains("if exist \"C:\\Users\\a\\AppData\\Local\\com.saba-chan.installer\" rmdir /s /q"), "{s}");
        assert!(s.contains("if exist \"C:\\%%TEMP%%\\Saba-chan\" rmdir /s /q \"C:\\%%TEMP%%\\Saba-chan\""));
        assert!(s.contains("del /f /q \"C:\\%%TEMP%%\\Saba-chan\\saba-chan-installer.exe\""));
        assert!(s.contains("rmdir \"C:\\%%TEMP%%\" 2>nul"));
        // 스크립트 자신은 경로를 적지 않고 `%~f0`으로 지운다
        assert!(s.ends_with("(goto) 2>nul & del /f /q \"%~f0\"\r\n"));
        // 전용 폴더가 아니면 비었을 때만 지운다
        let s = cleanup_script(7, "x.exe", Some("D:\\100%\\Saba"), false, None, &[]);
        assert!(s.contains("rmdir \"D:\\100%%\\Saba\" 2>nul"));
        assert!(!s.contains("/s /q \"D:"));
    }

    // M10: 스크립트는 UTF-8로 쓴다. cmd가 경로를 읽기 전에 코드 페이지를 UTF-8로 바꿔야 한글 경로가 맞고,
    // 지연 확장을 꺼야 경로의 `!`가 펼쳐지지 않는다
    #[test]
    fn the_cleanup_script_reads_non_ascii_paths_as_utf8_without_delayed_expansion() {
        let s = cleanup_script(7, "C:\\사바쨩!\\saba-chan-installer.exe", Some("C:\\사바쨩!"), true, None, &[]);
        let first_path = s.find("사바쨩").unwrap();
        let chcp = s.find("chcp 65001").expect("switches cmd to UTF-8");
        assert!(chcp < first_path);
        assert!(s[..first_path].contains("setlocal DisableDelayedExpansion"));
        assert!(s.contains("del /f /q \"C:\\사바쨩!\\saba-chan-installer.exe\""));
        assert!(CLEANUP_CMD_ARGS.contains(&"/V:OFF") && CLEANUP_CMD_ARGS.contains(&"/D"));
    }

    // M10: 실제 cmd로 돌려 한글과 `!`가 든 폴더가 지워지는지 본다 (PC에서 실행)
    #[cfg(target_os = "windows")]
    #[test]
    fn the_cleanup_script_removes_a_non_ascii_folder() {
        let root = std::env::temp_dir().join(format!("saba-cleanup-{}", std::process::id()));
        let dir = root.join("사바쨩!설치");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("saba-chan-installer.exe");
        std::fs::write(&exe, b"x").unwrap();
        // 이미 끝난 프로세스의 PID: 스크립트가 기다리지 않고 바로 지운다
        let mut done = std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd")).args(["/D", "/C", "exit"]).spawn().unwrap();
        let pid = done.id();
        done.wait().unwrap();
        let script = root.join("정리.cmd");
        let body = cleanup_script(pid, &exe.to_string_lossy(), Some(&dir.to_string_lossy()), true, None, &[]);
        std::fs::write(&script, body).unwrap();

        // 실제 제거기처럼 창 없는 콘솔(CREATE_NO_WINDOW)에서 돌려 `chcp 65001`이 먹는지까지 본다
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd"))
            .args(CLEANUP_CMD_ARGS)
            .arg(&script)
            .creation_flags(0x0800_0000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();

        assert!(status.success());
        assert!(!dir.exists(), "the folder is removed");
        assert!(!script.exists(), "the script removes itself");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_unpacked_gui_under_the_portable_launcher_is_found_but_not_the_uninstaller() {
        // 10 = 포터블 실행기, 20 = 임시 폴더의 GUI, 21·22 = GUI 도우미, 30 = 제거기(GUI가 띄움), 31 = 제거기의 자식,
        // 50 = GUI가 링크로 띄운 브라우저, 51 = 그 자식, 60 = 다른 폴더에 풀린 같은 이름의 GUI(다른 설치본)
        let listing = "10|1|5|saba-chan-gui.exe|C:\\Saba\\saba-chan-gui.exe\n\
                       20|10|5|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       21|20|5|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       22|20|5|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       30|20|5|saba-chan-installer.exe|C:\\Saba\\saba-chan-installer.exe\n\
                       31|30|5|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       50|20|5|chrome.exe|C:\\Program Files\\Google\\chrome.exe\n\
                       51|50|5|Saba-chan.exe|C:\\Temp\\2abc\\Saba-chan.exe\n\
                       60|10|5|Saba-chan.exe|D:\\Other\\Saba-chan.exe\n";
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
    fn a_folder_without_the_install_marker_keeps_the_users_files() {
        let dir = std::env::temp_dir().join(format!("saba-uninstall-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        std::fs::create_dir_all(dir.join("MyGame")).unwrap();
        std::fs::create_dir_all(dir.join("Saba-chan")).unwrap();
        for f in ["saba-core.exe", "saba-chan-gui.exe.old", "saba-runner.exe.old.1", "i18n.py", "notes.txt", "notes.old.1"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        assert!(remove_install_dir(&dir, false).is_empty());
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["MyGame", "Saba-chan", "notes.old.1", "notes.txt"]);

        // 전용 표시가 있으면 폴더째 지운다
        std::fs::write(dir.join(INSTALL_MARKER), b"").unwrap();
        assert!(remove_install_dir(&dir, is_dedicated(&dir)).is_empty());
        assert!(!dir.exists());
    }

    // L15 (b): 지우지 못한 설치 파일은 완료 화면에 남은 항목으로 보인다. 실행 중인 제거기 자신은 정리 스크립트가 지운다
    #[test]
    fn install_files_that_could_not_be_removed_are_reported_except_the_running_uninstaller() {
        let dir = std::env::temp_dir().join(format!("saba-uninstall-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        for f in ["saba-core.exe", "saba-chan-installer.exe", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        // 실행할 때 적은 이름의 대소문자가 달라도 제거기 자신이다
        let me = dir.join("SABA-CHAN-INSTALLER.exe");
        let names = |left: Vec<PathBuf>| {
            let mut n: Vec<_> = left.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
            n.sort();
            n
        };
        // 사용자 파일이 섞인 폴더에서는 사바쨩 항목만 남은 것으로 센다
        assert_eq!(names(install_leftovers(&dir, false, Some(&me))), ["locales", "saba-core.exe"]);
        // 전용 폴더는 남은 것 전부
        assert_eq!(names(install_leftovers(&dir, true, Some(&me))), ["locales", "notes.txt", "saba-core.exe"]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(install_leftovers(&dir, true, Some(&me)).is_empty());
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
