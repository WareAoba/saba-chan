//! hook·명령·slot: 바인딩 고르기, 실행 계획(`HookPlan`), hook 실행

use super::*;

impl ExtensionManager {
    /// 지정된 hook에 바인딩된 활성 익스텐션 목록. 실행 순서대로 (`priority`, 그다음 id)
    pub fn hooks_for(&self, hook_name: &str) -> Vec<(&DiscoveredExtension, &HookBinding)> {
        let mut result: Vec<_> = self
            .discovered
            .values()
            .filter(|ext| self.enabled.contains(&ext.manifest.id))
            .filter_map(|ext| ext.manifest.hooks.get(hook_name).map(|b| (ext, b)))
            .collect();
        result.sort_by(|(a, _), (b, _)| {
            (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id))
        });
        result
    }

    /// 이 인스턴스(`ext_data`)에서 쓸 수 있는 익스텐션 명령. `priority`, id 순
    pub fn commands_for(&self, ext_data: &HashMap<String, Value>) -> Vec<(&str, &ExtensionCommand)> {
        let mut exts: Vec<_> = self.discovered.values().filter(|e| self.enabled.contains(&e.manifest.id)).collect();
        exts.sort_by(|a, b| (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id)));
        exts.into_iter()
            .flat_map(|ext| ext.manifest.commands.iter().map(move |c| (ext.manifest.id.as_str(), c)))
            .filter(|(_, c)| c.condition.as_deref().is_none_or(|cond| Self::evaluate_condition(cond, ext_data)))
            .collect()
    }

    /// 익스텐션 명령 `name`을 부를 계획. 이 인스턴스에서 그 명령을 가진 익스텐션이 없으면 None.
    /// `context`에는 extension_data가 들어 있어야 한다
    /// 익스텐션 명령 실행 계획. 인자는 명령의 `inputs`로 검사해 `context.args`(이름 → 값)에 넣는다.
    /// 그런 명령이 없으면 None, 인자가 틀리면 Some(Err).
    pub fn plan_command(
        &self,
        name: &str,
        args: &crate::commands::Args,
        mut context: Value,
    ) -> Option<Result<HookPlan, crate::commands::PlanError>> {
        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let (ext_id, cmd) = self
            .commands_for(&ext_data)
            .into_iter()
            .find(|(_, c)| c.name.eq_ignore_ascii_case(name))?;
        let ext = self.discovered.get(ext_id)?;
        let bound = match crate::commands::bind(&cmd.inputs, args) {
            Ok(bound) => bound,
            Err(e) => return Some(Err(e)),
        };
        if let Some(obj) = context.as_object_mut() {
            obj.insert("args".into(), Value::Object(bound));
        }
        let target = ext
            .manifest
            .python_modules
            .get(&cmd.module)
            .map(|rel| (ext.dir.join(rel).to_string_lossy().to_string(), cmd.function.clone()))
            .ok_or_else(|| format!("Unknown python module: {}", cmd.module));
        Some(Ok(HookPlan {
            hook: format!("command.{}", cmd.name),
            calls: vec![HookCall { ext_id: ext_id.to_string(), target, context: self.context_for(ext_id, &context) }],
        }))
    }

    /// 켜진 익스텐션이 서버를 만들 때 묻는 칸 (`AddServer.options`의 선언 그대로, `field`가 있는 것만), priority·id 순
    pub fn create_options(&self) -> Vec<Value> {
        let mut exts: Vec<&DiscoveredExtension> = self.discovered.values().filter(|e| self.enabled.contains(&e.manifest.id)).collect();
        exts.sort_by(|a, b| (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id)));
        exts.iter()
            .filter_map(|e| e.manifest.cli.as_ref())
            .flat_map(|cli| normalized_slots(cli).remove(SLOT_OPTIONS).and_then(|v| v.as_array().cloned()).unwrap_or_default())
            .filter(|f| f.get("field").and_then(Value::as_str).is_some())
            .collect()
    }

    /// 서버 하나에 붙일 데이터 slot (Q-7): 켜진 익스텐션만, 조건이 참인 것만, 값은 찾아서.
    /// `ServerCard.badge` → `{extension, text}`, `ServerDetail.status` → `{extension, label, value}`
    /// (값은 `extension_status[<ext>]` → `extension_status.default` → `extension_data` 순, 못 찾으면 그 줄을 뺀다)
    pub fn server_slots(
        &self,
        ext_data: &HashMap<String, Value>,
        ext_status: &HashMap<String, Value>,
    ) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        let mut exts: Vec<&DiscoveredExtension> = self.discovered.values().filter(|e| self.enabled.contains(&e.manifest.id)).collect();
        exts.sort_by(|a, b| (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id)));
        let mut push = |slot: &str, item: Value| {
            if let Value::Array(list) = out.entry(slot.to_string()).or_insert_with(|| Value::Array(Vec::new())) {
                list.push(item);
            }
        };
        for ext in exts {
            let id = &ext.manifest.id;
            let Some(cli) = &ext.manifest.cli else { continue };
            let slots = normalized_slots(cli);
            for item in slots.get(SLOT_BADGE).and_then(Value::as_array).into_iter().flatten() {
                let shown = item.get("condition").and_then(Value::as_str).is_none_or(|c| Self::evaluate_condition(c, ext_data));
                if let (true, Some(text)) = (shown, item.get("text").and_then(Value::as_str)) {
                    push(SLOT_BADGE, serde_json::json!({ "extension": id, "text": text }));
                }
            }
            for item in slots.get(SLOT_STATUS).and_then(Value::as_array).into_iter().flatten() {
                let (Some(label), Some(key)) = (item.get("label").and_then(Value::as_str), item.get("value_from").and_then(Value::as_str)) else { continue };
                // 비밀값처럼 보이는 키는 보이지 않는다 (목록은 원격으로도 나간다)
                if crate::actions::is_secret_key(key, &[]) {
                    continue;
                }
                let found = ext_status
                    .get(id)
                    .and_then(|s| s.get(key))
                    .or_else(|| ext_status.get("default").and_then(|s| s.get(key)))
                    .or_else(|| ext_data.get(key))
                    .filter(|v| !v.is_null());
                if let Some(v) = found {
                    let value = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    push(SLOT_STATUS, serde_json::json!({ "extension": id, "label": label, "value": value }));
                }
            }
        }
        out
    }

    /// 조건 문자열 평가: "instance.ext_data.<key>" → ext_data[key] == true
    pub fn evaluate_condition(
        condition: &str,
        ext_data: &HashMap<String, Value>,
    ) -> bool {
        // "instance.ext_data.<key>" 패턴
        if let Some(key) = condition.strip_prefix("instance.ext_data.") {
            match ext_data.get(key) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
                Some(Value::String(s)) => !s.is_empty(),
                _ => false,
            }
        } else {
            tracing::warn!("Unknown condition pattern: {}", condition);
            false
        }
    }

    /// 이 익스텐션을 부를 context: `extension_config`에는 그 익스텐션 자신의 설정만 넣는다
    /// (`{<id>: {...}}`). 다른 익스텐션의 설정은 넘기지 않는다.
    fn context_for(&self, ext_id: &str, context: &Value) -> Value {
        let mut context = context.clone();
        if let Some(obj) = context.as_object_mut() {
            let own = serde_json::json!({ ext_id: self.get_extension_config(ext_id) });
            obj.insert("extension_config".to_string(), own);
        }
        context
    }

    /// 이 hook에서 부를 익스텐션 함수 목록을 정한다 (조건 평가, 익스텐션마다 자기 extension_config 주입).
    /// Python 실행은 돌려준 `HookPlan`으로, 매니저 락을 놓은 뒤에 한다 (C12).
    pub fn plan_hook(&self, hook_name: &str, context: Value, only: Option<&str>) -> HookPlan {
        let mut hooks = self.hooks_for(hook_name);
        if let Some(id) = only {
            hooks.retain(|(ext, _)| ext.manifest.id == id);
        }
        if hooks.is_empty() {
            return HookPlan { hook: hook_name.to_string(), calls: Vec::new() };
        }

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let calls = hooks
            .into_iter()
            .filter(|(ext, binding)| match binding.condition {
                Some(ref cond) if !Self::evaluate_condition(cond, &ext_data) => {
                    tracing::debug!("Hook '{}' from '{}' skipped: condition '{}' is false", hook_name, ext.manifest.id, cond);
                    false
                }
                _ => true,
            })
            .map(|(ext, binding)| {
                let target = match ext.manifest.python_modules.get(&binding.module) {
                    Some(rel_path) => Ok((ext.dir.join(rel_path).to_string_lossy().to_string(), binding.function.clone())),
                    None => {
                        tracing::error!(
                            "Extension '{}' hook '{}' references unknown module '{}'",
                            ext.manifest.id,
                            hook_name,
                            binding.module
                        );
                        Err(format!("Unknown python module: {}", binding.module))
                    }
                };
                HookCall { ext_id: ext.manifest.id.clone(), target, context: self.context_for(&ext.manifest.id, &context) }
            })
            .collect();
        HookPlan { hook: hook_name.to_string(), calls }
    }
}

/// `ExtensionManager::plan_hook`이 정한 hook 실행 목록
pub struct HookPlan {
    hook: String,
    pub(super) calls: Vec<HookCall>,
}

pub(super) struct HookCall {
    pub(super) ext_id: String,
    /// (Python 파일 경로, 함수 이름), 또는 왜 부를 수 없는지
    target: std::result::Result<(String, String), String>,
    /// 이 익스텐션에 넘길 context (자기 설정만 든 `extension_config` 포함)
    pub(super) context: Value,
}

impl HookPlan {
    /// 우선순위 순서로 부른다. `handled=true`가 나오면 나머지는 건너뛴다 (chain-of-responsibility).
    /// 실패한 익스텐션은 기록만 하고 다음으로 넘어간다.
    pub async fn run(self, timeout_secs: u64) -> Vec<(String, Result<Value>)> {
        let mut results = Vec::new();
        for call in self.calls {
            let (module_path, function) = match call.target {
                Ok(t) => t,
                Err(msg) => {
                    results.push((call.ext_id, Err(anyhow::anyhow!(msg))));
                    continue;
                }
            };
            tracing::debug!("Dispatching hook '{}' → ext '{}' → {}::{}", self.hook, call.ext_id, module_path, function);
            let result = crate::plugin::run_plugin_with_timeout(&module_path, &function, call.context, timeout_secs).await;
            match result {
                Ok(val) => {
                    let handled = val.get("handled").and_then(|h| h.as_bool()) == Some(true);
                    results.push((call.ext_id.clone(), Ok(val)));
                    if handled {
                        tracing::debug!("Hook '{}' handled by extension '{}', skipping remaining", self.hook, call.ext_id);
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Hook '{}' ext '{}' failed: {}", self.hook, call.ext_id, e);
                    results.push((call.ext_id, Err(e.context("Hook dispatch failed"))));
                }
            }
        }
        results
    }
}

/// 서버 hook(`server.*`, 익스텐션 명령)이 모두 받는 문맥 (D10): `instance_id`, `instance_name`, `instance_dir`, `module`,
/// `extension_data`, `instance`(비밀 값을 가린 인스턴스). hook마다 다른 키는 부르는 쪽이 덧붙인다
pub(crate) fn hook_context(
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    module: Option<&crate::module_loader::ModuleMetadata>,
) -> Value {
    serde_json::json!({
        "instance_id": &instance.id,
        "instance_name": &instance.name,
        "instance_dir": instance_dir.to_string_lossy(),
        "module": &instance.module_name,
        "extension_data": &instance.extension_data,
        "instance": redact_for_hooks(module, &serde_json::to_value(instance).unwrap_or_default()),
    })
}

/// hook에 넘기는 값(인스턴스·설정)에서 비밀을 가린다: REST 비밀번호, 모듈이 password로 선언했거나 이름이 비밀스러운 값 (D-E2).
/// 키 이름으로만 가린다: 인스턴스를 넘기면 비밀값과 같은 글자가 이름·경로에서도 지워진다
pub(crate) fn redact_for_hooks(module: Option<&crate::module_loader::ModuleMetadata>, value: &Value) -> Value {
    crate::audit::redact(module, None, None, Some(value)).1.unwrap_or_default()
}

/// 매니저 락은 실행 목록을 정할 때만 잡고, Python은 락 밖에서 부른다 (C12).
/// 오래 걸리는 hook 동안에도 익스텐션 켜기·끄기와 다른 hook이 막히지 않는다.
pub async fn run_hook(mgr: &tokio::sync::RwLock<ExtensionManager>, hook_name: &str, context: Value) -> Vec<(String, Result<Value>)> {
    run_hook_timed(mgr, hook_name, context, crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await
}

pub async fn run_hook_timed(
    mgr: &tokio::sync::RwLock<ExtensionManager>,
    hook_name: &str,
    context: Value,
    timeout_secs: u64,
) -> Vec<(String, Result<Value>)> {
    // 계획(옛 경로)부터 실행이 끝날 때까지 익스텐션 폴더를 옮기지 못하게 한다 (hook이 그 폴더의 파일을 실행하고 쓴다)
    let _in_use = crate::component_lock::in_use().await;
    let plan = mgr.read().await.plan_hook(hook_name, context, None);
    plan.run(timeout_secs).await
}

/// 익스텐션 하나에만 (예: 방금 켠 익스텐션의 `daemon.startup`)
pub async fn run_hook_to(
    mgr: &tokio::sync::RwLock<ExtensionManager>,
    ext_id: &str,
    hook_name: &str,
    context: Value,
) -> Option<Result<Value>> {
    let _in_use = crate::component_lock::in_use().await;
    let plan = mgr.read().await.plan_hook(hook_name, context, Some(ext_id));
    plan.run(crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await.into_iter().next().map(|(_, r)| r)
}
