//! TUI 세션 영속화 — 테마, 명령 히스토리, 최근 탭 등.
//!
//! 저장 경로: `%APPDATA%/saba-chan/tui-session.json` (Windows)
//! 프로세스 종료 시 `Session::save()`, 시작 시 `Session::load()`.

use saba_chan_updater_lib::constants;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
const HISTORY_MAX: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    /// 선호 테마 이름 ("dark" | "light")
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 레거시 `:명령` 입력 히스토리 (최신이 뒤)
    #[serde(default)]
    pub history: Vec<String>,
    /// 슬래시 명령 입력 히스토리
    #[serde(default)]
    pub slash_history: Vec<String>,
    /// 마지막에 열려있던 탭 source (재시작 시 복원 후보)
    #[serde(default)]
    pub recent_tabs: Vec<RecentTab>,
}

fn default_theme() -> String { "dark".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecentTab {
    Server { id: String, name: String, module: String },
    Daemon,
    ExtProcess { name: String },
}

impl Session {
    fn path() -> anyhow::Result<PathBuf> {
        Ok(constants::resolve_data_dir().join("tui-session.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn push_history(&mut self, cmd: String) {
        if cmd.trim().is_empty() { return; }
        if self.history.last() == Some(&cmd) { return; }
        self.history.push(cmd);
        if self.history.len() > HISTORY_MAX {
            let drop = self.history.len() - HISTORY_MAX;
            self.history.drain(..drop);
        }
    }

    #[allow(dead_code)]
    pub fn push_slash_history(&mut self, cmd: String) {
        if cmd.trim().is_empty() { return; }
        if self.slash_history.last() == Some(&cmd) { return; }
        self.slash_history.push(cmd);
        if self.slash_history.len() > HISTORY_MAX {
            let drop = self.slash_history.len() - HISTORY_MAX;
            self.slash_history.drain(..drop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_history_dedups_consecutive() {
        let mut s = Session::default();
        s.push_history("a".into());
        s.push_history("a".into());
        s.push_history("b".into());
        assert_eq!(s.history, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn push_history_ignores_empty() {
        let mut s = Session::default();
        s.push_history("".into());
        s.push_history("  ".into());
        assert!(s.history.is_empty());
    }

    #[test]
    fn push_history_caps_length() {
        let mut s = Session::default();
        for i in 0..(HISTORY_MAX + 50) {
            s.push_history(format!("c{}", i));
        }
        assert_eq!(s.history.len(), HISTORY_MAX);
        assert_eq!(s.history.first().unwrap(), &format!("c{}", 50));
    }

    #[test]
    fn json_round_trip() {
        let s = Session {
            theme: "light".into(),
            history: vec!["a".into(), "b".into()],
            slash_history: vec!["help".into()],
            recent_tabs: vec![RecentTab::Daemon, RecentTab::Server {
                id: "x".into(), name: "srv".into(), module: "mc".into(),
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.theme, "light");
        assert_eq!(back.history.len(), 2);
        assert_eq!(back.recent_tabs.len(), 2);
    }
}
