//! 찾기 맞춤: 대소문자를 가리지 않고, 한글은 초성만 쳐도 맞는다 (ㄴㄹ → 노래)

/// 한글 음절의 초성 (호환 자모)
const INITIALS: [char; 19] = ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];

fn initial(c: char) -> Option<char> {
    let code = c as u32;
    (0xAC00..=0xD7A3).contains(&code).then(|| INITIALS[((code - 0xAC00) / 588) as usize])
}

fn same(q: char, t: char) -> bool {
    q == t || q.to_lowercase().eq(t.to_lowercase()) || (INITIALS.contains(&q) && initial(t) == Some(q))
}

/// `query`가 `text` 안에 이어서 나오는가 (공백은 건너뛴다)
pub fn matches(text: &str, query: &str) -> bool {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return true;
    }
    let t: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    t.windows(q.len()).any(|w| w.iter().zip(&q).all(|(t, q)| same(*q, *t)))
}

/// 맞춤 점수: 앞에서 맞을수록 높다. 안 맞으면 None
pub fn score(text: &str, query: &str) -> Option<usize> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let at = t.windows(q.len()).position(|w| w.iter().zip(&q).all(|(t, q)| same(*q, *t)))?;
    Some(1000usize.saturating_sub(at * 10 + t.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_case_and_spaces() {
        assert!(matches("노래하는 사바쨩", "ㄴㄹ"));
        assert!(matches("노래하는 사바쨩", "노래ㅎ"));
        assert!(matches("Server Type", "server t"));
        assert!(matches("서버 종류", "ㅅㅂㅈ"));
        assert!(!matches("노래", "ㄹㄴ"));
        assert!(score("노래", "ㄴ") > score("음악 노래", "ㄴ"));
    }
}
