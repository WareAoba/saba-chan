//! 인스턴스 작업 잠금. 인스턴스 파일이나 게임 설정 파일을 다루는 API는 모두 이것을 거친다.
//!
//! - 긴 작업 하나(백업·복원·업데이트·설치·시작·삭제·초기화…)만 한 인스턴스를 쥔다. 이미 쥐어져 있으면 바로 거절한다(409).
//! - 짧은 설정 파일 읽기·쓰기(설정 저장의 configure, 속성 쓰기·초기화, 실제 값 읽기, 약관 동의)는 한 줄로 선다.
//!   서버 파일을 바꾸는 긴 작업(`take_for_writes`) 중에는 거절하고, 파일을 읽기만 하는 작업(`take`, 백업) 중에는 한다
//!   (설정 파일은 원자적으로 바뀌므로 백업에는 예전 파일이나 새 파일 중 하나가 온전히 담긴다).
//! - 서버 파일을 바꾸는 긴 작업은 잡을 때 진행 중인 설정 파일 작업이 끝나기를 기다리고, 끝날 때까지 새 것을 막는다.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Reads,
    Writes,
}

type FileLocks = HashMap<String, Arc<tokio::sync::Mutex<()>>>;

/// 긴 작업을 쥔 키와 그 작업의 종류
static BUSY: LazyLock<Mutex<HashMap<String, Work>>> = LazyLock::new(Default::default);
/// 인스턴스마다 설정 파일 작업 한 줄
static FILES: LazyLock<Mutex<FileLocks>> = LazyLock::new(Default::default);

/// 긴 작업 하나가 쥔 잠금. 놓아질 때 풀린다
pub(crate) struct InstanceLock {
    key: String,
    _files: Option<FilesGuard>,
}

impl InstanceLock {
    /// 파일을 읽기만 하는 긴 작업(백업, 백업 지우기)이나 인스턴스가 아닌 키(`module-install:<id>`)
    pub(crate) fn take(key: &str) -> Option<Self> {
        Self::mark(key, Work::Reads).then(|| Self { key: key.to_string(), _files: None })
    }

    /// 서버 파일을 바꾸는 긴 작업. 진행 중인 설정 파일 작업은 끝나기를 기다리고, 놓을 때까지 새 것은 거절한다
    pub(crate) async fn take_for_writes(id: &str) -> Option<Self> {
        if !Self::mark(id, Work::Writes) {
            return None;
        }
        // 표시를 먼저 해 두었으니 기다리는 동안 새 설정 파일 작업은 들어오지 못한다
        let mut lock = Self { key: id.to_string(), _files: None };
        lock._files = Some(FilesGuard::lock(id).await);
        Some(lock)
    }

    /// `take_for_writes`와 같지만 기다리지 않는다: 설정 파일 작업이 진행 중이면 `None`.
    /// 다른 잠금(supervisor)을 쥔 채 잡는 곳이 쓴다 (설정 저장은 파일 줄을 쥔 채 supervisor를 기다린다)
    pub(crate) fn try_take_for_writes(id: &str) -> Option<Self> {
        if !Self::mark(id, Work::Writes) {
            return None;
        }
        let mut lock = Self { key: id.to_string(), _files: None };
        lock._files = Some(FilesGuard::try_lock(id)?);
        Some(lock)
    }

    /// 잡지 않고 잡혀 있는지만 본다 (잠깐 잡아 보는 것만으로 다른 작업이 409를 받지 않게)
    pub(crate) fn held(key: &str) -> bool {
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).contains_key(key)
    }

    /// 짧은 설정 파일 작업 하나. 서버 파일을 바꾸는 긴 작업 중이면 `None` (바로 거절한다)
    pub(crate) async fn files(id: &str) -> Option<FilesGuard> {
        if Self::writing(id) {
            return None;
        }
        let guard = FilesGuard::lock(id).await;
        // 기다리는 사이 긴 작업이 잡았으면 그 작업은 이 guard가 놓이기를 기다린다: 여기서 놓고 거절한다
        if Self::writing(id) {
            return None;
        }
        Some(guard)
    }

    fn writing(id: &str) -> bool {
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).get(id) == Some(&Work::Writes)
    }

    fn mark(key: &str, work: Work) -> bool {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains_key(key) {
            return false;
        }
        busy.insert(key.to_string(), work);
        true
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // 설정 파일 줄은 표시보다 먼저 놓는다 (필드 순서로는 표시를 지운 뒤에 놓이므로 손으로)
        self._files.take();
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.key);
    }
}

/// 설정 파일 작업 한 줄을 쥔 동안의 잠금
pub(crate) struct FilesGuard {
    id: String,
    lock: Arc<tokio::sync::Mutex<()>>,
    _held: OwnedMutexGuard<()>,
}

impl FilesGuard {
    fn try_lock(id: &str) -> Option<Self> {
        let lock = FILES.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone();
        let held = lock.clone().try_lock_owned().ok()?;
        Some(Self { id: id.to_string(), lock, _held: held })
    }

    async fn lock(id: &str) -> Self {
        let lock = FILES.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone();
        let held = lock.clone().lock_owned().await;
        Self { id: id.to_string(), lock, _held: held }
    }
}

impl Drop for FilesGuard {
    fn drop(&mut self) {
        // 기다리는 쪽이 없으면 항목을 지운다: 지운 인스턴스·없는 id의 항목이 쌓이지 않게 (맵, 이 guard, 쥔 잠금이 센다)
        let mut locks = FILES.lock().unwrap_or_else(|e| e.into_inner());
        if Arc::strong_count(&self.lock) == 3 && locks.get(&self.id).is_some_and(|l| Arc::ptr_eq(l, &self.lock)) {
            locks.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn key(name: &str) -> String {
        format!("lock-test-{}-{}", name, std::process::id())
    }

    #[test]
    fn one_long_task_at_a_time() {
        let id = key("one");
        let first = InstanceLock::take(&id).expect("free");
        assert!(InstanceLock::take(&id).is_none());
        assert!(InstanceLock::held(&id));
        drop(first);
        assert!(!InstanceLock::held(&id));
        assert!(InstanceLock::take(&id).is_some());
    }

    #[tokio::test]
    async fn settings_files_are_refused_while_server_files_change_but_not_during_a_backup() {
        let id = key("files");
        let backup = InstanceLock::take(&id).unwrap();
        assert!(InstanceLock::files(&id).await.is_some(), "a backup only reads: saving settings goes on");
        drop(backup);

        let update = InstanceLock::take_for_writes(&id).await.unwrap();
        assert!(InstanceLock::files(&id).await.is_none(), "an update is changing the files");
        assert!(InstanceLock::take_for_writes(&id).await.is_none());
        drop(update);
        let saving = InstanceLock::files(&id).await.unwrap();
        assert!(InstanceLock::try_take_for_writes(&id).is_none(), "does not wait for the settings write");
        assert!(!InstanceLock::held(&id), "a refused take leaves no mark");
        drop(saving);
        assert!(InstanceLock::files(&id).await.is_some());
    }

    #[tokio::test]
    async fn a_writing_task_waits_for_the_settings_write_in_progress() {
        let id = key("wait");
        let saving = InstanceLock::files(&id).await.unwrap();
        let taker = tokio::spawn({
            let id = id.clone();
            async move { InstanceLock::take_for_writes(&id).await.is_some() }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!taker.is_finished(), "the update waits for the settings write to finish");
        assert!(InstanceLock::files(&id).await.is_none(), "no new settings write slips in while it waits");
        drop(saving);
        assert!(taker.await.unwrap());
    }

    #[tokio::test]
    async fn lock_entries_do_not_pile_up() {
        let id = key("pile");
        drop(InstanceLock::files(&id).await.unwrap());
        drop(InstanceLock::take_for_writes(&id).await.unwrap());
        assert!(!FILES.lock().unwrap().contains_key(&id));
        assert!(!BUSY.lock().unwrap().contains_key(&id));
    }
}
