use super::loopback;
use crate::{ipc::CredentialsCache, supervisor::Supervisor};
use axum::response::IntoResponse;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};
use tokio::sync::RwLock;

pub const PREFIX: &str = "HOST:";

#[derive(Clone)]
pub struct Runtime {
    supervisor: Weak<RwLock<Supervisor>>,
    credentials: CredentialsCache,
}

impl Runtime {
    pub fn new(supervisor: &Arc<RwLock<Supervisor>>, credentials: CredentialsCache) -> Self {
        Self {
            supervisor: Arc::downgrade(supervisor),
            credentials,
        }
    }

    pub fn for_instance(&self, id: &str) -> Context {
        Context {
            runtime: self.clone(),
            id: id.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Context {
    runtime: Runtime,
    id: String,
}

#[derive(Clone)]
pub struct Proxy {
    pub dir: PathBuf,
    pub context: Option<Context>,
    /// 이 호출이 가리키는 인스턴스의 서버 PID (REST 대상 포트가 그 서버 것인지 본다)
    pub server: Option<u32>,
    /// 격리된(확인되지 않은) 실행이면 그 진입 파일. 그 인스턴스 모듈의 진입 파일일 때 `instance.rest`만 잇는다:
    /// 콘솔 명령과 익스텐션 hook의 요청은 받지 않는다
    pub sandboxed: Option<PathBuf>,
}

fn failure(code: &str, error: impl std::fmt::Display) -> Value {
    json!({"ok":false,"error_code":code,"error":error.to_string()})
}

fn request_id(request: &Value) -> Option<&str> {
    request.get("id").and_then(Value::as_str).filter(|id| {
        !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

pub fn serve<'a>(
    proxy: &'a Proxy,
    line: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            tracing::warn!("[Host] unreadable request");
            return;
        };
        let Some(id) = request_id(&request) else {
            tracing::warn!("[Host] request without a usable id");
            return;
        };
        let reply = if line.len() > loopback::MAX_REQUEST {
            failure("request_too_large", "Host request exceeded 64 KiB")
        } else {
            match tokio::time::timeout(loopback::MAX_TIMEOUT, dispatch(proxy, &request)).await {
                Ok(reply) => reply,
                Err(_) => failure("request_timeout", "Host request exceeded 30 seconds"),
            }
        };
        write_reply(proxy, id, reply);
    })
}

pub fn refuse(proxy: &Proxy, line: &str, code: &str) {
    if let Ok(request) = serde_json::from_str::<Value>(line) {
        if let Some(id) = request_id(&request) {
            write_reply(proxy, id, failure(code, code));
        }
    }
}

fn write_reply(proxy: &Proxy, id: &str, mut reply: Value) {
    if serde_json::to_vec(&reply).map_or(true, |bytes| bytes.len() > loopback::MAX_RESPONSE) {
        reply = failure("response_too_large", "Host response exceeded 1 MiB");
    }
    if let Err(error) = loopback::write_reply(&proxy.dir, id, &reply) {
        tracing::warn!("[Host] could not write response: {}", error);
    }
}

async fn dispatch(proxy: &Proxy, request: &Value) -> Value {
    let op = request.get("op").and_then(Value::as_str).unwrap_or("");
    if !matches!(op, "instance.stdin" | "instance.rest" | "instance.rcon") {
        tracing::warn!("[Host] operation is not allowed");
        return failure("op_not_allowed", "Host operation is not allowed");
    }
    let Some(context) = &proxy.context else {
        return failure("no_instance", "This module call has no instance context");
    };
    let Some(supervisor) = context.runtime.supervisor.upgrade() else {
        return failure("no_instance", "The instance is no longer available");
    };
    if supervisor.read().await.instance_store.get(&context.id).is_none() {
        return failure("no_instance", "The instance is no longer available");
    }
    if let Some(component) = &proxy.sandboxed {
        if op != "instance.rest" || !own_module(&supervisor, &context.id, component).await {
            tracing::warn!("[Host] sandboxed {} refused: only the instance's own module may use REST", component.display());
            return failure("op_not_allowed", "Host operation is not allowed");
        }
    }
    let Some(args) = request.get("args").and_then(Value::as_object) else {
        return failure("invalid_request", "Host args must be an object");
    };
    let allowed: &[&str] = if matches!(op, "instance.stdin" | "instance.rcon") {
        &["command"]
    } else {
        &["method", "path", "body"]
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return failure("invalid_request", "Host args contain an unsupported field");
    }
    // RCON만 받는 서버의 한 줄 (모듈 `stop()`의 정지 명령). 콘솔이 있으면 데몬이 stdin으로 보낸다
    if op == "instance.rcon" {
        return response(crate::ipc::handlers::command::rcon_for(&supervisor, &context.id, &Value::Object(args.clone())).await).await;
    }
    if op == "instance.stdin" {
        return response(
            crate::ipc::handlers::command::stdin_for(&supervisor, &context.id, &Value::Object(args.clone())).await,
        )
        .await;
    }
    let Some(path) = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| p.starts_with('/') && !p.starts_with("//") && !p.contains(['\r', '\n']))
    else {
        return failure("invalid_request", "REST path must be an endpoint");
    };
    let Some(method) = args
        .get("method")
        .and_then(Value::as_str)
        .filter(|m| matches!(m.to_ascii_uppercase().as_str(), "GET" | "POST" | "PUT" | "DELETE"))
    else {
        return failure("invalid_request", "REST method is not supported");
    };
    // 모듈 설정으로 정해진 REST 주소가 그 인스턴스 서버의 것일 때만 잇는다 (LOOPBACK과 같은 규칙):
    // 아니면 확인되지 않은 모듈이 rest_port로 같은 PC의 다른 서비스에 데몬 권한으로 요청할 수 있다
    let (rest_host, rest_port, tracked) = {
        let sup = supervisor.read().await;
        let Some(instance) = sup.instance_store.get(&context.id) else {
            return failure("no_instance", "The instance is no longer available");
        };
        let module = sup.module_loader.get_module(&instance.module_name).ok();
        (
            instance.rest_host.clone().or_else(|| module.as_ref().map(|m| m.metadata.default_rest_host())).unwrap_or_else(|| "127.0.0.1".to_string()),
            instance.rest_port.or_else(|| module.as_ref().map(|m| m.metadata.default_rest_port())).unwrap_or(8212),
            sup.tracker.get_pid(&context.id).ok(),
        )
    };
    if rest_port == super::ipc_port() {
        tracing::warn!("[Host] refused REST to the daemon port {}", rest_port);
        return failure("port_not_allowed", "The REST port is the daemon's own port");
    }
    let local = matches!(rest_host.trim(), "127.0.0.1" | "localhost" | "::1" | "[::1]");
    // 서버를 가리키는 호출(stop·status 등)은 그 PID를, 익스텐션 명령·hook처럼 PID 없이 온 호출은 데몬이 추적하는 서버 PID를 쓴다
    let server = proxy.server.or(tracked);
    if !local || !tokio::task::spawn_blocking(move || loopback::served_by(rest_port, server)).await.unwrap_or(false) {
        tracing::warn!("[Host] refused REST to {}:{}: not the instance's server (PID {:?})", rest_host, rest_port, server);
        return failure("port_not_served", "The REST port is not held by this instance's server");
    }
    let mut payload = json!({"endpoint":path,"method":method});
    if let Some(body) = args.get("body") {
        payload["body"] = body.clone();
    }
    response(
        crate::ipc::handlers::command::rest_for(
            &supervisor,
            &context.runtime.credentials,
            &context.id,
            payload,
            Some(loopback::MAX_RESPONSE),
        )
        .await,
    )
    .await
}

/// 격리 실행의 진입 파일이 그 인스턴스 모듈의 진입 파일인가 (익스텐션·다른 모듈이면 false).
/// 폴더가 아니라 파일로 본다: zip 모듈은 `.extracted/<이름>`에서 돌아 구성요소 폴더로는 가릴 수 없다
async fn own_module(supervisor: &RwLock<Supervisor>, id: &str, component: &std::path::Path) -> bool {
    let module = {
        let sup = supervisor.read().await;
        let Some(instance) = sup.instance_store.get(id) else { return false };
        match sup.module_loader.get_module(&instance.module_name) {
            Ok(module) => PathBuf::from(module.entry_path()),
            Err(_) => return false,
        }
    };
    let component = component.to_path_buf();
    tokio::task::spawn_blocking(move || match (std::fs::canonicalize(module), std::fs::canonicalize(component)) {
        (Ok(module), Ok(component)) => module == component,
        _ => false,
    })
    .await
    .unwrap_or(false)
}

async fn response(value: impl IntoResponse) -> Value {
    let response = value.into_response();
    let status = response.status();
    let Ok(bytes) = axum::body::to_bytes(response.into_body(), loopback::MAX_RESPONSE).await else {
        return failure("response_too_large", "Host response exceeded 1 MiB");
    };
    let Ok(data) = serde_json::from_slice::<Value>(&bytes) else {
        return failure("invalid_response", "Daemon returned an invalid response");
    };
    if status.is_success() {
        json!({"ok":true,"data":data})
    } else {
        failure(
            data.get("error_code")
                .and_then(Value::as_str)
                .unwrap_or("request_failed"),
            data.get("error")
                .or_else(|| data.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Host operation failed"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn host_rejects_missing_context_unknown_operations_and_oversized_requests() {
        let dir = tempfile::tempdir().unwrap();
        let proxy = Proxy {
            dir: dir.path().into(),
            context: None,
            server: None,
            sandboxed: None,
        };
        for (id, op, args, code) in [
            ("missing", "instance.stdin", json!({"command":"say hi"}), "no_instance"),
            ("missing-rcon", "instance.rcon", json!({"command":"stop"}), "no_instance"),
            ("denied", "instance.delete", json!({}), "op_not_allowed"),
            (
                "large",
                "instance.stdin",
                json!({"command":"x".repeat(super::super::loopback::MAX_REQUEST)}),
                "request_too_large",
            ),
        ] {
            serve(&proxy, &json!({"id":id,"op":op,"args":args}).to_string()).await;
            let reply: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.path().join(format!("{id}.json"))).unwrap()).unwrap();
            assert_eq!(reply["ok"], false);
            assert_eq!(reply["error_code"], code);
        }
        serve(&proxy, r#"{"id":"../escape","op":"instance.stdin","args":{}}"#).await;
        assert!(!dir.path().parent().unwrap().join("escape.json").exists());
    }

    /// Codex 리뷰 #550 P1: HOST REST가 인스턴스만 보고 잇던 탓에, 꺼진 서버의 rest_port가 같은 PC의 다른 서비스를 가리키면
    /// 데몬 권한으로 그 서비스에 요청할 수 있었다. 그 포트를 인스턴스 서버가 들을 때만 잇는다
    #[tokio::test]
    async fn host_rest_refuses_a_port_the_instance_server_does_not_hold() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let mut instance = crate::instance::ServerInstance::new("host", "host-probe");
        instance.rest_port = Some(other.local_addr().unwrap().port());
        let id = instance.id.clone();
        supervisor.instance_store.add(instance).unwrap();
        let supervisor = Arc::new(RwLock::new(supervisor));
        let runtime = Runtime::new(&supervisor, CredentialsCache::default());
        let dir = tempfile::tempdir().unwrap();
        // 서버가 꺼져 있다(PID 없음): 다른 프로그램이 그 포트를 들어도 잇지 않는다
        let proxy = Proxy { dir: dir.path().into(), context: Some(runtime.for_instance(&id)), server: None, sandboxed: None };
        serve(&proxy, &json!({"id":"r1","op":"instance.rest","args":{"method":"GET","path":"/x"}}).to_string()).await;
        let reply: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("r1.json")).unwrap()).unwrap();
        assert_eq!(reply["error_code"], "port_not_served", "{reply}");
        // Codex 재검증 #550: PID 없이 온 호출(익스텐션 명령)도 데몬이 추적하는 서버가 그 포트를 들으면 잇는다
        supervisor.read().await.tracker.track(&id, std::process::id()).unwrap();
        serve(&proxy, &json!({"id":"r2","op":"instance.rest","args":{"method":"GET","path":"/x"}}).to_string()).await;
        let reply: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("r2.json")).unwrap()).unwrap();
        assert_ne!(reply["error_code"], "port_not_served", "{reply}");
        drop(other);
    }

    async fn probe_supervisor(rest_port: u16) -> (tempfile::TempDir, Arc<RwLock<Supervisor>>, String, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let module = tmp.path().join("modules").join("host-probe");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("module.toml"), "[module]\nschema_version=1\nname='host-probe'\nversion='1.0.0'\nentry='lifecycle.py'\n[protocols]\nsupported=['rest']\n").unwrap();
        std::fs::write(module.join("lifecycle.py"), "").unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let mut instance = crate::instance::ServerInstance::new("host", "host-probe");
        instance.rest_port = Some(rest_port);
        let id = instance.id.clone();
        supervisor.instance_store.add(instance).unwrap();
        supervisor.tracker.track(&id, std::process::id()).unwrap();
        (tmp, Arc::new(RwLock::new(supervisor)), id, module.join("lifecycle.py"))
    }

    fn ask(dir: &std::path::Path, id: &str) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap()
    }

    /// 격리된(확인되지 않은) 모듈도 자기 인스턴스 서버의 REST로 저장·정지를 요청할 수 있다 (REST 게임의 정상 정지).
    /// 콘솔 명령과 다른 구성요소(익스텐션 hook)의 요청은 지금처럼 받지 않는다
    /// (포트를 듣는 프로세스는 Windows·Linux에서만 읽는다)
    #[cfg(any(windows, target_os = "linux"))]
    #[tokio::test]
    async fn a_sandboxed_module_reaches_only_its_own_instance_rest() {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (_tmp, supervisor, id, module) = probe_supervisor(server.local_addr().unwrap().port()).await;
        let runtime = Runtime::new(&supervisor, CredentialsCache::default());
        let dir = tempfile::tempdir().unwrap();
        let rest = json!({"method":"POST","path":"/v1/api/shutdown","body":{}});
        let own = Proxy { dir: dir.path().into(), context: Some(runtime.for_instance(&id)), server: None, sandboxed: Some(module.clone()) };
        serve(&own, &json!({"id":"rest","op":"instance.rest","args":rest}).to_string()).await;
        let reply = ask(dir.path(), "rest");
        assert!(!matches!(reply["error_code"].as_str(), Some("op_not_allowed" | "no_instance" | "port_not_served")), "{reply}");
        serve(&own, &json!({"id":"stdin","op":"instance.stdin","args":{"command":"op someone"}}).to_string()).await;
        assert_eq!(ask(dir.path(), "stdin")["error_code"], "op_not_allowed");
        serve(&own, &json!({"id":"rcon","op":"instance.rcon","args":{"command":"op someone"}}).to_string()).await;
        assert_eq!(ask(dir.path(), "rcon")["error_code"], "op_not_allowed");
        let extension = tempfile::tempdir().unwrap();
        std::fs::write(extension.path().join("hooks.py"), "").unwrap();
        let other = Proxy { sandboxed: Some(extension.path().join("hooks.py")), ..own };
        serve(&other, &json!({"id":"ext","op":"instance.rest","args":rest}).to_string()).await;
        assert_eq!(ask(dir.path(), "ext")["error_code"], "op_not_allowed");
        drop(server);
    }

    /// 인스턴스 REST 포트가 데몬 API 포트면 그 포트를 듣는 프로세스와 상관없이 잇지 않는다
    #[tokio::test]
    async fn host_rest_never_targets_the_daemon_port() {
        let (_tmp, supervisor, id, _module) = probe_supervisor(super::super::ipc_port()).await;
        let runtime = Runtime::new(&supervisor, CredentialsCache::default());
        let dir = tempfile::tempdir().unwrap();
        let proxy = Proxy { dir: dir.path().into(), context: Some(runtime.for_instance(&id)), server: None, sandboxed: None };
        serve(&proxy, &json!({"id":"d","op":"instance.rest","args":{"method":"GET","path":"/api/instances"}}).to_string()).await;
        assert_eq!(ask(dir.path(), "d")["error_code"], "port_not_allowed");
    }

    /// RCON 한 줄은 그 호출의 인스턴스로 `/api/instance/:id/rcon`과 같은 길을 탄다 (콘솔도 RCON도 없으면 no_command_path)
    #[tokio::test]
    async fn host_rcon_uses_the_instance_command_path() {
        let (_tmp, supervisor, id, _module) = probe_supervisor(1).await;
        let runtime = Runtime::new(&supervisor, CredentialsCache::default());
        let dir = tempfile::tempdir().unwrap();
        let proxy = Proxy { dir: dir.path().into(), context: Some(runtime.for_instance(&id)), server: None, sandboxed: None };
        serve(&proxy, &json!({"id":"r","op":"instance.rcon","args":{"command":"stop"}}).to_string()).await;
        assert_eq!(ask(dir.path(), "r")["error_code"], "no_command_path");
        serve(&proxy, &json!({"id":"x","op":"instance.rcon","args":{"command":"stop","port":1}}).to_string()).await;
        assert_eq!(ask(dir.path(), "x")["error_code"], "invalid_request");
    }

    #[tokio::test]
    async fn host_never_returns_a_truncated_success() {
        let reply = response(axum::Json(
            json!({"data":"x".repeat(super::super::loopback::MAX_RESPONSE)}),
        ))
        .await;
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error_code"], "response_too_large");
    }

    #[tokio::test]
    async fn host_rest_uses_the_call_instance_and_reenters_credentials_when_all_slots_are_held() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let module = modules.join("host-probe");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("module.toml"), "[module]\nschema_version=1\nname='host-probe'\nversion='1.0.0'\nentry='lifecycle.py'\n[protocols]\nsupported=['rest']\n").unwrap();
        let entry = module.join("lifecycle.py");
        std::fs::write(
            &entry,
            r#"import json, os, sys
from daemon_rcon import daemon_rest
sys.stdin.read()
if sys.argv[1] == 'get_credentials':
    result = {'success': True, 'rest_username': 'tester', 'rest_password': 'fixture-only'}
else:
    reply = daemon_rest('forged-instance', 'PUT', '/host-probe', body={'message': 'from-module'}, timeout=20)
    result = {'success': True, 'reply': reply, 'host_dir': os.environ['SABA_HOST_DIR']}
print(json.dumps(result))
"#,
        )
        .unwrap();
        crate::sandbox::consent_test_component(&entry);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                if String::from_utf8_lossy(&request).contains("from-module") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        supervisor.initialize().await.unwrap();
        let mut instance = crate::instance::ServerInstance::new("host", "host-probe");
        instance.rest_port = Some(port);
        let id = instance.id.clone();
        supervisor.instance_store.add(instance).unwrap();
        // REST 포트는 그 인스턴스 서버가 들어야 잇는다: 이 테스트 프로세스가 서버이고 포트를 듣는다
        supervisor.tracker.track(&id, std::process::id()).unwrap();
        let state = crate::ipc::IPCServer::new(
            Arc::new(RwLock::new(supervisor)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        let lanes = super::super::call_lanes();
        let _all = lanes
            .general
            .acquire_many(super::super::GENERAL_SLOTS as u32)
            .await
            .unwrap();
        let call = Supervisor::query_instance(&state.supervisor, &id, "host_probe", Default::default());
        let result = super::super::CALL_DEADLINE
            .scope(super::super::CallDeadline::new(tokio::time::Instant::now() + std::time::Duration::from_secs(45)), call)
            .await
            .unwrap();
        assert_eq!(result["reply"]["success"], true, "{result}");
        assert_eq!(result["reply"]["method"], "PUT");
        assert!(state.credentials.get(&id).is_some(), "nested credentials completed");
        assert!(
            !std::path::Path::new(result["host_dir"].as_str().unwrap()).exists(),
            "call channel is cleaned up"
        );
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert!(request.starts_with("PUT /host-probe HTTP/1.1"), "{request}");
        assert!(request.contains("from-module"));
    }
}
