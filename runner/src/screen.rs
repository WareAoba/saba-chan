//! 의사 콘솔 한 줄을 터미널이 그린 대로 글자로 옮긴다.
//!
//! 의사 콘솔(Windows ConPTY)은 서버가 쓴 글자를 화면으로 다시 그려 보낸다. 그래서 줄 안에
//! - `\r`로 같은 줄을 덮어 쓴 진행 표시("Resetting game objects 10%" → 11% → …)와
//! - 빈칸 대신 커서 이동(`ESC[nC`)이나 글자 지우기(`ESC[nX`)로 맞춘 칸
//!
//! 이 섞여 온다. 제어 문자를 그냥 지우면 덮어 쓴 내용이 이어 붙고 칸이 사라진다("helpDisplays").
//! 한 줄짜리 화면에 그려 보고 마지막 모습만 남긴다. 다른 줄로 가는 이동은 줄 안 위치만 쓴다.

use unicode_width::UnicodeWidthChar;

/// 넓은 글자(한자·한글)의 오른쪽 칸
const WIDE_TAIL: char = '\0';
/// 커서 이동이 줄을 끝없이 늘리지 않게 (의사 콘솔 폭)
const MAX_COLUMNS: usize = 4096;

#[derive(Default)]
struct Line {
    cells: Vec<char>,
    col: usize,
}

impl Line {
    fn put(&mut self, c: char) {
        let width = c.width().unwrap_or(0);
        if width == 0 {
            return;
        }
        if self.col + width > MAX_COLUMNS {
            return;
        }
        if self.cells.len() < self.col + width {
            self.cells.resize(self.col + width, ' ');
        }
        self.cells[self.col] = c;
        if width == 2 {
            self.cells[self.col + 1] = WIDE_TAIL;
        }
        self.col += width;
    }

    fn move_to(&mut self, col: usize) {
        self.col = col.min(MAX_COLUMNS);
    }

    /// `ESC[` 뒤 매개변수와 끝 글자
    fn csi(&mut self, params: &str, end: char) {
        let nums: Vec<usize> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let n = nums.first().copied().filter(|n| *n > 0).unwrap_or(1);
        match end {
            'C' => self.move_to(self.col.saturating_add(n)),
            'D' => self.move_to(self.col.saturating_sub(n)),
            'G' | '`' => self.move_to(n - 1),
            // 줄·칸 위치: 줄은 버리고 칸만 쓴다
            'H' | 'f' => self.move_to(nums.get(1).copied().filter(|c| *c > 0).unwrap_or(1) - 1),
            'K' => match nums.first().copied().unwrap_or(0) {
                0 => self.cells.truncate(self.col),
                1 => self.cells.iter_mut().take(self.col + 1).for_each(|c| *c = ' '),
                _ => self.cells.clear(),
            },
            'X' => self.cells.iter_mut().skip(self.col).take(n).for_each(|c| *c = ' '),
            _ => {}
        }
    }

    fn text(self) -> String {
        let text: String = self.cells.into_iter().filter(|c| *c != WIDE_TAIL).collect();
        text.trim_end().to_string()
    }
}

/// 의사 콘솔이 보낸 한 줄(줄 끝은 뗀 글자)을 화면에 보인 모습으로
pub fn render(raw: &str) -> String {
    let mut line = Line::default();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    while let Some(&p) = chars.peek() {
                        if ('\x40'..='\x7e').contains(&p) {
                            break;
                        }
                        params.push(p);
                        chars.next();
                    }
                    if let Some(end) = chars.next() {
                        // `ESC[?25l` 같은 사설 표기는 화면 모양과 무관하다
                        if !params.starts_with(['?', '>', '=', '<']) {
                            line.csi(&params, end);
                        }
                    }
                }
                // 창 제목 등 (`ESC]…BEL` 또는 `ESC]…ESC\`)
                Some(']') => {
                    while let Some(p) = chars.next() {
                        if p == '\x07' || (p == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                // 글자 집합 고르기 `ESC(B` 등은 한 글자를 더 먹는다
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' => line.move_to(0),
            '\x08' => line.move_to(line.col.saturating_sub(1)),
            '\t' => line.move_to((line.col / 8 + 1) * 8),
            c if c.is_control() => {}
            c => line.put(c),
        }
    }
    line.text()
}

#[cfg(test)]
mod tests {
    use super::render;

    /// 테라리아 진행 표시: `\r`로 같은 줄을 덮어 쓴 것은 마지막 모습 한 줄이다
    #[test]
    fn a_line_redrawn_with_carriage_returns_keeps_its_last_state() {
        assert_eq!(render("Resetting game objects 10%\rResetting game objects 11%\rResetting game objects 12%"), "Resetting game objects 12%");
        // 의사 콘솔은 바뀐 칸만 다시 그리기도 한다
        assert_eq!(render("Loading world data: 9%\x1b[1;21H10%"), "Loading world data: 10%");
        assert_eq!(render("Settling liquids 100%\r\x1b[KServer started"), "Server started");
        assert_eq!(render("abcdef\x1b[3G\x1b[0K"), "ab");
    }

    /// 테라리아 help: 빈칸 대신 커서 이동·글자 지우기로 맞춘 칸을 빈칸으로 되살린다
    #[test]
    fn columns_aligned_by_cursor_moves_keep_their_spaces() {
        assert_eq!(render("help\x1b[16CDisplays a list of commands."), "help                Displays a list of commands.");
        assert_eq!(render("exit\x1b[16X\x1b[16CShutdown the server and save."), "exit                Shutdown the server and save.");
        assert_eq!(render("\x1b[1;1Hplaying\x1b[1;21HShows the list of players."), "playing             Shows the list of players.");
    }

    /// 넓은 글자는 두 칸, 색·커서 숨김·창 제목·글자 집합은 모양에 남지 않는다
    #[test]
    fn wide_characters_take_two_columns_and_other_sequences_vanish() {
        assert_eq!(render("서버\x1b[2C시작"), "서버  시작");
        assert_eq!(render("\x1b[?25l\x1b[32mServer\x1b[0m started\x1b[?25h"), "Server started");
        assert_eq!(render("\x1b]0;TerrariaServer.exe\x07: \x1b(Bhelp"), ": help");
        assert_eq!(render("\x1b]0;title\x1b\\ok"), "ok");
        assert_eq!(render("a\tb"), "a       b");
        assert_eq!(render("\x1b[6n"), "");
    }

    /// 커서를 아주 멀리 보내도 줄이 끝없이 늘지 않는다
    #[test]
    fn far_cursor_moves_stay_bounded() {
        assert!(render("x\x1b[999999999Cy").len() < 8);
        assert_eq!(render("\x1b[999999999;1Hok"), "ok");
    }
}
