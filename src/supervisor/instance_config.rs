//! 인스턴스 → 모듈 config: REST 포트 설정, 저장된 포트, 실행 파일, 인스턴스 필드·데이터 폴더

use super::*;

impl Supervisor {
    /// REST를 받는 모듈의 REST 포트 설정 이름 (module_name → `[protocols.rest] port_setting`).
    /// `check_port_conflicts`가 그 설정 값을 REST 포트로 비교한다
    pub(crate) fn rest_port_settings(&self) -> HashMap<String, String> {
        let modules = self.module_loader.modules().unwrap_or_default();
        modules.iter().filter_map(|m| Some((m.metadata.name.clone(), m.metadata.rest()?.port_setting.clone()))).collect()
    }

    /// 인스턴스가 저장해 둔 포트: 게임 포트와 모듈의 포트 설정 값
    pub(crate) fn saved_ports(&self, instance: &crate::instance::ServerInstance) -> Vec<u16> {
        let fields: Vec<String> = self.module_loader.get_module(&instance.module_name).ok()
            .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
            .unwrap_or_default();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        crate::validator::instance_ports(instance, &fields).into_iter().map(|(_, p)| p).collect()
    }

    /// 인스턴스의 실행 파일 경로를 자동 해석합니다.
    ///
    /// 우선순위:
    /// 1. `instance.executable_path` — 사용자가 명시한 경로 (override)
    /// 2. `working_dir / module.server_executable` — 모듈이 정의한 바이너리 파일명으로 자동 계산
    ///
    /// 모듈에 `server_executable`이 정의되어 있으면 `working_dir`만으로 실행 파일을 찾을 수 있으므로
    /// 사용자가 별도로 `executable_path`를 설정할 필요가 없습니다.
    pub(crate) fn resolve_executable(&self, instance: &crate::instance::ServerInstance, module_name: &str) -> Option<String> {
        // 1. 사용자 명시 경로 우선
        if let Some(ref exe_path) = instance.executable_path {
            if !exe_path.is_empty() {
                return Some(exe_path.clone());
            }
        }

        // 2. module_settings에 executable_path가 있으면 사용 (기존 인스턴스 호환)
        if let Some(exe_val) = instance.module_settings.get("executable_path") {
            if let Some(exe_str) = exe_val.as_str() {
                if !exe_str.is_empty() {
                    return Some(exe_str.to_string());
                }
            }
        }

        // 3. working_dir + module.server_executable 자동 계산
        if let Some(ref work_dir) = instance.working_dir {
            if let Ok(modules) = self.module_loader.modules() {
                if let Some(server_exe) = modules.iter().find(|m| m.metadata.name == module_name).and_then(|m| m.metadata.server_executable.as_ref()) {
                    let resolved = std::path::Path::new(work_dir).join(server_exe);
                    return Some(resolved.to_string_lossy().to_string());
                }
            }
        }

        None
    }

    /// 해석된 실행 파일 경로를 config map에 삽입합니다.
    pub(super) fn insert_resolved_exe(&self, config: &mut serde_json::Map<String, Value>, instance: &crate::instance::ServerInstance, module_name: &str) {
        if let Some(exe_path) = self.resolve_executable(instance, module_name) {
            config.insert("server_executable".to_string(), json!(&exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            config.insert("working_dir".to_string(), json!(work_dir));
        }
    }

    /// 모듈 함수가 인스턴스를 알아보는 데 쓰는 값 (포트·비밀번호·모듈 설정·데이터 폴더 등). REST 접속값도 모듈 설정이라
    /// 모듈이 선언한 설정 이름 그대로 실린다 (`[protocols.rest]`).
    /// 시작·정지·설정·초기화 모두 같은 값을 받아야 같은 서버 파일을 다룬다. 이미 있는 키는 두다.
    pub(crate) fn insert_instance_fields(cfg: &mut serde_json::Map<String, Value>, instance: &crate::instance::ServerInstance, instance_dir: &std::path::Path) {
        // 모듈이 서버 폴더 옆에 둔 데이터(좀보이드 cache_dir 등): 격리 실행이 쓸 수 있는 폴더로 싣는다
        cfg.insert(crate::sandbox::INSTANCE_DATA_DIRS.to_string(), json!(Self::instance_data_dirs(instance, instance_dir)));
        // RCON 접속 정보는 데몬이 만든 값이 기준이다 (RCON만 받는 모듈, `rcon.json`이 있을 때). 모듈은 이 값을 게임 설정에 쓴다
        if let Some(access) = crate::protocol::rcon::load(instance_dir) {
            cfg.insert("rcon_port".into(), json!(access.port));
            cfg.insert("rcon_password".into(), json!(access.password));
        }
        let mut put = |key: &str, value: Value| {
            cfg.entry(key.to_string()).or_insert(value);
        };
        // 인스턴스 폴더: 모듈이 그 안에 받아 둔 것(가져온 서버의 포터블 런타임 등)을 시작·점검 모두 같은 자리에서 본다
        put("instance_dir", json!(instance_dir.to_string_lossy()));
        for (key, value) in &instance.module_settings {
            // 격리 실행이 쓰기를 받는 폴더 키는 데몬만 정한다 (모듈은 import_settings로 아무 값이나 넣을 수 있다)
            // 호출마다 데몬이 정하는 키(`RESERVED_CALL_KEYS`)도 모듈 설정 값으로는 싣지 않는다
            if !crate::sandbox::grants_write(key) && !module_loader::RESERVED_CALL_KEYS.contains(&key.as_str()) {
                put(key, value.clone());
            }
        }
        put("protocol_mode", json!(&instance.protocol_mode));
        if let Some(v) = &instance.server_version { put("server_version", json!(v)); }
        // 모듈 도우미(daemon_stdin_command 등)가 데몬 API로 이 인스턴스를 가리킬 때 쓴다
        put("instance_id", json!(&instance.id));
        // 사용자가 더한 실행 인자. 넣을 자리를 아는 모듈은 직접 넣고 `extra_args_placed: true`를 돌려준다
        put("extra_args", json!(&instance.extra_args));
    }

    /// 모듈 설정 중 인스턴스 폴더 안을 가리키는 폴더: 그 인스턴스 폴더 바로 아래 항목 단위로 돌려준다.
    /// 인스턴스 폴더 자체, `..`가 든 경로, 데몬이 쓰는 항목(`instance.json`·`run/` 등)은 뺀다
    pub(crate) fn instance_data_dirs(instance: &crate::instance::ServerInstance, instance_dir: &std::path::Path) -> Vec<String> {
        use std::path::Component;
        let mut dirs: Vec<String> = instance
            .module_settings
            .values()
            .filter_map(Value::as_str)
            .filter_map(|value| {
                let rel = std::path::Path::new(value).strip_prefix(instance_dir).ok()?;
                if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                    return None;
                }
                let first = rel.components().next()?.as_os_str().to_str()?;
                (!is_daemon_entry(first)).then(|| instance_dir.join(first).to_string_lossy().into_owned())
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }
}
