use super::*;
use crate::supervisor::module_loader::CommandInput;
use axum::http::StatusCode;
use serde_json::json;
use std::sync::Mutex;

/// 호출을 기록하고 정해진 응답을 돌려주는 가짜 데몬
#[derive(Default)]
struct FakeDaemon {
    servers: Vec<ServerView>,
    modules: Vec<ModuleView>,
    reply: Option<InternalResponse>,
    calls: Mutex<Vec<String>>,
}

impl FakeDaemon {
    fn respond(&self, call: String) -> InternalResponse {
        self.calls.lock().unwrap().push(call);
        self.reply.clone().unwrap_or(InternalResponse { status: StatusCode::OK, body: json!({"success": true}) })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl DaemonApi for FakeDaemon {
    async fn servers(&self) -> Result<Vec<ServerView>, String> {
        Ok(self.servers.clone())
    }
    async fn modules(&self) -> Vec<ModuleView> {
        self.modules.clone()
    }
    async fn start(&self, server: &ServerView) -> InternalResponse {
        self.respond(format!("start {}", server.id))
    }
    async fn stop(&self, server: &ServerView) -> InternalResponse {
        self.respond(format!("stop {}", server.name))
    }
    async fn command(&self, id: &str, command: &str, args: Vec<String>) -> InternalResponse {
        self.respond(format!("command {} {} {:?}", id, command, args))
    }
}

fn server(id: &str, name: &str, module: &str, status: &str) -> ServerView {
    ServerView { id: id.into(), name: name.into(), module: module.into(), status: status.into(), pid: None }
}

fn input(name: &str, required: bool) -> CommandInput {
    CommandInput { name: name.into(), label: None, input_type: None, required: Some(required), placeholder: None, default: None, min: None, max: None, options: None }
}

fn field(name: &str, method: &str) -> CommandField {
    CommandField {
        name: name.into(),
        label: name.into(),
        description: None,
        method: Some(method.into()),
        http_method: None,
        endpoint_template: None,
        console_template: None,
        risk: None,
        inputs: vec![],
    }
}

fn fixture() -> FakeDaemon {
    let mut kick = field("kick", "rcon");
    kick.console_template = Some("kick {player} {reason}".into());
    kick.inputs = vec![input("player", true), input("reason", false)];
    let mut say = field("say", "console");
    say.console_template = Some("say {message}".into());
    say.inputs = vec![input("message", true)];
    let mut players = field("players", "rest");
    players.endpoint_template = Some("/v1/api/players".into());

    let minecraft = ModuleView {
        name: "minecraft".into(),
        module_aliases: vec!["마크".into(), "mc".into()],
        command_aliases: vec![("start".into(), vec!["시작".into()]), ("kick".into(), vec!["추방".into()])],
        commands: vec![kick, say],
        errors: HashMap::from([("server_not_running".to_string(), "MC is off".to_string())]),
        ..Default::default()
    };
    let palworld = ModuleView {
        name: "palworld".into(),
        module_aliases: vec!["팰월드".into()],
        commands: vec![players],
        ..Default::default()
    };
    FakeDaemon {
        servers: vec![server("i-mc", "survival", "minecraft", "running"), server("i-pal", "pal1", "palworld", "stopped")],
        modules: vec![minecraft, palworld],
        ..Default::default()
    }
}

fn settings() -> ChatSettings {
    ChatSettings { prefix: "!saba".into(), ..Default::default() }
}

fn discord(user: &str) -> Option<permissions::Requester> {
    Some(permissions::Requester { guild_id: Some("g1".into()), user_id: Some(user.into()), owner_id: Some("owner".into()), via_chatbot: false })
}

async fn run(api: &FakeDaemon, s: &ChatSettings, text: &str) -> String {
    let req = ChatRequest { text: text.into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() };
    process(api, s, &Catalog::for_language("en"), &req).await.text
}

#[tokio::test]
async fn help_lists_mounted_modules_with_aliases() {
    let api = fixture();
    let out = run(&api, &settings(), "").await;
    assert!(out.contains("**minecraft** (마크, mc)"), "{}", out);
    assert!(out.contains("**palworld** (팰월드)"), "{}", out);
    assert_eq!(run(&api, &settings(), "!saba help").await, out);
}

#[tokio::test]
async fn list_shows_status() {
    let out = run(&fixture(), &settings(), "list").await;
    assert!(out.contains("survival") && out.contains("🟢"), "{}", out);
    assert!(out.contains("pal1") && out.contains("⚪"), "{}", out);
}

#[tokio::test]
async fn start_via_alias_asks_the_daemon_to_start() {
    let api = fixture();
    let out = run(&api, &settings(), "마크 시작").await;
    assert_eq!(api.calls(), vec!["start i-mc"]);
    assert!(out.contains("survival"), "{}", out);
}

#[tokio::test]
async fn defined_commands_go_to_the_daemon_command_path_by_canonical_name() {
    let api = fixture();
    run(&api, &settings(), "mc 추방 Steve").await;
    run(&api, &settings(), "mc say hello there world").await;
    assert_eq!(
        api.calls(),
        vec![
            r#"command i-mc kick ["Steve"]"#,
            r#"command i-mc say ["hello", "there", "world"]"#,
        ]
    );
}

#[tokio::test]
async fn missing_argument_reply_names_the_input() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::BAD_REQUEST,
        body: json!({"error": "'player' is required", "error_code": "missing_argument", "field": "player"}),
    });
    let out = run(&api, &settings(), "mc kick").await;
    assert!(out.contains("player") && out.contains("!saba"), "{}", out);
}

#[tokio::test]
async fn not_running_reply_uses_the_module_text() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::CONFLICT,
        body: json!({"error": "not running", "error_code": "server_not_running", "error_message": "MC is off"}),
    });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.contains("survival") && out.contains("MC is off"), "{}", out);
}

#[tokio::test]
async fn module_error_text_is_used_for_failures() {
    let mut api = fixture();
    api.reply = Some(InternalResponse { status: StatusCode::OK, body: json!({"success": false, "error": "raw", "error_code": "server_not_running"}) });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.contains("MC is off"), "{}", out);

    api.reply = Some(InternalResponse { status: StatusCode::INTERNAL_SERVER_ERROR, body: json!({"error": "RCON execution failed"}) });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.starts_with("❌"), "{}", out);
}

#[tokio::test]
async fn module_command_reply_shows_the_module_message() {
    let mut api = fixture();
    api.reply = Some(InternalResponse { status: StatusCode::OK, body: json!({"success": true, "message": "월드를 저장했어요", "protocol": "module", "command": "say"}) });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.contains("월드를 저장했어요"), "{}", out);

    // stdin 경로의 message는 내부 문구라 보이지 않는다
    api.reply = Some(InternalResponse { status: StatusCode::OK, body: json!({"success": true, "message": "sent to stdin", "data": null, "protocol": "stdin"}) });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(!out.contains("stdin"), "{}", out);
}

#[tokio::test]
async fn unknown_inputs() {
    let api = fixture();
    let s = settings();
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &s, "nothing").await, cat.t("errors.unknown_input", &[]));
    let reply = run(&api, &s, "mc dance").await;
    assert!(reply.starts_with(&cat.t("errors.command_not_found", &[])), "{}", reply);
    assert!(reply.contains(&cat.t("modules.help_start", &[])), "{}", reply);
    assert!(run(&api, &s, "nothing start").await.contains("nothing"));
}

#[tokio::test]
async fn start_waiting_on_an_agreement_carries_the_list_for_the_relay() {
    let mut api = fixture();
    let agreements = json!([{"id": "eula", "label": "Minecraft EULA", "url": "https://aka.ms/MinecraftEULA", "accepted": false}]);
    api.reply = Some(InternalResponse {
        status: StatusCode::CONFLICT,
        body: json!({"success": false, "error": "Accept Minecraft EULA first", "error_code": "agreement_required", "agreements": agreements}),
    });
    let req = ChatRequest { text: "마크 시작".into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), ..Default::default() };
    let r = process(&api, &settings(), &Catalog::for_language("en"), &req).await;
    assert_eq!(r.code, Some("AGREEMENT_REQUIRED"));
    assert_eq!(r.args["agreements"], agreements);
    assert!(r.text.contains("Accept Minecraft EULA first"), "{}", r.text);
}

/// 알아보지 못한 입력은 코드가 붙는다: 릴레이가 이 코드로 모델 해석기에 묻는다. 처리한 응답에는 없다
#[tokio::test]
async fn unrecognized_input_carries_a_code_for_the_interpreter() {
    let api = fixture();
    let s = settings();
    let cat = Catalog::for_language("en");
    let reply = |text: &str| {
        let req = ChatRequest { text: text.into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), ..Default::default() };
        let (api, s, cat) = (&api, &s, &cat);
        async move { process(api, s, cat, &req).await }
    };
    let r = reply("서버 켜줘").await;
    assert_eq!((r.code, r.args["name"].as_str()), (Some("INSTANCE_NOT_FOUND"), Some("서버")));
    let r = reply("nothing").await;
    assert_eq!((r.code, r.args["command"].as_str()), (Some("UNKNOWN_COMMAND"), Some("nothing")));
    let r = reply("mc dance").await;
    assert_eq!((r.code, r.args["command"].as_str()), (Some("UNKNOWN_COMMAND"), Some("dance")));
    assert!(r.text.starts_with(&cat.t("errors.command_not_found", &[])), "{}", r.text);
    assert!(r.args["help"].as_str().is_some_and(|h| h.contains(&cat.t("modules.help_start", &[]))), "{}", r.args);
    // 모듈 이름만 쓰면 명령 안내, 권한 거부도 처리한 응답이다
    assert_eq!(reply("survival").await.code, None);
    assert_eq!(reply("help").await.code, None);
}

#[tokio::test]
async fn single_module_word_shows_module_help() {
    let out = run(&fixture(), &settings(), "survival").await;
    assert!(out.contains("!saba survival kick <player> [reason]"), "{}", out);
}

#[tokio::test]
async fn node_settings_filter_instances_and_deny_commands() {
    let api = fixture();
    let mut s = settings();
    s.node_settings = json!({
        "g1": {
            "allowedInstances": ["i-mc"],
            "memberPermissions": { "u1": { "i-mc": ["stop"] } }
        }
    });
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &s, "mc stop").await, cat.t("errors.permission_denied", &[]));
    assert_eq!(run(&api, &s, "팰월드 start").await, cat.t("errors.unknown_input", &[]));
    assert!(!run(&api, &s, "list").await.contains("pal1"));
    assert!(api.calls().is_empty());
}

/// 디스코드 요청은 레벨로 판정한다: 금지는 아무것도 못 하고, danger는 관리자부터, 챗봇 경로는 상한을 받는다.
/// 로컬(GUI·CLI) 요청은 판정하지 않는다
#[tokio::test]
async fn discord_requests_are_judged_by_level_and_local_ones_are_not() {
    let mut api = fixture();
    api.modules[0].commands[0].risk = Some("danger".into()); // kick
    let mut s = settings();
    s.node_settings = json!({ "g1": { "members": { "banned": -1, "admin": 2 } } });
    let cat = Catalog::for_language("en");
    let reply = |text: &str, remote: Option<permissions::Requester>| {
        let req = ChatRequest { text: text.into(), guild_id: Some("g1".into()), remote, ..Default::default() };
        let (api, s, cat) = (&api, &s, &cat);
        async move { process(api, s, cat, &req).await }
    };

    let r = reply("help", discord("banned")).await;
    assert_eq!((r.code, r.args["need"].clone(), r.args["level"].clone()), (Some("PERMISSION_DENIED"), json!(0), json!(-1)));
    let r = reply("mc kick steve", discord("someone")).await;
    assert_eq!((r.code, r.args["need"].clone()), (Some("PERMISSION_DENIED"), json!(2)));
    assert_eq!(reply("mc kick steve", discord("admin")).await.code, None);

    let mut bot = discord("owner");
    bot.as_mut().unwrap().via_chatbot = true;
    let r = reply("mc stop", bot).await;
    assert_eq!((r.code, r.args["via"].clone()), (Some("PERMISSION_DENIED"), json!("chatbot")));
    assert_eq!(reply("mc status", discord("someone")).await.code, None);

    assert_eq!(reply("mc kick steve", None).await.code, None);
    assert_eq!(api.calls().len(), 2);
}

#[tokio::test]
async fn ambiguous_alias_is_rejected() {
    let mut api = fixture();
    api.modules[1].module_aliases.push("mc".into());
    let out = run(&api, &settings(), "mc start").await;
    assert!(out.contains("minecraft, palworld"), "{}", out);
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn two_instances_of_one_module_need_instance_name() {
    let mut api = fixture();
    api.servers.push(server("i-mc2", "creative", "minecraft", "running"));
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &settings(), "mc start").await, cat.t("errors.multiple_instances", &[("module", "minecraft")]));
    run(&api, &settings(), "creative start").await;
    assert_eq!(api.calls(), vec!["start i-mc2"]);
}

#[tokio::test]
async fn instance_aliases_pick_one_of_several_instances() {
    let mut api = fixture();
    api.servers.push(server("i-mc2", "creative", "minecraft", "running"));
    let mut s = settings();
    s.instance_aliases = HashMap::from([("i-mc2".to_string(), "크리, cr".to_string())]);
    run(&api, &s, "CR start").await;
    run(&api, &s, "크리 start").await;
    assert_eq!(api.calls(), vec!["start i-mc2", "start i-mc2"]);
    // 두 인스턴스가 같은 별명을 쓰면 고르지 않는다
    let first = api.servers[0].id.clone();
    s.instance_aliases.insert(first, "cr".into());
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &s, "cr start").await, cat.t("errors.multiple_instances", &[("module", "minecraft")]));
}

#[test]
fn quoted_arguments_stay_together() {
    use super::split_quoted;
    assert_eq!(split_quoted(r#"give "Steve Jobs" diamond 3"#), vec!["give", "Steve Jobs", "diamond", "3"]);
    assert_eq!(split_quoted(r#""one""#), vec!["one"]);
    // 낱말 중간의 따옴표와 닫히지 않은 따옴표는 그대로
    assert_eq!(split_quoted(r#"{"text":"a b"}"#), vec![r#"{"text":"a"#, r#"b"}"#]);
    assert_eq!(split_quoted(r#""open ended"#), vec![r#""open"#, "ended"]);
    assert_eq!(split_quoted("plain  words"), vec!["plain", "words"]);
    // 따옴표 안의 공백은 그대로 둔다
    assert_eq!(split_quoted(r#"say "two  spaces" now"#), vec!["say", "two  spaces", "now"]);
    assert_eq!(super::skip_words("  mc   say  \"a  b\"", 2), "\"a  b\"");
}
