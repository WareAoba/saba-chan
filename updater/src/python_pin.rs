//! 포터블 Python(python-build-standalone) 고정 판과 SHA-256
//!
//! 데몬(`python_env`)과 설치기(`runtime_bootstrap`)가 같은 판을 같은 해시로 받는다.
//! 해시는 upstream `releases/download/{PYTHON_RELEASE_TAG}/SHA256SUMS`에서 옮겨 적었다.
//! 판을 올릴 때는 그 파일에서 아래 다섯 타겟의 `install_only_stripped` 해시를 다시 옮긴다.

pub const PYTHON_VERSION: &str = "3.12.8";
pub const PYTHON_RELEASE_TAG: &str = "20250106";

/// (타겟 트리플, sha256)
const PINS: [(&str, &str); 5] = [
    ("x86_64-pc-windows-msvc", "2c6af885033be26a8a2ac2e069aac4889c53ab9d212fdffe0d7c8157b66a3d14"),
    ("x86_64-unknown-linux-gnu", "5591da823631467e204baa520ee379ba09b5699b2e2c322c456c44cb2c2e60eb"),
    ("aarch64-unknown-linux-gnu", "29df00dc1435f536c50396d1753fb507f81a81c93a04a7c24b145c9ad68798f8"),
    ("x86_64-apple-darwin", "00e2d29ce8a688f5d2f192718e6f87e4aedae88ac2a0958011b8af9a49e049a7"),
    ("aarch64-apple-darwin", "3a31b4f82d589a0ff6efac3cdc1e55284cb67645470b78c5aa50f156c10a93d2"),
];

/// 이 플랫폼의 타겟 트리플. 지원하지 않으면 None
pub fn target_triple() -> Option<&'static str> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    };
    Some(triple)
}

/// 받을 주소와 기대 sha256. `SABA_PYTHON_URL`(미러)이 있으면 그 주소를 쓰고,
/// 해시는 `SABA_PYTHON_SHA256`이 있을 때만 검사한다 (미러 판은 고정 해시와 다를 수 있다)
pub fn download() -> Option<(String, Option<String>)> {
    if let Ok(url) = std::env::var("SABA_PYTHON_URL") {
        if !url.is_empty() {
            return Some((url, std::env::var("SABA_PYTHON_SHA256").ok().filter(|s| !s.is_empty())));
        }
    }
    let triple = target_triple()?;
    let sha = PINS.iter().find(|(t, _)| *t == triple).map(|(_, s)| s.to_string());
    Some((
        format!(
            "https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz",
            tag = PYTHON_RELEASE_TAG,
            ver = PYTHON_VERSION,
        ),
        sha,
    ))
}

/// 받은 파일이 기대 해시와 같은지. 기대 해시가 없으면 통과
pub fn verify(path: &std::path::Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else { return Ok(()) };
    let got = crate::integrity::compute_sha256(path)?;
    if got.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(format!("Python archive sha256 mismatch: expected {}, got {}", expected, got))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_target_is_pinned() {
        for (triple, sha) in PINS {
            assert_eq!(sha.len(), 64, "{triple}");
        }
        if let Some(triple) = target_triple() {
            assert!(PINS.iter().any(|(t, _)| *t == triple));
        }
    }

    #[test]
    fn verify_rejects_a_wrong_hash() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(tmp.path(), Some(abc)).is_ok());
        assert!(verify(tmp.path(), Some(&"0".repeat(64))).is_err());
        assert!(verify(tmp.path(), None).is_ok());
    }
}
