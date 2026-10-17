//! 업데이터 `--apply` 실행 표 (C13)
//!
//! 데몬이 `/api/updates/apply`를 받아들였을 때만 표를 하나 끊어 업데이터에 넘긴다.
//! 업데이터는 표가 맞을 때만 적용하고, 맞든 틀리든 표를 지운다(한 번만 쓴다).
//! 그래서 오래된 바로가기나 다른 프로그램이 `--apply`로 띄워도 아무것도 바꾸지 않는다.

use std::path::{Path, PathBuf};

const FILE_NAME: &str = "apply-ticket";

/// 표를 끊기 시작한 데몬 버전. 이보다 오래된 데몬은 `--launch-token` 없이 업데이터를 띄운다.
/// 이 버전 이상 데몬은 기동할 때 자기 버전을 installed-manifest에 적는다(`record_core_version`)
pub const CORE_ISSUES_TICKETS_SINCE: &str = "0.1.2";

/// 전환 경로: 설치된 데몬이 표를 모르는 버전이면 표 없는 적용을 받아 준다.
/// 업데이터만 먼저 새것으로 바뀐 경우에도 옛 데몬이 다음 적용을 할 수 있게 한다.
/// 버전이 없으면 옛 데몬이다(새 데몬은 기동 때 늘 적는다). 읽을 수 없는 버전은 거부
pub fn core_predates_tickets(installed_core: Option<&str>) -> bool {
    use crate::version::SemVer;
    let since = SemVer::parse(CORE_ISSUES_TICKETS_SINCE).expect("valid version");
    match installed_core {
        // 0.1.1 설치는 코어 버전을 적은 적이 없다
        None => true,
        Some(v) => SemVer::parse(v).is_some_and(|v| v < since),
    }
}

/// 데몬 기동 때 부른다: 표를 끊는 데몬이라는 기록
pub fn record_core_version(version: &str) {
    if let Err(e) = crate::UpdateManager::update_installed_version("saba-core", version) {
        tracing::warn!("[Updater] Could not record core version: {}", e);
    }
}

/// 표 없는 적용끼리도 한 번에 하나만: 살아 있는 다른 프로세스가 쥐고 있으면 false.
/// 잠금은 PID가 이미 적힌 임시 파일을 덮어쓰지 않는 하드 링크로 한 번에 만든다.
/// 비었거나 읽을 수 없는 잠금은 쥔 것으로 본다
pub fn claim_legacy_apply(staging_dir: &Path) -> bool {
    let lock = staging_dir.join(format!("{}.legacy-lock", FILE_NAME));
    let pid = std::process::id();
    let tmp = staging_dir.join(format!("{}.legacy-lock.{}", FILE_NAME, pid));
    let _ = std::fs::create_dir_all(staging_dir);
    if std::fs::write(&tmp, pid.to_string()).is_err() {
        return false;
    }
    let mut claimed = false;
    for _ in 0..2 {
        if std::fs::hard_link(&tmp, &lock).is_ok() {
            claimed = true;
            break;
        }
        let Some(dead) = read_pid(&lock) else { break };
        if crate::ProcessChecker::is_pid_running(dead) {
            break;
        }
        // 죽은 프로세스의 잠금: 이름을 바꿔 하나만 차지하고, 그사이 바뀌었으면 되돌린다
        let stale = staging_dir.join(format!("{}.legacy-lock.stale-{}", FILE_NAME, pid));
        if std::fs::rename(&lock, &stale).is_err() {
            break;
        }
        if read_pid(&stale) != Some(dead) {
            let _ = std::fs::hard_link(&stale, &lock);
            let _ = std::fs::remove_file(&stale);
            break;
        }
        let _ = std::fs::remove_file(&stale);
    }
    let _ = std::fs::remove_file(&tmp);
    claimed
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

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

/// 표를 쓴다. 저장된 표와 같으면 true. 어느 쪽이든 저장된 표는 지운다.
/// 이름 바꾸기로 표를 먼저 차지하므로 두 프로세스가 같은 표를 함께 쓸 수 없다
pub fn redeem(staging_dir: &Path, token: &str) -> bool {
    let claimed = staging_dir.join(format!("{}.claimed-{}", FILE_NAME, std::process::id()));
    if std::fs::rename(ticket_path(staging_dir), &claimed).is_err() {
        return false;
    }
    let stored = std::fs::read_to_string(&claimed).ok();
    let _ = std::fs::remove_file(&claimed);
    matches!(stored, Some(s) if !token.is_empty() && s.trim() == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pre_ticket_cores_may_skip_the_ticket() {
        assert!(core_predates_tickets(Some("0.1.1")));
        assert!(core_predates_tickets(None));
        assert!(!core_predates_tickets(Some("garbage")));
        assert!(!core_predates_tickets(Some("0.1.2")));
        assert!(!core_predates_tickets(Some("0.2.0")));
    }

    #[test]
    fn legacy_applies_are_serialised() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(claim_legacy_apply(tmp.path()));
        assert!(!claim_legacy_apply(tmp.path()), "this process still holds it");
        std::fs::write(tmp.path().join("apply-ticket.legacy-lock"), "4000000000").unwrap();
        assert!(claim_legacy_apply(tmp.path()), "a dead holder's lock is taken over");
        std::fs::write(tmp.path().join("apply-ticket.legacy-lock"), "").unwrap();
        assert!(!claim_legacy_apply(tmp.path()), "an empty lock counts as held");
    }

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
