//! 같은 서버 폴더를 쓰는 인스턴스들: 한쪽이 파일을 바꾸는 동안 다른 쪽의 작업·실행을 가른다

use serde_json::json;

use super::Failure;
use crate::ipc::IPCServer;

/// 켜져 있는 서버의 파일을 바꾸는 작업을 거절하는 409
pub(crate) fn server_running() -> Failure {
    Failure::running(json!({ "success": false, "error": "Stop the server first", "error_code": "server_running" }))
}

/// `dir`과 겹치는 폴더(working_dir 또는 인스턴스 폴더, 그 안이나 바깥)를 쓰는 인스턴스 (`except`는 뺀다)
async fn other_instances_using(state: &IPCServer, dir: &str, except: Option<&str>) -> Vec<String> {
    let supervisor = state.supervisor.read().await;
    let dir = std::path::Path::new(dir);
    supervisor.instance_store.list().iter()
        .filter(|i| {
            i.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::paths_overlap(std::path::Path::new(d), dir))
                || saba_chan_updater_lib::paths_overlap(&supervisor.instance_store.instance_dir(&i.id), dir)
        })
        .filter(|i| except != Some(i.id.as_str()))
        .map(|i| i.id.clone())
        .collect()
}

/// `hold_other_instances_using`이 지금 거절할지를 잠금을 잡지 않고 본다: 그 인스턴스 중 작업 중이거나 켜진 것이 있다
pub(crate) async fn other_instances_busy(state: &IPCServer, dir: &str, except: Option<&str>) -> bool {
    for id in other_instances_using(state, dir, except).await {
        if crate::instance::lock::InstanceLock::held(&id) || crate::service::backup::is_running(state, &id).await {
            return true;
        }
    }
    false
}

/// `dir`과 겹치는 폴더(working_dir 또는 인스턴스 폴더, 그 안이나 바깥)를 쓰는 인스턴스의 작업 잠금을 모두 잡는다.
/// `except`(이미 잠금을 쥔 인스턴스)는 건너뛴다. 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running`
pub(crate) async fn hold_other_instances_using(state: &IPCServer, dir: &str, except: Option<&str>) -> Result<Vec<crate::instance::lock::InstanceLock>, Failure> {
    let mut guards = Vec::new();
    for id in other_instances_using(state, dir, except).await {
        let Some(guard) = crate::instance::lock::InstanceLock::take_for_writes(&id).await else {
            return Err(Failure::busy());
        };
        guards.push(guard);
        if crate::service::backup::is_running(state, &id).await {
            return Err(server_running());
        }
    }
    Ok(guards)
}
