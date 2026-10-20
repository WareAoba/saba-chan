//! 대화형 입력 — 터미널에서만 묻는다
//!
//! 터미널이 아니거나 `--json`이면 묻지 않는다. 그때 꼭 필요한 값이 없으면 호출한 쪽이
//! 사용법 오류를 낸다. `--yes`는 확인 질문만 건너뛴다(값을 지어내지는 않는다).
//! 묻는 창구도 바꿔 끼울 수 있다: 끼워져 있으면 터미널 대신 그 창구가 묻는다 (TUI의 대화상자).

use crate::error::{CliError, Result};
use console::Term;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};
use std::io::IsTerminal;

/// 묻는 창구. 답이 None이면 사람이 취소한 것
pub trait Asker: Send + Sync {
    fn confirm(&self, question: &str, default: bool) -> Option<bool>;
    fn select(&self, question: &str, items: &[String], default: usize) -> Option<usize>;
    fn text(&self, question: &str, default: Option<&str>, allow_empty: bool) -> Option<String>;
    fn secret(&self, question: &str) -> Option<String>;
}

static ASKER: std::sync::RwLock<Option<Box<dyn Asker>>> = std::sync::RwLock::new(None);

/// 창구는 하나라서, 창구를 끼우는 시험은 이 잠금을 잡고 차례로 돈다
#[cfg(test)]
pub static ASKER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 끼워 둔 창구로 묻는다. 없으면 None
fn ask_with<T>(f: impl FnOnce(&dyn Asker) -> T) -> Option<T> {
    ASKER.read().ok().as_ref().and_then(|a| a.as_deref()).map(f)
}

/// 창구를 바꿔 끼운다. 놓으면 터미널로 돌아간다 (TUI가 끼운다. 그 전까지는 시험만 쓴다)
#[must_use]
#[cfg_attr(not(test), allow(dead_code))]
pub struct AskerGuard(());

#[cfg_attr(not(test), allow(dead_code))]
impl AskerGuard {
    pub fn install(asker: Box<dyn Asker>) -> Self {
        if let Ok(mut a) = ASKER.write() {
            *a = Some(asker);
        }
        Self(())
    }
}

impl Drop for AskerGuard {
    fn drop(&mut self) {
        if let Ok(mut a) = ASKER.write() {
            *a = None;
        }
    }
}

#[derive(Clone, Copy)]
pub struct Prompt {
    /// 물어볼 수 있는가 (stdin·stderr가 터미널이고 JSON 모드가 아님)
    pub enabled: bool,
    /// 확인 질문에 자동으로 예
    pub yes: bool,
}

impl Prompt {
    pub fn new(json: bool, yes: bool) -> Self {
        let tty = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        Self {
            enabled: tty && !json,
            yes,
        }
    }

    fn theme() -> ColorfulTheme {
        ColorfulTheme::default()
    }

    fn map_err(e: dialoguer::Error) -> anyhow::Error {
        match e {
            dialoguer::Error::IO(io) if io.kind() == std::io::ErrorKind::Interrupted => CliError::Cancelled.into(),
            other => anyhow::anyhow!(other),
        }
    }

    /// 예/아니오. `--yes`면 묻지 않고 예. 물을 수 없으면 `--yes`를 붙이라는 오류.
    pub fn confirm(&self, question: &str, default: bool) -> Result<bool> {
        if self.yes {
            return Ok(true);
        }
        if !self.enabled {
            return Err(CliError::Usage(t!("prompt.needs_yes", question = question)).into());
        }
        if let Some(answer) = ask_with(|a| a.confirm(question, default)) {
            return answer.ok_or_else(|| CliError::Cancelled.into());
        }
        Confirm::with_theme(&Self::theme())
            .with_prompt(question)
            .default(default)
            .interact_on_opt(&Term::stderr())
            .map_err(Self::map_err)?
            .ok_or_else(|| CliError::Cancelled.into())
    }

    /// 사람이 직접 하는 동의 (약관 등). `--yes`로 대신하지 않고, 물을 수 없거나 취소·오류면 동의하지 않은 것
    pub fn consent(&self, question: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(answer) = ask_with(|a| a.confirm(question, false)) {
            return answer.unwrap_or(false);
        }
        Confirm::with_theme(&Self::theme())
            .with_prompt(question)
            .default(false)
            .interact_on_opt(&Term::stderr())
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    /// 사람이 직접 하는 동의(약관, 격리 없이 돌리기)를 받는다: `--yes`로 대신하지 않고, 물을 수 없으면 터미널에서 다시 하라고 한다
    pub fn agreed(&self, question: &str) -> Result<()> {
        if !self.enabled {
            return Err(CliError::Usage(t!("sandbox.needs_terminal")).into());
        }
        if self.consent(question) {
            Ok(())
        } else {
            Err(CliError::Cancelled.into())
        }
    }

    /// 확인을 받고, 아니면 취소로 끝낸다
    pub fn ensure(&self, question: &str) -> Result<()> {
        if self.confirm(question, false)? {
            Ok(())
        } else {
            Err(CliError::Cancelled.into())
        }
    }

    /// 목록에서 하나. 물을 수 없으면 None.
    pub fn select(&self, question: &str, items: &[String], default: usize) -> Result<Option<usize>> {
        if !self.enabled || items.is_empty() {
            return Ok(None);
        }
        let default = default.min(items.len() - 1);
        if let Some(picked) = ask_with(|a| a.select(question, items, default)) {
            return picked.map(Some).ok_or_else(|| CliError::Cancelled.into());
        }
        let picked = Select::with_theme(&Self::theme())
            .with_prompt(question)
            .items(items)
            .default(default)
            .interact_on_opt(&Term::stderr())
            .map_err(Self::map_err)?;
        picked.map(Some).ok_or_else(|| CliError::Cancelled.into())
    }

    /// 글자 입력. 물을 수 없으면 None.
    pub fn text(&self, question: &str, default: Option<&str>, allow_empty: bool) -> Result<Option<String>> {
        if !self.enabled {
            return Ok(None);
        }
        if let Some(answer) = ask_with(|a| a.text(question, default, allow_empty)) {
            return answer.map(Some).ok_or_else(|| CliError::Cancelled.into());
        }
        let theme = Self::theme();
        let mut input = Input::<String>::with_theme(&theme)
            .with_prompt(question)
            .allow_empty(allow_empty);
        if let Some(d) = default {
            input = input.default(d.to_string());
        }
        input.interact_text_on(&Term::stderr()).map(Some).map_err(Self::map_err)
    }

    /// 비밀값: 터미널이면 숨겨서 묻고, 아니면 표준 입력 한 줄 (예: `cat key.txt | saba-chan-cli chatbot key`).
    /// 셸 기록에 남지 않게 비밀값은 인자로 받지 않는다
    pub fn secret_or_stdin(&self, question: &str) -> Result<String> {
        use std::io::{BufRead, IsTerminal};
        match self.secret(question)? {
            Some(v) => Ok(v),
            None if !std::io::stdin().is_terminal() => {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                Ok(line)
            }
            None => Ok(String::new()),
        }
    }

    pub fn secret(&self, question: &str) -> Result<Option<String>> {
        if !self.enabled {
            return Ok(None);
        }
        if let Some(answer) = ask_with(|a| a.secret(question)) {
            return answer.map(Some).ok_or_else(|| CliError::Cancelled.into());
        }
        Password::with_theme(&Self::theme())
            .with_prompt(question)
            .allow_empty_password(true)
            .interact_on(&Term::stderr())
            .map(Some)
            .map_err(Self::map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted;

    impl Asker for Scripted {
        fn confirm(&self, question: &str, _default: bool) -> Option<bool> {
            match question {
                "cancel?" => None,
                "agree?" => Some(false),
                _ => Some(true),
            }
        }
        fn select(&self, _question: &str, items: &[String], _default: usize) -> Option<usize> {
            Some(items.len() - 1)
        }
        fn text(&self, _question: &str, default: Option<&str>, _allow_empty: bool) -> Option<String> {
            Some(format!("typed-{}", default.unwrap_or("")))
        }
        fn secret(&self, _question: &str) -> Option<String> {
            Some("s3cret".into())
        }
    }

    /// 사람이 직접 하는 동의는 `--yes`로 대신하지 않는다: 물을 수 없으면 터미널에서 하라는 오류
    #[test]
    fn yes_does_not_stand_in_for_a_consent() {
        let ask = Prompt { enabled: false, yes: true };
        let err = ask.agreed("q").unwrap_err();
        assert!(matches!(err.downcast_ref::<CliError>(), Some(CliError::Usage(_))), "{err}");
    }

    /// 끼운 창구가 묻는다. `--yes`는 동의를 대신하지 않고, 창구에서 취소하면 취소로 끝난다
    #[test]
    fn questions_go_to_the_installed_asker() {
        let _lock = ASKER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = AskerGuard::install(Box::new(Scripted));
        let ask = Prompt {
            enabled: true,
            yes: false,
        };
        assert!(ask.confirm("go?", false).unwrap());
        assert!(matches!(
            ask.confirm("cancel?", true).unwrap_err().downcast_ref::<CliError>(),
            Some(CliError::Cancelled)
        ));
        assert_eq!(ask.select("pick", &["a".into(), "b".into()], 0).unwrap(), Some(1));
        assert_eq!(ask.text("name", Some("x"), false).unwrap().as_deref(), Some("typed-x"));
        assert_eq!(ask.secret("key").unwrap().as_deref(), Some("s3cret"));
        assert!(!Prompt {
            enabled: true,
            yes: true
        }
        .consent("agree?"));
        // 물을 수 없을 때(파이프·--json)는 창구가 있어도 묻지 않는다
        assert_eq!(
            Prompt {
                enabled: false,
                yes: false
            }
            .text("name", None, true)
            .unwrap(),
            None
        );
    }
}
