//! 끝난 서버의 종료 판정, 종료 이력, 크래시 뒤 재시작 예약

use super::*;
use super::stop::announce_stopped;

impl Supervisor {
    /// 백그라운드 프로세스 모니터링 (주기적 실행)
    ///
    /// `process_snapshot`은 write lock 획득 전에 미리 스캔한 프로세스 목록입니다.
    /// 이를 통해 비용이 큰 시스템 콜(sysinfo)이 write lock 밖에서 실행되어
    /// GUI read 요청에 대한 블로킹을 최소화합니다.
    /// 끝난 managed 프로세스가 왜 끝났는지 가르고, 크래시면 재시작을 예약한다 (A1). `may_restart`가 false면 예약하지 않는다
    pub(super) async fn judge_exit(&mut self, instance_id: &str, proc: &ManagedProcess, may_restart: bool) {
        use managed_process::LogLevel;
        if proc.log_follower {
            return;
        }
        // 끝난 실행의 PID는 놓는다: 남겨 두면 모니터가 다음 순회에서 같은 실행을 원인 모를 종료로 다시 판정한다
        if self.tracker.get_pid(instance_id).is_ok_and(|pid| pid == proc.pid || proc.runner_pid.is_some()) {
            let _ = self.tracker.untrack(instance_id);
        }
        let stop_origin = self.runtime.existing(instance_id, |r| r.stop_requested.take()).flatten();
        let stop_requested = stop_origin.is_some();
        let Some(instance) = self.instance_store.get(instance_id).cloned() else { return };
        // 준비되기 전에 끝났으면 그 시작의 준비 알림을 닫는다 (R39)
        ready::exited(&self.runtime, instance_id, &instance.name, stop_requested);
        let became_ready = ready::take_ready(&self.runtime, instance_id);
        let (graceful, artifacts, ready_patterns) = self
            .module_loader
            .get_module(&instance.module_name)
            .map(|m| (m.metadata.graceful_exit_patterns.clone(), m.metadata.crash_artifacts.clone(), m.metadata.ready_patterns.clone()))
            .unwrap_or_default();
        let tail: Vec<String> = proc
            .get_recent_console(crash::CONSOLE_TAIL_LINES)
            .await
            .into_iter()
            .filter(|l| l.source.is_server_output())
            .map(|l| l.content)
            .collect();
        let working_dir = proc.working_dir.clone().or_else(|| instance.working_dir.as_ref().map(std::path::PathBuf::from));
        let crash_files = match &working_dir {
            Some(dir) if !stop_requested => crash::new_crash_files(dir, &artifacts, proc.started_at),
            _ => Vec::new(),
        };
        let verdict = crash::classify(&crash::ExitFacts {
            stop_requested,
            exit: proc.exit(),
            console_tail: &tail,
            graceful_patterns: &graceful,
            new_crash_files: &crash_files,
            ran_secs: if proc.reattached { u64::MAX } else { saba_runner::now().saturating_sub(proc.started_at) },
            // 준비 문구를 다음 확인(0.5초 간격) 전에 찍고 끝났으면 기다리던 쪽은 못 봤다: 콘솔 끝에서도 찾는다
            ready: (!ready_patterns.is_empty()).then(|| became_ready || ready::any_match(&ready_patterns, &tail)),
        });

        let auto_restart = setting_flag(&instance, "auto_restart", true);
        let on_unknown = setting_flag(&instance, "restart_on_unknown_exit", false);
        let mut attempt = None;
        let (restart_in, note, note_code) = if may_restart && crash::wants_restart(verdict.kind, auto_restart, on_unknown) {
            let planned = self.runtime.with(instance_id, |r| crash::next_restart_delay(&mut r.restart_history, Instant::now()).map(|delay| (delay, r.run_generation)));
            match planned {
                Some((delay, generation)) => {
                    self.pending_restarts.push((instance_id.to_string(), delay, generation));
                    attempt = Some(generation);
                    (Some(delay.as_secs()), None, None)
                }
                None => (
                    None,
                    Some(format!("auto restart gave up after {} restarts in 10 minutes", crash::MAX_RESTARTS)),
                    Some(("gave_up", serde_json::json!({ "count": crash::MAX_RESTARTS }))),
                ),
            }
        } else if verdict.kind == crash::ExitKind::Crash && !auto_restart {
            (None, Some("auto_restart is off".to_string()), Some(("auto_restart_off", serde_json::json!({}))))
        } else {
            (None, None, None)
        };

        let line = match (verdict.kind, restart_in, &note) {
            (crash::ExitKind::Crash, Some(secs), _) => Some(format!("── Crash detected ({}). Restarting in {}s. ──", verdict.evidence, secs)),
            (crash::ExitKind::UnknownExit, Some(secs), _) => Some(format!("── Server ended unexpectedly ({}). Restarting in {}s. ──", verdict.evidence, secs)),
            (crash::ExitKind::Crash, None, Some(why)) => Some(format!("── Crash detected ({}). Not restarting: {}. ──", verdict.evidence, why)),
            (crash::ExitKind::UnknownExit, None, Some(why)) => Some(format!("── Server ended unexpectedly ({}). Not restarting: {}. ──", verdict.evidence, why)),
            _ => None,
        };
        if let Some(line) = line {
            tracing::warn!("'{}': {}", instance.name, line);
            proc.push_system_line(line, LogLevel::Warn).await;
        } else {
            tracing::info!("'{}' exited: {:?} ({})", instance.name, verdict.kind, verdict.evidence);
        }

        let (note_code, note_args) = note_code.map(|(c, a)| (c.to_string(), a)).unzip();
        let kind = verdict.kind;
        let event = crash::ExitEvent { at: saba_runner::now(), verdict, restart_in_secs: restart_in, note, note_code, note_args };
        self.record_exit(instance_id, &instance.name, event, attempt, None);
        match (kind, stop_origin) {
            (crash::ExitKind::UserStop, Some(origin)) => announce_stopped(instance_id, &instance.name, kind, &origin),
            (crash::ExitKind::InGameStop, _) => announce_stopped(instance_id, &instance.name, kind, &crate::audit::Origin::new("game")),
            _ => {}
        }
    }

    /// 종료 판정 하나를 알리고(`instance.exit`) 이력에 남긴다.
    /// `attempt`는 다시 띄우기로 했을 때 그 복구 시도의 번호 (끝을 알리는 이벤트와 같은 `restartAttempt`)
    /// 시작 실패는 시킨 쪽(`source`·`userName?`·`remoteHost?`, `instance.started`와 같다)을 함께 싣는다: 켜는 중에 실패했으면 `started_by`,
    /// 켜진 뒤 곧 끝났으면 그 시작을 알린 쪽. 앱 버튼으로 켠 실패에 앱이 효과음을 내지 않게 한다
    pub(super) fn record_exit(&mut self, instance_id: &str, instance_name: &str, event: crash::ExitEvent, attempt: Option<u64>, started_by: Option<crate::audit::Origin>) {
        let (evidence_code, evidence_args) = crash::evidence_code(&event.verdict.evidence);
        let mut data = serde_json::json!({
            "instanceId": instance_id,
            "instance": instance_name,
            "kind": event.verdict.kind,
            "exitCode": event.verdict.exit_code,
            "signal": event.verdict.signal,
            "evidence": event.verdict.evidence,
            "evidenceCode": evidence_code,
            "evidenceArgs": evidence_args,
            "restartInSecs": event.restart_in_secs,
            "restartAttempt": attempt,
            "note": event.note,
            "noteCode": event.note_code,
            "noteArgs": event.note_args,
        });
        if event.verdict.kind == crash::ExitKind::StartFailed {
            let origin = started_by.or_else(|| self.runtime.existing(instance_id, |r| r.start_origin.clone()).flatten());
            if let (Some(origin), Some(map)) = (origin, data.as_object_mut()) {
                map.extend(origin_fields(&origin));
            }
        }
        crate::events::emit("instance.exit", data);
        // 크래시·예기치 않은 종료는 작업 기록에도 남긴다 (출처는 데몬 자신, 이유는 근거 코드)
        if matches!(event.verdict.kind, crash::ExitKind::Crash | crash::ExitKind::UnknownExit) {
            crate::audit::log().record(&crate::audit::Origin::new("daemon"), &crate::audit::Event {
                action: "crash",
                instance_id,
                instance: Some(instance_name),
                code: Some(evidence_code),
                ..Default::default()
            });
        }
        // 이 실행에서 처음 남기면 지난 실행의 이력부터 읽는다 (파일은 잠금 밖에서 읽는다)
        let saved = (!self.runtime.existing(instance_id, |r| r.exit_events.is_some()).unwrap_or(false)).then(|| self.saved_exit_events(instance_id));
        self.runtime.with(instance_id, |r| {
            let events = r.exit_events.get_or_insert_with(|| saved.unwrap_or_default().into());
            events.push_back(event);
            while events.len() > crash::EVENTS_KEPT {
                events.pop_front();
            }
        });
        self.save_exit_events(instance_id);
    }

    fn exit_events_path(&self, instance_id: &str) -> std::path::PathBuf {
        self.instance_store.instance_dir(instance_id).join("exit-events.json")
    }

    /// 지난 데몬 실행에서 남긴 종료 이력
    fn saved_exit_events(&self, instance_id: &str) -> Vec<crash::ExitEvent> {
        std::fs::read_to_string(self.exit_events_path(instance_id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub(super) fn save_exit_events(&self, instance_id: &str) {
        let Some(events) = self.runtime.existing(instance_id, |r| r.exit_events.clone()).flatten() else { return };
        let path = self.exit_events_path(instance_id);
        if !path.parent().is_some_and(|d| d.is_dir()) {
            return;
        }
        match serde_json::to_vec(&events) {
            Ok(bytes) => {
                if let Err(e) = saba_atomic_file::write(&path, &bytes, saba_atomic_file::Mode::Plain) {
                    tracing::warn!("Failed to save exit history for '{}': {}", instance_id, e);
                }
            }
            Err(e) => tracing::warn!("Failed to encode exit history for '{}': {}", instance_id, e),
        }
    }

    /// 모니터가 정한 재시작을 가져간다. 대기 뒤 `restart_still_wanted`로 다시 확인하고 시작한다.
    pub fn take_pending_restarts(&mut self) -> Vec<(String, Duration, u64)> {
        std::mem::take(&mut self.pending_restarts)
    }

    /// 대기하는 동안 사용자가 정지·시작했거나 인스턴스가 지워졌으면 false.
    /// `generation`은 예약 때의 실행 세대 (꺼진 서버에 정지를 보내면 정지 표시가 바로 지워지므로 세대로 가른다)
    pub async fn restart_still_wanted(&self, instance_id: &str, generation: u64) -> bool {
        if self.runtime.existing(instance_id, |r| r.stop_requested.is_some()).unwrap_or(false)
            || self.instance_store.get(instance_id).is_none()
            || self.run_generation_of(instance_id) != generation
        {
            return false;
        }
        !matches!(self.managed_store.get(instance_id).await, Some(p) if p.is_running())
    }

    /// 자동 재시작이 실패했을 때 이력에 남긴다
    pub fn record_restart_failure(&mut self, instance_id: &str, attempt: u64, reason: String) {
        let noted = self.runtime.existing(instance_id, |r| {
            let last = r.exit_events.as_mut().and_then(|e| e.back_mut())?;
            last.note = Some(format!("restart failed: {}", reason));
            last.note_code = Some("restart_failed".into());
            last.note_args = Some(serde_json::json!({ "reason": reason }));
            Some((last.note_code.clone(), last.note_args.clone()))
        });
        if let Some((note_code, note_args)) = noted.flatten() {
            // `instance.exit`는 이미 나갔으므로 실패는 따로 알린다 (오류 원문이 있어 릴레이로는 올리지 않는다)
            crate::events::emit("instance.restart_failed", serde_json::json!({
                "instanceId": instance_id,
                "instance": self.instance_store.get(instance_id).map(|i| i.name.clone()).unwrap_or_default(),
                "noteCode": note_code,
                "noteArgs": note_args,
                "restartAttempt": attempt,
            }));
        }
        self.save_exit_events(instance_id);
    }

    /// 자동 재시작 대기가 끝났을 때 알린다: 다시 켜졌으면 `instance.restarted`,
    /// 대기 중에 정지·시작·삭제돼 그만뒀으면 `instance.restart_cancelled` (실패는 `record_restart_failure`).
    /// `attempt`는 시작을 알린 `instance.exit`의 `restartAttempt`: 그 사이 새 크래시로 새 시도가 시작됐으면 번호가 다르다
    pub fn record_restart_outcome(&self, instance_id: &str, attempt: u64, restarted: bool) {
        let kind = if restarted { "instance.restarted" } else { "instance.restart_cancelled" };
        crate::events::emit(kind, serde_json::json!({
            "instanceId": instance_id,
            "instance": self.instance_store.get(instance_id).map(|i| i.name.clone()).unwrap_or_default(),
            "restartAttempt": attempt,
        }));
    }

    /// 최근 종료 이력, 오래된 것부터
    pub fn exit_events(&self, instance_id: &str) -> Vec<crash::ExitEvent> {
        match self.runtime.existing(instance_id, |r| r.exit_events.as_ref().map(|e| e.iter().cloned().collect())).flatten() {
            Some(events) => events,
            None => self.saved_exit_events(instance_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::supervisor::start::announce_started;

    /// 켜진 뒤 곧 끝난 시작 실패는 그 시작을 시킨 쪽을 싣는다: 앱 버튼(`gui`)으로 켠 실패에는 앱이 효과음을 내지 않는다
    #[cfg(unix)]
    #[tokio::test]
    async fn start_failure_names_who_started_it() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), instances_dir.to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("sf", "game");
        inst.id = "start-fail-src".to_string();
        supervisor.instance_store.add(inst).unwrap();
        let work = tmp.path().join("sf");
        std::fs::create_dir_all(&work).unwrap();
        let proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), "exit 1".into()], work.to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        supervisor.managed_store.insert("start-fail-src", proc).await;
        let mut discord = crate::audit::Origin::new("discord");
        discord.user_name = Some("주인님".into());
        announce_started(&supervisor.runtime, "start-fail-src", "sf", &discord, false);
        for _ in 0..50 {
            if supervisor.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let mut exits = crate::events::bus().subscribe(None);
        supervisor.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        let exit = std::iter::from_fn(|| exits.rx.try_recv().ok())
            .find(|e| e.kind == "instance.exit" && e.data["instanceId"] == "start-fail-src")
            .unwrap();
        assert_eq!((exit.data["kind"].clone(), exit.data["source"].clone(), exit.data["userName"].clone()), (json!("start_failed"), json!("discord"), json!("주인님")));
    }

    /// 끝난 managed 서버를 크래시·정상 종료·사용자 정지·강제 종료로 가르고 크래시만 재시작한다 (A1)
    #[cfg(unix)]
    #[tokio::test]
    async fn exits_are_judged_and_only_crashes_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [detection]\ngraceful_exit_patterns = [\"Stopping the server\"]\ncrash_artifacts = [\"hs_err_pid*.log\"]\n",
        )
        .unwrap();
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), instances_dir.to_str().unwrap());

        let cases = [
            ("crash", "touch hs_err_pid1.log; exit 1", None),
            ("clean", "echo Stopping the server; exit 1", None),
            ("killed", "exit 1", None),
            ("stopped", "touch hs_err_pid1.log; exit 1", None),
            ("off", "touch hs_err_pid1.log; exit 1", Some(false)),
            ("reattached", "exit 1", None),
        ];
        for (id, script, auto_restart) in cases {
            let mut inst = crate::instance::ServerInstance::new(id, "game");
            inst.id = id.to_string();
            if let Some(flag) = auto_restart {
                inst.module_settings.insert("auto_restart".into(), json!(flag));
            }
            supervisor.instance_store.add(inst).unwrap();
            let work = tmp.path().join(id);
            std::fs::create_dir_all(&work).unwrap();
            let mut proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), script.into()], work.to_str().unwrap(), Vec::new(), None, None)
                .await
                .unwrap();
            // 켜지자마자 끝난 것(start_failed)과 가르려고, 한동안 돌다 끝난 것으로 둔다.
            // 다시 붙은 프로세스는 붙은 시각만 알므로 그대로 두어도 시작 실패가 아니다
            if id == "reattached" {
                proc.reattached = true;
            } else {
                proc.started_at -= 600;
            }
            supervisor.managed_store.insert(id, proc).await;
        }
        supervisor.runtime.with("stopped", |r| r.stop_requested = Some(crate::audit::Origin::new("gui")));
        for _ in 0..50 {
            if supervisor.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let mut exits = crate::events::bus().subscribe(None);
        supervisor.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        // 다시 띄우기로 한 종료는 그 복구 시도의 번호(예약한 재시작의 실행 세대)를 싣는다
        let crash_exit = std::iter::from_fn(|| exits.rx.try_recv().ok()).find(|e| e.kind == "instance.exit" && e.data["instanceId"] == "crash").unwrap();
        assert_eq!(crash_exit.data["restartAttempt"], 0);

        let kind = |id: &str| supervisor.exit_events(id).last().map(|e| e.verdict.kind);
        assert_eq!(kind("crash"), Some(crash::ExitKind::Crash));
        assert_eq!(kind("clean"), Some(crash::ExitKind::InGameStop));
        assert_eq!(kind("killed"), Some(crash::ExitKind::UnknownExit));
        assert_eq!(kind("stopped"), Some(crash::ExitKind::UserStop));
        assert_eq!(kind("off"), Some(crash::ExitKind::Crash));
        assert_eq!(kind("reattached"), Some(crash::ExitKind::UnknownExit), "a server running before the daemon restart did not fail to start");
        assert_eq!(supervisor.exit_events("off")[0].note.as_deref(), Some("auto_restart is off"));
        assert_eq!(supervisor.exit_events("off")[0].note_code.as_deref(), Some("auto_restart_off"));
        // 크래시·예기치 않은 종료만 작업 기록에 crash로 남는다
        let crashed = |id: &str| crate::audit::log().recent(10, Some(id)).iter().any(|v| v["action"] == "crash" && v["ok"] == false);
        assert!(crashed("crash") && crashed("killed") && crashed("off"));
        assert!(!crashed("clean") && !crashed("stopped"));
        let mut sub = crate::events::bus().subscribe(None);
        supervisor.record_restart_failure("crash", 0, "no java".into());
        let failed = supervisor.exit_events("crash").last().cloned().unwrap();
        assert_eq!((failed.note_code.as_deref(), failed.note_args), (Some("restart_failed"), Some(json!({"reason": "no java"}))));
        // 이미 나간 instance.exit 뒤에 실패를 따로 알린다
        let mut told = false;
        while let Ok(e) = sub.rx.try_recv() {
            told |= e.kind == "instance.restart_failed" && e.data["noteArgs"]["reason"] == "no java" && e.data["restartAttempt"] == 0;
        }
        assert!(told, "restart failure was not announced on the event stream");
        // 복구 시도의 끝도 알린다: 다시 켜졌거나, 대기 중에 정지·시작해서 그만뒀거나.
        // 어느 시도의 끝인지 시작(`instance.exit`)과 같은 `restartAttempt`로 가른다
        supervisor.record_restart_outcome("crash", 7, true);
        supervisor.record_restart_outcome("crash", 8, false);
        let ends: Vec<(String, Value)> = std::iter::from_fn(|| sub.rx.try_recv().ok())
            .filter(|e| e.data["instanceId"] == "crash")
            .map(|e| (e.kind.clone(), e.data["restartAttempt"].clone()))
            .collect();
        assert_eq!(ends, vec![("instance.restarted".to_string(), json!(7)), ("instance.restart_cancelled".to_string(), json!(8))]);
        assert_eq!(supervisor.take_pending_restarts(), vec![("crash".to_string(), Duration::from_secs(5), 0)]);
        assert!(supervisor.restart_still_wanted("crash", 0).await);
        supervisor.runtime.with("crash", |r| r.stop_requested = Some(crate::audit::Origin::new("gui")));
        assert!(!supervisor.restart_still_wanted("crash", 0).await, "a stop during the wait cancels the restart");
        // 꺼진 서버에 보낸 정지는 정지 표시를 바로 지운다: 그래도 대기 중 정지가 있었으면 재시작하지 않는다
        supervisor.runtime.with("crash", |r| r.stop_requested = None);
        let sup = tokio::sync::RwLock::new(supervisor);
        Supervisor::stop_server(&sup, "crash", "game", false).await.unwrap();
        assert!(!sup.read().await.restart_still_wanted("crash", 0).await, "a stop of the already exited server cancels the restart");

        // 데몬을 다시 켜도 종료 이력이 남는다
        let restarted = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), instances_dir.to_str().unwrap());
        assert_eq!(restarted.exit_events("crash").last().map(|e| e.verdict.kind), Some(crash::ExitKind::Crash));
    }
}
