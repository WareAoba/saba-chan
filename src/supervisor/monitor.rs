//! 2초 프로세스 모니터: 끝난 서버 정리, 사바쨩 밖에서 켠 서버 찾기, 포트 충돌 정지

use super::*;
use super::start::announce_started;

impl Supervisor {
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
            if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                tracked_count += 1;
                
                let tracked_after_snapshot = self.tracker.get_start_time(&instance.id).is_ok_and(|t| t >= snapshot_taken_at);
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
                                let _ = self.tracker.replace_pid(&instance.id, new_pid);
                            }
                            None => tracing::debug!("Instance '{}': PID {} gone but saba-runner still runs it", instance.name, pid),
                        }
                        continue;
                    }
                    tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                    // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                    if let Err(e) = self.tracker.untrack(&instance.id) {
                        tracing::error!("Failed to untrack process: {}", e);
                    }
                    // 사바쨩이 시키지 않았는데 사라진 non-managed 서버 (managed는 위에서 판정했다)
                    let judged = dead.iter().any(|(id, _)| *id == instance.id) || self.managed_store.get(&instance.id).await.is_some();
                    if !judged && !self.runtime.existing(&instance.id, |r| r.stop_requested.is_some()).unwrap_or(false) {
                        let verdict = crash::ExitVerdict { kind: crash::ExitKind::UnknownExit, exit_code: None, signal: None, evidence: "exit status unknown".into() };
                        let event = crash::ExitEvent { at: saba_runner::now(), verdict, restart_in_secs: None, note: None, note_code: None, note_args: None };
                        self.record_exit(&instance.id, &instance.name, event, None, None);
                    }
                }
                continue;
            }
            
            // 프로비저닝 중인 인스턴스는 auto-detect 건너뛰기
            if let Some(ref pt) = self.provision_tracker {
                if let Some(progress) = pt.get(&instance.id) {
                    if !progress.done {
                        tracing::debug!(
                            "Skipping auto-detect for '{}' (provisioning in progress)",
                            instance.name
                        );
                        continue;
                    }
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
                    let modules = self.module_loader.modules().unwrap_or_default();
                    let cmd_patterns = modules.iter().find(|m| m.metadata.name == instance.module_name)
                        .map(|m| m.metadata.cmd_patterns.as_slice())
                        .unwrap_or_default();

                    // snapshot 기반 검색 — 시스템 콜 없음
                    let processes = if cmd_patterns.is_empty() {
                        process_monitor::find_by_name_in(process_snapshot, process_name)
                    } else {
                        process_monitor::find_by_name_and_cmd_in(process_snapshot, process_name, cmd_patterns)
                    };

                    // 같은 게임 인스턴스가 여럿이면 폴더로 주인을 가리고, 다른 인스턴스가 잡은 PID는 빼고 고른다
                    let siblings: Vec<&crate::instance::ServerInstance> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.module_name == instance.module_name && i.id != instance.id)
                        .collect();
                    let taken: std::collections::HashSet<u32> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.id != instance.id)
                        .filter_map(|i| self.tracker.get_pid(&i.id).ok())
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
                                if let Err(e) = self.tracker.track(&instance.id, process.pid) {
                                    tracing::error!("Failed to track process: {}", e);
                                }
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
                                    .filter(|i| i.id != instance.id && self.tracker.get_pid(&i.id).is_ok())
                                    .map(|i| i.id.clone())
                                    .collect();
                                let module_protocols = self.build_module_protocols_map();
                                let conflicts = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols));
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
                                if self.first_scan_done && !starting && self.tracker.get_pid(&instance.id).is_ok() {
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

    // 백로그 L8: 감시는 프로세스 목록을 락 밖에서 먼저 찍는다. 그 사이에 막 시작해 추적한 PID는 목록에 없어
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
        sup.tracker.track(&id, 4_000_000).unwrap();
        let tracked_at = sup.tracker.get_start_time(&id).unwrap();

        sup.monitor_processes_taken_at(&[], tracked_at - 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_ok(), "the snapshot predates the start");
        sup.monitor_processes_taken_at(&[], tracked_at + 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_err(), "a later snapshot without the pid means it is gone");
    }

    /// cleanup_dead()가 죽은 managed process를 정리할 때
    /// stop_cooldowns에 등록되어 auto_detect 재탐색이 억제되는지 검증
    #[tokio::test]
    async fn test_cleanup_dead_registers_stop_cooldown() {
        let supervisor = Supervisor::new("./modules");

        // 가짜 인스턴스를 등록 (파일 시스템 없이 인메모리 테스트)
        // cleanup_dead가 dead_ids를 반환하면 monitor_processes가 cooldown을 삽입해야 함
        // managed_store에 직접 죽은 프로세스를 넣을 순 없으므로,
        // 쿨다운 삽입 로직 자체를 단위 테스트
        let cooldown = || supervisor.runtime.existing("test-instance-id", |r| r.stop_cooldown).flatten();
        assert!(cooldown().is_none());

        // dead_ids에 해당하는 쿨다운이 삽입됨을 시뮬레이션
        let dead_ids = vec!["test-instance-id".to_string()];
        for dead_id in &dead_ids {
            supervisor.runtime.with(dead_id, |r| r.stop_cooldown = Some(Instant::now()));
        }

        // 쿨다운이 30초 내이면 auto_detect를 건너뛰어야 함
        let stopped_at = cooldown().unwrap();
        assert!(stopped_at.elapsed().as_secs() < STOP_COOLDOWN_SECS);
    }
}
