//! 업데이트 설정 (`/api/updates/config`)과 `config.json` `updates` 절 읽기·쓰기

use super::*;

/// GET /api/updates/config
pub(super) async fn get_config(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let config = mgr.get_config();
    Json(json!({
        "success": true,
        "config": config,
        // 이 데몬이 모의 릴레이(scripts/mock-release-server.js)의 서명을 믿는 개발 빌드인가.
        // 아니면 GUI는 MOCK 출처를 감추거나 막는다
        "mock_relay": cfg!(feature = "mock-relay"),
    }))
}

/// 앱에서 고를 수 있는 업데이트 채널 (위 채널은 아래 채널 릴리즈도 받는다, updater `relay::CHANNELS`). 설치할 때 고르는 채널과 같다
use saba_chan_updater_lib::relay::SELECTABLE_CHANNELS;

/// 업데이트 설정 부분 변경. 바꿀 수 없는 키나 형식이 틀린 값은 아무것도 바꾸지 않고 거절한다 (말없이 넘기면 저장된 줄 안다)
pub(super) fn apply_config_patch(cfg: &mut UpdateConfig, body: &Value) -> Result<(), (axum::http::StatusCode, Value)> {
    use axum::http::StatusCode;
    let refuse = |status: StatusCode, error: String, code: &str| {
        Err((status, json!({ "success": false, "error": error, "error_code": code })))
    };
    let Some(patch) = body.as_object() else {
        return refuse(StatusCode::BAD_REQUEST, "The body must be a JSON object".into(), "invalid_update_config_value");
    };
    let settable = saba_chan_updater_lib::SETTABLE_CONFIG_KEYS;
    if let Some(key) = patch.keys().find(|k| !settable.contains(&k.as_str())) {
        return refuse(
            StatusCode::BAD_REQUEST,
            format!("Unknown update setting '{}' ({})", key, settable.join(", ")),
            "unknown_update_config_key",
        );
    }
    let mut next = cfg.clone();
    for (key, v) in patch {
        let wrong_type = || {
            refuse(
                StatusCode::BAD_REQUEST,
                format!("Update setting '{}' has the wrong type: {}", key, v),
                "invalid_update_config_value",
            )
        };
        match key.as_str() {
            "enabled" | "auto_download" | "auto_apply" => {
                let Some(b) = v.as_bool() else { return wrong_type() };
                *match key.as_str() {
                    "enabled" => &mut next.enabled,
                    "auto_download" => &mut next.auto_download,
                    _ => &mut next.auto_apply,
                } = b;
            }
            // 업데이트 출처는 앱에서 바꾸지 않는다: config.json의 updates.relay_url로만.
            // 모의 릴레이를 믿는 개발 빌드만 앱(개발 옵션)에서 바꿀 수 있다. 비우기(기본 릴레이로 되돌리기)는 늘 된다
            "relay_url" => {
                let Some(url) = v.as_str() else { return wrong_type() };
                if !url.is_empty() && !cfg!(feature = "mock-relay") {
                    return refuse(
                        StatusCode::FORBIDDEN,
                        "The update source can only be changed in config.json (updates.relay_url)".into(),
                        "relay_url_not_settable",
                    );
                }
                next.relay_url = if url.is_empty() { None } else { Some(url.to_string()) };
            }
            // 채널은 정식(stable)·미리보기(rc)·베타(beta)만 고른다 (nightly는 쓰지 않는다)
            _ => {
                let Some(channel) = v.as_str() else { return wrong_type() };
                if !SELECTABLE_CHANNELS.contains(&channel) {
                    return refuse(
                        StatusCode::BAD_REQUEST,
                        format!("Unknown update channel '{}' (stable, rc, beta)", channel),
                        "invalid_channel",
                    );
                }
                next.channel = channel.to_string();
            }
        }
    }
    *cfg = next;
    Ok(())
}

/// PUT /api/updates/config
pub(super) async fn set_config(
    State(state): State<UpdateState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;
    let mut cfg = mgr.get_config();
    if let Err((status, body)) = apply_config_patch(&mut cfg, &body) {
        return (status, Json(body)).into_response();
    }

    let channel_changed = mgr.get_config().channel != cfg.channel;
    if let Err(resp) = adopt_config(&mut mgr, &ConfigFile::default_location(), cfg.clone()) {
        return resp;
    }
    drop(mgr);
    // 채널을 바꾸면 곧바로 확인해 자동 적용 길을 탄다 (내림은 하지 않는다: 확인이 낮은 판을 거절한다)
    if channel_changed && cfg.enabled {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = check_and_follow_up(&state).await {
                tracing::warn!("[Updates] Check after a channel change failed: {}", e);
            }
        });
    }

    Json(json!({
        "success": true,
        "config": cfg,
    }))
    .into_response()
}

// ── 설정 유틸리티 ──

/// 저장된 업데이트 설정 (`config.json`의 `updates`). 없거나 깨졌으면 기본값.
pub(crate) fn load_updater_config() -> UpdateConfig {
    load_updater_config_from(&ConfigFile::default_location())
}

pub(super) fn load_updater_config_from(file: &ConfigFile) -> UpdateConfig {
    match file.section("updates") {
        serde_json::Value::Null => UpdateConfig::default(),
        mut v => {
            // 0.1.1 이름(`relay_base_url`)과 새 이름이 같이 있으면 별칭이 겹쳐 설정 전체가 깨진다: 새 이름을 쓴다
            if let Some(obj) = v.as_object_mut().filter(|o| o.contains_key("relay_url")) {
                obj.remove("relay_base_url");
            }
            let mut cfg: UpdateConfig = serde_json::from_value(v).unwrap_or_else(|e| {
                tracing::warn!("[Updates] Ignoring invalid 'updates' config: {}", e);
                UpdateConfig::default()
            });
            // 앱 업데이트는 묻지 않고 적용한다: 옛 설정은 한 번만 켠다. 그 뒤 고른 값은 그대로 둔다
            if !cfg.live_update_migrated {
                cfg.auto_apply = true;
                cfg.auto_download = true;
                cfg.live_update_migrated = true;
                if let Err(e) = save_updater_config_to(file, &cfg) {
                    tracing::warn!("[Updates] Failed to persist update config: {}", e);
                }
            }
            cfg
        }
    }
}

/// 사용자가 바꾼 설정: 디스크에 먼저 저장하고, 저장했을 때만 관리자 설정을 바꾼다.
/// 저장하지 못하면 아무것도 바꾸지 않고 500 `save_failed`로 답한다 (다시 켜면 되돌아가는 설정을 성공으로 보이지 않게)
#[allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답
pub(super) fn adopt_config(mgr: &mut UpdateManager, file: &ConfigFile, cfg: UpdateConfig) -> Result<(), axum::response::Response> {
    save_updater_config_to(file, &cfg).map_err(|e| {
        tracing::warn!("[Updates] Failed to persist update config: {}", e);
        crate::ipc::handlers::error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "save_failed", format!("Failed to save update settings: {e}"))
    })?;
    mgr.update_config(cfg);
    Ok(())
}

pub(super) fn save_updater_config(cfg: &UpdateConfig) -> std::io::Result<()> {
    save_updater_config_to(&ConfigFile::default_location(), cfg)
}

pub(super) fn save_updater_config_to(file: &ConfigFile, cfg: &UpdateConfig) -> std::io::Result<()> {
    file.set_section("updates", serde_json::to_value(cfg).map_err(std::io::Error::other)?)
}
