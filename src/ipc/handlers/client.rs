use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{ClientKind, ClientRegistry, IPCServer};

/// POST /api/daemon/shutdown — 데몬 강제 종료 (CLI 등 외부에서 호출)
pub async fn daemon_shutdown(
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    tracing::info!("[Shutdown] Daemon shutdown requested via API");
    state.request_shutdown();
    (StatusCode::OK, Json(json!({"ok": true, "message": "Shutdown initiated"}))).into_response()
}

/// POST /api/client/register — 클라이언트(GUI/CLI) 등록
pub async fn client_register(
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let kind_str = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("gui");
    let kind = match kind_str {
        "cli" => ClientKind::Cli,
        _ => ClientKind::Gui,
    };

    let pid = payload.get("pid").and_then(|v| v.as_u64()).and_then(|p| u32::try_from(p).ok());
    let install_root = payload
        .get("installRoot")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let client_id = state.client_registry.register(kind.clone(), pid, install_root).await;
    state.client_registry.set_resident(&client_id, resident_flag(&payload)).await;
    let count = state.client_registry.count().await;
    tracing::info!("[Heartbeat] Active clients: {}", count);

    (
        StatusCode::OK,
        Json(json!({
            "client_id": client_id,
            "kind": kind_str,
            "heartbeat_interval_ms": 30000,
            "timeout_ms": 90000
        })),
    )
        .into_response()
}

/// GET /api/clients — 지금 등록된 클라이언트. 인스톨러가 이 설치의 GUI(임시 폴더에 풀린 포터블 GUI 포함)를 찾는다
pub async fn client_list(State(state): State<IPCServer>) -> impl IntoResponse {
    let clients: Vec<_> = state
        .client_registry
        .list()
        .await
        .into_iter()
        .map(|(id, c)| json!({"id": id, "kind": c.kind, "pid": c.pid, "installRoot": c.install_root, "resident": c.resident}))
        .collect();
    Json(json!({"clients": clients}))
}

/// 등록·heartbeat 본문의 `resident`: 창 없이 트레이만 든 GUI (없으면 false)
fn resident_flag(body: &serde_json::Value) -> bool {
    body.get("resident").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// POST /api/client/:id/heartbeat — TTL 갱신. 본문 `{resident?}`로 창 없이 트레이만 든 상태를 알린다
pub async fn client_heartbeat(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let body = body.map(|Json(v)| v).unwrap_or_default();
    if state.client_registry.heartbeat(&client_id).await {
        state.client_registry.set_resident(&client_id, resident_flag(&body)).await;
        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Client not registered"})),
        )
            .into_response()
    }
}

/// DELETE /api/client/:id/unregister — 클라이언트 명시적 해제
/// Query: ?shutdown=true → 완전히 종료: 데몬이 띄운 CLI 창을 닫고, 마지막 클라이언트면 데몬도 종료
///        ?shutdown=false → 데몬 유지 (인터페이스만 종료)
pub async fn client_unregister(
    Path(client_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let wants_shutdown = params.get("shutdown").map(|v| v == "true").unwrap_or(false);

    if state.client_registry.unregister(&client_id).await.is_some() {
        if wants_shutdown {
            state.client_registry.close_launched_clis();
        }
        let count = state.client_registry.count().await;
        tracing::info!("[Heartbeat] Active clients after unregister: {}", count);

        // shutdown=true && 남은 클라이언트 0 → 데몬 자체 종료
        if wants_shutdown && count == 0 {
            tracing::info!("[Shutdown] Voluntary shutdown requested via unregister — initiating graceful shutdown");
            state.request_shutdown();
        }

        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Client not registered"})),
        )
            .into_response()
    }
}

/// 백그라운드 태스크에서 호출 — 만료 클라이언트 정리
pub async fn reap_expired_clients(registry: &ClientRegistry) {
    let timeout = std::time::Duration::from_secs(90);
    let expired = registry.reap_expired(timeout).await;

    for (id, client) in &expired {
        tracing::warn!(
            "[Heartbeat] Cleaning up expired client: {} ({:?})",
            id,
            client.kind
        );
    }

    if !expired.is_empty() {
        let remaining = registry.count().await;
        tracing::info!(
            "[Heartbeat] Reap complete. Cleaned: {}, remaining clients: {}",
            expired.len(),
            remaining
        );
    }
}

/// GET /api/daemon/console?since=0&count=200&spans=1 — 데몬 자체 tracing 로그 조회 (`spans=1`이면 구문 강조 토큰을 붙인다)
pub async fn daemon_console(
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params.get("count").and_then(|c| c.parse::<usize>().ok());

    let entries = state.daemon_log_buffer.get_since(since_id, count);
    let count = entries.len();
    let mut lines = serde_json::to_value(entries).unwrap_or_else(|_| json!([]));
    // `spans=1`: 줄마다 구문 강조 토큰 구간 (인스턴스 콘솔과 같은 토큰, UTF-16 오프셋)
    if params.get("spans").is_some_and(|v| v == "1" || v == "true") {
        let hl = crate::supervisor::highlight::daemon_log();
        for line in lines.as_array_mut().into_iter().flatten() {
            let spans: Vec<serde_json::Value> = line
                .get("message")
                .and_then(|m| m.as_str())
                .map(|m| hl.spans(m).into_iter().map(|(s, e, t)| json!([s, e, t])).collect())
                .unwrap_or_default();
            if let Some(obj) = line.as_object_mut() {
                obj.insert("spans".to_string(), serde_json::Value::Array(spans));
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "lines": lines,
            "count": count,
            "epoch": state.daemon_log_buffer.epoch(),
        })),
    )
        .into_response()
}

/// 띄울 수 있는 인터페이스 (`POST /api/clients/launch`의 `kind`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interface {
    Cli,
    Gui,
}

impl Interface {
    fn binary(self) -> &'static str {
        match self {
            Interface::Cli => "saba-chan-cli",
            Interface::Gui => "saba-chan-gui",
        }
    }
}

/// 띄울 인터페이스, 실행 인자, 그 줄이 가리키는 인스턴스 id
type LaunchLine = (Interface, Vec<String>, Option<String>);

/// `POST /api/clients/launch` 본문에서 실행 줄을 만든다 (GUI ↔ CLI 전환).
/// 실행 줄은 여기서 정한 모양뿐이다: 본문 문자열을 인자로 그대로 넘기지 않는다. 틀리면 `(field, reason)`.
/// 인스턴스는 `--옵션=값` 한 칸으로 넘겨 id가 다른 옵션으로 읽히지 않는다
fn launch_args(body: &serde_json::Value) -> Result<LaunchLine, (&'static str, &'static str)> {
    let kind = match body.get("kind").and_then(|v| v.as_str()) {
        Some("cli") => Interface::Cli,
        Some("gui") => Interface::Gui,
        Some(_) => return Err(("kind", "must be \"cli\" or \"gui\"")),
        None => return Err(("kind", "required")),
    };
    let instance = match body.get("instance") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(v.as_str().filter(|s| !s.is_empty()).ok_or(("instance", "must be an instance id"))?.to_string()),
    };
    // GUI가 고른 원격 데몬 (G1, CLI 전역 `--daemon`). GUI를 띄울 때는 받지 않는다: GUI는 설정 `gui.daemonTarget`을 따른다
    let daemon = match body.get("daemon") {
        None | Some(serde_json::Value::Null) => None,
        Some(_) if kind == Interface::Gui => return Err(("daemon", "only for the cli")),
        Some(v) => Some(v.as_str().filter(|s| !s.is_empty()).ok_or(("daemon", "must be a saved remote daemon"))?.to_string()),
    };
    let mut args = match (kind, &instance) {
        (Interface::Cli, None) => vec!["tui".to_string()],
        (Interface::Cli, Some(id)) => vec!["tui".to_string(), format!("--server={id}")],
        (Interface::Gui, None) => Vec::new(),
        (Interface::Gui, Some(id)) => vec![format!("--open-server={id}")],
    };
    if let Some(d) = daemon {
        args.push(format!("--daemon={d}"));
    }
    Ok((kind, args, instance))
}

/// 띄울 GUI가 붙을 데몬 주소 (`IPC_BASE`). GUI는 설정 포트(`gui.ipcPort`)를 읽으므로, 이 데몬이 그와 다른 포트에서
/// 들을 때만(`SABA_IPC_PORT`) 알려 준다. 같으면 넘기지 않아 GUI가 나중에 바꾼 포트 설정을 따른다
pub(crate) fn gui_ipc_base(listen_addr: &str, configured: u16) -> Option<String> {
    let port = listen_addr.parse::<std::net::SocketAddr>().ok()?.port();
    (port != configured).then(|| format!("http://127.0.0.1:{port}"))
}

/// GUI가 읽는 설정 포트 (main.rs와 같은 자리, 없으면 기본값)
pub(crate) fn configured_gui_port() -> u16 {
    saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get("ipcPort")
        .and_then(|v| v.as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(saba_chan_updater_lib::constants::DEFAULT_IPC_PORT)
}

/// CLI는 새 콘솔 창이 있어야 한다: Windows, 또는 화면과 터미널 프로그램이 있는 리눅스. GUI는 데스크톱 화면이 있어야 한다
fn has_screen(kind: Interface) -> bool {
    match kind {
        Interface::Cli => cli_terminal_available(),
        Interface::Gui => crate::tray_host::has_desktop(),
    }
}

fn can_launch(kind: Interface) -> bool {
    has_screen(kind) && crate::boot_selector::find_interface_binary(kind.binary()).is_some()
}

/// GET /api/clients/launch — 이 PC에서 띄울 수 있는 인터페이스 `{cli, gui}` (전환 버튼을 보일지)
pub async fn client_launchable() -> impl IntoResponse {
    Json(json!({ "cli": can_launch(Interface::Cli), "gui": can_launch(Interface::Gui) }))
}

/// POST /api/clients/launch — 다른 인터페이스를 띄운다 (GUI ↔ CLI 전환). 띄운 뒤 부른 쪽이 스스로 닫는다.
/// CLI는 새 터미널 창에 전체 화면 메뉴로, GUI는 창으로. 로컬 전용: 액션 카탈로그에 없다
pub async fn client_launch(State(state): State<IPCServer>, Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let (kind, args, instance) = match launch_args(&body) {
        Ok(v) => v,
        Err((field, reason)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "error": format!("Invalid input '{}': {}", field, reason), "error_code": "invalid_input", "field": field, "reason": reason })),
            )
                .into_response()
        }
    };
    // 원격 데몬의 서버는 그 데몬에 있다: 저장한 원격 데몬인지만 보고, 인스턴스는 띄운 CLI가 그 데몬에 묻는다
    let remote = body.get("daemon").and_then(|v| v.as_str());
    if let Some(wanted) = remote {
        if let Err(code) = crate::remote_daemon::target::resolve(&saba_chan_updater_lib::config_file::ConfigFile::default_location(), wanted) {
            return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("Remote daemon not found: {wanted}"), "error_code": code }))).into_response();
        }
    }
    if let Some(id) = instance.as_ref().filter(|_| remote.is_none()) {
        if state.supervisor.read().await.instance_store.get(id).is_none() {
            return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response();
        }
    }
    // 띄울 화면이 없으면(CLI는 화면이나 터미널 프로그램 없음, GUI는 데스크톱 화면 없음) 띄운 척하지 않는다: 부른 쪽이 닫히지 않게
    if !has_screen(kind) {
        let error = match kind {
            Interface::Cli => "No terminal program to open the CLI in",
            Interface::Gui => "This PC has no desktop screen to open the app on",
        };
        return (StatusCode::NOT_IMPLEMENTED, Json(json!({ "success": false, "error": error, "error_code": "unsupported_platform" }))).into_response();
    }
    let Some(binary) = crate::boot_selector::find_interface_binary(kind.binary()) else {
        let code = match kind {
            Interface::Cli => "cli_not_found",
            Interface::Gui => "gui_not_found",
        };
        return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("{} was not found next to the daemon", kind.binary()), "error_code": code }))).into_response();
    };
    let spawned = match kind {
        Interface::Cli => spawn_in_new_console(&binary, &args, &state.client_registry),
        Interface::Gui => {
            let env: Vec<(&str, String)> = gui_ipc_base(&state.listen_addr, configured_gui_port()).map(|b| ("IPC_BASE", b)).into_iter().collect();
            crate::boot_selector::spawn_gui_with(&binary, &args, &env)
        }
    };
    match spawned {
        Ok(pid) => {
            tracing::info!("[Launch] Opened {} {:?} (pid {})", kind.binary(), args, pid);
            (StatusCode::OK, Json(json!({ "success": true, "pid": pid }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": format!("Could not start {}: {}", kind.binary(), e), "error_code": "launch_failed" }))).into_response(),
    }
}

fn cli_terminal_available() -> bool {
    #[cfg(windows)]
    return true;
    #[cfg(target_os = "linux")]
    return crate::tray_host::has_desktop() && crate::boot_selector::has_terminal();
    #[cfg(not(any(windows, target_os = "linux")))]
    return false;
}

#[cfg(windows)]
fn spawn_in_new_console(cli: &std::path::Path, args: &[String], registry: &crate::ipc::ClientRegistry) -> std::io::Result<u32> {
    crate::boot_selector::spawn_cli_in_new_console(cli, args).inspect(|&pid| registry.note_launched_cli(pid))
}

/// 터미널이 CLI를 띄우기까지 잠시 걸린다: 표시를 단 CLI가 나타나면 그 pid를 기록한다 (완전히 종료할 때 닫는다)
#[cfg(target_os = "linux")]
fn spawn_in_new_console(cli: &std::path::Path, args: &[String], registry: &crate::ipc::ClientRegistry) -> std::io::Result<u32> {
    let tag = uuid::Uuid::new_v4().to_string();
    let pid = crate::boot_selector::spawn_cli_in_new_console(cli, args, &tag)?;
    let name = cli.file_name().and_then(|n| n.to_str()).unwrap_or("saba-chan-cli").to_string();
    let registry = registry.clone();
    tokio::spawn(async move {
        for _ in 0..100 {
            if let Some(cli_pid) = crate::boot_selector::find_tagged_process(&tag, &name) {
                return registry.note_launched_cli(cli_pid);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        tracing::warn!("[Launch] The CLI opened in a terminal did not show up; a full quit will not close it");
    });
    Ok(pid)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn spawn_in_new_console(_cli: &std::path::Path, _args: &[String], _registry: &crate::ipc::ClientRegistry) -> std::io::Result<u32> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not supported on this OS"))
}

#[cfg(test)]
mod launch_tests {
    use super::{launch_args, Interface};
    use serde_json::json;

    /// 전환에 쓰는 실행 줄은 정해진 모양뿐: CLI는 전체 화면 메뉴, GUI는 창. 인스턴스는 `--옵션=값` 한 칸
    #[test]
    fn only_fixed_command_lines_are_built() {
        let line = |b| launch_args(&b).unwrap();
        assert_eq!(line(json!({ "kind": "cli" })), (Interface::Cli, vec!["tui".to_string()], None));
        assert_eq!(
            line(json!({ "kind": "cli", "instance": "-abc" })),
            (Interface::Cli, vec!["tui".to_string(), "--server=-abc".to_string()], Some("-abc".to_string()))
        );
        assert_eq!(line(json!({ "kind": "gui" })), (Interface::Gui, vec![], None));
        assert_eq!(line(json!({ "kind": "gui", "instance": "abc" })), (Interface::Gui, vec!["--open-server=abc".to_string()], Some("abc".to_string())));
        // 예전 본문(`view`)도 받는다: 더는 콘솔 창 하나만 띄우지 않고 메뉴를 연다
        assert_eq!(line(json!({ "kind": "cli", "view": "console", "instance": "abc" })).1, vec!["tui".to_string(), "--server=abc".to_string()]);

        let field = |b| launch_args(&b).unwrap_err().0;
        assert_eq!(field(json!({ "kind": "web" })), "kind");
        assert_eq!(field(json!({ "view": "shell" })), "kind");
        assert_eq!(field(json!({ "kind": "cli", "instance": 5 })), "instance");
        assert_eq!(field(json!({ "kind": "gui", "instance": "" })), "instance");

        // GUI에서 고른 원격 데몬(G1)은 CLI에도 넘긴다: 한 칸 `--daemon=값`
        assert_eq!(
            line(json!({ "kind": "cli", "instance": "abc", "daemon": "host-b" })).1,
            vec!["tui".to_string(), "--server=abc".to_string(), "--daemon=host-b".to_string()]
        );
        assert_eq!(field(json!({ "kind": "cli", "daemon": "" })), "daemon");
        assert_eq!(field(json!({ "kind": "gui", "daemon": "host-b" })), "daemon");
    }

    /// 띄운 GUI는 이 데몬에 붙는다: 설정 포트와 다른 포트에서 돌 때(`SABA_IPC_PORT`)만 주소를 넘기고,
    /// 같으면 넘기지 않아 GUI가 나중에 바꾼 포트 설정을 따른다
    #[test]
    fn a_launched_gui_is_pointed_at_this_daemon() {
        assert_eq!(super::gui_ipc_base("127.0.0.1:61234", 57474).as_deref(), Some("http://127.0.0.1:61234"));
        assert_eq!(super::gui_ipc_base("0.0.0.0:61234", 57474).as_deref(), Some("http://127.0.0.1:61234"));
        assert_eq!(super::gui_ipc_base("127.0.0.1:57474", 57474), None);
        assert_eq!(super::gui_ipc_base("nope", 57474), None);
    }
}

#[cfg(test)]
mod console_tests {
    use super::*;

    /// 데몬 로그 번호는 데몬을 켤 때마다 1부터 다시 센다: 답에 버퍼 세대 `epoch`를 실어,
    /// 이어 읽던 클라이언트가 바뀐 것을 알고 번호를 버리게 한다 (L55)
    #[tokio::test]
    async fn the_daemon_console_carries_an_epoch_that_changes_with_the_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let read = |buffer: crate::daemon_log::DaemonLogBuffer| {
            let state = IPCServer::new(sup.clone(), "127.0.0.1:0", buffer);
            async move {
                let res = daemon_console(State(state), axum::extract::Query(Default::default())).await.into_response();
                crate::ipc::internal::into_internal(res).await.body
            }
        };
        let first = read(crate::daemon_log::DaemonLogBuffer::new()).await;
        let again = read(crate::daemon_log::DaemonLogBuffer::new()).await;
        let (a, b) = (first["epoch"].as_u64().expect("epoch"), again["epoch"].as_u64().expect("epoch"));
        assert!(b > a, "a restarted daemon's buffer has a newer epoch: {a} then {b}");
    }

    /// `spans=1`이면 데몬 로그 줄마다 구문 강조 토큰을 붙인다 (CLI·GUI가 같은 규칙으로 칠한다)
    #[tokio::test]
    async fn the_daemon_console_can_carry_highlight_spans() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let buffer = crate::daemon_log::DaemonLogBuffer::new();
        buffer.push("info".into(), "saba_core::ipc".into(), "[Heartbeat] Active clients: 2".into());
        let read = |query: &[(&str, &str)]| {
            let state = IPCServer::new(sup.clone(), "127.0.0.1:0", buffer.clone());
            let query = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            async move {
                let res = daemon_console(State(state), axum::extract::Query(query)).await.into_response();
                crate::ipc::internal::into_internal(res).await.body
            }
        };
        let with = read(&[("spans", "1")]).await;
        assert_eq!(with["lines"][0]["spans"][0], json!([0, 11, "keyword"]), "{with}");
        assert_eq!(with["lines"][0]["message"], "[Heartbeat] Active clients: 2");
        let without = read(&[]).await;
        assert!(without["lines"][0].get("spans").is_none(), "{without}");
    }
}

#[cfg(test)]
mod quit_tests {
    use super::*;

    /// "완전히 종료"(unregister?shutdown=true)는 데몬이 "CLI로 전환"으로 띄운 CLI 창도 닫는다 (PC 확인)
    #[tokio::test]
    async fn a_full_quit_closes_the_cli_the_daemon_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let state = IPCServer::new(sup, "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        #[cfg(windows)]
        let mut cli = std::process::Command::new("ping").args(["-n", "60", "127.0.0.1"]).spawn().unwrap();
        #[cfg(not(windows))]
        let mut cli = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        state.client_registry.note_launched_cli(cli.id());
        let id = state.client_registry.register(ClientKind::Gui, None, None).await;
        let quit = [("shutdown".to_string(), "true".to_string())].into_iter().collect();
        let res = client_unregister(Path(id), axum::extract::Query(quit), State(state)).await.into_response();
        assert_eq!(res.status(), StatusCode::OK);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while cli.try_wait().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "the CLI the daemon opened is still running");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
