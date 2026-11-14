//! 서버 상태 조회: 모듈 `status()`, 추적 중인 프로세스, 실행 세대

use super::*;

impl Supervisor {
    /// 인스턴스 id로 서버 상태를 본다 (응답의 `server`는 이름)
    /// Called by IPC API: GET /api/instance/:id/status
    pub async fn get_server_status(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str) -> Result<Value> {
        tracing::debug!("Getting status for server '{}'", instance_id);

        // 락 안에서는 값만 챙긴다. 익스텐션 hook과 모듈 status()는 락 밖에서 (C12)
        let (instance, hook) = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let module = s.module_loader.get_module(module_name).ok();
            let mut ctx = crate::extension::hook_context(&instance, &s.instance_store.instance_dir(&instance.id), module.as_ref().map(|m| &m.metadata));
            ctx["process_patterns"] = json!(module.map(|m| m.metadata.process_patterns).unwrap_or_default());
            let hook = (s.extension_manager.clone(), ctx);
            (instance, hook)
        };

        // ── Extension hook: server.status ──
        {
            let (ext_mgr, ctx) = hook;
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.status", ctx)).await;
            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        return Ok(val.clone());
                    }
                }
            }
        }

        let (call, tracked_pid) = {
            let s = sup.read().await;
            // Managed 프로세스가 있으면 plugin 호출 없이 직접 판단
            if s.managed_store.get(&instance.id).await.is_some() {
                let (running, pid, start_time) = s.tracked_state(&instance.id).await;
                return Ok(json!({
                    "server": instance.name,
                    "status": if running { "running" } else { "stopped" },
                    "online": running,
                    "pid": pid,
                    "start_time": start_time,
                }));
            }

            // Non-managed: module plugin으로 상태 확인

            // tracker에서 PID를 먼저 확인 — start_server()가 저장한 PID
            let tracked_pid = s.tracker.get_pid(&instance.id).ok();

            // Find module
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            // Build config — 실행 파일, PID 등 Python status()에 필요한 값 포함
            let mut config_obj = serde_json::Map::new();
            s.insert_resolved_exe(&mut config_obj, &instance, module_name);
            if let Some(pid) = tracked_pid {
                config_obj.insert("pid".to_string(), json!(pid));
            }
            // module_settings 포함 (server_name 등 Python이 INI 경로 해석에 사용)
            for (key, value) in instance.module_settings.iter().filter(|(key, _)| !crate::sandbox::grants_write(key)) {
                config_obj.entry(key.clone()).or_insert_with(|| value.clone());
            }
            (crate::plugin::PluginCall::new(module_path, "status", Value::Object(config_obj)).serving(tracked_pid).in_instance(s.host_context(&instance.id)), tracked_pid)
        };

        // Ask module for status
        match call.run().await {
            Ok(result) => {
                // 모듈에 status가 없다: 데몬이 추적하는 PID가 살아 있는지로 답한다
                if result.get("error_code").and_then(Value::as_str) == Some("not_supported") {
                    let pid = tracked_pid.filter(|pid| process::is_process_alive(*pid));
                    let start_time = match pid {
                        Some(_) => sup.read().await.tracker.get_start_time(&instance.id).ok(),
                        None => None,
                    };
                    return Ok(json!({
                        "server": instance.name,
                        "status": if pid.is_some() { "running" } else { "stopped" },
                        "pid": pid,
                        "start_time": start_time,
                    }));
                }
                if result.get("success").and_then(Value::as_bool) == Some(false) {
                    return Ok(result);
                }
                let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
                let pid = result.get("pid").and_then(|p| p.as_u64());
                let start_time = match pid {
                    Some(_) => sup.read().await.tracker.get_start_time(&instance.id).ok(),
                    None => None,
                };
                Ok(json!({
                    "server": instance.name,
                    "status": status,
                    "pid": pid,
                    "start_time": start_time,
                }))
            },
            Err(e) => {
                tracing::error!("Failed to get status: {}", e);
                let mut result = crate::plugin::error_response(&e);
                result["server"] = json!(instance.name);
                result["status"] = json!("unknown");
                result["pid"] = Value::Null;
                Ok(result)
            }
        }
    }

    /// 콘솔 명령을 stdin으로 보내야 하는가.
    /// 모듈이 `stdin`을 지원하고 stdin이 열린 managed 프로세스가 돌고 있으면 stdin을 쓴다
    /// (RCON은 보안상 늘 꺼져 있다).
    /// 데몬이 아는 실행 상태 `(실행 중, PID, 시작 시각)`: 서버 목록과 개별 상태 조회가 같이 쓴다.
    /// managed 프로세스가 있으면 그것이 살아 있는지로 정한다(끝났지만 모니터가 아직 치우지 않은 PID를 실행 중으로 보지 않는다).
    /// 없으면 추적 중인 PID가 있는지다
    pub async fn tracked_state(&self, instance_id: &str) -> (bool, Option<u32>, Option<u64>) {
        let pid = self.tracker.get_pid(instance_id).ok();
        let running = match self.managed_store.get(instance_id).await {
            Some(managed) => managed.is_running(),
            None => pid.is_some(),
        };
        if !running {
            return (false, None, None);
        }
        (true, pid, self.tracker.get_start_time(instance_id).ok())
    }

    /// 서버가 켜져 있는가: 추적표(non-managed와 managed 모두 띄운 뒤 올린다)나 managed 저장소에 살아 있으면.
    /// "켜져 있으면 거절" 판정은 모두 이것을 쓴다 (재부착 때 추적표 등록이 실패해 managed에만 남은 경우도 켜진 것이다)
    pub async fn is_running(&self, instance_id: &str) -> bool {
        self.tracker.get_pid(instance_id).is_ok() || self.managed_store.running_instance_ids().await.iter().any(|id| id == instance_id)
    }

    /// 켜져 있는 서버 인스턴스 id (`is_running` 판정)
    pub async fn running_instances(&self) -> Vec<String> {
        let mut running = Vec::new();
        for instance in self.instance_store.list() {
            if self.is_running(&instance.id).await {
                running.push(instance.id.clone());
            }
        }
        running
    }

    /// 지금 실행 세대 (시작할 때마다 1씩 오른다)
    pub fn run_generation_of(&self, instance_id: &str) -> u64 {
        self.runtime.existing(instance_id, |r| r.run_generation).unwrap_or_default()
    }
}
