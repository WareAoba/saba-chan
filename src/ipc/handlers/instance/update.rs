use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::ipc::IPCServer;

/// 업데이트 확인/적용에 쓸 실제 서버 설치 경로를 계산한다.
///
/// 인스턴스의 `working_dir`을 우선한다 — 마이그레이션 인스턴스는 서버가 외부 경로
/// (예: Steam 라이브러리)에 있고, 프로비저닝 인스턴스도 설치 완료 시
/// `working_dir = instance_dir/install_subdir`로 저장되므로 동일 규칙으로 통일된다.
/// `working_dir`이 없을 때만 `instance_dir/install_subdir` 폴백을 사용한다.
/// (이 규칙이 없으면 마이그레이션 서버는 local buildid를 못 찾아 영구 "업데이트 필요"로
/// 오판되고, apply-update는 기존 서버 대신 saba-chan 폴더에 새 복사본을 설치한다.)
fn resolve_install_dir(
    working_dir: Option<&str>,
    instance_dir: &std::path::Path,
    install_subdir: &str,
) -> String {
    match working_dir {
        Some(dir) if !dir.is_empty() => dir.to_string(),
        _ => instance_dir.join(install_subdir).to_string_lossy().to_string(),
    }
}

/// GET /api/instance/:id/check-update - 게임 서버 업데이트 확인
///
/// extension_data에 install_method_steamcmd 가 있는 인스턴스에 대해
/// server.check_update hook을 디스패치하여 buildid 비교 결과를 반환합니다.
pub async fn check_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id),
                    "error_code": "instance_not_found",
                })),
            )
                .into_response();
        }
    };

    // extension_data 가 비어 있으면 hook 이 동작하지 않으므로 조기 반환
    if instance.extension_data.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({
                "update_available": false,
                "reason": "no_extension_data",
            })),
        )
            .into_response();
    }

    let instance_dir = supervisor.instance_store.instance_dir(&id);

    // 모듈에서 install config 가져오기 (install_subdir, app_id 등)
    let install_config = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .and_then(|m| m.metadata.install.clone());

    // install_dir 계산: 인스턴스의 working_dir 우선 (resolve_install_dir 참조).
    let install_subdir = install_config
        .as_ref()
        .and_then(|cfg| cfg.install_subdir.clone())
        .unwrap_or_else(|| "server".to_string());
    let install_dir_str = resolve_install_dir(
        instance.working_dir.as_deref(),
        &instance_dir,
        &install_subdir,
    );

    // app_id: extension_data에서 우선, 없으면 module config에서
    let app_id = instance
        .extension_data
        .get("steamcmd_app_id")
        .and_then(|v| v.as_u64())
        .or_else(|| install_config.as_ref().and_then(|cfg| cfg.app_id.map(|id| id as u64)));

    // supervisor lock 해제 (hook 실행 중 필요 없음)
    drop(supervisor);

    let ctx = json!({
        "instance_id": &id,
        "instance_name": &instance.name,
        "module": &instance.module_name,
        "instance_dir": instance_dir.to_string_lossy(),
        "install_dir": &install_dir_str,
        "app_id": app_id,
        "extension_data": &instance.extension_data,
    });

    let mgr = state.extension_manager.read().await;
    let results = mgr.dispatch_hook("server.check_update", ctx).await;

    // 첫 번째 성공 결과를 클라이언트로 전달
    for (_ext_id, result) in &results {
        if let Ok(val) = result {
            return (StatusCode::OK, Json(val.clone())).into_response();
        }
    }

    // hook이 등록되지 않았거나 모두 condition 불일치
    (
        StatusCode::OK,
        Json(json!({
            "update_available": false,
            "reason": "no_hook_handled",
        })),
    )
        .into_response()
}

/// POST /api/instance/:id/apply-update - 게임 서버 업데이트 적용
///
/// SteamCMD install (== update)을 재실행하여 서버를 최신 빌드로 업데이트합니다.
/// 서버가 실행 중이면 업데이트할 수 없습니다.
pub async fn apply_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id),
                    "error_code": "instance_not_found",
                })),
            )
                .into_response();
        }
    };

    // 실행 중이면 업데이트 불가
    if supervisor.tracker.get_pid(&id).is_ok() {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "Cannot update while server is running. Stop the server first.",
                "error_code": "server_running",
            })),
        )
            .into_response();
    }

    // SteamCMD 인스턴스인지 확인
    let is_steamcmd = instance
        .extension_data
        .get("install_method_steamcmd")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_steamcmd {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Only SteamCMD-installed instances can be updated this way.",
                "error_code": "not_steamcmd",
            })),
        )
            .into_response();
    }

    // 모듈에서 install config 가져오기
    let module_info = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .map(|m| (m.metadata.install.clone(), m.path.clone()));

    let (install_config, _module_path) = match module_info {
        Some((Some(cfg), path)) => (cfg, path),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Module install configuration not found.",
                    "error_code": "no_install_config",
                })),
            )
                .into_response();
        }
    };

    let instance_dir = supervisor.instance_store.instance_dir(&id);
    let install_subdir = install_config
        .install_subdir
        .clone()
        .unwrap_or_else(|| "server".to_string());
    // 마이그레이션 인스턴스는 working_dir(외부 경로)의 서버를 업데이트해야 한다 (resolve_install_dir 참조).
    let install_dir_str = resolve_install_dir(
        instance.working_dir.as_deref(),
        &instance_dir,
        &install_subdir,
    );

    let tracker = state.provision_tracker.clone();
    let supervisor_arc = state.supervisor.clone();
    let instance_name = instance.name.clone();
    let module_name_owned = instance.module_name.clone();

    // 프로비저닝 상태 등록
    tracker.update(
        &instance_name,
        crate::ipc::ProvisionProgress {
            step: 0,
            total: 1,
            label: "steamcmd".to_string(),
            message: "Updating server...".to_string(),
            done: false,
            error: None,
            percent: Some(0),
            steps: None,
        },
    );

    // supervisor lock 해제 후 백그라운드 태스크 실행
    drop(supervisor);

    let tracker_done = tracker.clone();
    let name_done = instance_name.clone();

    tokio::spawn(async move {
        // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
        let lifecycle_path = {
            let sup = supervisor_arc.read().await;
            match sup.module_loader.get_module(&module_name_owned) {
                Ok(m) => format!("{}/lifecycle.py", m.path),
                Err(e) => {
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Update failed".to_string(),
                            done: true,
                            error: Some(format!("Module not found: {}", e)),
                            percent: Some(100),
                            steps: None,
                        },
                    );
                    return;
                }
            }
        };
        // supervisor read lock 해제됨

        // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
        let result = {
            let config = serde_json::json!({
                "install_dir": &install_dir_str,
                "accept_eula": true,
            });

            let tracker_progress = tracker_done.clone();
            let name_progress = name_done.clone();
            let on_progress = move |prog: crate::plugin::ExtensionProgress| {
                let pct = prog.percent.unwrap_or(0);
                let msg = prog.message.clone().unwrap_or_default();
                tracker_progress.update(
                    &name_progress,
                    crate::ipc::ProvisionProgress {
                        step: prog.step.unwrap_or(0),
                        total: prog.total.unwrap_or(1),
                        label: prog.label.unwrap_or_else(|| "steamcmd".to_string()),
                        message: msg,
                        done: false,
                        error: None,
                        percent: Some(pct),
                        steps: None,
                    },
                );
            };

            crate::plugin::run_plugin_with_progress_and_timeout(
                &lifecycle_path,
                "install_server",
                config,
                on_progress,
                1800, // 30분 — SteamCMD 대용량 다운로드 허용
            )
            .await
        };

        match result {
            Ok(val) => {
                let success = val
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    tracing::info!("Server update completed for '{}'", name_done);
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Server updated successfully".to_string(),
                            done: true,
                            error: None,
                            percent: Some(100),
                            steps: None,
                        },
                    );
                } else {
                    let err_msg = val
                        .get("message")
                        .or_else(|| val.get("error"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error")
                        .to_string();
                    tracing::error!("Server update failed for '{}': {}", name_done, err_msg);
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Update failed".to_string(),
                            done: true,
                            error: Some(err_msg),
                            percent: Some(100),
                            steps: None,
                        },
                    );
                }
            }
            Err(e) => {
                tracing::error!("Server update error for '{}': {}", name_done, e);
                tracker_done.update(
                    &name_done,
                    crate::ipc::ProvisionProgress {
                        step: 0,
                        total: 1,
                        label: "done".to_string(),
                        message: "Update failed".to_string(),
                        done: true,
                        error: Some(format!("{}", e)),
                        percent: Some(100),
                        steps: None,
                    },
                );
            }
        }

        // 5초 후 프로비저닝 tracker 자동 정리
        let tracker_cleanup = tracker_done.clone();
        let name_cleanup = name_done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracker_cleanup.remove(&name_cleanup);
        });
    });

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "id": id,
            "provisioning": true,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 마이그레이션 인스턴스: working_dir(외부 경로)이 그대로 install_dir이 되어야 한다.
    /// 이 규칙이 없으면 local buildid를 못 찾아 영구 "업데이트 필요"로 오판되고,
    /// apply-update가 기존 서버 대신 saba-chan 폴더에 새 복사본을 설치한다.
    #[test]
    fn resolve_install_dir_prefers_working_dir() {
        let instance_dir = std::path::Path::new("C:/data/instances/abc");
        let resolved = resolve_install_dir(
            Some("D:/SteamLibrary/steamapps/common/PalServer"),
            instance_dir,
            "server",
        );
        assert_eq!(resolved, "D:/SteamLibrary/steamapps/common/PalServer");
    }

    #[test]
    fn resolve_install_dir_falls_back_to_instance_dir() {
        let instance_dir = std::path::Path::new("C:/data/instances/abc");
        // working_dir 없음
        let resolved = resolve_install_dir(None, instance_dir, "server");
        assert!(resolved.ends_with("server"));
        assert!(resolved.contains("abc"));
        // working_dir 빈 문자열도 폴백
        let resolved_empty = resolve_install_dir(Some(""), instance_dir, "server");
        assert_eq!(resolved, resolved_empty);
    }
}

