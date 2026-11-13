//! 인스턴스 만들기·설치의 본체와 그 공용 거절 (핸들러 `ipc::handlers::instance`가 부른다)

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::ipc::IPCServer;
use crate::service::Failure;

pub(crate) mod create;
pub(crate) mod provision;
pub(crate) mod settings;

/// 요청 값 하나가 잘못됐다는 400 `invalid_format`: `error`에 이유
pub(crate) fn invalid_field(field: &str, error: impl Into<String>) -> Failure {
    Failure::new(StatusCode::BAD_REQUEST, json!({ "success": false, "error": error.into(), "error_code": "invalid_format", "fields": [{"field": field}] }))
}

/// 쉼표가 든 서버 이름 (만들기·이름 바꾸기 공통 400 `invalid_name`)
pub(crate) fn invalid_name() -> Failure {
    Failure::new(StatusCode::BAD_REQUEST, json!({ "success": false, "error": "A server name cannot contain a comma", "error_code": "invalid_name", "fields": [{ "field": "name" }] }))
}

/// 필수 익스텐션이 없어 `action`(예: "create instance")을 할 수 없다는 422 응답
pub(crate) fn extension_required(action: &str, found: &crate::extension::MissingExtensions) -> Failure {
    let msg = if !found.not_installed.is_empty() {
        format!(
            "Cannot {}: required extension(s) not installed: {}. Install them in Settings → Extensions first.",
            action,
            found.not_installed.join(", ")
        )
    } else {
        format!(
            "Cannot {}: required extension(s) not enabled: {}. Enable them in Settings → Extensions first.",
            action,
            found.missing.join(", ")
        )
    };
    let error = json!({
        "success": false,
        "error": msg,
        "error_code": "extension_required",
        "missing_extensions": found.missing,
        "not_installed": found.not_installed,
    });
    Failure::new(StatusCode::UNPROCESSABLE_ENTITY, error)
}

/// 요청 본문의 약관 id 목록 (`key`: 문자열 배열, 없으면 빈 목록). 동의는 이 PC(GUI·CLI)에서만 받는다:
/// 원격(디스코드·챗봇)에서 온 요청이 하나라도 담으면 403 `agreement_local_only`
pub(crate) fn requested_agreements(payload: &Value, key: &str) -> Result<Vec<String>, Failure> {
    let ids: Vec<String> = match payload.get(key) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            items.iter().filter_map(Value::as_str).map(str::to_string).collect()
        }
        Some(_) => {
            return Err(invalid_field(key, format!("'{}' must be a list of agreement ids", key)));
        }
    };
    if !ids.is_empty() && crate::audit::current().is_relay() {
        return Err(Failure::coded(StatusCode::FORBIDDEN, "agreement_local_only", "Agreements can only be accepted on this PC"));
    }
    Ok(ids)
}

/// `id` 말고 다른 인스턴스들의 인스턴스 폴더와 서버 폴더 (`sandbox::release`가 그들이 함께 쓰는 위 폴더를 둔다)
pub(crate) fn other_instance_folders(supervisor: &crate::supervisor::Supervisor, id: &str) -> Vec<std::path::PathBuf> {
    supervisor.instance_store.list().iter()
        .filter(|i| i.id != id)
        .flat_map(|i| [Some(supervisor.instance_store.instance_dir(&i.id)), i.working_dir.as_deref().map(std::path::PathBuf::from)])
        .flatten()
        .collect()
}

/// 인스턴스를 지우기 전 익스텐션 hook `server.pre_delete`. 익스텐션이 정리했으면 파일 잠금이 풀리기를 잠깐 기다린다
pub(crate) async fn pre_delete_hook(state: &IPCServer, instance: &crate::instance::ServerInstance) {
    let ext_mgr = state.extension_manager.clone();
    let ctx = {
        let sup = state.supervisor.read().await;
        let module = sup.module_loader.get_module(&instance.module_name).ok();
        crate::extension::hook_context(instance, &sup.instance_store.instance_dir(&instance.id), module.as_ref().map(|m| &m.metadata))
    };
    let results = crate::plugin::with_instance(Some(state.host_context(&instance.id)), crate::extension::run_hook(&ext_mgr, "server.pre_delete", ctx)).await;
    let handled = results.iter().any(|(_id, r)| {
        r.as_ref()
            .map(|v| v.get("handled").and_then(|h| h.as_bool()) == Some(true))
            .unwrap_or(false)
    });
    if handled {
        tracing::info!("Extension handled pre_delete cleanup for '{}'", instance.name);
        // 익스텐션 정리 후 파일 잠금 해제 대기
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// 사용자가 런타임 경로 설정(`runtime`)에 정한 값을 그 모듈의 격리 실행에 연다 (폴더 권한 고치기는 오래 걸릴 수 있어 따로)
pub(crate) fn open_user_runtime(module: &crate::module_loader::LoadedModule, instance: &crate::instance::ServerInstance) {
    for path in module.metadata.user_runtime_paths(instance) {
        let (entry, path) = (module.entry_path(), path.to_string());
        tokio::task::spawn_blocking(move || crate::sandbox::open_user_runtime(std::path::Path::new(&entry), &path));
    }
}
