//! 서버 정지: 익스텐션·모듈·콘솔 정지와 강제 종료

use super::*;

impl Supervisor {
    /// 인스턴스 id로 서버를 정지한다 (이름은 겹칠 수 있다)
    ///
    /// 데몬 락은 준비와 정리 때만 잡는다. 익스텐션 hook, 모듈 `stop()`, 종료 대기(최대
    /// `stop_timeout`초)는 락 밖에서 한다 (C12).
    pub async fn stop_server(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str, force: bool) -> Result<Value> {
        let result = Self::stop_server_inner(sup, instance_id, module_name, force).await;
        // 정지가 실패하면 정지 표시를 지운다: 남겨 두면 뒤에 오는 진짜 크래시를 "사용자가 멈춤"으로 판정한다
        let failed = match &result {
            Err(_) => true,
            Ok(v) => v.get("success").and_then(Value::as_bool) == Some(false),
        };
        if failed {
            let s = sup.write().await;
            s.runtime.existing(instance_id, |r| r.stop_requested = None);
        }
        result
    }

    async fn stop_server_inner(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str, force: bool) -> Result<Value> {
        tracing::info!("Stopping server '{}' (force: {})", instance_id, force);

        // ── 1) 준비 (쓰기 락: 정지 표시를 남긴다) ──
        let (instance, hook, managed, rt) = {
            let s = sup.write().await;
            let instance = s.instance_store.get(instance_id)
                .cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let generation = s.runtime.with(&instance.id, |r| {
                r.stop_requested = Some(crate::audit::current());
                r.run_generation += 1;
                r.run_generation
            });
            let _ = STOPPED_AT.try_with(|at| at.set(Some(generation)));
            let module = s.module_loader.get_module(module_name).ok();
            let ctx = crate::extension::hook_context(&instance, &s.instance_store.instance_dir(&instance.id), module.as_ref().map(|m| &m.metadata));
            let hook = (s.extension_manager.clone(), ctx);
            let managed = s.managed_store.get(&instance.id).await;
            (instance, hook, managed, s.runtime.clone())
        };
        // 받은 키가 id여도 메시지·응답에는 이름을 쓴다
        let server_name = instance.name.as_str();

        // ── Extension hook: server.stop ──
        // 정지 전에 불린다. 익스텐션(예: 컨테이너)이 정지를 맡으면 handled=true를 돌려준다.
        {
            let (ext_mgr, ctx) = hook;
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.stop", ctx)).await;
            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        sup.write().await.runtime.with(&instance.id, |r| r.stop_cooldown = Some(Instant::now()));
                        if val.get("success").and_then(Value::as_bool) != Some(false) {
                            announce_stopped(&instance.id, server_name, crash::ExitKind::UserStop, &crate::audit::current());
                        }
                        return Ok(val.clone());
                    }
                }
            }
            // 정지를 맡을 수 있는 익스텐션이 실패했으면 "안 돌고 있음"으로 덮지 않고 실패를 돌려준다
            let failed = results.iter().find_map(|(_, result)| match result {
                Err(e) => Some(crate::plugin::error_response(e)),
                Ok(val) if val.get("success").and_then(Value::as_bool) == Some(false) => Some(val.clone()),
                Ok(_) => None,
            });
            if let Some(mut error) = failed {
                error["server"] = json!(server_name);
                return Ok(error);
            }
        }

        // ── Native 모드 ──
        // 모듈 정보와 모듈 stop()에 넘길 값은 읽기 락 안에서 만든다
        let (module, stop_config, tracked_pid, host_context) = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let mut config_obj = serde_json::Map::new();
            s.insert_resolved_exe(&mut config_obj, &instance, module_name);
            Self::insert_instance_fields(&mut config_obj, &instance, &s.instance_store.instance_dir(&instance.id));
            // 모듈이 기다릴 시간은 데몬이 기다리는 시간과 같다 (인스턴스 값, 모듈 값, 기본 30초 순)
            config_obj.insert("stop_timeout".to_string(), json!(stop_timeout_of(&instance, module.metadata.stop_timeout)));
            // 추적표는 id로만 찾는다 (다른 인스턴스 id와 이름이 같아도 섞이지 않게)
            let tracked_pid = s.tracker.get_pid(&instance.id).ok();
            (module, config_obj, tracked_pid, s.host_context(&instance.id))
        };
        let module_path = module.entry_path();

        // ── Managed mode: stdin에 stop_command 전송으로 graceful shutdown ──
        if let Some(managed) = managed {
            if managed.is_running() {
                // 정지 방법
                // - force: 즉시 PID 강제 종료
                // - 모듈이 `stop_signal = "interrupt"`: Ctrl+C (보내지 못하면 다음 길로)
                // - 모듈이 stdin을 받고 stdin이 열려 있음: stop_command를 stdin으로
                // - 그 밖(REST 전용 게임, stdin 없는 재부착): 모듈 lifecycle.py `stop()`에
                //   PID와 managed=true를 넘겨 맡긴다. 시간 안에 안 끝나면 아래에서 강제 종료.
                let module_has_stdin = module.metadata.supports_protocol("stdin");
                let via_stdin = module_has_stdin && managed.is_stdin_available();
                // 모듈에서 stop_command를 가져옴 (없으면 "stop" 기본값)
                let stop_cmd = module.metadata.stop_command.clone().unwrap_or_else(|| "stop".to_string());

                let wants_interrupt = module.metadata.stop_signal == Some(module_loader::StopSignal::Interrupt);

                // 준비 문구 전에 받은 명령을 실패하는 서버가 있다 (마인크래프트 26.3은 오류만 찍고 계속 돈다).
                // Ctrl+C 저장 처리기도 준비 뒤에야 붙는다. 켜지는 중이면 준비될 때까지(정지 대기 시간 안에서) 들고 있다가 보낸다
                if !force && (wants_interrupt || via_stdin) && ready::is_waiting(&rt, &instance.id) {
                    tracing::info!("'{}' is still starting; holding the stop until it is ready", server_name);
                    let hold = Duration::from_secs(stop_timeout_of(&instance, module.metadata.stop_timeout));
                    let until = tokio::time::Instant::now() + hold;
                    while ready::is_waiting(&rt, &instance.id) && managed.is_running() && tokio::time::Instant::now() < until {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }

                // Ctrl+C로 저장하고 끝내는 서버 (`[config] stop_signal = "interrupt"`). 보내지 못하면 아래 길로 넘어간다
                let interrupted = !force
                    && wants_interrupt
                    && match managed.interrupt().await {
                        Ok(()) => {
                            tracing::info!("Sent Ctrl+C to managed server '{}'", server_name);
                            true
                        }
                        Err(e) => {
                            tracing::warn!("Could not send Ctrl+C to '{}': {}", server_name, e);
                            false
                        }
                    };

                if force {
                    tracing::info!("Force-killing managed server '{}'", server_name);
                    if let Err(e) = managed.force_kill().await {
                        tracing::warn!("Force kill of '{}' failed: {}", server_name, e);
                    }
                } else if interrupted {
                    // 서버가 저장하고 스스로 끝나기를 아래에서 기다린다
                } else if via_stdin {
                    tracing::info!("Sending '{}' to managed server '{}' via stdin", stop_cmd, server_name);
                    if let Err(e) = managed.send_command(&stop_cmd).await {
                        tracing::warn!("Failed to send stop command via stdin: {}", e);
                    }
                } else {
                    tracing::info!("Asking module '{}' to stop managed server '{}'", module_name, server_name);
                    let mut config_obj = stop_config;
                    config_obj.insert("pid".to_string(), json!(managed.pid));
                    config_obj.insert("managed".to_string(), json!(true));
                    config_obj.insert("force".to_string(), json!(false));
                    config_obj.insert("stop_call_limit".to_string(), json!(crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS));
                    if let Some(port) = instance.rest_port {
                        config_obj.insert("rest_port".to_string(), json!(port));
                    }
                    match crate::plugin::PluginCall::new(module_path.clone(), "stop", Value::Object(config_obj)).serving(Some(managed.pid)).in_instance(host_context.clone()).run().await {
                        // 저장 후 종료를 요청할 길이 없다: 기다렸다 강제 종료하지 않고 사용자가 고르게 돌려준다 (MK10)
                        Ok(result) if is_graceful_stop_unavailable(&result) => return Ok(graceful_stop_unavailable(server_name, &result)),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Module stop for '{}' failed: {} (will force kill on timeout)", server_name, e),
                    }
                }

                // 프로세스 종료 대기 (force kill 시 5초, graceful 시 인스턴스 설정 `stop_timeout`, 모듈 값, 기본 30초 순)
                let timeout_secs = if force { 5 } else { stop_timeout_of(&instance, module.metadata.stop_timeout) };
                let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
                loop {
                    if !managed.is_running() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!("Managed server '{}' did not exit in {}s, force killing", server_name, timeout_secs);
                        if let Err(e) = managed.force_kill().await {
                            tracing::warn!("Force kill of '{}' failed: {}", server_name, e);
                        }
                        // 끝난 것을 확인하기 전에는 추적을 놓지 않는다 (종료가 거부됐으면 아직 돌고 있다)
                        if !process::wait_gone(Duration::from_secs(5), || managed.is_running()).await {
                            return Err(anyhow::anyhow!("Server '{}' is still running: force kill did not take effect", server_name));
                        }
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }

                let mut s = sup.write().await;
                // managed store에서 제거 (그새 모니터가 치웠으면 그쪽이 이미 종료를 기록했다)
                if s.managed_store.get(&instance.id).await.is_some_and(|p| Arc::ptr_eq(&p, &managed)) {
                    s.managed_store.remove(&instance.id).await;
                    // 종료 기록 (user_stop)과 정지 표시 정리
                    s.judge_exit(&instance.id, &managed, false).await;
                    if force {
                        s.runtime.existing(&instance.id, |r| {
                            if let Some(last) = r.exit_events.as_mut().and_then(|e| e.back_mut()) {
                                last.verdict.evidence = "force stop requested through saba-chan".into();
                            }
                        });
                        s.save_exit_events(&instance.id);
                    }
                }

                // Clean up managed.pid
                let pid_file = s.instance_store.managed_pid_file(&instance.id);
                let _ = std::fs::remove_file(&pid_file);

                tracing::info!("Managed server '{}' stopped", server_name);
                // auto-detect 쿨다운 등록
                s.runtime.with(&instance.id, |r| r.stop_cooldown = Some(Instant::now()));
                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' stopped", server_name)
                }));
            } else {
                // 프로세스가 이미 종료됨 (유저가 콘솔에서 직접 stop 명령어 입력 등)
                // managed store 정리 후 정상 종료로 간주
                tracing::info!(
                    "Managed server '{}' already exited (user-initiated stop command), cleaning up",
                    server_name
                );
                let mut s = sup.write().await;
                // 이미 스스로 끝났다: 정지 요청 때문이 아니므로 표시를 지우고 기록한다
                s.runtime.existing(&instance.id, |r| r.stop_requested = None);
                // 크래시였어도 다시 켜지 않는다: 사용자가 정지를 눌렀다 (D-S6)
                if let Some(proc) = s.managed_store.remove(&instance.id).await {
                    s.judge_exit(&instance.id, &proc, false).await;
                }
                let pid_file = s.instance_store.managed_pid_file(&instance.id);
                let _ = std::fs::remove_file(&pid_file);
                s.runtime.with(&instance.id, |r| r.stop_cooldown = Some(Instant::now()));

                // tracker에서도 제거
                let _ = s.tracker.untrack(&instance.id);

                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' already stopped", server_name)
                }));
            }
        }

        // ── Non-managed mode: Python lifecycle.py를 통한 정지 ──
        // 데몬이 추적하는 PID가 없으면 멈출 것이 없다. 모듈 stop()을 PID 없이 부르면
        // 이미지 이름으로 같은 게임의 다른 서버까지 끝낼 수 있다
        let Some(pid) = tracked_pid else {
            sup.write().await.runtime.existing(&instance.id, |r| r.stop_requested = None);
            return Ok(json!({
                "success": true,
                "server": server_name,
                "was_running": false,
                "message": format!("Server '{}' is not running", server_name)
            }));
        };
        let mut config_obj = stop_config;
        // Pass PID from tracker so the module can kill the actual process
        config_obj.insert("pid".to_string(), json!(pid));
        config_obj.insert("force".to_string(), json!(force));
        // managed 서버는 위에서 데몬이 직접 멈춘다. 여기까지 오면 항상 non-managed.
        config_obj.insert("managed".to_string(), json!(false));
        // 모듈이 `stop_timeout`만큼 기다리므로 호출은 그보다 조금 더 기다린다. 그 한도를 모듈에도 알린다
        let limit = module_stop_limit(stop_timeout_of(&instance, module.metadata.stop_timeout));
        config_obj.insert("stop_call_limit".to_string(), json!(limit));
        let config = Value::Object(config_obj);

        // Execute stop function
        let result = crate::plugin::PluginCall::new(module_path.clone(), "stop", config).serving(Some(pid)).in_instance(host_context).timeout(limit).run().await?;

        // Check if the module actually succeeded
        let plugin_success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        let plugin_message = result.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        if !force && is_graceful_stop_unavailable(&result) {
            return Ok(graceful_stop_unavailable(server_name, &result));
        }
        if plugin_success {
            tracing::info!("Server '{}' stopped successfully: {}", server_name, plugin_message);
            // auto-detect 쿨다운 등록
            let s = sup.write().await;
            // 정지 표시는 다음 시작까지 남긴다: 모니터가 사라진 PID를 원인 모를 종료로 보지 않게
            let origin = s.runtime.with(&instance.id, |r| {
                r.stop_cooldown = Some(Instant::now());
                r.stop_requested.clone()
            });
            let origin = origin.unwrap_or_else(crate::audit::current);
            announce_stopped(&instance.id, server_name, crash::ExitKind::UserStop, &origin);
            Ok(json!({
                "success": true,
                "server": server_name,
                "message": format!("Server '{}' stopped", server_name)
            }))
        } else {
            tracing::error!("Failed to stop server '{}': {}", server_name, plugin_message);
            Err(module_failure(&result, module_name, "stop"))
        }
    }
}

/// non-managed 정지에서 모듈 `stop()` 호출의 시간 제한: 모듈이 기다리는 `stop_timeout`에 저장·정리 여유 30초
pub(crate) fn module_stop_limit(stop_timeout: u64) -> u64 {
    stop_timeout + 30
}

/// 모듈 `stop()`이 저장 후 종료를 요청할 길이 없다고 답했는가 (콘솔·REST가 없는 서버. 모듈은 이때 서버를 끝내지 않는다)
fn is_graceful_stop_unavailable(result: &Value) -> bool {
    result.get("success").and_then(Value::as_bool) != Some(true) && result.get("error_code").and_then(Value::as_str) == Some("graceful_stop_unavailable")
}

/// 정지하지 않았다는 응답. 강제 종료(저장 없음)는 사용자가 `force`로 다시 요청한다
fn graceful_stop_unavailable(server_name: &str, result: &Value) -> Value {
    tracing::warn!("Server '{}' has no way to save and exit; waiting for a force stop", server_name);
    let mut response = result.clone();
    response["success"] = json!(false);
    response["server"] = json!(server_name);
    if response.get("error").is_none() {
        response["error"] = result.get("message").cloned().unwrap_or_else(|| json!("graceful stop unavailable"));
    }
    response
}

/// `instance.stopped`: 사바쨩을 거쳐 끄거나(`user_stop`) 게임 안에서 끈(`in_game_stop`) 정상 정지.
/// 크래시·원인 모를 종료는 `instance.exit`만 알린다. 접속자가 없어 자동 정지(`idle`)는 비어 있던 분(`idleMinutes`)을 싣는다
pub(super) fn announce_stopped(instance_id: &str, name: &str, kind: crash::ExitKind, origin: &crate::audit::Origin) {
    let mut data = origin_fields(origin);
    data.insert("instanceId".into(), json!(instance_id));
    data.insert("instance".into(), json!(name));
    data.insert("kind".into(), json!(kind));
    if let Some(stopped) = idle::stopped(instance_id).filter(|_| origin.source == "idle") {
        data.insert("idleMinutes".into(), json!(stopped.minutes));
    }
    crate::events::emit("instance.stopped", Value::Object(data));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// D-S10: 정지는 자기가 올린 세대를 알려 준다. 재시작이 정지 전에 따로 읽은 세대를 기준으로 삼으면,
    /// 그 틈에 끼어든 자동 재시작 시작을 "다른 정지·시작"으로 보고 스스로 취소해 서버가 꺼진 채 남았다
    #[tokio::test]
    async fn a_stop_reports_the_generation_it_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.module_loader.discover_modules().unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        // 정지 전에 다른 쪽(자동 재시작)이 세대를 올려 둔다
        sup.runtime.with(&id, |r| r.run_generation += 5);
        let sup = tokio::sync::RwLock::new(sup);
        let stopped_at = STOPPED_AT
            .scope(std::cell::Cell::new(None), async {
                let _ = Supervisor::stop_server(&sup, &id, "game", false).await;
                STOPPED_AT.with(|c| c.get())
            })
            .await;
        assert_eq!(stopped_at, Some(6));
        assert_eq!(sup.read().await.run_generation_of(&id), 6);
    }

    #[test]
    fn a_legacy_stop_unavailable_is_still_a_failure() {
        let result = graceful_stop_unavailable("game", &json!({ "error_code": "graceful_stop_unavailable", "message": "no stop path", "module": "game", "function": "stop" }));
        assert_eq!(result["success"], false);
        assert_eq!(result["message"], "no stop path");
        assert_eq!(result["module"], "game");
        assert_eq!(result["function"], "stop");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_right_after_a_crash_does_not_restart_the_server() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[detection]\ncrash_artifacts = [\"hs_err_pid*.log\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "crashed".into();
        supervisor.instance_store.add(inst).unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), "touch hs_err_pid1.log; exit 1".into()], work.to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        proc.started_at -= 600;
        supervisor.managed_store.insert("crashed", proc).await;
        for _ in 0..50 {
            if supervisor.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let sup = tokio::sync::RwLock::new(supervisor);
        Supervisor::stop_server(&sup, "crashed", "game", false).await.unwrap();

        let mut s = sup.write().await;
        assert_eq!(s.exit_events("crashed").last().map(|e| e.verdict.kind), Some(crash::ExitKind::Crash), "it did crash");
        assert!(s.take_pending_restarts().iter().all(|(id, ..)| id != "crashed"), "but the stop keeps it down");
    }

    /// 콘솔이 있는 managed 서버의 일반 정지는 stop_command를 stdin으로 보내고 스스로 끝나기를 기다린다
    /// (강제 종료 먼저가 아니다: 세이브를 건너뛰지 않게)
    #[cfg(unix)]
    #[tokio::test]
    async fn normal_stop_asks_the_console_and_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("console-game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"console-game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_command = \"quit-now\"\n\n[protocols]\nsupported = [\"stdin\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut inst = crate::instance::ServerInstance::new("srv", "console-game");
        inst.id = "srv".into();
        supervisor.instance_store.add(inst).unwrap();
        // 받은 줄을 남기고 스스로 끝나는 "게임 서버"
        let got = tmp.path().join("got");
        let script = format!("read line; printf '%s' \"$line\" > '{}'; exit 0", got.display());
        let proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), script], tmp.path().to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        supervisor.managed_store.insert("srv", proc).await;

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let started = std::time::Instant::now();
        let result = Supervisor::stop_server(&supervisor, "srv", "console-game", false).await.unwrap();
        assert_eq!(result["success"], true, "{}", result);
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "quit-now", "the stop command went over stdin");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it exited on its own, not after the kill timeout");
    }

    /// `[config] stop_signal = "interrupt"`인 모듈의 일반 정지는 콘솔 명령 대신 Ctrl+C(unix는 SIGINT)를 보내고
    /// 서버가 저장하고 스스로 끝나기를 기다린다 (발헤임처럼 Ctrl+C에만 저장 처리기가 붙은 서버)
    #[cfg(unix)]
    #[tokio::test]
    async fn normal_stop_sends_ctrl_c_when_the_module_asks_for_it() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("ctrlc-game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"ctrlc-game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_signal = \"interrupt\"\n\n[protocols]\nsupported = [\"stdin\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut inst = crate::instance::ServerInstance::new("srv", "ctrlc-game");
        inst.id = "srv".into();
        supervisor.instance_store.add(inst).unwrap();
        // Ctrl+C를 받으면 "저장"하고 끝나는 서버. 콘솔 줄을 받으면 그 줄을 남긴다(오면 안 된다)
        let got = tmp.path().join("got");
        let armed = tmp.path().join("armed");
        let script = format!(
            "trap 'printf saved > \"{0}\"; exit 0' INT; : > \"{1}\"; while :; do if read -t 1 line; then printf '%s' \"$line\" > \"{0}\"; fi; done",
            got.display(),
            armed.display()
        );
        let proc = managed_process::ManagedProcess::spawn("bash", &["-c".into(), script], tmp.path().to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        supervisor.managed_store.insert("srv", proc).await;
        // trap이 걸리기 전에 보낸 SIGINT는 bash를 저장 없이 끝낸다. 걸린 뒤에 정지한다
        for _ in 0..100 {
            if armed.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(armed.exists(), "the test server never set its Ctrl+C handler");

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let started = std::time::Instant::now();
        let result = Supervisor::stop_server(&supervisor, "srv", "ctrlc-game", false).await.unwrap();
        assert_eq!(result["success"], true, "{}", result);
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "saved", "Ctrl+C, not a console line");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it exited on its own, not after the kill timeout");
    }

    /// Ctrl+C로 끝내는 서버도 켜지는 중에 정지하면 준비된 뒤 Ctrl+C를 받는다.
    /// 저장 처리기는 준비 뒤에야 붙는다: 그 전에 보내면 저장 없이 죽는다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_ctrl_c_stop_while_starting_waits_for_the_ready_line() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("slow-ctrlc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"slow-ctrlc\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [config]\nstop_signal = \"interrupt\"\nstop_timeout = 4\n\n[protocols]\nsupported = [\"stdin\"]\n\n\
             [extensions.saba-core]\nready_patterns = [\"^Done\"]\n",
        )
        .unwrap();
        let saved = tmp.path().join("saved");
        let game = format!(
            "import signal, sys, time\n\
             print('booting', flush=True)\n\
             time.sleep(1.5)\n\
             def save(*_):\n    open({saved:?}, 'w').write('saved')\n    sys.exit(0)\n\
             signal.signal(signal.SIGINT, save)\n\
             print('Done (1.5s)!', flush=True)\n\
             while True:\n    time.sleep(0.1)\n",
            saved = saved.display().to_string()
        );
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys\n\
                 if sys.argv[1] == 'get_launch_command':\n    \
                 print(json.dumps({{'success': True, 'program': 'python3', 'args': ['-c', {game:?}], 'working_dir': '.'}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n"
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("c-early", "slow-ctrlc");
        inst.id = "c-early".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let started = Supervisor::start_managed_server(&sup, "c-early", "slow-ctrlc", json!({})).await.unwrap();
        assert_eq!(started["success"], true, "{started}");

        let stopped = Supervisor::stop_server(&sup, "c-early", "slow-ctrlc", false).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        assert_eq!(std::fs::read_to_string(&saved).ok().as_deref(), Some("saved"), "Ctrl+C reached the ready server");
    }

    /// 준비 문구 전에 받은 명령을 실패하는 서버(마인크래프트 26.3: "An unexpected error occurred")도
    /// 켜지는 중에 정지하면 준비된 뒤 stop_command를 받아 스스로 저장하고 끝난다 (강제 종료로 넘어가지 않는다)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_while_starting_waits_for_the_ready_line() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("slow-console");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"slow-console\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [config]\nstop_command = \"quit-now\"\nstop_timeout = 4\n\n[protocols]\nsupported = [\"stdin\"]\n\n\
             [extensions.saba-core]\nready_patterns = [\"^Done\"]\n",
        )
        .unwrap();
        // 준비 전에 온 줄은 버리고(실패), 준비 뒤의 quit-now에만 저장 표시를 남기고 끝나는 서버
        let saved = tmp.path().join("saved");
        let game = format!(
            "import os, select, sys, time\n\
             print('booting', flush=True)\n\
             time.sleep(1.5)\n\
             while select.select([0], [], [], 0)[0] and os.read(0, 4096):\n    pass\n\
             print('Done (1.5s)!', flush=True)\n\
             for line in sys.stdin:\n    \
             if line.strip() == 'quit-now':\n        open({saved:?}, 'w').write('saved')\n        sys.exit(0)\n",
            saved = saved.display().to_string()
        );
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys\n\
                 if sys.argv[1] == 'get_launch_command':\n    \
                 print(json.dumps({{'success': True, 'program': 'python3', 'args': ['-c', {game:?}], 'working_dir': '.'}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n"
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("s-early", "slow-console");
        inst.id = "s-early".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let started = Supervisor::start_managed_server(&sup, "s-early", "slow-console", json!({})).await.unwrap();
        assert_eq!(started["success"], true, "{started}");

        let stopped = Supervisor::stop_server(&sup, "s-early", "slow-console", false).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        assert_eq!(std::fs::read_to_string(&saved).ok().as_deref(), Some("saved"), "the stop command reached the ready server");
    }
}
