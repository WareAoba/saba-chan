use super::*;
use crate::chat::permissions::Requester;
use std::sync::Arc;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\nlabel = \"pw\"\n\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\nlabel = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\narg = \"-motd={value}\"\n\n[[settings.fields]]\nname = \"players\"\ntype = \"number\"\nlabel = \"players\"\narg = \"-players={value}\"\n\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\ninputs = [{ name = \"limit\", type = \"number\", min = 1, max = 10 }]\n";
const MODULE_KO: &str = r#"{ "queries": { "players": { "label": "접속자 목록", "description": "지금 접속한 사람", "inputs": { "limit": { "label": "최대 인원" } } } } }"#;

async fn server() -> (IPCServer, String, tempfile::TempDir) {
    // 토큰은 한 번만 (병렬 테스트가 서로의 토큰을 바꾸지 않게)
    static TOKEN: std::sync::Once = std::sync::Once::new();
    TOKEN.call_once(|| {
        crate::ipc::auth::generate_and_save_token().unwrap();
    });
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let dir = modules.join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), MODULE).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(dir.join("locales").join("ko.json"), MODULE_KO).unwrap();
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
    // 복원은 디스코드 명령으로는 열려 있지 않다
    let err = plan(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("admin", false)).await.unwrap_err();
    assert_eq!(err.code, "action_not_allowed");

    let made = plan(&state, "backup.restore", Some(&id), &json!({ "name": "b" }), &relay("admin", true)).await.unwrap();
    assert_eq!(made["needsConfirm"], true);
    let plan_id = made["planId"].as_str().unwrap().to_string();
    let err = run(&state, "", None, &Value::Null, Some(&plan_id), &relay("someone", false)).await.unwrap_err();
    assert_eq!(err.code, "plan_not_yours");

    // 챗봇이 만든 계획: 사람이 확인 버튼을 눌러도 챗봇 상한을 쓴다
    set_node(&state, json!({ "ownerDiscordId": "owner", "chatbotCap": 0, "members": { "admin": 2 } })).await;
    let err = run(&state, "", None, &Value::Null, Some(&plan_id), &relay("admin", false)).await.unwrap_err();
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
    let ran = run(&state, "", None, &Value::Null, Some(&plan_id), &relay("owner", false)).await.unwrap();
    assert_eq!(ran["success"], false, "{ran}");
    assert_eq!(ran["status"], 400, "{ran}");
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
    let restore = actions.iter().find(|a| a["id"] == "backup.restore").unwrap();
    assert_eq!(restore["surfaces"], json!(["chatbot"]));
}

// 실사용: 디스코드 "/작업"에 길드 설정 작업이 "guild" 필수 칸과 함께 보였다. 길드 설정은 "/사바쨩" 전용 명령이
// 같은 액션을 부르므로 메뉴에는 싣지 않고, 원격 요청이 스스로 채우는 길드 칸은 어느 메뉴 입력에도 두지 않는다
#[test]
fn the_remote_menu_has_no_guild_settings_and_no_guild_input() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    for id in ["chatbot.settings.get", "chatbot.settings.set", "discord.settings.get", "discord.settings.set"] {
        assert!(actions.iter().all(|a| a["id"] != id), "{id} is served by the /사바쨩 commands");
        assert!(find(id).unwrap().surfaces.contains(&Discord), "{id} stays callable from Discord");
    }
    for a in &actions {
        assert!(a["input"]["properties"].get(GUILD_FIELD).is_none(), "{} asks for a guild", a["id"]);
        assert!(a["input"]["required"].as_array().is_none_or(|r| !r.iter().any(|f| f == GUILD_FIELD)), "{}", a["id"]);
    }
}

// 릴레이 사전 검사가 데몬 판정과 같은 답을 내게, 고정 레벨이 있는 액션은 그 레벨을 싣는다
#[test]
fn remote_menu_items_carry_their_fixed_level() {
    let (actions, _) = heartbeat_actions(&crate::chat::i18n::languages(), &[]);
    let item = |id: &str| actions.iter().find(|a| a["id"] == id).unwrap().clone();
    assert_eq!(item("update.apply")["level"], crate::chat::permissions::OWNER);
    assert!(item("server.start").get("level").is_none());
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
    assert!(query(&state, "instance.events", Some(&id), &json!({}), &relay("someone", false)).await.unwrap()["text"].as_str().unwrap().contains("기록이 없어요"));
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
    let owner = relay("owner", false);
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
    // 빈 lifecycle.py라 시작이 실패한다
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

    let done = run(&state, "", None, &Value::Null, planned["planId"].as_str(), &relay("owner", false)).await.unwrap();
    let players = |state: &IPCServer| {
        let state = state.clone();
        let id = id.clone();
        async move { state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.get("players").cloned() }
    };
    assert_eq!(players(&state).await, Some(json!(4)));
    // 되돌리기는 같은 요청자만, 한 번
    let undo = done["undoPlanId"].as_str().unwrap();
    assert_eq!(run(&state, "", None, &Value::Null, Some(undo), &relay("admin", false)).await.unwrap_err().code, "plan_not_yours");
    assert_eq!(run(&state, "", None, &Value::Null, Some(undo), &relay("owner", false)).await.unwrap()["success"], true);
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
    let who = |chatbot| Requester { guild_id: Some("lanes".into()), user_id: Some("owner".into()), owner_id: Some("owner".into()), via_chatbot: chatbot, ..Default::default() };
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
