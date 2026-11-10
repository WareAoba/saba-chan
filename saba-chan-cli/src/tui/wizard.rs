//! 새 서버 만들기: 방법 → 게임 → 이름·폴더 → 추천 설정 → 확인. 끝나면 같은 뜻의 `server add` 한 줄을 실행한다
//!
//! 단계마다 Enter는 다음, Esc는 앞 단계(첫 단계에서는 닫기)다. 확인 단계에서 고칠 줄에 Enter를 누르면 그 단계로 돌아간다.

use super::data::s;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::collections::BTreeSet;

pub const STEPS: [&str; 5] = ["method", "game", "name", "preset", "confirm"];

#[derive(Debug, Clone, PartialEq)]
pub enum WizOut {
    Close,
    Run(String),
    /// 아래 줄에 잠깐 보일 안내
    Notice(String),
}

/// 확인 단계의 줄
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmRow {
    /// 그 단계로 돌아가는 요약 줄 (단계 번호, 이름 문구 키, 값)
    Summary(usize, &'static str, String),
    /// 모듈이 요구하는 동의 (id, 링크)
    Agreement(String, String),
    Start,
    Create,
}

#[derive(Debug, Clone, Default)]
pub struct Wizard {
    pub step: usize,
    /// 0 새로 설치, 1 있는 폴더 가져오기
    pub method: usize,
    /// 고른 게임 (모듈 목록 번호)
    pub module: usize,
    pub name: String,
    /// 가져올 폴더 (가져오기일 때)
    pub dir: String,
    /// 이름·폴더 단계에서 폴더 칸에 있는가
    pub on_dir: bool,
    /// 0은 추천 설정 없음, 1부터 모듈 프리셋
    pub preset: usize,
    pub accepted: BTreeSet<String>,
    pub start: bool,
    /// 목록 단계의 고른 줄
    pub cursor: usize,
}

use super::form::quote as q;

impl Wizard {
    pub fn new() -> Self {
        Wizard { start: true, ..Wizard::default() }
    }

    pub fn module<'a>(&self, modules: &'a [Value]) -> Option<&'a Value> {
        modules.get(self.module)
    }

    pub fn presets(&self, modules: &[Value]) -> Vec<Value> {
        self.module(modules).and_then(|m| m.get("presets")).and_then(Value::as_array).cloned().unwrap_or_default()
    }

    pub fn agreements(&self, modules: &[Value]) -> Vec<(String, String)> {
        self.module(modules)
            .and_then(|m| m.get("agreements"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| (s(a, "id").to_string(), s(a, "url").to_string()))
            .filter(|(id, _)| !id.is_empty())
            .collect()
    }

    pub fn game_name(m: &Value) -> String {
        [s(m, "game_name"), s(m, "display_name"), s(m, "name")].into_iter().find(|n| !n.is_empty()).unwrap_or("").to_string()
    }

    pub fn preset_label(&self, modules: &[Value]) -> String {
        match self.preset {
            0 => t!("tui.wiz.no_preset"),
            n => self.presets(modules).get(n - 1).map(|p| if s(p, "label").is_empty() { s(p, "id").to_string() } else { s(p, "label").to_string() }).unwrap_or_default(),
        }
    }

    /// 같은 뜻의 명령 한 줄
    pub fn command(&self, modules: &[Value]) -> String {
        let module = self.module(modules).map(|m| s(m, "name").to_string()).unwrap_or_default();
        let name = if self.name.trim().is_empty() { format!("<{}>", t!("tui.wiz.name")) } else { q(self.name.trim()) };
        let mut line = format!("server add {name} -m {}", q(&module));
        if self.method == 1 {
            line.push_str(&format!(" --from {}", q(self.dir.trim())));
        }
        if self.preset > 0 {
            if let Some(p) = self.presets(modules).get(self.preset - 1) {
                line.push_str(&format!(" --preset {}", q(s(p, "id"))));
            }
        }
        for (id, _) in self.agreements(modules) {
            if self.accepted.contains(&id) {
                line.push_str(&format!(" --accept {}", q(&id)));
            }
        }
        if self.start {
            line.push_str(" --start");
        }
        line
    }

    /// 목록 단계의 줄 수
    fn list_len(&self, modules: &[Value]) -> usize {
        match self.step {
            0 => 2,
            1 => modules.len(),
            3 => self.presets(modules).len() + 1,
            4 => self.confirm_rows(modules).len(),
            _ => 0,
        }
    }

    pub fn confirm_rows(&self, modules: &[Value]) -> Vec<ConfirmRow> {
        let method = if self.method == 0 { t!("tui.wiz.method_new") } else { t!("tui.wiz.method_import") };
        let mut rows = vec![
            ConfirmRow::Summary(0, "tui.wiz.step.method", method),
            ConfirmRow::Summary(1, "tui.wiz.step.game", self.module(modules).map(Self::game_name).unwrap_or_default()),
            ConfirmRow::Summary(2, "tui.wiz.name", self.name.trim().to_string()),
        ];
        if self.method == 1 {
            rows.push(ConfirmRow::Summary(2, "tui.wiz.folder", self.dir.trim().to_string()));
        }
        if !self.presets(modules).is_empty() {
            rows.push(ConfirmRow::Summary(3, "tui.wiz.step.preset", self.preset_label(modules)));
        }
        rows.extend(self.agreements(modules).into_iter().map(|(id, url)| ConfirmRow::Agreement(id, url)));
        rows.push(ConfirmRow::Start);
        rows.push(ConfirmRow::Create);
        rows
    }

    /// 단계를 옮긴다. 목록 단계에서는 지금 값에 커서를 둔다
    pub fn go(&mut self, step: usize, modules: &[Value]) {
        self.step = step;
        self.cursor = match step {
            0 => self.method,
            1 => self.module,
            3 => self.preset,
            4 => self.confirm_rows(modules).iter().position(|r| *r == ConfirmRow::Create).unwrap_or(0),
            _ => 0,
        };
        self.on_dir = false;
    }

    fn next(&mut self, modules: &[Value]) -> Option<WizOut> {
        match self.step {
            0 => self.method = self.cursor,
            1 => {
                if modules.is_empty() {
                    return Some(WizOut::Notice(t!("tui.wiz.no_modules")));
                }
                if self.module != self.cursor {
                    self.preset = 0;
                    self.accepted.clear();
                }
                self.module = self.cursor;
            }
            2 => {
                if self.name.trim().is_empty() {
                    return Some(WizOut::Notice(t!("tui.wiz.need_name")));
                }
                if self.method == 1 && self.dir.trim().is_empty() {
                    self.on_dir = true;
                    return Some(WizOut::Notice(t!("tui.wiz.need_folder")));
                }
            }
            3 => self.preset = self.cursor,
            _ => {}
        }
        // 추천 설정이 없는 게임은 그 단계를 건너뛴다
        let mut step = self.step + 1;
        if step == 3 && self.presets(modules).is_empty() {
            step = 4;
        }
        self.go(step, modules);
        None
    }

    fn back(&mut self, modules: &[Value]) -> Option<WizOut> {
        if self.step == 0 {
            return Some(WizOut::Close);
        }
        let mut step = self.step - 1;
        if step == 3 && self.presets(modules).is_empty() {
            step = 2;
        }
        self.go(step, modules);
        None
    }

    pub fn on_key(&mut self, key: KeyEvent, modules: &[Value]) -> Option<WizOut> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc {
            return self.back(modules);
        }
        if self.step == 2 {
            let field = if self.on_dir { &mut self.dir } else { &mut self.name };
            match key.code {
                KeyCode::Enter => return self.next(modules),
                KeyCode::Tab | KeyCode::Down | KeyCode::Up if self.method == 1 => self.on_dir = !self.on_dir,
                KeyCode::Backspace => {
                    field.pop();
                }
                KeyCode::Char(c) if !ctrl => field.push(c),
                _ => {}
            }
            return None;
        }
        let n = self.list_len(modules);
        match key.code {
            KeyCode::Up if n > 0 => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down if n > 0 => self.cursor = (self.cursor + 1) % n,
            KeyCode::Char(' ') if self.step == 4 => self.toggle(modules),
            KeyCode::Enter if self.step == 4 => {
                let rows = self.confirm_rows(modules);
                match rows.get(self.cursor) {
                    Some(ConfirmRow::Summary(step, _, _)) => self.go(*step, modules),
                    Some(ConfirmRow::Create) => {
                        let missing = self.agreements(modules).into_iter().any(|(id, _)| !self.accepted.contains(&id));
                        if missing {
                            return Some(WizOut::Notice(t!("tui.wiz.need_agree")));
                        }
                        return Some(WizOut::Run(self.command(modules)));
                    }
                    _ => self.toggle(modules),
                }
            }
            KeyCode::Enter => return self.next(modules),
            _ => {}
        }
        None
    }

    fn toggle(&mut self, modules: &[Value]) {
        match self.confirm_rows(modules).get(self.cursor) {
            Some(ConfirmRow::Agreement(id, _)) => {
                if !self.accepted.remove(id) {
                    self.accepted.insert(id.clone());
                }
            }
            Some(ConfirmRow::Start) => self.start = !self.start,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn modules() -> Vec<Value> {
        vec![
            json!({ "name": "palworld", "game_name": "Palworld" }),
            json!({ "name": "minecraft", "game_name": "Minecraft", "presets": [{ "id": "friends", "label": "친구용" }], "agreements": [{ "id": "eula", "url": "https://aka.ms/MinecraftEULA" }] }),
        ]
    }

    fn typed(w: &mut Wizard, text: &str, m: &[Value]) {
        for c in text.chars() {
            w.on_key(key(KeyCode::Char(c)), m);
        }
    }

    /// 다섯 단계를 키로 지나 `server add` 한 줄이 된다. 동의가 필요한 게임은 동의해야 만든다
    #[test]
    fn the_steps_end_in_one_server_add_line() {
        crate::i18n::init("en");
        let m = modules();
        let mut w = Wizard::new();
        w.on_key(key(KeyCode::Enter), &m);
        w.on_key(key(KeyCode::Down), &m);
        w.on_key(key(KeyCode::Enter), &m);
        assert_eq!(w.step, 2);
        assert!(matches!(w.on_key(key(KeyCode::Enter), &m), Some(WizOut::Notice(_))), "a name is needed");
        typed(&mut w, "야생 서버", &m);
        w.on_key(key(KeyCode::Enter), &m);
        assert_eq!(w.step, 3);
        w.on_key(key(KeyCode::Down), &m);
        w.on_key(key(KeyCode::Enter), &m);
        assert_eq!(w.step, 4);
        assert!(matches!(w.on_key(key(KeyCode::Enter), &m), Some(WizOut::Notice(_))), "EULA first");
        let eula = w.confirm_rows(&m).iter().position(|r| matches!(r, ConfirmRow::Agreement(..))).unwrap();
        w.cursor = eula;
        w.on_key(key(KeyCode::Char(' ')), &m);
        w.go(4, &m);
        assert_eq!(w.on_key(key(KeyCode::Enter), &m), Some(WizOut::Run("server add '야생 서버' -m minecraft --preset friends --accept eula --start".into())));
        // 요약 줄에서 Enter는 그 단계로, Esc는 한 단계씩 앞으로, 첫 단계에서 닫는다
        w.cursor = 1;
        w.on_key(key(KeyCode::Enter), &m);
        assert_eq!(w.step, 1);
        w.on_key(key(KeyCode::Esc), &m);
        assert_eq!(w.on_key(key(KeyCode::Esc), &m), Some(WizOut::Close));
    }

    /// 가져오기는 폴더를 받고, 추천 설정이 없는 게임은 그 단계를 건너뛴다
    #[test]
    fn importing_asks_for_a_folder_and_games_without_presets_skip_that_step() {
        let m = modules();
        let mut w = Wizard::new();
        w.on_key(key(KeyCode::Down), &m);
        w.on_key(key(KeyCode::Enter), &m);
        w.on_key(key(KeyCode::Enter), &m);
        typed(&mut w, "pal", &m);
        w.on_key(key(KeyCode::Enter), &m);
        assert!(w.on_dir, "the folder is asked for next");
        typed(&mut w, "/srv/pal", &m);
        w.on_key(key(KeyCode::Enter), &m);
        assert_eq!(w.step, 4);
        w.start = false;
        assert_eq!(w.command(&m), "server add pal -m palworld --from /srv/pal");
        w.on_key(key(KeyCode::Esc), &m);
        assert_eq!(w.step, 2);
    }
}
