//! 원격 데몬 고르기 (G1): GUI·CLI가 로컬 IPC 요청에 `X-Saba-Target: <hostId 또는 저장한 이름, 퍼센트 인코딩>`을 실으면 저장한 그 원격 데몬에 중계한다.
//!
//! 이 PC의 앱·연결에 관한 경로(`stays_local`)는 고른 데몬과 상관없이 이 데몬이 처리한다.
//! 머리가 없거나 `local`이면 그대로 이 데몬이 처리한다. 원격 포트(`server`)에는 이 층을 씌우지 않는다 (중계를 잇지 않는다)

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use saba_chan_updater_lib::config_file::ConfigFile;

use super::{config, peer, server};
use crate::ipc::IPCServer;

pub const HEADER: &str = "x-saba-target";
const BAD: &str = "bad_target";

/// 고른 데몬과 상관없이 이 PC에서 처리하는 경로 (푼 조각 목록, `gate::path_segments`). 데몬 API(`/api/…`)가 아닌 것(`/health` 포함)도 이 PC다
pub fn stays_local(segments: &[String]) -> bool {
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    // 연결 관리, 이 PC의 인터페이스, 앱 설정, 이 PC 프로그램 업데이트·로그인 자동 실행, 이 데몬 종료
    match segments.as_slice() {
        ["api", rest @ ..] => matches!(
            rest,
            [] | ["remote" | "client" | "clients" | "updates", ..] | ["config", "gui" | "client", ..] | ["system", "autostart"] | ["daemon", "shutdown"]
        ),
        _ => true,
    }
}

/// 요청이 고른 원격 데몬 (hostId 또는 이름). `Err`는 읽을 수 없는 머리의 오류 코드
pub fn remote_of(req: &Request<Body>) -> Result<Option<String>, &'static str> {
    let Some(value) = req.headers().get(HEADER) else { return Ok(None) };
    // 이름은 한국어일 수 있어 퍼센트 인코딩해 싣는다
    let value = crate::remote::gate::percent_decode(value.to_str().map_err(|_| BAD)?).ok_or(BAD)?;
    let value = value.trim();
    // 이 PC의 경로인지는 푼 경로로 본다. 풀 수 없는 경로는 중계 쪽(`forward_path`)이 거절한다
    let here = crate::remote::gate::path_segments(req.uri().path()).is_some_and(|s| stays_local(&s));
    if value.is_empty() || value == "local" || here {
        return Ok(None);
    }
    Ok(Some(value.to_string()))
}

/// 머리 값(hostId 또는 저장한 이름)에 맞는 저장한 원격 데몬. 이름이 둘 이상에 맞으면 고르지 않는다
pub fn resolve(file: &ConfigFile, wanted: &str) -> Result<String, &'static str> {
    let daemons = config::daemons_in(file);
    if daemons.iter().any(|d| d.host_id == wanted) {
        return Ok(wanted.to_string());
    }
    let mut named = daemons.iter().filter(|d| d.name == wanted);
    match (named.next(), named.next()) {
        (Some(d), None) => Ok(d.host_id.clone()),
        (Some(_), Some(_)) => Err("remote_ambiguous"),
        _ => Err("remote_unknown"),
    }
}

pub async fn layer(State(state): State<IPCServer>, req: Request<Body>, next: Next) -> Response {
    relay_with(&ConfigFile::default_location(), &state, req, next).await
}

async fn relay_with(file: &ConfigFile, state: &IPCServer, req: Request<Body>, next: Next) -> Response {
    let wanted = match remote_of(&req) {
        Ok(None) => return next.run(req).await,
        Ok(Some(w)) => w,
        Err(code) => return peer::error(StatusCode::BAD_REQUEST, code),
    };
    // 풀지 않은 경로 그대로 (원격 데몬이 다시 푼다). `..`·`.` 조각은 보내지 않는다
    let Some(path) = peer::forward_path(req.uri().path(), req.uri().query()) else {
        return peer::error(StatusCode::BAD_REQUEST, "remote_bad_path");
    };
    let host_id = match resolve(file, &wanted) {
        Ok(h) => h,
        Err(code) => return peer::error(if code == "remote_ambiguous" { StatusCode::CONFLICT } else { StatusCode::NOT_FOUND }, code),
    };
    let relay = server::RelaySource::Configured(state.clone()).target().await;
    // 이 층은 감사 출처 층(`audit::layer`) 바깥이라 출처를 여기서 정한다 (`audit::layer`와 같이). 중계한 조작을 이 PC 감사 로그에 남길 때 쓴다
    let origin = req.extensions().get::<crate::audit::Origin>().cloned().unwrap_or_else(|| crate::audit::Origin::from_headers(req.headers()));
    crate::audit::with_origin(origin, peer::forward(file, relay, &host_id, &path, req, state.shutdown_token.clone())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(path: &str, target: Option<&str>) -> Request<Body> {
        let mut r = Request::get(path);
        if let Some(t) = target {
            r = r.header(HEADER, t);
        }
        r.body(Body::empty()).unwrap()
    }

    fn local(path: &str) -> bool {
        crate::remote::gate::path_segments(path).is_some_and(|s| stays_local(&s))
    }

    #[test]
    fn this_pcs_own_app_and_connections_stay_local() {
        for path in [
            "/api/remote/daemons",
            "/api/remote/daemons/host-b/proxy/api/servers",
            "/api/client/register",
            "/api/client/abc/heartbeat",
            "/api/clients",
            "/api/clients/launch",
            "/api/config/gui",
            "/api/config/gui/defaults",
            "/api/config/client/cli",
            "/api/updates/status",
            "/api/system/autostart",
            "/api/daemon/shutdown",
            "/health",
            "/remote/pair",
            "/apix",
        ] {
            assert!(local(path), "{path}");
        }
        for path in [
            "/api/servers",
            "/api/events",
            "/api/instance/x/start",
            "/api/config/bot",
            "/api/config/bot/defaults",
            "/api/system/metrics",
            "/api/relay/agent",
            "/api/remotex",
            "/api/clientsx",
            "/api/config/guix",
            "/api/daemon/console",
            // 인코딩한 `/`는 조각 안의 글자라 `/api/remote` 아래가 아니다 (원격 데몬도 그 경로를 모른다)
            "/api/remote%2Fclients",
        ] {
            assert!(!local(path), "{path}");
        }
    }

    #[test]
    fn only_a_named_remote_daemon_on_a_forwarded_path_is_a_remote_target() {
        assert_eq!(remote_of(&req("/api/servers", Some("host-b"))), Ok(Some("host-b".into())));
        assert_eq!(remote_of(&req("/api/servers", None)), Ok(None));
        assert_eq!(remote_of(&req("/api/servers", Some(""))), Ok(None));
        assert_eq!(remote_of(&req("/api/servers", Some("local"))), Ok(None));
        assert_eq!(remote_of(&req("/api/servers", Some("%EA%B2%8C%EC%9E%84%20PC"))), Ok(Some("게임 PC".into())));
        assert_eq!(remote_of(&req("/api/servers", Some("%FF"))), Err(BAD));
        assert_eq!(remote_of(&req("/api/remote/daemons", Some("host-b"))), Ok(None));
        // 이 PC의 경로는 인코딩해도 이 PC에 남는다
        assert_eq!(remote_of(&req("/api/config/%67ui", Some("host-b"))), Ok(None));
        assert_eq!(remote_of(&req("/api/%72emote/daemons", Some("host-b"))), Ok(None));
        let bad = Request::get("/api/servers").header(HEADER, &b"\xff"[..]).body(Body::empty()).unwrap();
        assert_eq!(remote_of(&bad), Err(BAD));
    }

    #[test]
    fn a_target_is_a_saved_host_id_or_one_saved_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let saved = |host: &str, name: &str| config::Daemon {
            host_id: host.into(),
            name: name.into(),
            address: "h.example".into(),
            port: 57480,
            cert_sha256: "ab".repeat(32),
            token: "sbr_x".into(),
            paired_at: String::new(),
        };
        for d in [saved("host-b", "게임 PC"), saved("host-c", "클라우드"), saved("host-d", "클라우드")] {
            config::save_daemon_in(&file, d).unwrap();
        }
        assert_eq!(resolve(&file, "host-b"), Ok("host-b".into()));
        assert_eq!(resolve(&file, "게임 PC"), Ok("host-b".into()));
        assert_eq!(resolve(&file, "클라우드"), Err("remote_ambiguous"));
        assert_eq!(resolve(&file, "host-x"), Err("remote_unknown"));
    }

    #[tokio::test]
    async fn the_layer_sends_a_targeted_request_on_and_leaves_the_rest_here() {
        use axum::{routing::get, Router};
        use tower::ServiceExt;
        let tmp = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(tmp.path());
        let supervisor = std::sync::Arc::new(tokio::sync::RwLock::new(crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        )));
        let server = IPCServer::new(supervisor, "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let app = Router::new()
            .route("/health", get(|| async { "here" }))
            .route("/api/servers", get(|| async { "here" }))
            .route("/api/remote/daemons", get(|| async { "here" }))
            .layer(axum::middleware::from_fn(move |req: Request<Body>, next: Next| {
                let (file, server) = (file.clone(), server.clone());
                async move { relay_with(&file, &server, req, next).await }
            }));
        let call = |path: &'static str, target: Option<&'static str>| {
            let app = app.clone();
            async move {
                let res = app.oneshot(req(path, target)).await.unwrap();
                let status = res.status().as_u16();
                (status, String::from_utf8(axum::body::to_bytes(res.into_body(), 4096).await.unwrap().to_vec()).unwrap())
            }
        };
        assert_eq!(call("/api/servers", None).await, (200, "here".into()));
        assert_eq!(call("/api/remote/daemons", Some("host-b")).await, (200, "here".into()));
        assert_eq!(call("/health", Some("host-b")).await, (200, "here".into()));
        let (s, b) = call("/api/servers", Some("host-b")).await;
        assert!(s == 404 && b.contains("remote_unknown"), "{s} {b}");
        for path in ["/api/x/../../remote/pair", "/api/x/%2e%2e/%2e%2e/remote/pair"] {
            let (s, b) = call(path, Some("host-b")).await;
            assert!(s == 400 && b.contains("remote_bad_path"), "{path}: {s} {b}");
        }
    }
}
