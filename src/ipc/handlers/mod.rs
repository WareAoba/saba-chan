pub mod server;
pub mod instance;
pub mod command;
pub mod managed;
pub mod client;
pub mod bot;
pub mod config;
pub mod extension;
pub mod relay;
pub mod chat;
pub mod backup;
pub mod chatbot;
pub mod discord;
pub mod migration;
pub mod diagnostics;
pub mod system;
pub mod permissions;
pub mod actions;

/// 감사 로그 (A10): 응답은 그대로 돌려주고, 출처(`crate::audit::current`)와 함께 한 건 남긴다.
/// `key`는 인스턴스 id나 이름. 성공은 2xx이면서 본문의 `success`가 false가 아닐 때
pub(crate) async fn audited(
    state: &super::IPCServer,
    action: &str,
    key: &str,
    command: Option<&str>,
    args: Option<&serde_json::Value>,
    resp: axum::response::Response,
) -> axum::response::Response {
    if crate::audit::is_quiet() {
        return resp;
    }
    let (id, name, command, args) = {
        let sup = state.supervisor.read().await;
        let instance = sup.instance_store.get(key).or_else(|| sup.instance_store.list().iter().find(|i| i.name == key));
        let module = instance.and_then(|i| sup.module_loader.get_module(&i.module_name).ok()).map(|m| m.metadata);
        // 비밀 인자·설정값은 가려서 남긴다
        let (command, args) = crate::audit::redact(module.as_ref(), instance, command, args);
        match instance {
            Some(i) => (i.id.clone(), Some(i.name.clone()), command, args),
            None => (key.to_string(), None, command, args),
        }
    };
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
    let declined = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("success").and_then(serde_json::Value::as_bool))
        == Some(false);
    crate::audit::record(crate::audit::Event {
        action,
        instance_id: &id,
        instance: name.as_deref(),
        command: command.as_deref(),
        args: args.as_ref(),
        ok: parts.status.is_success() && !declined,
        status: parts.status.as_u16(),
    });
    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}
