//! 대화형 입력 — 터미널에서만 묻는다
//!
//! 터미널이 아니거나 `--json`이면 묻지 않는다. 그때 꼭 필요한 값이 없으면 호출한 쪽이
//! 사용법 오류를 낸다. `--yes`는 확인 질문만 건너뛴다(값을 지어내지는 않는다).

use crate::error::{CliError, Result};
use console::Term;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};
use std::io::IsTerminal;

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
        Confirm::with_theme(&Self::theme())
            .with_prompt(question)
            .default(false)
            .interact_on_opt(&Term::stderr())
            .ok()
            .flatten()
            .unwrap_or(false)
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
        let picked = Select::with_theme(&Self::theme())
            .with_prompt(question)
            .items(items)
            .default(default.min(items.len() - 1))
            .interact_on_opt(&Term::stderr())
            .map_err(Self::map_err)?;
        picked.map(Some).ok_or_else(|| CliError::Cancelled.into())
    }

    /// 글자 입력. 물을 수 없으면 None.
    pub fn text(&self, question: &str, default: Option<&str>, allow_empty: bool) -> Result<Option<String>> {
        if !self.enabled {
            return Ok(None);
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
        Password::with_theme(&Self::theme())
            .with_prompt(question)
            .allow_empty_password(true)
            .interact_on(&Term::stderr())
            .map(Some)
            .map_err(Self::map_err)
    }
}
