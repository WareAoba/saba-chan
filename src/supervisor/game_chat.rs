//! 게임 채팅 전달: 콘솔에 찍힌 게임 채팅을 디스코드 게임 채팅 채널로 보낸다.
//!
//! - 모듈의 `[extensions.saba-core] chat_patterns`(`name`·`message` 그룹)가 콘솔 줄에 맞으면 채팅으로 본다 (`player_events.rs`가 같은 콘솔을 본다).
//! - 인스턴스 설정 `game_chat`: `off`(기본)·`all`(모든 채팅)·`keyword`(첫머리에 감지 단어 `game_chat_keyword`가 있는 채팅만, 단어는 빼고).
//! - 데몬이 보낼 문장을 다 만들어 `instance.chat {instanceId, instance, text}`로 낸다. 릴레이는 그 문장을 그대로 보낸다 (멘션 없이).
//!   이름·본문은 디스코드 서식 문자를 이스케이프하고 제어 문자를 뺀다.
//! - 반대 방향(디스코드 → 게임)은 액션 `server.chat`이 모듈 `chat_send` 함수를 부른다. 보낼 이름·본문은 [`outgoing`]이 줄바꿈·제어
//!   문자를 빼서 한 줄로 만든다(콘솔 명령을 끼워 넣지 못하게). 보낸 글이 게임 채팅으로 다시 찍혀도 디스코드로 되돌리지 않는다 ([`is_echo`])

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 이벤트에 싣는 본문의 최대 길이 (문자)
const MESSAGE_MAX: usize = 300;
/// 인스턴스 이름의 최대 길이 (문자)
const INSTANCE_MAX: usize = 64;
/// 감지 단어의 최대 길이 (문자)
pub const KEYWORD_MAX: usize = 32;
/// 게임으로 보내는 이름·본문의 최대 길이 (문자)
const OUT_NAME_MAX: usize = 32;
const OUT_TEXT_MAX: usize = 200;
/// 게임으로 보낸 글이 게임 채팅으로 다시 찍히기를 기다리는 시간
const ECHO_SECS: u64 = 30;

/// 인스턴스 설정 `game_chat` (`instance.json`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    All,
    Keyword,
}

impl Mode {
    pub fn is_off(&self) -> bool {
        *self == Mode::Off
    }

    /// PATCH 값 (null·""는 끔)
    pub fn parse(value: &serde_json::Value) -> Result<Self, &'static str> {
        match value {
            serde_json::Value::Null => Ok(Mode::Off),
            serde_json::Value::String(s) if s.trim().is_empty() => Ok(Mode::Off),
            v => serde_json::from_value(v.clone()).map_err(|_| "game_chat must be off, all or keyword"),
        }
    }
}

/// PATCH 값 `game_chat_keyword`를 다듬는다 (null은 비움)
pub fn parse_keyword(value: &serde_json::Value) -> Result<String, &'static str> {
    let keyword = match value {
        serde_json::Value::Null => "",
        serde_json::Value::String(s) => s.trim(),
        _ => return Err("game_chat_keyword must be text"),
    };
    if keyword.chars().count() > KEYWORD_MAX || keyword.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("game_chat_keyword must be one word of at most 32 characters");
    }
    Ok(keyword.to_string())
}

/// 게임 채팅 전달 설정 (인스턴스의 `game_chat`·`game_chat_keyword`)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameChat {
    pub mode: Mode,
    /// `keyword` 모드에서 채팅 첫머리에 있어야 하는 단어. 모드를 바꿔도 남겨 둔다
    pub keyword: String,
}

impl GameChat {
    /// 함께 저장할 수 있는 값인지. 감지 단어 모드에는 단어가 있어야 한다
    pub fn check(&self) -> Result<(), &'static str> {
        if self.mode == Mode::Keyword && self.keyword.is_empty() {
            return Err("game_chat_keyword is required when game_chat is keyword");
        }
        Ok(())
    }

    /// 보낼 본문. 보내지 않을 채팅이면 None
    pub fn pick<'a>(&self, message: &'a str) -> Option<&'a str> {
        let message = message.trim();
        let picked = match self.mode {
            Mode::Off => return None,
            Mode::All => message,
            Mode::Keyword => strip_keyword(message, &self.keyword)?,
        };
        (!picked.is_empty()).then_some(picked)
    }
}

/// 첫머리의 감지 단어를 떼고 남은 본문. 영문 대소문자는 가리지 않는다.
/// 단어가 글자·숫자로 끝나면 바로 뒤가 공백이거나 끝이어야 한다 (`사바`가 `사바나`에 맞지 않게). `!`처럼 기호로 끝나면 붙여 써도 된다
fn strip_keyword<'a>(message: &'a str, keyword: &str) -> Option<&'a str> {
    if keyword.is_empty() {
        return None;
    }
    let head = message.get(..keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &message[keyword.len()..];
    let needs_gap = keyword.chars().last().is_some_and(char::is_alphanumeric);
    if needs_gap && rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    Some(rest.trim())
}

/// 디스코드에 보낼 한 줄: `[서버] **이름**: 본문`
pub fn compose(instance: &str, player: &str, message: &str) -> String {
    format!("\\[{}\\] **{}**: {}", escape(instance, INSTANCE_MAX), escape(player, INSTANCE_MAX), escape(message, MESSAGE_MAX))
}

/// 제어 문자를 빼고 `max`자에서 자른 뒤 디스코드 서식·멘션 문자를 이스케이프한다
fn escape(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()).take(max) {
        if matches!(c, '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '<' | '#' | '-' | '[' | ']' | '(' | ')' | '@' | ':') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 게임으로 보낼 이름과 본문: 제어 문자(줄바꿈 포함)를 빼고 다듬어 자른다. 둘 중 하나라도 비면 None
pub fn outgoing(name: &str, text: &str) -> Option<(String, String)> {
    let line = |s: &str, max: usize| s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect::<String>();
    let (name, text) = (line(name, OUT_NAME_MAX), line(text, OUT_TEXT_MAX));
    (!name.is_empty() && !text.is_empty()).then_some((name, text))
}

/// 인스턴스별로 게임에 보낸 이름·본문과 보낸 때
type Sent = Mutex<HashMap<String, Vec<(String, String, std::time::Instant)>>>;

fn sent() -> &'static Sent {
    static SENT: OnceLock<Sent> = OnceLock::new();
    SENT.get_or_init(Default::default)
}

/// 게임으로 보낸 이름·본문을 기억한다 (`is_echo`가 그 메아리를 거른다)
pub fn remember_sent(instance_id: &str, name: &str, text: &str) {
    let mut sent = sent().lock().unwrap_or_else(|e| e.into_inner());
    let list = sent.entry(instance_id.to_string()).or_default();
    list.retain(|(_, _, at)| at.elapsed().as_secs() < ECHO_SECS);
    list.push((name.to_string(), text.to_string(), std::time::Instant::now()));
}

/// 콘솔 채팅이 방금 게임으로 보낸 글의 메아리인가: 채팅 본문이 보낸 본문이나 `이름: 본문`과 똑같다
/// (마인크래프트처럼 이름을 따로 찍는 게임, 테라리아 `say`처럼 서버 이름으로 `이름: 본문`을 찍는 게임).
/// 본문 일부만 같은 남의 채팅은 거르지 않는다. 맞으면 그 기억은 지운다
pub fn is_echo(instance_id: &str, message: &str) -> bool {
    let mut sent = sent().lock().unwrap_or_else(|e| e.into_inner());
    let Some(list) = sent.get_mut(instance_id) else { return false };
    list.retain(|(_, _, at)| at.elapsed().as_secs() < ECHO_SECS);
    let message = message.trim();
    let same = |name: &str, text: &str| message == text || message.strip_prefix(name).and_then(|m| m.strip_prefix(": ")) == Some(text);
    match list.iter().position(|(name, text, _)| same(name, text)) {
        Some(i) => {
            list.remove(i);
            true
        }
        None => false,
    }
}

fn current() -> &'static Mutex<HashMap<String, GameChat>> {
    static CURRENT: OnceLock<Mutex<HashMap<String, GameChat>>> = OnceLock::new();
    CURRENT.get_or_init(Default::default)
}

/// 켜진 서버가 읽을 설정을 맞춘다 (시작할 때, 설정을 바꿀 때)
pub fn remember(instance_id: &str, chat: &GameChat) {
    current().lock().unwrap_or_else(|e| e.into_inner()).insert(instance_id.to_string(), chat.clone());
}

/// 지금 설정 (모르면 끔)
pub fn settings(instance_id: &str) -> GameChat {
    current().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chat(mode: Mode, keyword: &str) -> GameChat {
        GameChat { mode, keyword: keyword.into() }
    }

    // 사용자 요청(2026-10-29): 모든 채팅을 보내거나, 첫머리에 감지 단어가 있으면 그 단어를 빼고 보낸다. 기본은 보내지 않는다
    #[test]
    fn all_sends_every_chat_and_keyword_sends_only_marked_chat_without_the_word() {
        assert_eq!(GameChat::default().pick("hello"), None, "off by default");
        assert_eq!(chat(Mode::All, "").pick("  hello  "), Some("hello"));
        assert_eq!(chat(Mode::All, "").pick("   "), None, "nothing to send");
        let kw = chat(Mode::Keyword, "!d");
        assert_eq!(kw.pick("!d 안녕하세요"), Some("안녕하세요"));
        assert_eq!(kw.pick("!D hi"), Some("hi"), "ASCII case is ignored");
        assert_eq!(kw.pick("hi !d there"), None, "the word must lead the chat");
        assert_eq!(kw.pick("!d"), None, "only the word: nothing to send");
        let word = chat(Mode::Keyword, "디코");
        assert_eq!(word.pick("디코 오늘 몇 시?"), Some("오늘 몇 시?"));
        assert_eq!(word.pick("디코야 안녕"), None, "a word ending in a letter needs a gap");
        assert_eq!(chat(Mode::Keyword, "!").pick("!hello"), Some("hello"), "a symbol may be joined");
    }

    #[test]
    fn settings_are_checked() {
        assert_eq!(Mode::parse(&json!("keyword")), Ok(Mode::Keyword));
        assert_eq!(Mode::parse(&json!("")), Ok(Mode::Off));
        assert_eq!(Mode::parse(&json!(null)), Ok(Mode::Off));
        assert!(Mode::parse(&json!("loud")).is_err());
        assert_eq!(parse_keyword(&json!(" !d ")), Ok("!d".to_string()));
        assert!(parse_keyword(&json!("two words")).is_err());
        assert!(parse_keyword(&json!("x".repeat(33))).is_err());
        assert!(chat(Mode::Keyword, "").check().is_err(), "keyword mode needs a word");
        assert!(chat(Mode::All, "").check().is_ok());
    }

    // 채팅 본문은 남이 친 글이다: 서식·멘션·가린 링크가 그대로 살지 않게 한다
    #[test]
    fn composed_line_escapes_discord_markup() {
        assert_eq!(compose("survival", "Steve", "hello"), "\\[survival\\] **Steve**: hello");
        let line = compose("s", "a_b", "@everyone **x** [click](https://e.x) <@1>\u{7}");
        assert_eq!(line, "\\[s\\] **a\\_b**: \\@everyone \\*\\*x\\*\\* \\[click\\]\\(https\\://e.x\\) \\<\\@1\\>");
        assert_eq!(compose("s", "p", &"가".repeat(400)).matches('가').count(), MESSAGE_MAX);
    }

    // 사용자 요청(2026-10-30): 디스코드 글을 콘솔 명령으로 게임에 보낸다. 줄바꿈이 남으면 다음 콘솔 명령을 끼워 넣을 수 있다
    #[test]
    fn outgoing_chat_is_one_line_without_control_characters() {
        assert_eq!(outgoing(" 김치 ", "안녕\nstop\r\n"), Some(("김치".to_string(), "안녕 stop".to_string())));
        assert_eq!(outgoing("a\u{1b}[31m", "x\ty"), Some(("a [31m".to_string(), "x y".to_string())));
        assert_eq!(outgoing("p", "가".repeat(300).as_str()).unwrap().1.chars().count(), OUT_TEXT_MAX);
        assert_eq!(outgoing("p", " \n "), None);
        assert_eq!(outgoing("", "hi"), None);
    }

    #[test]
    fn a_sent_chat_coming_back_from_the_console_is_not_forwarded_again() {
        remember_sent("echo-a", "김치", "hi");
        assert!(!is_echo("echo-b", "김치: hi"), "other servers are not affected");
        assert!(!is_echo("echo-a", "this is a different message"), "someone else's chat that only contains it is kept");
        assert!(is_echo("echo-a", "김치: hi"));
        assert!(!is_echo("echo-a", "김치: hi"), "only once");
        remember_sent("echo-a", "김치", "hi");
        assert!(is_echo("echo-a", "hi"), "games that print the name apart");
        assert!(!is_echo("echo-a", "김치: hi"), "one echo per sent chat");
    }

    #[test]
    fn running_servers_read_the_latest_settings() {
        assert_eq!(settings("game-chat-unknown"), GameChat::default());
        remember("game-chat-a", &chat(Mode::All, ""));
        assert_eq!(settings("game-chat-a").mode, Mode::All);
        remember("game-chat-a", &GameChat::default());
        assert_eq!(settings("game-chat-a").mode, Mode::Off);
    }
}
