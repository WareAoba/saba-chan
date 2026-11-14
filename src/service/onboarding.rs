//! 처음 쓰는 사람의 다음 할 일 (J1-a): 단계 계산과 첫 시작 기록

use serde_json::json;

use crate::ipc::IPCServer;

/// 지금의 온보딩 단계 (`GET /api/onboarding`과 relay 하트비트가 같이 쓴다)
pub(crate) async fn now(state: &IPCServer) -> serde_json::Value {
    let started_before = started_before(&state.config_store);
    let (modules, servers, running) = {
        let sup = state.supervisor.read().await;
        // 이미 켜 본 적이 있으면 지금 켜진 서버를 셀 필요가 없다
        let running = !started_before && !sup.running_instances().await.is_empty();
        (sup.list_modules().map(|m| m.len()).unwrap_or(0), sup.instance_store.list().len(), running)
    };
    onboarding_steps(&Progress {
        paired: state.relay_agent.status().has_token,
        modules,
        servers,
        started: running || started_before,
        chatbot: state.config_store.get_chatbot().api_key.is_some_and(|k| !k.trim().is_empty()),
    })
}

/// 온보딩 단계를 가르는 지금 상태
#[derive(Default)]
struct Progress {
    paired: bool,
    modules: usize,
    servers: usize,
    /// 서버를 한 번이라도 켰다 (지금 켜져 있거나 `onboarding.started`)
    started: bool,
    chatbot: bool,
}

/// `{steps:[{id, done, optional}], next}`: `next`는 아직 안 한 첫 필수 단계 (다 했으면 null).
/// 디스코드 연결이 먼저다 (사용자 결정 2026-11-09): 연결하면 친구 접속 주소·포트 확인도 바로 쓴다
fn onboarding_steps(p: &Progress) -> serde_json::Value {
    let steps = [
        ("discord", p.paired, false),
        ("module", p.modules > 0, false),
        ("server", p.servers > 0, false),
        ("start", p.started, false),
        ("chatbot", p.chatbot, true),
    ];
    let next = steps.iter().find(|(_, done, optional)| !done && !optional).map(|(id, _, _)| *id);
    json!({
        "steps": steps.iter().map(|(id, done, optional)| json!({ "id": id, "done": done, "optional": optional })).collect::<Vec<_>>(),
        "next": next,
    })
}

/// 첫 시작을 남기는 config.json 칸 (`onboarding.started`). 서버를 지우거나 꺼 둬도 단계가 되돌아가지 않게 남긴다
const ONBOARDING_SECTION: &str = "onboarding";

fn started_before(store: &crate::config_store::ConfigStore) -> bool {
    store.get_client_section(ONBOARDING_SECTION).ok().and_then(|v| v.get("started").and_then(|s| s.as_bool())).unwrap_or(false)
}

/// 첫 시작을 남긴다. 남겼거나 이미 있으면 true
fn note_started(store: &crate::config_store::ConfigStore) -> bool {
    if started_before(store) {
        return true;
    }
    match store.merge_client_section(ONBOARDING_SECTION, json!({ "started": true })) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("Could not remember the first server start: {}", e);
            false
        }
    }
}

/// 이 단계를 처음 보는 설치(`onboarding` 칸이 없다)라면, 그때 서버가 있으면 이 단계가 생기기 전부터 쓰던 사람이라 켜 본 것으로 친다.
/// 칸을 한 번 만들어 두므로 그 뒤에 가져온 서버는 실제로 켜야 끝난다
fn adopt_existing_install(store: &crate::config_store::ConfigStore, servers: usize) {
    let seen = store.get_client_section(ONBOARDING_SECTION).ok().and_then(|v| v.as_object().map(|o| o.contains_key("started"))).unwrap_or(false);
    if seen {
        return;
    }
    if let Err(e) = store.merge_client_section(ONBOARDING_SECTION, json!({ "started": servers > 0 })) {
        tracing::warn!("Could not set up the onboarding state: {}", e);
    }
}

/// 서버가 처음 켜지면(`instance.started`) 온보딩 "서버 시작해 보기"를 끝낸 것으로 남긴다. 이미 남겼으면 듣지 않는다
pub(crate) async fn watch_first_start(state: IPCServer) {
    let mut sub = crate::events::bus().subscribe(None);
    let running = {
        let sup = state.supervisor.read().await;
        adopt_existing_install(&state.config_store, sup.instance_store.list().len());
        !sup.running_instances().await.is_empty()
    };
    // 듣기 전에 켜진 서버(부팅 때 자동 시작)도 놓치지 않는다
    if started_before(&state.config_store) || (running && note_started(&state.config_store)) {
        return;
    }
    loop {
        match sub.rx.recv().await {
            // 남기지 못했으면 다음 시작 때 다시 남긴다
            Ok(event) if event.kind == "instance.started" => {
                if note_started(&state.config_store) {
                    return;
                }
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_starts_with_discord_and_ends_with_a_first_start() {
        let ids: Vec<_> = onboarding_steps(&Progress::default())["steps"].as_array().unwrap().iter().map(|s| s["id"].clone()).collect();
        assert_eq!(ids, ["discord", "module", "server", "start", "chatbot"]);
        assert_eq!(onboarding_steps(&Progress::default())["next"], "discord");
        let p = |paired, modules, servers, started| Progress { paired, modules, servers, started, chatbot: false };
        assert_eq!(onboarding_steps(&p(true, 0, 0, false))["next"], "module");
        assert_eq!(onboarding_steps(&p(true, 2, 0, false))["next"], "server");
        assert_eq!(onboarding_steps(&p(true, 2, 1, false))["next"], "start");
        assert_eq!(onboarding_steps(&p(false, 2, 1, true))["next"], "discord", "skipping discord in the app does not finish the step");
        let done = onboarding_steps(&p(true, 2, 1, true));
        assert!(done["next"].is_null(), "the chatbot is optional");
        assert_eq!(done["steps"][4], json!({ "id": "chatbot", "done": false, "optional": true }));
    }

    #[test]
    fn a_first_start_is_remembered_once_and_survives_without_running_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::config_store::ConfigStore::new(tmp.path());
        assert!(!started_before(&store));
        note_started(&store);
        note_started(&store);
        assert!(started_before(&crate::config_store::ConfigStore::new(tmp.path())), "kept in config.json");
    }

    #[test]
    fn an_upgraded_install_with_servers_does_not_go_back_to_the_start_step() {
        // 이 단계가 생기기 전부터 쓰던 사람: 서버가 이미 있으면 켜 본 것으로 친다
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::config_store::ConfigStore::new(tmp.path());
        adopt_existing_install(&store, 2);
        assert!(started_before(&store));

        // 새로 설치한 사람: 처음 본 때 서버가 없으면 그 뒤에 가져온 서버는 켜 봐야 끝난다
        let fresh = tempfile::tempdir().unwrap();
        let store = crate::config_store::ConfigStore::new(fresh.path());
        adopt_existing_install(&store, 0);
        adopt_existing_install(&store, 1);
        assert!(!started_before(&store), "only the first look counts");
    }
}
