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
    async fn players(&self, server: &ServerView) -> InternalResponse {
        self.respond(format!("players {}", server.id))
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
        remote: true,
        reply_pattern: None,
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
    ChatSettings { prefixes: vec!["!saba".into()], ..Default::default() }
}

fn discord(user: &str) -> Option<permissions::Requester> {
    Some(permissions::Requester { guild_id: Some("g1".into()), user_id: Some(user.into()), owner_id: Some("owner".into()), via_chatbot: false, ..Default::default() })
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
    // 실사용: 상태 문구에 이미 아이콘이 있어 "🟢 실행 중 🟢"처럼 두 번 찍혔다
    assert_eq!(out.matches('🟢').count(), 1, "{}", out);
}

#[tokio::test]
async fn start_via_alias_asks_the_daemon_to_start() {
    let api = fixture();
    let out = run(&api, &settings(), "마크 시작").await;
    assert_eq!(api.calls(), vec!["start i-mc"]);
    assert!(out.contains("survival"), "{}", out);
}

#[tokio::test]
async fn remote_module_failure_text_hides_addresses_secrets_and_paths() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        body: json!({ "success": false, "error_code": "game_failed", "module": "minecraft", "function": "start_server", "error": "203.0.113.7 token=hunter2 at C:\\Users\\me\\server.py", "stderr_tail": ["private traceback"] }),
    });
    for command in ["mc start", "mc stop"] {
        let remote = run(&api, &settings(), command).await;
        assert!(remote.contains("x.x.x.x") && remote.contains("token=***") && remote.contains("<path>"), "{remote}");
        for private in ["203.0.113.7", "hunter2", "Users", "private traceback"] {
            assert!(!remote.contains(private), "{remote}");
        }
        let req = ChatRequest { text: command.into(), ..Default::default() };
        let local = process(&api, &settings(), &Catalog::for_language("en"), &req).await.text;
        assert!(local.contains("203.0.113.7") && local.contains("hunter2") && local.contains("Users"), "{local}");
    }
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
async fn command_to_a_server_without_console_says_why() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::CONFLICT,
        body: json!({"error": "no console", "error_code": "no_command_path"}),
    });
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.contains("survival") && out.contains("managed mode") && !out.contains("no console"), "{}", out);
    assert!(!out.contains("Internal server error"), "{}", out);
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

/// 시작 전에 예약 범위를 피해 옮긴 포트는 디스코드 답에도 알린다 (M13. 저장값만 조용히 바뀌지 않게)
#[tokio::test]
async fn a_start_says_which_port_it_moved() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::OK,
        body: json!({"success": true, "port_moved": [{"field": "rest_port", "from": 8212, "to": 8242}]}),
    });
    let out = run(&api, &settings(), "mc start").await;
    assert!(out.contains("8212") && out.contains("8242"), "{}", out);
    // 시작이 실패해도 옮긴 것은 남았으므로 알린다
    api.reply = Some(InternalResponse {
        status: StatusCode::OK,
        body: json!({"success": false, "message": "port in use", "port_moved": [{"field": "rest_port", "from": 8212, "to": 8242}]}),
    });
    let out = run(&api, &settings(), "mc start").await;
    assert!(out.contains("port in use") && out.contains("8242"), "{}", out);
}

/// 켜기 전 점검에 막히면 점검 코드와 모듈이 준 해결 방법을 보이고, 경로가 든 문제 문구는 내보내지 않는다
#[tokio::test]
async fn start_blocked_by_a_preflight_check_shows_the_solution() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::CONFLICT,
        body: json!({"success": false, "error": "server.jar not found in C:\\Users\\me\\mc", "error_code": "preflight_failed", "issue_code": "server_jar_not_found", "solution": "Install the server again"}),
    });
    let out = run(&api, &settings(), "mc start").await;
    assert!(out.contains("server_jar_not_found") && out.contains("Install the server again"), "{}", out);
    assert!(!out.contains("C:\\Users"), "paths in the check message stay on the PC: {}", out);
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
async fn node_settings_filter_instances() {
    let api = fixture();
    let mut s = settings();
    s.node_settings = json!({
        "g1": {
            "allowedInstances": ["i-mc"]
        }
    });
    let cat = Catalog::for_language("en");
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
    let r = reply("survival kick steve", discord("someone")).await;
    assert_eq!((r.code, r.args["need"].clone()), (Some("PERMISSION_DENIED"), json!(2)));
    assert_eq!(reply("survival kick steve", discord("admin")).await.code, None);

    let mut bot = discord("owner");
    bot.as_mut().unwrap().via_chatbot = true;
    let r = reply("mc stop", bot).await;
    assert_eq!((r.code, r.args["via"].clone()), (Some("PERMISSION_DENIED"), json!("chatbot")));
    assert_eq!(reply("mc status", discord("someone")).await.code, None);

    assert_eq!(reply("survival kick steve", None).await.code, None);
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

/// 게임명(모듈 locales의 display_name)도 기본 호출명이다. 같은 게임의 서버가 둘이면 고르지 않는다
#[tokio::test]
async fn game_name_calls_the_only_instance_of_that_game() {
    let mut api = fixture();
    api.modules[1].game_names = vec!["Palworld".into(), "パルワールド".into()];
    run(&api, &settings(), "パルワールド start").await;
    run(&api, &settings(), "palworld start").await;
    assert_eq!(api.calls(), vec!["start i-pal", "start i-pal"]);

    api.servers.push(server("i-pal2", "pal2", "palworld", "stopped"));
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &settings(), "パルワールド start").await, cat.t("errors.multiple_instances", &[("module", "palworld")]));
    assert_eq!(api.calls().len(), 2);
}

/// 위험 명령은 게임명·모듈 별명으로 부르면 실행하지 않는다: 인스턴스 이름·id나 그 서버 낱말로 불러야 한다
#[tokio::test]
async fn danger_commands_need_the_instance_name_not_the_game_name() {
    let mut api = fixture();
    let mut wipe = field("wipe", "console");
    wipe.console_template = Some("wipe".into());
    wipe.risk = Some("danger".into());
    api.modules[0].commands.push(wipe);
    api.modules[0].game_names = vec!["마인크래프트".into()];
    let mut s = settings();
    s.node_settings = json!({ "g1": { "members": { "u1": 2 } } });
    s.server_words = serde_json::from_value(serde_json::json!({ "i-mc": { "words": ["야생"] } })).unwrap();
    let cat = Catalog::for_language("en");
    let want = cat.t("errors.danger_needs_name", &[("name", "survival")]);
    for by_game in ["마인크래프트 wipe", "mc wipe", "minecraft wipe"] {
        assert_eq!(run(&api, &s, by_game).await, want, "{by_game}");
    }
    assert!(api.calls().is_empty());
    run(&api, &s, "survival wipe").await;
    run(&api, &s, "i-mc wipe").await;
    run(&api, &s, "야생 wipe").await;
    assert_eq!(api.calls().len(), 3, "{:?}", api.calls());
    // 위험하지 않은 명령은 게임명으로도 된다
    run(&api, &s, "마인크래프트 start").await;
    assert_eq!(api.calls()[3], "start i-mc");
}

/// 두 모듈이 같은 게임명을 가져도 인스턴스 이름이 정확히 맞으면 그 서버다 (이름이 별명보다 먼저)
#[tokio::test]
async fn an_instance_name_wins_over_a_game_name_two_modules_share() {
    let mut api = fixture();
    api.modules[0].game_names = vec!["Shared".into()];
    api.modules[1].game_names = vec!["Shared".into()];
    api.servers[1].name = "shared".into();
    run(&api, &settings(), "Shared start").await;
    assert_eq!(api.calls(), vec!["start i-pal"]);
}

#[tokio::test]
async fn instance_words_pick_one_of_several_instances() {
    let mut api = fixture();
    api.servers.push(server("i-mc2", "creative", "minecraft", "running"));
    let mut s = settings();
    s.server_words = serde_json::from_value(serde_json::json!({ "i-mc2": { "words": ["크리", "cr"], "commands": { "kick": ["내보내"] } } })).unwrap();
    run(&api, &s, "CR start").await;
    run(&api, &s, "크리 start").await;
    assert_eq!(api.calls(), vec!["start i-mc2", "start i-mc2"]);
    // 그 서버에서 정한 명령 낱말
    run(&api, &s, "크리 내보내 steve").await;
    assert_eq!(api.calls()[2], "command i-mc2 kick [\"steve\"]");
    // 낱말은 인스턴스에 딸린다: 다른 길드에서도 같다
    let req = ChatRequest { text: "크리 start".into(), guild_id: Some("g2".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() };
    assert_eq!(process(&api, &s, &Catalog::for_language("en"), &req).await.code, None);
    assert_eq!(api.calls()[3], "start i-mc2");
    // 두 인스턴스가 같은 낱말을 쓰면 고르지 않는다
    s.server_words.insert("i-mc".into(), serde_json::from_value(serde_json::json!({ "words": ["cr"] })).unwrap());
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

#[test]
fn a_prefix_matches_without_case_only_before_a_break() {
    let p = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(strip_prefix("사바쨩, help", &p(&["사바쨩"])), "help");
    assert_eq!(strip_prefix("사바쨩: list", &p(&["사바쨩"])), "list");
    assert_eq!(strip_prefix("사바쨩", &p(&["사바쨩"])), "");
    assert_eq!(strip_prefix("사바쨩아 help", &p(&["사바쨩"])), "사바쨩아 help");
    assert_eq!(strip_prefix("SABA-CHAN list", &p(&["Saba-chan"])), "list");
    // 긴 접두사가 먼저다
    assert_eq!(strip_prefix("!sab list", &p(&["!s", "!sab"])), "list");
    // 기호로 끝나는 접두사는 명령에 바로 붙어도 된다
    assert_eq!(strip_prefix("!list", &p(&["!"])), "list");
    assert_eq!(strip_prefix("!slist", &p(&["!s"])), "!slist");
}

#[tokio::test]
async fn help_shows_the_language_default_when_no_prefix_is_set() {
    let api = fixture();
    let empty = ChatSettings::default();
    let req = ChatRequest { text: "사바쨩 help".into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() };
    let ko = process(&api, &empty, &Catalog::for_language("ko"), &req).await.text;
    assert!(ko.contains("사바쨩 <"), "{}", ko);
    let req = ChatRequest { text: "".into(), ..req };
    let en = process(&api, &empty, &Catalog::for_language("en"), &req).await.text;
    assert!(en.contains("Saba-chan <"), "{}", en);
}

/// 콘솔이 없어 저장 후 종료를 못 하는 서버는 정지가 거절된다(MK10): 디스코드에서는 강제 종료하지 않고 그 이유와 이 PC에서 할 일을 알린다
#[tokio::test]
async fn a_stop_that_cannot_save_explains_itself() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::OK,
        body: json!({"success": false, "error_code": "graceful_stop_unavailable", "error": "no console"}),
    });
    let out = run(&api, &settings(), "mc stop").await;
    let cat = Catalog::for_language("en");
    assert_eq!(out, cat.t("server.graceful_stop_unavailable", &[("name", "survival")]));
    assert!(!out.contains("no console"), "{out}");
}

// ── 명령 결과 문구 (사용자 item 4) ──

/// 모듈 locales를 담은 임시 폴더 (`<dir>/locales/<lang>.json`)
fn module_texts(lang: &str, body: Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("locales")).unwrap();
    std::fs::write(dir.path().join("locales").join(format!("{lang}.json")), body.to_string()).unwrap();
    dir
}

fn console_reply(response: &str) -> InternalResponse {
    InternalResponse { status: StatusCode::OK, body: json!({ "success": true, "protocol": "stdin", "data": { "response": response, "executed": "x" } }) }
}

/// 서버 명령은 성공·실패와 명령 이름만 보인다. 콘솔 원문·명령 문법(백틱)은 싣지 않는다
#[tokio::test]
async fn a_console_command_reply_is_just_success_and_its_label() {
    let mut api = fixture();
    api.modules[0].commands[1].label = "Send Chat Message".into();
    api.reply = Some(console_reply("[12:00:00] [Server thread/INFO]: [Server] hi"));
    let out = run(&api, &settings(), "mc say hi").await;
    assert!(out.starts_with('✅') && out.contains("survival") && out.contains("Send Chat Message"), "{out}");
    assert!(!out.contains('`') && !out.contains("Server thread") && !out.contains('{'), "{out}");
}

/// 모듈은 명령 답을 자기 문구로 정할 수 있다: `reply_pattern`이 콘솔 답에서 뽑은 값을 locales `commands.<이름>.done`에 넣는다
#[tokio::test]
async fn a_module_defines_the_reply_from_the_console_answer() {
    let mut api = fixture();
    let texts = module_texts("en", json!({ "commands": { "say": { "label": "Chat", "done": "Seed is {{seed}}" } } }));
    api.modules[0].locale_dir = Some(texts.path().to_path_buf());
    api.modules[0].commands[1].reply_pattern = Some(r"Seed: \[(?P<seed>-?\d+)\]".into());
    api.reply = Some(console_reply("[12:00:00] [Server thread/INFO]: Seed: [-42]"));
    assert_eq!(run(&api, &settings(), "mc say hi").await, "✅ **survival** · Seed is -42");
    // 답이 모양과 맞지 않으면 기본 성공 문구
    api.reply = Some(console_reply("something else"));
    assert!(run(&api, &settings(), "mc say hi").await.contains("Chat"));
}

/// 콘솔 답이 모듈의 실패 모양과 맞으면 실패로 알린다 (`[commands] failure_patterns`, `reason` 그룹이 이유)
#[tokio::test]
async fn a_console_answer_matching_a_failure_pattern_is_a_failure() {
    let mut api = fixture();
    api.modules[0].failure_patterns = vec![r"(?P<reason>No player was found)".into()];
    api.reply = Some(console_reply("[12:00:00] [Server thread/INFO]: No player was found"));
    let out = run(&api, &settings(), "mc kick Steve").await;
    assert!(out.starts_with('❌') && out.contains("No player was found") && !out.contains("Server thread"), "{out}");
}

/// 틀린 인자는 무엇이 틀렸는지 응답 언어로 알린다 (데몬 검사 결과의 `rule`, 모델 해석 없이)
#[tokio::test]
async fn an_invalid_argument_is_explained_in_the_reply_language() {
    let mut api = fixture();
    api.reply = Some(InternalResponse {
        status: StatusCode::BAD_REQUEST,
        body: json!({ "error": "'amount' must be between 1 and 64", "error_code": "invalid_argument", "field": "reason", "rule": "out_of_range", "params": { "value": "99", "min": 1.0, "max": 64.0 } }),
    });
    let req = ChatRequest { text: "mc kick Steve 99".into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() };
    let out = process(&api, &settings(), &Catalog::for_language("ko"), &req).await.text;
    assert!(out.starts_with('❌') && out.contains("reason") && out.contains("99") && out.contains('1') && out.contains("64"), "{out}");
    assert!(!out.contains("must be"), "the English check message stays out: {out}");
}

/// `[status] players`를 선언한 모듈은 `players`(별명 포함)가 접속자 조회다. 모듈의 목록 명령과 겹치지 않는다
#[tokio::test]
async fn players_is_a_built_in_query_for_modules_that_count_players() {
    let mut api = fixture();
    api.modules[0].players = true;
    api.modules[0].command_aliases.push(("players".into(), vec!["플레이어".into()]));
    api.reply = Some(InternalResponse { status: StatusCode::OK, body: json!({ "success": true, "online": 1, "max": 20, "players": [{ "name": "Steve" }] }) });
    let out = run(&api, &settings(), "mc 플레이어").await;
    assert_eq!(api.calls(), vec!["players i-mc"]);
    assert!(out.contains("1/20") && out.contains("Steve"), "{out}");
    // 꺼진 서버는 짧은 이유로
    api.reply = Some(InternalResponse { status: StatusCode::CONFLICT, body: json!({ "success": false, "error_code": "server_not_running", "error": "The server is not running" }) });
    let out = run(&api, &settings(), "mc players").await;
    assert!(out.starts_with('❌') && !out.contains("The server is not running"), "{out}");
}

/// `remote = false` 명령은 디스코드·챗봇에서 부를 수 없다 (같은 일을 하는 데몬 기능이 있는 명령). PC에서는 그대로
#[tokio::test]
async fn a_command_kept_off_remote_surfaces_is_unknown_there() {
    let mut api = fixture();
    api.modules[0].commands[1].remote = false;
    let r = process(&api, &settings(), &Catalog::for_language("en"), &ChatRequest { text: "mc say hi".into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() }).await;
    assert_eq!(r.code, Some("UNKNOWN_COMMAND"));
    assert!(api.calls().is_empty());
    process(&api, &settings(), &Catalog::for_language("en"), &ChatRequest { text: "mc say hi".into(), ..Default::default() }).await;
    assert_eq!(api.calls().len(), 1);
    // 원격 도움말에도 보이지 않는다
    assert!(!r.args["help"].as_str().unwrap_or_default().contains(" say"), "{}", r.args["help"]);
}

fn remote_req(text: &str) -> ChatRequest {
    ChatRequest { text: text.into(), guild_id: Some("g1".into()), user_id: Some("u1".into()), remote: discord("u1"), ..Default::default() }
}

/// 해석만 하는 단계는 아무것도 실행하지 않고, 실행할 명령을 액션 id와 서버로 알려 준다 (릴레이가 계획으로 받는다)
#[tokio::test]
async fn interpreting_names_the_action_without_running_it() {
    let mut api = fixture();
    api.modules[0].players = true;
    let cat = Catalog::for_language("en");
    let cases = [
        ("마크 시작", "server.start", "i-mc"),
        ("survival stop", "server.stop", "i-mc"),
        ("mc status", "server.status", "i-mc"),
        ("mc players", "server.players", "i-mc"),
        ("mc 추방 Steve griefing", "server.command:kick", "i-mc"),
        ("팰월드 players", "server.command:players", "i-pal"),
    ];
    for (text, action, instance) in cases {
        match interpret(&api, &settings(), &cat, &remote_req(text)).await {
            Interpreted::Run(target) => {
                assert_eq!((target.action_id().as_str(), target.instance()), (action, instance), "{text}");
            }
            Interpreted::Reply(r) => panic!("{text}: {r:?}"),
        }
    }
    assert!(api.calls().is_empty(), "{:?}", api.calls());
}

/// 실행할 것이 없는 입력(도움말·목록·모르는 명령·권한 없음)은 해석 단계에서 답이 나온다
#[tokio::test]
async fn interpreting_answers_what_needs_no_run() {
    let api = fixture();
    let cat = Catalog::for_language("en");
    let answer = |text: &'static str| {
        let api = &api;
        let cat = &cat;
        async move {
            match interpret(api, &settings(), cat, &remote_req(text)).await {
                Interpreted::Reply(r) => r,
                Interpreted::Run(t) => panic!("{text} ran {}", t.action_id()),
            }
        }
    };
    assert_eq!(answer("help").await.text, run(&api, &settings(), "help").await);
    assert_eq!(answer("list").await.text, run(&api, &settings(), "list").await);
    assert_eq!(answer("mc dance").await.code, Some("UNKNOWN_COMMAND"));
    assert_eq!(answer("zzz start").await.code, Some("INSTANCE_NOT_FOUND"));
    assert!(api.calls().is_empty());
}

/// 해석한 명령을 실행하면 한 번에 처리한 것과 같은 답을 쓰고, 성공 여부를 따로 알려 준다
#[tokio::test]
async fn running_an_interpreted_command_answers_like_the_whole_command() {
    let mut api = fixture();
    let cat = Catalog::for_language("en");
    let req = remote_req("mc 추방 Steve");
    let Interpreted::Run(target) = interpret(&api, &settings(), &cat, &req).await else { panic!() };
    let done = execute(&api, &settings(), &cat, &req, &target).await;
    assert!(done.ok);
    assert_eq!(api.calls(), vec![r#"command i-mc kick ["Steve"]"#]);
    assert_eq!(done.reply.text, run(&api, &settings(), "mc 추방 Steve").await);

    api.reply = Some(InternalResponse { status: StatusCode::INTERNAL_SERVER_ERROR, body: json!({ "success": false, "error": "boom" }) });
    let req = remote_req("mc stop");
    let Interpreted::Run(target) = interpret(&api, &settings(), &cat, &req).await else { panic!() };
    let done = execute(&api, &settings(), &cat, &req, &target).await;
    assert!(!done.ok && done.reply.text.starts_with('❌'), "{:?}", done.reply);
}

/// 이미 켜져 있는 서버의 시작은 그렇게 답하고, 다 켜졌다는 알림을 기다릴 일이 아니라고 표시한다
#[tokio::test]
async fn starting_a_running_server_says_it_is_already_on() {
    let mut api = fixture();
    api.reply = Some(InternalResponse { status: StatusCode::OK, body: json!({ "success": true, "already_running": true }) });
    let cat = Catalog::for_language("ko");
    let req = remote_req("마크 시작");
    let Interpreted::Run(target) = interpret(&api, &settings(), &cat, &req).await else { panic!() };
    let done = execute(&api, &settings(), &cat, &req, &target).await;
    assert!(done.ok && done.already_running);
    assert_eq!(done.reply.text, cat.t("server.already_running", &[("name", "survival")]));
    assert_ne!(done.reply.text, "server.already_running");
}

/// 값으로 답하는 REST 조회(지표·설정 보기·목록)는 "실행됨"이 아니라 그 값을 보인다. 칸 이름은 모듈 locales
/// `commands.<명령>.output.<키>`, 비밀 칸은 가린다
#[tokio::test]
async fn a_rest_query_reply_shows_the_returned_values() {
    let mut api = fixture();
    api.servers[1].status = "running".into();
    let mut metrics = field("metrics", "rest");
    metrics.endpoint_template = Some("/v1/api/metrics".into());
    api.modules[1].commands.push(metrics);
    let texts = module_texts("en", json!({ "commands": { "metrics": { "output": { "players": "Players", "level": "Level" } } } }));
    api.modules[1].locale_dir = Some(texts.path().to_path_buf());
    let rest = |response: Value| InternalResponse {
        status: StatusCode::OK,
        body: json!({ "success": true, "protocol": "rest", "command": "metrics",
            "data": { "status": 200, "url": "http://127.0.0.1:8212/v1/api/players", "method": "GET", "response": response, "response_text": "x" } }),
    };
    api.reply = Some(rest(json!({ "players": [{ "name": "Kim", "level": 24 }] })));
    let out = run(&api, &settings(), "팰월드 metrics").await;
    assert!(out.starts_with('✅') && out.contains("**Players** (1)") && out.contains("Kim") && out.contains("Level: 24"), "{out}");
    assert!(!out.contains("8212") && !out.contains("response_text"), "{out}");

    api.reply = Some(rest(json!({ "serverfps": 59, "AdminPassword": "hunter2" })));
    let out = run(&api, &settings(), "팰월드 metrics").await;
    assert!(out.contains("• **serverfps**: 59") && out.contains("***") && !out.contains("hunter2"), "{out}");
}
