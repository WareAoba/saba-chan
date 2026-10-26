use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::super::IPCServer;

// ── GUI Settings ──

/// GET /api/config/gui — GUI 설정(config.json의 gui) 조회
pub async fn get_gui_config(State(state): State<IPCServer>) -> impl IntoResponse {
    let mut json = state.config_store.get_gui_settings_json().await;
    json["revision"] = json!(crate::config_store::patch::revision(&json));
    (StatusCode::OK, Json(json)).into_response()
}

/// PUT /api/config/gui — GUI 설정 저장 (최상위 키 단위로 얹는다)
pub async fn save_gui_config(State(state): State<IPCServer>, Json(settings): Json<Value>) -> impl IntoResponse {
    write_gui_config(&state, settings, crate::config_store::Apply::Overlay).await
}

/// PATCH /api/config/gui — 바뀐 칸만 (JSON Merge Patch, `null`은 지우기). `revision`을 주면 그사이 바뀌었을 때 409 `settings_changed`
pub async fn patch_gui_config(State(state): State<IPCServer>, Json(settings): Json<Value>) -> impl IntoResponse {
    write_gui_config(&state, settings, crate::config_store::Apply::MergePatch).await
}

async fn write_gui_config(state: &IPCServer, mut settings: Value, how: crate::config_store::Apply) -> axum::response::Response {
    // PUT은 예전처럼 revision을 보지 않는다 (GET 본문을 그대로 돌려보내는 클라이언트가 있다)
    let revision = crate::config_store::patch::take_revision(&mut settings).filter(|_| how == crate::config_store::Apply::MergePatch);
    match state.config_store.write_gui(settings, how, revision.as_deref()).await {
        Ok((saved, revision)) => {
            // 저장에 성공한 뒤에만 런타임 포트 검사 플래그를 맞춘다
            state.supervisor.read().await.skip_port_check.store(!saved.port_conflict_check, std::sync::atomic::Ordering::Relaxed);
            (StatusCode::OK, Json(json!({ "ok": true, "revision": revision }))).into_response()
        }
        Err(crate::config_store::ConfigStoreError::RevisionConflict(revision)) => (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "The settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": revision })),
        )
            .into_response(),
        Err(crate::config_store::ConfigStoreError::InvalidData(msg)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": msg, "error_code": "invalid_request" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("{}", e) })),
        )
            .into_response(),
    }
}

// ── 기본값 ──

/// GET /api/config/:kind/defaults — `gui` 또는 `bot` 설정의 기본값
pub async fn get_config_defaults(Path(kind): Path<String>) -> impl IntoResponse {
    match crate::config_store::ConfigStore::defaults_json(&kind) {
        Some(defaults) => (StatusCode::OK, Json(json!({ "ok": true, "defaults": defaults }))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("Unknown config: {}", kind), "error_code": "not_found" })),
        )
            .into_response(),
    }
}

/// POST /api/config/:kind/reset — `{keys?: [..]}`를 기본값으로 (없으면 전부). 응답 `{ok, config}`
pub async fn reset_config(
    State(state): State<IPCServer>,
    Path(kind): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let bad = |msg: &str| {
        (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": msg, "error_code": "invalid_request" }))).into_response()
    };
    // 빈 본문 = 전부. 깨진 JSON은 전부 초기화로 치지 않고 거부한다
    let body: Option<Value> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        match serde_json::from_slice(&body) {
            Ok(v @ Value::Object(_)) => Some(v),
            Ok(_) => return bad("body must be a JSON object"),
            Err(_) => return bad("body is not valid JSON"),
        }
    };
    let keys: Option<Vec<String>> = match body.as_ref().and_then(|b| b.get("keys")).cloned() {
        None => None,
        Some(Value::Null) => return bad("keys is null; omit keys to reset everything"),
        Some(k) => match serde_json::from_value(k) {
            Ok(keys) => Some(keys),
            Err(_) => return bad("keys must be an array of setting names"),
        },
    };
    let old_relay = state.config_store.get_bot_config().await.cloud.relay_url;
    match state.config_store.reset_to_defaults(&kind, keys.as_deref()).await {
        Ok(config) => {
            // 저장에 성공한 뒤에만 런타임 플래그(포트 검사)를 맞춘다
            if kind == "gui" {
                if let Some(check) = config.get("portConflictCheck").and_then(Value::as_bool) {
                    state.supervisor.read().await.skip_port_check.store(!check, std::sync::atomic::Ordering::Relaxed);
                }
            }
            // 봇 설정 저장과 같이, 릴레이 주소가 바뀌면 에이전트를 새 주소로 다시 시작한다
            let new_relay = config.pointer("/cloud/relayUrl").and_then(Value::as_str).unwrap_or("");
            if kind == "bot" && new_relay.trim() != old_relay.trim() && state.relay_agent.status().running {
                crate::relay_agent::restart(&state).await;
            }
            (StatusCode::OK, Json(json!({ "ok": true, "config": config }))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e.to_string(), "error_code": "invalid_request" })),
        )
            .into_response(),
    }
}

// ── Client namespaces ──

/// GET /api/config/client/:kind — 클라이언트별 설정 네임스페이스 (예: `cli`)
pub async fn get_client_config(
    Path(kind): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match state.config_store.get_client_section(&kind) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

/// PUT /api/config/client/:kind — 최상위 키 단위로 머지해 저장, 결과를 돌려준다
pub async fn save_client_config(
    Path(kind): Path<String>,
    State(state): State<IPCServer>,
    Json(patch): Json<Value>,
) -> impl IntoResponse {
    match state.config_store.merge_client_section(&kind, patch) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(crate::config_store::ConfigStoreError::InvalidData(msg)) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// ── Node Token ──

/// GET /api/config/node-token
pub async fn get_node_token(State(_state): State<IPCServer>) -> impl IntoResponse {
    let token = super::relay::load_node_token().unwrap_or_default();
    (StatusCode::OK, Json(json!({ "token": token }))).into_response()
}

/// PUT /api/config/node-token
pub async fn save_node_token(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let token = payload
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // 페어링 완료 때 데몬이 이미 저장하고 재시작했으면 같은 토큰을 다시 받아도 재시작하지 않는다
    match super::relay::write_node_token(Some(token)) {
        Ok(changed) => {
            super::relay::adopt_node_token(&state, token, changed).await;
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to write node token: {}", e) })),
        )
            .into_response(),
    }
}

/// DELETE /api/config/node-token — 연결 해제 (`relay::disconnect`)
pub async fn delete_node_token(State(state): State<IPCServer>) -> impl IntoResponse {
    if let Err(e) = super::relay::disconnect(&state).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to delete node token: {}", e) })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

// ── Module Locales ──

/// GET /api/module/:name/locales — 모듈 로케일 파일 전체 반환
pub async fn get_module_locales(
    State(state): State<IPCServer>,
    Path(module_name): Path<String>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let modules_dir = supervisor.module_loader.modules_dir().to_string();
    drop(supervisor);

    let locales_dir = std::path::PathBuf::from(&modules_dir)
        .join(&module_name)
        .join("locales");

    let mut result: HashMap<String, Value> = HashMap::new();

    if locales_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&locales_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(lang) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                                result.insert(lang.to_string(), parsed);
                            }
                        }
                    }
                }
            }
        }
    }

    (StatusCode::OK, Json(json!(result)))
}

// ── System Components ──

/// GET /api/system/components — 설치된 컴포넌트 버전
///
/// `installed-manifest.json`(업데이터가 적용할 때마다 기록)이 기준이고, 거기에 없는
/// 컴포넌트는 이 빌드에 묶인 `versions.toml` 버전으로 채운다. 설치 구성에 들지 않는 것(헤드리스의 GUI)은 뺀다.
pub async fn get_system_components(State(_state): State<IPCServer>) -> impl IntoResponse {
    use saba_chan_updater_lib::{profile::Profile, versions, UpdateManager};

    let bundled = versions::bundled();
    let components =
        installed_components(&bundled.components, UpdateManager::load_installed_manifest(), Profile::load());

    let manifest_path = saba_chan_updater_lib::constants::resolve_installed_manifest_path();
    let last_updated = std::fs::metadata(&manifest_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string());

    (
        StatusCode::OK,
        Json(json!({
            "version": bundled.release,
            "components": components,
            "lastUpdated": last_updated
        })),
    )
        .into_response()
}

fn installed_components(
    bundled: &std::collections::BTreeMap<String, String>,
    installed: std::collections::HashMap<String, String>,
    profile: saba_chan_updater_lib::profile::Profile,
) -> std::collections::BTreeMap<String, String> {
    let mut components = bundled.clone();
    components.extend(installed);
    components.retain(|key, _| profile.includes(key));
    components
}

#[cfg(test)]
mod system_components_tests {
    use super::installed_components;
    use saba_chan_updater_lib::profile::Profile;

    #[test]
    fn a_headless_install_does_not_report_a_gui() {
        let bundled = [("gui", "0.1.9"), ("saba-core", "0.1.9")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        let installed: std::collections::HashMap<_, _> =
            [("saba-core".to_string(), "0.1.8".to_string())].into_iter().collect();

        let desktop = installed_components(&bundled, installed.clone(), Profile::Desktop);
        assert_eq!(desktop.get("gui").map(String::as_str), Some("0.1.9"));
        assert_eq!(desktop.get("saba-core").map(String::as_str), Some("0.1.8"), "the install record wins");

        let headless = installed_components(&bundled, installed, Profile::Headless);
        assert_eq!(headless.keys().collect::<Vec<_>>(), ["saba-core"]);
    }
}

// ── Directory Scan (migration) ──
