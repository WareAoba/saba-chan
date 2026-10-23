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
    if !matches!(op, "instance.stdin" | "instance.rest") {
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
    let Some(args) = request.get("args").and_then(Value::as_object) else {
        return failure("invalid_request", "Host args must be an object");
    };
    let allowed: &[&str] = if op == "instance.stdin" {
        &["command"]
    } else {
        &["method", "path", "body"]
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return failure("invalid_request", "Host args contain an unsupported field");
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
        };
        for (id, op, args, code) in [
            ("missing", "instance.stdin", json!({"command":"say hi"}), "no_instance"),
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
