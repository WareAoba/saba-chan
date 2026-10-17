//! 업데이터 스폰 인증 티켓 — 단일 사용, 만료 시간 기반
//!
//! ## 목적
//! 데몬 API `/api/updates/apply`가 성공했을 때만 티켓을 발급하여
//! 데몬 인증을 통과한 클라이언트만 업데이터를 실행할 수 있도록 보장한다.
//!
//! ## 플로우
//! 1. 데몬: `issue(&["gui", "cli"])` → 랜덤 UUID 토큰 + 티켓 파일 기록 → 토큰 반환
//! 2. 클라이언트(GUI/CLI): API 응답에서 토큰 수신 → `--launch-token <token>`로 업데이터 스폰
//! 3. 업데이터: 시작 시 `consume(token, spawned_by)` 호출 → 티켓 파일 삭제 후 토큰·실행 주체·만료 검증

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::constants;

/// 티켓 유효 시간 — 발급 후 이 시간 내에 업데이터가 스폰되어야 한다.
const TICKET_LIFETIME_SECS: i64 = 120;

#[derive(Serialize, Deserialize)]
struct Ticket {
    token: String,
    allowed_spawners: Vec<String>,
    expires_at: i64,
}

fn ticket_path() -> PathBuf {
    constants::resolve_staging_dir().join("launch-ticket.json")
}

/// 단일 사용 티켓 발급 — 반환된 토큰을 `--launch-token`에 전달해야 한다.
///
/// `allowed_spawners`는 이 티켓으로 허용되는 `--spawned-by` 값 집합.
pub fn issue(allowed_spawners: &[&str]) -> Result<String> {
    issue_at(&ticket_path(), allowed_spawners)
}

pub fn issue_at(path: &std::path::Path, allowed_spawners: &[&str]) -> Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create dir: {}", parent.display()))?;
    }

    let token = Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp() + TICKET_LIFETIME_SECS;

    let ticket = Ticket {
        token: token.clone(),
        allowed_spawners: allowed_spawners.iter().map(|s| s.to_string()).collect(),
        expires_at,
    };

    let json = serde_json::to_string(&ticket)?;
    fs::write(path, json).with_context(|| format!("write: {}", path.display()))?;

    Ok(token)
}

/// 티켓 검증 및 소비 — 성공 여부와 무관하게 티켓 파일은 즉시 삭제된다.
///
/// 실패 시 업데이터는 즉시 종료되어야 한다.
pub fn consume(provided_token: &str, spawned_by: &str) -> Result<()> {
    consume_at(&ticket_path(), provided_token, spawned_by)
}

pub fn consume_at(path: &std::path::Path, provided_token: &str, spawned_by: &str) -> Result<()> {
    if !path.exists() {
        bail!("launch ticket not found — direct execution is not allowed");
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("read: {}", path.display()))?;

    // 일회성 보장: 파싱 전에 파일 삭제 — 실패하더라도 재사용 방지
    let _ = fs::remove_file(path);

    let ticket: Ticket =
        serde_json::from_str(&content).context("parse launch ticket")?;

    if ticket.token != provided_token {
        bail!("launch ticket token mismatch");
    }
    if !ticket.allowed_spawners.iter().any(|s| s == spawned_by) {
        bail!("launch ticket does not allow spawner '{}'", spawned_by);
    }
    if chrono::Utc::now().timestamp() > ticket.expires_at {
        bail!("launch ticket expired");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("launch-ticket.json");
        (tmp, path)
    }

    #[test]
    fn issue_then_consume_ok() {
        let (_tmp, path) = ticket_file();
        let token = issue_at(&path, &["gui"]).unwrap();
        assert!(consume_at(&path, &token, "gui").is_ok());
    }

    #[test]
    fn consume_rejects_wrong_token() {
        let (_tmp, path) = ticket_file();
        let _ = issue_at(&path, &["gui"]).unwrap();
        let err = consume_at(&path, "not-the-token", "gui").unwrap_err();
        assert!(err.to_string().contains("token mismatch"));
    }

    #[test]
    fn consume_fails_without_ticket() {
        let (_tmp, path) = ticket_file();
        let err = consume_at(&path, "any-token", "gui").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn consume_is_single_use() {
        let (_tmp, path) = ticket_file();
        let token = issue_at(&path, &["gui"]).unwrap();
        assert!(consume_at(&path, &token, "gui").is_ok());
        let err = consume_at(&path, &token, "gui").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn consume_rejects_disallowed_spawner() {
        let (_tmp, path) = ticket_file();
        let token = issue_at(&path, &["gui"]).unwrap();
        let err = consume_at(&path, &token, "cli").unwrap_err();
        assert!(err.to_string().contains("does not allow spawner"));
    }

    #[test]
    fn consume_rejects_expired_ticket() {
        let (_tmp, path) = ticket_file();
        // 수동으로 만료된 티켓 작성
        let expired = Ticket {
            token: "abc".to_string(),
            allowed_spawners: vec!["gui".to_string()],
            expires_at: chrono::Utc::now().timestamp() - 10,
        };
        fs::write(&path, serde_json::to_string(&expired).unwrap()).unwrap();
        let err = consume_at(&path, "abc", "gui").unwrap_err();
        assert!(err.to_string().contains("expired"));
    }
}
