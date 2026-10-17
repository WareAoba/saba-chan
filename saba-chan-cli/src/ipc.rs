//! 데몬 IPC 공통 — 주소·토큰 해석과 blocking 요청 헬퍼.
//!
//! - async 요청은 [`crate::client::DaemonClient`]가 담당한다.
//! - 이 모듈은 동기 코드(프로세스 관리, 설정 조회, 모듈 레지스트리)용
//!   blocking 요청 하나만 제공한다.
//! - IPC 포트는 로컬 `settings.json`에서만 읽는다. 데몬에 포트를 묻는 것은
//!   순환이므로 하지 않는다.

use std::time::Duration;

use saba_chan_updater_lib::constants;
use serde_json::Value;

/// 데몬(`src/ipc/auth.rs`)이 작성한 `.ipc_token` 파일을 읽는다.
/// 파일이 없거나 비어 있으면 `None`.
pub(crate) fn read_token_from_file() -> Option<String> {
    std::fs::read_to_string(constants::token_file_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// IPC 포트 — 로컬 `settings.json`의 `ipcPort`, 없으면 기본값.
pub(crate) fn ipc_port() -> u16 {
    std::fs::read_to_string(constants::resolve_settings_path())
        .ok()
        .and_then(|c| serde_json::from_str::<Value>(&c).ok())
        .and_then(|s| s.get("ipcPort").and_then(|v| v.as_u64()))
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(constants::DEFAULT_IPC_PORT)
}

/// IPC base URL (`http://127.0.0.1:{port}`).
pub fn base_url() -> String {
    format!("http://127.0.0.1:{}", ipc_port())
}

/// tokio multi-thread runtime 안이면 `block_in_place`로 감싸서 실행한다.
/// 런타임 밖(동기 테스트 등)이면 그대로 실행한다.
fn in_blocking_section<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// 로컬 데몬에 blocking 요청을 보낸다. 2xx가 아니면 에러.
/// 응답 본문이 JSON이 아니면 `Value::Null`을 돌려준다.
pub(crate) fn blocking_request(
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Value> {
    in_blocking_section(|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        let mut req = client.request(method.clone(), format!("{}{}", base_url(), path));
        if let Some(token) = read_token_from_file() {
            req = req.header("X-Saba-Token", token);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send()?;
        if !resp.status().is_success() {
            anyhow::bail!("Daemon {} {} failed: {}", method, path, resp.status());
        }
        Ok(resp.json().unwrap_or(Value::Null))
    })
}
