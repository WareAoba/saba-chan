//! 데몬 단일 인스턴스 보장.
//!
//! `<data_dir>/daemon.lock`에 OS 배타 잠금을 건다. 잠금은 프로세스가 끝나면
//! OS가 풀어 주므로 비정상 종료 뒤에도 남지 않는다. 두 번째 데몬은
//! IPC 토큰을 덮어쓰기 전에 여기서 멈춘다.
//!
//! 실행 중 데몬 정보(pid, 주소)는 잠금 파일 옆의 `<lock>.owner`에 쓴다.
//! Windows의 배타 잠금은 다른 핸들의 읽기까지 막기 때문에 잠금 파일 자체에는 쓰지 않는다.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// 잠금 파일에 기록하는 실행 중 데몬 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockOwner {
    pub pid: u32,
    pub listen_addr: String,
}

impl LockOwner {
    fn encode(&self) -> String {
        format!("pid={}\naddr={}\n", self.pid, self.listen_addr)
    }

    fn decode(text: &str) -> Option<Self> {
        let mut pid = None;
        let mut addr = None;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("pid=") {
                pid = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("addr=") {
                addr = Some(v.trim().to_string());
            }
        }
        Some(Self { pid: pid?, listen_addr: addr? })
    }
}

/// 잠금을 쥐고 있는 동안 살아 있어야 하는 핸들. drop하면 잠금이 풀린다.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum AcquireError {
    /// 다른 데몬이 이미 실행 중이다. 기록을 읽을 수 있으면 그 정보를 담는다.
    AlreadyRunning(Option<LockOwner>),
    Io(std::io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning(Some(o)) => write!(
                f,
                "another saba-core is already running (pid {}, {})",
                o.pid, o.listen_addr
            ),
            Self::AlreadyRunning(None) => write!(f, "another saba-core is already running"),
            Self::Io(e) => write!(f, "failed to open daemon lock file: {}", e),
        }
    }
}

impl std::error::Error for AcquireError {}

/// 기본 위치(`<data_dir>/daemon.lock`)의 잠금을 얻는다.
pub fn acquire_default(listen_addr: &str) -> Result<InstanceLock, AcquireError> {
    let path = saba_chan_updater_lib::constants::resolve_data_dir().join("daemon.lock");
    acquire(&path, listen_addr)
}

/// `path`에 배타 잠금을 걸고 현재 프로세스 정보를 기록한다.
pub fn acquire(path: &Path, listen_addr: &str) -> Result<InstanceLock, AcquireError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AcquireError::Io)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(AcquireError::Io)?;

    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(AcquireError::AlreadyRunning(read_owner(path)));
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(AcquireError::Io(e)),
    }

    let owner = LockOwner { pid: std::process::id(), listen_addr: listen_addr.to_string() };
    std::fs::write(owner_path(path), owner.encode()).map_err(AcquireError::Io)?;

    Ok(InstanceLock { _file: file })
}

fn owner_path(lock_path: &Path) -> PathBuf {
    let mut name = lock_path.file_name().unwrap_or_default().to_os_string();
    name.push(".owner");
    lock_path.with_file_name(name)
}

/// 잠금 파일 `path`에 대응하는 실행 중 데몬 정보를 읽는다(잠금은 걸지 않는다).
pub fn read_owner(path: &Path) -> Option<LockOwner> {
    LockOwner::decode(&std::fs::read_to_string(owner_path(path)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_reports_running_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.lock");

        let first = acquire(&path, "127.0.0.1:57474").expect("first lock");
        match acquire(&path, "127.0.0.1:57475") {
            Err(AcquireError::AlreadyRunning(Some(owner))) => {
                assert_eq!(owner.pid, std::process::id());
                assert_eq!(owner.listen_addr, "127.0.0.1:57474");
            }
            other => panic!("expected AlreadyRunning, got {:?}", other),
        }

        drop(first);
        let _again = acquire(&path, "127.0.0.1:57476").expect("lock after release");
        assert_eq!(read_owner(&path).unwrap().listen_addr, "127.0.0.1:57476");
    }

    #[test]
    fn decode_rejects_incomplete_record() {
        assert!(LockOwner::decode("pid=12\n").is_none());
        assert!(LockOwner::decode("").is_none());
    }
}
