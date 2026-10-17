//! API 계약 테스트 (C3)
//!
//! 1. GUI(`saba-chan-gui/main.js`)와 CLI(`saba-chan-cli/src`)가 부르는 데몬 API가
//!    라우트 표(`src/ipc/mod.rs`, `src/ipc/updates.rs`)에 있고 메서드도 맞는지
//!    소스에서 대조한다. 없는 라우트·틀린 메서드(404/405) 호출을 CI에서 잡는다.
//! 2. 실제로 IPC 서버를 띄워 모든 라우트가 토큰 없이는 401인지 확인한다.
//! 3. 표의 모든 경로가 실제 라우터에 등록됐는지 확인한다(등록 안 된 메서드 → 405).
//!
//! 인증을 켜야 하므로 `SABA_AUTH_DISABLED`를 쓰는 다른 테스트와 다른 바이너리로 둔다.

use regex::Regex;
use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct Route {
    path: String,
    methods: Vec<String>,
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("read {}: {}", rel, e))
}

/// `.route("/path", get(h).put(h2))` 형태를 모은다.
fn route_table() -> Vec<Route> {
    let re = Regex::new(r#"\.route\(\s*"([^"]+)"\s*,\s*((?:[a-z]+\([^()]*\)\.?)+)"#).unwrap();
    let method = Regex::new(r"(get|post|put|patch|delete)\(").unwrap();
    let mut routes: Vec<Route> = Vec::new();
    for file in ["src/ipc/mod.rs", "src/ipc/updates.rs"] {
        for cap in re.captures_iter(&read(file)) {
            let methods: Vec<String> = method.captures_iter(&cap[2]).map(|m| m[1].to_string()).collect();
            match routes.iter_mut().find(|r| r.path == cap[1]) {
                Some(r) => r.methods.extend(methods),
                None => routes.push(Route { path: cap[1].to_string(), methods }),
            }
        }
    }
    routes
}

/// (출처, 메서드, 경로) — 경로의 `${…}`·`{}` 보간은 `*`로 바뀐다.
fn client_calls() -> Vec<(String, String, String)> {
    let mut calls = Vec::new();
    let gui = Regex::new(r"axios\.(get|post|put|patch|delete)\(\s*`\$\{IPC_BASE\}([^`]*)`").unwrap();
    for cap in gui.captures_iter(&read("saba-chan-gui/main.js")) {
        calls.push(("gui/main.js".to_string(), cap[1].to_string(), cap[2].to_string()));
    }
    let cli = Regex::new(
        r#"(get_json|post_json_long|post_json|put_json|patch_json|delete_json|daemon_get|daemon_put|daemon_post|daemon_delete)\(\s*&?(?:format!\()?\s*"([^"]+)""#,
    )
    .unwrap();
    let cli_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("saba-chan-cli/src");
    for entry in walk(&cli_dir) {
        let text = std::fs::read_to_string(&entry).unwrap();
        for cap in cli.captures_iter(&text) {
            let method = match &cap[1] {
                "get_json" | "daemon_get" => "get",
                "post_json" | "post_json_long" | "daemon_post" => "post",
                "put_json" | "daemon_put" => "put",
                "patch_json" => "patch",
                _ => "delete",
            };
            calls.push((
                format!("cli/{}", entry.file_name().unwrap().to_string_lossy()),
                method.to_string(),
                cap[2].to_string(),
            ));
        }
    }
    calls
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}

/// 클라이언트 경로가 라우트 패턴과 맞는지. 보간 부분은 와일드카드, 쿼리는 무시.
fn matches(call: &str, pattern: &str) -> bool {
    static INTERP: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let interp = INTERP.get_or_init(|| Regex::new(r"\$\{[^}]*\}|\{[^}]*\}").unwrap());
    let call = call.split('?').next().unwrap();
    let call = interp.replace_all(call, "\u{1}");
    let a: Vec<&str> = call.trim_matches('/').split('/').collect();
    let b: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    a.len() == b.len()
        && a.iter().zip(&b).all(|(x, y)| {
            // 마지막 세그먼트 끝에 붙은 `${qs}` 같은 보간은 쿼리로 본다
            let x = x.trim_end_matches('\u{1}');
            y.starts_with(':') || x == *y || (x.is_empty() && !y.is_empty())
        })
}

#[test]
fn clients_only_call_existing_routes_with_the_right_method() {
    let routes = route_table();
    let calls = client_calls();
    assert!(routes.len() > 60, "route parser found only {} routes", routes.len());
    assert!(calls.len() > 80, "client parser found only {} calls", calls.len());

    let mut problems = Vec::new();
    for (who, method, path) in &calls {
        let candidates: Vec<&Route> = routes.iter().filter(|r| matches(path, &r.path)).collect();
        if candidates.is_empty() {
            problems.push(format!("{}: {} {} — no such route", who, method.to_uppercase(), path));
        } else if !candidates.iter().any(|r| r.methods.contains(method)) {
            let allowed: Vec<String> = candidates.iter().flat_map(|r| r.methods.clone()).collect();
            problems.push(format!(
                "{}: {} {} — route allows {:?}",
                who,
                method.to_uppercase(),
                path,
                allowed
            ));
        }
    }
    assert!(problems.is_empty(), "client/daemon contract mismatches:\n{}", problems.join("\n"));
}

fn concrete(path: &str) -> String {
    path.split('/')
        .map(|s| if s.starts_with(':') { "contract-test" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

#[tokio::test]
async fn every_route_is_registered_and_requires_the_token() {
    std::env::remove_var("SABA_AUTH_DISABLED");
    let token = saba_core::ipc::auth::generate_and_save_token().expect("token");

    let instances = tempfile::tempdir().unwrap();
    let modules = tempfile::tempdir().unwrap();
    let supervisor = Arc::new(RwLock::new(Supervisor::new_with_instances_dir(
        &modules.path().to_string_lossy(),
        &instances.path().to_string_lossy(),
    )));
    supervisor.write().await.initialize().await.expect("supervisor init");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let base = format!("http://{}", addr);
    let server = IPCServer::new(supervisor, &addr.to_string(), saba_core::daemon_log::DaemonLogBuffer::new());
    let task = tokio::spawn(async move {
        let _ = server.start().await;
    });

    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..100 {
        if let Ok(r) = client.get(format!("{}/health", base)).header("X-Saba-Token", token.as_str()).send().await {
            if r.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(ready, "IPC server did not start");

    let mut problems = Vec::new();
    for route in route_table() {
        let url = format!("{}{}", base, concrete(&route.path));
        for method in &route.methods {
            let req = match method.as_str() {
                "get" => client.get(&url),
                "post" => client.post(&url),
                "put" => client.put(&url),
                "patch" => client.patch(&url),
                _ => client.delete(&url),
            };
            let status = req.json(&serde_json::json!({})).send().await.unwrap().status();
            if status != reqwest::StatusCode::UNAUTHORIZED {
                problems.push(format!("{} {} without token -> {}", method.to_uppercase(), route.path, status));
            }
        }
        // 등록된 경로에 등록되지 않은 메서드를 보내면 axum은 405, 없는 경로면 404를 준다.
        // 핸들러를 실행하지 않고 라우트 존재만 확인한다.
        let unused = ["patch", "delete", "put", "post", "get"]
            .into_iter()
            .find(|m| !route.methods.iter().any(|x| x == m))
            .expect("some method is unused");
        let status = client
            .request(unused.to_uppercase().parse().unwrap(), &url)
            .header("X-Saba-Token", token.as_str())
            .send()
            .await
            .unwrap()
            .status();
        if status != reqwest::StatusCode::METHOD_NOT_ALLOWED {
            problems.push(format!("{} is not registered ({} -> {})", route.path, unused.to_uppercase(), status));
        }
    }
    task.abort();
    assert!(problems.is_empty(), "route contract problems:\n{}", problems.join("\n"));
}
