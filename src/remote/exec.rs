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
    let blocked = {
        let segments = gate::path_segments(&req.path).unwrap_or_default();
        let supervisor = server.supervisor.read().await;
        blocked_fields(&supervisor, &req.method, pattern, &segments, req.body.as_ref()).0
    };
    if !blocked.is_empty() {
        return Response {
            status: StatusCode::FORBIDDEN.as_u16(),
            body: field_refusal(blocked),
        };
    }
    let mut origin = crate::audit::Origin::new("web");
    origin.user = user;
    let res = crate::actions::call_as(server, &req.method, &req.path, req.body, Some(origin)).await;
    let mut body = res.body;
    if gate::shows_instances(&req.method, pattern) {
        hide_secrets(server, &mut body).await;
    }
    Response {
        status: res.status.as_u16(),
        body,
    }
}

/// PC에서만 바꾸는 칸을 담은 요청의 거절 본문 (원격 포트도 같은 모양으로 거절한다)
pub(crate) fn field_refusal(fields: Vec<String>) -> Value {
    refusal(StatusCode::FORBIDDEN, "remote_field_blocked", "These settings can only be changed on the PC", Some(fields)).body
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

/// 모듈 설정 중 원격에서 읽지도 바꾸지도 않는 비밀값 칸
fn is_secret(f: &crate::supervisor::module_loader::SettingField) -> bool {
    f.field_type == "password"
}

/// 모듈 설정 중 원격에서 바꾸지 않는 칸: 비밀값과 경로(실행 파일·런타임·폴더를 고르는 칸)
fn is_pc_only(f: &crate::supervisor::module_loader::SettingField) -> bool {
    pc_only_type(&f.field_type, f.format.as_deref())
}

/// 원격에서 바꾸지 않는 칸 종류: 비밀번호, 파일·경로, 경로 형식 글 (모듈 설정과 익스텐션이 묻는 칸이 같은 규칙)
pub(crate) fn pc_only_type(field_type: &str, format: Option<&str>) -> bool {
    matches!(field_type, "password" | "file" | "path") || format == Some("path")
}

fn module_fields(
    supervisor: &crate::supervisor::Supervisor,
    module: &str,
    keep: fn(&crate::supervisor::module_loader::SettingField) -> bool,
) -> Vec<String> {
    let Ok(module) = supervisor.module_loader.get_module(module) else {
        return Vec::new();
    };
    module
        .metadata
        .settings
        .iter()
        .flat_map(|s| s.fields.iter())
        .filter(|f| keep(f))
        .map(|f| f.name.clone())
        .collect()
}

/// 서버를 만들 때 켜진 익스텐션이 물은 칸(`AddServer.options`, `options`는 그 선언)의 답(`given`)을 원격에서 받는가.
/// 모든 키가 선언한 칸이고 값이 선언한 종류에 맞아야 한다(boolean은 참거짓, number는 수나 수로 읽히는 글자(GUI·CLI는 글자로 싣는다), select는 선언한 선택지, 그 밖은 글자.
/// 객체·목록은 받지 않는다). 경로·파일·비밀번호 칸(`pc_only_type`)은 선언한 기본값 그대로일 때만 받는다 (GUI는 손대지 않은 칸도 기본값을 싣는다)
pub(crate) fn create_option_answers_allowed(options: &[Value], given: &serde_json::Map<String, Value>) -> bool {
    given.iter().all(|(key, value)| {
        let Some(option) = options.iter().find(|o| o.get("field").and_then(Value::as_str) == Some(key)) else {
            return false;
        };
        if value.is_object() || value.is_array() {
            return false;
        }
        if value.is_null() || option.get("default") == Some(value) {
            return true;
        }
        let kind = option.get("type").and_then(Value::as_str).unwrap_or("text");
        if pc_only_type(kind, option.get("format").and_then(Value::as_str)) {
            return false;
        }
        match kind {
            "boolean" => value.is_boolean(),
            "number" => value.is_number() || value.as_str().is_some_and(|n| n.trim().parse::<f64>().is_ok_and(f64::is_finite)),
            "select" => option.get("options").and_then(Value::as_array).is_some_and(|choices| {
                choices.iter().any(|c| c == value || c.get("value") == Some(value))
            }),
            _ => value.is_string(),
        }
    })
}

/// 요청이 설정을 고치는 인스턴스 (인스턴스 설정, 게임 설정 파일 쓰기). `segments`는 푼 경로 조각이다
/// (라우터는 id를 풀어서 찾으므로 여기서도 푼 id로 찾는다: %61bc로 모듈 칸 검사를 비켜 가지 않게)
fn edited_instance<'a>(
    supervisor: &'a crate::supervisor::Supervisor,
    pattern: &str,
    segments: &[String],
) -> Option<&'a crate::instance::ServerInstance> {
    match pattern {
        "/api/instance/{id}" | "/api/instance/{id}/properties" => {
            gate::param(pattern, segments, "id").and_then(|id| supervisor.instance_store.get(id))
        }
        _ => None,
    }
}

/// 이 요청 본문에서 원격이 바꾸지 못하는 칸(`gate::blocked_fields`, 그 인스턴스 모듈의 PC 전용 설정 포함)과
/// 요청이 고치는 인스턴스. 웹 원격과 원격 포트가 같이 쓴다 (원격 포트는 저장된 값과 같은 칸을 더 뺀다)
pub(crate) fn blocked_fields<'a>(
    supervisor: &'a crate::supervisor::Supervisor,
    method: &str,
    pattern: &str,
    segments: &[String],
    body: Option<&Value>,
) -> (Vec<String>, Option<&'a crate::instance::ServerInstance>) {
    let instance = edited_instance(supervisor, pattern, segments);
    let pc_only = pc_only_fields(supervisor, pattern, instance, body);
    (gate::blocked_fields(method, pattern, body, &pc_only), instance)
}

/// 이 요청이 고치는 인스턴스(`instance`, `edited_instance`) 또는 만드는 인스턴스(`body.module_name`) 모듈의 PC 전용 설정 키
fn pc_only_fields(
    supervisor: &crate::supervisor::Supervisor,
    pattern: &str,
    instance: Option<&crate::instance::ServerInstance>,
    body: Option<&Value>,
) -> Vec<String> {
    let module = match pattern {
        "/api/instances" => body.and_then(|b| b.get("module_name")).and_then(Value::as_str),
        _ => instance.map(|i| i.module_name.as_str()),
    };
    module.map(|m| module_fields(supervisor, m, is_pc_only)).unwrap_or_default()
}

/// 인스턴스를 담은 응답에서 비밀값(REST 비밀번호, 모듈의 비밀번호 칸)을 뺀다.
/// 인스턴스 객체는 `module_name`(인스턴스) 또는 `module`(서버 목록)과 `module_settings`를 가진 객체다
async fn hide_secrets(server: &IPCServer, body: &mut Value) {
    let supervisor = server.supervisor.read().await;
    let mut cache: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut stack = vec![body];
    while let Some(v) = stack.pop() {
        match v {
            Value::Array(items) => stack.extend(items.iter_mut()),
            Value::Object(map) => {
                let module = map
                    .get("module_name")
                    .or_else(|| map.get("module"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let (Some(module), true) = (module, map.contains_key("module_settings")) {
                    if let Some(p) = map.get_mut("rest_password") {
                        *p = Value::Null;
                    }
                    let secrets = cache
                        .entry(module.clone())
                        .or_insert_with(|| module_fields(&supervisor, &module, is_secret));
                    if let Some(settings) = map.get_mut("module_settings").and_then(Value::as_object_mut) {
                        settings.retain(|k, _| !secrets.contains(k));
                    }
                }
                stack.extend(map.values_mut());
            }
            _ => {}
        }
    }
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

    /// 서버를 만들 때 켜진 익스텐션이 물은 칸의 답: 선언한 종류에 맞는 값만, 경로·파일·비밀번호 칸은 선언한 기본값일 때만
    #[test]
    fn create_option_answers_match_what_the_extension_declared() {
        let options = vec![
            json!({ "field": "boxed", "type": "boolean" }),
            json!({ "field": "slots", "type": "number" }),
            json!({ "field": "tier", "type": "select", "options": ["small", "large"] }),
            json!({ "field": "label", "type": "text" }),
            json!({ "field": "box_dir", "type": "path", "default": "boxes" }),
            json!({ "field": "key", "type": "password" }),
        ];
        let ok = |v: Value| create_option_answers_allowed(&options, v.as_object().unwrap());
        assert!(ok(json!({ "boxed": true, "slots": 3, "tier": "large", "label": "hi", "box_dir": "boxes" })));
        // GUI·CLI는 숫자 칸 답을 글자로 싣는다
        assert!(ok(json!({ "slots": "3" })));
        assert!(ok(json!({ "slots": "2.5" })));
        for bad in [
            json!({ "boxed": "yes" }),
            json!({ "slots": "three" }),
            json!({ "slots": "" }),
            json!({ "tier": "huge" }),
            json!({ "label": 1 }),
            json!({ "label": ["a"] }),
            json!({ "boxed": { "x": 1 } }),
            json!({ "box_dir": "/etc" }),
            json!({ "key": "x" }),
            json!({ "other": 1 }),
        ] {
            assert!(!ok(bad.clone()), "{bad}");
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

    /// 비밀번호 칸 하나, 경로 형식 글 칸 하나, 보통 칸 하나를 가진 모듈과 그 인스턴스
    async fn secret_server(tmp: &std::path::Path) -> (IPCServer, String) {
        let dir = tmp.join("modules").join("secret");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = "[module]\nschema_version = 1\nname = \"secret\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"rcon_password\"\ntype = \"password\"\nlabel = \"rcon\"\n\n[[settings.fields]]\nname = \"world_dir\"\ntype = \"text\"\nformat = \"path\"\nlabel = \"world\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\n";
        std::fs::write(dir.join("module.toml"), toml).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let server = server(tmp).await;
        let mut inst = crate::instance::ServerInstance::new("s1", "secret");
        inst.rest_password = Some("rest-secret".into());
        inst.module_settings
            .insert("rcon_password".into(), json!("rcon-secret"));
        inst.module_settings.insert("motd".into(), json!("hi"));
        let id = inst.id.clone();
        server.supervisor.write().await.instance_store.add(inst).unwrap();
        (server, id)
    }

    #[tokio::test]
    async fn instance_reads_hide_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let (server, id) = secret_server(tmp.path()).await;
        for path in [
            format!("/api/instance/{id}"),
            "/api/instances".into(),
            "/api/servers".into(),
        ] {
            let res = run(&server, req("GET", &path, None), None).await;
            assert_eq!(res.status, 200, "{path}: {:?}", res.body);
            let text = res.body.to_string();
            assert!(
                !text.contains("rest-secret") && !text.contains("rcon-secret"),
                "{path}: {text}"
            );
            assert!(text.contains("\"motd\":\"hi\""), "{path}: {text}");
        }
    }

    #[tokio::test]
    async fn secret_and_path_format_settings_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (server, id) = secret_server(tmp.path()).await;
        let body = json!({ "rcon_password": "x", "world_dir": "C:/", "rest_password": "y", "motd": "ok" });
        let res = run(&server, req("PATCH", &format!("/api/instance/{id}"), Some(body)), None).await;
        assert_eq!(res.status, 403, "{:?}", res.body);
        assert_eq!(
            res.body["fields"],
            json!(["rcon_password", "rest_password", "world_dir"])
        );
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
