//! 무결성 확인과 복구 (`/api/updates/integrity`·`repair`)

use super::*;
use super::apply::{ApplyRequest, DownloadRequest, apply_updates_with, download_components_with};

/// GET /api/updates/integrity — 업데이터가 설치하며 기록한 파일 해시와 지금 파일을 비교
pub(super) async fn check_integrity(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let report = state.manager.read().await.verify_integrity();

    let components: Vec<Value> = report.components.iter().map(|c| {
        json!({
            "component": c.component,
            "display_name": c.display_name,
            "status": format!("{:?}", c.status),
            "expected_hash": c.expected_hash,
            "actual_hash": c.actual_hash,
            "file_path": c.file_path,
            "message": c.message,
        })
    }).collect();

    Json(json!({
        "success": true,
        "checked_at": report.checked_at,
        "overall": format!("{:?}", report.overall),
        "total": report.total,
        "verified": report.verified,
        "failed": report.failed,
        "skipped": report.skipped,
        "components": components,
    }))
}

/// 설치 점검: 무결성 검사와 지금 판 그대로 다시 받을 계획. 고칠 것이 있는데 확인한 적이 없으면 먼저 확인한다 (받기와 같다).
/// `mark`면 고칠 것을 다시 받을 수 있게 표시한다
async fn repair_plan(state: &UpdateState, mark: bool) -> Result<Value, String> {
    let report = state.manager.read().await.verify_integrity();
    let broken = report.components.iter().any(|c| matches!(c.status, IntegrityStatus::Tampered | IntegrityStatus::FileNotFound));
    if broken && state.manager.read().await.get_status().components.is_empty() {
        saba_chan_updater_lib::check_shared(&state.manager, None).await.map_err(|e| format!("Update check failed: {}", e))?;
    }
    let mut mgr = state.manager.write().await;
    let plan = mgr.repair_plan(&report);
    if mark && !plan.repair.is_empty() {
        mgr.mark_for_repair(&plan);
    }
    Ok(json!({
        "success": true,
        "verified": report.verified,
        "failed": report.failed,
        "skipped": report.skipped,
        "repair": plan.repair,
        "unavailable": plan.unavailable,
    }))
}

/// GET /api/updates/repair — 설치 점검: 무결성 요약(`verified`·`failed`·`skipped`)과 바뀌었거나 없어진 구성요소 중
/// 지금 판 그대로 다시 받을 것(`repair: [{component, display_name, installed, offered}]`)과 받을 수 없는 것(`unavailable`, 다른 판이 나왔거나 릴리즈에 없다)
pub(super) async fn get_repair(State(state): State<UpdateState>) -> axum::response::Response {
    match repair_plan(&state, false).await {
        Ok(plan) => Json(plan).into_response(),
        Err(e) => crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "update_check_failed", e),
    }
}

/// POST /api/updates/repair — 점검한 뒤 고칠 것만 지금 판 그대로 다시 받아(`download_components_with`) 적용한다(`apply_updates_with`).
/// 데몬·GUI·업데이터가 끼면 적용하지 않고 `apply`처럼 `requires_updater`·`needs_updater`로 돌려준다(부른 쪽이 `launch-updater`).
/// 답: GET의 계획에 고친 키 `repaired`, `needs_updater`, `requires_updater`, 받거나 적용하지 못한 것 `errors`
pub(super) async fn repair(State(state): State<UpdateState>) -> axum::response::Response {
    use crate::ipc::internal::into_internal;
    let plan = match repair_plan(&state, true).await {
        Ok(plan) => plan,
        Err(e) => return crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "update_check_failed", e),
    };
    let keys: Vec<String> = plan["repair"].as_array().into_iter().flatten().filter_map(|r| r["component"].as_str().map(str::to_string)).collect();
    if keys.is_empty() {
        return Json(repair_result(plan, &json!({}), &json!({}))).into_response();
    }
    let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest { components: keys }).await).await.body;
    let got = downloaded_keys(&downloaded);
    let applied = if got.is_empty() {
        json!({})
    } else {
        into_internal(apply_updates_with(state, ApplyRequest { components: got, ..Default::default() }).await).await.body
    };
    Json(repair_result(plan, &downloaded, &applied)).into_response()
}

fn downloaded_keys(downloaded: &Value) -> Vec<String> {
    downloaded["downloaded"].as_array().into_iter().flatten().filter_map(|d| d["component"].as_str().map(str::to_string)).collect()
}

/// 계획에 받기·적용 결과를 더한다. 받아서 데몬이 바로 적용한 것만 고친 것이다
pub(super) fn repair_result(mut plan: Value, downloaded: &Value, applied: &Value) -> Value {
    let applied_names: Vec<&str> = applied["applied"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let repaired: Vec<String> = downloaded_keys(downloaded).into_iter().filter(|k| applied_names.contains(&Component::from_manifest_key(k).display_name().as_str())).collect();
    let mut errors: Vec<Value> = downloaded["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| json!(format!("{}: {}", e["component"].as_str().unwrap_or_default(), e["error"].as_str().unwrap_or_default())))
        .collect();
    errors.extend(applied["errors"].as_array().into_iter().flatten().cloned());
    plan["success"] = json!(errors.is_empty());
    plan["repaired"] = json!(repaired);
    plan["needs_updater"] = applied.get("needs_updater").cloned().unwrap_or_else(|| json!([]));
    plan["requires_updater"] = json!(applied["requires_updater"].as_bool() == Some(true));
    plan["errors"] = json!(errors);
    plan
}
