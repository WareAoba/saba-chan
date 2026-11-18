/// 릴레이의 새 릴리즈 알림은 데몬이 고른 채널이 그 채널을 받고 지금보다 새 버전일 때만 확인을 돌린다
#[test]
fn update_hint_runs_a_check_only_for_a_newer_release_in_the_chosen_channel() {
    let hint = |version: &str, channel: &str| serde_json::json!({ "version": version, "channel": channel });
    // 베타 채널을 쓰지 않으면 베타 릴리즈는 알리지 않는다
    assert!(!super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "stable", "0.1.12"));
    assert!(!super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "rc", "0.1.12"));
    assert!(super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "beta", "0.1.12"));
    // 위 채널은 아래 채널 릴리즈도 받는다
    assert!(super::hint_wants_check(&hint("0.1.13", "stable"), "beta", "0.1.13-beta.1"));
    // 같거나 낮은 버전, 읽을 수 없는 알림은 무시한다
    assert!(!super::hint_wants_check(&hint("0.1.12", "stable"), "stable", "0.1.12"));
    assert!(!super::hint_wants_check(&hint("0.1.11", "stable"), "stable", "0.1.12"));
    assert!(!super::hint_wants_check(&serde_json::json!({ "channel": "stable" }), "stable", "0.1.12"));
    assert!(!super::hint_wants_check(&hint("0.1.13", "nope"), "stable", "0.1.12"));
}

/// 띄운 업데이터가 바로 끝나면(라이브러리 없음·백신 차단) 데몬이 내려가기 전에 실패로 알린다
#[cfg(unix)]
#[tokio::test]
async fn updater_that_exits_at_once_is_a_launch_failure() {
    let sh = std::path::Path::new("/bin/sh");
    let quick = std::time::Duration::from_millis(400);
    let err = super::spawn_and_confirm(sh, &["-c".to_string(), "exit 127".to_string()], quick).await.unwrap_err();
    assert!(err.to_string().contains("127"), "{err}");
    super::spawn_and_confirm(sh, &["-c".to_string(), "sleep 3".to_string()], quick).await.unwrap();
}

/// 상태 응답은 설치된 번역 판(`locales_version`, 없으면 null)을 늘 싣는다: GUI·TUI가 바뀌면 번역을 다시 읽는다
#[tokio::test]
async fn status_carries_installed_locales_version() {
    use axum::response::IntoResponse;
    let resp = super::get_status(axum::extract::State(super::UpdateState::default())).await.into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.get("locales_version"), Some(&serde_json::json!(crate::chat::i18n::installed_version())));
}

/// 받아 둔 업데이트의 `pending.json`을 같이 쓰는 시험끼리 차례로
static STAGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 받아 둔 뒤 데몬·PC를 다시 켜면 상태가 비어 있다: 다음 실행 때 적용은 디스크의 대기 목록을 되살려 적용한다
#[tokio::test]
async fn applying_on_launch_restores_what_was_downloaded_before_a_restart() {
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join("saba-chan-gui-restore-test.zip");
    std::fs::write(&staged, b"zip").unwrap();
    let pending = json!([{
        "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
        "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-restore-test.zip",
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

    let state = UpdateState::new();
    let mut mgr = state.manager.write().await;
    assert!(mgr.get_pending_components().is_empty(), "a fresh daemon has not read the staging folder yet");
    super::restore_pending(&mut mgr);
    assert_eq!(mgr.get_pending_components().len(), 1);
    std::fs::remove_file(staging.join("pending.json")).unwrap();
    std::fs::remove_file(&staged).unwrap();
}

/// 자동 적용을 끈 설정(받아 두기·알리기만): 받아 둔 앱 업데이트를 적용하지 않고 `ready`(`manual`)로 둔다.
/// 받을 것이 없어지면 그 단계를 걷는다
#[tokio::test]
async fn with_auto_apply_off_the_update_waits_for_a_restart() {
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join("saba-chan-gui-manual-test.zip");
    std::fs::write(&staged, b"zip").unwrap();
    let pending = json!([{
        "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
        "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-manual-test.zip",
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    {
        let mut mgr = state.manager.write().await;
        super::restore_pending(&mut mgr);
        let mut cfg = mgr.get_config();
        cfg.auto_apply = false;
        mgr.update_config(cfg);
    }
    super::core_apply(&state).await;
    let phase = state.phase();
    assert_eq!((phase.state, phase.reason), (PhaseState::Ready, Some(ReadyReason::Manual)));
    assert!(!state.updater_launched());

    std::fs::remove_file(staging.join("pending.json")).unwrap();
    std::fs::remove_file(&staged).unwrap();
    let cfg = state.manager.read().await.get_config();
    *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, tmp.path().to_str().unwrap());
    super::core_apply(&state).await;
    assert_eq!(state.phase(), Phase::default(), "nothing left to apply");
}

/// 사용자가 고른 설치(`/api/updates/install`)는 코어를 바꾸면 끊길 작업이 있으면 업데이터를 띄우지 않고 기다리는 것을 답한다.
/// 그 작업이 끝나면 적용 반복이 이어서 설치한다 (자동 적용을 꺼 두었어도)
#[cfg(unix)]
#[tokio::test]
async fn a_requested_install_waits_for_running_work_and_then_installs() {
    use crate::instance::lock::{InstanceLock, Purpose};
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join("saba-chan-gui-linux-x64.zip");
    std::fs::write(&staged, b"zip").unwrap();
    let pending = json!([{
        "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
        "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-linux-x64.zip",
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();
    let fake = staging.join("fake-updater.sh");
    std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
    unsafe { std::env::set_var("SABA_UPDATER_PATH", &fake) };

    let tmp = tempfile::tempdir().unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let mut state = state_in(tmp.path());
    state.stop_requested = Some(stop.clone());
    {
        let mut mgr = state.manager.write().await;
        super::restore_pending(&mut mgr);
        let mut cfg = mgr.get_config();
        cfg.auto_apply = false;
        mgr.update_config(cfg);
    }
    let id = format!("requested-install-{}", std::process::id());
    let backup = InstanceLock::take(&id).unwrap().purpose(Purpose::Backup);

    let resp = super::install_checked(state.clone()).await;
    let body: Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let waiting = body["waiting"].as_array().expect("the answer names what the install waits for");
    assert!(waiting.iter().any(|w| w["kind"] == "backup" && w["instance_id"] == id.as_str()), "{body}");
    assert!(body.get("updater_launched").is_none(), "{body}");
    assert!(!state.updater_launched(), "the running backup is not cut off");
    assert_eq!(state.phase().state, PhaseState::Waiting);

    // 기다리는 반복은 여기서 세우고, 그 반복의 다음 차례를 직접 돌린다 (30초를 기다리지 않게)
    stop.cancel();
    while state.core_applying.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    drop(backup);
    let next_round = UpdateState { stop_requested: None, ..state.clone() };
    super::core_apply(&next_round).await;
    assert!(state.updater_launched(), "the install goes on once the backup is done, with auto-apply off");

    std::fs::remove_file(staging.join("pending.json")).ok();
    std::fs::remove_file(&staged).ok();
}

#[test]
fn auto_apply_is_kept_with_the_update_settings() {
    let mut cfg = saba_chan_updater_lib::UpdateConfig::default();
    assert!(cfg.auto_apply && cfg.auto_download, "a new install applies updates without asking");
    cfg.auto_apply = false;
    let dir = tempfile::tempdir().unwrap();
    super::save_updater_config_to(&saba_chan_updater_lib::config_file::ConfigFile::at(dir.path()), &cfg).unwrap();
    assert!(!super::load_updater_config_from(&saba_chan_updater_lib::config_file::ConfigFile::at(dir.path())).auto_apply);
}

/// 옛 설정(알리기만)은 한 번만 묻지 않는 자동 적용으로 바꾼다. 그 뒤 사용자가 끈 값은 그대로 둔다
#[test]
fn old_settings_move_to_automatic_updates_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = ConfigFile::at(dir.path());
    file.set_section("updates", json!({ "enabled": true, "auto_download": false, "auto_apply": false, "channel": "beta" })).unwrap();
    let loaded = load_updater_config_from(&file);
    assert!(loaded.auto_apply && loaded.auto_download && loaded.live_update_migrated);
    assert_eq!(loaded.channel, "beta");
    assert_eq!(file.section("updates")["live_update_migrated"], true, "the move is saved");

    let mut off = loaded;
    off.auto_apply = false;
    off.auto_download = false;
    save_updater_config_to(&file, &off).unwrap();
    let again = load_updater_config_from(&file);
    assert!(!again.auto_apply && !again.auto_download, "a later choice is kept");
}

/// 등록된 GUI 중 GUI 실행 파일로 떠 있는 것만 고르고, 업데이터를 띄울 때 끈다
#[cfg(unix)]
#[tokio::test]
async fn launching_the_updater_closes_only_registered_gui_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let gui_exe = tmp.path().join("saba-chan-gui");
    // 리눅스는 sleep을 쓰지 않는다: uutils coreutils(우분투 러너)는 실행 파일 이름이 유틸리티 이름과 다르면 바로 끝난다.
    // 셸 내장 read로 표준 입력이 닫힐 때까지 기다린다. 맥의 /bin/sh는 다른 셸을 다시 띄우는 실행기라 sleep을 쓴다
    #[cfg(target_os = "linux")]
    let (waiter, args): (&str, &[&str]) = ("/bin/sh", &["-c", "read _"]);
    #[cfg(not(target_os = "linux"))]
    let (waiter, args): (&str, &[&str]) = ("/bin/sleep", &["30"]);
    std::fs::copy(waiter, &gui_exe).unwrap();
    let spawn = |exe: &std::path::Path| {
        // 방금 복사한 파일은 다른 테스트가 fork하는 동안 쓰기 핸들이 남아 ETXTBSY가 날 수 있다
        let mut child = (0..50)
            .find_map(|_| match std::process::Command::new(exe).args(args).stdin(std::process::Stdio::piped()).spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                }
                other => Some(other.unwrap()),
            })
            .expect("executable stayed busy");
        let pid = child.id();
        let stdin = child.stdin.take(); // wait가 표준 입력을 닫으면 read가 끝나므로 기다리는 동안 쥐고 있는다
        std::thread::spawn(move || {
            let _stdin = stdin;
            child.wait()
        }); // 끝나면 바로 거둬 좀비로 남지 않게
        pid
    };
    let gui = spawn(&gui_exe);
    let other = spawn(std::path::Path::new(waiter));
    let registry = crate::ipc::ClientRegistry::new();
    registry.register(crate::ipc::ClientKind::Gui, Some(gui), None).await;
    registry.register(crate::ipc::ClientKind::Gui, Some(other), None).await; // PID가 남의 프로그램에 다시 쓰인 경우
    registry.register(crate::ipc::ClientKind::Cli, Some(other), None).await;

    let guis = super::running_guis(&registry.list().await);
    // 맥은 임시 폴더를 링크(/var → /private/var)를 풀지 않은 경로로 알려 준다
    let found: Vec<_> = guis.iter().map(|(pid, exe)| (*pid, std::fs::canonicalize(exe).unwrap())).collect();
    assert_eq!(found, vec![(gui, std::fs::canonicalize(&gui_exe).unwrap())]);

    super::close_guis(&guis).await;
    assert!(!crate::supervisor::process::is_process_alive(gui), "the registered GUI is closed");
    assert!(crate::supervisor::process::is_process_alive(other), "other programs are left alone");
    let _ = crate::supervisor::process::force_kill_pid(other);
}

/// 업데이터를 띄운 뒤 다시 부르면 표를 새로 끊지 않고 409로 거절한다.
/// 표를 새로 끊으면 먼저 띄운 업데이터의 표가 무효가 되어 둘 다 아무것도 적용하지 못한다
#[cfg(unix)]
#[tokio::test]
async fn a_second_launch_is_refused_while_one_is_pending() {
    use axum::extract::State;
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join("saba-chan-gui-linux-x64.zip");
    std::fs::write(&staged, b"zip").unwrap();
    let pending = json!([{
        "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
        "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-linux-x64.zip",
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();
    // 진짜 업데이터처럼 데몬을 기다리며 살아 있다 (바로 끝나면 띄우기 실패로 본다)
    let fake = staging.join("fake-updater.sh");
    std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
    unsafe { std::env::set_var("SABA_UPDATER_PATH", &fake) };

    let state = UpdateState::new();
    state.manager.write().await.load_pending_manifest().unwrap();
    let mut events = crate::events::bus().subscribe(None).rx;
    let first = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    // 데몬이 내려가기 전에 붙은 GUI가 알게 한다: 남는 GUI는 데몬을 띄우지 않고 기다리고, 끌 GUI는 스스로 끝난다
    let swap = std::iter::from_fn(|| events.try_recv().ok())
        .find(|e| e.kind == "update.restarting")
        .expect("the swap is announced on the event stream");
    assert_eq!(swap.data["closingGuis"], json!([]), "no GUI is registered, so none is closed");
    let ticket = std::fs::read_to_string(staging.join("apply-ticket")).unwrap();

    let second = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
    assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);
    let body = axum::body::to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error_code"], "updater_already_launched");
    assert_eq!(std::fs::read_to_string(staging.join("apply-ticket")).unwrap(), ticket, "the first ticket stays valid");
}

/// 업데이트는 설치기(`saba-chan-installer --apply`)가 적용한다. 설치기가 아직 없는 옛 설치(0.1.12 전)는
/// 남아 있는 옛 업데이터로 적용해 그 업데이트로 설치기를 받는다. 옛 설치기(`--apply`를 모르고 설치 창을 연다)만
/// 있는 동안에도 옛 업데이터를 쓴다: 데몬만 먼저 새로 받았거나 설치기 업데이트가 실패한 Windows 설치
#[test]
fn the_installer_applies_updates_and_the_old_updater_is_the_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let daemon = bin.join(exe("saba-core"));
    assert_eq!(super::updater_binary_near(&daemon, true), None);
    std::fs::write(bin.join(exe("saba-chan-updater")), b"").unwrap();
    assert_eq!(super::updater_binary_near(&daemon, true), Some(bin.join(exe("saba-chan-updater"))));
    std::fs::write(tmp.path().join(exe("saba-chan-installer")), b"").unwrap();
    assert_eq!(super::updater_binary_near(&daemon, true), Some(tmp.path().join(exe("saba-chan-installer"))), "one folder up");
    std::fs::write(bin.join(exe("saba-chan-installer")), b"").unwrap();
    assert_eq!(super::updater_binary_near(&daemon, true), Some(bin.join(exe("saba-chan-installer"))));
    assert_eq!(
        super::updater_binary_near(&daemon, false),
        Some(bin.join(exe("saba-chan-updater"))),
        "an old installer cannot apply; the old updater still can"
    );
    std::fs::remove_file(bin.join(exe("saba-chan-updater"))).unwrap();
    assert_eq!(super::updater_binary_near(&daemon, false), Some(bin.join(exe("saba-chan-installer"))), "nothing else to use");
}

/// 설치기가 적용을 맡은 첫 판 이상으로 기록돼 있어야 `--apply`를 안다고 본다. 데몬만 오른 릴리즈 뒤에도 그대로다
#[test]
fn only_an_installer_since_the_merge_counts_as_applying() {
    let manifest = |v: Option<&str>| v.map(|v| [("installer".to_string(), v.to_string())].into_iter().collect()).unwrap_or_default();
    let since = saba_chan_updater_lib::INSTALLER_APPLIES_SINCE;
    assert!(super::installer_applies(&manifest(Some(since)), since));
    assert!(super::installer_applies(&manifest(Some("0.1.12")), since));
    assert!(super::installer_applies(&manifest(Some("0.1.12-beta.3")), since));
    assert!(!super::installer_applies(&manifest(Some("0.1.12-beta.1")), since), "the last installer without --apply");
    assert!(!super::installer_applies(&manifest(Some("0.1.11")), since));
    assert!(!super::installer_applies(&manifest(None), since));
}

/// CLI가 데몬을 다시 띄우라고 해도 GUI가 떠 있었으면 GUI를 띄운다. GUI가 부른 요청은 그대로
#[test]
fn the_updater_relaunches_the_gui_that_was_open() {
    let gui = std::path::PathBuf::from("/opt/saba/saba-chan-gui");
    let guis = vec![(42, gui.clone())];
    let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
    assert_eq!(super::relaunch_target(daemon.clone(), &guis, false), vec![gui.to_string_lossy().into_owned()]);
    assert_eq!(super::relaunch_target(Vec::new(), &guis, false), vec![gui.to_string_lossy().into_owned()]);
    assert_eq!(super::relaunch_target(daemon.clone(), &[], false), daemon);
    let own = vec![format!("/other/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0])];
    assert_eq!(super::relaunch_target(own.clone(), &guis, false), own);
}

/// GUI가 바뀌지 않으면 GUI는 끄지도 다시 띄우지도 않는다: 데몬만 다시 뜨고 떠 있는 GUI가 다시 붙는다.
/// 트레이만 든 GUI는 데몬이 사라지면 따라 끝나므로 예전처럼 끄고 트레이로 다시 띄운다
#[test]
fn only_a_changed_gui_is_closed_and_relaunched() {
    let gui = std::path::PathBuf::from(format!("/opt/saba/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0]));
    let guis = vec![(42, gui.clone())];
    let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
    let from_gui = vec![gui.to_string_lossy().into_owned()];

    let (close, relaunch) = super::restart_plan(false, from_gui.clone(), &guis, false, daemon.clone());
    assert!(close.is_empty(), "an unchanged GUI stays open");
    assert_eq!(relaunch, daemon, "only the daemon comes back");
    let (close, relaunch) = super::restart_plan(false, Vec::new(), &guis, false, daemon.clone());
    assert!(close.is_empty());
    assert_eq!(relaunch, daemon);
    let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string(), "--from-cli".to_string()];
    assert_eq!(super::restart_plan(false, cli.clone(), &[], false, daemon.clone()).1, cli, "a non-GUI request stands");

    let (close, relaunch) = super::restart_plan(true, daemon.clone(), &guis, false, daemon.clone());
    assert_eq!(close, guis, "a changed GUI is closed so its files can be replaced");
    assert_eq!(relaunch, from_gui);
    let (close, relaunch) = super::restart_plan(false, daemon.clone(), &guis, true, daemon.clone());
    assert_eq!(close, guis, "a tray-only GUI would quit with the daemon");
    assert_eq!(relaunch, vec![gui.to_string_lossy().into_owned(), "--tray".to_string()]);
}

/// GUI가 적용 대상일 때만 GUI를 다시 띄운다. 남을 줄 모르는 옛 GUI가 부른 적용은 예전처럼 그 GUI를 다시 띄운다
#[test]
fn an_old_gui_that_always_quits_is_relaunched() {
    let core = vec!["saba-core".to_string()];
    let with_gui = vec!["saba-core".to_string(), "gui".to_string()];
    let gui = vec![format!("/opt/saba/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0])];
    let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
    assert!(!super::gui_must_restart(&core, &gui, true));
    assert!(super::gui_must_restart(&core, &gui, false), "an old GUI quits on success");
    assert!(!super::gui_must_restart(&core, &cli, false), "the CLI does not quit a GUI");
    assert!(super::gui_must_restart(&with_gui, &gui, true));
    assert!(super::gui_must_restart(&with_gui, &cli, false));
}

/// 트레이만 든 GUI는 업데이트 뒤에도 창 없이 트레이로 다시 뜬다 (자동 업데이트마다 창이 튀어나오지 않게)
#[tokio::test]
async fn a_tray_only_gui_is_relaunched_as_a_tray() {
    let gui = std::path::PathBuf::from("/opt/saba/saba-chan-gui");
    let guis = vec![(42, gui.clone())];
    let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
    assert_eq!(
        super::relaunch_target(daemon, &guis, true),
        vec![gui.to_string_lossy().into_owned(), "--tray".to_string()]
    );

    let registry = crate::ipc::ClientRegistry::new();
    assert!(!super::only_resident_guis(&registry.list().await), "no GUI at all is not a tray");
    let tray = registry.register(crate::ipc::ClientKind::Gui, Some(1), None).await;
    registry.register(crate::ipc::ClientKind::Cli, Some(2), None).await;
    assert!(!super::only_resident_guis(&registry.list().await));
    registry.set_resident(&tray, true).await;
    assert!(super::only_resident_guis(&registry.list().await), "CLIs do not count");
    registry.register(crate::ipc::ClientKind::Gui, Some(3), None).await;
    assert!(!super::only_resident_guis(&registry.list().await), "a GUI with a window is relaunched with its window");
}

/// 받지 못했거나 바꾸지 못한 모듈·익스텐션은 다음 정기 확인(하루 뒤)을 기다리지 않고 5분, 30분, 30분 뒤에 다시 해 본다
#[test]
fn a_failed_addon_replacement_is_tried_again_a_few_times_before_the_next_check() {
    use super::{addon_retry_wait, Leftover, ADDON_HOLD_RETRY};
    let minutes = |m: u64| Some(std::time::Duration::from_secs(m * 60));
    let failed = Leftover { held: false, failed: true };
    let mut failures = 0;
    let waits: Vec<_> = (0..4).map(|_| addon_retry_wait(failed, &mut failures)).collect();
    assert_eq!(waits, [minutes(5), minutes(30), minutes(30), None]);

    // 켜진 서버 때문에 미룬 것은 서버가 꺼질 때까지 5분마다 본다. 실패가 섞여도 더 늦추지 않는다
    let mut failures = 0;
    assert_eq!(addon_retry_wait(Leftover { held: true, failed: false }, &mut failures), Some(ADDON_HOLD_RETRY));
    for _ in 0..5 {
        assert_eq!(addon_retry_wait(Leftover { held: true, failed: true }, &mut failures), Some(ADDON_HOLD_RETRY));
    }
    // 잘 끝난 차례 뒤의 실패는 처음부터 센다
    assert_eq!(addon_retry_wait(Leftover::default(), &mut failures), None);
    assert_eq!(addon_retry_wait(failed, &mut failures), minutes(5));
}

/// 실패한 교체를 다시 해 보려고 기다리는 사이에 확인이 또 오면(네트워크를 고치고 '업데이트 확인'을 누름) 남은 잠을 기다리지 않는다
#[tokio::test]
async fn a_new_check_does_not_wait_out_the_retry_delay_of_a_failed_addon_replacement() {
    use super::{replace_addons_until_done, Leftover};
    let recheck = tokio::sync::Notify::new();
    let (ran, mut runs) = tokio::sync::mpsc::unbounded_channel();
    let rounds = replace_addons_until_done(
        || {
            let _ = ran.send(());
            async { Leftover { held: false, failed: true } }
        },
        &recheck,
    );
    let checks = async {
        runs.recv().await.unwrap();
        recheck.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), runs.recv()).await.expect("the replacement ran again right after the check");
    };
    tokio::select! {
        _ = rounds => panic!("the retries ended early"),
        _ = checks => {}
    }
}

/// 헤드리스 설치는 창 없이 적용하고, 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄운다. 데스크톱은 그대로
/// 코어가 낀 묶음에서 업데이터가 새 코어를 기다리게 둔 모듈(완료 마커 `deferred`)은 업데이터가 새 데몬을 다 지켜본 뒤
/// 새 데몬이 마저 바꾼다. 사용자가 고른 묶음이므로 업데이트 확인을 꺼 두었거나 카탈로그 그대로가 아닌 폴더여도 바꾸고,
/// 결과는 그 묶음의 결과(패치노트 포함)에 더한다
#[tokio::test]
async fn an_addon_left_for_the_new_core_is_applied_after_the_updater_stops_watching() {
    use std::io::Write;
    use saba_chan_updater_lib::{live_apply, UpdateCompletionMarker};
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let name = format!("deferred-{}", std::process::id());
    let key = format!("module-{name}");
    let staged = staging.join(format!("{key}.zip"));
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&staged).unwrap());
        zw.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"new").unwrap();
        zw.finish().unwrap();
    }
    let pending = json!([{
        "component": { "module": name }, "current_version": "0.1.0", "latest_version": "0.2.0",
        "update_available": true, "download_url": null, "asset_name": null,
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let module_dir = tmp.path().join("modules").join(&name);
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(module_dir.join("module.toml"), "old, edited by the user").unwrap();
    let state = state_in(tmp.path());
    let cfg = saba_chan_updater_lib::UpdateConfig { enabled: false, ..Default::default() };
    *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, tmp.path().join("modules").to_str().unwrap());
    let release = saba_chan_updater_lib::marker::ReleaseSummary { version: "9.9.9".into(), ..Default::default() };
    UpdateCompletionMarker { deferred: vec![key.clone()], release: Some(release), ..UpdateCompletionMarker::success(vec!["saba-core".into()]) }
        .save()
        .unwrap();

    live_apply::begin_watch(&staging).unwrap();
    let first = super::apply_deferred_once(&state, &staging).await;
    live_apply::end_watch(&staging);
    assert!(first.is_some(), "waits while the updater may still roll the core back");
    assert_eq!(std::fs::read_to_string(module_dir.join("module.toml")).unwrap(), "old, edited by the user");

    assert_eq!(super::apply_deferred_once(&state, &staging).await, None);
    assert_eq!(std::fs::read_to_string(module_dir.join("module.toml")).unwrap(), "new");
    let marker = UpdateCompletionMarker::load().unwrap();
    assert_eq!(marker.updated_components, vec!["saba-core".to_string(), key.clone()]);
    assert!(marker.success && marker.deferred.is_empty(), "{marker:?}");
    assert_eq!(marker.release.map(|r| r.version).as_deref(), Some("9.9.9"), "the notes of the chosen bundle stay");

    let _ = std::fs::remove_file(staging.join("pending.json"));
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_file(UpdateCompletionMarker::path());
}

#[test]
fn a_module_or_extension_a_running_server_uses_waits_for_the_server_to_stop() {
    use saba_chan_updater_lib::Component;
    let busy_modules = std::collections::HashSet::from(["palworld".to_string()]);
    let busy_extensions = std::collections::HashSet::from(["steamcmd".to_string()]);
    let pending = vec![
        Component::Module("palworld".into()),
        Component::Module("minecraft".into()),
        Component::Extension("steamcmd".into()),
        Component::Extension("xna".into()),
    ];
    let (ready, held) = super::split_by_busy(pending, &busy_modules, &busy_extensions);
    assert_eq!(ready, [Component::Module("minecraft".into()), Component::Extension("xna".into())]);
    assert_eq!(held, [Component::Module("palworld".into()), Component::Extension("steamcmd".into())]);
}

#[test]
fn a_headless_install_applies_without_a_window_and_relaunches_the_daemon() {
    use saba_chan_updater_lib::profile::Profile;
    let me = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    let args = super::updater_args("t".into(), Profile::Headless, Vec::new(), None);
    let relaunch = args.iter().position(|a| a == "--relaunch").expect("relaunches something");
    assert!(args[..relaunch].contains(&"--headless".to_string()), "{args:?}");
    assert_eq!(args[relaunch + 1..], [me, "--spawned".to_string()]);

    let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
    let args = super::updater_args("t".into(), Profile::Headless, cli.clone(), None);
    assert_eq!(args[args.len() - 2..], cli[..]);

    let args = super::updater_args("t".into(), Profile::Desktop, Vec::new(), None);
    assert!(!args.iter().any(|a| a == "--headless" || a == "--relaunch"), "{args:?}");

    // 서비스로 돈 데몬은 서비스를 다시 시작한다
    let service = vec!["/usr/bin/systemctl".to_string(), "--user".into(), "start".into(), "saba-chan.service".into()];
    let args = super::updater_args("t".into(), Profile::Headless, Vec::new(), Some(service.clone()));
    assert_eq!(args[args.len() - 4..], service[..]);
    // CLI가 데몬을 다시 띄우라고 해도 서비스가 먼저다
    let args = super::updater_args("t".into(), Profile::Headless, cli.clone(), Some(service.clone()));
    assert_eq!(args[args.len() - 4..], service[..]);
    assert!(!args.contains(&cli[0]), "{args:?}");
}

/// 데몬이 바로 적용한 업데이트(재시작 없는 모듈·익스텐션·설치기)도 결과와 패치노트를 남긴다.
/// 한 클라이언트가 확인해도 결과를 지우지 않아 `?include_seen=true`로 다시 볼 수 있다(CLI `update last`).
/// 매개변수 없는 조회는 예전처럼 확인한 결과를 보지 않는다(확인 뒤 GUI가 다시 띄우지 않게)
#[test]
fn applied_updates_keep_their_notes_after_a_client_acknowledges_them() {
    use saba_chan_updater_lib::marker::ReleaseSummary;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("update-complete.json");
    let release = ReleaseSummary { version: "0.1.8".into(), released_at: None, notes: [("ko".to_string(), "고침".to_string())].into() };
    super::record_daemon_apply(&path, &["module-minecraft".into(), "ext-steamcmd".into()], &["module-minecraft".into()], &["ext-steamcmd: busy".into()], Some(release), false);

    let first = super::last_result_at(&path, false, "ko");
    assert_eq!((first["version"].as_str(), first["notes"].as_str()), (Some("0.1.8"), Some("고침")), "{first}");
    assert_eq!(first["result"]["updated_components"], json!(["module-minecraft"]));
    assert_eq!(first["result"]["failed"], json!(["ext-steamcmd: busy"]));
    assert_eq!(first["result"]["success"], false);
    assert_eq!(first["acknowledged"], false);

    saba_chan_updater_lib::UpdateCompletionMarker::acknowledge_at(&path).unwrap();
    assert!(super::last_result_at(&path, false, "ko")["result"].is_null(), "an acknowledged result is not shown as new");
    let again = super::last_result_at(&path, true, "ko");
    assert_eq!(again["notes"], "고침", "the notes stay readable after a client acknowledged them");
    assert_eq!(again["acknowledged"], true);
}

/// 데몬이 바로 적용한 복구(`update repair`)는 업데이트 알림이 되지 않는다: 확인한 결과로 남아 `update last`만 본다
#[test]
fn a_repair_the_daemon_applied_is_not_announced_as_an_update() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("update-complete.json");
    let release = saba_chan_updater_lib::marker::ReleaseSummary { version: "0.1.8".into(), ..Default::default() };
    super::record_daemon_apply(&path, &["cli".into()], &["cli".into()], &[], Some(release), true);

    assert!(super::last_result_at(&path, false, "ko")["result"].is_null(), "{}", super::last_result_at(&path, false, "ko"));
    let seen = super::last_result_at(&path, true, "ko");
    assert_eq!((seen["result"]["result"].as_str(), seen["result"]["success"].as_bool()), (Some("repaired"), Some(true)), "{seen}");
}

#[test]
fn apply_and_download_accept_a_missing_body_but_not_a_broken_one() {
    let empty: super::ApplyRequest = super::body_or_default(b"").ok().unwrap();
    assert!(empty.components.is_empty());
    let blank: super::DownloadRequest = super::body_or_default(b" \n").ok().unwrap();
    assert!(blank.components.is_empty());
    let some: super::ApplyRequest = super::body_or_default(br#"{"components":["gui"]}"#).ok().unwrap();
    assert_eq!(some.components, vec!["gui".to_string()]);
    assert!(super::body_or_default::<super::ApplyRequest>(b"{oops").is_err(), "a broken body must not mean 'everything'");
}

use super::*;
use saba_chan_updater_lib::relay::SELECTABLE_CHANNELS;

/// 모듈 목록은 업데이트 잠금을 기다리지 않고 마지막으로 알린 상태에서 확인이 찾은 새 판만 읽는다
#[test]
fn the_module_list_reads_the_last_announced_new_version() {
    let state = UpdateState::new();
    assert_eq!(state.announced_latest("module-a"), None, "nothing announced yet");
    *state.announced.lock().unwrap() = Some(json!({ "components": [
        { "component": "module-a", "update_available": true, "latest_version": "1.1.0" },
        { "component": "module-b", "update_available": false, "latest_version": "1.0.0" },
    ] }));
    assert_eq!(state.announced_latest("module-a").as_deref(), Some("1.1.0"));
    assert_eq!(state.announced_latest("module-b"), None);
    assert_eq!(state.announced_latest("module-c"), None);
}

/// 릴리즈 빌드에서는 앱이 업데이트 출처를 바꿀 수 없다 (거부는 저장 전에 끝난다)
#[cfg(not(feature = "mock-relay"))]
#[tokio::test]
async fn the_update_source_cannot_be_changed_from_the_app() {
    let state = UpdateState::new();
    let before = state.manager.read().await.get_config().relay_url;
    let resp = set_config(State(state.clone()), Json(json!({ "relay_url": "https://evil.example" })))
        .await
        .into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    assert_eq!(state.manager.read().await.get_config().relay_url, before);
}

/// 시험용: 상태 파일을 임시 폴더에 둔다
fn state_in(dir: &std::path::Path) -> UpdateState {
    let mut state = UpdateState::new();
    state.live = Arc::default();
    state.live_path = Arc::new(dir.join("live-update.json"));
    state
}

/// 알린 업데이트는 상태 파일에 남는다: 데몬을 다시 켜도 같은 업데이트를 또 알리지 않고, 더 새 판이나 다 받은 뒤의 새 업데이트는 알린다
#[test]
fn an_announced_update_is_not_announced_again_after_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let core = |v: &str| vec![("core".to_string(), Some(v.to_string()))];
    assert!(state_in(tmp.path()).note_updates(core("0.1.14")));
    // 다시 켠 데몬: 상태 파일에서 읽는다
    let restarted = || {
        let state = state_in(tmp.path());
        *state.live.lock().unwrap() = LiveState::load_from(&state.live_path);
        state
    };
    assert!(!restarted().note_updates(core("0.1.14")));
    assert!(restarted().note_updates(core("0.1.15")));
    assert!(!restarted().note_updates(Vec::new()));
    assert!(restarted().note_updates(core("0.1.15")));
}

/// 상태 응답에 알약 단계와 정식 기다림을 싣는다. `done`은 확인(ack)하면 `idle`이 되고 상태 파일에 남는다
#[tokio::test]
async fn the_status_carries_the_phase_and_ack_clears_done() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let body = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await.body };
    let status = body(get_status(State(state.clone())).await.into_response()).await;
    assert_eq!(status["phase"]["state"], "idle");
    assert!(status.as_object().unwrap().contains_key("stable_wait"));

    state.set_phase(Phase::new(PhaseState::Done, Some("0.1.14".into()), None));
    let status = body(get_status(State(state.clone())).await.into_response()).await;
    assert_eq!((status["phase"]["state"].clone(), status["phase"]["version"].clone()), (json!("done"), json!("0.1.14")));
    assert_eq!(LiveState::load_from(&tmp.path().join("live-update.json")).phase.state, PhaseState::Done);

    // ready는 ack로 지워지지 않는다 (다시 시작으로만)
    let acked = body(ack_phase(State(state.clone())).await.into_response()).await;
    assert_eq!(acked["phase"]["state"], "idle");
    state.set_phase(Phase::ready(ReadyReason::RolledBack, Some("0.1.14".into()), None));
    let acked = body(ack_phase(State(state.clone())).await.into_response()).await;
    assert_eq!(acked["phase"]["state"], "ready");
}

/// 업데이트가 데몬 작업만 기다리는 동안에만 새 예약 작업을 미룬다. 켜진 서버를 기다리면 예약 작업은 돈다
#[test]
fn scheduled_work_is_held_only_while_an_update_waits() {
    use crate::supervisor::core_swap::{CoreSwapBlocker, CoreSwapBlockerKind as K};
    let blocker = |kind| CoreSwapBlocker { kind, instance_id: "i".into(), name: "n".into() };
    let waiting = |kinds: &[K]| Phase {
        waiting_for: kinds.iter().map(|k| blocker(*k)).collect(),
        ..Phase::new(PhaseState::Waiting, Some("0.1.14".into()), None)
    };
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    assert!(!state.core_update_waiting());
    state.set_phase(waiting(&[K::Backup, K::ModuleCall]));
    assert!(state.core_update_waiting());
    for server in [K::NonRunnerServer, K::ExtensionBridge, K::RunnerFormat] {
        state.set_phase(waiting(&[K::Backup, server]));
        assert!(!state.core_update_waiting(), "{server:?}");
    }
    state.set_phase(waiting(&[K::ServerUpdate, K::Provision]));
    assert!(state.core_update_waiting());
    state.set_phase(Phase::new(PhaseState::Applying, Some("0.1.14".into()), None));
    assert!(!state.core_update_waiting());
}

/// 같은 heartbeat 판 번호는 다시 확인하지 않는다. 본 것으로 남기는 것은 확인이 성공한 뒤다:
/// 확인 중에는 같은 번호로 또 묻지 않고, 실패·취소·꺼짐이면 다음 heartbeat가 다시 확인한다
#[test]
fn release_ids_count_as_seen_only_after_a_successful_check() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64)]);
    assert!(state.release_ids_to_check(std::collections::BTreeMap::new()).is_none());
    let checking = state.release_ids_to_check(ids.clone()).expect("new ids are checked");
    assert!(state.release_ids_to_check(ids.clone()).is_none(), "already being checked");
    state.finish_release_check(checking, None);
    assert!(LiveState::load_from(&tmp.path().join("live-update.json")).seen.is_empty());
    let checking = state.release_ids_to_check(ids.clone()).expect("a failed check is tried again");
    state.finish_release_check(checking, Some(Some(12)));
    assert!(state.release_ids_to_check(ids).is_none());
    let newer = std::collections::BTreeMap::from([("stable".to_string(), 13u64)]);
    let checking = state.release_ids_to_check(newer).unwrap();
    state.finish_release_check(checking, Some(Some(14)));
    assert_eq!(LiveState::load_from(&tmp.path().join("live-update.json")).seen["stable"], 13);
}

/// heartbeat `latestAddons`: 이 데몬 채널의 값이 바뀌면 확인한다. 본 것으로 남기는 것은 확인이 성공한 뒤다
#[test]
fn addon_revision_counts_as_seen_only_after_a_successful_check() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let revs = serde_json::json!({ "stable": "aaa", "beta": "bbb" });
    assert!(state.addon_revision_to_check(&serde_json::json!({}), "beta").is_none(), "no value for the channel");
    let checking = state.addon_revision_to_check(&revs, "beta").expect("a new revision is checked");
    assert_eq!(checking, "bbb");
    assert!(state.addon_revision_to_check(&revs, "beta").is_none(), "already being checked");
    state.finish_addon_check(checking, false);
    let checking = state.addon_revision_to_check(&revs, "beta").expect("a failed check is tried again");
    state.finish_addon_check(checking, true);
    assert!(state.addon_revision_to_check(&revs, "beta").is_none());
    assert_eq!(LiveState::load_from(&tmp.path().join("live-update.json")).addons_seen.as_deref(), Some("bbb"));
    assert!(state.addon_revision_to_check(&serde_json::json!({ "beta": "ccc" }), "beta").is_some(), "a new publish");
}

/// heartbeat가 부르지 않은 확인도 확인을 마친 가장 큰 릴리즈 id를 캐시 키로 쓴다
#[test]
fn other_checks_reuse_the_largest_confirmed_release_id() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    assert_eq!(state.known_release_id(), None);
    let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64), ("beta".to_string(), 15u64)]);
    let checking = state.release_ids_to_check(ids).unwrap();
    state.finish_release_check(checking, Some(Some(15)));
    assert_eq!(state.known_release_id(), Some(15));
}

/// 확인했는데 응답에 알린 번호가 없으면(캐시된 옛 응답) 본 것으로 두지 않고 캐시 수명 뒤 한 번 더 본다.
/// 그때도 없으면(이 플랫폼 몫이 아님) 본 것으로 둔다: 한 번호를 두 번 넘게 묻지 않는다
#[test]
fn a_release_id_missing_from_the_answer_is_checked_once_more_later() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let seen = || LiveState::load_from(&tmp.path().join("live-update.json")).seen;
    let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64)]);
    let checking = state.release_ids_to_check(ids.clone()).unwrap();
    state.finish_release_check(checking, Some(Some(11)));
    assert!(seen().is_empty(), "a stale answer does not count");
    assert!(state.release_ids_to_check(ids.clone()).is_none(), "not again within the cache lifetime");
    // 캐시 수명이 지났다
    state.release_unconfirmed.lock().unwrap().as_mut().unwrap().1 = std::time::Instant::now() - RELEASE_RECHECK;
    let checking = state.release_ids_to_check(ids.clone()).expect("checked once more");
    state.finish_release_check(checking, Some(None));
    assert_eq!(seen(), ids, "the second miss counts as seen");
    assert!(state.release_ids_to_check(ids).is_none());
}

/// 적용 반복이 도는 중에 들어온 확인은 버려지지 않는다: 반복이 다음 차례에 새로 받고, 그 뒤에 왔으면 끝난 뒤 다시 돈다
#[test]
fn a_check_during_the_apply_loop_leaves_a_recheck() {
    let state = UpdateState::new();
    assert!(state.begin_core_apply(), "nothing running: start");
    assert!(state.take_apply_recheck(), "the first round downloads");
    assert!(!state.take_apply_recheck());
    assert!(!state.begin_core_apply(), "one loop at a time");
    assert!(state.take_apply_recheck(), "the running loop picks up the new check");
    assert!(!state.end_core_apply(), "nothing came after the last round");
    assert!(state.begin_core_apply());
    assert!(!state.begin_core_apply());
    let _ = state.take_apply_recheck();
    assert!(!state.begin_core_apply());
    assert!(state.end_core_apply(), "a check after the last round runs the loop again");
    assert!(state.begin_core_apply());
}

/// 다른 폴더의 GUI(미리보기 빌드 등)가 붙어도 데몬이 설치된 구성요소를 찾는 폴더는 바뀌지 않는다
#[tokio::test]
async fn a_client_cannot_move_where_the_daemon_looks_for_installed_parts() {
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join(saba_chan_updater_lib::GUI_EXE), b"exe").unwrap();
    let state = UpdateState::new();
    let before = state.manager.read().await.is_component_installed(&Component::Gui);
    let _ = set_config(State(state.clone()), Json(json!({ "install_root": other.path().to_string_lossy() }))).await.into_response();
    assert_eq!(state.manager.read().await.is_component_installed(&Component::Gui), before);
}

/// 예전 GUI가 config.json에 남긴 설치 폴더는 읽지도 다시 쓰지도 않는다
#[test]
fn an_install_root_left_in_config_json_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let file = ConfigFile::at(dir.path());
    file.set_section("updates", json!({ "install_root": "D:\\SabaPreview\\exe", "channel": "beta" })).unwrap();
    let loaded = load_updater_config_from(&file);
    assert_eq!(loaded.install_root, None);
    save_updater_config_to(&file, &loaded).unwrap();
    assert!(file.section("updates").get("install_root").is_none());
}

/// 설정을 디스크에 저장하지 못하면 관리자 설정도 바꾸지 않고 500 `save_failed`로 답한다
#[tokio::test]
async fn an_update_setting_that_cannot_be_saved_is_not_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("file");
    std::fs::write(&not_a_dir, b"x").unwrap();
    let state = UpdateState::new();
    let mut mgr = state.manager.write().await;
    let before = mgr.get_config().auto_apply;
    let mut cfg = mgr.get_config();
    cfg.auto_apply = !before;
    let resp = adopt_config(&mut mgr, &ConfigFile::at(&not_a_dir), cfg).unwrap_err();
    assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(mgr.get_config().auto_apply, before);
}

/// 형식이 틀린 값이나 바꿀 수 없는 키는 말없이 넘기지 않고 거절한다. GUI가 보내는 모양은 그대로 받는다
#[test]
fn a_wrong_typed_or_unknown_update_config_key_is_refused() {
    let refused = |body: Value| {
        let mut cfg = UpdateConfig::default();
        let before = serde_json::to_value(&cfg).unwrap();
        let err = apply_config_patch(&mut cfg, &body).expect_err("must be refused");
        assert_eq!(serde_json::to_value(&cfg).unwrap(), before, "nothing changes on a refusal");
        (err.0, err.1["error_code"].as_str().unwrap_or_default().to_string())
    };
    let bad = (axum::http::StatusCode::BAD_REQUEST, "invalid_update_config_value".to_string());
    assert_eq!(refused(json!({ "auto_apply": "off" })), bad);
    assert_eq!(refused(json!({ "enabled": 1 })), bad);
    assert_eq!(refused(json!({ "auto_download": true, "channel": 1 })), bad);
    let unknown = (axum::http::StatusCode::BAD_REQUEST, "unknown_update_config_key".to_string());
    for key in ["failed_release", "user_held", "live_update_migrated", "install_root"] {
        assert_eq!(refused(json!({ key: "x" })), unknown, "{key}");
    }

    let mut cfg = UpdateConfig::default();
    apply_config_patch(&mut cfg, &json!({ "auto_download": true, "auto_apply": false })).unwrap();
    apply_config_patch(&mut cfg, &json!({ "enabled": false, "channel": "beta", "relay_url": "" })).unwrap();
    assert_eq!((cfg.enabled, cfg.auto_download, cfg.auto_apply, cfg.channel.as_str()), (false, true, false, "beta"));
}

/// 모르는 채널은 저장하지 않는다
#[tokio::test]
async fn an_unknown_update_channel_is_refused() {
    let state = UpdateState::new();
    let before = state.manager.read().await.get_config().channel;
    let resp = set_config(State(state.clone()), Json(json!({ "channel": "nightly" }))).await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(state.manager.read().await.get_config().channel, before);
    assert!(SELECTABLE_CHANNELS.iter().all(|c| saba_chan_updater_lib::relay::CHANNELS.contains(c)));
}

/// 적용 결과에 릴리즈 버전과 고른 언어의 패치노트가 붙는다. 예전 마커(릴리즈 없음)는 null
#[test]
fn last_result_carries_the_version_and_notes_in_the_asked_language() {
    use saba_chan_updater_lib::marker::ReleaseSummary;
    let mut marker = saba_chan_updater_lib::UpdateCompletionMarker::success(vec!["saba-core".into()]);
    marker.release = Some(ReleaseSummary {
        version: "0.1.3".into(),
        released_at: None,
        notes: [("ko".to_string(), "고침".to_string()), ("en".to_string(), "Fixes".to_string())].into(),
    });
    let body = last_result_body(Some(marker.clone()), "ko");
    assert_eq!((body["version"].as_str(), body["notes"].as_str()), (Some("0.1.3"), Some("고침")));
    assert_eq!(last_result_body(Some(marker.clone()), "ja")["notes"], "Fixes");
    assert_eq!(body["result"]["release"]["notes"]["en"], "Fixes");

    marker.release = None;
    let old = last_result_body(Some(marker), "ko");
    assert!(old["notes"].is_null() && old["version"].is_null());
    assert!(last_result_body(None, "ko")["result"].is_null());
}

#[test]
fn updater_config_survives_restart() {
    let mut cfg = UpdateConfig::default();
    cfg.enabled = false;
    cfg.auto_download = true;
    cfg.failed_release = Some("0.1.14".into());
    cfg.user_held = true;
    let dir = tempfile::tempdir().unwrap();
    save_updater_config_to(&ConfigFile::at(dir.path()), &cfg).unwrap();

    let loaded = load_updater_config_from(&ConfigFile::at(dir.path()));
    assert!(!loaded.enabled);
    assert!(loaded.auto_download);
    assert_eq!((loaded.failed_release.as_deref(), loaded.user_held), (Some("0.1.14"), true));

    // 고정을 지우면 두 기록이 함께 없어진다
    let mut cleared = loaded;
    assert!(cleared.clear_failed_release());
    save_updater_config_to(&ConfigFile::at(dir.path()), &cleared).unwrap();
    let section = ConfigFile::at(dir.path()).section("updates");
    assert!(section.get("user_held").is_none() && section["failed_release"].is_null());
    // 확인 주기는 상수(CHECK_EVERY_HOURS)라 설정에 쓰지 않는다
    assert!(section.get("check_interval_hours").is_none());
}

/// 0.1.1이 남긴 `updates` 설정: 채널과 릴레이 주소가 업데이트 뒤에도 남아야 한다
#[test]
fn settings_written_by_0_1_1_keep_their_channel_and_relay() {
    let dir = tempfile::tempdir().unwrap();
    let file = ConfigFile::at(dir.path());
    file.set_section(
        "updates",
        json!({
            "enabled": true, "check_interval_hours": 3, "auto_download": false, "auto_apply": false,
            "install_root": null, "relay_base_url": "http://127.0.0.1:9876", "channel": "beta",
            "platform": "windows-x64", "client_id": "abcdefgh12345678"
        }),
    )
    .unwrap();
    let loaded = load_updater_config_from(&file);
    assert_eq!(loaded.channel, "beta");
    assert_eq!(loaded.relay_url.as_deref(), Some("http://127.0.0.1:9876"));

    file.set_section("updates", json!({ "relay_base_url": "", "channel": "beta" })).unwrap();
    assert_eq!(load_updater_config_from(&file).relay_url, None, "blank means the default relay");

    // 둘 다 있으면 새 이름을 쓴다 (설정 전체가 기본값으로 떨어지지 않는다)
    file.set_section("updates", json!({ "relay_base_url": "http://old", "relay_url": "http://new", "channel": "beta" })).unwrap();
    let both = load_updater_config_from(&file);
    assert_eq!((both.relay_url.as_deref(), both.channel.as_str()), (Some("http://new"), "beta"));
}

/// 복구 답: 받지 못한 것과 적용하지 못한 것은 오류로 모으고, 받아서 적용한 것만 고친 것(`repaired`)이다
#[test]
fn a_repair_reports_what_was_fixed_and_what_failed() {
    let plan = json!({ "success": true, "verified": 9, "failed": 1, "skipped": 1, "repair": [{ "component": "cli" }, { "component": "module-minecraft" }], "unavailable": [] });
    let downloaded = json!({ "success": false, "downloaded": [{ "component": "module-minecraft", "asset": "m.zip" }], "errors": [{ "component": "cli", "error": "net down" }] });
    let applied = json!({ "success": true, "applied": ["Module: minecraft"], "needs_updater": [], "requires_updater": false, "errors": [] });
    let r = super::repair_result(plan.clone(), &downloaded, &applied);
    assert_eq!(r["success"], json!(false));
    assert_eq!(r["repaired"], json!(["module-minecraft"]));
    assert_eq!(r["errors"], json!(["cli: net down"]));
    assert_eq!((r["verified"].clone(), r["requires_updater"].clone()), (json!(9), json!(false)));

    // 데몬·GUI가 끼면 고친 것은 업데이터가 적용한 뒤에 안다: 넘길 목록만 싣는다
    let core = json!({ "success": true, "downloaded": [{ "component": "saba-core" }], "errors": [] });
    let handed = json!({ "success": true, "applied": [], "needs_updater": ["saba-core"], "requires_updater": true, "errors": [] });
    let r = super::repair_result(plan, &core, &handed);
    assert_eq!((r["success"].clone(), r["requires_updater"].clone(), r["needs_updater"].clone()), (json!(true), json!(true), json!(["saba-core"])));
    assert_eq!(r["repaired"], json!([]));
}

fn kept(version: &str, replaced_by: &str) -> saba_chan_updater_lib::live_apply::PreviousVersion {
    saba_chan_updater_lib::live_apply::PreviousVersion {
        version: version.into(),
        replaced_by: replaced_by.into(),
        updated_at: "2026-11-12T03:09:00Z".into(),
        copy: saba_chan_updater_lib::live_apply::KeptCopy { dir: "/d/s/1".into(), data_dir: "/d".into(), instances_dir: "/d/i".into() },
        exes: vec![("/d/s/1/exes/0/saba-core".into(), "/opt/saba/saba-core".into())],
        hold: None,
    }
}

/// 되돌릴 수 있는 판은 지금 코어가 그 직전 판을 바꾼 판일 때만 알린다 (그 뒤 설치기로 다시 깔았으면 맞지 않는다)
#[test]
fn the_previous_version_is_offered_only_over_the_version_that_replaced_it() {
    let body = super::rollback_body(Some(&kept("0.1.13", "0.1.14")), "0.1.14");
    assert_eq!(body["available"], json!(true));
    assert_eq!((body["version"].as_str(), body["current"].as_str()), (Some("0.1.13"), Some("0.1.14")));
    assert_eq!(body["updated_at"], json!("2026-11-12T03:09:00Z"));
    assert_eq!(super::rollback_body(Some(&kept("0.1.13", "0.1.14")), "0.1.15")["available"], json!(false));
    assert_eq!(super::rollback_body(None, "0.1.14")["available"], json!(false));
}

/// 앱의 "다른 버전 선택": 지금 판, 이 PC에 남긴 직전 판(되돌리기와 같은 규칙), 가장 새 판, 사용자 고정
#[test]
fn the_versions_body_lists_installed_kept_latest_and_held() {
    let cfg = |failed: Option<&str>, user_held: bool| UpdateConfig { channel: "beta".into(), failed_release: failed.map(Into::into), user_held, ..Default::default() };

    // 남긴 판 없음, 최신
    let none = super::versions_body(&cfg(None, false), "0.1.14", None, Vec::new(), None, false);
    assert_eq!(none, json!({ "success": true, "channel": "beta", "installed": "0.1.14", "kept_previous": null, "latest": null, "held_until_newer_than": null }));

    // 직전 판을 남겼고 더 새 판이 있다. 그 판을 바꾼 코어가 아니면 남긴 판으로 보이지 않는다
    let previous = super::versions_body(&cfg(None, false), "0.1.14", Some(&kept("0.1.13", "0.1.14")), vec!["module-minecraft".into()], Some("0.1.15"), false);
    // 이미 아는 더 새 판(0.1.15)이 있으면 되돌린 뒤 그 판까지 고정한다
    assert_eq!(
        previous["kept_previous"],
        json!({ "version": "0.1.13", "updated_at": "2026-11-12T03:09:00Z", "needs_newer_core": ["module-minecraft"], "hold_until_newer_than": "0.1.15" })
    );
    let no_newer = super::versions_body(&cfg(None, false), "0.1.14", Some(&kept("0.1.13", "0.1.14")), Vec::new(), None, true);
    assert_eq!(no_newer["kept_previous"]["hold_until_newer_than"], json!("0.1.14"));
    assert_eq!(previous["latest"], json!("0.1.15"));
    assert!(super::versions_body(&cfg(None, false), "0.1.15", Some(&kept("0.1.13", "0.1.14")), Vec::new(), None, false)["kept_previous"].is_null());
    // 같은 판 다시 받기(`update_available`인데 같은 판)는 더 새 판이 아니다
    assert!(super::versions_body(&cfg(None, false), "0.1.14", None, Vec::new(), Some("0.1.14"), false)["latest"].is_null());

    // 되돌려 고정했고 확인한 새 판을 모른다: 가장 새 판은 고정한 판이다
    let held = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), None, false);
    assert_eq!((held["latest"].clone(), held["held_until_newer_than"].clone()), (json!("0.1.14"), json!("0.1.14")));
    // 확인했는데 받을 새 판이 없다(고정한 판이 내려갔다): 바꿀 새 판은 없고 고정만 남는다
    let withdrawn = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), None, true);
    assert_eq!((withdrawn["latest"].clone(), withdrawn["held_until_newer_than"].clone()), (Value::Null, json!("0.1.14")));
    // 고정 기준보다 새 코어가 나왔으면 고정은 풀린 것이다 (자동 적용을 끈 설정은 기록이 남아 있다)
    let released = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), Some("0.1.15"), true);
    assert_eq!((released["latest"].clone(), released["held_until_newer_than"].clone()), (json!("0.1.15"), Value::Null));
    // 업데이트 실패로 되돌린 것은 고정이 아니다
    assert!(super::versions_body(&cfg(Some("0.1.14"), false), "0.1.13", None, Vec::new(), None, false)["held_until_newer_than"].is_null());

    // 남은 기록: 그 판이나 더 새 판을 깔았으면 고정이 아니다
    let stale = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.14", None, Vec::new(), None, false);
    assert!(stale["held_until_newer_than"].is_null() && stale["latest"].is_null());
}

/// 사용자가 되돌려 고정한 코어를 받아 두어도 알약은 준비됨이 아니다 (자동 적용을 꺼도 같다). 앞서 남은 준비됨도 걷는다
#[tokio::test]
async fn a_held_core_leaves_no_ready_phase() {
    let _staging = STAGING.lock().await;
    let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join("saba-core-held-test.zip");
    std::fs::write(&staged, b"zip").unwrap();
    let pending = json!([{
        "component": "core_daemon", "current_version": "0.1.13", "latest_version": "99.0.0",
        "update_available": true, "download_url": null, "asset_name": "saba-core-held-test.zip",
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged.to_string_lossy(), "installed": true
    }]);
    std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    for auto_apply in [true, false] {
        {
            let mut mgr = state.manager.write().await;
            super::restore_pending(&mut mgr);
            let mut cfg = mgr.get_config();
            cfg.auto_apply = auto_apply;
            cfg.failed_release = Some("99.0.0".into());
            cfg.user_held = true;
            mgr.update_config(cfg);
        }
        state.set_phase(Phase::ready(ReadyReason::FailedRelease, Some("99.0.0".into()), None));
        super::core_apply(&state).await;
        assert_eq!(state.phase(), Phase::default(), "auto_apply={auto_apply}");
        assert!(!state.updater_launched());
    }

    std::fs::remove_file(staging.join("pending.json")).unwrap();
    std::fs::remove_file(&staged).unwrap();
}

/// 되돌려도 모듈·익스텐션은 그대로 남는다: 직전 판 코어가 읽지 않을 것(그 판보다 새 `saba-core`를 요구)을 미리 알린다
#[test]
fn the_rollback_names_addons_the_previous_core_will_not_load() {
    let needs = |min: &str| std::collections::HashMap::from([("saba-core".to_string(), min.to_string())]);
    let addons = super::addons_needing_core(
        "0.1.13",
        vec![
            ("module-minecraft".to_string(), needs(">=0.1.14")),
            ("module-palworld".to_string(), needs(">=0.1.10")),
            ("ext-steamcmd".to_string(), std::collections::HashMap::new()),
            ("ext-music".to_string(), needs("0.1.14")),
        ],
    );
    assert_eq!(addons, vec!["module-minecraft".to_string(), "ext-music".to_string()]);
}

/// 남긴 직전 판이 없으면 업데이터를 띄우지 않고(데몬도 끄지 않고) 그 까닭을 돌려준다
#[tokio::test]
async fn rolling_back_without_a_previous_version_launches_nothing() {
    let state = UpdateState::default();
    for previous in [None, Some(kept("0.1.13", "9.9.9"))] {
        let resp = super::roll_back_with(state.clone(), super::LaunchUpdaterRequest::default(), previous).await;
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
        let body = crate::ipc::internal::into_internal(resp).await.body;
        assert_eq!(body["error_code"], json!("no_previous_version"));
    }
    assert!(!state.updater_launched.load(std::sync::atomic::Ordering::SeqCst));
}

/// 판 고르기 창의 목록: 새 판부터, 내린 판은 고를 수 없게 표시하고, 가장 새 판은 내리지 않은 것 중에서 고른다
#[test]
fn the_version_list_marks_installed_latest_and_yanked_versions() {
    let entry = |v: &str| -> saba_chan_updater_lib::ManifestComponent {
        serde_json::from_value(json!({ "version": v, "asset": "m.zip", "sha256": "a".repeat(64), "url": "https://cdn.example/m.zip" })).unwrap()
    };
    let list = saba_chan_updater_lib::relay::AddonVersions {
        versions: vec![entry("0.1.13"), entry("0.1.12"), entry("0.1.10")],
        yanked: vec!["0.1.15".into(), "0.1.11".into()],
        excluded: vec!["0.1.14".into()],
    };
    let pin = addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() };
    let body = addon_versions_body("module-minecraft", "0.1.14", Some("0.1.12"), &list, Some(&pin));
    let rows: Vec<(String, bool, bool, bool)> = body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["version"].as_str().unwrap().to_string(), v["installed"] == true, v["latest"] == true, v["yanked"] == true))
        .collect();
    let row = |v: &str, installed, latest, yanked| (v.to_string(), installed, latest, yanked);
    assert_eq!(
        rows,
        [row("0.1.15", false, false, true), row("0.1.13", false, true, false), row("0.1.12", true, false, false), row("0.1.11", false, false, true), row("0.1.10", false, false, false)]
    );
    assert_eq!((body["installed"].as_str(), body["core"].as_str()), (Some("0.1.12"), Some("0.1.14")));
    assert_eq!(body["excluded"], json!(["0.1.14"]));
    assert_eq!(body["pinned"], json!({ "version": "0.1.12", "until_newer_than": "0.1.13" }));
}

/// 판 고르기 시험용 릴레이: 모듈 하나의 0.1.13(`newest_requires`를 단다)·0.1.12(받을 zip까지)와 내린 0.1.11
async fn addon_versions_relay(name: &str, newest_requires: Value) -> String {
    use axum::routing::get;
    use sha2::Digest;
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let private = KEY.get_or_init(|| {
        let (private, public) = saba_chan_updater_lib::signature::generate_key_pair().unwrap();
        // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
        unsafe { std::env::set_var(saba_chan_updater_lib::signature::TEST_PUBKEY_ENV, &public) };
        private
    });
    let zip_of = |version: &str| {
        use std::io::Write;
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        out.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(format!("[module]\nname = \"{name}\"\nversion = \"{version}\"\n").as_bytes()).unwrap();
        out.finish().unwrap().into_inner()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let key = format!("module-{name}");
    let sign = |m: Value| serde_json::to_value(saba_chan_updater_lib::signature::sign(m, saba_chan_updater_lib::signature::TEST_KEY_ID, private).unwrap()).unwrap();
    let mut app = axum::Router::new();
    let mut versions = Vec::new();
    for version in ["0.1.13", "0.1.12"] {
        let bytes = zip_of(version);
        let asset = format!("{key}-{version}.zip");
        let mut entry = json!({ "key": key, "version": version, "asset": asset, "sha256": hex::encode(sha2::Sha256::digest(&bytes)),
                                "url": format!("{base}/r2/{asset}") });
        if version == "0.1.13" && !newest_requires.is_null() {
            entry["requires"] = newest_requires.clone();
        }
        versions.push(sign(entry));
        app = axum::Router::route(app, &format!("/r2/{asset}"), get(move || async move { bytes }));
    }
    let body = json!({ "key": key, "versions": versions, "yanked": [sign(json!({ "key": key, "version": "0.1.11", "yanked": true }))], "excluded": [] });
    // 메서드 꼴로 부르지 않는다: 라우트 분류 시험(gate, CLI route_coverage)이 이 파일의 메서드 꼴 라우트를 데몬 라우트로 읽는다
    app = axum::Router::route(app, "/v1/addons/versions", get(move || async move { Json(body) }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// 판 고르기: 고른 판을 받아 바꾸고 바꾸기 전의 가장 새 판까지 고정한다. 내린 판은 고르지 못하고, 손으로 고친 폴더는 덮어쓰지 않으며,
/// 고정을 풀면 가장 새 판으로 바로 돌아간다
#[tokio::test]
async fn choosing_a_module_version_installs_and_pins_it_and_unpinning_returns_to_the_newest() {
    let _staging = STAGING.lock().await;
    let name = format!("pick{}", std::process::id());
    let key = format!("module-{name}");
    let base = addon_versions_relay(&name, Value::Null).await;
    let modules = crate::plugin::resolve_modules_dir();
    let dir = modules.join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.13\"\n")).unwrap();
    saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
    saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), ..Default::default() };
    *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());
    let call = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await };
    let pin = |version: &str| pin_addon(State(state.clone()), Path(key.clone()), axum::body::Bytes::from(json!({ "version": version }).to_string()));
    let installed = || std::fs::read_to_string(dir.join("module.toml")).unwrap();

    let refused = call(pin("0.1.11").await).await;
    assert_eq!((refused.status, refused.body["error_code"].as_str()), (axum::http::StatusCode::CONFLICT, Some("version_unavailable")));

    let picked = call(pin("0.1.12").await).await;
    assert_eq!(picked.body, json!({ "success": true, "state": "applied", "version": "0.1.12", "waiting_for": [] }));
    assert!(installed().contains("0.1.12"), "{}", installed());
    let choice = addon_pins::get(&key);
    assert_eq!(choice.pinned, Some(addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() }));
    assert_eq!(choice.pending_version, None, "nothing is waiting once the chosen version is in place");

    // 사용자가 넣은 파일이 있으면 폴더를 바꾸지 않고 고정도 그대로 둔다
    std::fs::write(dir.join("my-notes.txt"), "mine").unwrap();
    let refused = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
    assert_eq!(refused.body["error_code"], "module_modified", "{}", refused.body);
    assert!(installed().contains("0.1.12"));
    assert!(addon_pins::get(&key).pinned.is_some());

    std::fs::remove_file(dir.join("my-notes.txt")).unwrap();
    let back = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
    assert_eq!(back.body, json!({ "success": true, "state": "applied", "version": "0.1.13", "waiting_for": [] }));
    assert!(installed().contains("0.1.13"));
    assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());

    // 가장 새 판을 고르면 고정 없이 바꾼다: 고정을 푼 것과 같다
    assert_eq!(call(pin("0.1.12").await).await.body["state"], "applied");
    let newest = call(pin("0.1.13").await).await;
    assert_eq!(newest.body, json!({ "success": true, "state": "applied", "version": "0.1.13", "waiting_for": [] }));
    assert!(installed().contains("0.1.13"));
    assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(modules.join(format!(".{name}.saba-old")));
}

/// 시험용: 인스턴스가 켜진 것으로 보이게 하거나 끈다
async fn set_running(state: &UpdateState, id: &str, on: bool) {
    let sup = state.supervisor.as_ref().unwrap().read().await;
    if on {
        sup.tracker.track(id, std::process::id());
    } else {
        let _ = sup.tracker.untrack(id);
    }
}

/// 켜진 서버가 쓰는 모듈은 고른 판을 받아 두고 서버가 꺼질 때까지 기다린다(`waiting`). 꺼진 뒤 교체 한 차례가 바꾸고 기다림을 지우며,
/// 업데이트 확인을 꺼 두어도 고른 것은 바꾼다. 기다리는 사이 고정을 풀어 깔린 판으로 끝나면 받아 둔 판은 버려, 서버가 꺼져도 바뀌지 않는다
#[tokio::test]
async fn a_chosen_version_waits_for_the_running_server_and_cancelling_it_keeps_the_installed_one() {
    use std::sync::atomic::Ordering;
    let _staging = STAGING.lock().await;
    // 교체 반복은 이 시험이 직접 한 차례씩 돌린다
    ADDON_REPLACING.store(true, Ordering::SeqCst);
    let name = format!("wait{}", std::process::id());
    let key = format!("module-{name}");
    let base = addon_versions_relay(&name, Value::Null).await;
    let modules = crate::plugin::resolve_modules_dir();
    let dir = modules.join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.13\"\n")).unwrap();
    saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
    saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let mut state = state_in(tmp.path());
    let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), enabled: true, ..Default::default() };
    *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    let server = crate::instance::ServerInstance::new("srv", &name);
    let id = server.id.clone();
    sup.instance_store.add(server).unwrap();
    state.supervisor = Some(Arc::new(RwLock::new(sup)));
    let call = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await };
    let pin = |version: &str| pin_addon(State(state.clone()), Path(key.clone()), axum::body::Bytes::from(json!({ "version": version }).to_string()));
    let installed = || std::fs::read_to_string(dir.join("module.toml")).unwrap();
    let waiting = json!({ "success": true, "state": "waiting", "version": "0.1.12", "waiting_for": ["srv"] });

    set_running(&state, &id, true).await;
    assert_eq!(call(pin("0.1.12").await).await.body, waiting);
    assert!(installed().contains("0.1.13"), "the running server keeps its version");
    assert_eq!(addon_pins::get(&key).pending_version.as_deref(), Some("0.1.12"));

    // 기다리는 사이 자동 업데이트로 돌아가면 깔린 0.1.13 그대로 끝나고, 서버가 꺼져도 받아 둔 0.1.12로 바꾸지 않는다
    let back = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
    assert_eq!(back.body["state"], "applied", "{}", back.body);
    assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());
    set_running(&state, &id, false).await;
    assert_eq!(replace_addons_once(&state).await, Leftover::default());
    assert!(installed().contains("0.1.13"), "a cancelled switch is not applied: {}", installed());

    // 업데이트 확인을 꺼 두어도 고른 판은 서버가 꺼진 뒤 바꾼다
    let mut cfg = state.manager.read().await.get_config();
    cfg.enabled = false;
    state.manager.write().await.update_config(cfg);
    set_running(&state, &id, true).await;
    assert_eq!(call(pin("0.1.12").await).await.body, waiting);
    assert_eq!(replace_addons_once(&state).await, Leftover { held: true, failed: false });
    assert!(installed().contains("0.1.13"));
    set_running(&state, &id, false).await;
    assert_eq!(replace_addons_once(&state).await, Leftover::default());
    assert!(installed().contains("0.1.12"), "{}", installed());
    let choice = addon_pins::get(&key);
    assert_eq!(choice.pending_version, None, "the wait ends once the chosen version is in place");
    assert_eq!(choice.pinned, Some(addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() }));

    ADDON_REPLACING.store(false, Ordering::SeqCst);
    let _ = addon_pins::update(&key, |c| *c = addon_pins::AddonChoice::default());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(modules.join(format!(".{name}.saba-old")));
}

/// 고정을 풀 때도 가장 새 판의 요구 조건을 깔린 것과 대조한다: 맞지 않으면 받지 않고 거절하고 고정은 그대로 둔다
#[tokio::test]
async fn unpinning_refuses_a_newest_version_whose_requirements_are_unmet() {
    let _staging = STAGING.lock().await;
    let name = format!("needs{}", std::process::id());
    let key = format!("module-{name}");
    let base = addon_versions_relay(&name, json!({ "ext-not-installed": ">=1.0.0" })).await;
    let modules = crate::plugin::resolve_modules_dir();
    let dir = modules.join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.12\"\n")).unwrap();
    saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
    saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
    let pinned = addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() };
    addon_pins::update(&key, |c| c.pinned = Some(pinned.clone())).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), ..Default::default() };
    *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());

    let refused = crate::ipc::internal::into_internal(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
    assert_eq!((refused.status, refused.body["error_code"].as_str()), (axum::http::StatusCode::CONFLICT, Some("requires_unmet")), "{}", refused.body);
    assert!(std::fs::read_to_string(dir.join("module.toml")).unwrap().contains("0.1.12"));
    assert_eq!(addon_pins::get(&key).pinned, Some(pinned));

    let _ = addon_pins::update(&key, |c| *c = addon_pins::AddonChoice::default());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 알약·사바 스토리지·셸이 같은 상태를 따로 묻지 않게 데몬이 상태가 바뀔 때만 `update.status`(상태 조회와 같은 본문)를 낸다:
/// 띄우자마자 한 번(다시 붙는 클라이언트가 버퍼에서 받는다), 단계가 바뀔 때마다 한 번, 바뀐 것이 없으면 내지 않는다
#[tokio::test]
async fn status_changes_reach_clients_as_update_status_events() {
    const VERSION: &str = "9.9.9-status-event";
    let tmp = tempfile::tempdir().unwrap();
    let state = state_in(tmp.path());
    let mut events = crate::events::bus().subscribe(None).rx;
    let stop = tokio_util::sync::CancellationToken::new();
    spawn_status_events(state.clone(), stop.clone());
    // 이 시험의 상태가 낸 것만: 다른 시험도 같은 버스를 쓴다
    async fn collect(events: &mut tokio::sync::broadcast::Receiver<Arc<crate::events::Event>>, seen: &mut Vec<Value>, for_ms: u64) {
        let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
        while let Ok(Ok(e)) = tokio::time::timeout_at(until, events.recv()).await {
            if e.kind == "update.status" && (e.data["phase"]["version"] == VERSION || seen.is_empty()) {
                seen.push(e.data.clone());
            }
        }
    }
    let mut seen = Vec::new();
    collect(&mut events, &mut seen, 1000).await;
    assert_eq!(seen.len(), 1, "one status right after the daemon starts: {seen:?}");
    assert_eq!(seen[0]["success"], true);
    assert!(seen[0]["components"].is_array() && seen[0].get("locales_version").is_some(), "the same body as GET /api/updates/status");

    state.set_phase(Phase::new(PhaseState::Waiting, Some(VERSION.into()), None));
    collect(&mut events, &mut seen, 1000).await;
    state.status_may_have_changed();
    collect(&mut events, &mut seen, 1000).await;
    state.set_phase(Phase::new(PhaseState::Done, Some(VERSION.into()), None));
    collect(&mut events, &mut seen, 1000).await;
    let phases: Vec<&Value> = seen[1..].iter().map(|s| &s["phase"]["state"]).collect();
    assert_eq!(phases, [&json!("waiting"), &json!("done")], "only changes are announced");
    stop.cancel();
}

/// 받는 진행은 데몬이 퍼센트까지 계산해 `update.progress`로 낸다. 전체 크기를 모르면 퍼센트는 null
#[test]
fn download_progress_events_carry_the_percent_the_daemon_computed() {
    let progress = |received, total, active| DownloadProgress { component: Some("module-mc".into()), bytes_received: received, total_bytes: total, active };
    assert_eq!(
        progress_event(&progress(1, 3, true)),
        json!({ "component": "module-mc", "active": true, "bytesReceived": 1, "totalBytes": 3, "percent": 33 })
    );
    assert_eq!(progress_event(&progress(2, 3, true))["percent"], 67);
    assert_eq!(progress_event(&progress(5, 0, true))["percent"], Value::Null);
    assert_eq!(progress_event(&progress(0, 0, false))["active"], false);
}
