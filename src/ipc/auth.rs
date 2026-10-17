//! IPC 토큰 기반 인증 미들웨어
//!
//! 데몬 시작 시 랜덤 토큰을 생성하여 파일에 저장하고 메모리에 캐시합니다.
//! GUI, CLI, MCP는 이 파일을 읽어서 `X-Saba-Token` 헤더에 포함시킵니다.
//! 토큰이 일치하지 않는 요청은 401 Unauthorized로 거부됩니다.
//!
//! 추가로 [`host_guard_middleware`]가 루프백이 아닌 `Host` 헤더를 거부한다
//! (DNS rebinding으로 브라우저 페이지가 로컬 데몬에 접근하는 것을 차단).

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

/// 데몬이 생성한 토큰을 메모리에 캐시 (파일 I/O 제거)
/// RwLock을 사용하여 재기록 가능 (OnceLock은 한 번만 쓸 수 있어 데몬 재시작 시 문제)
static CACHED_TOKEN: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// 토큰 파일의 기본 경로
fn token_file_path() -> String {
    saba_chan_updater_lib::constants::token_file_path()
        .to_string_lossy()
        .to_string()
}

/// 데몬 시작 시 호출: 랜덤 토큰을 생성하고 파일에 저장 + 메모리 캐시
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

    // 메모리에 캐시 (이후 auth_middleware는 파일을 읽지 않음)
    {
        let mut cached = CACHED_TOKEN.write().unwrap_or_else(|e| e.into_inner());
        let had_previous = cached.is_some();
        *cached = Some(token.clone());
        if had_previous {
            tracing::info!("IPC auth token regenerated and cached");
        } else {
            tracing::info!("IPC auth token generated and cached");
        }
    }

    tracing::info!("IPC auth token saved to {}", path);
    Ok(Arc::new(token))
}

/// 토큰 파일에서 읽기 (클라이언트 측에서 사용)
pub fn read_token_from_file() -> Option<String> {
    let path = token_file_path();
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|token| !token.is_empty())
}

/// 인증 실패 로그 스팸을 억제하기 위한 rate-limiter
static AUTH_FAIL_LAST_LOG: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static AUTH_FAIL_SUPPRESSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `SABA_AUTH_DISABLED=1` 우회는 디버그(테스트 포함) 빌드에서만 허용한다.
/// 릴리스 바이너리는 환경 변수와 무관하게 항상 토큰을 검사한다.
fn auth_disabled_by_env() -> bool {
    cfg!(debug_assertions) && std::env::var("SABA_AUTH_DISABLED").map(|v| v == "1").unwrap_or(false)
}

/// axum 미들웨어: `X-Saba-Token` 헤더 검증
///
/// 디버그 빌드에서 `SABA_AUTH_DISABLED=1`이면 모든 요청을 허용합니다.
pub async fn auth_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if auth_disabled_by_env() {
        return Ok(next.run(req).await);
    }

    // 메모리 캐시된 토큰 사용 (파일 I/O 제거)
    // generate_and_save_token()에서 설정한 캐시를 우선 사용하고,
    // 캐시가 없으면(외부 데몬 등) 파일에서 읽기
    let expected = {
        let cached = CACHED_TOKEN.read().unwrap_or_else(|e| e.into_inner());
        cached.clone()
    };
    let expected = expected.or_else(read_token_from_file);
    authenticate_request(req, next, expected).await
}

async fn authenticate_request(
    req: Request<Body>,
    next: Next,
    expected: Option<String>,
) -> Result<Response, StatusCode> {
    // 헤더에서 토큰 추출    
    let provided = req
        .headers()
        .get("X-Saba-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if expected.as_deref().is_some_and(|token| !token.is_empty() && provided == token) {
        Ok(next.run(req).await)
    } else {
        // Rate-limit: 30초당 최대 1번 경고 로그 출력, 억제된 횟수 함께 표시
        let should_log = {
            let mut last = AUTH_FAIL_LAST_LOG.lock().unwrap_or_else(|e| e.into_inner());
            match *last {
                Some(t) if t.elapsed().as_secs() < 30 => false,
                _ => {
                    *last = Some(std::time::Instant::now());
                    true
                }
            }
        };
        if should_log {
            let suppressed = AUTH_FAIL_SUPPRESSED.swap(0, std::sync::atomic::Ordering::Relaxed);
            if suppressed > 0 {
                tracing::warn!(
                    "IPC auth failed for {} (suppressed {} previous)",
                    req.uri().path(), suppressed
                );
            } else {
                tracing::warn!("IPC auth failed for {}", req.uri().path());
            }
        } else {
            AUTH_FAIL_SUPPRESSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// `Host` 헤더 값이 루프백 주소(포트 포함 가능)인지 판정한다.
///
/// 허용: `127.0.0.1[:port]`, `localhost[:port]`, `[::1][:port]`.
/// 헤더가 없는 요청(HTTP/1.0 도구 등)은 브라우저 경유가 아니므로 허용한다.
pub fn is_loopback_host(host: Option<&str>) -> bool {
    let Some(host) = host.map(str::trim) else {
        return true;
    };
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        // IPv6 리터럴: [::1]:port
        match rest.split_once(']') {
            Some((addr, tail)) if tail.is_empty() || tail.starts_with(':') => addr,
            _ => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
            Some(_) => return false,
            None => host,
        }
    };
    hostname.eq_ignore_ascii_case("localhost") || hostname == "127.0.0.1" || hostname == "::1"
}

/// axum 미들웨어: 루프백이 아닌 `Host` 헤더를 403으로 거부 (DNS rebinding 방어).
/// `/health`를 포함한 모든 라우트에 적용된다.
pub async fn host_guard_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .map(|v| v.to_str().unwrap_or("\u{0}"));
    if is_loopback_host(host) {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("Rejected request with non-loopback Host header: {:?}", host);
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{middleware, routing::get, Router};
    use tower::ServiceExt;

    async fn request_status(expected: Option<&str>, provided: Option<&str>) -> StatusCode {
        let expected = expected.map(str::to_string);
        let app = Router::new()
            .route("/protected", get(|| async { StatusCode::NO_CONTENT }))
            .layer(middleware::from_fn(move |req, next| {
                authenticate_request(req, next, expected.clone())
            }));
        let mut request = Request::builder().uri("/protected");
        if let Some(token) = provided {
            request = request.header("X-Saba-Token", token);
        }
        app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    #[tokio::test]
    async fn recovery_auth_rejects_missing_or_empty_expected_token() {
        assert_eq!(request_status(None, None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(request_status(Some(""), None).await, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn loopback_host_accepts_local_forms_only() {
        for ok in [None, Some("127.0.0.1"), Some("127.0.0.1:57474"), Some("localhost:57474"),
                   Some("LOCALHOST"), Some("[::1]:57474"), Some("[::1]")] {
            assert!(is_loopback_host(ok), "{:?} must be allowed", ok);
        }
        for bad in [Some("evil.example"), Some("evil.example:57474"), Some("127.0.0.1.evil.example"),
                    Some("localhost.evil:80"), Some("[::2]:1"), Some("127.0.0.1:abc"), Some("")] {
            assert!(!is_loopback_host(bad), "{:?} must be rejected", bad);
        }
    }

    #[tokio::test]
    async fn recovery_auth_accepts_only_matching_nonempty_token() {
        assert_eq!(request_status(Some("test-token"), Some("test-token")).await, StatusCode::NO_CONTENT);
        assert_eq!(request_status(Some("test-token"), Some("wrong-token")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(request_status(Some("test-token"), None).await, StatusCode::UNAUTHORIZED);
    }
}
