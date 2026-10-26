//! 의사 콘솔 한 줄을 터미널이 그린 대로 글자로 옮긴다.
//!
//! 의사 콘솔(Windows ConPTY)은 서버가 쓴 글자를 화면으로 다시 그려 보낸다. 그래서 줄 안에
//! - `\r`로 같은 줄을 덮어 쓴 내용과
//! - 빈칸 대신 커서 이동(`ESC[nC`)이나 글자 지우기(`ESC[nX`)로 맞춘 칸(테라리아 help)
//!
//! 이 섞여 온다. 제어 문자를 그냥 지우면 덮어 쓴 내용이 이어 붙고 칸이 사라진다("helpDisplays").
//! 한 줄짜리 화면에 그려 보고 마지막 모습만 남긴다. 다른 화면 줄로 가는 이동(`ESC[r;cH`의 줄이 바뀜, `ESC[nA`·`ESC[nB`)이
//! 나오면 거기서 줄을 나눈다.

use unicode_width::UnicodeWidthChar;

/// 커서 이동이 줄을 끝없이 늘리지 않게 (의사 콘솔 폭)
const MAX_COLUMNS: usize = 4096;

/// 화면 한 줄. 칸마다 글자(뒤따르는 결합 문자 포함). 넓은 글자의 오른쪽 칸은 빈 글자다
#[derive(Default)]
struct Line {
    cells: Vec<String>,
    col: usize,
    /// `ESC[r;cH`로 마지막에 간 화면 줄
    row: Option<usize>,
    done: Vec<String>,
}

impl Line {
    fn put(&mut self, c: char) {
        let width = c.width().unwrap_or(0);
        if width == 0 {
            // 결합 문자·ZWJ 등은 앞 글자에 붙인다
            if let Some(cell) = self.col.checked_sub(1).and_then(|i| self.cells.get_mut(i)) {
                cell.push(c);
            }
            return;
        }
        // 줄 끝을 넘으면 터미널처럼 다음 줄로 넘긴다 (긴 출력도 버리지 않는다)
        if self.col + width > MAX_COLUMNS {
            self.next_row();
            self.col = 0;
        }
        if self.cells.len() < self.col + width {
            self.cells.resize(self.col + width, " ".to_string());
        }
        // 넓은 글자의 반쪽을 덮으면 남은 반쪽은 빈칸이 된다
        for i in self.col..self.col + width {
            if self.cells[i].is_empty() && i > 0 {
                self.cells[i - 1] = " ".to_string();
            }
            if self.cells.get(i + 1).is_some_and(String::is_empty) && i + 1 >= self.col + width {
                self.cells[i + 1] = " ".to_string();
            }
        }
        self.cells[self.col] = c.to_string();
        if width == 2 {
            self.cells[self.col + 1] = String::new();
        }
        self.col += width;
    }

    fn move_to(&mut self, col: usize) {
        self.col = col.min(MAX_COLUMNS);
    }

    /// 다른 화면 줄로 갔다: 지금까지 그린 것을 한 줄로 내놓고 새 줄에서 시작한다
    fn next_row(&mut self) {
        let text = self.text();
        if !text.is_empty() {
            self.done.push(text);
        }
        self.cells.clear();
    }

    /// `ESC[` 뒤 매개변수와 끝 글자
    fn csi(&mut self, params: &str, end: char) {
        let nums: Vec<usize> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let n = nums.first().copied().filter(|n| *n > 0).unwrap_or(1);
        match end {
            'C' => self.move_to(self.col.saturating_add(n)),
            'D' => self.move_to(self.col.saturating_sub(n)),
            'G' | '`' => self.move_to(n - 1),
            'A' | 'B' => {
                self.next_row();
                self.row = None;
            }
            'H' | 'f' => {
                if self.row.is_some_and(|row| row != n) {
                    self.next_row();
                }
                self.row = Some(n);
                self.move_to(nums.get(1).copied().filter(|c| *c > 0).unwrap_or(1) - 1);
            }
            'K' => match nums.first().copied().unwrap_or(0) {
                0 => self.cells.truncate(self.col),
                1 => self.cells.iter_mut().take(self.col + 1).for_each(|c| *c = " ".to_string()),
                _ => self.cells.clear(),
            },
            'X' => self.cells.iter_mut().skip(self.col).take(n).for_each(|c| *c = " ".to_string()),
            _ => {}
        }
    }

    fn text(&self) -> String {
        self.cells.concat().trim_end().to_string()
    }
}

/// 의사 콘솔이 보낸 한 줄(줄 끝은 뗀 글자)을 화면에 보인 모습으로. 다른 화면 줄로 가는 이동이 있으면 여러 줄이다
pub fn render(raw: &str) -> Vec<String> {
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
    line.next_row();
    line.done
}

#[cfg(test)]
mod tests {
    fn render(raw: &str) -> String {
        super::render(raw).join("\n")
    }

    /// `\r`로 같은 줄을 덮어 쓴 것은 마지막 모습 한 줄이다
    #[test]
    fn a_line_redrawn_with_carriage_returns_keeps_its_last_state() {
        assert_eq!(render("Resetting game objects 10%\rResetting game objects 11%\rResetting game objects 12%"), "Resetting game objects 12%");
        // 의사 콘솔은 바뀐 칸만 다시 그리기도 한다
        assert_eq!(render("\x1b[3;1HLoading world data: 9%\x1b[3;21H10%"), "Loading world data: 10%");
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

    /// 화면 여러 줄을 한 번에 다시 그리면(스크롤·크기 바꿈 뒤) 줄마다 나눈다
    #[test]
    fn moves_to_another_screen_row_split_the_line() {
        assert_eq!(super::render("\x1b[1;1HServer v1.4\x1b[2;1HListening on 7777"), ["Server v1.4", "Listening on 7777"]);
        assert_eq!(super::render("first\x1b[A\rsecond"), ["first", "second"]);
        assert!(super::render("\x1b[2;1H\x1b[5;1H").is_empty());
    }

    /// 넓은 글자는 두 칸, 결합 문자는 앞 글자에 붙고, 색·커서 숨김·창 제목·글자 집합은 모양에 남지 않는다
    #[test]
    fn wide_and_combining_characters_and_other_sequences() {
        assert_eq!(render("서버\x1b[2C시작"), "서버  시작");
        assert_eq!(render("cafe\u{301} \u{1100}\u{1161}"), "cafe\u{301} \u{1100}\u{1161}");
        // 넓은 글자의 반쪽을 덮으면 남은 반쪽은 빈칸이다
        assert_eq!(render("서버\rX"), "X 버");
        assert_eq!(render("서버\x1b[2GX"), " X버");
        assert_eq!(render("\x1b[?25l\x1b[32mServer\x1b[0m started\x1b[?25h"), "Server started");
        assert_eq!(render("\x1b]0;TerrariaServer.exe\x07: \x1b(Bhelp"), ": help");
        assert_eq!(render("\x1b]0;title\x1b\\ok"), "ok");
        assert_eq!(render("a\tb"), "a       b");
        assert_eq!(render("\x1b[6n"), "");
    }

    /// 커서를 아주 멀리 보내도 줄이 끝없이 늘지 않는다
    #[test]
    fn far_cursor_moves_stay_bounded() {
        assert_eq!(super::render("x\x1b[999999999Cy"), ["x", "y"]);
        // 폭보다 긴 출력은 다음 줄로 넘어갈 뿐 잃지 않는다 (리뷰 #605)
        let long = format!("{}END", "a".repeat(5000));
        assert_eq!(super::render(&long).concat(), long);
        assert_eq!(render("\x1b[999999999;1Hok"), "ok");
    }
}
