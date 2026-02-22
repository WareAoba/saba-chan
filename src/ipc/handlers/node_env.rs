use axum::response::IntoResponse;
use serde_json::json;

/// GET /api/node-env/status — Node.js 포터블 환경 상태
pub async fn node_env_status() -> impl IntoResponse {
    let info = crate::node_env::status().await;
    axum::Json(info)
}

/// POST /api/node-env/setup — Node.js 포터블 환경 부트스트랩 (다운로드 포함)
pub async fn node_env_setup() -> impl IntoResponse {
    match crate::node_env::find_or_bootstrap().await {
        Ok(node_path) => {
            let p = node_path.to_string_lossy().to_string();
            axum::Json(json!({
                "success": true,
                "node_path": p,
                "message": "Node.js 환경이 준비되었습니다.",
            }))
        }
        Err(e) => axum::Json(json!({
            "success": false,
            "error": format!("{}", e),
        })),
    }
}
