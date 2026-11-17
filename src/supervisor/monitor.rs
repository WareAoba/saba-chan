//! 2초 프로세스 모니터: 끝난 서버 정리, 사바쨩 밖에서 켠 서버 찾기, 포트 충돌 정지

use super::*;
use super::start::announce_started;

impl Supervisor {
    /// 표본을 뜰 인스턴스 목록 (프로세스 PID와 폴더)
    pub(crate) fn sample_roots(&self) -> Vec<crate::system_monitor::InstanceRoot> {
        self.instance_store
            .list()
            .iter()
            .map(|inst| {
                let mut dirs = vec![self.instance_store.instance_dir(&inst.id)];
                if let Some(wd) = inst.working_dir.as_deref().filter(|w| !w.is_empty()) {
                    dirs.push(std::path::PathBuf::from(wd));
                }
                let module = self.module_loader.get_module(&inst.module_name).ok();
                crate::system_monitor::InstanceRoot {
                    id: inst.id.clone(),
                    name: inst.name.clone(),
                    pid: self.tracker.get_pid(&inst.id),
                    dirs,
                    ports: crate::port_check::public_ports(inst, module.as_ref().and_then(|m| m.metadata.settings.as_ref())),
                }
            })
            .collect()
    }

    /// 인스턴스 서버 파일이 있을 수 있는 폴더들 (작업 폴더, 실행 파일 폴더, 인스턴스 폴더)
    fn instance_roots(&self, instance: &crate::instance::ServerInstance) -> Vec<String> {
        let mut roots = Vec::new();
        if let Some(dir) = instance.working_dir.as_deref().filter(|d| !d.is_empty()) {
            roots.push(dir.to_string());
        }
        // module_settings의 실행 파일 경로까지 본 실제 실행 파일
        let exe = self.resolve_executable(instance, &instance.module_name);
        if let Some(parent) = exe.as_deref().and_then(|p| std::path::Path::new(p).parent()) {
            let parent = parent.to_string_lossy().to_string();
            if !parent.is_empty() {
                roots.push(parent);
            }
        }
        roots.push(self.instance_store.instance_dir(&instance.id).to_string_lossy().to_string());
        roots
    }

    /// `snapshot_taken_at`(unix 초): 프로세스 목록을 찍기 시작한 시각. 그 뒤에 추적한 PID는 목록에 없어도
    /// 끝났다고 보지 않는다 (목록을 락 밖에서 찍는 사이에 막 시작한 서버)
    pub async fn monitor_processes_taken_at(&mut self, process_snapshot: &[crate::process_monitor::RunningProcess], snapshot_taken_at: u64) -> Result<()> {
        use crate::process_monitor;

        // Clean up dead managed processes and their PID files
        let dead = self.managed_store.cleanup_dead().await;
        for (dead_id, proc) in &dead {
            self.judge_exit(dead_id, proc, true).await;
            let pid_file = self.instance_store.managed_pid_file(dead_id);
            let _ = std::fs::remove_file(&pid_file);
            // 죽은 managed process에 대해 stop cooldown 등록 → auto_detect 재탐색 억제
            // 이전 세션에서 실행되어 reattach된 프로세스가 종료된 경우,
            // cooldown 없이 auto_detect가 즉시 동명 프로세스를 재추적하는 것을 방지
            self.runtime.with(dead_id, |r| r.stop_cooldown = Some(Instant::now()));
        }
        
        let all_instances_snapshot = self.instance_store.list().to_vec();
        let mut tracked_count = 0;
        let mut auto_detected_count = 0;
        
        for instance in &all_instances_snapshot {
            // 이미 추적 중이면 상태만 확인 (snapshot 기반 — 시스템 콜 없음)
            if let Some(pid) = self.tracker.get_pid(&instance.id) {
                tracked_count += 1;
                
                let tracked_after_snapshot = self.tracker.get_start_time(&instance.id).is_some_and(|t| t >= snapshot_taken_at);
                if !tracked_after_snapshot && !process_monitor::is_running_in(process_snapshot, pid) {
                    // 러너가 아직 서버를 돌리고 있으면(런처가 끝나고 진짜 서버가 남은 경우) 러너가 보는 PID로 옮긴다
                    // 로그만 따라 읽는 익스텐션 follower는 서버 수명을 말해 주지 않는다: 러너일 때만
                    if matches!(self.managed_store.get(&instance.id).await, Some(p) if p.is_running() && p.runner_pid.is_some()) {
                        let dir = self.instance_store.runner_home(&instance.id);
                        match managed_process::runner_server_pid(&dir)
                            .filter(|p| *p != pid && process_monitor::is_running_in(process_snapshot, *p))
                        {
                            Some(new_pid) => {
                                tracing::info!("Instance '{}': launcher {} exited, following server PID {}", instance.name, pid, new_pid);
                                self.tracker.replace_pid(&instance.id, new_pid);
                            }
                            None => tracing::debug!("Instance '{}': PID {} gone but saba-runner still runs it", instance.name, pid),
                        }
                        continue;
                    }
                    // 사바쨩이 시키지 않았는데 사라진 non-managed 서버 (managed는 위에서 판정했다)
                    let judged = dead.iter().any(|(id, _)| *id == instance.id) || self.managed_store.get(&instance.id).await.is_some();
                    let unrequested = !judged && !self.runtime.existing(&instance.id, |r| r.stop_requested.is_some()).unwrap_or(false);
                    // 데몬이 띄운 프로세스면 그 손잡이로 종료 상태를 안다. 끝을 아직 받지 못했으면(사라진 바로 그때) 다음 순회에 본다
                    let launched = self
                        .runtime
                        .existing(&instance.id, |r| r.launched.as_ref().filter(|l| l.pid == pid).map(|l| l.ended()))
                        .flatten();
                    if unrequested && launched == Some(None) {
                        continue;
                    }
                    tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                    let started_at = self.tracker.get_start_time(&instance.id);
                    // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                    self.tracker.untrack(&instance.id);
                    self.runtime.existing(&instance.id, |r| r.launched = None);
                    if unrequested {
                        let ran_secs = started_at.map_or(u64::MAX, |t| saba_runner::now().saturating_sub(t));
                        self.judge_unrequested_exit(instance, launched.flatten(), ran_secs);
                    }
                }
                continue;
            }
            
            // 프로비저닝 중인 인스턴스는 auto-detect 건너뛰기
            if let Some(progress) = self.provision_tracker.get(&instance.id) {
                if !progress.done {
                    tracing::debug!(
                        "Skipping auto-detect for '{}' (provisioning in progress)",
                        instance.name
                    );
                    continue;
                }
            }

            // auto_detect가 활성화되어 있고 process_name이 설정되어 있으면 감지 시도
            if instance.auto_detect {
                // stop 쿨다운 중이면 건너뛰기
                if let Some(stopped_at) = self.runtime.existing(&instance.id, |r| r.stop_cooldown).flatten() {
                    if stopped_at.elapsed().as_secs() < STOP_COOLDOWN_SECS {
                        tracing::debug!(
                            "Skipping auto-detect for '{}' (stop cooldown: {}s remaining)",
                            instance.name,
                            STOP_COOLDOWN_SECS - stopped_at.elapsed().as_secs()
                        );
                        continue;
                    } else {
                        // 쿨다운 만료 → 제거
                        self.runtime.existing(&instance.id, |r| r.stop_cooldown = None);
                    }
                }
                if let Some(process_name) = &instance.process_name {
                    // 모듈에서 cmd_patterns 가져오기 — java.exe 같은 범용 프로세스를 구분하기 위한 커맨드라인 패턴
                    let cmd_patterns = self.module_loader.get_module(&instance.module_name).map(|m| m.metadata.cmd_patterns.clone()).unwrap_or_default();

                    // snapshot 기반 검색 — 시스템 콜 없음
                    let processes = process_monitor::find_by_name_and_cmd_in(process_snapshot, process_name, &cmd_patterns);

                    // 같은 게임 인스턴스가 여럿이면 폴더로 주인을 가리고, 다른 인스턴스가 잡은 PID는 빼고 고른다
                    let siblings: Vec<&crate::instance::ServerInstance> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.module_name == instance.module_name && i.id != instance.id)
                        .collect();
                    let taken: std::collections::HashSet<u32> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.id != instance.id)
                        .filter_map(|i| self.tracker.get_pid(&i.id))
                        .collect();
                    let (own_roots, other_roots) = process_monitor::drop_shared_roots(
                        self.instance_roots(instance),
                        siblings.iter().flat_map(|i| self.instance_roots(i)).collect(),
                    );
                    let picked = process_monitor::pick_owned_process(&processes, &taken, &own_roots, &other_roots);

                    if let Some(process) = picked {
                                auto_detected_count += 1;
                                tracing::info!(
                                    "Auto-detected process '{}' (PID: {}) for instance '{}'",
                                    process_name, process.pid, instance.name
                                );
                                self.tracker.track(&instance.id, process.pid);
                                // 지난 정지의 표시를 지운다: 이 프로세스가 나중에 사라지면 사용자 정지가 아니다
                                self.runtime.existing(&instance.id, |r| r.stop_requested = None);

                                // ── 포트 충돌 검사: 새로 감지된 프로세스의 포트가 이미 실행 중인 인스턴스와 겹치면 강제 종료 ──
                                // GUI에서 portConflictCheck를 비활성화하면 auto-detect 시에도 강제 종료를 건너뜀
                                if self.skip_port_check.load(Ordering::Relaxed) {
                                    tracing::debug!("Port conflict check skipped for auto-detected '{}' (disabled by GUI setting)", instance.name);
                                } else {
                                let all_instances = self.instance_store.list();
                                let running_ids: std::collections::HashSet<String> = all_instances
                                    .iter()
                                    .filter(|i| i.id != instance.id && self.tracker.get_pid(&i.id).is_some())
                                    .map(|i| i.id.clone())
                                    .collect();
                                let conflicts = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, &self.rest_port_settings());
                                if !conflicts.is_empty() {
                                    let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
                                    tracing::warn!(
                                        "Port conflict detected for auto-detected '{}' — force stopping (PID {}). Conflicts: {:?}",
                                        instance.name, process.pid, details
                                    );

                                    // 자식까지 강제 종료 (런처형 서버는 런처만 끝내면 실제 서버가 남는다. taskkill은 동기라 spawn_blocking).
                                    // 끝나기를 기다리지 않는다: 여기는 supervisor 쓰기 잠금 안이라 기다리는 동안 모든 API 요청이 멈춘다
                                    let kill_pid = process.pid;
                                    let kill_name = instance.name.clone();
                                    tokio::task::spawn_blocking(move || match process::force_kill_tree(kill_pid) {
                                        Ok(()) => tracing::info!("Force-killed PID {} for instance '{}'", kill_pid, kill_name),
                                        Err(e) => tracing::warn!("Could not force-kill PID {} for instance '{}': {}", kill_pid, kill_name, e),
                                    });

                                    // tracker에서 제거 + 쿨다운 설정
                                    let _ = self.tracker.untrack(&instance.id);
                                    self.runtime.with(&instance.id, |r| r.stop_cooldown = Some(Instant::now()));

                                    // 이벤트 기록 (GUI 폴링에서 읽어감) — 충돌마다 개별 이벤트 생성
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0);
                                    // 알림함에도 (`crate::notices`)
                                    for conflict in &conflicts {
                                        crate::events::emit(
                                            "instance.port_conflict_stop",
                                            serde_json::json!({ "instanceId": instance.id, "instance": instance.name, "port": conflict.port, "existing": conflict.conflicting_instance_name, "existingId": conflict.conflicting_instance_id }),
                                        );
                                    }
                                    self.record_port_conflict_stops(conflicts.iter().map(|conflict| PortConflictStopEvent {
                                        id: 0,
                                        stopped_name: instance.name.clone(),
                                        stopped_id: instance.id.clone(),
                                        existing_name: conflict.conflicting_instance_name.clone(),
                                        existing_id: conflict.conflicting_instance_id.clone(),
                                        port: conflict.port,
                                        timestamp_ms: ts,
                                    }));
                                } // if !skip_port_check
                                }
                                // 사바쨩이 띄우지 않은 서버가 켜졌다. 데몬이 켜질 때 이미 돌던 것은 빼고, 포트 충돌로 끈 것도 뺀다.
                                // 사바쨩이 띄우는 중(추적 전)에 먼저 본 것도 뺀다
                                let starting = self.runtime.existing(&instance.id, |r| r.starting).unwrap_or(false);
                                if self.first_scan_done && !starting && self.tracker.get_pid(&instance.id).is_some() {
                                    announce_started(&self.runtime, &instance.id, &instance.name, &crate::audit::Origin::new("external"), true);
                                }
                            }
                }
            }
        }
        
        tracing::debug!("Monitor cycle: {} tracked, {} auto-detected", tracked_count, auto_detected_count);
        self.first_scan_done = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_monitoring() {
        let mut supervisor = Supervisor::new("./modules");
        
        // 모니터링이 에러 없이 실행되어야 함
        let snapshot = crate::process_monitor::get_running_processes();
        let result = supervisor.monitor_processes_taken_at(&snapshot, u64::MAX).await;
        assert!(result.is_ok());
    }

    // 감시는 프로세스 목록을 락 밖에서 먼저 찍는다. 그 사이에 막 시작해 추적한 PID는 목록에 없어
    // "끝났다"며 추적에서 풀렸다. 목록을 찍은 뒤에 추적한 PID는 다음 목록까지 기다린다
    #[tokio::test]
    async fn a_pid_tracked_after_the_snapshot_is_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let inst = crate::instance::ServerInstance::new("fresh", "none");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        sup.tracker.track(&id, 4_000_000);
        let tracked_at = sup.tracker.get_start_time(&id).unwrap();

        sup.monitor_processes_taken_at(&[], tracked_at - 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_some(), "the snapshot predates the start");
        sup.monitor_processes_taken_at(&[], tracked_at + 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_none(), "a later snapshot without the pid means it is gone");
    }

    /// 끝난 managed 서버를 치우면 잠깐(stop cooldown)은 auto-detect가 같은 게임의 프로세스를 다시 붙이지 않는다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_finished_managed_server_holds_off_auto_detect() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        let mut sup = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "ended".into();
        inst.auto_detect = true;
        inst.process_name = Some("fakegame".into());
        sup.instance_store.add(inst).unwrap();
        let proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), "exit 0".into()], tmp.path().to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        sup.managed_store.insert("ended", proc).await;
        for _ in 0..50 {
            if sup.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let dir = sup.instance_store.instance_dir("ended").to_string_lossy().to_string();
        let same_game = crate::process_monitor::RunningProcess { pid: 4_000_002, name: "fakegame".into(), executable_path: None, cmd: Vec::new(), cwd: Some(dir) };
        sup.monitor_processes_taken_at(std::slice::from_ref(&same_game), u64::MAX).await.unwrap();
        assert!(sup.tracker.get_pid("ended").is_none(), "not re-attached right after the managed server ended");
    }
}
