//! 게임 명령 칸 규칙 — 명령 줄을 칸에 나누고 값을 검사하는 규칙. 데몬(`src/commands`)과 CLI(`catalog`)가 함께 쓴다.
//! GUI(`saba-chan-gui/src/utils/commandSyntax.ts`)는 키 입력마다 표시해야 해서 TS 사본을 두고,
//! 데몬 시험과 함께 `tests/fixtures/command-lines.json` 표본으로 묶는다

use serde_json::{json, Value};

/// 칸 하나의 검사 기준 (모듈 `[[commands.fields.inputs]]`의 `type`·`min`·`max`·`options`)
#[derive(Debug, Clone, Copy, Default)]
pub struct Rule<'a> {
    pub kind: Option<&'a str>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub options: Option<&'a [String]>,
}

/// 칸 값이 틀린 이유. `rule`: `not_number` | `out_of_range` | `not_boolean` | `line_break` | `not_option`,
/// `params`: 이유의 기준(`value`(받은 값), `min`·`max`, `options`). 받는 쪽이 자기 언어로 쓴다
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub rule: &'static str,
    pub message: String,
    pub params: Value,
}

/// 칸 값을 검사해 보낼 모양으로 바꾼다: number는 수(정수면 정수), boolean은 참거짓(정해진 낱말만), 그 밖은 글자
pub fn check(rule: &Rule, label: &str, value: Value) -> Result<Value, Invalid> {
    let shown = match &value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let invalid = |name: &'static str, message: String, params: Value| {
        let mut params = params;
        params["value"] = json!(shown);
        Invalid { rule: name, message, params }
    };

    let value = if rule.kind == Some("number") {
        let n = match &value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
        // NaN·무한대는 범위 비교를 지나가므로 숫자로 보지 않는다
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid("not_number", format!("'{}' must be a number", label), json!({})))?;
        if rule.min.is_some_and(|min| n < min) || rule.max.is_some_and(|max| n > max) {
            return Err(invalid(
                "out_of_range",
                format!(
                    "'{}' must be between {} and {}",
                    label,
                    rule.min.map(|v| v.to_string()).unwrap_or_else(|| "-∞".into()),
                    rule.max.map(|v| v.to_string()).unwrap_or_else(|| "∞".into()),
                ),
                json!({ "min": rule.min, "max": rule.max }),
            ));
        }
        if n.fract() == 0.0 && n.abs() < 9.0e15 {
            Value::from(n as i64)
        } else {
            Value::from(n)
        }
    } else if rule.kind == Some("boolean") {
        let yes = match &value {
            Value::Bool(b) => Some(*b),
            Value::String(s) => match s.trim().to_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => Some(true),
                "false" | "off" | "no" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        };
        Value::Bool(yes.ok_or_else(|| invalid("not_boolean", format!("'{}' must be true or false", label), json!({})))?)
    } else {
        match value {
            Value::String(s) => Value::String(s),
            other => Value::String(other.to_string()),
        }
    };

    // 값 안의 줄바꿈은 콘솔에서 두 번째 명령이 된다
    if value.as_str().is_some_and(has_line_break) {
        return Err(invalid("line_break", format!("'{}' must not contain line breaks", label), json!({})));
    }

    if let Some(options) = rule.options {
        let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
        if !options.iter().any(|o| o == &text) {
            return Err(invalid("not_option", format!("'{}' must be one of: {}", label, options.join(", ")), json!({ "options": options })));
        }
    }
    Ok(value)
}

/// 칸 `count`개에 낱말을 순서대로 넣는다. 마지막 칸이 남은 낱말을 모두 받는다 (빈칸 하나로 잇는다). 낱말이 모자란 칸은 None
pub fn spread(count: usize, words: &[String]) -> Vec<Option<String>> {
    let last = count.saturating_sub(1);
    (0..count)
        .map(|i| if i == last && words.len() > i { Some(words[i..].join(" ")) } else { words.get(i).cloned() })
        .collect()
}

/// 비어 있는 값 (기본값으로 채운다)
pub fn is_blank(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
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
