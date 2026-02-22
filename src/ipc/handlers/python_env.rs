use axum::response::IntoResponse;
use serde_json::json;

/// GET /api/python-env/status — Python 가상환경 상태
#[allow(dead_code)]
pub async fn python_env_status() -> impl IntoResponse {
    let info = crate::python_env::status().await;
    axum::Json(info)
}

/// POST /api/python-env/setup — Python 가상환경 생성/초기화
#[allow(dead_code)]
pub async fn python_env_setup() -> impl IntoResponse {
    match crate::python_env::ensure_venv().await {
        Ok(python_path) => axum::Json(json!({
            "success": true,
            "python_path": python_path.to_string_lossy(),
            "message": "Python 가상환경이 준비되었습니다.",
        })),
        Err(e) => axum::Json(json!({
            "success": false,
            "error": format!("{}", e),
        })),
    }
}

/// POST /api/python-env/pip-install — pip 패키지 설치
#[allow(dead_code)]
pub async fn python_env_pip_install(
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let packages: Vec<String> = match body.get("packages") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => {
            return axum::Json(json!({
                "success": false,
                "error": "\"packages\" 배열이 필요합니다. 예: {\"packages\": [\"requests\"]}"
            }));
        }
    };

    if packages.is_empty() {
        return axum::Json(json!({
            "success": false,
            "error": "설치할 패키지가 없습니다."
        }));
    }

    let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
    match crate::python_env::pip_install(&pkg_refs).await {
        Ok(()) => axum::Json(json!({
            "success": true,
            "installed": packages,
            "message": format!("{} 패키지 설치 완료", packages.len()),
        })),
        Err(e) => axum::Json(json!({
            "success": false,
            "error": format!("{}", e),
        })),
    }
}
