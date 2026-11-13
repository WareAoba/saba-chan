use super::*;
use crate::chat::permissions::Requester;
use std::sync::Arc;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\nlabel = \"pw\"\n\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\nlabel = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\narg = \"-motd={value}\"\n\n[[settings.fields]]\nname = \"players\"\ntype = \"number\"\nlabel = \"players\"\narg = \"-players={value}\"\n\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\ninputs = [{ name = \"limit\", type = \"number\", min = 1, max = 10 }]\n";
const MODULE_KO: &str = r#"{ "queries": { "players": { "label": "접속자 목록", "description": "지금 접속한 사람", "inputs": { "limit": { "label": "최대 인원" } } } } }"#;

async fn server() -> (IPCServer, String, tempfile::TempDir) {
    server_with(MODULE).await
}

const LIFECYCLE: &str = r#"import json, sys
json.load(sys.stdin)
function = sys.argv[1]
if function == "functions":
    result = {"success": True, "functions": ["configure", "read_properties"]}
elif function in ("configure", "read_properties"):
    result = {"success": True}
else:
    result = {"success": False, "error_code": "not_supported", "message": "Fixture function not supported"}
print(json.dumps(result))
"#;

async fn server_with(module: &str) -> (IPCServer, String, tempfile::TempDir) {
    server_running(module, LIFECYCLE).await
}

async fn server_running(module: &str, lifecycle: &str) -> (IPCServer, String, tempfile::TempDir) {
    crate::ipc::auth::ensure_test_token();
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let dir = modules.join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), module).unwrap();
    std::fs::write(dir.join("lifecycle.py"), lifecycle).unwrap();
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(dir.join("locales").join("ko.json"), MODULE_KO).unwrap();
    crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
    let instances = tmp.path().join("instances");
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
    sup.initialize().await.unwrap();
    let mut inst = crate::instance::ServerInstance::new("srv", "plain");
    inst.module_settings.insert("admin_pass".into(), json!("S3CRET-MOD"));
    let id = inst.id.clone();
    sup.instance_store.add(inst).unwrap();
    let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    (state, id, tmp)
}

fn relay(user: &str, chatbot: bool) -> Caller {
    let who = Requester { guild_id: Some("g".into()), user_id: Some(user.into()), owner_id: Some("owner".into()), via_chatbot: chatbot, ..Default::default() };
    Caller { surface: if chatbot { Chatbot } else { Discord }, remote: Some(who), origin: crate::audit::Origin::new("discord"), locale: "ko".into() }
}

/// 챗봇은 읽은 판(`revision`)을 실어야 예약을 바꾼다: 지금 판을 붙인 입력
async fn with_revision(state: &IPCServer, id: &str, mut input: Value) -> Value {
    let got = query(state, "instance.schedule.get", Some(id), &json!({}), &gui()).await.unwrap();
    input["revision"] = got["result"]["revision"].clone();
    input
}

/// 확인 버튼을 누른 요청자 (릴레이 `confirmedBy`)
fn pressed(user: &str) -> Caller {
    let mut c = relay(user, false);
    c.remote.as_mut().unwrap().confirmed_by = Some(user.into());
    c
}

fn gui() -> Caller {
    Caller::local(crate::audit::Origin::new("gui"), "en".into())
}

async fn set_node(state: &IPCServer, node: Value) {
    state
        .config_store
        .update_node_settings("g", |n| {
            *n = node.as_object().cloned().unwrap();
            ((), true)
        })
        .await
        .unwrap();
}

/// 이 길드는 무엇이든 열어 둔다 (느슨, 챗봇 상한 관리자(줄 수 있는 최고), 명령별 덮어쓰기 0)
async fn open_everything(state: &IPCServer) {
    set_node(state, json!({ "ownerDiscordId": "owner", "preset": "loose", "chatbotCap": 2, "defaultLevel": 2, "commandLevels": { "update.apply": 0 } })).await;
}

#[tokio::test]
async fn the_chatbot_cannot_reach_forbidden_actions_whatever_the_levels() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let owner_bot = relay("owner", true);
    for (action, instance) in [("instance.delete", Some(id.as_str())), ("update.apply", None), ("extension.enable", None)] {
        let input = if action == "extension.enable" { json!({ "extension": "x" }) } else { json!({}) };
        let err = plan(&state, action, instance, &input, &owner_bot).await.unwrap_err();
        assert_eq!(err.code, "action_not_allowed", "{}", action);
    }
    // 비밀값은 챗봇이 바꾸지 못한다 (모듈의 password 필드와 알려진 이름)
    for key in ["admin_pass", "rest_password"] {
        let err = plan(&state, "instance.settings.set", Some(&id), &json!({ key: "x" }), &owner_bot).await.unwrap_err();
        assert_eq!((err.code, err.data["field"].as_str()), ("invalid_input", Some(key)), "{:?}", err);
    }
    assert!(plan(&state, "instance.settings.set", Some(&id), &json!({ "admin_pass": "x" }), &gui()).await.is_ok());
    // 사용 모드와 그 명단·자동 강퇴는 PC 앱·웹에서만 바꾼다 (디스코드·말하는 사바쨩 불가, 사용자 지정 2026-11-08)
    for (key, value) in [
        ("access", json!("shared")),
        ("controllers_only", json!(false)),
        ("controllers", json!([])),
        ("kick_players", json!([])),
    ] {
        let err = plan(&state, "instance.settings.set", Some(&id), &json!({ key: value }), &owner_bot).await.unwrap_err();
        assert_eq!((err.code, err.data["field"].as_str()), ("invalid_input", Some(key)), "{:?}", err);
    }
    assert!(plan(&state, "instance.settings.set", Some(&id), &json!({ "access": "members" }), &gui()).await.is_ok());
}

/// 원격 포트(짝 맺은 PC)에서 온 액션은 대상 라우트·칸이 원격 포트에서 막힌 것이면 계획·실행·여러 단계 계획 모두
/// 미리 거절한다 (여러 단계에서 앞 단계만 돌고 멈추지 않게). 안에서 다시 부르는 다른 호출도 같은 규칙 층을 지난다
#[tokio::test]
async fn a_paired_pc_cannot_reach_routes_the_remote_port_refuses_through_actions() {
    let (state, id, _tmp) = server().await;
    let remote = crate::audit::Origin::new("remote");
    let paired = Caller::local(remote.clone(), "en".into());
    for action in ["update.apply", "update.check"] {
        let err = plan(&state, action, None, &json!({}), &paired).await.unwrap_err();
        assert_eq!((err.status, err.code), (403, "remote_local_only"), "{action}");
        let err = run(&state, action, None, &json!({}), None, &paired).await.unwrap_err();
        assert_eq!((err.status, err.code), (403, "remote_local_only"), "{action}");
    }
    let steps = json!([{ "id": "server.status", "instance": id, "input": {} }, { "id": "update.apply", "input": {} }]);
    assert_eq!(plan_steps(&state, &steps, &paired).await.unwrap_err().code, "remote_local_only");
    // 원격 포트가 막는 PC 전용 칸도 같다
    let err = plan(&state, "instance.settings.set", Some(&id), &json!({ "java_path": "/bin/sh" }), &paired).await.unwrap_err();
    assert_eq!((err.status, err.code), (403, "remote_field_blocked"), "{err:?}");
    assert_eq!(err.data["fields"], json!(["java_path"]));
    // 같은 계획에서 만들 서버(`@N`)도 그 모듈의 PC 전용 칸으로 본다
    let on_new = json!([
        { "id": "instance.create", "input": { "name": "new-srv", "module_name": "plain" } },
        { "id": "instance.settings.set", "instance": "@1", "input": { "java_path": "/bin/sh" } },
    ]);
    let err = plan_steps(&state, &on_new, &paired).await.unwrap_err();
    assert_eq!((err.status, err.code, err.data["step"].as_i64()), (403, "remote_field_blocked", Some(2)), "{err:?}");
    assert_eq!(err.data["fields"], json!(["java_path"]));
    // 그 모듈을 찾지 못하면 설정 칸은 모두 막는다
    let unknown = json!([
        { "id": "instance.create", "input": { "name": "new-srv", "module_name": "missing" } },
        { "id": "instance.settings.set", "instance": "@1", "input": { "players": 6 } },
    ]);
    let err = plan_steps(&state, &unknown, &paired).await.unwrap_err();
    assert_eq!((err.code, err.data["step"].as_i64()), ("remote_field_blocked", Some(2)), "{err:?}");
    // 액션이 아닌 안쪽 호출도 같은 층을 지난다
    let res = call_as(&state, "GET", "/api/updates/status", None, Some(remote.clone())).await;
    assert_eq!((res.status.as_u16(), res.body["error_code"].as_str()), (403, Some("remote_local_only")), "{:?}", res.body);
    // 원격 포트가 받는 라우트는 그대로, 이 PC의 화면에서는 그대로
    assert!(plan(&state, "server.status", Some(&id), &json!({}), &paired).await.is_ok());
    assert!(plan(&state, "update.apply", None, &json!({}), &gui()).await.is_ok());
    let res = call_as(&state, "GET", "/api/updates/status", None, Some(crate::audit::Origin::new("gui"))).await;
    assert_ne!(res.body["error_code"].as_str(), Some("remote_local_only"), "{:?}", res.body);
}

#[tokio::test]
async fn update_apply_from_discord_needs_the_owner_even_with_an_override() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let err = plan(&state, "update.apply", None, &json!({}), &relay("admin", false)).await.unwrap_err();
    assert_eq!(err.code, "PERMISSION_DENIED");
    assert_eq!(err.data["args"]["need"], 3);
    assert!(plan(&state, "update.apply", None, &json!({}), &relay("owner", false)).await.is_ok());
}

#[tokio::test]
async fn plans_belong_to_their_requester_and_keep_the_chatbot_cap() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2, "members": { "admin": 2 } })).await;

    // 관리 액션은 릴레이에서 계획 없이 실행하지 않는다
    let err = run(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), None, &relay("admin", true)).await.unwrap_err();
    assert_eq!(err.code, "confirm_required");
    let made = plan(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("admin", true)).await.unwrap();
    assert_eq!(made["needsConfirm"], true);
    let plan_id = made["planId"].as_str().unwrap().to_string();
    let err = run(&state, "", None, &Value::Null, Some(&plan_id), &relay("someone", false)).await.unwrap_err();
    assert_eq!(err.code, "plan_not_yours");

    // 챗봇이 만든 계획: 사람이 확인 버튼을 눌러도 챗봇 상한을 쓴다
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 0, "members": { "admin": 2 } })).await;
    let err = run(&state, "", None, &Value::Null, Some(&plan_id), &pressed("admin")).await.unwrap_err();
    assert_eq!(err.code, "PERMISSION_DENIED");
    assert_eq!(err.data["args"]["via"], "chatbot");
}

/// 챗봇이 만든 계획은 확인한 쪽이 via를 보내지 않아도 챗봇 출처로 실행된다 (보관 백업 수 줄이기 같은 챗봇 제한이 그 출처를 본다)
#[tokio::test]
async fn a_chatbot_plan_runs_as_the_chatbot_whoever_confirms_it() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let lower = with_revision(&state, &id, json!({ "schedule": [], "backup_keep": 1 })).await;
    let made = plan(&state, "instance.schedule.set", Some(&id), &lower, &relay("owner", true)).await.unwrap();
    let plan_id = made["planId"].as_str().unwrap().to_string();
    let ran = run(&state, "", None, &Value::Null, Some(&plan_id), &pressed("owner")).await.unwrap();
    assert_eq!(ran["success"], false, "{ran}");
    assert_eq!(ran["status"], 400, "{ran}");
}

/// 챗봇이 만든 확인 계획은 요청자 본인이 확인 버튼을 눌렀을 때만 실행된다 (`confirmedBy`). 막힌 시도는 계획을 쓰지 않는다
#[tokio::test]
async fn a_chatbot_plan_that_needs_confirmation_runs_only_from_the_requesters_button() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let made = plan(&state, "instance.settings.set", Some(&id), &json!({ "players": 4 }), &relay("owner", true)).await.unwrap();
    assert_eq!(made["needsConfirm"], true);
    let plan_id = made["planId"].as_str().unwrap();
    let mut someone_else = relay("owner", false);
    someone_else.remote.as_mut().unwrap().confirmed_by = Some("admin".into());
    for caller in [relay("owner", true), relay("owner", false), someone_else] {
        let err = run(&state, "", None, &Value::Null, Some(plan_id), &caller).await.unwrap_err();
        assert_eq!((err.status, err.code), (409, "confirm_required"));
    }
    assert_eq!(run(&state, "", None, &Value::Null, Some(plan_id), &pressed("owner")).await.unwrap()["success"], true);

    // 확인이 필요 없는 챗봇 계획과 사람이 직접 만든 계획은 그대로
    let quick = plan(&state, "server.status", Some(&id), &json!({}), &relay("owner", true)).await.unwrap();
    assert_eq!(quick["needsConfirm"], false);
    assert!(run(&state, "", None, &Value::Null, quick["planId"].as_str(), &relay("owner", true)).await.is_ok());
    let direct = plan(&state, "discord.settings.set", None, &json!({ "guild": "g", "sleeping": true }), &relay("owner", false)).await.unwrap();
    assert!(run(&state, "", None, &Value::Null, direct["planId"].as_str(), &relay("owner", false)).await.is_ok());
}

#[tokio::test]
async fn direct_admin_commands_skip_confirmation_but_chatbot_and_danger_do_not() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let sleep = json!({ "guild": "g", "sleeping": true });
    let direct = plan(&state, "discord.settings.set", None, &sleep, &relay("owner", false)).await.unwrap();
    assert_eq!(direct["needsConfirm"], false);
    let by_bot = plan(&state, "instance.settings.set", Some(&id), &json!({ "max_players": 4 }), &relay("owner", true)).await.unwrap();
    assert_eq!(by_bot["needsConfirm"], true);
    let restore = plan(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("owner", true)).await.unwrap();
    assert_eq!(restore["needsConfirm"], true);
}

#[tokio::test]
async fn inputs_are_checked_and_actions_run_through_the_real_routes() {
    let (state, id, _tmp) = server().await;
    let err = plan(&state, "backup.restore", Some(&id), &json!({ "nme": "b" }), &gui()).await.unwrap_err();
    assert_eq!((err.code, err.data["field"].as_str()), ("invalid_input", Some("nme")));
    let err = plan(&state, "server.start", None, &json!({}), &gui()).await.unwrap_err();
    assert_eq!(err.data["field"], "instance");

    let listed = run(&state, "instance.list", None, &json!({}), None, &gui()).await.unwrap();
    assert_eq!(listed["success"], true);
    assert_eq!(listed["result"]["servers"][0]["name"], "srv");
    let status = run(&state, "server.status", Some(&id), &json!({}), None, &gui()).await.unwrap();
    assert_eq!(status["status"], 200, "{}", status);

    // 읽기 전용 조회는 query로만
    assert_eq!(query(&state, "server.start", Some(&id), &json!({}), &relay("owner", false)).await.unwrap_err().code, "not_a_query");
}

#[test]
fn heartbeat_actions_stay_within_the_limits_and_titles_are_localized() {
    let (actions, truncated) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    assert!(!truncated);
    assert!(actions.len() <= HEARTBEAT_MAX_ACTIONS);
    assert!(serde_json::to_vec(&actions).unwrap().len() <= HEARTBEAT_MAX_BYTES);
    let start = actions.iter().find(|a| a["id"] == "server.start").unwrap();
    assert_eq!(start["title"]["ko"], "서버 시작");
    assert_eq!(start["title"]["en"], "Start server");
    // 디스코드·챗봇에 없는 액션은 싣지 않는다
    assert!(actions.iter().all(|a| a["id"] != "instance.delete" && a["id"] != "extension.enable"));
}

// 릴레이는 heartbeat 액션 표의 `controllers`로 조작 가능 명단이 통과시키는 액션을 안다 (권한 계단과 같은 넷)
#[test]
fn heartbeat_actions_mark_the_controlled_actions() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    for a in &actions {
        let controlled = crate::controllers::CONTROLLED_ACTIONS.contains(&a["id"].as_str().unwrap_or_default());
        assert_eq!(a.get("controllers") == Some(&json!(true)), controlled, "{}", a["id"]);
    }
    for id in crate::controllers::CONTROLLED_ACTIONS {
        assert!(actions.iter().any(|a| a["id"] == id), "{id} is in the heartbeat");
    }
}

// 사용자 요청(2026-10-24): 서버 슬래시 루트 아래 이름은 "서버-상태"가 아니라 "상태"처럼 짧게. 제목은 그대로 두고
// 짧은 이름(`actions.<id>.name`)을 따로 싣는다. 이름이 없는 액션은 `name` 키가 없다
#[test]
fn heartbeat_actions_carry_short_names_for_the_server_root() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = |id: &str| actions.iter().find(|a| a["id"] == id).unwrap().clone();
    for (id, ko, en) in [("server.status", "상태", "Status"), ("server.start", "시작", "Start"), ("server.stop", "정지", "Stop"), ("server.restart", "재시작", "Restart")] {
        let a = item(id);
        assert_eq!(a["name"]["ko"], ko, "{id}");
        assert_eq!(a["name"]["en"], en, "{id}");
        assert_eq!(a["name"].as_object().unwrap().len(), crate::chat::i18n::languages().len(), "{id} in every locale");
        assert!(a["title"]["ko"].as_str().unwrap().starts_with("서버"), "{id} title stays");
    }
    assert!(item("backup.create").get("name").is_none());
}

// 실사용: 디스코드 "/작업"에 길드 설정 작업이 "guild" 필수 칸과 함께 보였다. 길드 설정은 "/사바쨩" 전용 명령이
// 같은 액션을 부르므로 메뉴에는 싣지 않고, 원격 요청이 스스로 채우는 길드 칸은 어느 메뉴 입력에도 두지 않는다
#[test]
fn the_remote_menu_has_no_guild_settings_and_no_guild_input() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    for id in ["chatbot.settings.get", "chatbot.settings.set", "discord.settings.set"] {
        assert!(actions.iter().all(|a| a["id"] != id), "{id} is served by the /사바쨩 commands");
        assert!(find(id).unwrap().surfaces.contains(&Discord), "{id} stays callable from Discord");
    }
    for a in &actions {
        assert!(a["input"]["properties"].get(GUILD_FIELD).is_none(), "{} asks for a guild", a["id"]);
        assert!(a["input"]["required"].as_array().is_none_or(|r| !r.iter().any(|f| f == GUILD_FIELD)), "{}", a["id"]);
    }
}

// 사용자 디스코드 검수(2026-10-20): 시작·정지 기록(3)은 디스코드에서 빼고, 백업 목록(4)은 디스코드 메뉴에서만 뺀다
// (챗봇 복원이 목록을 보고, 디스코드에서 부르는 것은 계속 받는다). 쓰는 곳 없는 길드 설정 조회(12)는 없앤다.
// 백업 만들기(5)의 이름은 "백업"
#[test]
fn the_discord_review_trims_the_remote_menu() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = |id: &str| actions.iter().find(|a| a["id"] == id).unwrap().clone();
    assert!(!find("instance.events").unwrap().surfaces.contains(&Discord));
    assert_eq!(item("instance.events")["surfaces"], json!(["chatbot"]));
    assert_eq!(item("backup.list")["surfaces"], json!(["chatbot"]), "off the Discord menu, still a chatbot tool");
    assert!(find("backup.list").unwrap().surfaces.contains(&Discord), "Discord can still call it");
    assert!(find("discord.settings.get").is_none());
    assert_eq!(item("backup.create")["title"]["ko"], "백업");
    assert_eq!(item("backup.create")["title"]["en"], "Backup");
}

// 릴레이 사전 검사가 데몬 판정과 같은 답을 내게, 고정 레벨이 있는 액션은 그 레벨을 싣는다
#[test]
fn remote_menu_items_carry_their_fixed_level() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = |id: &str| actions.iter().find(|a| a["id"] == id).unwrap().clone();
    assert_eq!(item("update.apply")["level"], crate::chat::permissions::OWNER);
    assert!(item("server.start").get("level").is_none());
}

// 조회 나만 보기는 부른 사람만 볼 진단·관리 조회만 숨긴다 — 서버 상태·접속자는 다 같이 본다 (2026-10-31 사용자 결정)
#[test]
fn only_diagnostic_queries_are_marked_private() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let private = |id: &str| actions.iter().find(|a| a["id"] == id).unwrap().get("private").cloned();
    assert_eq!(private("server.port_check"), Some(json!(true)));
    assert_eq!(private("monitor.summary"), Some(json!(true)));
    assert_eq!(private("server.status"), None);
    assert_eq!(private("server.players"), None);
    assert!(find("backup.list").unwrap().private);
}

// 원격 업데이트(디스코드 `/사바쨩 업데이트`)는 한 번에 받고·적용하고·필요하면 업데이터까지 띄운다. 주인만, 말하는 사바쨩은 못 부른다
#[test]
fn remote_update_installs_in_one_step_for_the_owner_only() {
    let def = find("update.apply").unwrap();
    assert_eq!((def.method, def.path), ("POST", "/api/updates/install"));
    assert_eq!((def.discord_level, def.risk), (Some(crate::chat::permissions::OWNER), "danger"));
    assert!(def.surfaces.contains(&Discord) && !def.surfaces.contains(&Chatbot));
}

// 실사용: 디스코드 입력칸이 "name"·"backup_keep" 같은 내부 이름으로 보였다. 칸마다 언어별 제목을 싣는다
#[test]
fn remote_inputs_carry_localized_field_titles() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let field = |id: &str, name: &str| actions.iter().find(|a| a["id"] == id).unwrap()["input"]["properties"][name]["title"].clone();
    assert_eq!((field("backup.restore", "name")["ko"].as_str(), field("backup.restore", "name")["en"].as_str()), (Some("이름"), Some("Name")));
    assert_eq!(field("instance.schedule.set", "backup_keep")["ko"], "보관할 백업 수");
    assert_eq!(field("instance.schedule.set", "backup_keep").as_object().unwrap().len(), crate::chat::i18n::languages().len());
}

// 상한을 넘으면 칸 제목부터 빼서 액션을 덜 잃는다
#[test]
fn field_titles_go_first_when_the_heartbeat_is_too_big() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("big");
    std::fs::create_dir_all(&dir).unwrap();
    let mut toml = String::from("[module]\nschema_version = 1\nname = \"big\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n");
    let label = "x".repeat(120);
    for q in 0..30 {
        toml.push_str(&format!("\n[[queries]]\nname = \"q{q:02}\"\nfunction = \"f\"\ninputs = [{{ name = \"a\", label = \"{label}\" }}, {{ name = \"b\", label = \"{label}\" }}]\n"));
    }
    std::fs::write(dir.join("module.toml"), toml).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    let modules = vec![crate::module_loader::load_module_dir(&dir).unwrap()];
    let (actions, truncated) = heartbeat_actions(&crate::chat::i18n::languages(), &modules);
    assert!(!truncated, "every action still fits without field titles");
    assert!(serde_json::to_vec(&actions).unwrap().len() <= HEARTBEAT_MAX_BYTES);
    assert!(actions.iter().all(|a| a["input"]["properties"].as_object().is_none_or(|p| p.values().all(|f| f.get("title").is_none()))));
}

// 실사용: 디스코드에 결과가 JSON 그대로 보였다. 원격 결과에는 요청 언어의 사람용 요약을 붙인다
#[tokio::test]
async fn remote_results_carry_a_readable_summary() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let listed = query(&state, "instance.list", None, &json!({}), &relay("someone", false)).await.unwrap();
    let text = shown(&listed["text"]);
    assert!(text.contains("name=srv") && text.contains("status.stopped"), "{text}");
    assert!(shown(&query(&state, "backup.list", Some(&id), &json!({}), &relay("someone", false)).await.unwrap()["text"]).contains("backups_empty"));
    assert!(shown(&query(&state, "instance.events", Some(&id), &json!({}), &relay("someone", true)).await.unwrap()["text"]).contains("events_empty"));
    assert!(shown(&query(&state, "monitor.summary", None, &json!({}), &relay("someone", false)).await.unwrap()["text"]).contains("actions.summary.monitor cpu="));
    // GUI·CLI는 자기 화면으로 그린다
    assert!(run(&state, "instance.list", None, &json!({}), None, &gui()).await.unwrap().get("text").is_none());
}

/// 결과에 실린 답(`Text` JSON)을 `[키 이름=값]`으로 펼친 글
fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(shown).collect(),
        Value::Object(m) => {
            let vars: String = m.get("vars").and_then(Value::as_object).into_iter().flatten().map(|(k, v)| format!(" {k}={}", shown(v))).collect();
            format!("[{}{}]", m.get("key").and_then(Value::as_str).unwrap_or_default(), vars)
        }
        _ => String::new(),
    }
}

/// 요약은 문구 키와 값으로 간다 (문장은 릴레이가 만든다). `plain()`은 `[키 이름=값]`으로 펼친 글이다
fn summary_of(id: &str, result: Value) -> Option<String> {
    summary::text(id, &result, &crate::chat::i18n::Catalog::for_language("ko")).map(|t| t.plain())
}

#[test]
fn summaries_read_like_sentences() {
    let ko = crate::chat::i18n::Catalog::for_language("ko");
    let backups: Vec<Value> = (0..12).map(|n| json!({ "name": format!("b{n:02}"), "kind": "scheduled", "created": 1_700_000_000 + n * 60, "size": 1_536 * 1024 * 1024 })).collect();
    let text = summary_of("backup.list", json!({ "success": true, "backups": backups })).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("count=12"), "{text}");
    // 백업 종류 이름은 데몬 문구다 (복원 칸 이름표와 같다)
    assert!(lines[1].contains("b11") && lines[1].contains("예약") && lines[1].contains("1.5 GB"), "newest first: {text}");
    assert!(lines.len() <= 12 && lines.last().unwrap().contains("[actions.summary.more count=2]"), "the rest is counted: {text}");

    let updates = json!({ "ok": true, "updates_available": 1, "components": [
        { "display_name": "Saba-chan GUI", "current_version": "0.1.7", "latest_version": "0.1.8", "update_available": true },
        { "display_name": "Saba-chan CLI", "current_version": "0.1.7", "latest_version": "0.1.7", "update_available": false },
    ] });
    let text = summary_of("update.check", updates).unwrap();
    assert!(text.contains("Saba-chan GUI") && text.contains("current=0.1.7 latest=0.1.8") && !text.contains("CLI"), "{text}");
    assert!(summary_of("update.check", json!({ "ok": true, "updates_available": 0, "components": [] })).unwrap().contains("updates_none"));

    let events = json!({ "events": [{ "at": 1_700_000_000, "kind": "crash" }, { "at": 1_700_000_600, "kind": "user_stop" }] });
    let text = summary_of("instance.events", events).unwrap();
    assert!(text.lines().nth(1).unwrap().contains("event_kind.user_stop") && text.contains("event_kind.crash"), "newest first: {text}");

    // 시각을 모르면 1970년으로 찍지 않는다
    let unknown = summary_of("backup.list", json!({ "backups": [{ "name": "old", "kind": "manual", "size": 0 }] })).unwrap();
    assert!(!unknown.contains("1970"), "{unknown}");

    // 게임 자동 백업은 크기를 재지 않는다: 크기 자리가 없는 문구로
    let game = summary_of("backup.list", json!({ "backups": [{ "name": "game-snapshot:S1", "kind": "game-snapshot", "created": 1_700_000_000 }] })).unwrap();
    assert!(game.contains("backup_item_unsized") && !game.contains("size="), "{game}");

    // 열지 못하는 백업은 복원할 수 없다고 함께 알린다 (603 P3-8)
    let mut broken = json!({ "backups": [{ "name": "bad", "kind": "manual", "created": 1_700_000_000, "size": 9, "readable": false }, { "name": "ok", "kind": "manual", "created": 1_699_000_000, "size": 9, "readable": true }] });
    let text = summary_of("backup.list", broken.clone()).unwrap();
    assert!(text.lines().nth(1).unwrap().contains("열 수 없음") && !text.lines().nth(2).unwrap().contains("열 수 없음"), "{text}");
    summary::label_backups(&mut broken, &ko);
    assert!(broken["backups"][0]["label"].as_str().unwrap().contains("열 수 없음") && !broken["backups"][1]["label"].as_str().unwrap().contains("열 수 없음"), "{broken}");

    // 모듈 조회는 범용 포맷으로 (모양이 모듈마다 다르다)
    let players = summary_of("server.query:players", json!({ "success": true, "players": [{ "name": "steve" }] })).unwrap();
    assert!(players.contains("steve") && !players.contains("success"), "{players}");
    // 모르는 액션은 요약하지 않는다 (릴레이가 결과를 그대로 보인다)
    assert!(summary_of("server.status", json!({})).is_none());
}

#[test]
fn chatbot_never_lists_deletion_shutdown_update_or_permissions() {
    for d in ACTIONS.iter().filter(|d| d.surfaces.contains(&Chatbot)) {
        for word in ["delete", "shutdown", "update", "permission", "extension"] {
            assert!(!d.id.contains(word), "{} must not be open to the chatbot", d.id);
        }
    }
}

/// 모듈의 위험 명령도 챗봇이 고를 수 있다. 물을지는 계획의 `needs_confirm`이 길드 챗봇 `confirmLevel`로 정한다 (사용자 결정 2026-10-28).
/// 결과는 콘솔·조회처럼 IP·비밀처럼 보이는 값을 가려 보낸다
#[test]
fn danger_module_commands_reach_the_chatbot_only_through_a_confirmation() {
    let wipe = command_view(&json!({ "name": "wipe", "risk": "danger" }), &Value::Null).unwrap();
    assert!(wipe["surfaces"].as_array().unwrap().iter().any(|s| s == "chatbot"));
    assert_eq!(wipe["risk"], "danger", "the plan decides whether to ask (needs_confirm)");
    for id in ["server.console.tail", "server.query:players", "server.command:kick"] {
        assert!(chatbot_masked(id), "{id}");
    }
    assert!(!chatbot_masked("server.status"));
}

#[tokio::test]
async fn remote_results_never_carry_secrets() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    for caller in [relay("someone", false), relay("someone", true)] {
        let listed = query(&state, "instance.list", None, &json!({}), &caller).await.unwrap();
        let text = listed.to_string();
        assert!(listed["result"]["servers"][0]["name"] == "srv" && !text.contains("S3CRET"), "{}", text);
    }
    // GUI·CLI는 PC 주인이라 그대로
    assert!(run(&state, "instance.list", None, &json!({}), None, &gui()).await.unwrap().to_string().contains("S3CRET-MOD"));
}

#[tokio::test]
async fn remote_settings_cannot_touch_launch_values_at_any_depth() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2 })).await;
    let owner_bot = relay("owner", true);
    let cases = [
        json!({ "executable_path": "x" }),
        json!({ "extra_args": "-x" }),
        json!({ "working_dir": "x" }),
        json!({ "extension_data": { "a": 1 } }),
        json!({ "java_path": "x" }),
        json!({ "motd": "x" }),
        json!({ "module_settings": { "java_path": "x" } }),
        json!({ "module_settings": { "admin_pass": "x" } }),
    ];
    for input in &cases {
        let err = plan(&state, "instance.settings.set", Some(&id), input, &owner_bot).await.unwrap_err();
        assert_eq!(err.code, "invalid_input", "{}", input);
        assert!(plan(&state, "instance.settings.set", Some(&id), input, &gui()).await.is_ok(), "{}", input);
    }
    // 실행 인자여도 형식이 정해진 값(숫자)은 바꿀 수 있다
    let ok = plan(&state, "instance.settings.set", Some(&id), &json!({ "players": 8 }), &owner_bot).await;
    assert!(ok.is_ok(), "{:?}", ok);
    let create = json!({ "name": "n", "module_name": "plain", "module_settings": { "java_path": "x" } });
    assert_eq!(plan(&state, "instance.create", None, &create, &owner_bot).await.unwrap_err().code, "invalid_input");
    // 런타임 경로 설정(`runtime`)은 모듈이 글자 형식으로 선언해도 막는다
    let (state, id, _tmp) = server_with(&format!("{MODULE}\n[[settings.fields]]\nname = \"jre\"\ntype = \"text\"\nlabel = \"jre\"\nruntime = \"java\"\n")).await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2 })).await;
    assert_eq!(plan(&state, "instance.settings.set", Some(&id), &json!({ "jre": "x" }), &owner_bot).await.unwrap_err().code, "invalid_input");
    assert!(plan(&state, "instance.settings.set", Some(&id), &json!({ "jre": "x" }), &gui()).await.is_ok());
}

#[tokio::test]
async fn guilds_only_reach_their_allowed_instances() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "allowedInstances": ["other"] })).await;
    let owner = relay("owner", false);
    let err = query(&state, "server.status", Some(&id), &json!({}), &owner).await.unwrap_err();
    assert_eq!(err.code, "NODE_NOT_ALLOWED");
    assert_eq!(plan(&state, "server.stop", Some(&id), &json!({}), &owner).await.unwrap_err().code, "NODE_NOT_ALLOWED");
    let listed = query(&state, "instance.list", None, &json!({}), &owner).await.unwrap();
    assert_eq!(listed["result"]["servers"], json!([]));

    set_node(&state, json!({ "ownerDiscordId": "owner", "allowedInstances": [id.clone()] })).await;
    assert!(query(&state, "server.status", Some(&id), &json!({}), &owner).await.is_ok());
    let listed = query(&state, "instance.list", None, &json!({}), &owner).await.unwrap();
    assert_eq!(listed["result"]["servers"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn discord_settings_stay_in_the_guild_that_asked_and_need_its_owner() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let owner = relay("owner", false);
    let input = json!({ "locale": "ko" });
    // 길드를 주지 않으면 요청이 온 길드로
    let planned = plan(&state, "discord.settings.set", None, &input, &owner).await.unwrap();
    assert_eq!(planned["actionId"], json!("discord.settings.set"));
    let other = json!({ "guild": "other", "locale": "ko" });
    assert_eq!(plan(&state, "discord.settings.set", None, &other, &owner).await.unwrap_err().code, "invalid_input");
    // 관리자 레벨이어도 주인만
    assert_eq!(plan(&state, "discord.settings.set", None, &input, &relay("admin", false)).await.unwrap_err().code, "PERMISSION_DENIED");
    // 챗봇에는 열지 않는다
    assert_eq!(plan(&state, "discord.settings.set", None, &input, &relay("owner", true)).await.unwrap_err().code, "action_not_allowed");
    // GUI·CLI는 길드를 직접 고른다
    assert_eq!(plan(&state, "discord.settings.set", None, &input, &gui()).await.unwrap_err().code, "invalid_input");
    assert!(plan(&state, "discord.settings.set", None, &other, &gui()).await.is_ok());
    // 읽어주는 사바쨩 볼륨·속도·피치도 같은 액션으로 (/사바쨩 읽어주기)
    assert!(plan(&state, "discord.settings.set", None, &json!({ "tts": { "volume": 80 } }), &owner).await.is_ok());
}

/// 원격 데몬 R2b: 디스코드 서버 주인(릴레이 `guildOwnerDiscordId`)도 길드 공통 설정은 주인으로 본다. 다른 주인 액션은 아니다
#[tokio::test]
async fn the_discord_server_owner_may_change_guild_settings_only() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let mut boss = relay("boss", false);
    boss.remote.as_mut().unwrap().guild_owner_id = Some("boss".into());
    let input = json!({ "locale": "ko" });
    assert!(plan(&state, "discord.settings.set", None, &input, &boss).await.is_ok());
    assert_eq!(plan(&state, "chatbot.settings.set", None, &json!({ "memo": "x" }), &boss).await.unwrap_err().code, "PERMISSION_DENIED");
    assert_eq!(plan(&state, "update.apply", None, &json!({}), &boss).await.unwrap_err().code, "PERMISSION_DENIED");
    // 서버 주인이 다른 사람이면 그대로
    let mut someone = relay("admin", false);
    someone.remote.as_mut().unwrap().guild_owner_id = Some("boss".into());
    assert_eq!(plan(&state, "discord.settings.set", None, &input, &someone).await.unwrap_err().code, "PERMISSION_DENIED");
}

#[tokio::test]
async fn console_input_to_a_module_without_stdin_is_refused() {
    use axum::response::IntoResponse;
    let (state, id, _tmp) = server().await;
    let resp = crate::ipc::handlers::managed::send_stdin_handler(
        axum::extract::Path(id),
        axum::extract::State(state),
        axum::Json(json!({ "command": "say hi" })),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error_code"], "stdin_not_supported");
}

#[test]
fn chatbot_console_hides_addresses_and_secret_looking_values() {
    let mut v = json!({ "lines": [
        "[12:34:56 INFO]: Player joined from 203.0.113.7:51234",
        "[12:34:57 INFO]: peer 2001:db8::1 and fe80::1ff:fe23:4567:890a",
        "rcon password=hunter2 token: abc",
        "key a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8",
        "Done (12.3s)! For help, type \"help\"",
    ]});
    mask_console(&mut v);
    let lines: Vec<&str> = v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert_eq!(lines[0], "[12:34:56 INFO]: Player joined from x.x.x.x:51234");
    assert_eq!(lines[1], "[12:34:57 INFO]: peer x:x::x and x:x::x");
    assert_eq!(lines[2], "rcon password=*** token: ***");
    assert_eq!(lines[3], "key ***");
    assert_eq!(lines[4], "Done (12.3s)! For help, type \"help\"");
}

#[test]
fn remote_module_errors_hide_stderr_and_sensitive_text_at_every_depth() {
    let mut body = json!({ "result": [{
        "success": false, "error_code": "module_exception", "module": "plain", "function": "install_server",
        "error": "peer 203.0.113.7 token=hunter2", "message": "failed at C:\\Users\\me\\server.py",
        "path": "/home/me/server", "stderr_tail": ["traceback with private data"],
        "details": { "stderr_tail": ["more private data"] }
    }] });
    strip_local_details(&mut body);
    let error = &body["result"][0];
    assert!(error.get("stderr_tail").is_none());
    assert!(error["details"].get("stderr_tail").is_none());
    assert_eq!(error["error"], "peer x.x.x.x token=***");
    assert_eq!(error["message"], "failed at <path>");
    assert_eq!(error["path"], "<path>");
    assert_eq!(error["module"], "plain");
    assert_eq!(error["function"], "install_server");
    assert_eq!(error["error_code"], "module_exception");
}

#[test]
fn remote_module_errors_hide_single_component_absolute_paths() {
    let mut error = json!({ "success": false, "module": "plain", "function": "read", "error_code": "game_failed", "error": "open /tmp", "message": "open '/private' failed", "path": "/etc", "details": { "path": "/root" } });
    strip_local_details(&mut error);
    assert_eq!(error["error"], "open <path>");
    assert_eq!(error["message"], "open '<path>' failed");
    assert_eq!(error["path"], "<path>");
    assert_eq!(error["details"]["path"], "<path>");
    assert_eq!(error["error_code"], "game_failed");
    let mut normal = json!({ "success": true, "message": "Use /help for commands" });
    strip_local_details(&mut normal);
    assert_eq!(normal["message"], "Use /help for commands");
}

#[tokio::test]
async fn module_queries_are_safe_read_actions_on_every_surface() {
    let (state, id, tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner" })).await;
    let action = "server.query:players";

    // 인스턴스를 주면 모듈 locales 문구로 펼쳐진다 (없는 언어는 module.toml label)
    let listed = list(&state, Some(Chatbot), Some(&id), "ko").await;
    let q = listed["actions"].as_array().unwrap().iter().find(|a| a["id"] == action).cloned().unwrap();
    assert_eq!((q["kind"].as_str(), q["risk"].as_str(), q["title"].as_str()), (Some("query"), Some("safe"), Some("접속자 목록")));
    assert_eq!(q["surfaces"], json!(["gui", "cli", "discord", "chatbot"]));
    assert_eq!(q["input"]["properties"]["limit"]["type"], "number");
    let en = list(&state, None, Some(&id), "en").await;
    assert_eq!(en["actions"].as_array().unwrap().iter().find(|a| a["id"] == action).unwrap()["title"], "Players");

    // 챗봇·일반 사용자도 계획 없이 부를 수 있는 조회다
    let planned = plan(&state, action, Some(&id), &json!({ "limit": 5 }), &relay("someone", true)).await.unwrap();
    assert_eq!(planned["needsConfirm"], false);
    // 입력은 선언대로
    for bad in [json!({ "limit": 50 }), json!({ "limit": "x" }), json!({ "lmit": 5 })] {
        assert_eq!(plan(&state, action, Some(&id), &bad, &gui()).await.unwrap_err().code, "invalid_input", "{}", bad);
    }
    assert_eq!(plan(&state, action, None, &json!({}), &gui()).await.unwrap_err().data["field"], "instance");
    assert_eq!(query(&state, "server.query:nope", Some(&id), &json!({}), &relay("owner", false)).await.unwrap_err().code, "unknown_action");

    // 라우트: 모르는 조회는 404, 입력 검사는 라우트도 한다
    let resp = call(&state, "POST", &format!("/api/instance/{}/query/nope", id), None).await;
    assert_eq!((resp.status.as_u16(), resp.body["error_code"].as_str()), (404, Some("unknown_query")));
    let resp = call(&state, "POST", &format!("/api/instance/{}/query/players", id), Some(json!({ "args": { "limit": 99 } }))).await;
    assert_eq!((resp.status.as_u16(), resp.body["error_code"].as_str()), (400, Some("invalid_argument")));

    // heartbeat: 고정 액션 뒤에 모듈 조회가 모듈 이름과 함께 실린다
    let modules = vec![crate::module_loader::load_module_dir(&tmp.path().join("modules").join("plain")).unwrap()];
    let (actions, truncated) = heartbeat_actions(&crate::chat::i18n::languages(), &modules);
    assert!(!truncated);
    let q = actions.iter().find(|a| a["id"] == action).unwrap();
    assert_eq!((q["module"].as_str(), q["kind"].as_str()), (Some("plain"), Some("query")));
    assert_eq!((q["title"]["ko"].as_str(), q["title"]["en"].as_str()), (Some("접속자 목록"), Some("Players")));
    assert_eq!(q["description"]["ko"], "지금 접속한 사람");
    // 입력 칸 이름도 모듈 locales 문구로 (없는 언어는 module.toml label, 그것도 없으면 싣지 않는다)
    assert_eq!(q["input"]["properties"]["limit"]["title"], json!({ "ko": "최대 인원" }));
    assert!(serde_json::to_vec(&actions).unwrap().len() <= HEARTBEAT_MAX_BYTES);
}

/// 느린 러너에서는 계획 실행이 뒤로 넘어간다(`running`): 끝날 때까지 보고 결과를 돌려준다
async fn finished(mut done: Value, caller: &Caller) -> Value {
    let waited = std::time::Instant::now();
    while done["running"] == true {
        assert!(waited.elapsed() < std::time::Duration::from_secs(180), "the plan never finished: {}", done);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let status = run_status(done["runId"].as_str().unwrap(), caller).unwrap();
        if status["done"] == true {
            done = status["result"].clone();
        }
    }
    done
}

/// 같은 길드에서 여러 단계 계획 둘을 동시에 눌러도, 409를 받은 계획은 쓰이지 않고 남아 다시 누를 수 있다 (D-F6)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_refused_because_another_runs_can_be_pressed_again() {
    let (state, id, _tmp) = server().await;
    // 자리와 실행 횟수는 길드마다 전역이다: 다른 시험과 겹치지 않는 길드를 쓴다
    let guild = "steps-lane-race";
    state
        .config_store
        .update_node_settings(guild, |n| {
            *n = json!({ "ownerDiscordId": "owner", "chatbotCap": 2, "adminRunsPerHour": 100000 }).as_object().cloned().unwrap();
            ((), true)
        })
        .await
        .unwrap();
    let who = |chatbot| Requester { guild_id: Some(guild.into()), user_id: Some("owner".into()), owner_id: Some("owner".into()), via_chatbot: chatbot, confirmed_by: (!chatbot).then(|| "owner".into()), ..Default::default() };
    let caller = |chatbot| Caller { surface: if chatbot { Chatbot } else { Discord }, remote: Some(who(chatbot)), origin: crate::audit::Origin::new("discord"), locale: "en".into() };
    let owner = caller(false);
    for round in 0..40 {
        let steps = json!([{ "id": "instance.settings.set", "instance": id, "input": { "players": 4 + round % 2 } }]);
        let mut ids = Vec::new();
        for _ in 0..2 {
            ids.push(plan_steps(&state, &steps, &caller(true)).await.unwrap()["planId"].as_str().unwrap().to_string());
        }
        let runs: Vec<_> = ids
            .iter()
            .map(|pid| {
                let (state, pid, owner) = (state.clone(), pid.clone(), owner.clone());
                tokio::spawn(async move { run(&state, steps::PLAN_ID, None, &Value::Null, Some(&pid), &owner).await })
            })
            .collect();
        let mut refused = Vec::new();
        for (pid, r) in ids.iter().zip(runs) {
            match r.await.unwrap() {
                Ok(started) => {
                    finished(started, &owner).await;
                }
                Err(e) => {
                    assert_eq!(e.code, "plan_running", "round {round}");
                    refused.push(pid.clone());
                }
            }
        }
        for pid in refused {
            // 앞 실행이 자리를 놓을 때까지는 409가 날 수 있다. 계획이 사라졌으면(plan_expired) 버그다
            let mut again = run(&state, steps::PLAN_ID, None, &Value::Null, Some(&pid), &owner).await;
            for _ in 0..100 {
                match &again {
                    Err(e) if e.code == "plan_running" => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        again = run(&state, steps::PLAN_ID, None, &Value::Null, Some(&pid), &owner).await;
                    }
                    _ => break,
                }
            }
            let started = again.unwrap_or_else(|e| panic!("round {round}: the refused plan was used up: {}", e.code));
            finished(started, &owner).await;
        }
    }
}

#[tokio::test]
async fn a_plan_of_steps_makes_a_server_and_uses_it_with_one_confirmation() {
    let (state, _id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2, "allowedInstances": ["other"] })).await;
    let steps = json!([
        { "id": "instance.create", "input": { "name": "new-srv", "module_name": "plain" } },
        { "id": "instance.settings.set", "instance": "@1", "input": { "players": 6 } },
    ]);
    let owner_bot = relay("owner", true);
    let planned = plan_steps(&state, &steps, &owner_bot).await.unwrap();
    assert_eq!((planned["actionId"].as_str(), planned["needsConfirm"].as_bool()), (Some(steps::PLAN_ID), Some(true)));
    let summary = planned["summary"].to_string();
    assert!(summary.contains("1. ") && summary.contains("2. ") && summary.contains("new-srv"), "{}", summary);

    // 새 서버를 쓰는 단계도 원격 검사를 그 모듈로 한다
    let sneaky = json!([steps[0].clone(), { "id": "instance.settings.set", "instance": "@1", "input": { "java_path": "x" } }]);
    let err = plan_steps(&state, &sneaky, &owner_bot).await.unwrap_err();
    assert_eq!((err.code, err.data["field"].as_str(), err.data["step"].as_i64()), ("invalid_input", Some("steps[1].java_path"), Some(2)));
    // 앞의 생성 단계만 가리킬 수 있다. 상태 조회도 id 길이라 새 서버에 쓸 수 있다
    let wrong = json!([{ "id": "server.start", "instance": "@1", "input": {} }]);
    assert_eq!(plan_steps(&state, &wrong, &owner_bot).await.unwrap_err().data["field"], "steps[0].instance");
    let status = json!([steps[0].clone(), { "id": "server.status", "instance": "@1", "input": {} }]);
    assert!(plan_steps(&state, &status, &owner_bot).await.is_ok());

    // 다른 사람은 실행하지 못하고, 요청자가 누르면 순서대로 한 번에
    let plan_id = planned["planId"].as_str().unwrap();
    assert_eq!(run(&state, steps::PLAN_ID, None, &Value::Null, Some(plan_id), &relay("someone", false)).await.unwrap_err().code, "plan_not_yours");
    let owner = pressed("owner");
    let done = finished(run(&state, steps::PLAN_ID, None, &Value::Null, Some(plan_id), &owner).await.unwrap(), &owner).await;
    assert_eq!(done["success"], true, "{}", done);
    let new_id = done["steps"][0]["instance"].as_str().unwrap().to_string();
    assert_eq!(done["steps"][1]["status"], "done");
    // 만든 길드의 허용 목록에 들어가 바로 쓸 수 있다
    assert!(query(&state, "server.status", Some(&new_id), &json!({}), &relay("owner", false)).await.is_ok());
    let made = state.supervisor.read().await.instance_store.get(&new_id).cloned().unwrap();
    assert_eq!(made.module_settings.get("players"), Some(&json!(6)));
}

#[tokio::test]
async fn a_failed_step_stops_the_plan_and_the_rest_is_skipped() {
    let (state, _id, _tmp) = server().await;
    let steps = json!([
        { "id": "instance.create", "input": { "name": "srv2", "module_name": "plain" } },
        { "id": "server.start", "instance": "@1", "input": {} },
        { "id": "backup.create", "instance": "@1", "input": {} },
    ]);
    let planned = plan_steps(&state, &steps, &gui()).await.unwrap();
    // 모듈이 시작 함수를 지원하지 않아 시작이 실패한다
    let done = finished(run(&state, "", None, &Value::Null, planned["planId"].as_str(), &gui()).await.unwrap(), &gui()).await;
    assert_eq!((done["success"].as_bool(), done["stoppedAt"].as_i64()), (Some(false), Some(2)), "{}", done);
    let statuses: Vec<&str> = done["steps"].as_array().unwrap().iter().map(|s| s["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["done", "failed", "skipped"]);
    // 로컬은 오류 원문도 보인다
    assert!(done["steps"][1]["error"].is_string());

}

#[tokio::test]
async fn remote_settings_show_the_old_value_and_can_be_undone() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2 })).await;
    run(&state, "instance.settings.set", Some(&id), &json!({ "players": 2 }), None, &gui()).await.unwrap();
    let owner_bot = relay("owner", true);
    let planned = plan(&state, "instance.settings.set", Some(&id), &json!({ "players": 4 }), &owner_bot).await.unwrap();
    let row = planned["summary"].as_array().unwrap().iter().find(|r| r["labelKey"] == "actions.field.players").cloned().unwrap();
    assert_eq!((row["before"].clone(), row["value"].clone()), (json!(2), json!(4)));

    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &pressed("owner")).await.unwrap();
    let players = |state: &IPCServer| {
        let state = state.clone();
        let id = id.clone();
        async move { state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.get("players").cloned() }
    };
    assert_eq!(players(&state).await, Some(json!(4)));
    // 되돌리기는 같은 요청자만, 한 번
    let undo = done["undoPlanId"].as_str().unwrap();
    assert_eq!(run(&state, "", None, &Value::Null, Some(undo), &relay("admin", false)).await.unwrap_err().code, "plan_not_yours");
    assert_eq!(run(&state, "", None, &Value::Null, Some(undo), &relay("owner", false)).await.unwrap_err().code, "confirm_required", "the chatbot's change is undone from the button too");
    assert_eq!(run(&state, "", None, &Value::Null, Some(undo), &pressed("owner")).await.unwrap()["success"], true);
    assert_eq!(players(&state).await, Some(json!(2)));
    // GUI·CLI는 되돌리기 계획을 만들지 않는다 (자기 화면이 있다)
    assert!(run(&state, "instance.settings.set", Some(&id), &json!({ "players": 3 }), None, &gui()).await.unwrap().get("undoPlanId").is_none());
}

#[tokio::test]
async fn plan_summaries_never_show_stored_or_new_secrets() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 2 })).await;
    run(&state, "instance.settings.set", Some(&id), &json!({ "players": 2 }), None, &gui()).await.unwrap();
    assert_eq!(run(&state, "instance.settings.set", Some(&id), &json!({ "admin_pass": "OLD-PW" }), None, &gui()).await.unwrap()["success"], true);
    let row = |planned: &Value, key: &str| planned["summary"].as_array().unwrap().iter().find(|r| r["labelKey"] == format!("actions.field.{}", key)).cloned().unwrap();
    // 로컬도 요약에는 가린 값만
    let planned = plan(&state, "instance.settings.set", Some(&id), &json!({ "admin_pass": "NEW-PW" }), &gui()).await.unwrap();
    assert_eq!((row(&planned, "admin_pass")["before"].clone(), row(&planned, "admin_pass")["value"].clone()), (json!("***"), json!("***")));
    // 묶음 값의 이전 값은 바꾸는 칸만
    let planned = plan(&state, "instance.settings.set", Some(&id), &json!({ "module_settings": { "players": 4 } }), &gui()).await.unwrap();
    assert_eq!(row(&planned, "module_settings")["before"], json!({ "players": 2 }));
    assert!(!planned.to_string().contains("OLD-"), "{planned}");
    // 원격은 묶음 값을 받지 않는다
    let err = plan(&state, "instance.settings.set", Some(&id), &json!({ "module_settings": { "players": 4 } }), &relay("owner", true)).await.unwrap_err();
    assert_eq!((err.code, err.data["field"].as_str()), ("invalid_input", Some("module_settings")));
}

#[tokio::test]
async fn a_running_plan_does_not_block_single_confirmations_in_the_guild() {
    let (state, id, _tmp) = server().await;
    state
        .config_store
        .update_node_settings("lanes", |n| {
            *n = json!({ "ownerDiscordId": "owner", "chatbotCap": 2 }).as_object().cloned().unwrap();
            ((), true)
        })
        .await
        .unwrap();
    let who = |chatbot| Requester { guild_id: Some("lanes".into()), user_id: Some("owner".into()), owner_id: Some("owner".into()), via_chatbot: chatbot, confirmed_by: (!chatbot).then(|| "owner".into()), ..Default::default() };
    let caller = |chatbot| Caller { surface: if chatbot { Chatbot } else { Discord }, remote: Some(who(chatbot)), origin: crate::audit::Origin::new("discord"), locale: "en".into() };
    let planned = plan(&state, "instance.settings.set", Some(&id), &json!({ "players": 4 }), &caller(true)).await.unwrap();
    let _busy = steps::Lane::take(&caller(false)).unwrap();
    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &caller(false)).await.unwrap();
    assert_eq!(done["success"], true);
}

#[tokio::test]
async fn remote_admin_runs_are_limited_per_guild_and_hour() {
    let (state, id, _tmp) = server().await;
    state
        .config_store
        .update_node_settings("limited", |n| {
            *n = json!({ "ownerDiscordId": "owner", "adminRunsPerHour": 1 }).as_object().cloned().unwrap();
            ((), true)
        })
        .await
        .unwrap();
    let who = Requester { guild_id: Some("limited".into()), user_id: Some("owner".into()), owner_id: Some("owner".into()), via_chatbot: false, ..Default::default() };
    let owner = Caller { surface: Discord, remote: Some(who), origin: crate::audit::Origin::new("discord"), locale: "en".into() };
    let input = json!({ "guild": "limited", "sleeping": false });
    assert!(run(&state, "discord.settings.set", None, &input, None, &owner).await.is_ok());
    let err = run(&state, "discord.settings.set", None, &input, None, &owner).await.unwrap_err();
    assert_eq!((err.code, err.data["args"]["perHour"].as_i64()), ("rate_limited", Some(1)));
    // 읽기와 로컬은 세지 않는다
    assert!(query(&state, "server.status", Some(&id), &json!({}), &owner).await.is_ok());
    assert!(run(&state, "discord.settings.set", None, &input, None, &gui()).await.is_ok());
}

#[tokio::test]
async fn the_pc_owner_sets_the_shared_model_and_key_from_discord_without_the_key_showing_up() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let input = json!({ "model": "m2", "apiKey": "sk-NEW-SECRET-1234" });
    // 내 PC 공통 값이라 PC 주인(소유자)만, 챗봇은 못 바꾼다
    let planned = plan(&state, "chatbot.common.set", None, &input, &relay("owner", false)).await.unwrap();
    assert!(!planned.to_string().contains("NEW-SECRET"), "{planned}");
    assert_eq!(plan(&state, "chatbot.common.set", None, &input, &relay("admin", false)).await.unwrap_err().code, "PERMISSION_DENIED");
    assert_eq!(plan(&state, "chatbot.common.set", None, &input, &relay("owner", true)).await.unwrap_err().code, "action_not_allowed");
    // 모델과 키 말고는 받지 않는다
    let other = json!({ "enabled": true });
    assert_eq!(plan(&state, "chatbot.common.set", None, &other, &relay("owner", false)).await.unwrap_err().code, "invalid_input");

    let done = run(&state, "chatbot.common.set", None, &input, None, &gui()).await.unwrap();
    assert_eq!(done["success"], true);
    assert!(!done.to_string().contains("NEW-SECRET"), "{done}");
    let cfg = state.config_store.get_chatbot();
    assert_eq!((cfg.settings["model"].as_str(), cfg.api_key.as_deref()), (Some("m2"), Some("sk-NEW-SECRET-1234")));
    // 키를 빼면 그대로, 짧은 키·빈 모델은 거절한다
    run(&state, "chatbot.common.set", None, &json!({ "model": "m3" }), None, &gui()).await.unwrap();
    assert_eq!(state.config_store.get_chatbot().api_key.as_deref(), Some("sk-NEW-SECRET-1234"));
    for bad in [json!({ "apiKey": "short" }), json!({ "model": " " })] {
        assert_eq!(run(&state, "chatbot.common.set", None, &bad, None, &gui()).await.unwrap()["success"], false, "{bad}");
    }
    assert_eq!(state.config_store.get_chatbot().settings["model"].as_str(), Some("m3"));
}

#[tokio::test]
async fn chatbot_settings_from_discord_are_the_owners_and_never_the_chatbots() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let input = json!({ "enabled": true, "memo": "사바" });
    let planned = plan(&state, "chatbot.settings.set", None, &input, &relay("owner", false)).await.unwrap();
    assert_eq!(planned["needsConfirm"], false);
    assert_eq!(plan(&state, "chatbot.settings.set", None, &input, &relay("admin", false)).await.unwrap_err().code, "PERMISSION_DENIED");
    assert_eq!(plan(&state, "chatbot.settings.set", None, &input, &relay("owner", true)).await.unwrap_err().code, "action_not_allowed");
    // 키는 액션으로 다루지 않는다
    let key = json!({ "apiKey": "sk-x" });
    assert_eq!(plan(&state, "chatbot.settings.set", None, &key, &relay("owner", false)).await.unwrap_err().code, "invalid_input");
    // 모델은 PC 전체 설정이라 길드 설정 액션의 칸이 아니다
    let model = json!({ "model": "m2" });
    assert_eq!(plan(&state, "chatbot.settings.set", None, &model, &relay("owner", false)).await.unwrap_err().code, "invalid_input");

    let local = json!({ "guild": "g", "confirmLevel": "danger" });
    assert_eq!(run(&state, "chatbot.settings.set", None, &local, None, &gui()).await.unwrap()["success"], true);
    let read = run(&state, "chatbot.settings.get", None, &json!({ "guild": "g" }), None, &gui()).await.unwrap();
    assert_eq!((read["result"]["settings"]["confirmLevel"].as_str(), read["result"]["settings"]["model"].as_str()), (Some("danger"), Some("gpt-6-luna")));
    // null은 그 길드의 값을 지워 기본값으로 되돌린다
    run(&state, "chatbot.settings.set", None, &json!({ "guild": "g", "confirmLevel": null }), None, &gui()).await.unwrap();
    let read = run(&state, "chatbot.settings.get", None, &json!({ "guild": "g" }), None, &gui()).await.unwrap();
    assert_eq!(read["result"]["settings"]["confirmLevel"].as_str(), Some("all"));
    // 토큰 알림 (릴레이 /설정 칸): 한도·잔액 금액, 처음엔 없음(null)
    assert!(read["result"]["settings"]["tokenLimit"].is_null());
    let alerts = json!({ "guild": "g", "tokenLimit": 2_000_000, "balanceAlert": 2.5 });
    assert_eq!(run(&state, "chatbot.settings.set", None, &alerts, None, &gui()).await.unwrap()["success"], true);
    let read = run(&state, "chatbot.settings.get", None, &json!({ "guild": "g" }), None, &gui()).await.unwrap();
    assert_eq!((read["result"]["settings"]["tokenLimit"].as_i64(), read["result"]["settings"]["balanceAlert"].as_f64()), (Some(2_000_000), Some(2.5)));
    // 메모도 소유자만 읽는다
    assert_eq!(plan(&state, "chatbot.settings.get", None, &json!({}), &relay("admin", false)).await.unwrap_err().code, "PERMISSION_DENIED");
}

#[tokio::test]
async fn every_action_says_how_to_do_it_from_the_cli() {
    let (state, id, _tmp) = server().await;
    let listed = list(&state, None, None, "en").await;
    let create = listed["actions"].as_array().unwrap().iter().find(|a| a["id"] == "instance.create").cloned().unwrap();
    assert_eq!(create["cli"], "saba-chan-cli do instance.create name=<name> module_name=<module_name>");
    let per_server = list(&state, None, Some(&id), "en").await;
    let restore = per_server["actions"].as_array().unwrap().iter().find(|a| a["id"] == "backup.restore").cloned().unwrap();
    assert_eq!(restore["cli"], "saba-chan-cli do backup.restore --server <server> name=<name>");
    assert!(per_server["actions"].as_array().unwrap().iter().all(|a| a["cli"].as_str().is_some_and(|c| c.starts_with("saba-chan-cli do "))));
}

#[tokio::test]
async fn built_in_server_actions_use_the_levels_set_for_their_commands() {
    let (state, id, _tmp) = server().await;
    // GUI 권한 편집기는 내장 명령을 `<instance>/stop`처럼 쓴다
    let levels = json!({ format!("{}/stop", id): 3, format!("{}/start", id): 3, format!("{}/restart", id): 3, format!("{}/status", id): 3 });
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "defaultLevel": 2, "commandLevels": levels })).await;
    let admin = relay("someone", false);
    for action in ["server.stop", "server.start", "server.restart"] {
        assert_eq!(plan(&state, action, Some(&id), &json!({}), &admin).await.unwrap_err().code, "PERMISSION_DENIED", "{}", action);
    }
    assert_eq!(query(&state, "server.status", Some(&id), &json!({}), &admin).await.unwrap_err().code, "PERMISSION_DENIED");
    assert!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("owner", false)).await.is_ok());
}

/// 예전 사람별 금지 목록(`memberPermissions`)은 판정에 쓰지 않는다: 레벨은 누적이다 (사용자 지정 2026-10-28)
#[tokio::test]
async fn old_per_member_deny_lists_no_longer_block_actions() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "defaultLevel": 2, "memberPermissions": { "admin": { id.clone(): ["stop"] } } })).await;
    assert!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("admin", false)).await.is_ok());
}

/// 권한 계단: 원격에서 부를 수 있는 명령·액션마다 `authorize`와 같은 요구 레벨을 싣는다
#[tokio::test]
async fn the_permission_board_lists_every_remote_command_at_the_level_it_needs() {
    let module = format!("{MODULE}\n[[commands.fields]]\nname = \"say\"\nrisk = \"safe\"\n\n[[commands.fields]]\nname = \"ban\"\nrisk = \"danger\"\n\n[[commands.fields]]\nname = \"wipe\"\nrisk = \"safe\"\nremote = false\n");
    let (state, id, _tmp) = server_with(&module).await;
    set_node(&state, json!({ "preset": "default", "commandLevels": { format!("{}/stop", id): 3, "instance.create": 1 } })).await;
    let board = permission_board(&state, "g", "ko").await;
    let item = |key: &str| board["items"].as_array().unwrap().iter().find(|i| i["key"] == key).cloned();
    let say = item(&format!("{}/say", id)).unwrap();
    assert_eq!((&say["level"], &say["preset"], &say["overridden"], &say["kind"]), (&json!(0), &json!(0), &json!(false), &json!("command")));
    assert_eq!(say["module"], true, "a game command shows its own name on the chip");
    assert_eq!(item(&format!("{}/ban", id)).unwrap()["level"], 2);
    assert!(item(&format!("{}/wipe", id)).is_none(), "remote = false commands are not reachable from Discord");
    // 내장 명령은 액션 하나로, 명령 경로와 같은 키로
    let stop = item(&format!("{}/stop", id)).unwrap();
    assert_eq!((&stop["level"], &stop["preset"], &stop["overridden"], &stop["name"]), (&json!(3), &json!(1), &json!(true), &json!("stop")));
    assert_eq!(stop["controllers"], true, "the board says listed people run it too");
    assert_eq!(stop["module"], Value::Null, "a Saba action keeps its translated name");
    assert_eq!(stop["keys"], json!([format!("{}/stop", id), format!("{}/server.stop", id)]));
    assert_eq!(board["items"].as_array().unwrap().iter().filter(|i| i["name"] == "stop").count(), 1);
    let restore = item(&format!("{}/backup.restore", id)).unwrap();
    assert_eq!((&restore["level"], &restore["kind"]), (&json!(2), &json!("admin")));
    let create = item("instance.create").unwrap();
    assert_eq!((&create["level"], &create["preset"], &create["instance"]), (&json!(1), &json!(2), &Value::Null));
    let update = item("update.apply").unwrap();
    assert_eq!((&update["level"], &update["fixed"], &update["overridden"]), (&json!(3), &json!(true), &json!(false)));
    assert!(item("extension.enable").is_none() && item(&format!("{}/instance.delete", id)).is_none(), "PC-only actions are not listed");
    // 계단의 요구 레벨은 실제 판정과 같다
    set_node(&state, json!({ "ownerDiscordId": "owner", "commandLevels": { format!("{}/stop", id): 3 }, "members": { "admin": 2 } })).await;
    assert_eq!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("admin", false)).await.unwrap_err().data["args"]["need"], 3);
    // 이 길드에 열지 않은 서버는 싣지 않는다
    set_node(&state, json!({ "allowedInstances": [] })).await;
    let board = permission_board(&state, "g", "ko").await;
    assert!(board["items"].as_array().unwrap().iter().all(|i| i["instance"].is_null()));
    assert_eq!(board["instances"], json!([]));
}

/// 길드 하나에만 닿는 설정 액션은 기본이 주인이지만 길드가 옮길 수 있다. PC 전체에 닿는 것(업데이트, 공통 챗봇 값)은 고정이다
#[tokio::test]
async fn guild_scoped_settings_have_a_movable_default_level() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let board = permission_board(&state, "g", "ko").await;
    let item = |board: &Value, key: &str| board["items"].as_array().unwrap().iter().find(|i| i["key"] == key).cloned().unwrap();
    for key in ["discord.settings.set", "chatbot.settings.set"] {
        let it = item(&board, key);
        assert_eq!((&it["level"], &it["preset"], &it["fixed"]), (&json!(3), &json!(3), &json!(false)), "{key}");
    }
    let notice = item(&board, &format!("{id}/instance.start_notice.set"));
    assert_eq!((&notice["level"], &notice["fixed"]), (&json!(2), &json!(false)));
    assert_eq!(item(&board, "update.apply")["fixed"], true);
    assert!(crate::actions::overridable("discord.settings.set"));
    assert!(!crate::actions::overridable("update.apply") && !crate::actions::overridable("chatbot.common.set"));
    // 관리자(2)에게 내리면 관리자가 쓸 수 있다
    set_node(&state, json!({ "ownerDiscordId": "owner", "members": { "admin": 2 }, "commandLevels": { "discord.settings.set": 2 } })).await;
    assert!(plan(&state, "discord.settings.set", None, &json!({ "locale": "ko" }), &relay("admin", false)).await.is_ok());
    assert_eq!(item(&permission_board(&state, "g", "ko").await, "discord.settings.set")["overridden"], true);
    assert_eq!(plan(&state, "chatbot.settings.set", None, &json!({ "memo": "x" }), &relay("admin", false)).await.unwrap_err().code, "PERMISSION_DENIED");
}

/// 계단 항목마다 되묻기 단계 키(`commandRisks`와 같은 `<서버>/<액션 id>`)와 이 길드에서 쓰는 단계를 싣는다
#[tokio::test]
async fn the_permission_board_carries_each_commands_confirm_tier() {
    let module = format!("{MODULE}\n[[commands.fields]]\nname = \"say\"\nrisk = \"safe\"\n");
    let (state, id, _tmp) = server_with(&module).await;
    let risks = json!({ format!("{id}/server.start"): "danger", format!("{id}/server.command:say"): "caution" });
    set_node(&state, json!({ "discord": { "commandRisks": risks } })).await;
    let board = permission_board(&state, "g", "ko").await;
    let item = |name: &str| board["items"].as_array().unwrap().iter().find(|i| i["name"] == name).cloned().unwrap();
    let start = item("start");
    assert_eq!((&start["riskKey"], &start["confirmRisk"]), (&json!(format!("{id}/server.start")), &json!("danger")));
    let say = item("say");
    assert_eq!((&say["riskKey"], &say["risk"], &say["confirmRisk"]), (&json!(format!("{id}/server.command:say")), &json!("safe"), &json!("caution")));
    let create = item("instance.create");
    assert_eq!((&create["riskKey"], &create["confirmRisk"]), (&json!("instance.create"), &create["risk"]));
}

/// 채팅 `players`와 액션 `server.players`는 같은 덮어쓰기 키를 본다 (채팅 경로 BUILTIN_COMMANDS와 같다)
#[test]
fn every_chat_builtin_command_shares_its_override_key_with_its_action() {
    for cmd in ["start", "stop", "restart", "status"] {
        assert_eq!(override_keys("mc", &format!("server.{cmd}")), vec![format!("mc/{cmd}"), format!("mc/server.{cmd}")], "{cmd}");
    }
    // players는 릴레이 사전 판정과 같은 액션 키에 먼저 쓴다
    assert_eq!(override_keys("mc", "server.players"), vec!["mc/server.players".to_string(), "mc/players".to_string()]);
}

/// 예약 작업은 나중에 요청자 없이 돈다: 새로 넣거나 켜는 서버 명령은 요청자가 지금 그 명령을 쓸 수 있어야 한다
#[tokio::test]
async fn scheduling_a_command_needs_the_requesters_permission_for_it() {
    let module = format!("{MODULE}\n[[commands.fields]]\nname = \"say\"\nrisk = \"safe\"\n\n[aliases.commands]\nsay = {{ aliases = [\"broadcast\"] }}\n");
    let (state, id, _tmp) = server_with(&module).await;
    let levels = json!({ format!("{}/say", id): 3, format!("{}/stop", id): 3 });
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "chatbotCap": 2, "defaultLevel": 2, "commandLevels": levels })).await;
    let task = |action: &str, command: Option<&str>| {
        json!({ "schedule": [{ "action": action, "at": "04:00", "command": command, "enabled": true }] })
    };
    let admin = relay("admin", true);
    for (input, what) in [(task("command", Some("say")), "say"), (task("command", Some("BROADCAST")), "alias"), (task("stop", None), "stop")] {
        let err = plan(&state, "instance.schedule.set", Some(&id), &input, &admin).await.unwrap_err();
        assert_eq!((err.code, &err.data["args"]["need"]), ("PERMISSION_DENIED", &json!(3)), "{}", what);
    }
    let start = with_revision(&state, &id, task("start", None)).await;
    assert!(plan(&state, "instance.schedule.set", Some(&id), &start, &admin).await.is_ok(), "only the limited commands");
    let off = with_revision(&state, &id, json!({ "schedule": [{ "action": "command", "at": "04:00", "command": "say", "enabled": false }] })).await;
    assert!(plan(&state, "instance.schedule.set", Some(&id), &off, &admin).await.is_ok(), "a disabled task never runs");
}

/// 예약 공지·저장도 모듈의 게임 명령을 돌린다: 그 명령의 레벨이 요청자에게 없으면 예약하지 못한다
#[tokio::test]
async fn scheduling_an_announcement_or_save_needs_the_permission_for_its_command() {
    let module = format!(
        "{MODULE}\n[config]\nannounce_command = \"say\"\n\n[backup]\npaths = [\"world\"]\nsave_command = \"save-all\"\n\n[[commands.fields]]\nname = \"say\"\nrisk = \"safe\"\n\n[[commands.fields]]\nname = \"save-all\"\nrisk = \"safe\"\n"
    );
    let (state, id, _tmp) = server_with(&module).await;
    let levels = json!({ format!("{}/say", id): 3, format!("{}/save-all", id): 3 });
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "chatbotCap": 2, "defaultLevel": 2, "commandLevels": levels })).await;
    let admin = relay("admin", true);
    for task in [json!({ "action": "announce", "at": "04:00", "message": "hi" }), json!({ "action": "save", "at": "04:00" })] {
        let input = json!({ "schedule": [task] });
        let err = plan(&state, "instance.schedule.set", Some(&id), &input, &admin).await.unwrap_err();
        assert_eq!((err.code, &err.data["args"]["need"]), ("PERMISSION_DENIED", &json!(3)), "{}", input);
    }
}

#[test]
fn remote_results_keep_paths_console_lines_and_raw_errors_on_the_pc() {
    let mut v = json!({
        "servers": [{ "id": "a", "name": "srv", "working_dir": "C:\\Games\\srv", "executable_path": "C:\\Games\\srv\\run.exe", "extra_args": "-x", "status": "running" }],
        "events": [
            { "kind": "crash", "evidence": "crash file C:\\Games\\srv\\crash.txt" },
            { "kind": "start_failed", "evidence": "ended 3s after start: Player Steve joined from 1.2.3.4" },
            { "kind": "in_game_stop", "evidence": "console: Stopping server" },
            { "kind": "crash", "evidence": "exit code 0xC0000005", "note_code": "restart_failed", "note_args": { "reason": "create /home/me/srv failed" } },
        ],
        "restored": "/home/me/.config/saba-chan/backups/x.zip",
        "error": "Traceback (most recent call last):\n  File \"C:\\Users\\me\\modules\\mc\\lifecycle.py\", line 3, in <module>\nValueError: bad value in C:\\Users\\me\\x.txt",
        "url": "https://example.com/a/b/c",
        "spaced": "copy C:\\Program Files\\Saba-chan\\a.txt (os error 5)",
        "quoted": "open '/home/me/Game Server/b.txt' failed",
        "glued": "failed:/opt/srv1/run.sh [/opt/srv2/run.sh] ~/srv3/world",
    });
    strip_local_details(&mut v);
    let text = v.to_string();
    for leak in ["Games", "crash.txt", "Steve", "Stopping", "/home/me", "Users", "lifecycle.py", "-x", "Files", "Saba-chan", "Server", "srv1", "srv2", "srv3"] {
        assert!(!text.contains(leak), "{} leaked: {}", leak, text);
    }
    assert_eq!(v["servers"][0], json!({ "id": "a", "name": "srv", "status": "running" }));
    assert_eq!(v["events"][3]["evidence"], "exit code 0xC0000005");
    assert_eq!(v["events"][3]["note_code"], "restart_failed");
    assert_eq!(v["error"], "ValueError: bad value in <path>");
    assert_eq!(v["url"], "https://example.com/a/b/c");
    assert_eq!(v["quoted"], "open '<path>' failed");
}

#[tokio::test]
async fn guilds_only_see_their_allowed_instances_in_any_listing() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "allowedInstances": ["other"] })).await;
    let summary = query(&state, "monitor.summary", None, &json!({}), &relay("owner", false)).await.unwrap();
    assert!(!summary.to_string().contains(&id), "{}", summary);
    let mut nested = json!({ "instances": [{ "id": id.clone() }, { "id": "other" }], "disks": [{ "instances": [{ "id": id.clone(), "bytes": 1 }] }] });
    retain_allowed(&mut nested, &["other".to_string()]);
    assert_eq!(nested, json!({ "instances": [{ "id": "other" }], "disks": [{ "instances": [] }] }));
}

#[tokio::test]
async fn the_chatbot_reads_logs_with_inputs_and_without_local_details() {
    let (state, id, _tmp) = server().await;
    let dir = crate::supervisor::managed_process::sessions_dir(&state.supervisor.read().await.instance_store.instance_dir(&id));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("20261020-110000.log"),
        "[2026-10-20 11:00:00] [STDOUT/INFO] Steve joined from 10.1.2.3\n[2026-10-20 11:00:01] [STDOUT/INFO] saved C:\\Games\\srv\\world\n[2026-10-20 11:00:02] [STDOUT/INFO] ready\n",
    )
    .unwrap();
    let out = query(&state, "server.logs.read", Some(&id), &json!({ "lines": 2 }), &relay("owner", true)).await.unwrap();
    let lines = &out["result"]["lines"];
    assert_eq!(lines.as_array().unwrap().len(), 2, "the input reached the route: {}", out);
    assert_eq!(lines[0], "[2026-10-20 11:00:01] [STDOUT/INFO] saved <path>");
    assert!(!out.to_string().contains("10.1.2.3"));

    // 내보내기 결과는 줄마다 가린다: 앞 줄의 traceback이 뒤 줄을 지우지 않는다
    let mut file = json!({ "fileName": "srv.log", "content": "Traceback (most recent call last):\n  File \"C:\\Users\\me\\a.py\", line 1\nValueError: x\nnext line\n", "bytes": 79 });
    strip_local_details(&mut file);
    assert_eq!(file["content"], "Traceback (most recent call last):\n  File \"<path>\", line 1\nValueError: x\nnext line\n");
    assert_eq!(file["bytes"], file["content"].as_str().unwrap().len(), "bytes follow the masked content");
    assert_eq!(file["fileName"], "srv.log");
    // 말하는 사바쨩 상한 기본값(0)에서도 "로그 파일 줘"가 된다: 가린 읽기 전용이라 읽기와 같은 safe
    assert_eq!((find("server.logs.read").unwrap().risk, find("server.logs.export").unwrap().risk), ("safe", "safe"));
}

/// 플레이어 (검수 13번): 접속자 수와 이름을 한 문장으로, heartbeat 항목은 지원하는 모듈(`[status] players`)만 이름으로 싣는다
#[tokio::test]
async fn players_read_as_a_count_and_names_for_supporting_modules() {
    let names: Vec<Value> = (0..12).map(|n| json!({ "name": format!("p{n}") })).collect();
    let text = summary_of("server.players", json!({ "success": true, "online": 12, "max": 20, "players": names })).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "[actions.summary.players_max online=12 max=20]");
    assert_eq!(lines[1], "[actions.summary.player_item name=p0]");
    assert_eq!(lines.last().copied(), Some("[actions.summary.players_more count=2]"));
    assert_eq!(summary_of("server.players", json!({ "success": true, "online": 0, "players": [] })).unwrap(), "[actions.summary.players online=0]");

    let (state, _id, _tmp) = server().await;
    let mut modules = state.supervisor.read().await.module_loader.discover_modules().unwrap();
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    assert_eq!(actions.iter().find(|a| a["id"] == "server.players").unwrap()["modules"], json!([]));
    modules[0].metadata.players_function = Some("player_count".into());
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    let item = actions.iter().find(|a| a["id"] == "server.players").unwrap();
    assert_eq!((item["modules"].clone(), item["title"]["ko"].clone(), item["surfaces"].clone()), (json!(["plain"]), json!("플레이어"), json!(["discord", "chatbot"])));
}

/// 포트포워딩 확인: 디스코드 요약은 판정·포트·안내만 (공인 주소는 채널에 싣지 않는다), heartbeat 항목은 `public` 포트를 선언한 모듈만,
/// 챗봇에는 열지 않는다
#[tokio::test]
async fn port_check_reads_as_a_verdict_without_the_address() {
    let report = json!({
        "verdict": "closed", "address": "203.0.114.7", "publicAddress": true, "nat": "router",
        "ports": [{ "field": "port", "port": 8211, "protocol": "udp", "status": "closed" }, { "field": "port", "port": 8211, "protocol": "tcp", "status": "open" }],
    });
    let text = summary_of("server.port_check", report).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "[actions.summary.port_check.closed]");
    assert_eq!(lines[1], "[actions.summary.port_item port=8211 protocol=UDP status=[actions.summary.port_status.closed]]");
    assert_eq!(lines[2], "[actions.summary.port_item port=8211 protocol=TCP status=[actions.summary.port_status.open]]");
    assert_eq!(lines[3], "[actions.summary.port_hint.closed]");
    assert!(!text.contains("203.0.114.7"));
    assert_eq!(summary_of("server.port_check", json!({ "verdict": "open", "ports": [] })).unwrap(), "[actions.summary.port_check.open]");

    assert_eq!(find("server.port_check").unwrap().surfaces, &[Gui, Cli, Discord]);
    let (state, _id, _tmp) = server().await;
    let mut modules = state.supervisor.read().await.module_loader.discover_modules().unwrap();
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    assert_eq!(actions.iter().find(|a| a["id"] == "server.port_check").unwrap()["modules"], json!([]));
    let settings = modules[0].metadata.settings.get_or_insert_with(|| crate::module_loader::ModuleSettings { fields: Vec::new() });
    settings.fields.push(crate::module_loader::SettingField { name: "port".into(), port: true, public: Some("udp".into()), ..Default::default() });
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    let item = actions.iter().find(|a| a["id"] == "server.port_check").unwrap();
    assert_eq!((item["modules"].clone(), item["title"]["ko"].clone(), item["surfaces"].clone()), (json!(["plain"]), json!("포트포워딩 확인"), json!(["discord"])));
}

/// 디스코드에서 백업 복원 (사용자 디스코드 검수 14번): 디스코드 메뉴에 싣고, 이름 칸은 백업 목록에서 고르며(`optionsFrom`),
/// 목록 항목에는 사람이 읽을 이름(`label`)이 붙는다. 켜진 서버는 복원 계획만으로는 409이고, 먼저 끄는 단계가 앞에 있으면 계획된다
#[tokio::test]
async fn discord_restores_a_backup_picked_from_the_list() {
    let (actions, _) = heartbeat_actions(&["ko"], &[]);
    let restore = actions.iter().find(|a| a["id"] == "backup.restore").unwrap();
    assert_eq!(restore["surfaces"], json!(["discord", "chatbot"]));
    assert_eq!(
        restore["input"]["properties"]["name"]["optionsFrom"],
        json!({ "action": "backup.list", "list": "backups", "value": "name", "label": "label" })
    );

    let (state, id, tmp) = server().await;
    let local = list(&state, None, None, "ko").await["actions"].as_array().unwrap().iter().find(|a| a["id"] == "backup.restore").cloned().unwrap();
    assert_eq!(local["input"]["properties"]["name"]["optionsFrom"]["action"], "backup.list");
    open_everything(&state).await;
    let world = tmp.path().join("world");
    std::fs::create_dir_all(&world).unwrap();
    let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
    let made = crate::backup::create(&dir, crate::backup::BackupKind::Manual, &[world]).unwrap().name;
    let listed = query(&state, "backup.list", Some(&id), &json!({}), &relay("owner", false)).await.unwrap();
    let label = listed["result"]["backups"][0]["label"].as_str().unwrap();
    assert!(label.contains("직접") && label.contains(" · "), "{label}");

    // 켜진 서버
    state.supervisor.read().await.tracker.track(&id, std::process::id()).unwrap();
    let err = plan(&state, "backup.restore", Some(&id), &json!({ "name": made }), &relay("owner", false)).await.unwrap_err();
    assert_eq!((err.status, err.code), (409, "server_running"));
    let alone = json!([{ "id": "backup.restore", "instance": id, "input": { "name": made } }]);
    assert_eq!(plan_steps(&state, &alone, &relay("owner", false)).await.unwrap_err().code, "server_running");
    let stop_first = json!([{ "id": "server.stop", "instance": id }, { "id": "backup.restore", "instance": id, "input": { "name": made } }]);
    assert!(plan_steps(&state, &stop_first, &relay("owner", false)).await.is_ok());
    // 원격 정지는 강제 종료를 받지 않는다
    let forced = json!([{ "id": "server.stop", "instance": id, "input": { "force": true } }]);
    assert!(plan_steps(&state, &forced, &relay("owner", false)).await.is_err());
    let _ = state.supervisor.read().await.tracker.untrack(&id);
}

/// 원격 실행이 실패하면 요청 언어의 짧은 이유(`reason`)를 함께 준다: 릴레이가 영어 원문 대신 보인다
#[tokio::test]
async fn a_failed_remote_action_carries_a_short_reason() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner" })).await;
    let out = query(&state, "server.query:players", Some(&id), &json!({}), &relay("owner", false)).await.unwrap();
    assert_eq!(out["success"], false, "{out}");
    assert_eq!(shown(&out["reason"]), "[reasons.not_supported file=]", "{out}");
    // 이 PC 화면은 원문을 그대로 쓴다
    let local = query(&state, "server.query:players", Some(&id), &json!({}), &gui()).await.unwrap();
    assert!(local.get("reason").is_none(), "{local}");
}

/// `remote = false` 명령은 디스코드·챗봇 액션으로도 부를 수 없다 (GUI·CLI에서만)
#[test]
fn a_command_kept_off_remote_surfaces_is_not_a_remote_action() {
    let hidden = command_view(&json!({ "name": "save-off", "risk": "caution", "inputs": [], "remote": false }), &Value::Null).unwrap();
    assert_eq!(hidden["surfaces"], json!(["gui", "cli"]));
    let shown = command_view(&json!({ "name": "say", "risk": "safe", "inputs": [] }), &Value::Null).unwrap();
    assert_eq!(shown["surfaces"], json!(["gui", "cli", "discord", "chatbot"]));
}

/// 모듈 명령 이름·설명은 요청 언어의 모듈 locales가 먼저다 (권한 계단에 module.toml 영어 이름이 뜨던 문제, 2026-11-07)
#[test]
fn module_command_titles_come_from_the_module_locales_first() {
    let cmd = json!({ "name": "say", "risk": "safe", "label": "Send Chat Message", "description": "Broadcast" });
    let ko = json!({ "commands": { "say": { "label": "채팅 메시지 전송", "description": "서버에 메시지를 알립니다" } } });
    let v = command_view(&cmd, &ko).unwrap();
    assert_eq!((v["title"].as_str(), v["description"].as_str()), (Some("채팅 메시지 전송"), Some("서버에 메시지를 알립니다")));
    let v = command_view(&cmd, &Value::Null).unwrap();
    assert_eq!((v["title"].as_str(), v["description"].as_str()), (Some("Send Chat Message"), Some("Broadcast")));
    assert_eq!(command_view(&json!({ "name": "x" }), &ko).unwrap()["title"], "x");
}

// 사용자 요청(2026-10-26): 켜짐 알림은 디스코드 `/<서버> 켜짐-알림`으로도 바꾼다. 관리자 이상, 요청이 온 길드만, 말하는 사바쨩은 아니다
#[tokio::test]
async fn start_notice_is_set_from_discord_by_admins_for_their_own_guild() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "defaultLevel": 1, "members": { "admin": 2 } })).await;
    let on = json!({ "enabled": true });
    let id_ = "instance.start_notice.set";
    assert_eq!(plan(&state, id_, Some(&id), &on, &relay("user", false)).await.unwrap_err().code, "PERMISSION_DENIED");
    assert_eq!(plan(&state, id_, Some(&id), &on, &relay("admin", true)).await.unwrap_err().code, "action_not_allowed");
    let other = json!({ "guild": "other", "enabled": true });
    assert_eq!(plan(&state, id_, Some(&id), &other, &relay("admin", false)).await.unwrap_err().code, "invalid_input");

    let planned = plan(&state, id_, Some(&id), &on, &relay("admin", false)).await.unwrap();
    assert_eq!(planned["needsConfirm"], false);
    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &relay("admin", false)).await.unwrap();
    assert_eq!(done["success"], true, "{done}");
    assert_eq!(state.config_store.get_bot_config().await.node_settings["g"]["startNotice"], json!([id]));

    // 서버 루트 아래 메뉴에 실린다
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = actions.iter().find(|a| a["id"] == id_).expect("in the Discord menu");
    assert_eq!(item["title"]["ko"], "시작·정지 알림");
    assert_eq!(item["name"]["en"], "Start alert");
}

// 사용자 11-04: 자리 꽉 참 알림도 서버마다 켠다. 켜짐 알림처럼 관리자 이상, 요청이 온 길드만
#[tokio::test]
async fn full_notice_is_set_from_discord_by_admins() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "defaultLevel": 1, "members": { "admin": 2 } })).await;
    let on = json!({ "enabled": true });
    let id_ = "instance.full_notice.set";
    assert_eq!(plan(&state, id_, Some(&id), &on, &relay("user", false)).await.unwrap_err().code, "PERMISSION_DENIED");
    let planned = plan(&state, id_, Some(&id), &on, &relay("admin", false)).await.unwrap();
    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &relay("admin", false)).await.unwrap();
    assert_eq!(done["success"], true, "{done}");
    assert_eq!(state.config_store.get_bot_config().await.node_settings["g"]["fullNotice"], json!([id]));
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = actions.iter().find(|a| a["id"] == id_).expect("in the Discord menu");
    assert_eq!(item["title"]["ko"], "자리 꽉 참 알림");
}

/// 디스코드 문장 명령도 계획 → 실행 한 길로: 데몬이 문장을 해석해 액션과 서버를 알려 주고, 실행하면 문장 명령과 같은 답을 쓴다
#[tokio::test]
async fn a_sentence_plans_the_action_it_names_and_runs_with_the_same_reply() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let caller = relay("u1", false);
    let planned = plan_text(&state, "srv status", &caller).await.unwrap();
    assert_eq!(planned["actionId"], "server.status");
    assert_eq!(planned["instance"], json!(id));
    assert_eq!(planned["needsConfirm"], false);
    let plan_id = planned["planId"].as_str().unwrap().to_string();

    // 다른 사람은 남의 계획을 실행하지 못한다
    let err = run(&state, "", None, &Value::Null, Some(&plan_id), &relay("u2", false)).await.unwrap_err();
    assert_eq!(err.code, "plan_not_yours");

    let done = run(&state, "", None, &Value::Null, Some(&plan_id), &caller).await.unwrap();
    assert_eq!(done["success"], true, "{done}");
    assert_eq!(done["actionId"], "server.status");
    assert_eq!(done["instance"], json!(id));
    let text = shown(&done["text"]);
    assert!(text.starts_with("[server.status_check") && text.contains("name=srv"), "{text}");
    let (settings, cat) = chat_settings(&state, &caller).await;
    let reply = crate::chat::process(&state, &settings, &cat, &chat_request("srv status", &caller)).await;
    assert_eq!(done["text"], reply.text.to_json());
}

/// 문장 계획도 실행할 때 지금 권한으로 다시 판정한다: 계획 뒤에 권한을 잃었으면 실행하지 않는다
#[tokio::test]
async fn a_sentence_plan_is_judged_again_when_it_runs() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let caller = relay("u1", false);
    let planned = plan_text(&state, "srv status", &caller).await.unwrap();
    set_node(&state, json!({ "ownerDiscordId": "owner", "defaultLevel": 1, "members": { "u1": -1 } })).await;
    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &caller).await.unwrap();
    assert_eq!(done["success"], false, "{done}");
}

/// 실행할 것이 없는 문장(도움말·모르는 명령)은 계획 없이 답한다. 모르는 입력은 코드와 함께 실패로
#[tokio::test]
async fn a_sentence_with_nothing_to_run_is_answered_without_a_plan() {
    let (state, _id, _tmp) = server().await;
    open_everything(&state).await;
    let help = plan_text(&state, "help", &relay("u1", false)).await.unwrap();
    assert_eq!(help["success"], true);
    assert!(help.get("planId").is_none(), "{help}");
    assert!(!shown(&help["text"]).is_empty(), "{help}");

    let unknown = plan_text(&state, "srv dance", &relay("u1", false)).await.unwrap();
    assert_eq!(unknown["success"], false);
    assert_eq!(unknown["code"], "UNKNOWN_COMMAND");
    assert!(!shown(&unknown["text"]).is_empty(), "{unknown}");
    assert!(unknown.get("planId").is_none());
}

/// 모듈 명령이 아닌 낱말은 그 서버의 디스코드 인스턴스 액션 이름으로 찾는다 (슬래시 서버 루트와 같은 명령)
#[tokio::test]
async fn a_sentence_naming_an_instance_action_plans_that_action() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let caller = relay("owner", false);
    for (text, action) in [("srv 재시작", "server.restart"), ("srv restart", "server.restart"), ("srv 백업 목록", "backup.list"), ("srv 백업", "backup.create")] {
        let planned = plan_text(&state, text, &caller).await.unwrap();
        assert_eq!((planned["actionId"].clone(), planned["instance"].clone()), (json!(action), json!(id)), "{text}: {planned}");
    }
    // 디스코드에 내보내지 않는 액션, 위험 액션, 남는 낱말은 그대로 모르는 명령이다
    for text in ["srv 콘솔끝부분", "srv 백업복원", "srv 재시작 지금"] {
        let unknown = plan_text(&state, text, &caller).await.unwrap();
        assert_eq!(unknown["code"], "UNKNOWN_COMMAND", "{text}: {unknown}");
    }
    // 확인 버튼을 눌러 다시 보낸 문장은 묻지 않는다
    set_confirm(&state, "all", "all").await;
    assert_eq!(plan_text(&state, "srv 재시작", &caller).await.unwrap()["needsConfirm"], true);
    assert_eq!(plan_text(&state, "srv 재시작", &pressed("owner")).await.unwrap()["needsConfirm"], false);
}

/// 길드 되묻기 단계: 직접 요청은 디스코드 설정(`discord.confirmLevel`), 말하는 사바쨩은 챗봇 설정(`confirmLevel`)
async fn set_confirm(state: &IPCServer, direct: &str, chatbot: &str) {
    set_node(state, json!({ "ownerDiscordId": "owner", "preset": "loose", "chatbotCap": 2, "defaultLevel": 2, "discord": { "confirmLevel": direct } })).await;
    let mut cfg = state.config_store.get_chatbot();
    cfg.guilds.insert("g".into(), json!({ "confirmLevel": chatbot }).as_object().cloned().unwrap());
    state.config_store.set_chatbot(&cfg).unwrap();
}

/// 직접 요청은 기본으로 위험만 묻고, 길드가 고른 단계부터 묻는다. "묻지 않음"이면 위험도 바로 실행한다
#[tokio::test]
async fn direct_discord_requests_ask_from_the_guilds_chosen_level() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    async fn asks(state: &IPCServer, action: &str, instance: &str, input: Value) -> bool {
        plan(state, action, Some(instance), &input, &relay("owner", false)).await.unwrap()["needsConfirm"] == true
    }
    let stop = || asks(&state, "server.stop", &id, json!({}));
    let restore = || asks(&state, "backup.restore", &id, json!({ "name": "b" }));
    assert_eq!((stop().await, restore().await), (false, true));

    // 시작·정지는 위험·주의 밖이므로 "모두 묻기"에서만 묻는다
    set_confirm(&state, "caution", "all").await;
    assert!(!stop().await);
    set_confirm(&state, "all", "all").await;
    assert!(stop().await);
    set_confirm(&state, "none", "all").await;
    assert!(!restore().await);
}

/// 말하는 사바쨩은 챗봇 확인 단계를 따른다. 고른 관리 명령은 단계와 상관없이 늘 묻는다
#[tokio::test]
async fn chatbot_requests_follow_the_chatbot_level_but_admin_actions_always_ask() {
    let (state, id, _tmp) = server().await;
    set_confirm(&state, "danger", "none").await;
    let stop = plan(&state, "server.stop", Some(&id), &json!({}), &relay("owner", true)).await.unwrap();
    assert_eq!(stop["needsConfirm"], false);
    let settings = plan(&state, "instance.settings.set", Some(&id), &json!({ "players": 4 }), &relay("owner", true)).await.unwrap();
    assert_eq!(settings["needsConfirm"], true);
    set_confirm(&state, "danger", "all").await;
    assert_eq!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("owner", true)).await.unwrap()["needsConfirm"], true);
}

/// 문장 명령 계획도 같은 규칙으로 묻는다. 요청자가 확인 버튼을 눌러 다시 보낸 문장은 묻지 않는다
#[tokio::test]
async fn sentence_plans_ask_by_the_same_rule_unless_the_requester_confirmed() {
    let (state, _id, _tmp) = server().await;
    set_confirm(&state, "all", "all").await;
    let asked = plan_text(&state, "srv start", &relay("owner", false)).await.unwrap();
    assert_eq!(asked["needsConfirm"], true, "{asked}");
    let err = run(&state, "", None, &Value::Null, asked["planId"].as_str(), &relay("owner", false)).await.unwrap_err();
    assert_eq!((err.status, err.code), (409, "confirm_required"));
    let confirmed = plan_text(&state, "srv start", &pressed("owner")).await.unwrap();
    assert_eq!(confirmed["needsConfirm"], false, "{confirmed}");
    set_confirm(&state, "danger", "danger").await;
    assert_eq!(plan_text(&state, "srv start", &relay("owner", true)).await.unwrap()["needsConfirm"], false);
}

/// 말하는 사바쨩이 고른 위험 모듈 명령도 같은 규칙으로 묻는다: 확인 전이면 거절 대신 확인 계획을, "묻지 않음"이면 바로 실행할 계획을
#[tokio::test]
async fn chatbot_danger_sentences_follow_the_chatbot_confirm_level() {
    let module = format!("{MODULE}\n[[commands.fields]]\nname = \"ban\"\nrisk = \"danger\"\n");
    let (state, _id, _tmp) = server_with(&module).await;
    set_confirm(&state, "danger", "danger").await;
    let asked = plan_text(&state, "srv ban steve", &relay("owner", true)).await.unwrap();
    assert_eq!((asked["needsConfirm"].clone(), asked["actionId"].clone()), (json!(true), json!("server.command:ban")), "{asked}");
    set_confirm(&state, "danger", "none").await;
    let direct = plan_text(&state, "srv ban steve", &relay("owner", true)).await.unwrap();
    assert_eq!(direct["needsConfirm"], false, "{direct}");
}

/// 앱·CLI는 길드와 상관없이 위험과 정지·재시작을 묻는다. 묻는 액션은 계획 없이 바로 실행하지 않는다
#[tokio::test]
async fn local_surfaces_ask_for_danger_and_for_stopping_a_server() {
    let (state, id, _tmp) = server().await;
    set_confirm(&state, "none", "none").await;
    for (action, input, asks) in [("server.stop", json!({}), true), ("server.restart", json!({}), true), ("server.start", json!({}), false), ("backup.restore", json!({ "name": "b" }), true)] {
        let made = plan(&state, action, Some(&id), &input, &gui()).await.unwrap();
        assert_eq!(made["needsConfirm"].as_bool(), Some(asks), "{action}");
    }
    let err = run(&state, "server.stop", Some(&id), &json!({}), None, &gui()).await.unwrap_err();
    assert_eq!((err.status, err.code), (409, "confirm_required"));
}

/// 확인 단계 값은 정해진 넷만 받는다
#[test]
fn the_direct_confirm_level_takes_only_known_values() {
    let mut s = crate::service::discord::GuildSettings::default();
    assert_eq!(s.confirm_level, "danger");
    s.apply(json!({ "confirmLevel": "none" }).as_object().unwrap()).unwrap();
    assert_eq!(s.confirm_level, "none");
    assert_eq!(s.apply(json!({ "confirmLevel": "sometimes" }).as_object().unwrap()).unwrap_err().0, "confirmLevel");
}

/// 원격 MCP는 길드 설정이 아니라 앱과 같은 고정 규칙으로 묻는다. 확인은 요청자 본인 것이어야 한다
#[tokio::test]
async fn mcp_requests_ask_by_the_local_rule_not_the_guild_level() {
    let (state, id, _tmp) = server().await;
    set_confirm(&state, "none", "none").await;
    let mut mcp = relay("owner", false);
    mcp.origin.via = Some(MCP_VIA.into());
    let stop = plan(&state, "server.stop", Some(&id), &json!({}), &mcp).await.unwrap();
    assert_eq!(stop["needsConfirm"], true);
    let err = run(&state, "", None, &Value::Null, stop["planId"].as_str(), &mcp).await.unwrap_err();
    assert_eq!(err.code, "confirm_required");
}

/// 원격 MCP(계정 주인의 AI 에이전트)는 앱과 같은 액션을 주인 레벨로 쓴다.
/// 웹 원격·짝 맺은 PC처럼 PC에서만 하는 일과 PC 전용 칸은 막고, 확인이 필요한 계획은 주인의 확인으로만 돈다
#[tokio::test]
async fn an_mcp_owner_reaches_app_actions_but_not_pc_only_ones() {
    let (state, id, _tmp) = server().await;
    let owner = Caller::mcp("owner".into(), "owner".into(), None, None, "en".into());
    assert_eq!(owner.surface, Gui);
    assert_eq!(owner.origin.source, "mcp");
    let enable = plan(&state, "extension.enable", None, &json!({ "extension": "x" }), &owner).await.unwrap();
    assert_eq!(enable["needsConfirm"], false);
    // 앱 업데이트는 개인 에이전트에 연다 (사용자 결정 2026-11-05). 위험이라 늘 묻는다
    let update = plan(&state, "update.apply", None, &json!({}), &owner).await.unwrap();
    assert_eq!(update["needsConfirm"], true);
    // 디스코드에서 온 원격은 그대로 PC 전용이다
    let paired = Caller::local(crate::audit::Origin::new("remote"), "en".into());
    assert_eq!(plan(&state, "update.apply", None, &json!({}), &paired).await.unwrap_err().code, "remote_local_only");
    let err = plan(&state, "instance.settings.set", Some(&id), &json!({ "java_path": "/bin/sh" }), &owner).await.unwrap_err();
    assert_eq!((err.code, err.data["field"].as_str()), ("invalid_input", Some("java_path")), "{err:?}");
    // 주인이 아니면 기본 레벨이다 (관리 액션은 막힌다)
    let other = Caller::mcp("other".into(), "owner".into(), None, None, "en".into());
    let err = plan(&state, "extension.enable", None, &json!({ "extension": "x" }), &other).await.unwrap_err();
    assert_eq!(err.code, "PERMISSION_DENIED");

    let stop = plan(&state, "server.stop", Some(&id), &json!({}), &owner).await.unwrap();
    assert_eq!(stop["needsConfirm"], true);
    let pid = stop["planId"].as_str().unwrap();
    let err = run(&state, "", None, &Value::Null, Some(pid), &owner).await.unwrap_err();
    assert_eq!(err.code, "confirm_required");
    let confirmed = Caller::mcp("owner".into(), "owner".into(), Some("owner".into()), None, "en".into());
    let ran = run(&state, "", None, &Value::Null, Some(pid), &confirmed).await;
    assert!(ran.is_ok(), "{ran:?}");
}

/// 길드가 명령별로 정한 위험 단계(`discord.commandRisks["<서버>/<액션>"]`)로 디스코드 되묻기를 정한다. 앱은 고정 규칙 그대로
#[tokio::test]
async fn a_guild_can_retier_one_command_for_discord_confirms() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let risks = json!({ format!("{id}/server.start"): "danger", format!("{id}/backup.restore"): "safe" });
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "chatbotCap": 2, "defaultLevel": 2, "discord": { "confirmLevel": "danger", "commandRisks": risks } })).await;
    let asks = |action: &'static str, input: Value, caller: Caller| {
        let (state, id) = (&state, id.clone());
        async move { plan(state, action, Some(&id), &input, &caller).await.unwrap()["needsConfirm"] == true }
    };
    assert!(asks("server.start", json!({}), relay("owner", false)).await);
    assert!(!asks("backup.restore", json!({ "name": "b" }), relay("owner", false)).await);
    assert!(asks("backup.restore", json!({ "name": "b" }), gui()).await);
    assert!(!asks("server.start", json!({}), gui()).await);
}

/// 명령별 위험 단계는 safe·caution·danger만 받고, null이면 지운다
#[test]
fn command_risks_take_known_tiers_and_null_removes() {
    let mut s = crate::service::discord::GuildSettings::default();
    s.apply(json!({ "commandRisks": { "i/server.start": "danger" } }).as_object().unwrap()).unwrap();
    assert_eq!(s.command_risks.get("i/server.start").map(String::as_str), Some("danger"));
    assert_eq!(s.apply(json!({ "commandRisks": { "i/x": "scary" } }).as_object().unwrap()).unwrap_err().0, "commandRisks.i/x");
    s.apply(json!({ "commandRisks": { "i/server.start": null } }).as_object().unwrap()).unwrap();
    assert!(s.command_risks.is_empty());
}

/// 원격 MCP가 앱 화면에서 하던 일(설정·예약 읽기, 모듈·플러그인 목록과 설치, 게임 서버 업데이트, 진단, 콘솔 명령,
/// 이름 바꾸기, 백업 지우기)도 액션으로 닿는다. 결과의 비밀값은 다른 원격 요청과 같이 빠진다
#[tokio::test]
async fn an_mcp_owner_reaches_what_the_app_screens_do() {
    let (state, id, _tmp) = server().await;
    let owner = Caller::mcp("owner".into(), "owner".into(), None, None, "en".into());
    let listed = list(&state, Some(owner.surface), None, "en").await;
    let ids: Vec<&str> = listed["actions"].as_array().unwrap().iter().filter_map(|a| a["id"].as_str()).collect();
    for want in [
        "instance.get",
        "instance.schedule.get",
        "instance.rename",
        "module.list",
        "module.available",
        "extension.list",
        "extension.available",
        "extension.install",
        "server.update.check",
        "server.update.apply",
        "server.update.cancel",
        "server.diagnose",
        "server.console.send",
        "backup.delete",
    ] {
        assert!(ids.contains(&want), "{want} missing from the app catalog");
        // 디스코드·챗봇 표면은 넓히지 않는다. 게임 서버 업데이트만 디스코드에도 연다 (사용자 결정 2026-11-10, 챗봇은 아니다).
        // 예약 읽기는 말하는 사바쨩에도 연다 (사용자 결정 2026-11-13)
        let discord_ok = want.starts_with("server.update.");
        let chatbot_ok = want == "instance.schedule.get";
        assert!(!find(want).unwrap().surfaces.iter().any(|s| (matches!(s, Chatbot) && !chatbot_ok) || (matches!(s, Discord) && !discord_ok)), "{want}");
    }

    let got = query(&state, "instance.get", Some(&id), &json!({}), &owner).await.unwrap();
    let text = got.to_string();
    assert_eq!(got["result"]["name"], "srv", "{got}");
    assert!(!text.contains("S3CRET-MOD"), "{text}");
    let schedule = query(&state, "instance.schedule.get", Some(&id), &json!({}), &owner).await.unwrap();
    assert!(schedule["result"]["schedule"].is_array(), "{schedule}");
    let modules = query(&state, "module.list", None, &json!({}), &owner).await.unwrap();
    assert!(modules.to_string().contains("plain"), "{modules}");

    // 이름 바꾸기는 묻지 않고, 콘솔 명령·백업 지우기는 늘 묻는다
    let rename = plan(&state, "instance.rename", Some(&id), &json!({ "name": "renamed" }), &owner).await.unwrap();
    assert_eq!(rename["needsConfirm"], false);
    let ran = run(&state, "", None, &Value::Null, rename["planId"].as_str(), &owner).await.unwrap();
    assert_eq!(ran["success"], true, "{ran}");
    assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().name, "renamed");
    for (action, input) in [("server.console.send", json!({ "command": "say hi" })), ("backup.delete", json!({ "name": "b1" }))] {
        assert_eq!(plan(&state, action, Some(&id), &input, &owner).await.unwrap()["needsConfirm"], true, "{action}");
    }
    // 백업 이름은 백업 목록에서 고른다
    let delete = &listed["actions"].as_array().unwrap().iter().find(|a| a["id"] == "backup.delete").unwrap()["input"];
    assert_eq!(delete["properties"]["name"]["optionsFrom"]["action"], "backup.list");
}

/// 원격 MCP는 게임 설정 원문을 비밀값째 읽고 쓰며, 예약은 늘 주인에게 묻고 저장한다 (사용자 결정 2026-11-05)
#[tokio::test]
async fn an_mcp_owner_reads_raw_game_settings_and_schedules_with_confirmation() {
    let (state, id, _tmp) = server().await;
    let owner = Caller::mcp("owner".into(), "owner".into(), None, None, "en".into());
    let ids: Vec<String> = list(&state, Some(Gui), None, "en").await["actions"].as_array().unwrap().iter().filter_map(|a| a["id"].as_str().map(str::to_string)).collect();
    assert!(ids.iter().any(|i| i == "instance.properties.get") && ids.iter().any(|i| i == "instance.properties.set"));
    for want in ["instance.properties.get", "instance.properties.set"] {
        assert!(!find(want).unwrap().surfaces.iter().any(|s| matches!(s, Discord | Chatbot)), "{want}");
    }
    // 게임 설정 쓰기는 PC 전용 칸 규칙을 받지 않는다 (웹 원격·짝 맺은 PC는 그대로 받는다)
    let set = plan(&state, "instance.properties.set", Some(&id), &json!({ "settings": { "admin_pass": "x" } }), &owner).await;
    assert!(set.is_ok(), "{set:?}");
    let paired = Caller::local(crate::audit::Origin::new("remote"), "en".into());
    let err = plan(&state, "instance.properties.set", Some(&id), &json!({ "settings": { "admin_pass": "x" } }), &paired).await.unwrap_err();
    assert_eq!(err.code, "remote_field_blocked", "{err:?}");
    // 읽기 결과에서 비밀값을 지우지 않는다 (MCP만)
    assert!(!strips_secrets("instance.properties.get", &owner));
    assert!(strips_secrets("instance.properties.get", &relay("owner", false)));
    assert!(strips_secrets("instance.get", &owner));
    // 길드 설정은 PC 주인이 고른 길드로 (앱과 같다). 디스코드에서 온 요청은 그 길드뿐이다
    let guild = plan(&state, "discord.settings.set", None, &json!({ "guild": "123456789012345678", "locale": "ko" }), &owner).await;
    assert!(guild.is_ok(), "{guild:?}");
    let err = plan(&state, "discord.settings.set", None, &json!({ "locale": "ko" }), &owner).await.unwrap_err();
    assert_eq!(err.data["field"], "guild");
    let err = plan(&state, "discord.settings.set", None, &json!({ "guild": "other", "locale": "ko" }), &relay("owner", false)).await.unwrap_err();
    assert_eq!(err.data["field"], "guild");
    // 예약 바꾸기는 MCP에서 늘 묻는다
    let schedule = plan(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [] }), &owner).await.unwrap();
    assert_eq!(schedule["needsConfirm"], true);
}

/// 확인 전 권한 검사 (릴레이 `check`): 계획과 같은 판정을 지나되 계획을 남기지 않는다.
/// 권한이 없으면 확인 버튼을 만들기 전에 거절을 받는다
#[tokio::test]
async fn a_check_judges_permission_without_leaving_a_plan() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "defaultLevel": 0, "members": { "admin": 2 } })).await;

    let err = check(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("guest", false)).await.unwrap_err();
    assert_eq!(err.code, "PERMISSION_DENIED");
    let ok = check(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("admin", false)).await.unwrap();
    assert_eq!((ok["success"].clone(), ok["actionId"].clone(), ok["instance"].clone()), (json!(true), json!("backup.restore"), json!(id)), "{ok}");
    assert!(ok.get("planId").is_none(), "{ok}");

    // 문장: 실행할 것이면 그 액션을, 권한이 없으면 계획과 같은 거절 답을
    let denied = check_text(&state, "srv stop", &relay("guest", false)).await.unwrap();
    assert_eq!((denied["success"].clone(), denied["code"].clone()), (json!(false), json!("PERMISSION_DENIED")), "{denied}");
    let ok = check_text(&state, "srv stop", &relay("owner", false)).await.unwrap();
    assert_eq!((ok["success"].clone(), ok["actionId"].clone()), (json!(true), json!("server.stop")), "{ok}");
    assert!(ok.get("planId").is_none(), "{ok}");
    // 인스턴스 액션 이름으로 부른 문장도 같은 판정
    assert_eq!(check_text(&state, "srv 백업", &relay("guest", false)).await.unwrap()["code"], "PERMISSION_DENIED");
    assert_eq!(check_text(&state, "srv 백업", &relay("admin", false)).await.unwrap()["actionId"], "backup.create");
}

const MODULE_WITH_PORT: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"port\"\ntype = \"number\"\nlabel = \"Game port\"\nport = true\npublic = \"both\"\ndefault = 8211\n\n[[settings.fields]]\nname = \"query_port\"\ntype = \"number\"\nlabel = \"Query port\"\nport = true\npublic = \"udp\"\ndefault = 27015\n";

/// 서버 주소 (사용자 요청 2026-11-05): `/<서버> 주소`는 데몬이 릴레이에 접속한 주소와 그 서버가 밖에 여는 포트를
/// 부른 사람에게만 보인다. 길드의 "조회 결과 나만 보기"와 상관없이 늘 숨기고(`secret`), 챗봇과 문장 명령(채널에 그대로 보인다)에는 열지 않는다
#[tokio::test]
async fn server_address_is_offered_only_where_the_reply_stays_with_the_caller() {
    let def = find("server.address").unwrap();
    assert_eq!(def.surfaces, &[Gui, Cli, Discord]);
    assert!(def.secret);
    assert!(find("server.port_check").is_some_and(|d| !d.secret));
    // 공인 IP가 보이므로 기본은 유저(Lv1) 이상, 길드가 명령별로 옮길 수 있다 (사용자 결정 2026-11-07)
    assert_eq!((def.discord_level, def.level_movable), (Some(crate::chat::permissions::USER), true));

    let (state, _id, _tmp) = server_with(MODULE_WITH_PORT).await;
    let modules = state.supervisor.read().await.module_loader.discover_modules().unwrap();
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &modules);
    let item = actions.iter().find(|a| a["id"] == "server.address").unwrap();
    assert_eq!((item["secret"].clone(), item.get("private")), (json!(true), None));
    assert_eq!((item.get("level"), item["defaultLevel"].clone()), (None, json!(1)));
    assert_eq!((item["name"]["ko"].clone(), item["name"]["en"].clone()), (json!("주소"), json!("Address")));
    assert_eq!(item["name"].as_object().unwrap().len(), crate::chat::i18n::languages().len());
    assert_eq!((item["modules"].clone(), item["surfaces"].clone()), (json!(["plain"]), json!(["discord"])));
    assert!(actions.iter().filter(|a| a["id"] != "server.address").all(|a| a.get("secret").is_none()));

    // 문장 명령의 답은 채널에 보이므로 실행하지 않고, 슬래시 명령으로 다시 부르라고 안내한다 (사용자 지정 2026-11-06)
    open_everything(&state).await;
    for text in ["srv 주소", "srv address"] {
        let hint = plan_text(&state, text, &relay("owner", false)).await.unwrap();
        assert!(hint.get("planId").is_none() && hint.get("code").is_none(), "{text}: {hint}");
        assert!(shown(&hint["text"]).starts_with("[errors.slash_only") && shown(&hint["text"]).contains("name=srv"), "{text}: {hint}");
        assert_eq!(check_text(&state, text, &relay("owner", false)).await.unwrap()["text"], hint["text"], "{text}");
    }
}

/// 결과는 릴레이가 본 주소(연결돼 있을 때만)와 밖에 여는 포트. 같은 포트의 TCP·UDP는 한 줄로 합친다
#[tokio::test]
async fn server_address_joins_the_relay_address_with_the_public_ports() {
    let (state, id, _tmp) = server_with(MODULE_WITH_PORT).await;
    open_everything(&state).await;
    let caller = relay("u1", false);
    let unknown = query(&state, "server.address", Some(&id), &json!({}), &caller).await.unwrap();
    assert_eq!(unknown["result"]["address"], Value::Null, "{unknown}");
    assert_eq!(unknown["result"]["running"], false);
    let ports: Vec<(Value, Value)> = unknown["result"]["ports"].as_array().unwrap().iter().map(|p| (p["port"].clone(), p["label"].clone())).collect();
    assert_eq!(ports, vec![(json!(8211), json!("Game port")), (json!(27015), json!("Query port"))]);

    state.relay_agent.set_seen_address_for_test(Some("203.0.113.5"));
    let known = query(&state, "server.address", Some(&id), &json!({}), &caller).await.unwrap();
    assert_eq!(known["result"]["address"], "203.0.113.5", "{known}");
    let text = shown(&known["text"]);
    assert!(text.contains("address=203.0.113.5:8211") && text.contains("address=203.0.113.5:27015"), "{text}");
}

/// 공개 주소를 정했으면 그 도메인으로, 디스코드에 알려 주지 않게 했으면 디스코드에만 주소·포트를 빼고 알린다 (사용자 결정 2026-11-10)
#[test]
fn server_address_uses_the_public_name_and_hides_it_from_discord_when_turned_off() {
    let ports = json!([{ "field": "port", "label": "Port", "port": 25565, "protocols": ["tcp"] }]);
    let named = json!({ "address": "mc.example.com", "custom": true, "running": true, "ports": ports });
    assert_eq!(summary_of("server.address", named).unwrap(), "[actions.summary.address.item address=mc.example.com:25565]");

    let hidden_result = json!({ "instanceId": "i1", "address": "mc.example.com", "shareAddress": false, "running": true, "ports": ports });
    for surface in [Gui, Cli] {
        let mut r = hidden_result.clone();
        hide_address(surface, &mut r);
        assert_eq!(r, hidden_result, "the owner's own app still sees it");
    }
    let mut r = hidden_result.clone();
    hide_address(Discord, &mut r);
    assert_eq!(r, json!({ "instanceId": "i1", "hidden": true }));
    let text = summary_of("server.address", r).unwrap();
    assert_eq!(text, "[actions.summary.address.hidden]");
    let mut shared = json!({ "shareAddress": true, "address": "mc.example.com", "ports": ports });
    hide_address(Discord, &mut shared);
    assert_eq!(shared["address"], "mc.example.com");
}

#[test]
fn server_address_reads_as_address_and_port() {
    let one = json!({ "address": "203.0.113.5", "running": true, "ports": [{ "field": "port", "label": "Port", "port": 8211, "protocols": ["tcp", "udp"] }] });
    assert_eq!(summary_of("server.address", one.clone()).unwrap(), "[actions.summary.address.item address=203.0.113.5:8211]");
    let v6 = json!({ "address": "2001:db8::1", "running": true, "ports": [{ "field": "port", "label": "Port", "port": 25565, "protocols": ["tcp"] }] });
    assert_eq!(summary_of("server.address", v6).unwrap(), "[actions.summary.address.item address=[2001:db8::1]:25565]");
    let two = json!({ "address": "203.0.113.5", "running": false, "ports": [
        { "field": "port", "label": "Game port", "port": 8211, "protocols": ["udp"] },
        { "field": "query_port", "label": "Query port", "port": 27015, "protocols": ["udp"] },
    ] });
    let text = summary_of("server.address", two).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "[actions.summary.address.item_named label=Game port address=203.0.113.5:8211]");
    assert_eq!(lines[1], "[actions.summary.address.item_named label=Query port address=203.0.113.5:27015]");
    assert_eq!(lines[2], "[actions.summary.address.stopped]");
    let unknown = summary_of("server.address", json!({ "address": null, "running": true, "ports": one["ports"] })).unwrap();
    assert_eq!(unknown, "[actions.summary.address.unknown]");
    let no_ports = summary_of("server.address", json!({ "address": "203.0.113.5", "running": true, "ports": [] })).unwrap();
    assert_eq!(no_ports, "[actions.summary.address.no_ports]");
}

/// 사용 모드 (사용자 지정 2026-11-08): 서버 공용은 레벨만 따르고(시작·정지·재시작·백업은 기본 유저), 전용 멤버는
/// 명단에 있는 사람만 한다. 명단 밖은 관리자도 거절되고 거절에 `members_only`가 실린다
#[tokio::test]
async fn members_servers_listen_only_to_their_members_and_shared_servers_follow_levels() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "default", "defaultLevel": 0, "members": { "admin": 2, "user": 1, "banned": -1 } })).await;
    let set = |access: crate::controllers::Access| {
        let (state, id) = (state.clone(), id.clone());
        async move {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.access = access;
            inst.controllers = vec![
                crate::controllers::Controller { name: "Steve".into(), discord_id: Some("friend".into()) },
                crate::controllers::Controller { name: "Bad".into(), discord_id: Some("banned".into()) },
            ];
            sup.instance_store.update(&id, inst).unwrap();
        }
    };
    // 서버 공용: 명단은 레벨을 넘지 않는다
    set(crate::controllers::Access::Shared).await;
    for action in crate::controllers::CONTROLLED_ACTIONS {
        assert_eq!(find(action).unwrap().risk, "safe", "{action} is not a risk tier");
        assert!(plan(&state, action, Some(&id), &json!({}), &relay("user", false)).await.is_ok(), "{action}: a user by default");
        let refused = plan(&state, action, Some(&id), &json!({}), &relay("friend", false)).await.unwrap_err();
        assert_eq!((refused.data["args"]["need"].clone(), refused.data["args"]["members_only"].clone()), (json!(1), Value::Null), "{action}");
    }
    // 전용 멤버: 명단에 있는 사람만 (레벨 상관없이), 관리자도 명단 밖이면 거절, 금지는 그대로 금지
    set(crate::controllers::Access::Members).await;
    for action in crate::controllers::CONTROLLED_ACTIONS {
        assert!(plan(&state, action, Some(&id), &json!({}), &relay("friend", false)).await.is_ok(), "{action}: a member");
        let refused = plan(&state, action, Some(&id), &json!({}), &relay("admin", false)).await.unwrap_err();
        assert_eq!(refused.data["args"]["members_only"], json!(true), "{action}: admins are not members");
        assert!(plan(&state, action, Some(&id), &json!({}), &relay("banned", false)).await.is_err(), "{action}: banned stays banned");
        // 말하는 사바쨩을 거치면 챗봇 상한이 그대로다
        let capped = plan(&state, action, Some(&id), &json!({}), &relay("friend", true)).await.unwrap_err();
        assert_eq!(capped.data["args"]["members_only"], Value::Null, "{action}: the chatbot cap, not the list");
    }
    // 권한 계단은 이 넷을 조작 가능 줄로 표시한다
    let board = permission_board(&state, "g", "ko").await;
    let start = board["items"].as_array().unwrap().iter().find(|i| i["key"] == format!("{id}/start")).cloned().unwrap();
    assert_eq!((&start["level"], &start["controllers"]), (&json!(1), &json!(true)));
}

/// 저장 후 정지를 요청할 길이 없는 서버(MK10)도 정지를 부르면 묻지 않고 바로 강제 정지한다: 이 PC 화면도 디스코드도 (사용자 2026-11-13)
#[cfg(unix)]
#[tokio::test]
async fn a_stop_of_a_server_that_cannot_save_stops_it_anyway() {
    let lifecycle = "import json, os, signal, sys\n\
config = json.load(sys.stdin)\n\
if sys.argv[1] == 'stop' and not config.get('force'):\n    \
print(json.dumps({'success': False, 'error_code': 'graceful_stop_unavailable', 'message': 'no console'}))\n\
elif sys.argv[1] == 'stop':\n    os.kill(config['pid'], signal.SIGKILL)\n    print(json.dumps({'success': True}))\n\
else:\n    print(json.dumps({'success': True}))\n";
    let (state, id, _tmp) = server_running(MODULE, lifecycle).await;
    set_node(&state, json!({ "ownerDiscordId": "owner" })).await;
    for caller in [gui(), relay("owner", false)] {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track(&id, child.id()).unwrap();
        let planned = plan(&state, "server.stop", Some(&id), &json!({}), &caller).await.unwrap();
        let out = run(&state, "server.stop", Some(&id), &json!({}), planned["planId"].as_str(), &caller).await.unwrap();
        assert_eq!(out["success"], true, "{out}");
        assert!(child.wait().is_ok());
    }
}

/// 챗봇이 고른 정지·재시작은 저장하지 못하는 서버도 바로 끝내므로 되묻기 단계와 상관없이 묻는다. "묻지 않음"이면 묻지 않는다
#[tokio::test]
async fn the_chatbot_always_asks_before_a_stop_unless_asking_is_off() {
    let (state, id, _tmp) = server().await;
    for level in ["all", "caution", "danger"] {
        set_confirm(&state, "danger", level).await;
        for action in ["server.stop", "server.restart"] {
            assert_eq!(plan(&state, action, Some(&id), &json!({}), &relay("owner", true)).await.unwrap()["needsConfirm"], true, "{level} {action}");
        }
    }
    set_confirm(&state, "danger", "none").await;
    assert_eq!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("owner", true)).await.unwrap()["needsConfirm"], false);
    // 시작은 단계를 따른다
    set_confirm(&state, "danger", "danger").await;
    assert_eq!(plan(&state, "server.start", Some(&id), &json!({}), &relay("owner", true)).await.unwrap()["needsConfirm"], false);
}

/// heartbeat `permissions[guild].required`: 릴레이가 규칙을 따로 갖지 않게 데몬이 판정과 같은 함수로 요구 레벨을 싣는다.
/// 덮어쓰기가 없는 서버의 값(`actions`·`commands`)과, 서버마다 그와 다른 것(`servers`)만 싣는다
#[test]
fn the_required_snapshot_carries_the_levels_authorize_would_ask() {
    let action = |id: &str, kind: &str, risk: &str, scope: &str| json!({ "id": id, "kind": kind, "risk": risk, "scope": scope });
    let targets = RemoteTargets {
        actions: vec![
            action("server.start", "command", "safe", "instance"),
            action("server.status", "query", "safe", "instance"),
            action("backup.restore", "admin", "danger", "instance"),
            action("module.install", "admin", "caution", "none"),
            action("server.query:tps", "query", "safe", "instance"),
        ],
        commands: vec![("mc".into(), vec![("say".into(), "safe".into()), ("kick".into(), "danger".into())])],
        instances: vec![("i1".into(), "mc".into()), ("i2".into(), "mc".into()), ("i3".into(), "pal".into())],
    };
    // 엄격 프리셋: 시작은 위험도가 아니라 유저부터다 (릴레이가 따로 셈하다 어긋났던 값)
    let strict = NodePermissions::from_node(Some(&json!({ "preset": "strict" })));
    let plain = required_snapshot(&strict, None, &targets);
    assert_eq!(plain["actions"], json!({ "server.start": 1, "server.status": 1, "backup.restore": 3, "module.install": 3, "server.query:tps": 1 }));
    assert_eq!(plain["commands"], json!({ "mc": { "say": 1, "kick": 3 } }));
    assert_eq!(plain["servers"], json!({}), "no overrides, nothing per server");

    // 덮어쓰기: 내장 명령 키(`i1/start`)는 그 액션에, 모듈 명령 키는 그 서버의 명령에만 닿는다. 서버 없는 액션은 최종값을 싣는다
    let levels = json!({ "i1/start": 2, "i1/kick": 2, "i2/server.status": 3, "module.install": 2, "i9/say": 3 });
    let set = NodePermissions::from_node(Some(&json!({ "commandLevels": levels })));
    let snap = required_snapshot(&set, Some(&["i1", "i2"]), &targets);
    assert_eq!(snap["actions"]["module.install"], 2);
    assert_eq!(snap["actions"]["server.start"], 1);
    assert_eq!(snap["commands"]["mc"], json!({ "say": 0, "kick": 2 }));
    assert_eq!(snap["servers"], json!({ "i1": { "actions": { "server.start": 2 } }, "i2": { "actions": { "server.status": 3 } } }), "only what differs; kick already is 2 by default");
    let unlisted = required_snapshot(&set, Some(&["i2"]), &targets);
    assert!(unlisted["servers"].get("i1").is_none(), "servers the guild cannot see are left out");
}

/// 말하는 사바쨩의 예약 편집 (사용자 결정 2026-11-13): 목록을 읽을 수 있고, 읽은 판(`revision`)을 실어야 바꾼다.
/// 확인 글은 달라지는 작업만 적고, 확인을 기다리는 사이 다른 곳에서 바뀐 일정은 덮어쓰지 않는다
#[tokio::test]
async fn the_chatbot_reads_the_schedule_and_changes_it_only_from_what_it_read() {
    let (state, id, _tmp) = server().await;
    open_everything(&state).await;
    let chatbot = relay("owner", true);
    let listed = list(&state, Some(Chatbot), None, "en").await;
    let offered = |want: &str| listed["actions"].as_array().unwrap().iter().any(|a| a["id"] == want);
    assert!(offered("instance.schedule.get") && offered("instance.schedule.set"));

    let restart = json!({ "action": "restart", "at": "04:00", "enabled": true });
    let saved = run(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [restart] }), None, &gui()).await.unwrap();
    assert_eq!(saved["success"], true, "{saved}");
    let read = query(&state, "instance.schedule.get", Some(&id), &json!({}), &chatbot).await.unwrap();
    assert_eq!(read["result"]["schedule"], json!([restart]), "{read}");
    // 실행 이력은 길어서 모델에 주지 않는다 (일정과 판이 잘리지 않게)
    assert!(read["result"].get("history").is_none(), "{read}");
    let revision = read["result"]["revision"].clone();

    let later = json!({ "action": "restart", "at": "05:00", "enabled": true });
    let blind = plan(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [later] }), &chatbot).await.unwrap_err();
    assert_eq!((blind.code, &blind.data["field"]), ("invalid_input", &json!("revision")), "the chatbot must read first");
    let stale = plan(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [later], "revision": "old" }), &chatbot).await.unwrap_err();
    assert_eq!((stale.status, stale.code), (409, "schedule_changed"));
    // 여러 단계 계획으로 돌아가도 같다
    let steps = json!([{ "id": "instance.schedule.set", "instance": id, "input": { "schedule": [later] } }]);
    assert_eq!(plan_steps(&state, &steps, &chatbot).await.unwrap_err().code, "invalid_input");

    let made = plan(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [later], "revision": revision }), &chatbot).await.unwrap();
    assert_eq!(made["text"][0]["key"], "actions.summary.schedule.changed", "{made}");
    // 이 PC 화면의 계획에는 원격용 글을 싣지 않는다
    let local = plan(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [later] }), &gui()).await.unwrap();
    assert!(local.get("text").is_none(), "{local}");

    // 확인을 기다리는 사이 앱에서 일정이 바뀌면 덮어쓰지 않는다
    let saved = run(&state, "instance.schedule.set", Some(&id), &json!({ "schedule": [] }), None, &gui()).await.unwrap();
    assert_eq!(saved["success"], true, "{saved}");
    let ran = run(&state, "", None, &Value::Null, made["planId"].as_str(), &pressed("owner")).await.unwrap();
    assert_eq!((&ran["success"], &ran["status"], &ran["result"]["error_code"]), (&json!(false), &json!(409), &json!("schedule_changed")), "{ran}");
    let now = query(&state, "instance.schedule.get", Some(&id), &json!({}), &gui()).await.unwrap();
    assert_eq!(now["result"]["schedule"], json!([]), "{now}");
    assert!(now["result"]["history"].is_array(), "the app still gets the run history");
}

/// 챗봇에 싣는 예약 작업 모양(`schedule.items`)은 실제 작업 구조와 같은 칸을 가진다
#[test]
fn the_schedule_item_schema_names_the_fields_of_a_task() {
    let (actions, _) = heartbeat_actions(&["en"], &[]);
    let items = actions.iter().find(|a| a["id"] == "instance.schedule.set").unwrap()["input"]["properties"]["schedule"]["items"].clone();
    let full = json!({
        "action": "command", "every_minutes": 30, "at": "04:00", "every_days": 2, "start_date": "2026-01-01", "weekdays": ["mon"], "date": "2026-01-01",
        "warn_minutes": [5], "message": "m", "command": "say", "args": { "a": 1 }, "enabled": true
    });
    let task: crate::scheduler::ScheduledTask = serde_json::from_value(full.clone()).unwrap();
    let mut fields: Vec<String> = serde_json::to_value(&task).unwrap().as_object().unwrap().keys().cloned().collect();
    let mut declared: Vec<String> = items["properties"].as_object().unwrap().keys().cloned().collect();
    fields.sort();
    declared.sort();
    assert_eq!(declared, fields);
    let all = crate::scheduler::TaskAction::Command;
    assert!(items["properties"]["action"]["enum"].as_array().unwrap().contains(&serde_json::to_value(all).unwrap()));
}
