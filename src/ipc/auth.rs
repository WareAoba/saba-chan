//! IPC 토큰 기반 인증 미들웨어
//!
//! 데몬 시작 시 랜덤 토큰을 생성하여 파일에 저장하고 메모리에 캐시합니다.
//! GUI, CLI, Bot은 이 파일을 읽어서 `X-Saba-Token` 헤더에 포함시킵니다.
//! 토큰이 일치하지 않는 요청은 401 Unauthorized로 거부됩니다.

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
            tracing::info!(
                "IPC auth token regenerated and cached (token: {}…, previous overwritten)",
                &token[..8]
            );
        } else {
            tracing::info!("IPC auth token generated and cached (token: {}…)", &token[..8]);
        }
    }

    tracing::info!("IPC auth token saved to {}", path);
    Ok(Arc::new(token))
}

/// 토큰 파일에서 읽기 (클라이언트 측에서 사용)
pub fn read_token_from_file() -> Option<String> {
    let path = token_file_path();
    std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string())
}

/// 인증 실패 로그 스팸을 억제하기 위한 rate-limiter
static AUTH_FAIL_LAST_LOG: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static AUTH_FAIL_SUPPRESSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `Host` 헤더가 루프백(127.0.0.1, localhost, ::1)을 가리키는지. 헤더가 없으면(HTTP/1.0 도구) 허용
fn host_is_loopback(req: &Request<Body>) -> bool {
    let Some(host) = req.headers().get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
}

/// axum 미들웨어: `X-Saba-Token` 헤더 검증
///
/// 개발 빌드에서 `SABA_AUTH_DISABLED=1`이면 토큰 검사를 건너뛴다 (Host 검사는 늘 한다).
pub async fn auth_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // 브라우저 페이지가 DNS 리바인딩으로 들어오는 것을 막는다: 데몬은 루프백에만 묶인다
    if !host_is_loopback(&req) {
        return Err(StatusCode::FORBIDDEN);
    }

    // 개발·테스트 빌드에서만 환경 변수로 인증을 끌 수 있다
    if cfg!(debug_assertions) && std::env::var("SABA_AUTH_DISABLED").unwrap_or_default() == "1" {
        return Ok(next.run(req).await);
    }

    // 메모리 캐시된 토큰 사용 (파일 I/O 제거)
    // generate_and_save_token()에서 설정한 캐시를 우선 사용하고,
    // 캐시가 없으면(외부 데몬 등) 파일에서 읽기
    let expected = {
        let cached = CACHED_TOKEN.read().unwrap_or_else(|e| e.into_inner());
        cached.clone()
    };
    let expected = match expected {
        Some(t) => t,
        None => match read_token_from_file() {
            Some(t) => t,
            None => {
                // 토큰이 없으면 아무도 통과시키지 않는다 (데몬은 기동할 때 늘 만든다)
                tracing::warn!("No IPC token at {} — rejecting request", token_file_path());
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    };

    // 헤더에서 토큰 추출    
    let provided = req
        .headers()
        .get("X-Saba-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !expected.is_empty() && provided == expected {
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
            let expected_hint = if expected.len() >= 8 { &expected[..8] } else { &expected };
            let provided_hint = if provided.is_empty() {
                "(empty)"
            } else if provided.len() >= 8 {
                &provided[..8]
            } else {
                provided
            };
            if suppressed > 0 {
                tracing::warn!(
                    "IPC auth failed from {}: expected={}… got={}… (suppressed {} previous)",
                    req.uri(), expected_hint, provided_hint, suppressed
                );
            } else {
                tracing::warn!(
                    "IPC auth failed from {}: expected={}… got={}…",
                    req.uri(), expected_hint, provided_hint
                );
            }
        } else {
            AUTH_FAIL_SUPPRESSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_host(host: &str) -> Request<Body> {
        Request::builder().uri("/api/servers").header("host", host).body(Body::empty()).unwrap()
    }

    #[test]
    fn only_loopback_hosts_pass() {
        for ok in ["127.0.0.1:57474", "localhost:57474", "LOCALHOST", "[::1]:57474", "127.0.0.1"] {
            assert!(host_is_loopback(&with_host(ok)), "{ok}");
        }
        for bad in ["evil.example:57474", "127.0.0.1.evil.example", "192.168.0.2:57474", "[::2]:1"] {
            assert!(!host_is_loopback(&with_host(bad)), "{bad}");
        }
    }
}
