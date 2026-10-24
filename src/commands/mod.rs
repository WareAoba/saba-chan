//! 게임 명령 해석 — 모든 인터페이스(GUI·CLI·디스코드)가 쓰는 한 곳
//!
//! `POST /api/instance/:id/command`가 받은 요청을 module.toml의 `[[commands.fields]]`
//! 정의로 해석해 "무엇을 어디로 보낼지"(`Plan`)를 만든다. 실행은 IPC 핸들러가 맡는다.
//!
//! 요청 형태 (셋 다 같은 경로):
//! - `{ "command": "give", "args": { "player": "Steve", "item": "diamond" } }` — 이름으로
//! - `{ "command": "give", "args": ["Steve", "diamond", "3"] }` — 순서대로, 마지막 칸이 남은 단어를 모두 받는다
//! - `{ "command": "give Steve diamond 3" }` — 한 줄 그대로 (데몬이 나눈다)
//!
//! 명령 찾기: 이름(대소문자 무시) → `[aliases.commands]`의 별명.
//! 정의된 명령의 `method`:
//! - `console`: `console_template`의 `{입력}` 자리를 채워 서버 콘솔(stdin)로. 예전 이름 `rcon`도 받는다 (RCON은 쓰지 않는다)
//! - `rest`: `endpoint_template` + `http_method`, 입력을 본문으로
//! - `module`(예전 이름 `dual`/`both`) 또는 생략: 모듈 `lifecycle.py`의 `command()`가 처리
//!
//! 정의에 없는 명령은 한 줄 그대로 `Plan::Raw`가 된다(콘솔 입력).
//!
//! `[commands] hook = true`인 모듈은 lifecycle `instance_commands()`로 인스턴스마다 명령을 더 준다
//! (서버에 설치된 플러그인 명령 등, `InstanceCommands`). 같은 이름이면 module.toml 쪽이 먼저다.
//! 새 실행 방식은 `Plan`에 갈래 하나와 `plan()`의 match 한 줄을 더하면 된다(슬롯).

use crate::supervisor::module_loader::{CommandField, CommandInput, ModuleMetadata};
use serde_json::{Map, Value};

/// lifecycle `instance_commands()`가 준 인스턴스별 명령
#[derive(Debug, Clone, Default)]
pub struct InstanceCommands {
    pub fields: Vec<CommandField>,
    /// 명령 이름 → 묶음 이름 (예: 그 명령을 주는 플러그인)
    pub groups: std::collections::HashMap<String, String>,
    /// 모듈이 판정한 이 인스턴스의 구성요소 (설치된 플러그인·모드 등). 데몬은 내용을 보지 않고 넘긴다
    pub components: Value,
}

#[derive(serde::Deserialize)]
struct InstanceCommandItem {
    #[serde(flatten)]
    field: CommandField,
    #[serde(default)]
    group: Option<String>,
}

impl InstanceCommands {
    /// `{commands: [명령 필드 + group], components}`를 읽는다. 모양이 틀린 항목은 건너뛴다.
    pub fn parse(value: &Value) -> Self {
        let mut out = Self { components: value.get("components").cloned().unwrap_or(Value::Null), ..Self::default() };
        for item in value.get("commands").and_then(Value::as_array).into_iter().flatten() {
            let Ok(InstanceCommandItem { mut field, group }) = serde_json::from_value::<InstanceCommandItem>(item.clone())
            else {
                continue;
            };
            // 이름·템플릿이 콘솔 한 줄을 넘으면 뺀다 (NUL은 공백이 아니라 따로 본다)
            if field.name.trim().is_empty()
                || field.name.contains(char::is_whitespace)
                || has_line_break(&field.name)
                || field.console_template.as_deref().is_some_and(has_line_break)
            {
                continue;
            }
            if field.method.as_deref() == Some("rcon") {
                field.method = Some("console".into());
            }
            field.risk = Some(risk_of(field.risk.as_deref()).to_string());
            if let Some(group) = group {
                out.groups.insert(field.name.clone(), group);
            }
            out.fields.push(field);
        }
        out
    }
}

/// module.toml 명령 뒤에 인스턴스별 명령을 붙인 메타데이터. module.toml의 이름·별명과 겹치는 것은 뺀다.
pub fn with_instance_commands(module: &ModuleMetadata, extra: &InstanceCommands) -> ModuleMetadata {
    let mut merged = module.clone();
    let commands = merged.commands.get_or_insert_with(Default::default);
    for field in &extra.fields {
        if find(module, &field.name).is_none() {
            commands.fields.push(field.clone());
        }
    }
    merged
}

/// 요청 인자
#[derive(Debug, Clone, PartialEq)]
pub enum Args {
    Named(Map<String, Value>),
    Positional(Vec<String>),
}

/// 요청 하나를 이름·인자·원문으로 나눈 것
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub name: String,
    pub args: Args,
    /// 사용자가 입력한 한 줄 (정의에 없는 명령을 그대로 보낼 때 쓴다)
    pub line: String,
}

impl Request {
    pub fn parse(command: &str, args: &Value) -> Self {
        let line = command.trim().to_string();
        let mut words = line.split_whitespace();
        let name = words.next().unwrap_or("").to_string();
        let rest: Vec<String> = words.map(str::to_string).collect();

        let args = match args {
            Value::Array(items) => Args::Positional(
                items
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                    .collect(),
            ),
            Value::Object(map) if !map.is_empty() => Args::Named(map.clone()),
            _ if !rest.is_empty() => Args::Positional(rest),
            _ => Args::Named(Map::new()),
        };
        // 인자를 따로 받은 한 단어 명령이면 원문에도 인자를 붙여 둔다
        let line = match (&args, line.contains(char::is_whitespace)) {
            (Args::Positional(items), false) if !items.is_empty() => format!("{} {}", line, items.join(" ")),
            _ => line,
        };
        Self { name, args, line }
    }
}

/// 실행 계획
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// 서버 콘솔(stdin)에 보낼 한 줄
    Console { line: String },
    Rest { endpoint: String, http_method: String, body: Map<String, Value> },
    /// lifecycle.py `command()`에 위임
    Module { command: String, args: Map<String, Value> },
    /// 정의에 없는 명령 — 콘솔 한 줄
    Raw { line: String },
}

/// 요청을 계획으로 바꾸지 못한 이유. `code`는 응답의 `error_code`가 된다.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanError {
    pub code: &'static str,
    pub field: Option<String>,
    pub message: String,
}

impl PlanError {
    fn new(code: &'static str, field: Option<&str>, message: String) -> Self {
        Self { code, field: field.map(str::to_string), message }
    }
}

/// 이름이나 별명으로 명령 정의를 찾는다.
pub fn find<'m>(module: &'m ModuleMetadata, name: &str) -> Option<&'m CommandField> {
    let fields = module.commands.as_ref().map(|c| c.fields.as_slice()).unwrap_or(&[]);
    if let Some(field) = fields.iter().find(|f| f.name.eq_ignore_ascii_case(name)) {
        return Some(field);
    }
    let canonical = module.command_aliases.iter().find_map(|(cmd, info)| {
        info.aliases.iter().any(|a| a.eq_ignore_ascii_case(name)).then_some(cmd)
    })?;
    fields.iter().find(|f| f.name.eq_ignore_ascii_case(canonical))
}

/// 요청을 계획으로. 정의된 명령이면 인자를 검사하고, 아니면 원문 그대로.
pub fn plan(module: &ModuleMetadata, req: &Request) -> Result<Plan, PlanError> {
    let Some(field) = find(module, &req.name) else {
        return Ok(Plan::Raw { line: req.line.clone() });
    };
    let inputs = bind(&field.inputs, &req.args)?;
    match field.method.as_deref().unwrap_or("module") {
        "console" | "rcon" => {
            let template = field.console_template.as_deref().unwrap_or(&field.name);
            for input in &field.inputs {
                let Some(value) = inputs.get(&input.name).and_then(Value::as_str) else { continue };
                if let Some(rule) = console_place_problem(template, &input.name, value) {
                    let label = input.label.as_deref().unwrap_or(&input.name);
                    return Err(PlanError::new("invalid_argument", Some(&input.name), format!("'{}' {}", label, rule)));
                }
            }
            Ok(Plan::Console { line: render_template(template, &inputs) })
        }
        "rest" => Ok(Plan::Rest {
            endpoint: field
                .endpoint_template
                .as_deref()
                .map(|t| render_template(t, &inputs))
                .unwrap_or_else(|| format!("/{}", field.name)),
            http_method: field.http_method.as_deref().unwrap_or("GET").to_uppercase(),
            body: inputs,
        }),
        "module" | "dual" | "both" => Ok(Plan::Module { command: field.name.clone(), args: inputs }),
        other => Err(PlanError::new(
            "unsupported_method",
            None,
            format!("Command '{}' uses unsupported method '{}'", field.name, other),
        )),
    }
}

/// 입력 칸에 값을 넣고 검사한다. 빈 칸은 기본값으로 채우고, 필수인데 없으면 오류.
/// 익스텐션 명령도 같은 검사를 쓴다.
pub fn bind(inputs: &[CommandInput], args: &Args) -> Result<Map<String, Value>, PlanError> {
    let mut given: Map<String, Value> = match args {
        Args::Named(map) => map.clone(),
        Args::Positional(words) => {
            let mut map = Map::new();
            let last = inputs.len().saturating_sub(1);
            for (i, input) in inputs.iter().enumerate() {
                let value = if i == last && words.len() > i {
                    Some(words[i..].join(" "))
                } else {
                    words.get(i).cloned()
                };
                if let Some(v) = value {
                    map.insert(input.name.clone(), Value::String(v));
                }
            }
            map
        }
    };

    let mut out = Map::new();
    for input in inputs {
        let label = input.label.as_deref().unwrap_or(&input.name);
        let value = given.remove(&input.name).filter(|v| !is_blank(v));
        let value = match value {
            Some(v) => check(input, label, v)?,
            None => match input.default.as_ref().and_then(toml_to_json) {
                Some(d) => d,
                None if input.required == Some(true) => {
                    return Err(PlanError::new(
                        "missing_argument",
                        Some(&input.name),
                        format!("'{}' is required", label),
                    ))
                }
                None => continue,
            },
        };
        out.insert(input.name.clone(), value);
    }
    Ok(out)
}

fn is_blank(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
}

fn check(input: &CommandInput, label: &str, value: Value) -> Result<Value, PlanError> {
    let invalid = |message: String| PlanError::new("invalid_argument", Some(&input.name), message);

    let value = if input.input_type.as_deref() == Some("number") {
        let n = match &value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
        .ok_or_else(|| invalid(format!("'{}' must be a number", label)))?;
        if input.min.is_some_and(|min| n < min) || input.max.is_some_and(|max| n > max) {
            return Err(invalid(format!(
                "'{}' must be between {} and {}",
                label,
                input.min.map(|v| v.to_string()).unwrap_or_else(|| "-∞".into()),
                input.max.map(|v| v.to_string()).unwrap_or_else(|| "∞".into()),
            )));
        }
        if n.fract() == 0.0 && n.abs() < 9.0e15 {
            Value::from(n as i64)
        } else {
            Value::from(n)
        }
    } else {
        match value {
            Value::String(s) => Value::String(s),
            other => Value::String(other.to_string()),
        }
    };

    // 값 안의 줄바꿈은 콘솔에서 두 번째 명령이 된다
    if value.as_str().is_some_and(has_line_break) {
        return Err(invalid(format!("'{}' must not contain line breaks", label)));
    }

    if let Some(options) = &input.options {
        let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
        if !options.iter().any(|o| o == &text) {
            return Err(invalid(format!("'{}' must be one of: {}", label, options.join(", "))));
        }
    }
    Ok(value)
}

fn toml_to_json(v: &toml::Value) -> Option<Value> {
    serde_json::to_value(v).ok()
}

/// 명령 위험도: safe·caution·danger만 그대로, 없으면 caution. 모르는 값(오타·대소문자 다름)은 danger로 다룬다
pub fn risk_of(risk: Option<&str>) -> &'static str {
    match risk {
        None => "caution",
        Some("safe") => "safe",
        Some("caution") => "caution",
        Some(_) => "danger",
    }
}

/// 콘솔 한 줄을 여러 줄로 쪼갤 수 있는 문자(`\r`, `\n`, NUL)가 있는지
pub fn has_line_break(text: &str) -> bool {
    text.contains(['\r', '\n', '\0'])
}

/// 값이 콘솔 템플릿에서 자기 자리를 넘는가. 따옴표로 감싼 자리(`"{key}"`)는 따옴표를, 뒤에 템플릿 글자가 더 있는
/// 자리는 공백을 받지 않는다 (값이 다음 인자가 되어 명령의 뜻을 바꾸지 않게). 마지막 자리는 남은 말을 모두 받는다
fn console_place_problem(template: &str, key: &str, value: &str) -> Option<&'static str> {
    let place = format!("{{{}}}", key);
    for (start, _) in template.match_indices(&place) {
        let end = start + place.len();
        let quoted = template[..start].ends_with('"') && template[end..].starts_with('"');
        if quoted {
            if value.contains('"') {
                return Some("must not contain quotes");
            }
        } else if !template[end..].trim().is_empty() && value.contains(char::is_whitespace) {
            return Some("must be one word");
        }
    }
    None
}

/// `{key}` 자리를 값으로 채우고, 남은 `{key}` 자리는 앞 공백과 함께 지운다.
/// 템플릿을 한 번만 훑는다: 넣은 값 안의 `{key}`는 다시 채우지 않는다 (값이 다른 자리를 불러오지 못하게)
pub fn render_template(template: &str, inputs: &Map<String, Value>) -> String {
    static PLACE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PLACE.get_or_init(|| regex::Regex::new(r"(\s*)\{([^{}]+)\}").expect("valid regex"));
    let line = re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[2];
        match inputs.get(key) {
            Some(value) => {
                let v = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
                format!("{}{}", &caps[1], v)
            }
            None if key.chars().all(|c| c.is_alphanumeric() || c == '_') => String::new(),
            None => caps[0].to_string(),
        }
    });
    line.trim().to_string()
}

/// 실패 응답의 오류 코드. 본문에 있으면 그것, 없으면 상태 코드로 정한다.
pub fn error_code(status: u16, body: &Value) -> String {
    if let Some(code) = body.get("error_code").and_then(|v| v.as_str()) {
        return code.to_string();
    }
    match status {
        401 | 403 => "auth_failed",
        404 => "not_found",
        409 | 503 => "server_not_running",
        _ => "internal_server_error",
    }
    .to_string()
}

#[cfg(test)]
mod tests;
