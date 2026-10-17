//! SHA256SUMS 파일 파서 — nodejs.org / python-build-standalone 형식 공통
//!
//! 형식(업스트림 양쪽 동일):
//! ```text
//! <64-hex>  <filename>
//! <64-hex> *<filename>      # "*"는 바이너리 모드 표식
//! ```
//!
//! 빈 줄 / 주석(`#`) / 공백은 무시.

use super::hash::{FileHash, IntegrityError};

/// SHA256SUMS 본문에서 특정 파일명의 해시를 찾아 반환.
///
/// - 파일명 부분에서 선행 `*`(바이너리 모드 표식)는 자동 제거.
/// - 정확히 일치하는 파일명만 매치(부분/접두사 매치 금지).
pub fn find_hash_for_filename(
    sums_body: &str,
    filename: &str,
) -> Result<FileHash, IntegrityError> {
    for raw in sums_body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(hash_hex) = it.next() else { continue };
        let Some(name_raw) = it.next() else { continue };
        let name = name_raw.trim_start_matches('*');
        if name == filename {
            return FileHash::parse_hex(hash_hex);
        }
    }
    Err(IntegrityError::InvalidHashFormat(format!(
        "SHA256SUMS 본문에서 '{}'을(를) 찾지 못함",
        filename
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE_SAMPLE: &str = "\
abc123def4567890abc123def4567890abc123def4567890abc123def4567890  node-v22.14.0-linux-x64.tar.gz
1111111111111111111111111111111111111111111111111111111111111111 *node-v22.14.0-win-x64.zip
# comment line
2222222222222222222222222222222222222222222222222222222222222222  node-v22.14.0-darwin-arm64.tar.gz
";

    #[test]
    fn finds_exact_filename_linux() {
        let h = find_hash_for_filename(NODE_SAMPLE, "node-v22.14.0-linux-x64.tar.gz").unwrap();
        assert_eq!(
            h.to_hex_lower(),
            "abc123def4567890abc123def4567890abc123def4567890abc123def4567890"
        );
    }

    #[test]
    fn strips_binary_mode_star_prefix() {
        let h = find_hash_for_filename(NODE_SAMPLE, "node-v22.14.0-win-x64.zip").unwrap();
        assert_eq!(
            h.to_hex_lower(),
            "1111111111111111111111111111111111111111111111111111111111111111"
        );
    }

    #[test]
    fn rejects_unknown_filename() {
        let err = find_hash_for_filename(NODE_SAMPLE, "node-v99.0.0-linux-x64.tar.gz")
            .unwrap_err();
        assert!(matches!(err, IntegrityError::InvalidHashFormat(_)));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let body = "\n# header\n\n3333333333333333333333333333333333333333333333333333333333333333  foo.tar.gz\n";
        let h = find_hash_for_filename(body, "foo.tar.gz").unwrap();
        assert_eq!(
            h.to_hex_lower(),
            "3333333333333333333333333333333333333333333333333333333333333333"
        );
    }

    #[test]
    fn does_not_match_suffix() {
        // "x64.zip" 이라는 쿼리가 "win-x64.zip"에 부분매치되면 안됨
        let err = find_hash_for_filename(NODE_SAMPLE, "x64.zip").unwrap_err();
        assert!(matches!(err, IntegrityError::InvalidHashFormat(_)));
    }
}
