//! 업데이터 `--apply` 실행 표 (C13)
//!
//! 데몬이 `/api/updates/apply`를 받아들였을 때만 표를 하나 끊어 업데이터에 넘긴다.
//! 업데이터는 표가 맞을 때만 적용하고, 맞든 틀리든 표를 지운다(한 번만 쓴다).
//! 그래서 오래된 바로가기나 다른 프로그램이 `--apply`로 띄워도 아무것도 바꾸지 않는다.

use std::path::{Path, PathBuf};

const FILE_NAME: &str = "apply-ticket";

fn ticket_path(staging_dir: &Path) -> PathBuf {
    staging_dir.join(FILE_NAME)
}

/// 새 표를 끊어 저장하고 그 값을 돌려준다 (이전 표는 무효)
pub fn issue(staging_dir: &Path) -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| std::io::Error::other("no system randomness"))?;
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    std::fs::create_dir_all(staging_dir)?;
    std::fs::write(ticket_path(staging_dir), &token)?;
    Ok(token)
}

/// 표를 쓴다. 저장된 표와 같으면 true. 어느 쪽이든 저장된 표는 지운다
pub fn redeem(staging_dir: &Path, token: &str) -> bool {
    let path = ticket_path(staging_dir);
    let stored = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    matches!(stored, Some(s) if !token.is_empty() && s.trim() == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_works_once() {
        let tmp = tempfile::tempdir().unwrap();
        let token = issue(tmp.path()).unwrap();
        assert!(!redeem(tmp.path(), "wrong"), "a wrong token fails");
        let token2 = issue(tmp.path()).unwrap();
        assert_ne!(token, token2);
        assert!(redeem(tmp.path(), &token2));
        assert!(!redeem(tmp.path(), &token2), "used once only");
    }
}
