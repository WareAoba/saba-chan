//! 게임 명령과 콘솔: 모듈 `command()`, stdin·RCON 경로, 콘솔 출력

use super::*;

impl Supervisor {
    /// 서버에 명령어 실행
    pub async fn execute_command(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: Value,
    ) -> Result<String> {
        let call = {
            let s = sup.read().await;
            tracing::info!(
                "Supervisor executing command '{}' for instance '{}' (module: {})",
                command,
                instance_id,
                module_name
            );

            // 인스턴스 정보 얻기
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            // 모듈 찾기
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            // 명령어 config 구성
            let pid = s.tracker.get_pid(&instance.id).ok();
            let instance_dir = s.instance_store.instance_dir(&instance.id);
            let working_dir = instance.working_dir.clone()
                .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());
            let config = json!({
                "command": command,
                "args": args,
                "protocol_mode": &instance.protocol_mode,
                "working_dir": &working_dir,
                "rest_host": module.metadata.rest_host(instance),
                "rest_port": module.metadata.rest_port(instance),
                "pid": pid,
                "instance_id": instance_id,
            });

            // 플러그인 실행
            crate::plugin::PluginCall::new(module_path, "command", config).serving(pid).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;

        // 결과 처리
        if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
            if success {
                let message = result
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default();
                Ok(message.to_string())
            } else {
                Err(module_failure(&result, module_name, "command"))
            }
        } else {
            Err(anyhow::anyhow!("Invalid response from module"))
        }
    }

    pub async fn routes_commands_to_stdin(&self, instance_id: &str) -> bool {
        if !self.module_accepts_stdin(instance_id) {
            return false;
        }
        match self.managed_store.get(instance_id).await {
            Some(proc) => proc.is_running() && proc.is_stdin_available(),
            None => false,
        }
    }

    /// 명령을 RCON으로 받을 인스턴스면 그 모듈의 기본 RCON 포트: RCON만 받는 모듈, 또는 stdin·rcon을 받는 모듈인데
    /// 관리형 시작을 꺼서 콘솔 stdin이 없는 인스턴스 (사용자 정의 2026-11-08)
    fn rcon_port_for(&self, instance: &crate::instance::ServerInstance) -> Option<u16> {
        let meta = &self.module_loader.get_module(&instance.module_name).ok()?.metadata;
        meta.rcon_only_port().or_else(|| meta.rcon_port_without_console().filter(|_| !self.uses_managed_start(instance)))
    }

    /// 명령을 RCON으로 받을 인스턴스면(`rcon_port_for`) 접속 정보를 만들어 둔다 (시작 전). 그 밖에는 아무것도 하지 않는다
    /// RCON을 더 쓰지 않으면(관리형 시작을 켬, 모듈 업데이트로 stdin·rest가 생김) 남은 접속 정보를 지운다.
    /// 준비하지 못하면 시작을 멈춘다: 비밀번호 없이 띄우면 명령을 보낼 길이 없다
    pub(crate) fn ensure_rcon_access(&self, instance: &crate::instance::ServerInstance) -> Result<()> {
        let dir = self.instance_store.instance_dir(&instance.id);
        let Some(port) = self.rcon_port_for(instance) else {
            let _ = std::fs::remove_file(dir.join(crate::protocol::rcon::ACCESS_FILE));
            return Ok(());
        };
        // 두 인스턴스가 동시에 처음 켜져도 같은 포트를 고르지 않게, 고르고 저장하는 동안 하나씩만
        static PICKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _picking = PICKING.lock().unwrap_or_else(|e| e.into_inner());
        // 다른 인스턴스의 RCON 포트와 저장해 둔 포트(게임·REST 등), 이 인스턴스의 포트는 고르지 않는다
        let taken: Vec<u16> = self
            .instance_store
            .list()
            .iter()
            .filter(|other| other.id != instance.id)
            .flat_map(|other| {
                let rcon = crate::protocol::rcon::load(&self.instance_store.instance_dir(&other.id)).map(|a| a.port);
                rcon.into_iter().chain(self.saved_ports(other))
            })
            .chain(self.saved_ports(instance))
            .collect();
        crate::protocol::rcon::ensure(&dir, port, &taken)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("Could not prepare RCON access for '{}': {}", instance.name, e))
    }

    /// 명령을 RCON으로 보낼 인스턴스면 접속 정보: 모듈이 RCON을 받고, 시작할 때 접속 정보를 만들어 둔 경우.
    /// 관리형 시작 설정은 보지 않는다: 켜진 동안 설정을 바꿔도(재시작 필요) 지금 도는 서버는 시작 때 방식 그대로다.
    /// 접속 정보는 시작 준비(`ensure_rcon_access`)만 만들고 지우므로 지금 도는 서버의 방식을 따른다
    pub fn rcon_route(&self, instance_id: &str) -> Option<crate::protocol::rcon::Access> {
        let instance = self.instance_store.get(instance_id)?;
        self.module_loader.get_module(&instance.module_name).ok()?.metadata.rcon_port_without_console()?;
        crate::protocol::rcon::load(&self.instance_store.instance_dir(instance_id))
    }

    /// 인스턴스의 모듈이 콘솔 입력(`protocols.supported`에 "stdin")을 받는가
    pub fn module_accepts_stdin(&self, instance_id: &str) -> bool {
        let Some(instance) = self.instance_store.get(instance_id) else { return false };
        let Ok(modules) = self.module_loader.modules() else { return false };
        modules
            .iter()
            .find(|m| m.metadata.name == instance.module_name)
            .is_some_and(|m| m.metadata.supports_protocol("stdin"))
    }

    /// Send a command to a managed process's stdin
    pub async fn send_stdin_command(&self, instance_id: &str, command: &str) -> Result<String> {
        let proc = self.managed_store.get(instance_id).await
            .ok_or_else(|| anyhow::anyhow!("No managed process for instance '{}'", instance_id))?;

        if !proc.is_running() {
            return Err(anyhow::anyhow!("Process is no longer running"));
        }

        proc.send_command(command).await?;
        Ok(format!("Sent to stdin: {}", command))
    }

    /// Get console output from a managed process.
    /// Falls back to reading from the disk log file if no managed process is
    /// currently registered (e.g. after daemon restart before reattachment, or
    /// for recently stopped servers).
    pub async fn get_console_output(
        &self,
        instance_id: &str,
        since_id: Option<u64>,
        count: Option<usize>,
    ) -> Result<Value> {
        // Try in-memory managed process first
        if let Some(proc) = self.managed_store.get(instance_id).await {
            let lines = if let Some(since) = since_id {
                proc.get_console_since(since).await
            } else {
                proc.get_recent_console(count.unwrap_or(100)).await
            };

            return Ok(json!({
                "success": true,
                "lines": lines,
                "epoch": proc.console_epoch().await,
                "running": proc.is_running(),
                // `/stdin`과 같은 판정: REST 전용 모듈은 러너가 stdin을 쥐어도 입력을 받지 않는다
                "stdin_available": self.module_accepts_stdin(instance_id) && proc.is_stdin_available(),
            }));
        }

        // Fallback: read from disk log
        let inst_dir = self.instance_store.instance_dir(instance_id);
        let log_path = managed_process::latest_console_log(&inst_dir);
        if log_path.exists() {
            let effective_count = count.unwrap_or(100);
            let dir_owned = inst_dir.clone();
            let lines = tokio::task::spawn_blocking(move || {
                managed_process::read_console_log_from_disk(&dir_owned, effective_count)
            }).await.unwrap_or_default();

            // Note: since_id is ignored for disk fallback because disk log IDs
            // are re-assigned from 0 on each read and don't correspond to the
            // in-memory session's IDs.

            return Ok(json!({
                "success": true,
                "lines": lines,
                "running": false,
                "source": "disk",
            }));
        }

        if self.instance_store.get(instance_id).is_none() {
            anyhow::bail!("Instance not found: {}", instance_id);
        }
        // 한 번도 시작하지 않은 서버: 빈 콘솔
        Ok(json!({ "success": true, "lines": [], "running": false, "source": "none" }))
    }

    /// `get_console_output`에 줄마다 `spans`(의미 토큰 구간, UTF-16 오프셋)를 붙인다 (`?spans=1`)
    pub async fn get_console_output_with_spans(
        &self,
        instance_id: &str,
        since_id: Option<u64>,
        count: Option<usize>,
    ) -> Result<Value> {
        let mut result = self.get_console_output(instance_id, since_id, count).await?;
        let module = self.instance_store.get(instance_id).map(|i| i.module_name.clone()).unwrap_or_default();
        let loaded = self.module_loader.get_module(&module).ok();
        let hl = highlight::for_module(&module, loaded.as_ref().and_then(|m| m.metadata.syntax_highlight.as_ref()));
        if let Some(lines) = result.get_mut("lines").and_then(Value::as_array_mut) {
            for line in lines {
                let spans: Vec<Value> = line
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|c| hl.spans(c).into_iter().map(|(s, e, t)| json!([s, e, t])).collect())
                    .unwrap_or_default();
                if let Some(obj) = line.as_object_mut() {
                    obj.insert("spans".to_string(), Value::Array(spans));
                }
            }
        }
        Ok(result)
    }
}
