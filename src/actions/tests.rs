use super::*;
use crate::chat::permissions::Requester;
use std::sync::Arc;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\nlabel = \"pw\"\n\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\nlabel = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\nlabel = \"motd\"\narg = \"-motd={value}\"\n\n[[settings.fields]]\nname = \"players\"\ntype = \"number\"\nlabel = \"players\"\narg = \"-players={value}\"\n\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\ninputs = [{ name = \"limit\", type = \"number\", min = 1, max = 10 }]\n";
const MODULE_KO: &str = r#"{ "queries": { "players": { "label": "접속자 목록", "description": "지금 접속한 사람" } } }"#;

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
    let who = Requester { guild_id: Some("g".into()), user_id: Some(user.into()), owner_id: Some("owner".into()), via_chatbot: chatbot };
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

#[test]
fn chatbot_never_lists_deletion_shutdown_update_or_permissions() {
    for d in ACTIONS.iter().filter(|d| d.surfaces.contains(&Chatbot)) {
        for word in ["delete", "shutdown", "update", "permission", "extension"] {
            assert!(!d.id.contains(word), "{} must not be open to the chatbot", d.id);
        }
    }
    assert!(command_view(&json!({ "name": "wipe", "risk": "danger" })).unwrap()["surfaces"].as_array().unwrap().iter().all(|s| s != "chatbot"));
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
    assert!(serde_json::to_vec(&actions).unwrap().len() <= HEARTBEAT_MAX_BYTES);
}
