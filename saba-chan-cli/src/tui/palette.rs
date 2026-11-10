//! 무엇이든 찾기 (Ctrl+K): 화면·설정 칸·CLI 명령을 한 곳에서 찾는다. 한글 초성도 맞는다
//!
//! 고른 줄마다 같은 뜻의 CLI 명령을 함께 보여 준다. Enter는 그 화면(설정 칸)으로 가고, 명령은 명령 줄을 채운다.
//! Tab은 같은 명령을 명령 줄에 넣는다.

use super::search;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Screen,
    Setting,
    Command,
}

impl EntryKind {
    pub fn label(self) -> String {
        t!(match self {
            EntryKind::Screen => "tui.find.screen",
            EntryKind::Setting => "tui.find.setting",
            EntryKind::Command => "tui.find.command",
        })
    }
}

/// 고르면 갈 곳
#[derive(Debug, Clone, PartialEq)]
pub enum Go {
    /// 쪽 번호, 상세 탭 번호, 목록에서 고를 줄
    Screen { page: usize, tab: Option<usize>, item: Option<usize> },
    /// 설정 탭으로 가서 그 칸에 커서 (쪽·탭·줄, 칸 키)
    Field { page: usize, tab: usize, item: Option<usize>, key: String },
    /// 명령 줄을 채운다
    Prefill(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub title: String,
    /// 어디에 있는지 (`디스코드 › 우리 서버 › 노래`)
    pub path: String,
    /// 같은 뜻의 CLI 명령 (없으면 빈 줄)
    pub cmd: String,
    pub go: Go,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteOut {
    Close,
    Go(Go),
    /// 명령 줄에 넣는다
    Prefill(String),
}

#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    pub cursor: usize,
    pub entries: Vec<Entry>,
}

/// 한 화면에 보이는 결과 수
pub const SHOWN: usize = 8;

impl Palette {
    pub fn new(entries: Vec<Entry>) -> Self {
        Palette { entries, ..Palette::default() }
    }

    /// 찾은 것: 제목에서 맞을수록, 그다음 위치·명령에서 맞는 순서
    pub fn results(&self) -> Vec<&Entry> {
        let mut found: Vec<(usize, usize, &Entry)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let score = search::score(&e.title, &self.query)
                    .map(|s| s + 2000)
                    .or_else(|| search::score(&e.path, &self.query).map(|s| s + 1000))
                    .or_else(|| search::score(&e.cmd, &self.query))?;
                Some((score, i, e))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found.into_iter().map(|(_, _, e)| e).collect()
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.results().get(self.cursor).copied()
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<PaletteOut> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.results().len();
        match key.code {
            KeyCode::Esc => return Some(PaletteOut::Close),
            KeyCode::Char('k') if ctrl => return Some(PaletteOut::Close),
            KeyCode::Up if n > 0 => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down if n > 0 => self.cursor = (self.cursor + 1) % n,
            KeyCode::Enter => {
                let e = self.selected()?;
                return Some(PaletteOut::Go(e.go.clone()));
            }
            KeyCode::Tab => {
                let e = self.selected()?;
                return (!e.cmd.is_empty()).then(|| PaletteOut::Prefill(format!("{} ", e.cmd)));
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.cursor = 0;
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: EntryKind, title: &str, path: &str, cmd: &str) -> Entry {
        Entry { kind, title: title.into(), path: path.into(), cmd: cmd.into(), go: Go::Prefill(cmd.into()) }
    }

    /// 초성으로 찾고, 제목이 맞는 것이 위치만 맞는 것보다 앞에 온다. Tab은 같은 명령을 명령 줄에
    #[test]
    fn initials_find_titles_first_and_tab_prefills_the_command() {
        let mut p = Palette::new(vec![
            e(EntryKind::Setting, "기본 음량", "디스코드 › 우리 서버 › 노래", "bot music"),
            e(EntryKind::Screen, "노래", "디스코드 › 우리 서버", "bot music"),
            e(EntryKind::Command, "server add", "새 서버 만들기", "server add"),
        ]);
        for c in "ㄴㄹ".chars() {
            p.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let titles: Vec<&str> = p.results().iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["노래", "기본 음량"]);
        assert_eq!(p.on_key(KeyEvent::from(KeyCode::Tab)), Some(PaletteOut::Prefill("bot music ".into())));
        p.query = "새서버".into();
        assert_eq!(p.on_key(KeyEvent::from(KeyCode::Enter)), Some(PaletteOut::Go(Go::Prefill("server add".into()))));
        assert_eq!(p.on_key(KeyEvent::from(KeyCode::Esc)), Some(PaletteOut::Close));
    }
}
