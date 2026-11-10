//! 설정 폼: 칸 목록을 보여 주고 키로 값을 바꾼다. 저장은 같은 뜻의 CLI 명령 한 줄로 한다
//!
//! 칸 종류는 켜기/끄기, 고르기(←→), 숫자, 글자(Enter로 입력)다. 목록·여러 줄 칸은 읽기만 한다.
//! 바뀐 칸은 원래 값 → 새 값으로 보이고, Ctrl+S로 바뀐 칸만 모아 저장 명령을 만든다. Esc는 한 단계씩 되돌린다:
//! 입력 중이면 입력 취소, 찾기 중이면 찾기 지우기, 바뀐 칸이 있으면 되돌리기, 없으면 폼을 떠난다.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Bool,
    /// (값, 보이는 이름)
    Choice(Vec<(String, String)>),
    Number { min: Option<f64>, max: Option<f64> },
    Text { secret: bool },
    /// 목록·여러 줄: 여기서는 읽기만
    ReadOnly,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub group: String,
    pub kind: Kind,
    /// 처음 값 (빈 칸은 정하지 않음 = 기본값)
    pub original: String,
    pub value: String,
    pub default: String,
    pub description: String,
    pub restart: bool,
}

impl Field {
    pub fn dirty(&self) -> bool {
        self.value != self.original
    }

    /// 정하지 않았으면 기본값으로 본다
    fn effective(&self) -> &str {
        if self.value.is_empty() {
            &self.default
        } else {
            &self.value
        }
    }

    /// 화면에 보일 값
    pub fn shown(&self, raw: &str) -> String {
        let v = if raw.is_empty() { self.default.as_str() } else { raw };
        match &self.kind {
            Kind::Bool => crate::output::yes_no(v == "true"),
            Kind::Choice(items) => items.iter().find(|(k, _)| k == v).map(|(_, l)| l.clone()).unwrap_or_else(|| v.to_string()),
            Kind::Text { secret: true } if !v.is_empty() => "•".repeat(8),
            _ => v.to_string(),
        }
    }
}

/// 폼의 한 줄: 묶음 머리줄이거나 칸
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Group(String),
    Field(usize),
}

/// 키를 누른 결과
#[derive(Debug, Clone, PartialEq)]
pub enum FormOut {
    /// 바뀐 칸 (키, 새 값)
    Save(Vec<(String, String)>),
    /// 폼을 떠난다 (한 단계 위로)
    Leave,
}

/// 묶음 순서. 고급은 처음에 접혀 있다
const GROUPS: [&str; 3] = ["basic", "saba-chan", "advanced"];

#[derive(Debug, Clone, Default)]
pub struct Form {
    pub fields: Vec<Field>,
    pub cursor: usize,
    /// Enter로 입력 중인 글자
    pub editing: Option<String>,
    /// `/` 찾기 (입력 중이면 typing)
    pub filter: String,
    pub filtering: bool,
    pub show_keys: bool,
    /// 펼친 묶음 (고급만 접혀 시작한다)
    pub open: BTreeSet<String>,
    pub scroll: usize,
}

fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

impl Form {
    /// `server settings --json`과 같은 문서에서 (`SettingsData::doc`)
    pub fn from_doc(doc: &Value) -> Self {
        let fields = doc
            .get("fields")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|f| f.get("hidden").and_then(Value::as_bool) != Some(true))
            .map(|f| {
                let get = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let options: Vec<String> = f.get("options").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
                let kind = match get("type").as_str() {
                    "boolean" => Kind::Bool,
                    "number" => Kind::Number { min: f.get("min").and_then(Value::as_f64), max: f.get("max").and_then(Value::as_f64) },
                    "select" if !options.is_empty() => Kind::Choice(
                        options.iter().map(|o| (o.clone(), f.pointer(&format!("/option_labels/{o}")).and_then(Value::as_str).unwrap_or(o).to_string())).collect(),
                    ),
                    "password" => Kind::Text { secret: true },
                    "list" | "multiline" => Kind::ReadOnly,
                    _ => Kind::Text { secret: false },
                };
                let value = text_of(f.get("value").unwrap_or(&Value::Null));
                let group = match f.get("group").and_then(Value::as_str) {
                    None | Some("") => "basic".to_string(),
                    Some(g) => g.to_string(),
                };
                Field {
                    key: get("key"),
                    label: get("label"),
                    group,
                    kind,
                    original: value.clone(),
                    value,
                    default: text_of(f.get("default").unwrap_or(&Value::Null)),
                    description: get("description"),
                    restart: f.get("restart_required").and_then(Value::as_bool) == Some(true),
                }
            })
            .collect();
        Self::new(fields)
    }

    pub fn new(fields: Vec<Field>) -> Self {
        let mut form = Form { fields, ..Form::default() };
        form.open = form.groups().into_iter().filter(|g| g != "advanced").collect();
        form.cursor = form.rows().iter().position(|r| matches!(r, Row::Field(_))).unwrap_or(0);
        form
    }

    /// 칸이 있는 묶음들 (정한 순서 뒤에 나머지)
    pub fn groups(&self) -> Vec<String> {
        let mut out: Vec<String> = GROUPS.iter().filter(|g| self.fields.iter().any(|f| f.group == **g)).map(|g| g.to_string()).collect();
        for f in &self.fields {
            if !out.contains(&f.group) {
                out.push(f.group.clone());
            }
        }
        out
    }

    fn matches(&self, f: &Field) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let q = self.filter.to_lowercase();
        f.label.to_lowercase().contains(&q) || f.key.to_lowercase().contains(&q) || super::search::matches(&f.label, &self.filter)
    }

    /// 보이는 줄. 찾는 중에는 접힌 묶음도 맞는 칸을 보인다
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in self.groups() {
            let fields: Vec<usize> = (0..self.fields.len()).filter(|i| self.fields[*i].group == g && self.matches(&self.fields[*i])).collect();
            if fields.is_empty() {
                continue;
            }
            rows.push(Row::Group(g.clone()));
            if self.open.contains(&g) || !self.filter.is_empty() {
                rows.extend(fields.into_iter().map(Row::Field));
            }
        }
        rows
    }

    /// 묶음의 칸 수 (머리줄에 보인다)
    pub fn group_len(&self, g: &str) -> usize {
        self.fields.iter().filter(|f| f.group == g).count()
    }

    pub fn changes(&self) -> Vec<(String, String)> {
        self.fields.iter().filter(|f| f.dirty()).map(|f| (f.key.clone(), f.value.clone())).collect()
    }

    pub fn current(&self) -> Option<&Field> {
        match self.rows().get(self.cursor)? {
            Row::Field(i) => self.fields.get(*i),
            Row::Group(_) => None,
        }
    }

    fn current_index(&self) -> Option<usize> {
        match self.rows().get(self.cursor)? {
            Row::Field(i) => Some(*i),
            Row::Group(_) => None,
        }
    }

    pub fn revert(&mut self) {
        for f in &mut self.fields {
            f.value = f.original.clone();
        }
    }

    /// 저장이 끝나 새로 읽은 값으로 바꾼다. 커서·펼침·찾기는 남긴다
    pub fn reload(&mut self, fresh: Form) {
        let (cursor, open, filter, show_keys) = (self.cursor, std::mem::take(&mut self.open), std::mem::take(&mut self.filter), self.show_keys);
        // 아직 저장하지 않은 칸은 새로 읽은 값 위에 남긴다 (다른 명령이 끝나 다시 읽어도, 저장이 실패해도 고친 것을 잃지 않게)
        let edits: Vec<(String, String)> = self.fields.iter().filter(|f| f.dirty()).map(|f| (f.key.clone(), f.value.clone())).collect();
        let editing = self.editing.take();
        *self = fresh;
        for (key, value) in edits {
            if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
                f.value = value;
            }
        }
        self.editing = editing;
        self.open = open;
        self.filter = filter;
        self.show_keys = show_keys;
        self.cursor = cursor.min(self.rows().len().saturating_sub(1));
    }

    /// ←→: 고르기는 다음 값, 켜기/끄기는 뒤집기, 숫자는 1씩
    fn step(&mut self, forward: bool) {
        let Some(i) = self.current_index() else { return };
        let f = &mut self.fields[i];
        let cur = f.effective().to_string();
        match &f.kind {
            Kind::Bool => f.value = if cur == "true" { "false".into() } else { "true".into() },
            Kind::Choice(items) => {
                let n = items.len();
                let at = items.iter().position(|(k, _)| *k == cur);
                let next = match (at, forward) {
                    (Some(a), true) => (a + 1) % n,
                    (Some(a), false) => (a + n - 1) % n,
                    (None, _) => 0,
                };
                f.value = items[next].0.clone();
            }
            Kind::Number { min, max } => {
                let n: f64 = cur.parse().unwrap_or(0.0);
                let mut next = if forward { n + 1.0 } else { n - 1.0 };
                if let Some(m) = min {
                    next = next.max(*m);
                }
                if let Some(m) = max {
                    next = next.min(*m);
                }
                f.value = if next.fract() == 0.0 { format!("{}", next as i64) } else { next.to_string() };
            }
            _ => {}
        }
    }

    fn move_cursor(&mut self, by: isize) {
        let n = self.rows().len();
        if n == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + by).clamp(0, n as isize - 1) as usize;
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<FormOut> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(buf) = self.editing.as_mut() {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter => {
                    let text = self.editing.take().unwrap_or_default();
                    if let Some(i) = self.current_index() {
                        self.fields[i].value = text.trim().to_string();
                    }
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) if !ctrl => buf.push(c),
                _ => {}
            }
            return None;
        }
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                }
                KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                    self.filtering = false;
                    self.cursor = self.rows().iter().position(|r| matches!(r, Row::Field(_))).unwrap_or(0);
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) if !ctrl => self.filter.push(c),
                _ => {}
            }
            self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
            return None;
        }
        match key.code {
            KeyCode::Char('s') if ctrl => {
                let changes = self.changes();
                return (!changes.is_empty()).then_some(FormOut::Save(changes));
            }
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::Left => self.step(false),
            KeyCode::Right => self.step(true),
            KeyCode::Char(' ') => {
                if matches!(self.current().map(|f| &f.kind), Some(Kind::Bool)) {
                    self.step(true);
                }
            }
            KeyCode::Enter => match self.rows().get(self.cursor).cloned() {
                Some(Row::Group(g)) => {
                    if !self.open.remove(&g) {
                        self.open.insert(g);
                    }
                }
                Some(Row::Field(i)) => match self.fields[i].kind {
                    Kind::Text { .. } | Kind::Number { .. } => self.editing = Some(self.fields[i].value.clone()),
                    Kind::Bool | Kind::Choice(_) => self.step(true),
                    Kind::ReadOnly => {}
                },
                None => {}
            },
            KeyCode::Backspace | KeyCode::Delete => {
                // 정한 값을 지워 기본값으로
                if let Some(i) = self.current_index().filter(|i| self.fields[*i].kind != Kind::ReadOnly) {
                    self.fields[i].value.clear();
                }
            }
            KeyCode::Char('/') => {
                self.filtering = true;
            }
            KeyCode::Char('k') => self.show_keys = !self.show_keys,
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                } else if self.fields.iter().any(Field::dirty) {
                    self.revert();
                } else {
                    return Some(FormOut::Leave);
                }
            }
            _ => {}
        }
        None
    }
}

/// 명령 줄에 넣을 낱말: 글자·숫자와 `-_./:=,@+`만이면 그대로, 아니면 따옴표로 (한글 이름도 그대로 보이게)
pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "-_./:=,@+".contains(c)) && !s.starts_with('-') {
        return s.to_string();
    }
    shlex::try_quote(s).map(|c| c.into_owned()).unwrap_or_else(|_| s.to_string())
}

/// 바뀐 칸을 저장하는 명령 한 줄: `server set 이름 키=값 …` (값이 비면 그 설정을 지운다)
pub fn save_line(server: &str, changes: &[(String, String)]) -> String {
    let q = quote;
    let mut line = format!("server set {}", q(server));
    for (k, v) in changes {
        line.push(' ');
        line.push_str(&q(&format!("{k}={v}")));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventState;
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: ratatui::crossterm::event::KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent { modifiers: KeyModifiers::CONTROL, ..key(KeyCode::Char(c)) }
    }

    fn doc() -> Value {
        json!({ "server": "마크 야생", "fields": [
            { "key": "difficulty", "label": "난이도", "type": "select", "group": null, "value": "normal", "default": "easy",
              "options": ["peaceful", "easy", "normal", "hard"], "option_labels": { "hard": "어려움", "normal": "보통" } },
            { "key": "pvp", "label": "PVP", "type": "boolean", "group": null, "value": null, "default": true },
            { "key": "max_players", "label": "최대 인원", "type": "number", "group": null, "value": 20, "min": 1, "max": 20 },
            { "key": "motd", "label": "서버 소개", "type": "text", "group": null, "value": "hi" },
            { "key": "rcon_password", "label": "RCON 비밀번호", "type": "password", "group": "saba-chan", "value": "secret" },
            { "key": "hidden_one", "label": "숨김", "type": "text", "group": null, "value": "x", "hidden": true },
            { "key": "jvm_args", "label": "JVM 인자", "type": "list", "group": "advanced", "value": ["-Xmx4G"] },
        ]})
    }

    /// 칸 종류마다 키로 값을 바꾸고, Ctrl+S는 바뀐 칸만 `server set` 한 줄로 만든다
    #[test]
    fn keys_change_values_and_save_only_what_changed() {
        crate::i18n::init("en");
        let mut f = Form::from_doc(&doc());
        assert_eq!(f.current().map(|x| x.key.as_str()), Some("difficulty"));
        assert_eq!(f.on_key(ctrl('s')), None, "nothing to save yet");
        f.on_key(key(KeyCode::Right));
        assert_eq!(f.current().unwrap().shown(&f.current().unwrap().value), "어려움");
        f.on_key(key(KeyCode::Down));
        // 정하지 않은 켜기/끄기는 기본값(켜짐)에서 뒤집는다
        f.on_key(key(KeyCode::Char(' ')));
        assert_eq!(f.current().unwrap().value, "false");
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Right));
        assert_eq!(f.current().unwrap().value, "20", "numbers stay in range");
        f.on_key(key(KeyCode::Left));
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Enter));
        for c in " 안녕 친구들".chars() {
            f.on_key(key(KeyCode::Char(c)));
        }
        f.on_key(key(KeyCode::Enter));
        let Some(FormOut::Save(changes)) = f.on_key(ctrl('s')) else { panic!("save") };
        assert_eq!(
            save_line("마크 야생", &changes),
            "server set '마크 야생' difficulty=hard pvp=false max_players=19 'motd=hi 안녕 친구들'"
        );
    }

    /// 숨긴 칸은 없고, 고급은 접혀 있다가 Enter로 펼친다. 목록 칸은 읽기만 한다
    #[test]
    fn hidden_fields_are_left_out_and_advanced_starts_folded() {
        let mut f = Form::from_doc(&doc());
        assert!(!f.fields.iter().any(|x| x.key == "hidden_one"));
        let rows = f.rows();
        assert_eq!(rows.last(), Some(&Row::Group("advanced".into())));
        f.cursor = rows.len() - 1;
        f.on_key(key(KeyCode::Enter));
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.current().unwrap().kind, Kind::ReadOnly);
        f.on_key(key(KeyCode::Enter));
        f.on_key(key(KeyCode::Right));
        assert!(f.changes().is_empty());
    }

    /// Esc는 한 단계씩: 입력 취소 → 찾기 지우기 → 바뀐 칸 되돌리기 → 떠나기
    #[test]
    fn esc_goes_back_one_step_at_a_time() {
        let mut f = Form::from_doc(&doc());
        f.on_key(key(KeyCode::Right));
        f.on_key(key(KeyCode::Char('/')));
        for c in "motd".chars() {
            f.on_key(key(KeyCode::Char(c)));
        }
        f.on_key(key(KeyCode::Enter));
        assert_eq!(f.current().map(|x| x.key.as_str()), Some("motd"));
        f.on_key(key(KeyCode::Enter));
        assert!(f.editing.is_some());
        f.on_key(key(KeyCode::Esc));
        assert!(f.editing.is_none());
        assert_eq!(f.filter, "motd", "the first Esc only closes typing");
        f.on_key(key(KeyCode::Esc));
        assert!(f.filter.is_empty());
        assert_eq!(f.changes().len(), 1);
        f.on_key(key(KeyCode::Esc));
        assert!(f.changes().is_empty());
        assert_eq!(f.on_key(key(KeyCode::Esc)), Some(FormOut::Leave));
    }

    /// 다시 읽어도 저장하지 않은 칸은 남는다
    #[test]
    fn a_reload_keeps_unsaved_edits() {
        let mut f = Form::from_doc(&doc());
        f.on_key(key(KeyCode::Right));
        let mut fresh = Form::from_doc(&doc());
        fresh.fields[1].value = "false".into();
        fresh.fields[1].original = "false".into();
        f.reload(fresh);
        assert_eq!(f.changes(), vec![("difficulty".to_string(), "hard".to_string())]);
        assert_eq!(f.fields[1].value, "false", "the rest comes from the daemon");
    }

    /// 한글 초성으로도 칸을 찾는다
    #[test]
    fn the_filter_understands_korean_initials() {
        let mut f = Form::from_doc(&doc());
        f.filter = "ㄴㅇㄷ".into();
        assert_eq!(f.rows(), vec![Row::Group("basic".into()), Row::Field(0)]);
    }
}
