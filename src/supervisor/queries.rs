//! 인스턴스 점검·조회: 설정 검증, 모듈 `query`·속성 관리, 약관 동의, 진단

use super::*;

impl Supervisor {
    /// Run validation on an instance (via module's validate function).
    /// 모듈 점검 결과의 `issues`에 데몬이 본 설정 값 문제(`setting_issues`)를 더한다 (사용자 지정 2026-11-08)
    pub async fn validate_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str) -> Result<Value> {
        let (call, settings) = {
            let s = sup.read().await;
            (s.instance_call(instance_id, "validate")?, s.setting_issues(instance_id))
        };
        let result = call.run().await?;
        Ok(with_setting_issues(result, settings))
    }

    /// 저장된 모듈 설정이 module.toml이 선언한 타입·범위·선택지·형식에 맞는지 (서버 점검).
    /// `visible_when`으로 숨긴 칸과 값이 없는 칸은 보지 않는다. 한 칸이 한 건이다:
    /// `{code: "setting_invalid", severity: "error", field, error_type, value, message}` (화면이 칸 이름과 error_type으로 번역한다)
    pub(crate) fn setting_issues(&self, instance_id: &str) -> Vec<Value> {
        let Some(instance) = self.instance_store.get(instance_id) else { return Vec::new() };
        let Ok(module) = self.module_loader.get_module(&instance.module_name) else { return Vec::new() };
        let Some(settings) = module.metadata.settings.as_ref() else { return Vec::new() };
        let hidden = crate::validator::hidden_fields(&settings.fields, |name| instance.module_settings.get(name).cloned());
        settings
            .fields
            .iter()
            .filter(|f| !hidden.contains(&f.name))
            .filter_map(|f| {
                let value = instance.module_settings.get(&f.name).filter(|v| !v.is_null())?;
                let e = crate::validator::validate_setting_value(f, Some(value)).err()?;
                // 비밀 칸(password 타입, 이름에 pass·token 같은 낱말)은 값도, 값이 든 원문 메시지도 싣지 않는다.
                // 원격 웹·디스코드도 점검을 부르고 이 응답은 비밀 가리기를 거치지 않는다 (리뷰 P2)
                let secret = f.field_type == "password" || crate::audit::sensitive(&f.name);
                let mut issue = json!({
                    "code": "setting_invalid",
                    "severity": "error",
                    "field": f.name,
                    "error_type": e.error_type,
                });
                if !secret {
                    issue["value"] = value.clone();
                    issue["message"] = json!(e.message);
                }
                Some(issue)
            })
            .collect()
    }

    /// 모듈 `[[queries]]` 조회 (X-2): `validate`와 같은 config에 `args`(입력 이름 → 값)를 더해 lifecycle `function`을 부른다
    pub async fn query_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str, function: &str, args: serde_json::Map<String, Value>) -> Result<Value> {
        let mut call = sup.read().await.instance_call(instance_id, function)?;
        if let Value::Object(cfg) = call.config_mut() {
            cfg.insert("args".to_string(), Value::Object(args));
        }
        call.run().await
    }

    /// 인스턴스를 알아보는 config로 lifecycle 함수 호출을 만든다 (`validate`·조회가 같은 값을 받는다)
    pub(crate) fn instance_call(&self, instance_id: &str, function: &str) -> Result<crate::plugin::PluginCall> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = module.entry_path();

        let mut cfg = serde_json::Map::new();
        // 시작 때와 같은 키와 같은 해석 (옛 module_settings 경로, 작업 폴더 + 모듈 server_executable) (D-S11)
        self.insert_resolved_exe(&mut cfg, instance, &instance.module_name);
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        // Extension 데이터 전달 (Python lifecycle 스크립트에서 참조)
        cfg.insert("extension_data".to_string(), json!(&instance.extension_data));
        // 시작 때와 같은 인스턴스 값 (zomboid server_name·cache_dir 등)
        Self::insert_instance_fields(&mut cfg, instance, &self.instance_store.instance_dir(instance_id));
        // 켜져 있으면 자기 포트를 "다른 프로그램이 쓰는 중"으로 보지 않게
        let server = self.tracker.get_pid(instance_id).ok();
        cfg.insert("running".to_string(), json!(server.is_some()));

        Ok(crate::plugin::PluginCall::new(module_path, function, Value::Object(cfg)).serving(server).in_instance(self.host_context(instance_id)))
    }

    /// 게임 설정 파일 읽기·쓰기·초기화·가져오기 (모듈 configure/read_properties/reset_properties/import_settings)
    pub async fn manage_properties(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        action: &str,  // "read" or "write"
        settings: Option<Value>,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            let module = s.module_loader.get_module(&instance.module_name)?;
            let module_path = module.entry_path();

            let mut cfg = serde_json::Map::new();
            // working_dir 결정: 명시적 설정 > executable_path의 부모 디렉토리
            let effective_working_dir = instance.working_dir.clone()
                .or_else(|| {
                    instance.executable_path.as_ref()
                        .and_then(|p| std::path::Path::new(p).parent())
                        .map(|p| p.to_string_lossy().to_string())
                });
            if let Some(work_dir) = &effective_working_dir {
                cfg.insert("working_dir".to_string(), json!(work_dir));
            }
            // Extension 데이터 전달 — Python lifecycle 스크립트가 플랫폼 경로를 올바르게 판단하도록
            cfg.insert("extension_data".to_string(), json!(&instance.extension_data));
            // 시작 때와 같은 인스턴스 값 (예: zomboid cache_dir — 다른 서버의 파일을 건드리지 않게)
            Self::insert_instance_fields(&mut cfg, instance, &s.instance_store.instance_dir(instance_id));
            if let Some(port) = instance.port {
                cfg.entry("port".to_string()).or_insert(json!(port));
            }

            let function = match action {
                "write" | "configure" => {
                    if let Some(s) = settings {
                        cfg.insert("settings".to_string(), s);
                    }
                    "configure"
                }
                "reset" => "reset_properties",
                // 지울 범위 (모듈 `reset_preview`가 밝힌 id). 없으면 모듈의 기본 범위
                "reset_server" => {
                    if let Some(scope) = settings {
                        cfg.insert("scope".to_string(), scope);
                    }
                    "reset_server"
                }
                "reset_preview" => "reset_preview",
                // 플레이어 목록 (`[[lists]]`)의 지금 명단
                "read_lists" => "read_lists",
                // 실제 설정 파일을 saba-chan 키로 (설정 화면이 캐시 대신 실제 값을 보여 준다)
                "import" => "import_settings",
                // 사바쨩이 고칠 수 있는 게임 설정 파일 (가져온 서버를 해제할 때 되돌린다)
                "config_files" => "config_files",
                _ => "read_properties",
            };

            crate::plugin::PluginCall::new(module_path, function, Value::Object(cfg)).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// 모듈이 선언한 약관(`[[agreements]]`)과 이 인스턴스의 동의 여부. 이름은 모듈 locales `agreements.<id>` (없으면 id)
    pub fn agreement_views(&self, instance: &crate::instance::ServerInstance, lang: &str) -> Vec<Value> {
        let Ok(module) = self.module_loader.get_module(&instance.module_name) else { return Vec::new() };
        if module.metadata.agreements.is_empty() {
            return Vec::new();
        }
        let lang = crate::chat::i18n::normalize_language(lang);
        let locales = [crate::actions::module_locale(&module, &lang), crate::actions::module_locale(&module, "en")];
        module.metadata.agreements.iter().map(|a| {
            let pointer = format!("/agreements/{}", a.id);
            let label = locales.iter().find_map(|l| l.pointer(&pointer).and_then(Value::as_str)).unwrap_or(&a.id);
            let at = instance.accepted_agreements.get(&a.id);
            json!({ "id": a.id, "label": label, "url": a.url, "accepted": at.is_some(), "accepted_at": at })
        }).collect()
    }

    /// 아직 동의하지 않은 약관 (켜기 전 점검)
    pub fn missing_agreements(&self, instance_id: &str, lang: &str) -> Vec<Value> {
        let Some(instance) = self.instance_store.get(instance_id) else { return Vec::new() };
        self.agreement_views(instance, lang).into_iter().filter(|v| v["accepted"] == json!(false)).collect()
    }

    /// 약관 동의를 기록하고 모듈 `accept_agreements`(`accepted_agreements: [id]`)로 모듈 파일(eula.txt 등)을 쓰게 한다.
    /// 모듈이 선언하지 않은 id면 Err(그 id)
    pub async fn accept_agreements(sup: &tokio::sync::RwLock<Self>, instance_id: &str, ids: &[String]) -> Result<std::result::Result<Value, String>> {
        let call = {
            let mut s = sup.write().await;
            let mut instance = s.instance_store.get(instance_id).cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let module = s.module_loader.get_module(&instance.module_name)?;
            if let Some(unknown) = ids.iter().find(|id| !module.metadata.agreements.iter().any(|a| &a.id == *id)) {
                return Ok(Err(unknown.clone()));
            }
            let now = chrono::Utc::now().to_rfc3339();
            for id in ids {
                instance.accepted_agreements.entry(id.clone()).or_insert_with(|| now.clone());
            }
            let accepted: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
            s.instance_store.update(instance_id, instance)?;
            let mut call = s.instance_call(instance_id, "accept_agreements")?;
            if let Value::Object(cfg) = call.config_mut() {
                cfg.insert("accepted_agreements".to_string(), json!(accepted));
            }
            call
        };
        Ok(Ok(call.run().await?))
    }

    /// Diagnose errors from instance logs
    pub async fn diagnose_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            let module = s.module_loader.get_module(&instance.module_name)?;
            let module_path = module.entry_path();

            let mut cfg = serde_json::Map::new();
            if let Some(work_dir) = &instance.working_dir {
                cfg.insert("working_dir".to_string(), json!(work_dir));
            }

            // If managed, provide recent console output for diagnosis
            if let Some(proc) = s.managed_store.get(&instance.id).await {
                let recent = proc.get_recent_console(500).await;
                // 이번 실행의 줄만 (재부착 때 디스크에서 읽어 온 지난 실행의 오류를 지금 문제로 보지 않게)
                let log_lines: Vec<String> = recent
                    .iter()
                    .filter(|l| l.timestamp >= proc.started_at && l.source != managed_process::LogSource::Input)
                    .map(|l| l.content.clone())
                    .collect();
                cfg.insert("log_lines".to_string(), json!(log_lines));
            }

            crate::plugin::PluginCall::new(module_path, "diagnose_log", Value::Object(cfg)).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;
        Ok(result)
    }
}

/// 모듈 점검 결과에 설정 값 문제를 더한다: `issues`가 있으면 뒤에 붙이고, 치명 문제가 아니므로 `success`는 그대로 둔다
pub(super) fn with_setting_issues(mut result: Value, extra: Vec<Value>) -> Value {
    if extra.is_empty() {
        return result;
    }
    match &mut result {
        Value::Object(map) => match map.get_mut("issues") {
            Some(Value::Array(issues)) => issues.extend(extra),
            _ => {
                map.insert("issues".to_string(), Value::Array(extra));
            }
        },
        Value::Array(issues) => issues.extend(extra),
        _ => return json!({ "issues": extra }),
    }
    result
}
