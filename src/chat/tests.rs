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
        rcon_template: None,
        risk: None,
        inputs: vec![],
    }
}

fn fixture() -> FakeDaemon {
    let mut kick = field("kick", "rcon");
    kick.rcon_template = Some("kick {player} {reason}".into());
    kick.inputs = vec![input("player", true), input("reason", false)];
    let mut say = field("say", "rcon");
    say.rcon_template = Some("say {message}".into());
    say.inputs = vec![input("message", true)];
    let mut players = field("players", "rest");
    players.endpoint_template = Some("/v1/api/players".into());

    let minecraft = ModuleView {
        name: "minecraft".into(),
        module_aliases: vec!["마크".into(), "mc".into()],
        command_aliases: vec![("start".into(), vec!["시작".into()]), ("kick".into(), vec!["추방".into()])],
        commands: vec![kick, say],
        errors: HashMap::from([("server_not_running".to_string(), "MC is off".to_string())]),
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

async fn run(api: &FakeDaemon, s: &ChatSettings, text: &str) -> String {
    let req = ChatRequest { text: text.into(), guild_id: Some("g1".into()), user_id: Some("u1".into()) };
    process(api, s, &Catalog::for_language("en"), &req).await
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
async fn unknown_inputs() {
    let api = fixture();
    let s = settings();
    let cat = Catalog::for_language("en");
    assert_eq!(run(&api, &s, "nothing").await, cat.t("errors.unknown_input", &[]));
    assert_eq!(run(&api, &s, "mc dance").await, cat.t("errors.command_not_found", &[]));
    assert!(run(&api, &s, "nothing start").await.contains("nothing"));
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
