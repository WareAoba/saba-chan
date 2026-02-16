//! IPC 토큰 기반 인증 미들웨어
//!
//! 데몬 시작 시 랜덤 토큰을 생성하여 `config/.ipc_token` 파일에 저장합니다.
//! GUI, CLI, Bot은 이 파일을 읽어서 `X-Saba-Token` 헤더에 포함시킵니다.
//! 토큰이 일치하지 않는 요청은 401 Unauthorized로 거부됩니다.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

/// 토큰 파일의 기본 경로
fn token_file_path() -> String {
    std::env::var("SABA_TOKEN_PATH").unwrap_or_else(|_| {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(|appdata| format!("{}\\saba-chan\\.ipc_token", appdata))
                .unwrap_or_else(|_| "config/.ipc_token".to_string())
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("HOME")
                .map(|home| format!("{}/.config/saba-chan/.ipc_token", home))
                .unwrap_or_else(|_| "config/.ipc_token".to_string())
        }
    })
}

/// 데몬 시작 시 호출: 랜덤 토큰을 생성하고 파일에 저장
pub fn generate_and_save_token() -> anyhow::Result<Arc<String>> {
    let token = uuid::Uuid::new_v4().to_string();
    let path = token_file_path();

    // 부모 디렉토리 생성
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &token)?;

    // 파일 퍼미션 제한 (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!("IPC auth token saved to {}", path);
    Ok(Arc::new(token))
}

/// 토큰 파일에서 읽기 (클라이언트 측에서 사용)
pub fn read_token_from_file() -> Option<String> {
    let path = token_file_path();
    std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string())
}

/// axum 미들웨어: `X-Saba-Token` 헤더 검증
///
/// 인증 비활성화 시 (SABA_AUTH_DISABLED=1), 모든 요청을 허용합니다.
pub async fn auth_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // 환경 변수로 인증 비활성화 가능 (개발/테스트용)
    if std::env::var("SABA_AUTH_DISABLED").unwrap_or_default() == "1" {
        return Ok(next.run(req).await);
    }

    // 토큰 파일에서 기대 토큰 읽기
    let expected = match read_token_from_file() {
        Some(t) => t,
        None => {
            // 토큰 파일이 없으면 인증 없이 통과 (하위 호환성)
            tracing::warn!("No IPC token file found at {} — skipping auth", token_file_path());
            return Ok(next.run(req).await);
        }
    };

    // 헤더에서 토큰 추출    
    let provided = req
        .headers()
        .get("X-Saba-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided == expected {
        Ok(next.run(req).await)
    } else {
        tracing::warn!(
            "IPC auth failed: invalid token from {}",
            req.uri()
        );
        Err(StatusCode::UNAUTHORIZED)
    }
}
