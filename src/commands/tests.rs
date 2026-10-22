use super::*;
use crate::supervisor::module_loader::{CommandAliasInfo, ModuleCommands};
use serde_json::json;

fn input(name: &str, ty: &str, required: bool) -> CommandInput {
    CommandInput {
        name: name.into(),
        label: None,
        input_type: Some(ty.into()),
        required: Some(required),
        placeholder: None,
        default: None,
        min: None,
        max: None,
        options: None,
    }
}

fn field(name: &str, method: &str, inputs: Vec<CommandInput>) -> CommandField {
    CommandField {
        name: name.into(),
        label: name.into(),
        description: None,
        method: Some(method.into()),
        http_method: None,
        endpoint_template: None,
        console_template: None,
        risk: None,
        inputs,
    }
}

fn module() -> ModuleMetadata {
    let mut give = field("give", "rcon", vec![input("player", "text", true), input("item", "text", true), {
        let mut amount = input("amount", "number", true);
        amount.default = Some(toml::Value::Integer(1));
        amount.min = Some(1.0);
        amount.max = Some(64.0);
        amount
    }]);
    give.console_template = Some("give {player} {item} {amount}".into());

    let mut say = field("say", "console", vec![input("message", "text", true)]);
    say.console_template = Some("say {message}".into());

    let mut announce = field("announce", "rest", vec![input("message", "text", true)]);
    announce.http_method = Some("post".into());
    announce.endpoint_template = Some("/v1/api/announce".into());

    let mut weather = field("weather", "console", vec![{
        let mut kind = input("type", "text", true);
        kind.options = Some(vec!["clear".into(), "rain".into()]);
        kind
    }]);
    weather.console_template = Some("weather {type}".into());

    let mut meta = crate::supervisor::module_loader::parse_module_toml(
        "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
    )
    .expect("minimal module.toml");
    meta.commands = Some(ModuleCommands {
        fields: vec![give, say, announce, weather, field("kickall", "dual", vec![]), field("odd", "ftp", vec![])],
        hook: false,
    });
    meta.command_aliases.insert(
        "say".into(),
        CommandAliasInfo { aliases: vec!["말".into()], description: None },
    );
    meta
}

fn plan_of(command: &str, args: Value) -> Result<Plan, PlanError> {
    plan(&module(), &Request::parse(command, &args))
}

#[test]
fn one_line_input_fills_inputs_in_order_and_defaults_the_rest() {
    assert_eq!(
        plan_of("give Steve diamond", Value::Null),
        Ok(Plan::Console { line: "give Steve diamond 1".into() })
    );
}

#[test]
fn last_input_takes_the_remaining_words() {
    assert_eq!(
        plan_of("say hello there world", Value::Null),
        Ok(Plan::Console { line: "say hello there world".into() })
    );
}

#[test]
fn named_and_positional_args_give_the_same_plan() {
    let named = plan_of("give", json!({ "player": "Steve", "item": "stone", "amount": "3" }));
    let positional = plan_of("give", json!(["Steve", "stone", "3"]));
    assert_eq!(named, positional);
    assert_eq!(named, Ok(Plan::Console { line: "give Steve stone 3".into() }));
}

#[test]
fn aliases_and_case_find_the_command() {
    assert_eq!(plan_of("말 안녕", Value::Null), Ok(Plan::Console { line: "say 안녕".into() }));
    assert_eq!(plan_of("SAY hi", Value::Null), Ok(Plan::Console { line: "say hi".into() }));
}

#[test]
fn rest_commands_build_endpoint_method_and_body() {
    let Ok(Plan::Rest { endpoint, http_method, body }) = plan_of("announce", json!({ "message": "hi" })) else {
        panic!("expected REST plan");
    };
    assert_eq!(endpoint, "/v1/api/announce");
    assert_eq!(http_method, "POST");
    assert_eq!(Value::Object(body), json!({ "message": "hi" }));
}

#[test]
fn module_method_and_its_old_names_go_to_lifecycle() {
    assert_eq!(
        plan_of("kickall", Value::Null),
        Ok(Plan::Module { command: "kickall".into(), args: Map::new() })
    );
}

#[test]
fn unknown_commands_pass_through_as_one_line() {
    assert_eq!(plan_of("tp Steve 0 64 0", Value::Null), Ok(Plan::Raw { line: "tp Steve 0 64 0".into() }));
    assert_eq!(plan_of("tp", json!(["Steve", "Alex"])), Ok(Plan::Raw { line: "tp Steve Alex".into() }));
}

#[test]
fn invalid_input_is_rejected_with_the_field() {
    let missing = plan_of("give", Value::Null).unwrap_err();
    assert_eq!((missing.code, missing.field.as_deref()), ("missing_argument", Some("player")));

    let not_number = plan_of("give Steve stone lots", Value::Null).unwrap_err();
    assert_eq!((not_number.code, not_number.field.as_deref()), ("invalid_argument", Some("amount")));

    let out_of_range = plan_of("give Steve stone 65", Value::Null).unwrap_err();
    assert_eq!(out_of_range.code, "invalid_argument");

    let not_an_option = plan_of("weather snow", Value::Null).unwrap_err();
    assert_eq!((not_an_option.code, not_an_option.field.as_deref()), ("invalid_argument", Some("type")));
    assert_eq!(plan_of("weather rain", Value::Null), Ok(Plan::Console { line: "weather rain".into() }));

    assert_eq!(plan_of("odd", Value::Null).unwrap_err().code, "unsupported_method");
}

#[test]
fn error_codes_come_from_the_body_or_the_status() {
    assert_eq!(error_code(500, &json!({ "error_code": "player_offline" })), "player_offline");
    assert_eq!(error_code(401, &json!({})), "auth_failed");
    assert_eq!(error_code(503, &json!({})), "server_not_running");
    assert_eq!(error_code(500, &Value::Null), "internal_server_error");
}

/// 콘솔 한 줄에서 값은 자기 자리만 채운다 (입력 검증). 뒤에 다른 자리가 오는 값은 한 단어여야 하고,
/// 템플릿이 따옴표로 감싼 자리는 공백을 받되 따옴표는 받지 않는다. 마지막 자리는 남은 말을 모두 받는다
#[test]
fn a_console_value_fills_only_its_own_place() {
    let err = plan_of("give", json!({ "player": "Steve extra", "item": "diamond" })).unwrap_err();
    assert_eq!((err.code, err.field.as_deref()), ("invalid_argument", Some("player")));
    let err = plan_of("give", json!({ "player": "Steve", "item": "diamond\tx" })).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("item"));
    assert_eq!(plan_of("say", json!({ "message": "hello there" })), Ok(Plan::Console { line: "say hello there".into() }));

    let mut meta = module();
    let mut xp = field("addxp", "console", vec![input("player", "text", true), input("amount", "text", true)]);
    xp.console_template = Some("addxp \"{player}\" fitness={amount}".into());
    meta.commands.as_mut().unwrap().fields.push(xp);
    let named = |args: Value| plan(&meta, &Request::parse("addxp", &args));
    assert_eq!(named(json!({ "player": "Mr Smith", "amount": "5" })), Ok(Plan::Console { line: "addxp \"Mr Smith\" fitness=5".into() }));
    let err = named(json!({ "player": "Mr\" x \"", "amount": "5" })).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("player"));
}

/// 값 안의 `{자리}`는 다시 채우지 않는다: 앞 값이 뒤 자리를 불러와 여러 낱말 값을 중간에 넣으면 안 된다
#[test]
fn a_value_cannot_pull_another_input_into_its_place() {
    let mut meta = module();
    let mut check = field("check", "console", vec![input("group", "text", true), input("node", "text", true)]);
    check.console_template = Some("lp group {group} permission check {node}".into());
    meta.commands.as_mut().unwrap().fields.push(check);
    let line = plan(&meta, &Request::parse("check", &json!({ "group": "{node}", "node": "a b c" })));
    assert_eq!(line, Ok(Plan::Console { line: "lp group {node} permission check a b c".into() }));
}

#[test]
fn a_line_break_inside_an_argument_cannot_add_a_second_console_command() {
    for value in ["hi\nop attacker", "hi\rop attacker", "hi\0op attacker"] {
        let err = plan_of("say", json!({ "message": value })).unwrap_err();
        assert_eq!((err.code, err.field.as_deref()), ("invalid_argument", Some("message")), "{value:?}");
    }
}

#[test]
fn instance_commands_come_after_module_commands_and_bind_like_them() {
    let extra = InstanceCommands::parse(&json!({
        "commands": [
            { "name": "lp-user-info", "label": "User info", "method": "console", "console_template": "lp user {user} info",
              "risk": "safe", "group": "LuckPerms",
              "inputs": [{ "name": "user", "type": "text", "required": true }] },
            // 같은 이름(별명 포함)은 module.toml이 먼저다
            { "name": "말", "label": "shadow", "method": "console", "console_template": "nope" },
            { "name": "home", "label": "/home", "method": "rcon", "console_template": "home {args}",
              "inputs": [{ "name": "args", "type": "text", "required": false }] },
            // 모양이 틀린 항목은 건너뛴다
            { "name": "two words", "label": "x" },
            { "label": "no name" },
            { "name": "nul\u{0}x", "label": "x", "method": "console", "console_template": "x" },
            { "name": "split", "label": "x", "method": "console", "console_template": "say a\nstop" },
        ],
        "components": [{ "id": "LuckPerms", "kind": "paper" }],
    }));
    assert_eq!(extra.fields.len(), 3);
    assert_eq!(extra.groups.get("lp-user-info").map(String::as_str), Some("LuckPerms"));
    assert_eq!(extra.components[0]["id"], "LuckPerms");

    let merged = with_instance_commands(&module(), &extra);
    let plan_with = |line: &str| plan(&merged, &Request::parse(line, &Value::Null));
    assert_eq!(plan_with("lp-user-info Steve"), Ok(Plan::Console { line: "lp user Steve info".into() }));
    assert_eq!(plan_with("home"), Ok(Plan::Console { line: "home".into() }));
    assert_eq!(plan_with("home bed 2"), Ok(Plan::Console { line: "home bed 2".into() }));
    assert_eq!(plan_with("말 hi"), plan(&module(), &Request::parse("말 hi", &Value::Null)));
    assert_eq!(plan_with("lp-user-info").unwrap_err().code, "missing_argument");
}

/// 위험도는 safe·caution·danger만 그대로 둔다. 없으면 caution, 모르는 값(오타·대소문자 다름)은 danger로 다룬다
#[test]
fn unknown_risk_values_count_as_danger() {
    assert_eq!(risk_of(None), "caution");
    for (given, want) in [("safe", "safe"), ("caution", "caution"), ("danger", "danger"), ("Safe", "danger"), ("dangerous", "danger"), ("", "danger")] {
        assert_eq!(risk_of(Some(given)), want, "{given}");
    }
    let hooked = InstanceCommands::parse(&json!({ "commands": [{ "name": "wipe", "label": "Wipe", "method": "console", "console_template": "wipe", "risk": "DANGER" }] }));
    assert_eq!(hooked.fields[0].risk.as_deref(), Some("danger"));
}
