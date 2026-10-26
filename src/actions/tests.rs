use super::*;
use crate::chat::permissions::Requester;
use std::sync::Arc;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\nlabel = \"pw\"\n\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\nlabel = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\narg = \"-motd={value}\"\n\n[[settings.fields]]\nname = \"players\"\ntype = \"number\"\nlabel = \"players\"\narg = \"-players={value}\"\n\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\ninputs = [{ name = \"limit\", type = \"number\", min = 1, max = 10 }]\n";
const MODULE_KO: &str = r#"{ "queries": { "players": { "label": "접속자 목록", "description": "지금 접속한 사람", "inputs": { "limit": { "label": "최대 인원" } } } } }"#;

async fn server() -> (IPCServer, String, tempfile::TempDir) {
    crate::ipc::auth::ensure_test_token();
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let dir = modules.join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), MODULE).unwrap();
    std::fs::write(dir.join("lifecycle.py"), r#"import json, sys
json.load(sys.stdin)
function = sys.argv[1]
if function == "functions":
    result = {"success": True, "functions": ["configure", "read_properties"]}
elif function in ("configure", "read_properties"):
    result = {"success": True}
else:
    result = {"success": False, "error_code": "not_supported", "message": "Fixture function not supported"}
print(json.dumps(result))
"#).unwrap();
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(dir.join("locales").join("ko.json"), MODULE_KO).unwrap();
    crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
    let instances = tmp.path().join("instances");
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
    sup.initialize().await.unwrap();
    let mut inst = crate::instance::ServerInstance::new("srv", "plain");
    inst.rest_password = Some("S3CRET-REST".into());
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
    let lower = json!({ "schedule": [], "backup_keep": 1 });
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
    let modules = vec![crate::supervisor::module_loader::load_module_dir(&dir).unwrap()];
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
    let text = listed["text"].as_str().unwrap();
    assert!(text.contains("srv") && text.contains("정지됨"), "{text}");
    assert!(query(&state, "backup.list", Some(&id), &json!({}), &relay("someone", false)).await.unwrap()["text"].as_str().unwrap().contains("백업이 없어요"));
    assert!(query(&state, "instance.events", Some(&id), &json!({}), &relay("someone", true)).await.unwrap()["text"].as_str().unwrap().contains("기록이 없어요"));
    assert!(query(&state, "monitor.summary", None, &json!({}), &relay("someone", false)).await.unwrap()["text"].as_str().unwrap().contains("CPU"));
    // GUI·CLI는 자기 화면으로 그린다
    assert!(run(&state, "instance.list", None, &json!({}), None, &gui()).await.unwrap().get("text").is_none());
}

#[test]
fn summaries_read_like_sentences() {
    let ko = crate::chat::i18n::Catalog::for_language("ko");
    let backups: Vec<Value> = (0..12).map(|n| json!({ "name": format!("b{n:02}"), "kind": "scheduled", "created": 1_700_000_000 + n * 60, "size": 1_536 * 1024 * 1024 })).collect();
    let text = summary::text("backup.list", &json!({ "success": true, "backups": backups }), &ko).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("12"), "{text}");
    assert!(lines[1].contains("b11") && lines[1].contains("예약") && lines[1].contains("1.5 GB"), "newest first: {text}");
    assert!(lines.len() <= 12 && lines.last().unwrap().contains('2'), "the rest is counted: {text}");

    let updates = json!({ "ok": true, "updates_available": 1, "components": [
        { "display_name": "Saba-chan GUI", "current_version": "0.1.7", "latest_version": "0.1.8", "update_available": true },
        { "display_name": "Saba-chan CLI", "current_version": "0.1.7", "latest_version": "0.1.7", "update_available": false },
    ] });
    let text = summary::text("update.check", &updates, &ko).unwrap();
    assert!(text.contains("Saba-chan GUI") && text.contains("0.1.7 → 0.1.8") && !text.contains("CLI"), "{text}");
    assert!(summary::text("update.check", &json!({ "ok": true, "updates_available": 0, "components": [] }), &ko).unwrap().contains("최신"));

    let events = json!({ "events": [{ "at": 1_700_000_000, "kind": "crash" }, { "at": 1_700_000_600, "kind": "user_stop" }] });
    let text = summary::text("instance.events", &events, &ko).unwrap();
    assert!(text.lines().nth(1).unwrap().contains("정지") && text.contains("비정상 종료"), "newest first: {text}");

    // 시각을 모르면 1970년으로 찍지 않는다
    let unknown = summary::text("backup.list", &json!({ "backups": [{ "name": "old", "kind": "manual", "size": 0 }] }), &ko).unwrap();
    assert!(!unknown.contains("1970"), "{unknown}");

    // 게임 자동 백업은 크기를 재지 않는다: 0 B로 쓰지 않는다
    let game = summary::text("backup.list", &json!({ "backups": [{ "name": "game-snapshot:S1", "kind": "game-snapshot", "created": 1_700_000_000 }] }), &ko).unwrap();
    assert!(!game.contains("0 B") && !game.trim_end().ends_with('·'), "{game}");

    // 열지 못하는 백업은 복원할 수 없다고 함께 알린다 (603 P3-8)
    let mut broken = json!({ "backups": [{ "name": "bad", "kind": "manual", "created": 1_700_000_000, "size": 9, "readable": false }, { "name": "ok", "kind": "manual", "created": 1_699_000_000, "size": 9, "readable": true }] });
    let text = summary::text("backup.list", &broken, &ko).unwrap();
    assert!(text.lines().nth(1).unwrap().contains("열 수 없음") && !text.lines().nth(2).unwrap().contains("열 수 없음"), "{text}");
    summary::label_backups(&mut broken, &ko);
    assert!(broken["backups"][0]["label"].as_str().unwrap().contains("열 수 없음") && !broken["backups"][1]["label"].as_str().unwrap().contains("열 수 없음"), "{broken}");

    // 모듈 조회는 범용 포맷으로 (모양이 모듈마다 다르다)
    let players = summary::text("server.query:players", &json!({ "success": true, "players": [{ "name": "steve" }] }), &ko).unwrap();
    assert!(players.contains("steve") && !players.contains("success"), "{players}");
    // 모르는 액션은 요약하지 않는다 (릴레이가 결과를 그대로 보인다)
    assert!(summary::text("server.status", &json!({}), &ko).is_none());
}

#[test]
fn chatbot_never_lists_deletion_shutdown_update_or_permissions() {
    for d in ACTIONS.iter().filter(|d| d.surfaces.contains(&Chatbot)) {
        for word in ["delete", "shutdown", "update", "permission", "extension"] {
            assert!(!d.id.contains(word), "{} must not be open to the chatbot", d.id);
        }
    }
}

/// 모듈의 위험 명령도 챗봇이 고를 수 있지만 늘 확인 계획을 거친다(요청자 본인이 확인 버튼을 누른다, 사용자 결정 2026-10-20).
/// 결과는 콘솔·조회처럼 IP·비밀처럼 보이는 값을 가려 보낸다
#[test]
fn danger_module_commands_reach_the_chatbot_only_through_a_confirmation() {
    let wipe = command_view(&json!({ "name": "wipe", "risk": "danger" })).unwrap();
    assert!(wipe["surfaces"].as_array().unwrap().iter().any(|s| s == "chatbot"));
    assert_eq!(wipe["risk"], "danger", "danger always needs a confirmed plan (needs_confirm)");
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
    // java_path는 모듈이 글자 형식으로 선언해도 막는다
    assert_eq!(forbidden_remote_key(&json!({ "java_path": "x" }), &FieldRules::default()).as_deref(), Some("java_path"));
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
    let modules = vec![crate::supervisor::module_loader::load_module_dir(&tmp.path().join("modules").join("plain")).unwrap()];
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
    // 앞의 생성 단계만 가리킬 수 있고, 이름으로 찾는 액션은 안 된다
    let wrong = json!([{ "id": "server.start", "instance": "@1", "input": {} }]);
    assert_eq!(plan_steps(&state, &wrong, &owner_bot).await.unwrap_err().data["field"], "steps[0].instance");
    let by_name = json!([steps[0].clone(), { "id": "server.status", "instance": "@1", "input": {} }]);
    assert_eq!(plan_steps(&state, &by_name, &owner_bot).await.unwrap_err().data["field"], "steps[1].id");

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
    assert_eq!(run(&state, "instance.settings.set", Some(&id), &json!({ "rest_password": "OLD-PW" }), None, &gui()).await.unwrap()["success"], true);
    let row = |planned: &Value, key: &str| planned["summary"].as_array().unwrap().iter().find(|r| r["labelKey"] == format!("actions.field.{}", key)).cloned().unwrap();
    // 로컬도 요약에는 가린 값만
    let planned = plan(&state, "instance.settings.set", Some(&id), &json!({ "rest_password": "NEW-PW" }), &gui()).await.unwrap();
    assert_eq!((row(&planned, "rest_password")["before"].clone(), row(&planned, "rest_password")["value"].clone()), (json!("***"), json!("***")));
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

    let local = json!({ "guild": "g", "confirmCommands": false });
    assert_eq!(run(&state, "chatbot.settings.set", None, &local, None, &gui()).await.unwrap()["success"], true);
    let read = run(&state, "chatbot.settings.get", None, &json!({ "guild": "g" }), None, &gui()).await.unwrap();
    assert_eq!((read["result"]["settings"]["confirmCommands"].as_bool(), read["result"]["settings"]["model"].as_str()), (Some(false), Some("gpt-6-luna")));
    // null은 그 길드의 값을 지워 기본값으로 되돌린다
    run(&state, "chatbot.settings.set", None, &json!({ "guild": "g", "confirmCommands": null }), None, &gui()).await.unwrap();
    let read = run(&state, "chatbot.settings.get", None, &json!({ "guild": "g" }), None, &gui()).await.unwrap();
    assert_ne!(read["result"]["settings"]["confirmCommands"].as_bool(), Some(false));
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

/// GUI에서 사용자별로 막은 명령(`memberPermissions`)은 채팅 명령뿐 아니라 액션 메뉴·확인 버튼으로도 막힌다
#[tokio::test]
async fn commands_denied_to_a_member_stay_denied_through_actions() {
    let (state, id, _tmp) = server().await;
    set_node(&state, json!({ "ownerDiscordId": "owner", "preset": "loose", "defaultLevel": 2, "memberPermissions": { "admin": { id.clone(): ["stop"] } } })).await;
    let admin = relay("admin", false);
    let err = plan(&state, "server.stop", Some(&id), &json!({}), &admin).await.unwrap_err();
    assert_eq!(err.code, "PERMISSION_DENIED");
    assert_eq!(err.data["args"]["need"], 3);
    assert!(plan(&state, "server.start", Some(&id), &json!({}), &admin).await.is_ok(), "only the denied command");
    assert!(plan(&state, "server.stop", Some(&id), &json!({}), &relay("someone", false)).await.is_ok(), "only that member");
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
    let ko = crate::chat::i18n::Catalog::for_language("ko");
    let names: Vec<Value> = (0..12).map(|n| json!({ "name": format!("p{n}") })).collect();
    let text = summary::text("server.players", &json!({ "success": true, "online": 12, "max": 20, "players": names }), &ko).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "👥 접속 12/20명");
    assert_eq!(lines[1], "• p0");
    assert_eq!(lines.last().copied(), Some("…외 2명"));
    assert_eq!(summary::text("server.players", &json!({ "success": true, "online": 0, "players": [] }), &ko).unwrap(), "👥 접속 0명");

    let (state, _id, _tmp) = server().await;
    let mut modules = state.supervisor.read().await.module_loader.discover_modules().unwrap();
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    assert_eq!(actions.iter().find(|a| a["id"] == "server.players").unwrap()["modules"], json!([]));
    modules[0].metadata.players_function = Some("player_count".into());
    let (actions, _) = heartbeat_actions(&["ko"], &modules);
    let item = actions.iter().find(|a| a["id"] == "server.players").unwrap();
    assert_eq!((item["modules"].clone(), item["title"]["ko"].clone(), item["surfaces"].clone()), (json!(["plain"]), json!("플레이어"), json!(["discord", "chatbot"])));
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
    assert_eq!(out["reason"], "이 게임은 지원하지 않아요", "{out}");
    // 이 PC 화면은 원문을 그대로 쓴다
    let local = query(&state, "server.query:players", Some(&id), &json!({}), &gui()).await.unwrap();
    assert!(local.get("reason").is_none(), "{local}");
}

/// `remote = false` 명령은 디스코드·챗봇 액션으로도 부를 수 없다 (GUI·CLI에서만)
#[test]
fn a_command_kept_off_remote_surfaces_is_not_a_remote_action() {
    let hidden = command_view(&json!({ "name": "save-off", "risk": "caution", "inputs": [], "remote": false })).unwrap();
    assert_eq!(hidden["surfaces"], json!(["gui", "cli"]));
    let shown = command_view(&json!({ "name": "say", "risk": "safe", "inputs": [] })).unwrap();
    assert_eq!(shown["surfaces"], json!(["gui", "cli", "discord", "chatbot"]));
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
    assert_eq!(item["title"]["ko"], "켜짐 알림");
    assert_eq!(item["name"]["en"], "Start alert");
}
