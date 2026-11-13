//! 봇 답장 문구 (`Text`)
//!
//! 데몬은 디스코드에 보낼 문장을 만들지 않는다. 문구 키와 채울 값만 묶어 보내고,
//! 문장은 릴레이가 그 길드 언어로 만든다 (문구 표: server-chan `relay-server/src/locales/daemon/`).
//! 모듈 locales처럼 PC에만 있는 글은 이미 번역된 글자(`Raw`)로 싣는다.
//!
//! JSON 모양 (릴레이 `renderDaemonText`가 읽는다):
//! - 글자: `"…"`
//! - 이어 붙이기: `[Text, …]`
//! - 문구: `{"key": "server.start_complete", "vars": {"name": Text}, "or": Text?}`.
//!   릴레이 문구 표에 키가 없으면 `or`, 그것도 없으면 키 이름을 보인다

use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Text {
    Raw(String),
    Key { key: String, vars: Vec<(String, Text)>, or: Option<Box<Text>> },
    Seq(Vec<Text>),
}

impl Text {
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key { key: key.into(), vars: Vec::new(), or: None }
    }

    /// 문구 하나와 `{{name}}` 자리 값들
    pub fn key_with(key: impl Into<String>, vars: &[(&str, &str)]) -> Self {
        vars.iter().fold(Self::key(key), |t, (k, v)| t.var(k, *v))
    }

    /// `{{name}}` 자리에 들어갈 값
    pub fn var(mut self, name: &str, value: impl Into<Text>) -> Self {
        if let Self::Key { vars, .. } = &mut self {
            vars.push((name.to_string(), value.into()));
        }
        self
    }

    /// 릴레이 문구 표에 이 키가 없을 때 보일 것 (모르는 실패 코드 등)
    pub fn or(mut self, fallback: impl Into<Text>) -> Self {
        if let Self::Key { or, .. } = &mut self {
            *or = Some(Box::new(fallback.into()));
        }
        self
    }

    pub fn seq(parts: impl IntoIterator<Item = Text>) -> Self {
        Self::Seq(parts.into_iter().collect())
    }

    /// 사이에 `sep`을 넣어 잇는다
    pub fn join(sep: &str, parts: impl IntoIterator<Item = Text>) -> Self {
        let mut out = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            if i > 0 {
                out.push(Text::from(sep));
            }
            out.push(part);
        }
        Self::Seq(out)
    }

    /// 줄바꿈으로 잇는다
    pub fn lines(parts: impl IntoIterator<Item = Text>) -> Self {
        Self::join("\n", parts)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Raw(s) => s.trim().is_empty(),
            Self::Seq(parts) => parts.iter().all(Text::is_empty),
            Self::Key { .. } => false,
        }
    }

    /// 문구 하나면 그 키
    pub fn key_name(&self) -> Option<&str> {
        match self {
            Self::Key { key, .. } => Some(key),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// 시험용으로 펼친 글: 문구는 `[키 이름=값 …]`
    #[cfg(test)]
    pub fn plain(&self) -> String {
        match self {
            Self::Raw(s) => s.clone(),
            Self::Seq(parts) => parts.iter().map(Text::plain).collect(),
            Self::Key { key, vars, .. } => {
                let vars: String = vars.iter().map(|(k, v)| format!(" {k}={}", v.plain())).collect();
                format!("[{key}{vars}]")
            }
        }
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::Raw(s)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::Raw(s.to_string())
    }
}

impl From<&String> for Text {
    fn from(s: &String) -> Self {
        Self::Raw(s.clone())
    }
}

impl Serialize for Text {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Raw(text) => s.serialize_str(text),
            Self::Seq(parts) => parts.serialize(s),
            Self::Key { key, vars, or } => {
                let mut map = s.serialize_map(None)?;
                map.serialize_entry("key", key)?;
                if !vars.is_empty() {
                    let vars: serde_json::Map<String, Value> = vars.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                    map.serialize_entry("vars", &vars)?;
                }
                if let Some(or) = or {
                    map.serialize_entry("or", or)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 릴레이가 읽는 모양: 글자는 문자열, 이어 붙인 것은 배열, 문구는 키·값·대신할 것
    #[test]
    fn serializes_to_the_shape_the_relay_renders() {
        let reason = Text::key("reasons.timeout").or(Text::key("reasons.failed"));
        let text = Text::lines([Text::key("command.failed").var("name", "mc").var("reason", reason), "끝".into()]);
        assert_eq!(
            text.to_json(),
            json!([
                { "key": "command.failed", "vars": { "name": "mc", "reason": { "key": "reasons.timeout", "or": { "key": "reasons.failed" } } } },
                "\n",
                "끝"
            ])
        );
    }
}
