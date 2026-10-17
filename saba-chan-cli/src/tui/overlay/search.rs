//! Ctrl+F 검색 오버레이 — 활성 탭의 로그 라인 중 패턴에 맞는 항목만 강조·필터.
//!
//! 동작:
//! - 입력 후 Enter → 활성 탭의 `filter` 필드에 소문자 소문자 substring으로 적용
//! - Esc → 필터 해제 및 오버레이 닫기
//! - Backspace/문자 입력으로 실시간 편집

#[derive(Clone, Debug, Default)]
pub struct SearchOverlay {
    pub query: String,
    pub cursor: usize,
}

impl SearchOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_pos = char_to_byte(&self.query, self.cursor);
        self.query.insert(byte_pos, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 { return; }
        let end = char_to_byte(&self.query, self.cursor);
        let start = char_to_byte(&self.query, self.cursor - 1);
        self.query.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        let char_len = self.query.chars().count();
        if self.cursor >= char_len { return; }
        let start = char_to_byte(&self.query, self.cursor);
        let end = char_to_byte(&self.query, self.cursor + 1);
        self.query.replace_range(start..end, "");
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 { self.cursor -= 1; }
    }

    pub fn move_right(&mut self) {
        let max = self.query.chars().count();
        if self.cursor < max { self.cursor += 1; }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_ascii_char_advances_cursor() {
        let mut s = SearchOverlay::new();
        s.insert_char('a');
        s.insert_char('b');
        assert_eq!(s.query, "ab");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_removes_char_before_cursor() {
        let mut s = SearchOverlay::new();
        s.insert_char('a');
        s.insert_char('b');
        s.backspace();
        assert_eq!(s.query, "a");
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn handles_multibyte_chars() {
        let mut s = SearchOverlay::new();
        s.insert_char('한');
        s.insert_char('글');
        assert_eq!(s.query, "한글");
        assert_eq!(s.cursor, 2);
        s.backspace();
        assert_eq!(s.query, "한");
        assert_eq!(s.cursor, 1);
    }
}
