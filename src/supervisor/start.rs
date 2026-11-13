//! 서버 시작: 시작 전 점검, 일반 시작과 managed 시작, 시작 중 정지 처리

use super::*;
use super::reattach::forget_legacy_run;

/// 시작하는 동안 쥐는 표시. 놓으면(drop) 지워진다
struct StartGuard {
    runtime: runtime::Runtimes,
    id: String,
    /// 시작을 받았을 때의 실행 세대. 정지 요청은 세대를 올리므로, 띄우기 전후에 이 값과 다르면
    /// 시작 준비 중에 정지가 들어온 것이다 (그 정지는 PID가 없어 바로 성공으로 끝난다)
    generation: u64,
}

/// `StartGuard`의 기준 세대: 시작 API가 미리 잡아 두었으면 그 값, 아니면 지금 값
fn start_baseline(s: &Supervisor, id: &str) -> u64 {
    START_SINCE.try_with(|g| *g).unwrap_or_else(|_| s.run_generation_of(id))
}

impl StartGuard {
    fn claim(runtime: &runtime::Runtimes, id: &str, generation: u64) -> Option<Self> {
        let fresh = runtime.with(id, |r| !std::mem::replace(&mut r.starting, true));
        fresh.then(|| Self { runtime: runtime.clone(), id: id.to_string(), generation })
    }

    /// 시작 준비 중에 정지(또는 삭제)가 들어왔는가
    fn stop_requested(&self, s: &Supervisor) -> bool {
        s.run_generation_of(&self.id) != self.generation || s.instance_store.get(&self.id).is_none()
    }
}

/// 시작 전에 예약 범위를 피해 옮긴 포트를 시작 응답(`success:false`도)에 싣는다: `port_moved: [{field, from, to}]`.
/// 응답을 받지 않는 경로를 위해 옮길 때 `instance.port_moved` 이벤트도 낸다 (`move_default_rest_port_if`)
fn with_port_moved(mut result: Value, moved: Option<(u16, u16)>) -> Value {
    if let (Some((from, to)), Value::Object(map)) = (moved, &mut result) {
        map.insert("port_moved".to_string(), json!([{ "field": "rest_port", "from": from, "to": to }]));
    }
    result
}

/// 시작 준비 중에 정지가 들어와 띄우지 않았다 (또는 띄운 것을 바로 껐다)
fn start_cancelled(server_name: &str) -> Value {
    json!({
        "success": false,
        "error": "start_cancelled",
        "error_code": "start_cancelled",
        "message": format!("Server '{}' was stopped while it was starting", server_name),
    })
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        self.runtime.existing(&self.id, |r| r.starting = false);
    }
}

/// 모듈이 돌려준 REST 자격 증명을 인스턴스에 옮긴다: `get_launch_command`의 `rest_credentials`, 없으면 `[credential_map]`으로
/// 게임 설정 키(`module_settings` 또는 최상위)에서 읽는다. 바뀐 것이 있으면 true
fn sync_rest_credentials(
    instance: &mut crate::instance::ServerInstance,
    launch_result: &Value,
    credential_map: &std::collections::HashMap<String, String>,
) -> bool {
    // 1) 명시적 rest_credentials (모듈이 직접 반환한 경우)
    // 2) 없으면 credential_map 기반 (module.toml에 선언된 매핑): launch_result의 module_settings 또는 최상위에서 게임 키 값을 찾는다
    let given: Vec<(&str, &str)> = match launch_result.get("rest_credentials").and_then(|v| v.as_object()) {
        Some(rest_creds) => ["rest_username", "rest_password"]
            .into_iter()
            .filter_map(|key| rest_creds.get(key).and_then(|v| v.as_str()).map(|value| (key, value)))
            .collect(),
        None => credential_map
            .iter()
            .filter_map(|(daemon_key, game_key)| {
                let value = launch_result.get("module_settings")
                    .and_then(|ms| ms.get(game_key))
                    .and_then(|v| v.as_str())
                    .or_else(|| launch_result.get(game_key).and_then(|v| v.as_str()))?;
                Some((daemon_key.as_str(), value))
            })
            .collect(),
    };
    let mut creds_changed = false;
    for (key, value) in given {
        let slot = match key {
            "rest_username" => &mut instance.rest_username,
            "rest_password" => &mut instance.rest_password,
            _ => {
                tracing::debug!("credential_map: unknown daemon key '{}', skipping", key);
                continue;
            }
        };
        if slot.as_deref() != Some(value) {
            *slot = Some(value.to_string());
            creds_changed = true;
            tracing::info!("Synced {} from module for instance '{}'", key, instance.id);
        }
    }
    creds_changed
}

/// `get_launch_command`가 돌려준 실행 명령 (실행 검사를 거친 프로그램)
struct LaunchSpec {
    program: String,
    args: Vec<String>,
    working_dir: String,
    env_vars: Vec<(String, String)>,
}

/// 모듈·익스텐션이 준 `env_vars` 객체를 (이름, 값) 쌍으로. 문자열이 아닌 값은 버린다
fn env_pairs(source: &Value) -> Vec<(String, String)> {
    source.get("env_vars")
        .and_then(|e| e.as_object())
        .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

/// `get_launch_command` 결과의 실행 인자. 모듈이 추가 인자를 넣지 않았으면 끝에 붙인다
pub(crate) fn launch_args(launch_result: &Value, instance: &crate::instance::ServerInstance) -> Vec<String> {
    let mut args: Vec<String> = launch_result
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if launch_result.get("extra_args_placed").and_then(Value::as_bool) != Some(true) {
        args.extend(instance.extra_args.iter().cloned());
    }
    args
}

impl Supervisor {
    /// 0.1.6까지 만든 인스턴스는 모듈 기본 REST 포트(팰월드 8212)를 그대로 저장해 두었다. 그 포트가 Windows 예약
    /// 범위에 들면 서버가 REST를 열지 못하므로, 시작 전에 새 인스턴스처럼 예약 범위와 다른 인스턴스의 포트를 피해
    /// 옮겨 저장한다(모듈이 시작 때 게임 설정 파일을 이 값으로 맞춘다). 기본값이 아닌 값은 사용자가 정한 것이므로
    /// 옮기지 않는다(시작 전 검사가 port_conflict로 알린다)
    /// 옮겼으면 `(옮기기 전, 옮긴 뒤)`
    fn move_reserved_default_rest_port(&mut self, id: &str) -> Option<(u16, u16)> {
        // 이미 떠 있는 서버는 지금 포트로 붙어 있다
        if self.tracker.get_pid(id).is_ok_and(process::is_process_alive) {
            return None;
        }
        let before = self.instance_store.get(id)?.rest_port?;
        self.move_default_rest_port_if(id, crate::validator::tcp_port_reserved_by_windows, crate::validator::port_reserved_by_windows)
            .map(|after| (before, after))
    }

    fn move_default_rest_port_if(&mut self, id: &str, reserved: impl Fn(u16) -> bool, avoid: impl Fn(u16) -> bool) -> Option<u16> {
        let instance = self.instance_store.get(id)?.clone();
        let rest = instance.rest_port?;
        let module = self.module_loader.get_module(&instance.module_name).ok()?;
        let supports_rest = module.metadata.protocols_supported.as_deref().unwrap_or(&[]).iter().any(|p| p == "rest");
        if !supports_rest || rest != module.metadata.default_rest_port() || !reserved(rest) {
            return None;
        }
        let port_fields_of = |name: &str| -> Vec<String> {
            self.module_loader.get_module(name).ok()
                .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
                .unwrap_or_default()
        };
        let mut taken = std::collections::HashSet::new();
        for other in self.instance_store.list().iter().filter(|o| o.id != id) {
            let fields = port_fields_of(&other.module_name);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            taken.extend(crate::validator::instance_ports(other, &fields).into_iter().map(|(_, p)| p));
        }
        let free = crate::validator::next_free_port(rest, |p| taken.contains(&p) || avoid(p))?;
        let mut moved = instance;
        moved.rest_port = Some(free);
        if moved.module_settings.contains_key("rest_port") {
            moved.module_settings.insert("rest_port".to_string(), json!(free));
        }
        let name = moved.name.clone();
        if let Err(e) = self.instance_store.update(id, moved) {
            tracing::warn!("Instance '{}': could not save the moved REST port: {}", name, e);
            return None;
        }
        tracing::info!("Instance '{}': REST port {} is reserved by Windows, using {}", name, rest, free);
        // 자동·예약 재시작처럼 시작 응답을 아무도 보지 않는 경로에서도 바뀐 포트를 알 수 있게
        crate::events::emit("instance.port_moved", json!({ "instanceId": id, "instance": name, "field": "rest_port", "from": rest, "to": free }));
        Some(free)
    }

    /// 시작 전 포트 검사. 실행 중인 다른 인스턴스와 겹치거나 다른 프로그램이 잡고 있는 포트가 있으면
    /// `port_conflict` 응답을 돌려준다. 끄는 방법은 전역 설정(gui.portConflictCheck) 하나뿐이다
    fn port_conflict_response(&self, instance: &crate::instance::ServerInstance) -> Option<Value> {
        if self.skip_port_check.load(Ordering::Relaxed) {
            return None;
        }
        let all_instances = self.instance_store.list();
        let running_ids: std::collections::HashSet<String> = all_instances
            .iter()
            .filter(|i| self.tracker.get_pid(&i.id).is_ok())
            .map(|i| i.id.clone())
            .collect();
        let module_protocols = self.build_module_protocols_map();
        let mut details: Vec<String> = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols))
            .iter()
            .map(|c| c.to_string())
            .collect();
        // 인스턴스끼리 겹치지 않을 때만 다른 프로그램이 잡은 포트를 직접 묶어 본다.
        // 이 인스턴스가 이미 실행 중이면 그 포트는 자기 것이므로 건너뛴다
        if details.is_empty() && !running_ids.contains(&instance.id) {
            let module = self.module_loader.get_module(&instance.module_name).ok();
            let port_fields = module.as_ref().and_then(|m| m.metadata.settings.as_ref()).map(|s| s.port_fields()).unwrap_or_default();
            let ports = crate::validator::start_check_ports(instance, Some(&module_protocols), &port_fields);
            details = crate::validator::ports_held_elsewhere(&ports)
                .into_iter()
                .map(|(name, port)| format!("Port {} ({}) is in use by another program", port, name))
                .collect();
            // 예약 범위의 포트는 아무도 쓰지 않아도 서버가 열지 못한다 (사용 중 목록에 없어 따로 본다)
            let reserved: Vec<String> = crate::validator::ports_reserved_by_windows(&ports)
                .into_iter()
                .filter(|(_, port)| !details.iter().any(|d| d.starts_with(&format!("Port {} ", port))))
                .map(|(name, port)| format!("Port {} ({}) is reserved by Windows (excluded port range); choose another port", port, name))
                .collect();
            details.extend(reserved);
        }
        if details.is_empty() {
            return None;
        }
        tracing::warn!("Port conflict detected for '{}': {:?}", instance.name, details);
        Some(json!({
            "success": false,
            "error": "port_conflict",
            "error_code": "port_conflict",
            "message": format!("Cannot start '{}': port conflict detected", instance.name),
            "conflicts": details,
        }))
    }

    /// 실행 인자 미리보기: 저장된 설정 위에 초안(`draft`)을, 그 위에 추가 인자(`extra`, 없으면 저장된 것)에서 읽은 필드를 얹어
    /// `get_launch_command`를 `preview: true`로 부른다. 익스텐션 hook·서버 시작·파일 쓰기는 없다
    /// (모듈은 `preview`이면 설정 파일을 고치지 않는다). 응답 모양은 `docs/architecture/01-daemon-core.md`
    pub async fn launch_preview(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        draft: &serde_json::Map<String, Value>,
        extra: Option<&[String]>,
    ) -> Result<Value> {
        let (call, fields, settings, preview_instance) = {
            let s = sup.read().await;
            let instance = s
                .instance_store
                .get(instance_id)
                .cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let module = s.module_loader.get_module(&instance.module_name)?;
            let fields = module.metadata.settings.as_ref().map(|m| m.fields.clone()).unwrap_or_default();

            let draft_errors = crate::launch_args::draft_errors(&fields, draft);
            // 추가 인자를 보내지 않았으면 저장된 것으로
            let bound = crate::launch_args::bind(&fields, extra.unwrap_or(&instance.extra_args));
            let mut settings: serde_json::Map<String, Value> = instance.module_settings.clone().into_iter().collect();
            for (k, v) in draft {
                if !draft_errors.iter().any(|e| &e.field == k) {
                    settings.insert(k.clone(), v.clone());
                }
            }
            settings.extend(bound.settings.clone());

            let mut preview_instance = instance.clone();
            preview_instance.module_settings = settings.clone().into_iter().collect();
            preview_instance.extra_args = bound.extra.clone();
            let mut cfg = serde_json::Map::new();
            s.insert_resolved_exe(&mut cfg, &preview_instance, &instance.module_name);
            if let Some(port) = settings.get("port").cloned().or_else(|| instance.port.map(|p| json!(p))) {
                cfg.insert("port".to_string(), port);
            }
            Self::insert_instance_fields(&mut cfg, &preview_instance, &s.instance_store.instance_dir(&instance.id));
            cfg.insert("managed".to_string(), json!(s.uses_managed_start(&preview_instance)));
            cfg.insert("preview".to_string(), json!(true));
            let call = crate::plugin::PluginCall::new(module.entry_path(), "get_launch_command", Value::Object(cfg)).in_instance(s.host_context(&instance.id));

            let mut errors = draft_errors;
            errors.extend(bound.errors.clone());
            let base = json!({
                "has_launch_args": crate::launch_args::has_launch_args(&fields),
                "settings_from_args": bound.settings,
                "extra_args": bound.extra,
                "extra_args_text": crate::launch_args::join_args(&bound.extra),
                "errors": errors,
            });
            ((call, base), fields, settings, preview_instance)
        };
        let (call, mut out) = call;
        // 실행 인자로 들어가는 설정이 없는 모듈은 부르지 않는다 (preview를 모르는 모듈이 파일을 고칠 수 있다)
        if !crate::launch_args::has_launch_args(&fields) {
            out.as_object_mut().expect("object").insert("success".into(), json!(true));
            return Ok(out);
        }
        let launch_result = call.run().await?;
        let obj = out.as_object_mut().expect("object");
        if launch_result.get("success").and_then(Value::as_bool) != Some(true) {
            obj.extend(launch_result.as_object().cloned().unwrap_or_default());
            obj.insert("success".into(), json!(false));
            if let Some(action) = launch_result.get("action_required") {
                obj.insert("action_required".into(), action.clone());
            } else if launch_result.get("success").and_then(Value::as_bool) != Some(false) {
                obj.insert("error".into(), json!("launch_command_failed"));
            }
            let message = launch_result.get("message").and_then(Value::as_str).map(str::to_string);
            let message = message.or_else(|| {
                launch_result.get("action_required").is_none().then(|| module_failure(&launch_result, &preview_instance.module_name, "get_launch_command").to_string())
            });
            if let Some(m) = message {
                obj.insert("message".into(), json!(m));
            }
            return Ok(out);
        }
        let program = launch_result.get("program").and_then(Value::as_str).unwrap_or("").to_string();
        let mut args = launch_args(&launch_result, &preview_instance);
        let rest_password = preview_instance.rest_password.as_deref().unwrap_or("");
        let rcon = crate::protocol::rcon::load(&sup.read().await.instance_store.instance_dir(instance_id));
        let rcon_password = rcon.as_ref().map(|a| a.password.as_str()).unwrap_or("");
        crate::launch_args::mask_secrets(&fields, &settings, &[rest_password, rcon_password], &mut args);
        obj.insert("success".into(), json!(true));
        obj.insert("command_line".into(), json!(crate::launch_args::command_line(&program, &args)));
        obj.insert("program".into(), json!(program));
        obj.insert("args".into(), json!(args));
        Ok(out)
    }

    /// 확인되지 않은 모듈의 실행 대상 검사 (Windows). 띄울 프로그램 경로를 돌려주고, 막으면 `action_required` 응답
    async fn guard_launch(
        sup: &tokio::sync::RwLock<Self>,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        program: &str,
        args: &[String],
        working_dir: &str,
        env_vars: &mut Vec<(String, String)>,
    ) -> Result<(String, crate::sandbox::LaunchGate), Value> {
        let (entry, roots) = {
            let s = sup.read().await;
            let mut dirs = vec![s.instance_store.instance_dir(&instance.id)];
            // 사용자가 정한 서버 폴더 (모듈이 돌려준 working_dir은 믿지 않는다)
            dirs.extend(instance.working_dir.iter().map(std::path::PathBuf::from));
            let Ok(module) = s.module_loader.get_module(module_name) else {
                // 모듈을 못 찾으면 예전처럼 검사 없이 띄우되 서버 폴더의 표시는 걷는다
                drop(s);
                let roots = crate::sandbox::LaunchRoots { dirs, runtimes: Vec::new() };
                return crate::sandbox::guard_launch_for(|| saba_chan_updater_lib::integrity::Trust::Verified, program, args, std::path::Path::new(working_dir), roots, env_vars)
                    .await
                    .map_err(|blocked| blocked.to_json(crate::sandbox::message(&blocked)));
            };
            // 사용자가 설정한 java 경로 (이름이 java일 때만 런타임으로 인정된다).
            // 모듈 기본값·프리셋처럼 사용자가 정하지 않은 값은 인정하지 않는다
            let mut runtimes: Vec<_> = instance.module_settings.get("java_path").and_then(Value::as_str)
                .filter(|_| instance.user_settings.contains("java_path"))
                .map(std::path::PathBuf::from)
                .filter(|p| crate::sandbox::plain_local_path(p))
                .into_iter().collect();
            // 데몬의 JAVA_HOME·PATH가 가리키는 JDK (격리 실행에 읽기를 준 런타임과 같다)
            runtimes.extend(crate::sandbox::daemon_runtimes());
            (module.entry_path(), crate::sandbox::LaunchRoots { dirs, runtimes })
        };
        let (program, gate) = crate::sandbox::guard_launch(&entry, program, args, std::path::Path::new(working_dir), roots, env_vars).await.map_err(|blocked| {
            tracing::warn!("Launch of '{}' blocked: {:?}", instance.name, blocked);
            blocked.to_json(crate::sandbox::message(&blocked))
        })?;
        tracing::info!("Starting '{}': program '{}' in '{}'", instance.name, program, working_dir);
        Ok((program, gate))
    }

    /// 모듈 `[module.dependencies]`를 못 맞추면 시작을 막는 응답. 다 맞으면 None
    async fn unmet_dependencies_response(&self, module_name: &str, instance_name: &str) -> Option<Value> {
        // 이 OS·CPU를 지원한다고 선언하지 않은 모듈 (D3·M1)
        if let Ok(module) = self.module_loader.get_module(module_name) {
            if let Some(body) = module_loader::host_unsupported_body(module_name, &module.metadata) {
                tracing::warn!("Module '{}' does not run on {}/{}; '{}' is not started", module_name, std::env::consts::OS, std::env::consts::ARCH, instance_name);
                return Some(body);
            }
        }
        // 데몬이 모자라 아예 로드하지 못한 모듈도 같은 응답을 준다
        let failed = self.module_loader.get_module(module_name).err().and_then(|_| self.module_loader.load_failure(module_name));
        // 데몬보다 새 스키마는 그 이유 그대로 준다 (클라이언트가 "업데이트가 필요해요"로 보인다)
        if let Some(f) = failed.as_ref().filter(|f| f.error_code.as_deref() == Some("module_schema_unsupported")) {
            tracing::warn!("Module '{}' needs a newer saba-chan for '{}': {}", module_name, instance_name, f.error);
            return Some(json!({
                "success": false,
                "error": "module_schema_unsupported",
                "error_code": "module_schema_unsupported",
                "message": format!("Cannot start '{}': {}", instance_name, f.error),
                "module": module_name,
                "required": f.required,
            }));
        }
        let unmet = match failed {
            // 의존성 때문에 로드하지 못한 것만. 스키마가 틀린 모듈 등은 빈 의존성 오류로 바꾸지 않는다
            Some(f) if f.error_code.as_deref() == Some("module_dependency_unsatisfied") => f.missing_dependencies,
            _ => self.unmet_module_dependencies(module_name).await?,
        };
        tracing::warn!("Module '{}' dependencies unmet for '{}': {}", module_name, instance_name, unmet.join(", "));
        Some(json!({
            "success": false,
            "error": "module_dependency_unsatisfied",
            "error_code": "module_dependency_unsatisfied",
            "message": format!("Cannot start '{}': module '{}' needs {}", instance_name, module_name, unmet.join(", ")),
            "module": module_name,
            "missing_dependencies": unmet,
        }))
    }

    /// 모듈 `[module.dependencies]` 중 못 맞춘 것 (`"ext-steamcmd >=1.0.0 (installed 0.9.0)"` 꼴). 다 맞으면 None.
    /// 데몬(`saba-core`)과 설치·활성화된 익스텐션(`ext-<id>`)만 본다. 다른 키(gui, cli 등)는 데몬이 알 수 없어 넘긴다
    async fn unmet_module_dependencies(&self, module_name: &str) -> Option<Vec<String>> {
        let module = self.module_loader.get_module(module_name).ok()?;
        if module.metadata.dependencies.is_empty() {
            return None;
        }
        let mut installed = std::collections::HashMap::from([("saba-core".to_string(), env!("CARGO_PKG_VERSION").to_string())]);
        if let Some(ext_mgr) = &self.extension_manager {
            let mgr = ext_mgr.read().await;
            let ready = mgr.installed_and_enabled_set();
            for ext in mgr.list().into_iter().filter(|e| ready.contains(&e.id)) {
                installed.insert(format!("ext-{}", ext.id), ext.version);
            }
        }
        let unmet: Vec<String> = module
            .metadata
            .check_dependencies(&installed)
            .into_iter()
            .filter(|(key, _, _)| key == "saba-core" || key.starts_with("ext-"))
            .map(|(key, req, got)| match got {
                Some(v) => format!("{} {} (installed {})", key, req, v),
                None => format!("{} {} (not installed)", key, req),
            })
            .collect();
        (!unmet.is_empty()).then_some(unmet)
    }

    /// 인스턴스 id로 서버를 시작한다
    /// Called by IPC API: POST /api/instance/:id/start (비관리)
    ///
    /// managed 시작과 같이, 익스텐션 hook과 `get_launch_command`는 데몬 락 밖에서 부른다 (C12).
    pub async fn start_server(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str, config: Value) -> Result<Value> {
        let _swap = ADDON_SWAP_GATE.read().await;
        tracing::info!("Starting server '{}' with module '{}'", instance_id, module_name);

        let moved = {
            let mut s = sup.write().await;
            s.move_reserved_default_rest_port(instance_id)
        };
        let result = Self::start_server_inner(sup, instance_id, module_name, config).await;
        Self::announce_ready(sup, instance_id, &result).await;
        result.map(|result| with_port_moved(result, moved))
    }

    /// 시작 결과로 준비 알림(R39, `ready.rs`)과 접속·퇴장 알림(MK8)·게임 채팅 전달(`player_events.rs`)을 시작한다. 콘솔을 읽는 managed 프로세스면 준비 문구를 기다린다
    async fn announce_ready(sup: &tokio::sync::RwLock<Self>, instance_id: &str, result: &Result<Value>) {
        let (rt, id, name, console, patterns, timeout, join, leave, chat) = {
            let s = sup.read().await;
            let Some(instance) = s.instance_store.get(instance_id) else { return };
            let module = s.module_loader.get_module(&instance.module_name).ok();
            let meta = module.as_ref().map(|m| &m.metadata);
            let patterns = meta.map(|m| m.ready_patterns.clone()).unwrap_or_default();
            let timeout = meta.and_then(|m| m.ready_timeout).unwrap_or(ready::DEFAULT_READY_TIMEOUT_SECS);
            let (join, leave) = meta.map(|m| (m.player_join_patterns.clone(), m.player_leave_patterns.clone())).unwrap_or_default();
            let chat = meta.map(|m| m.chat_patterns.clone()).unwrap_or_default();
            game_chat::remember(&instance.id, &instance.game_chat());
            let console = s.managed_store.get(&instance.id).await.filter(|p| !p.log_follower);
            (s.runtime.clone(), instance.id.clone(), instance.name.clone(), console, patterns, Duration::from_secs(timeout), join, leave, chat)
        };
        // 접속·퇴장 알림(MK8)과 게임 채팅도 같은 콘솔을 본다
        player_events::after_start(&rt, &id, &name, result, console.clone(), &join, &leave, &chat);
        ready::after_start(&rt, &id, &name, result, console, &patterns, timeout);
    }

    /// `server.pre_start` hook을 부른다 (락 밖). 익스텐션이 시작을 맡았으면(handled) 그 결과를 돌려주고, 맡은 곳이 없으면 `None`.
    /// 맡아 띄웠으면 익스텐션이 준 `log_follower`를 붙인다. `ctx`는 부르는 쪽이 만든다 (관리형은 비밀 값을 가린 인스턴스·설정을 싣는다)
    async fn run_pre_start_hook(
        sup: &tokio::sync::RwLock<Self>,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        starting: &StartGuard,
        hook: Option<(std::sync::Arc<tokio::sync::RwLock<crate::extension::ExtensionManager>>, Value)>,
        managed: bool,
    ) -> Option<Value> {
        let (ext_mgr, ctx) = hook?;
        let hook_label = if managed { "server.pre_start' (managed)" } else { "server.pre_start'" };
        let server_name = instance.name.as_str();
        let host_context = sup.read().await.host_context(&instance.id);
        let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.pre_start", ctx)).await;
        for (ext_id, result) in &results {
            if let Ok(val) = result {
                if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                    // Extension handled but failed → ensure error/message fields exist
                    if val.get("success").and_then(|s| s.as_bool()) != Some(true) {
                        let mut fail_val = val.clone();
                        if let Some(obj) = fail_val.as_object_mut() {
                            obj.entry("error".to_string())
                                .or_insert_with(|| json!("Extension failed to start server"));
                            obj.entry("message".to_string())
                                .or_insert_with(|| json!("Extension handled the start request but reported failure"));
                        }
                        let err_detail = fail_val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                        tracing::warn!(
                            ext = ext_id,
                            error = err_detail,
                            "Extension hook '{} handled but failed for '{}'",
                            hook_label,
                            server_name
                        );
                        return Some(fail_val);
                    }
                    // 익스텐션이 띄우는 사이 정지가 들어왔으면 그 정지(server.stop)는 이미 끝났다: 다시 꺼서 되돌린다
                    if starting.stop_requested(&*sup.read().await) {
                        Self::undo_extension_start(sup, &instance.id, module_name).await;
                        return Some(start_cancelled(&instance.name));
                    }
                    // Extension이 시작 성공 → log_follower가 있으면 로그 스트리머 등록
                    if let Some(log_cfg) = val.get("log_follower") {
                        if let Err(blocked) = Self::attach_log_follower(sup, instance, module_name, ext_id, log_cfg).await {
                            return Some(blocked);
                        }
                    }
                    announce_started(&starting.runtime, &instance.id, &instance.name, &crate::audit::current(), false);
                    return Some(val.clone());
                }
            }
        }
        None
    }

    /// 시작을 맡은 익스텐션이 준 `log_follower`(콘솔 다리)를 띄워 managed 저장소에 둔다. 실행 검사에서 막히면 그 응답을 `Err`로
    async fn attach_log_follower(
        sup: &tokio::sync::RwLock<Self>,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        ext_id: &str,
        log_cfg: &Value,
    ) -> std::result::Result<(), Value> {
        let server_name = instance.name.as_str();
        let instance_dir = sup.read().await.instance_store.instance_dir(&instance.id);
        let program = log_cfg.get("program").and_then(|p| p.as_str()).unwrap_or_default();
        let args: Vec<String> = log_cfg.get("args")
            .and_then(|a| a.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let work_dir = log_cfg.get("working_dir")
            .and_then(|w| w.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| instance_dir.clone());
        let desc = log_cfg.get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("Extension log follower");
        let interactive = log_cfg.get("interactive")
            .and_then(|i| i.as_bool())
            .unwrap_or(false);
        let strip_prefix = log_cfg.get("strip_prefix").and_then(|s| s.as_str());
        let log_pattern = sup.read().await.module_loader.get_module(module_name)
            .ok()
            .and_then(|m| m.metadata.log_pattern.clone());

        // interactive=true → spawn() (bidirectional stdin/stdout)
        // interactive=false → spawn_log_follower() (read-only stdout)
        let mut env_vars = env_pairs(log_cfg);
        // 확인되지 않은 익스텐션이 띄우는 프로그램도 모듈과 같은 검사를 받는다
        let roots = crate::sandbox::LaunchRoots {
            dirs: std::iter::once(instance_dir.clone()).chain(instance.working_dir.iter().map(std::path::PathBuf::from)).collect(),
            runtimes: Vec::new(),
        };
        let (program, gate) = match crate::sandbox::guard_launch_for(|| crate::sandbox::extension_trust(ext_id), program, &args, &work_dir, roots, &mut env_vars).await {
            Ok(launch) => launch,
            Err(blocked) => {
                tracing::warn!("Extension '{}' launch blocked: {:?}", ext_id, blocked);
                return Err(blocked.to_json(crate::sandbox::message(&blocked)));
            }
        };
        let program = program.as_str();
        let spawn_result = if interactive {
            tracing::info!("Spawning interactive IO bridge for '{}'", server_name);
            managed_process::ManagedProcess::spawn(
                program,
                &args,
                work_dir.to_str().unwrap_or("."),
                env_vars,
                log_pattern.as_deref(),
                Some(&instance_dir),
            ).await
        } else {
            managed_process::ManagedProcess::spawn_log_follower(
                program,
                &args,
                &work_dir,
                desc,
                log_pattern.as_deref(),
                strip_prefix,
                Some(&instance_dir),
            ).await
        };
        drop(gate);
        match spawn_result {
            Ok(mut follower) => {
                // 익스텐션이 붙인 다리라 서버 종료 판정(A1) 대상이 아니다
                follower.log_follower = true;
                sup.read().await.managed_store.insert(&instance.id, follower).await;
                tracing::info!(
                    "{} registered for '{}'",
                    if interactive { "Interactive IO bridge" } else { "Log follower" },
                    server_name
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to start {} for '{}': {}",
                    if interactive { "IO bridge" } else { "log follower" },
                    server_name, e
                );
            }
        }
        Ok(())
    }

    /// `get_launch_command` 호출을 만든다: 시작 설정에 실행 파일·포트·인스턴스 값을 더하고 실행 방식(`managed`)을 알린다
    fn launch_command_call(
        &self,
        config: &Value,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        managed: bool,
    ) -> Result<(crate::plugin::PluginCall, crate::supervisor::module_loader::LoadedModule)> {
        // 실행 파일은 모듈 정의에서 자동 해석
        let mut cfg = config.as_object().cloned().unwrap_or_default();
        self.insert_resolved_exe(&mut cfg, instance, module_name);
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        self.ensure_rcon_access(instance)?;
        Self::insert_instance_fields(&mut cfg, instance, &self.instance_store.instance_dir(&instance.id));
        // 모듈이 실행 방식을 구분
        cfg.insert("managed".to_string(), json!(managed));
        let final_config = Value::Object(cfg);

        let module = self.module_loader.get_module(module_name)?;
        if managed {
            // 비밀번호류 설정값과 REST 비밀번호는 가려서 남긴다
            let (_, logged) = crate::audit::redact(Some(&module.metadata), Some(instance), None, Some(&final_config));
            tracing::info!("get_launch_command config: {}", logged.unwrap_or_default());
        }
        let call = crate::plugin::PluginCall::new(module.entry_path(), "get_launch_command", final_config).in_instance(self.host_context(&instance.id));
        Ok((call, module))
    }

    /// `get_launch_command` 결과를 실행 명령으로 풀고 실행 검사(`guard_launch`)를 거친다.
    /// 사용자 조치가 필요하거나(`action_required`) 실행 검사에서 막히면 돌려줄 응답을 `Ok(Err(..))`로
    async fn prepare_launch(
        sup: &tokio::sync::RwLock<Self>,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        launch_result: &Value,
    ) -> Result<std::result::Result<(LaunchSpec, crate::sandbox::LaunchGate), Value>> {
        // action_required (e.g. server jar not found) → pass through to caller
        if launch_result.get("action_required").is_some() {
            tracing::warn!("Module requires user action: {:?}", launch_result.get("action_required"));
            return Ok(Err(launch_result.clone()));
        }

        if launch_result.get("success").and_then(|s| s.as_bool()) != Some(true) {
            return Err(module_failure(launch_result, module_name, "get_launch_command"));
        }

        let program = launch_result.get("program")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("Module did not return program"))?;

        let args = launch_args(launch_result, instance);

        let working_dir = launch_result.get("working_dir")
            .and_then(|w| w.as_str())
            .unwrap_or(".");

        let mut env_vars = env_pairs(launch_result);

        Ok(match Self::guard_launch(sup, instance, module_name, program, &args, working_dir, &mut env_vars).await {
            Ok((program, gate)) => Ok((LaunchSpec { program, args, working_dir: working_dir.to_string(), env_vars }, gate)),
            Err(blocked) => Err(blocked),
        })
    }

    async fn start_server_inner(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str, config: Value) -> Result<Value> {

        // ── 검사 (읽기 락) ──
        let (instance, starting, hook) = {
            let s = sup.read().await;
            // Find instance to get executable_path and working_dir
            let instance = s.instance_store.get(instance_id)
                .cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;

            // ── 중복 시작 방어: 추적 중인 PID가 살아 있으면 새로 띄우지 않는다 (추적을 덮어쓰지 않게) ──
            if let Ok(existing_pid) = s.tracker.get_pid(&instance.id) {
                if process::is_process_alive(existing_pid) {
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing_pid,
                        "already_running": true,
                        "message": format!("Server '{}' is already running with PID {}", instance.name, existing_pid)
                    }));
                }
            }

            // ── 포트 충돌 검사 (모듈별 프로토콜 인지) ──
            if let Some(blocked) = s.port_conflict_response(&instance) {
                return Ok(blocked);
            }

            // ── 모듈 의존성 버전 검사 ([module.dependencies]의 saba-core, ext-<id>) ──
            if let Some(blocked) = s.unmet_dependencies_response(module_name, &instance.name).await {
                return Ok(blocked);
            }

            // ── 인스턴스 필수 익스텐션 검증 ──
            // 기존 인스턴스의 required_extensions가 비어있으면 모듈의 install.requires_extensions에서 보충
            let effective_required = s.effective_required_extensions(&instance);

            if !effective_required.is_empty() {
                if let Some(ref ext_mgr) = s.extension_manager {
                    let crate::extension::MissingExtensions { missing, not_installed } = ext_mgr.read().await.missing_required(&effective_required);
                    if !missing.is_empty() {
                        // 활성화되었지만 설치되지 않은 항목 구분
                        if !not_installed.is_empty() {
                            tracing::error!(
                                "Instance '{}' requires extensions {:?} which are enabled but not installed on disk",
                                instance_id, not_installed
                            );
                        } else {
                            tracing::error!(
                                "Instance '{}' requires extensions {:?} but they are not enabled",
                                instance_id, missing
                            );
                        }
                        return Ok(json!({
                            "success": false,
                            "error": "extension_required",
                            "error_code": "extension_required",
                            "instance_name": instance_id,
                            "missing_extensions": missing,
                            "message": format!(
                                "Instance '{}' requires extension(s) {} to start. Please enable them first.",
                                instance_id,
                                missing.join(", ")
                            )
                        }));
                    }
                }
            }


            let Some(starting) = StartGuard::claim(&s.runtime, &instance.id, start_baseline(&s, &instance.id)) else {
                return Ok(json!({
                    "success": false,
                    "error": "start_in_progress",
                    "error_code": "start_in_progress",
                    "message": format!("Server '{}' is already starting", instance_id),
                }));
            };

            // ── Extension hook: server.pre_start ──
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                let ctx = json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                });
                (ext_mgr, ctx)
            });
            (instance, starting, hook)
        };
        // 받은 키가 id여도 메시지·응답에는 이름을 쓴다
        let server_name = instance.name.as_str();

        if let Some(handled) = Self::run_pre_start_hook(sup, &instance, module_name, &starting, hook, false).await {
            return Ok(handled);
        }

        let call = {
            let s = sup.read().await;
            // ── Non-managed 모드: get_launch_command로 명령어를 받아 데몬이 직접 spawn ──

            tracing::info!("Starting non-managed server for instance '{}' (module: {})", server_name, module_name);

            s.launch_command_call(&config, &instance, module_name, false)?.0
        };
        let launch_result = call.run().await?;
        let (LaunchSpec { program, args, working_dir, env_vars }, gate) = match Self::prepare_launch(sup, &instance, module_name, &launch_result).await? {
            Ok(launch) => launch,
            Err(response) => return Ok(response),
        };
        let (program, working_dir) = (program.as_str(), working_dir.as_str());
        if starting.stop_requested(&*sup.read().await) {
            return Ok(start_cancelled(server_name));
        }

        // 데몬이 입출력을 잡지 않는다. Windows는 서버를 새 콘솔 창에 띄워 사용자가 그 창에서 보고 입력한다
        // (`.exe`가 아니면 예전처럼 창 없이 띄운다, `utils::new_console_program`)
        #[cfg(windows)]
        let console = crate::utils::new_console_program(program, std::path::Path::new(working_dir), &env_vars);
        #[cfg(not(windows))]
        let console: Option<std::path::PathBuf> = None;
        // 새 콘솔로 띄운 서버는 tokio Child가 없다. 정지 대비로 프로세스 핸들을 들고 있는다
        #[cfg(windows)]
        let mut console_handle: Option<std::os::windows::io::OwnedHandle> = None;
        let (pid, mut child) = match console {
            #[cfg(windows)]
            Some(exe) => {
                let spawned = crate::utils::spawn_in_new_console(&exe, &args, Some(std::path::Path::new(working_dir)), &env_vars);
                drop(gate);
                let (pid, handle) = spawned.map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;
                console_handle = Some(handle);
                (pid, None)
            }
            _ => {
                let mut cmd = TokioCommand::new(program);
                cmd.args(&args)
                    .current_dir(working_dir)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());

                for (key, value) in &env_vars {
                    cmd.env(key, value);
                }

                crate::utils::apply_creation_flags(&mut cmd);

                let child = cmd.spawn();
                drop(gate);
                let child = child.map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;
                let pid = child.id().ok_or_else(|| anyhow::anyhow!("Failed to get PID of spawned process"))?;
                (pid, Some(child))
            }
        };

        // 다른 경로(포트 충돌 검사·상태·정지)와 같은 키(인스턴스 id)로 추적한다.
        // managed 시작과 같이 정지 표시를 지우고 세대를 올린다 (기다리던 재시작이 이 시작 뒤에 또 켜지 않게)
        {
            let s = sup.write().await;
            // 띄우는 사이 정지가 들어왔으면 추적하지 않고 끈다: 정지는 이미 "꺼져 있음"으로 끝났다
            if starting.stop_requested(&s) {
                drop(s);
                // 자식까지 끝낸다: 런처형 서버는 런처만 끝내면 실제 서버가 남는다
                let killed = tokio::task::spawn_blocking(move || process::force_kill_tree(pid)).await;
                if let Err(e) = killed.map_err(anyhow::Error::from).and_then(|r| r) {
                    tracing::warn!("Could not stop '{}' after a stop request during start: {}", server_name, e);
                    if let Some(child) = child.as_mut() {
                        let _ = child.kill().await;
                    }
                    #[cfg(windows)]
                    if let Some(handle) = console_handle.as_ref() {
                        use std::os::windows::io::AsRawHandle;
                        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(handle.as_raw_handle() as _, 1) };
                    }
                }
                if let Some(child) = child.as_mut() {
                    let _ = child.wait().await;
                }
                return Ok(start_cancelled(server_name));
            }
            s.runtime.with(&instance.id, |r| {
                r.stop_requested = None;
                r.run_generation += 1;
            });
            s.tracker.track(&instance.id, pid)?;
        }
        tracing::info!("Non-managed server '{}' started with PID {}", server_name, pid);
        announce_started(&starting.runtime, &instance.id, &instance.name, &crate::audit::current(), false);

        Ok(json!({
            "success": true,
            "server": server_name,
            "pid": pid,
            "message": format!("Server '{}' started with PID {} (non-managed mode)", server_name, pid)
        }))
    }

    /// 익스텐션이 맡은 시작이 끝났을 때 이미 정지가 들어와 있었다: 그 정지의 `server.stop`은 서버가 뜨기 전에 돌았으므로
    /// 정지를 한 번 더 불러 익스텐션이 띄운 서버를 끈다
    async fn undo_extension_start(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str) {
        match Self::stop_server(sup, instance_id, module_name, false).await {
            Ok(v) if v.get("success").and_then(Value::as_bool) != Some(false) => {}
            other => tracing::warn!("Could not stop '{}' after a stop request during an extension start: {:?}", instance_id, other),
        }
    }

    /// Start a server as a managed process with full stdio capture.
    /// Uses the module's `get_launch_command` to build the command, then spawns it natively.
    /// Extension 관리 모드에서는 extension hook을 통해 위임합니다.
    ///
    /// 데몬 락은 검사·준비·등록 때만 짧게 잡는다. 익스텐션 hook, `get_launch_command`,
    /// 프로세스 띄우기는 락 밖에서 한다 (C12). 같은 인스턴스를 동시에 두 번 시작하지 않도록
    /// 시작하는 동안 `starting` 표시를 쥔다.
    pub async fn start_managed_server(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {
        let _swap = ADDON_SWAP_GATE.read().await;
        let moved = sup.write().await.move_reserved_default_rest_port(instance_id);
        let result = Self::start_managed_server_inner(sup, instance_id, module_name, config).await;
        Self::announce_ready(sup, instance_id, &result).await;
        result.map(|result| with_port_moved(result, moved))
    }

    async fn start_managed_server_inner(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {

        // ── 1) 검사 (읽기 락) ──
        let (mut instance, starting, hook) = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id).cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;

            // ── 중복 프로세스 방어: 이미 실행 중인 managed 프로세스가 있으면 새로 생성하지 않음 ──
            if let Some(existing) = s.managed_store.get(instance_id).await {
                if existing.is_running() {
                    tracing::info!(
                        "Managed process for '{}' already running (PID {}), skipping duplicate start",
                        instance.name, existing.pid
                    );
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing.pid,
                        "managed": true,
                        "already_running": true,
                        "message": format!("Server '{}' is already running with PID {} (managed mode)", instance.name, existing.pid)
                    }));
                }
            }
            // managed_store에 없더라도 tracker에 PID가 있고 프로세스가 살아있으면 중복 방지
            if let Ok(existing_pid) = s.tracker.get_pid(instance_id) {
                if crate::supervisor::process::is_process_alive(existing_pid) {
                    tracing::warn!(
                        "Tracked process for '{}' still alive (PID {}) but not in managed store — preventing duplicate start",
                        instance.name, existing_pid
                    );
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing_pid,
                        "managed": true,
                        "already_running": true,
                        "message": format!("Server '{}' has a tracked process (PID {}) still alive", instance.name, existing_pid)
                    }));
                } else {
                    // 프로세스가 죽어있으면 tracker 정리
                    tracing::info!("Cleaning stale tracker entry for '{}' (PID {} no longer alive)", instance.name, existing_pid);
                    let _ = s.tracker.untrack(instance_id);
                }
            }

            if let Some(blocked) = s.unmet_dependencies_response(module_name, &instance.name).await {
                return Ok(blocked);
            }

            // ── 포트 충돌 검사 (모듈별 프로토콜 인지) ──
            if let Some(blocked) = s.port_conflict_response(&instance) {
                return Ok(blocked);
            }


            let Some(starting) = StartGuard::claim(&s.runtime, instance_id, start_baseline(&s, instance_id)) else {
                return Ok(json!({
                    "success": false,
                    "error": "start_in_progress",
                    "error_code": "start_in_progress",
                    "message": format!("Server '{}' is already starting", instance.name),
                }));
            };

            // ── Extension hook: server.pre_start (managed) ──
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                // 인스턴스와 시작 설정의 비밀 값은 post_create·settings_changed와 같은 기준으로 가린다 (D-E2)
                let module = s.module_loader.get_module(module_name).ok();
                let hidden = |v: &Value| crate::audit::redact(module.as_ref().map(|m| &m.metadata), None, None, Some(v)).1.unwrap_or_default();
                let ctx = json!({
                    "instance_id": instance_id,
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "instance": hidden(&serde_json::to_value(&instance).unwrap_or_default()),
                    "config": hidden(&config),
                    "managed": true,
                    "extension_data": &instance.extension_data,
                });
                (ext_mgr, ctx)
            });
            (instance, starting, hook)
        };

        // ── 2) 익스텐션 hook (락 밖) ──
        if let Some(handled) = Self::run_pre_start_hook(sup, &instance, module_name, &starting, hook, true).await {
            return Ok(handled);
        }

        // ── 3) 실행 명령 받기: 준비는 읽기 락, 모듈 호출은 락 밖 ──
        let (call, module) = {
            let s = sup.read().await;
            tracing::info!("Starting managed server for instance '{}' (module: {})", instance.name, module_name);

            s.launch_command_call(&config, &instance, module_name, true)?
        };
        let launch_result = call.run().await?;
        let (LaunchSpec { program, args, working_dir, env_vars }, gate) = match Self::prepare_launch(sup, &instance, module_name, &launch_result).await? {
            Ok(launch) => launch,
            Err(response) => return Ok(response),
        };
        let (program, working_dir) = (program.as_str(), working_dir.as_str());

        // Sync REST credentials from module via credential_map or rest_credentials.
        // 시작하는 동안 다른 설정이 바뀌었을 수 있으니 지금 값에 자격 증명만 덮는다
        if sync_rest_credentials(&mut instance, &launch_result, &module.metadata.credential_map) {
            let mut s = sup.write().await;
            if let Some(mut current) = s.instance_store.get(&instance.id).cloned() {
                current.rest_username = instance.rest_username.clone();
                current.rest_password = instance.rest_password.clone();
                if let Err(e) = s.instance_store.update(&instance.id, current) {
                    tracing::warn!("Failed to persist synced credentials: {}", e);
                }
            }
        }

        if starting.stop_requested(&*sup.read().await) {
            return Ok(start_cancelled(&instance.name));
        }

        // Spawn managed process
        let log_pattern = module.metadata.log_pattern.as_deref();
        let (inst_dir, state_dir, pid_file) = {
            let s = sup.read().await;
            (s.instance_store.instance_dir(&instance.id), s.instance_store.state_dir(&instance.id), s.instance_store.managed_pid_file(&instance.id))
        };
        // 새로 띄우는 서버의 기록은 데몬 전용 폴더에만 둔다
        if let Err(e) = forget_legacy_run(&sup.read().await.instance_store, &instance.id) {
            tracing::warn!("Could not stop following the old runner record of '{}': {}", instance.name, e);
        }
        // saba-runner가 서버를 쥐면 데몬이 재시작해도 콘솔·stdin·종료 코드가 이어진다 (C8b)
        let managed = match managed_process::runner_binary() {
            Some(runner) => {
                let keep = instance.module_settings.get("log_sessions_keep")
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .map(|n| n.max(1) as usize)
                    .unwrap_or(managed_process::DEFAULT_LOG_SESSIONS);
                let spawned = ManagedProcess::spawn_with_runner(
                    &runner, program, &args, working_dir, env_vars, log_pattern, &inst_dir, &state_dir, keep,
                    managed_process::RunnerLaunch {
                        log_files: &module.metadata.log_files,
                        console: module.metadata.console,
                        memory_limit_mb: instance.memory_limit_mb,
                        line_start: module.metadata.line_start.as_deref(),
                        progress_patterns: &module.metadata.progress_patterns,
                    },
                ).await;
                // 시작 중 상한에 걸린 것도 종료 이력에 시작 실패로 남긴다
                if let Some(hit) = spawned.as_ref().err().and_then(|e| e.downcast_ref::<managed_process::MemoryLimitAtStart>()) {
                    let verdict = crash::ExitVerdict {
                        kind: crash::ExitKind::StartFailed,
                        exit_code: hit.exit_code,
                        signal: None,
                        evidence: format!("memory limit {} MiB reached while starting", hit.limit_mb),
                    };
                    let event = crash::ExitEvent { at: saba_runner::now(), verdict, restart_in_secs: None, note: None, note_code: None, note_args: None };
                    sup.write().await.record_exit(&instance.id, &instance.name, event, None, Some(crate::audit::current()));
                }
                spawned?
            }
            // 단위 테스트는 러너를 빌드하지 않고 돈다: 그때만 데몬이 직접 붙는다
            None if cfg!(test) => ManagedProcess::spawn(program, &args, working_dir, env_vars, log_pattern, Some(&inst_dir)).await?,
            // 관리형 시작은 러너가 콘솔을 쥐는 시작이다. 러너 없이 데몬이 대신 띄우지 않는다 (사용자 정의 2026-11-08)
            None => {
                tracing::warn!("saba-runner not found next to the daemon; refusing managed start of '{}'", instance.name);
                return Err(managed_process::RunnerMissing.into());
            }
        };
        drop(gate);
        let pid = managed.pid;

        // Track the process
        {
            let s = sup.write().await;
            // 띄우는 사이 정지가 들어왔으면 추적하지 않고 끈다: 정지는 이미 "꺼져 있음"으로 끝났다
            if starting.stop_requested(&s) {
                drop(s);
                if let Err(e) = managed.force_kill().await {
                    tracing::warn!("Could not stop '{}' after a stop request during start: {}", instance.name, e);
                }
                return Ok(start_cancelled(&instance.name));
            }
            s.runtime.with(&instance.id, |r| {
                r.stop_requested = None;
                r.run_generation += 1;
            });
            s.tracker.track(&instance.id, pid)?;
            s.managed_store.insert(&instance.id, managed).await;
        }

        // Persist PID to disk for orphan reattachment on daemon restart
        if let Err(e) = saba_atomic_file::write(&pid_file, pid.to_string().as_bytes(), saba_atomic_file::Mode::Plain) {
            tracing::warn!("Failed to write managed.pid for '{}': {}", instance.name, e);
        }

        tracing::info!("Managed server '{}' started with PID {}", instance.name, pid);
        announce_started(&starting.runtime, &instance.id, &instance.name, &crate::audit::current(), false);
        Ok(json!({
            "success": true,
            "server": instance.name,
            "pid": pid,
            "managed": true,
            "message": format!("Server '{}' started with PID {} (managed mode)", instance.name, pid)
        }))
    }
}

/// `instance.started`: 서버가 켜졌다. `unexpected`면 사바쨩이 띄우지 않은 프로세스다.
/// 켠 쪽을 기억해 켜진 뒤 곧 끝난 시작 실패(`instance.exit` `start_failed`)에 싣는다
pub(super) fn announce_started(runtime: &runtime::Runtimes, instance_id: &str, name: &str, origin: &crate::audit::Origin, unexpected: bool) {
    runtime.with(instance_id, |r| r.start_origin = Some(origin.clone()));
    idle::clear_stopped(instance_id);
    let mut data = origin_fields(origin);
    data.insert("instanceId".into(), json!(instance_id));
    data.insert("instance".into(), json!(name));
    data.insert("unexpected".into(), json!(unexpected));
    crate::events::emit("instance.started", Value::Object(data));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 모듈이 준 REST 자격 증명: `rest_credentials`가 있으면 그것만, 없으면 `[credential_map]`으로 `module_settings`·최상위에서 읽는다
    #[test]
    fn rest_credentials_come_from_the_module_reply_or_its_credential_map() {
        let map: std::collections::HashMap<String, String> = [
            ("rest_password".to_string(), "AdminPassword".to_string()),
            ("rest_username".to_string(), "AdminName".to_string()),
            ("rcon_password".to_string(), "RconPassword".to_string()),
        ]
        .into();
        let fresh = || crate::instance::ServerInstance::new("s", "m");

        // rest_credentials가 있으면 credential_map은 보지 않는다
        let mut inst = fresh();
        let reply = json!({ "rest_credentials": { "rest_username": "admin", "rest_password": "pw" }, "AdminName": "other" });
        assert!(sync_rest_credentials(&mut inst, &reply, &map));
        assert_eq!((inst.rest_username.as_deref(), inst.rest_password.as_deref()), (Some("admin"), Some("pw")));
        assert!(!sync_rest_credentials(&mut inst, &reply, &map), "same values change nothing");

        let mut inst = fresh();
        inst.rest_password = Some("kept".into());
        assert!(sync_rest_credentials(&mut inst, &json!({ "rest_credentials": { "rest_username": "admin" }, "AdminPassword": "x" }), &map));
        assert_eq!(inst.rest_password.as_deref(), Some("kept"));

        // credential_map: module_settings가 최상위보다 먼저, 문자열만, 모르는 데몬 키는 건너뛴다
        let mut inst = fresh();
        let reply = json!({ "module_settings": { "AdminPassword": "inner", "AdminName": 5 }, "AdminPassword": "outer", "AdminName": "top", "RconPassword": "r" });
        assert!(sync_rest_credentials(&mut inst, &reply, &map));
        assert_eq!((inst.rest_username.as_deref(), inst.rest_password.as_deref()), (Some("top"), Some("inner")));

        // 둘 다 없으면 그대로
        let mut inst = fresh();
        assert!(!sync_rest_credentials(&mut inst, &json!({ "AdminPassword": "x" }), &Default::default()));
        assert!(!sync_rest_credentials(&mut inst, &json!({}), &map));
        assert_eq!((inst.rest_username, inst.rest_password), (None, None));
    }

    /// R39 준비 알림을 기다린다 (그 인스턴스의 것만)
    async fn ready_event(sub: &mut crate::events::Subscription, id: &str) -> Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let e = tokio::time::timeout(left, sub.rx.recv()).await.expect("instance.ready arrives").unwrap();
            if e.kind == "instance.ready" && e.data["instanceId"] == id {
                return e.data.clone();
            }
        }
    }

    /// R39: 시작마다 한 번 `instance.ready`. 준비 문구가 콘솔에 찍히면 ok, 준비 전에 끝나면 start_failed,
    /// 사용자가 끄면 stopped_while_starting, 시간이 지나면 ready_timeout. 선언이 없으면 시작이 성공한 때 ok
    #[cfg(unix)]
    #[tokio::test]
    async fn each_start_says_once_whether_the_server_became_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let script = |sh: &str| format!(
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
             else:\n    print(json.dumps({{'success': True}}))\n"
        );
        let games = [
            ("ready", "ready_patterns = [\"^Done \\\\(\"]\n", "echo booting; sleep 0.5; echo 'Done (1.2s)'; sleep 30"),
            ("dies", "ready_patterns = [\"^Done\"]\n", "echo booting; sleep 0.5; exit 3"),
            ("slow", "ready_patterns = [\"^Done\"]\nready_timeout = 1\n", "echo booting; sleep 30"),
            ("quiet", "ready_patterns = [\"^Done\"]\n", "echo booting; sleep 30"),
            ("plain", "", "sleep 30"),
        ];
        for (name, detection, sh) in games {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\n{detection}")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), script(sh)).unwrap();
            crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        }
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for (id, module) in [("r-ready", "ready"), ("r-dies", "dies"), ("r-slow", "slow"), ("r-plain", "plain"), ("r-stopped", "quiet")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.into();
            sup.instance_store.add(inst).unwrap();
        }
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        let start = |id: &'static str, module: &'static str| {
            let sup = sup.clone();
            async move { Supervisor::start_managed_server(&sup, id, module, json!({})).await.unwrap() }
        };

        assert_eq!(start("r-plain", "plain").await["success"], true);
        assert_eq!(ready_event(&mut sub, "r-plain").await["ok"], true);

        assert_eq!(start("r-ready", "ready").await["success"], true);
        let ready = ready_event(&mut sub, "r-ready").await;
        assert_eq!((ready["ok"].clone(), ready["instance"].clone(), ready.get("code").cloned()), (json!(true), json!("r-ready"), None));

        // 종료 판정이 준비 뒤의 종료를 시작 실패로 보지 않게 남긴다 (한 번 읽으면 지운다)
        let rt = sup.read().await.runtime.clone();
        assert!(ready::take_ready(&rt, "r-ready"));
        assert!(!ready::take_ready(&rt, "r-ready"));

        let code = |e: Value| (e["ok"].clone(), e["code"].clone());
        start("r-dies", "dies").await;
        assert_eq!(code(ready_event(&mut sub, "r-dies").await), (json!(false), json!("start_failed")));
        assert!(!ready::take_ready(&rt, "r-dies"));

        start("r-slow", "slow").await;
        assert_eq!(code(ready_event(&mut sub, "r-slow").await), (json!(false), json!("ready_timeout")));

        start("r-stopped", "quiet").await;
        let stopped = Supervisor::stop_server(&sup, "r-stopped", "quiet", true).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        assert_eq!(code(ready_event(&mut sub, "r-stopped").await), (json!(false), json!("stopped_while_starting")));

        for id in ["r-plain", "r-ready", "r-slow"] {
            let _ = Supervisor::stop_server(&sup, id, "plain", true).await;
        }
    }

    /// MK8 §8.3: managed 시작에서 접속·퇴장 문구가 콘솔에 찍히면 `instance.player`. 시스템 줄과 다른 그룹은 싣지 않는다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_managed_server_says_who_joined_and_left() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("chatty");
        std::fs::create_dir_all(&dir).unwrap();
        let head = "^\\[\\d{2}:\\d{2}:\\d{2}(?:\\] \\[[^\\]]*/| )INFO\\]: ";
        let table = format!("player_join_patterns = ['{head}(?P<name>[A-Za-z0-9_]{{1,16}}) joined the game\\s*$']\nplayer_leave_patterns = ['{head}(?P<name>[A-Za-z0-9_]{{1,16}}) left the game\\s*$']\n");
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"chatty\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\n{table}")).unwrap();
        let sh = "echo '[20:07:05] [Server thread/INFO]: Steve joined the game'; echo '[20:07:06] [Server thread/INFO]: <Alex> ]: Bob joined the game'; sleep 0.6; echo '[20:07:09] [Server thread/INFO]: Steve left the game'; sleep 30";
        std::fs::write(dir.join("lifecycle.py"), format!(
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
             else:\n    print(json.dumps({{'success': True}}))\n"
        )).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("p-chatty", "chatty");
        inst.id = "p-chatty".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        assert_eq!(Supervisor::start_managed_server(&sup, "p-chatty", "chatty", json!({})).await.unwrap()["success"], true);
        if let Some(proc) = sup.read().await.managed_store.get("p-chatty").await {
            proc.push_system_line("[20:07:07] [Server thread/INFO]: Mallory joined the game".into(), crate::supervisor::managed_process::LogLevel::Info).await;
        }

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.len() < 2 {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let e = tokio::time::timeout(left, sub.rx.recv()).await.expect("instance.player arrives").unwrap();
            if e.kind == "instance.player" && e.data["instanceId"] == "p-chatty" {
                assert_eq!(e.data["instance"], "p-chatty");
                assert!(e.data["at"].is_u64());
                seen.push((e.data["player"].clone(), e.data["change"].clone()));
            }
        }
        assert_eq!(seen, vec![(json!("Steve"), json!("join")), (json!("Steve"), json!("leave"))], "a chat line and a system line are no player");
        let _ = Supervisor::stop_server(&sup, "p-chatty", "chatty", true).await;
    }

    /// 게임 채팅 전달: 켠 서버의 콘솔 채팅이 설정(감지 단어)에 맞으면 데몬이 다 만든 문장으로 `instance.chat`을 낸다. 시스템 줄은 보지 않는다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_managed_server_forwards_game_chat_as_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("talky");
        std::fs::create_dir_all(&dir).unwrap();
        let table = "chat_patterns = ['^\\[\\d{2}:\\d{2}:\\d{2}\\] \\[Server thread/INFO\\]: <(?P<name>[A-Za-z0-9_]{1,16})> (?P<message>.*)$']\n";
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"talky\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\n{table}")).unwrap();
        let sh = "echo '[20:07:05] [Server thread/INFO]: <Steve> hello'; echo '[20:07:06] [Server thread/INFO]: <Steve> !d 안녕 *다들*'; sleep 30";
        std::fs::write(dir.join("lifecycle.py"), format!(
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
             else:\n    print(json.dumps({{'success': True}}))\n"
        )).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("survival", "talky");
        inst.id = "p-talky".into();
        inst.game_chat = game_chat::Mode::Keyword;
        inst.game_chat_keyword = "!d".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        assert_eq!(Supervisor::start_managed_server(&sup, "p-talky", "talky", json!({})).await.unwrap()["success"], true);
        if let Some(proc) = sup.read().await.managed_store.get("p-talky").await {
            proc.push_system_line("[20:07:07] [Server thread/INFO]: <Mallory> !d fake".into(), crate::supervisor::managed_process::LogLevel::Info).await;
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        let text = loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let e = tokio::time::timeout(left, sub.rx.recv()).await.expect("instance.chat arrives").unwrap();
            if e.kind == "instance.chat" && e.data["instanceId"] == "p-talky" {
                assert_eq!(e.data["instance"], "survival");
                break e.data["text"].clone();
            }
        };
        assert_eq!(text, "\\[survival\\] **Steve**: 안녕 \\*다들\\*", "the first chat had no keyword; the word is cut and markup escaped");
        // 설정을 끄면 켜 둔 서버도 바로 따른다
        game_chat::remember("p-talky", &Default::default());
        assert_eq!(game_chat::settings("p-talky").pick("!d x"), None);
        let _ = Supervisor::stop_server(&sup, "p-talky", "talky", true).await;
    }

    /// 데몬보다 새 saba-core를 요구해 로드되지 못한 모듈도 시작 응답과 목록이 이유를 준다
    #[tokio::test]
    async fn module_needing_newer_core_reports_unsatisfied() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("future");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"future\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[module.dependencies]\nsaba-core = \">=99.0.0\"\n",
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        assert!(supervisor.module_loader.get_module("future").unwrap_err().to_string().contains("requires saba-core"));
        let failed = supervisor.module_loader.load_failure("future").unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("module_dependency_unsatisfied"));
        let resp = supervisor.unmet_dependencies_response("future", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_dependency_unsatisfied");
        assert_eq!(resp["module"], "future", "clients name the module in \"update required\"");
        assert!(resp["missing_dependencies"][0].as_str().unwrap().starts_with("saba-core >=99.0.0"));
    }

    /// D3·M1: 이 OS를 지원한다고 선언하지 않은 모듈의 서버는 켜지 않는다
    #[tokio::test]
    async fn a_module_that_does_not_support_this_os_is_not_started() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("elsewhere");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let other = if cfg!(windows) { "linux" } else { "windows" };
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nname = \"elsewhere\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\nplatforms = [\"{other}\"]\n"),
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let resp = supervisor.unmet_dependencies_response("elsewhere", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_platform_unsupported", "{resp}");
        assert_eq!(resp["module"], "elsewhere");
        assert_eq!(resp["platforms"], json!([other]));
        assert_eq!(resp["os"], std::env::consts::OS);
    }

    /// 이 CPU를 지원한다고 선언하지 않은 모듈의 서버도 켜지 않는다 (`architectures`)
    #[tokio::test]
    async fn a_module_that_does_not_support_this_cpu_is_not_started() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("x86only");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let other = if std::env::consts::ARCH == "aarch64" { "x86_64" } else { "aarch64" };
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nname = \"x86only\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\narchitectures = [\"{other}\"]\n"),
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let resp = supervisor.unmet_dependencies_response("x86only", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_arch_unsupported", "{resp}");
        assert_eq!(resp["module"], "x86only");
        assert_eq!(resp["architectures"], json!([other]));
        assert_eq!(resp["arch"], std::env::consts::ARCH);
    }

    /// 데몬보다 새 스키마라 로드되지 못한 모듈은 시작 응답도 그 이유(module_schema_unsupported)를 준다.
    /// 빈 의존성 목록의 module_dependency_unsatisfied로 바꾸지 않는다
    #[tokio::test]
    async fn module_with_newer_schema_reports_unsupported_on_start() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("future");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 99\nname = \"future\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let resp = supervisor.unmet_dependencies_response("future", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_schema_unsupported", "{resp}");
        assert_eq!((resp["required"].clone(), resp["module"].clone()), (json!(99), json!("future")), "{resp}");
    }

    /// 스키마가 틀려 로드되지 못한 모듈(module_schema_invalid)은 빈 의존성 오류로 바꾸지 않는다
    #[tokio::test]
    async fn module_with_invalid_schema_is_not_reported_as_unmet_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"broken\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let _ = supervisor.initialize().await;
        assert_eq!(supervisor.module_loader.load_failure("broken").and_then(|f| f.error_code).as_deref(), Some("module_schema_invalid"));
        assert_eq!(supervisor.unmet_dependencies_response("broken", "srv").await, None);
    }

    /// [module.dependencies]: 익스텐션이 없거나 낮으면 막고, 맞으면 통과. 데몬이 모르는 키(gui)는 넘긴다
    #[tokio::test]
    async fn module_dependencies_check_extension_versions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[module.dependencies]\next-foo = \">=0.2.0\"\ngui = \">=99.0.0\"\n",
        )
        .unwrap();
        let ext_root = tmp.path().join("extensions");
        let write_ext = |version: &str| {
            std::fs::create_dir_all(ext_root.join("foo")).unwrap();
            std::fs::write(ext_root.join("foo").join("manifest.json"), format!(r#"{{"id":"foo","name":"Foo","version":"{version}"}}"#)).unwrap();
        };
        let mut supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        supervisor.module_loader.discover_modules().unwrap();
        let unmet = supervisor.unmet_module_dependencies("game").await.unwrap();
        assert_eq!(unmet, vec!["ext-foo >=0.2.0 (not installed)".to_string()]);

        write_ext("0.1.0");
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("foo").unwrap();
        supervisor.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        assert_eq!(supervisor.unmet_module_dependencies("game").await.unwrap(), vec!["ext-foo >=0.2.0 (installed 0.1.0)".to_string()]);

        write_ext("0.2.0");
        supervisor.extension_manager.as_ref().unwrap().write().await.discover().unwrap();
        assert!(supervisor.unmet_module_dependencies("game").await.is_none());
    }

    // 0.1.7 업그레이드: 0.1.6에서 만든 팰월드는 기본 REST 8212를 저장해 두었다. 8212가 Windows 예약 범위(8135-8234)에
    // 들면 시작 전에 새 인스턴스처럼 옮겨 저장해야 서버가 뜬다. 사용자가 정한 기본값 아닌 포트는 그대로 둔다
    #[test]
    fn a_reserved_default_rest_port_moves_before_start() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("pal");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"pal\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rest\"]\n\n\
             [[settings.fields]]\nname = \"rest_port\"\ntype = \"number\"\nport = true\nlabel = \"REST\"\ndefault = 8212\n",
        )
        .unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut old = crate::instance::ServerInstance::new("pal-016", "pal");
        old.rest_port = Some(8212);
        old.module_settings.insert("rest_port".into(), json!(8212));
        let mut other = crate::instance::ServerInstance::new("pal-other", "pal");
        other.rest_port = Some(8242);
        let mut picked = crate::instance::ServerInstance::new("pal-picked", "pal");
        picked.rest_port = Some(8200);
        let (old_id, picked_id) = (old.id.clone(), picked.id.clone());
        for inst in [old, other, picked] {
            sup.instance_store.add(inst).unwrap();
        }
        let reserved = |p: u16| (8135..=8234).contains(&p);

        let mut events = crate::events::bus().subscribe(None);
        assert_eq!(sup.move_default_rest_port_if(&old_id, reserved, reserved), Some(8252));
        // 리뷰 289: 자동·예약 재시작은 시작 응답을 로그에만 남기므로, 옮긴 일은 이벤트로도 알린다
        let told = std::iter::from_fn(|| events.rx.try_recv().ok()).find(|e| e.kind == "instance.port_moved" && e.data["instanceId"] == old_id.as_str());
        let told = told.expect("instance.port_moved was not emitted");
        assert_eq!((told.data["field"].clone(), told.data["from"].clone(), told.data["to"].clone()), (json!("rest_port"), json!(8212), json!(8252)));
        let moved = sup.instance_store.get(&old_id).unwrap();
        assert_eq!((moved.rest_port, moved.module_settings["rest_port"].clone()), (Some(8252), json!(8252)));
        let mut reloaded = crate::instance::InstanceStore::new(tmp.path().join("instances").to_str().unwrap());
        reloaded.load().unwrap();
        assert_eq!(reloaded.get(&old_id).unwrap().rest_port, Some(8252));

        assert_eq!(sup.move_default_rest_port_if(&picked_id, reserved, reserved), None);
        assert_eq!(sup.instance_store.get(&picked_id).unwrap().rest_port, Some(8200));
    }

    /// managed 시작도 인스턴스가 없으면 비관리 시작과 같은 종류로 알린다 (핸들러가 문구가 아니라 종류로 404를 고른다)
    #[tokio::test]
    async fn a_managed_start_of_a_missing_instance_says_so_by_type() {
        let tmp = tempfile::tempdir().unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let sup = tokio::sync::RwLock::new(supervisor);
        let e = Supervisor::start_managed_server(&sup, "gone", "game", json!({})).await.unwrap_err();
        assert_eq!(e.downcast_ref::<InstanceNotFound>().map(|m| m.0.as_str()), Some("gone"), "{e}");
    }

    /// start_managed_server()가 이미 실행 중인 managed 프로세스를 감지하여
    /// 중복 프로세스 생성을 방지하는지 검증
    #[tokio::test]
    async fn test_start_managed_rejects_duplicate_process() {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-dup-guard-{}", std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);

        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());

        // 테스트 인스턴스 등록
        let inst = crate::instance::ServerInstance {
            id: "test-dup-guard".to_string(),
            name: "TestServer".to_string(),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            port: Some(25565),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            server_build: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            schedule: Vec::new(),
            backup_keep: None,
            extra_args: Vec::new(),
            memory_limit_mb: None,
            accepted_agreements: Default::default(),
            user_settings: Default::default(),
            imported_settings: Default::default(),
            data_roots_backfill_attempted: Default::default(),
            game_chat: Default::default(),
            game_chat_keyword: String::new(),
            controllers: Vec::new(),
            controllers_only: false,
            access: Default::default(),
            kick_players: Vec::new(),
            auto_update: false,
            branch_of: None,
        };
        supervisor.instance_store.add(inst).unwrap();

        // 현재 프로세스의 PID로 reattached stub 생성 (is_running() == true 보장)
        let current_pid = std::process::id();
        let inst_dir = instances_dir.join("test-dup-guard");
        let _ = std::fs::create_dir_all(inst_dir.join("logs"));
        let stub = managed_process::ManagedProcess::create_reattached_stub(current_pid, &inst_dir).await;
        assert!(stub.is_running(), "Reattached stub should report running");

        supervisor.managed_store.insert("test-dup-guard", stub).await;

        // start_managed_server 호출 → 이미 실행 중이므로 중복 방지되어야 함
        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_managed_server(&supervisor, "test-dup-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["success"], true);
        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// tracker에 PID가 등록되어 있고 프로세스가 살아있으면
    /// managed_store에 없더라도 중복 시작을 방지하는지 검증
    #[tokio::test]
    async fn test_start_managed_rejects_when_tracker_has_live_pid() {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-tracker-guard-{}", std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);

        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());

        let inst = crate::instance::ServerInstance {
            id: "test-tracker-guard".to_string(),
            name: "TestServer2".to_string(),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            port: Some(25566),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            server_build: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            schedule: Vec::new(),
            backup_keep: None,
            extra_args: Vec::new(),
            memory_limit_mb: None,
            accepted_agreements: Default::default(),
            user_settings: Default::default(),
            imported_settings: Default::default(),
            data_roots_backfill_attempted: Default::default(),
            game_chat: Default::default(),
            game_chat_keyword: String::new(),
            controllers: Vec::new(),
            controllers_only: false,
            access: Default::default(),
            kick_players: Vec::new(),
            auto_update: false,
            branch_of: None,
        };
        let inst_dir = instances_dir.join("test-tracker-guard");
        let _ = std::fs::create_dir_all(&inst_dir);
        supervisor.instance_store.add(inst).unwrap();

        // managed_store에는 없지만 tracker에 현재 프로세스 PID를 등록
        let current_pid = std::process::id();
        supervisor.tracker.track("test-tracker-guard", current_pid).unwrap();

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_managed_server(&supervisor, "test-tracker-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["success"], true);
        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// non-managed 시작도 추적 중인 PID가 살아 있으면 새로 띄우지 않고 추적을 덮어쓰지 않는다
    #[tokio::test]
    async fn test_start_non_managed_rejects_when_tracker_has_live_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("NonManaged", "minecraft");
        inst.id = "test-nm-guard".into();
        supervisor.instance_store.add(inst).unwrap();
        let current_pid = std::process::id();
        supervisor.tracker.track("test-nm-guard", current_pid).unwrap();

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_server(&supervisor, "test-nm-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);
        assert_eq!(supervisor.read().await.tracker.get_pid("test-nm-guard").unwrap(), current_pid);
    }

    /// 시작 준비(모듈 호출) 중에 들어온 정지는 PID가 없어 바로 성공으로 끝난다. 그 뒤 서버가 떠서는 안 된다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_start_preparation_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\n\
             if sys.argv[1] == 'get_launch_command':\n    time.sleep(2)\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-start".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let starter = {
            let sup = sup.clone();
            tokio::spawn(async move { Supervisor::start_managed_server(&sup, "stop-during-start", "game", json!({})).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = Supervisor::stop_server(&sup, "stop-during-start", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap().unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = sup.read().await;
        assert!(s.tracker.get_pid("stop-during-start").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-start").await.is_none(), "no server was left running");
    }

    /// 익스텐션이 맡은 시작(pre_start handled) 중에 들어온 정지: 정지 hook이 먼저 끝나 익스텐션 서버가 뒤늦게 뜬다.
    /// 시작은 취소로 끝나고, 익스텐션의 정지를 다시 불러 그 서버를 끈다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_an_extension_start_stops_what_the_extension_started() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let ext_root = tmp.path().join("extensions");
        let ext = ext_root.join("box");
        std::fs::create_dir_all(&ext).unwrap();
        let manifest = json!({
            "id": "box", "name": "box", "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": {
                "server.pre_start": { "module": "m", "function": "start" },
                "server.stop": { "module": "m", "function": "stop" },
            },
        });
        std::fs::write(ext.join("manifest.json"), manifest.to_string()).unwrap();
        let log = tmp.path().join("hook.log");
        std::fs::write(
            ext.join("m.py"),
            format!(
                "import json, sys, time\nfn = sys.argv[1]\nif fn == 'start':\n    time.sleep(2)\n\
                 open({log:?}, 'a').write(fn + '\\n')\nprint(json.dumps({{'handled': True, 'success': True}}))\n",
                log = log.to_str().unwrap()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        crate::sandbox::consent_test_component(&ext.join("m.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("box").unwrap();
        sup.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-ext-start".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let starter = {
            let sup = sup.clone();
            tokio::spawn(async move { Supervisor::start_server(&sup, "stop-during-ext-start", "game", json!({})).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = Supervisor::stop_server(&sup, "stop-during-ext-start", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap().unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["stop", "start", "stop"], "the extension's server is stopped again");
    }

    /// managed 시작의 server.pre_start hook도 인스턴스와 시작 설정의 비밀 값을 가려서 넘긴다 (D-E2와 같은 기준)
    #[cfg(unix)]
    #[tokio::test]
    async fn the_managed_pre_start_hook_gets_no_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let ext_root = tmp.path().join("extensions");
        let ext = ext_root.join("box");
        std::fs::create_dir_all(&ext).unwrap();
        let manifest = json!({
            "id": "box", "name": "box", "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": { "server.pre_start": { "module": "m", "function": "start" } },
        });
        std::fs::write(ext.join("manifest.json"), manifest.to_string()).unwrap();
        let seen = tmp.path().join("seen.txt");
        std::fs::write(
            ext.join("m.py"),
            format!(
                "import json, sys\nopen({seen:?}, 'w').write(' '.join(sys.argv) + sys.stdin.read())\n\
                 print(json.dumps({{'handled': True, 'success': False, 'error': 'stop here'}}))\n",
                seen = seen.to_str().unwrap()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        crate::sandbox::consent_test_component(&ext.join("m.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("box").unwrap();
        sup.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "pre-start-secrets".into();
        inst.rest_password = Some("hunter2-rest".into());
        inst.module_settings.insert("server_password".into(), json!("hunter2-join"));
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let config = json!({ "server_password": "hunter2-join", "rest_password": "hunter2-rest", "port": 1 });
        let started = Supervisor::start_managed_server(&sup, "pre-start-secrets", "game", config).await.unwrap();
        assert_eq!(started["success"], false, "{started}");
        let ctx = std::fs::read_to_string(&seen).unwrap();
        assert!(ctx.contains("pre-start-secrets"), "the hook ran with the context: {ctx}");
        assert!(!ctx.contains("hunter2"), "secrets reached the extension: {ctx}");
    }

    // 백로그 M13: 예약 범위 때문에 옮긴 REST 포트를 알리지 않고 저장값만 바꿨다
    #[test]
    fn a_start_reports_the_port_it_moved() {
        let moved = with_port_moved(json!({ "success": false, "error_code": "port_conflict" }), Some((8212, 8242)));
        assert_eq!(moved["port_moved"], json!([{ "field": "rest_port", "from": 8212, "to": 8242 }]));
        assert_eq!(moved["error_code"], "port_conflict");
        assert!(with_port_moved(json!({ "success": true }), None).get("port_moved").is_none());
    }

    /// 시작은 락 밖에서 진행되므로, 같은 인스턴스의 두 번째 시작은 표시로 막는다 (C12)
    #[test]
    fn start_guard_blocks_a_second_start_until_dropped() {
        let set = runtime::Runtimes::default();
        let first = StartGuard::claim(&set, "a", 0).expect("first start");
        assert!(StartGuard::claim(&set, "a", 0).is_none());
        assert!(StartGuard::claim(&set, "b", 0).is_some(), "other instances are not blocked");
        drop(first);
        assert!(StartGuard::claim(&set, "a", 0).is_some());
    }
}
