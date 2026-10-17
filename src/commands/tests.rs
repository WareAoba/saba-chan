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

#[test]
fn a_line_break_inside_an_argument_cannot_add_a_second_console_command() {
    for value in ["hi\nop attacker", "hi\rop attacker", "hi\0op attacker"] {
        let err = plan_of("say", json!({ "message": value })).unwrap_err();
        assert_eq!((err.code, err.field.as_deref()), ("invalid_argument", Some("message")), "{value:?}");
    }
}
