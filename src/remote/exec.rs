//! 원격 요청 한 건 실행: 문지기(`gate`)를 지난 것만 데몬 라우터를 안에서 부른다

use super::gate::{self, Refusal};
use crate::ipc::IPCServer;
use axum::http::StatusCode;
use serde_json::{json, Value};

/// 브라우저가 보낸 요청 (`{method, path, body}`)
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
}

/// GUI 브리지가 받는 것과 같은 모양 (`{status, data}`)
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Value,
}

/// `user`는 릴레이가 링크를 내준 디스코드 사용자 (감사 기록용)
pub async fn run(server: &IPCServer, req: Request, user: Option<String>) -> Response {
    let pattern = match gate::decide(&req.method, &req.path) {
        Ok(p) => p,
        Err(Refusal::BadPath) => {
            return refusal(
                StatusCode::BAD_REQUEST,
                "remote_bad_path",
                "This path is not accepted",
                None,
            )
        }
        Err(Refusal::NotAllowed) => {
            return refusal(
                StatusCode::FORBIDDEN,
                "remote_not_allowed",
                "This can only be done on the PC",
                None,
            )
        }
    };
    let file_fields = file_fields(server, pattern, &req).await;
    let blocked = gate::blocked_fields(&req.method, pattern, req.body.as_ref(), &file_fields);
    if !blocked.is_empty() {
        let msg = "These settings can only be changed on the PC";
        return refusal(StatusCode::FORBIDDEN, "remote_field_blocked", msg, Some(blocked));
    }
    let mut origin = crate::audit::Origin::new("web");
    origin.user = user;
    let res = crate::actions::call_as(server, &req.method, &req.path, req.body, Some(origin)).await;
    Response {
        status: res.status.as_u16(),
        body: res.body,
    }
}

fn refusal(status: StatusCode, code: &str, error: &str, fields: Option<Vec<String>>) -> Response {
    let mut body = json!({ "ok": false, "success": false, "error": error, "error_code": code });
    if let Some(fields) = fields {
        body["fields"] = json!(fields);
    }
    Response {
        status: status.as_u16(),
        body,
    }
}

/// 이 요청이 고치는 인스턴스 모듈의 경로형 설정 키 (실행 파일·런타임 경로를 고르는 칸)
async fn file_fields(server: &IPCServer, pattern: &str, req: &Request) -> Vec<String> {
    let supervisor = server.supervisor.read().await;
    let module = match pattern {
        "/api/instance/:id" => req
            .path
            .split('?')
            .next()
            .and_then(|p| p.split('/').nth(3))
            .and_then(|id| supervisor.instance_store.get(id))
            .map(|i| i.module_name.clone()),
        "/api/instances" => req
            .body
            .as_ref()
            .and_then(|b| b.get("module_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    };
    let Some(module) = module.and_then(|m| supervisor.module_loader.get_module(&m).ok()) else {
        return Vec::new();
    };
    module
        .metadata
        .settings
        .iter()
        .flat_map(|s| s.fields.iter())
        .filter(|f| matches!(f.field_type.as_str(), "file" | "path"))
        .map(|f| f.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn server(tmp: &std::path::Path) -> IPCServer {
        crate::ipc::auth::ensure_test_token();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.join("modules").to_str().unwrap(),
            tmp.join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        )
    }

    fn req(method: &str, path: &str, body: Option<Value>) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            body,
        }
    }

    #[tokio::test]
    async fn allowed_request_reaches_the_router() {
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let res = run(&server, req("GET", "/api/instances", None), Some("u1".into())).await;
        assert_eq!(res.status, 200, "{:?}", res.body);
        assert!(res.body.is_array(), "{:?}", res.body);
    }

    #[tokio::test]
    async fn pc_only_route_is_refused_before_the_router() {
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let res = run(&server, req("GET", "/api/config/node-token", None), None).await;
        assert_eq!(res.status, 403);
        assert_eq!(res.body["error_code"], "remote_not_allowed");
        assert!(res.body.get("token").is_none());
    }

    #[tokio::test]
    async fn odd_path_is_a_bad_request() {
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let res = run(&server, req("POST", "/api/instance/..%2F/start", None), None).await;
        assert_eq!(res.status, 400);
        assert_eq!(res.body["error_code"], "remote_bad_path");
    }

    #[tokio::test]
    async fn module_path_settings_are_refused_on_create() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("plain");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\nlabel = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\n";
        std::fs::write(dir.join("module.toml"), toml).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let server = server(tmp.path()).await;
        let body = json!({ "name": "s1", "module_name": "plain", "module_settings": { "java_path": "C:/evil.exe", "motd": "hi" } });
        let res = run(&server, req("POST", "/api/instances", Some(body)), None).await;
        assert_eq!(res.status, 403, "{:?}", res.body);
        assert_eq!(res.body["fields"], json!(["module_settings.java_path"]));
    }

    #[tokio::test]
    async fn launch_fields_are_refused_with_their_names() {
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let body = json!({ "executable_path": "C:/Windows/System32/cmd.exe", "port": 1 });
        let res = run(&server, req("PATCH", "/api/instance/abc", Some(body)), None).await;
        assert_eq!(res.status, 403);
        assert_eq!(res.body["error_code"], "remote_field_blocked");
        assert_eq!(res.body["fields"], json!(["executable_path"]));
    }
}
