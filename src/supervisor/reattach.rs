//! 데몬을 다시 켰을 때 앞 실행이 띄운 서버에 다시 붙기

use super::*;

impl Supervisor {
    /// Detect orphaned managed processes from a previous daemon run and restore
    /// their console log history from disk.
    ///
    /// For each instance that has a `managed.pid` file:
    /// 1. Read the PID and check if it's still alive
    /// 2. If alive: re-register in tracker + create a stub ManagedProcess with
    ///    console history loaded from `logs/console.log`
    /// 3. If dead: clean up the stale PID file
    pub(super) async fn reattach_orphaned_managed(&mut self) {
        self.adopt_legacy_records();
        let instances: Vec<_> = self.instance_store.list().to_vec();
        let mut reattached = 0u32;

        for instance in &instances {
            let inst_dir = self.instance_store.instance_dir(&instance.id);
            let pid_file = self.instance_store.managed_pid_file(&instance.id);

            // saba-runner가 쥔 서버면 콘솔·stdin까지 그대로 다시 붙는다 (C8b)
            let log_pattern = self.module_loader.get_module(&instance.module_name)
                .ok()
                .and_then(|m| m.metadata.log_pattern.clone());
            let attached = managed_process::ManagedProcess::attach_runner(&self.instance_store.runner_home(&instance.id), log_pattern.as_deref()).await;
            if attached.is_none() {
                // 업데이트 전에 띄운 러너가 끝났다: 그 뒤로는 인스턴스 폴더의 러너 기록을 보지 않는다
                if let Err(e) = forget_legacy_run(&self.instance_store, &instance.id) {
                    tracing::warn!("Could not stop following the old runner record of '{}': {}", instance.name, e);
                }
            }
            if let Some(proc) = attached {
                let pid = proc.pid;
                // 가동 시간은 데몬이 다시 붙은 때가 아니라 러너가 서버를 띄운 때부터
                if let Err(e) = self.tracker.track_since(&instance.id, pid, proc.started_at) {
                    tracing::warn!("Failed to re-track '{}' (PID {}): {}", instance.name, pid, e);
                    continue;
                }
                let _ = saba_atomic_file::write(&pid_file, pid.to_string().as_bytes(), saba_atomic_file::Mode::Plain);
                self.managed_store.insert(&instance.id, proc).await;
                reattached += 1;
                tracing::info!("Reattached '{}' through saba-runner (PID {})", instance.name, pid);
                // 접속·퇴장 알림, 게임 채팅, 조작 가능한 사람만 접속도 다시 콘솔을 본다 (시작 때 `announce_ready`와 같다)
                if let Some(console) = self.managed_store.get(&instance.id).await {
                    let meta = self.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
                    let (join, leave, chat) = meta
                        .map(|m| (m.player_join_patterns, m.player_leave_patterns, m.chat_patterns))
                        .unwrap_or_default();
                    game_chat::remember(&instance.id, &instance.game_chat());
                    player_events::after_reattach(&self.runtime, &instance.id, &instance.name, console, &join, &leave, &chat);
                }
                continue;
            }

            let pid_str = match std::fs::read_to_string(&pid_file) {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue, // no managed.pid → not a managed instance
            };

            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    let _ = std::fs::remove_file(&pid_file);
                    continue;
                }
            };

            // 기록한 뒤에 시작한 프로세스는 재부팅 뒤 같은 PID를 받은 다른 프로세스다 (기록 시각 = managed.pid를 쓴 때)
            let recorded_at = std::fs::metadata(&pid_file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let same = match recorded_at {
                Some(at) => process::is_same_process(pid, at),
                None => process::is_process_alive(pid),
            };
            if same {
                // Process is still running — re-register it
                if let Err(e) = self.tracker.track(&instance.id, pid) {
                    tracing::warn!("Failed to re-track orphan '{}' (PID {}): {}", instance.name, pid, e);
                    continue;
                }

                // Create a stub ManagedProcess with history from disk
                let stub = managed_process::ManagedProcess::create_reattached_stub(
                    pid,
                    &inst_dir,
                ).await;

                self.managed_store.insert(&instance.id, stub).await;
                reattached += 1;
                tracing::info!(
                    "Reattached orphaned managed process '{}' (PID {})",
                    instance.name, pid
                );
            } else {
                // Process is dead (or the PID now belongs to another process) — clean up stale PID file
                tracing::info!(
                    "Cleaning up stale managed.pid for '{}' (PID {} is no longer the server)",
                    instance.name, pid
                );
                let _ = std::fs::remove_file(&pid_file);
            }
        }

        if reattached > 0 {
            tracing::info!("Reattached {} orphaned managed process(es)", reattached);
        }
    }

    /// 예전 배치(인스턴스 폴더의 managed.pid·run/)를 처음 한 번만 옮겨 받는다. 업데이트 전에 띄운 서버에 다시 붙기 위해서다.
    /// managed.pid는 기록 시각(파일 시각)을 지키려고 이름만 바꾼다. 아직 도는 러너는 띄울 때 받은 폴더에 계속 쓰므로
    /// 옮기지 않고 그 러너가 끝날 때까지 거기서 읽는다(`LEGACY_RUN`). 다 옮겨야 표시를 남긴다 (못 옮긴 것은 다음 시작 때 다시)
    fn adopt_legacy_records(&self) {
        use crate::instance::{LEGACY_ADOPTED, LEGACY_RUN, MANAGED_PID};
        let store = &self.instance_store;
        let marker = store.state_root().join(LEGACY_ADOPTED);
        if marker.exists() {
            return;
        }
        let mut adopted = true;
        for instance in store.list() {
            let dir = store.instance_dir(&instance.id);
            let state = store.state_dir(&instance.id);
            let legacy_pid = dir.join(MANAGED_PID);
            if legacy_pid.is_file() {
                if let Err(e) = std::fs::create_dir_all(&state).and_then(|_| std::fs::rename(&legacy_pid, store.managed_pid_file(&instance.id))) {
                    tracing::warn!("Could not move the server record of '{}': {}", instance.name, e);
                    adopted = false;
                }
            }
            let legacy_run = managed_process::run_dir(&dir);
            if saba_runner::runner_file(&legacy_run).is_file() {
                if let Err(e) = saba_atomic_file::write(&state.join(LEGACY_RUN), b"", saba_atomic_file::Mode::Plain) {
                    tracing::warn!("Could not keep following the runner of '{}': {}", instance.name, e);
                    adopted = false;
                }
            } else {
                managed_process::clear_run_dir(&legacy_run);
            }
        }
        if adopted {
            if let Err(e) = saba_atomic_file::write(&marker, b"", saba_atomic_file::Mode::Plain) {
                tracing::warn!("Could not mark the server records as moved: {}", e);
            }
        }
    }
}

/// 업데이트 전에 띄운 러너를 더 따라가지 않는다: 표시(`LEGACY_RUN`)를 지우고 인스턴스 폴더에 남은 그 러너의 기록을 치운다
pub(super) fn forget_legacy_run(store: &crate::instance::InstanceStore, id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(store.state_dir(id).join(crate::instance::LEGACY_RUN)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    managed_process::clear_run_dir(&managed_process::run_dir(&store.instance_dir(id)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 재부팅 뒤 기록된 PID를 다른 프로세스가 쓰면 다시 붙지 않는다 (붙으면 정지할 때 그 프로세스를 끈다)
    #[tokio::test]
    async fn a_pid_reused_after_reboot_is_not_reattached() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        for (id, before_start) in [("reused", true), ("same", false)] {
            let mut inst = crate::instance::ServerInstance::new(id, "minecraft");
            inst.id = id.into();
            supervisor.instance_store.add(inst).unwrap();
            let pid_file = supervisor.instance_store.managed_pid_file(id);
            std::fs::create_dir_all(pid_file.parent().unwrap()).unwrap();
            std::fs::write(&pid_file, std::process::id().to_string()).unwrap();
            if before_start {
                // 이 프로세스가 뜨기 한 시간 전에 기록한 PID
                let started = process::process_start_time(std::process::id()).unwrap();
                let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(started - 3600);
                std::fs::File::options().write(true).open(&pid_file).unwrap().set_modified(at).unwrap();
            }
        }
        supervisor.reattach_orphaned_managed().await;
        assert!(supervisor.tracker.get_pid("reused").is_err(), "a process that started after the record is someone else");
        assert!(!supervisor.instance_store.managed_pid_file("reused").exists());
        assert_eq!(supervisor.tracker.get_pid("same").unwrap(), std::process::id());
    }

    /// 인스턴스 폴더에 새로 생긴 PID 기록은 믿지 않는다 (그 폴더는 서버·모듈도 쓴다)
    #[tokio::test]
    async fn a_pid_record_in_the_instance_folder_is_not_reattached() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("srv", "minecraft");
        inst.id = "srv".into();
        supervisor.instance_store.add(inst).unwrap();
        supervisor.reattach_orphaned_managed().await;

        let inst_dir = supervisor.instance_store.instance_dir("srv");
        std::fs::write(inst_dir.join("managed.pid"), std::process::id().to_string()).unwrap();
        supervisor.reattach_orphaned_managed().await;
        assert!(supervisor.tracker.get_pid("srv").is_err());
    }

    /// 업데이트 전 배치의 PID 기록은 처음 시작 때 한 번만 옮겨 받는다 (그때 돌던 서버에 다시 붙는다)
    #[tokio::test]
    async fn a_server_record_from_before_the_update_is_adopted_once() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("srv", "minecraft");
        inst.id = "srv".into();
        supervisor.instance_store.add(inst).unwrap();
        let inst_dir = supervisor.instance_store.instance_dir("srv");
        std::fs::write(inst_dir.join("managed.pid"), std::process::id().to_string()).unwrap();
        // 끝난 러너의 기록: 따라가지 않는다
        std::fs::create_dir_all(inst_dir.join("run")).unwrap();
        std::fs::write(inst_dir.join("run").join("runner.json"), "{}").unwrap();

        supervisor.reattach_orphaned_managed().await;
        assert_eq!(supervisor.tracker.get_pid("srv").unwrap(), std::process::id());
        assert!(supervisor.instance_store.managed_pid_file("srv").exists());
        assert!(!inst_dir.join("managed.pid").exists());
        assert_eq!(supervisor.instance_store.runner_home("srv"), supervisor.instance_store.state_dir("srv"));
        assert!(!inst_dir.join("run").exists(), "the old runner record is cleared once it is not followed");

        supervisor.tracker.untrack("srv").unwrap();
        std::fs::remove_file(supervisor.instance_store.managed_pid_file("srv")).unwrap();
        std::fs::write(inst_dir.join("managed.pid"), std::process::id().to_string()).unwrap();
        supervisor.reattach_orphaned_managed().await;
        assert!(supervisor.tracker.get_pid("srv").is_err());
    }

    /// 데몬이 다시 붙으면 콘솔 줄 번호가 0부터 다시 매겨진다. PID가 같아도 클라이언트가 이어 읽던 번호를
    /// 버리게 콘솔 응답의 `epoch`가 바뀐다 (그대로면 새 줄이 옛 번호를 넘을 때까지 콘솔이 멈추거나 줄이 겹친다)
    #[tokio::test]
    async fn a_reattached_console_reports_a_new_epoch() {
        let tmp = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new_with_instances_dir("./modules", tmp.path().to_str().unwrap());
        let inst_dir = tmp.path().join("srv");
        let mut epochs = Vec::new();
        for _ in 0..2 {
            let stub = managed_process::ManagedProcess::create_reattached_stub(std::process::id(), &inst_dir).await;
            supervisor.managed_store.insert("srv", stub).await;
            let out = supervisor.get_console_output("srv", Some(0), None).await.unwrap();
            epochs.push(out["epoch"].as_u64().unwrap_or_else(|| panic!("no epoch: {out}")));
        }
        assert_ne!(epochs[0], epochs[1]);
    }
}
