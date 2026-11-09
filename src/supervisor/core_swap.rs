//! 코어(데몬) 교체를 막는 켜진 서버 판정
//!
//! 러너가 쥔 서버는 데몬이 바뀌어도 콘솔·stdin까지 다시 붙는다(`reattach_orphaned_managed`).
//! 그렇지 않은 켜진 서버가 하나라도 있으면 업데이트는 그 서버가 꺼질 때까지 기다린다.
//! - 러너 없이 데몬이 파이프로 직접 문 서버: 교체 뒤 입력·실시간 콘솔을 잃고, 파이프 쓰기로 죽을 수도 있다
//! - 익스텐션이 붙인 로그 따라가기·입출력 다리: 새 데몬이 다리를 다시 띄우지 않는다
//! - 새 코어가 읽지 못하는 형식의 러너 (`saba_runner::RUNNER_FORMAT`, `READS_RUNNER_FORMATS`)

use super::managed_process::ManagedProcess;
use super::Supervisor;
use serde::{Deserialize, Serialize};

/// 무엇이 교체를 막는가. 업데이트 상태 응답의 `waiting_for[].kind`와 같은 이름
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreSwapBlockerKind {
    NonRunnerServer,
    ExtensionBridge,
    RunnerFormat,
    /// 데몬 작업 (`ipc::update_phase::daemon_work`): 백업·복원
    Backup,
    /// 서버 설치·모듈 설치
    Provision,
    /// 게임 서버 업데이트
    ServerUpdate,
    /// 모듈·익스텐션 호출, 이름표 없는 인스턴스 작업(시작·초기화·설정 저장)
    ModuleCall,
}

impl CoreSwapBlockerKind {
    /// 데몬 작업(백업·설치·게임 서버 업데이트·모듈 호출)인가. 켜진 서버 때문이면 false
    pub fn is_daemon_job(self) -> bool {
        matches!(self, Self::Backup | Self::Provision | Self::ServerUpdate | Self::ModuleCall)
    }
}

/// 교체를 막는 켜진 서버 하나 (`waiting_for` 항목과 같은 모양)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSwapBlocker {
    pub kind: CoreSwapBlockerKind,
    pub instance_id: String,
    pub name: String,
}

/// 켜진 관리형 프로세스 하나가 교체를 막는 까닭. 막지 않으면 None
fn blocker_kind(proc: &ManagedProcess, new_core_reads: (u32, u32)) -> Option<CoreSwapBlockerKind> {
    if !proc.is_running() {
        return None;
    }
    if proc.log_follower {
        return Some(CoreSwapBlockerKind::ExtensionBridge);
    }
    match proc.runner_format {
        Some(format) => {
            let (min, max) = new_core_reads;
            (format < min || format > max).then_some(CoreSwapBlockerKind::RunnerFormat)
        }
        // 이전 데몬에서 이미 파이프를 잃고 다시 붙은 서버는 교체로 더 잃을 것이 없다
        None if proc.reattached => None,
        None => Some(CoreSwapBlockerKind::NonRunnerServer),
    }
}

impl Supervisor {
    /// 새 코어(`new_core_reads` = 그 코어가 읽는 러너 형식 범위, 양 끝 포함)로 바꾸면 끊기는 켜진 서버들.
    /// 비어 있어야 데몬을 바꾼다
    pub async fn core_swap_blockers(&self, new_core_reads: (u32, u32)) -> Vec<CoreSwapBlocker> {
        let mut blockers: Vec<CoreSwapBlocker> = self
            .managed_store
            .snapshot()
            .await
            .into_iter()
            .filter_map(|(instance_id, proc)| {
                let kind = blocker_kind(&proc, new_core_reads)?;
                let name = self
                    .instance_store
                    .get(&instance_id)
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| instance_id.clone());
                Some(CoreSwapBlocker {
                    kind,
                    instance_id,
                    name,
                })
            })
            .collect();
        blockers.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        blockers
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::supervisor::managed_process::READS_RUNNER_FORMATS;
    use serde_json::json;

    fn supervisor(tmp: &std::path::Path) -> Supervisor {
        Supervisor::new_with_instances_dir(
            tmp.join("modules").to_str().unwrap(),
            tmp.join("instances").to_str().unwrap(),
        )
    }

    fn add(sup: &mut Supervisor, name: &str) -> String {
        let inst = crate::instance::ServerInstance::new(name, "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        id
    }

    async fn sleeper(dir: &std::path::Path) -> ManagedProcess {
        ManagedProcess::spawn(
            "sh",
            &["-c".into(), "sleep 30".into()],
            dir.to_str().unwrap(),
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap()
    }

    /// 러너가 쥔 척하는 서버: 살아 있는 러너 대신 이 테스트 프로세스. `format`이 None이면 형식 칸이 없는 옛 runner.json
    /// (이 프로세스를 가리키므로 force_kill하지 않는다)
    async fn runner_held(dir: &std::path::Path, format: Option<u32>) -> ManagedProcess {
        let run = crate::supervisor::managed_process::run_dir(dir);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("token"), "tok").unwrap();
        let log = dir.join("session.log");
        std::fs::write(&log, "").unwrap();
        let mut info = json!({ "runner_pid": std::process::id(), "pid": std::process::id(), "port": 1, "log": log, "started_at": saba_runner::now() });
        if let Some(f) = format {
            info["format"] = json!(f);
        }
        std::fs::write(saba_runner::runner_file(&run), info.to_string()).unwrap();
        ManagedProcess::attach_runner(dir, None).await.expect("attached")
    }

    /// 데몬이 읽는 러너 형식 범위는 지금 러너와 형식 칸이 없는 옛 러너를 모두 담는다 (발행 때 manifest로 들어가는 값)
    #[test]
    fn the_daemon_reads_the_current_and_the_unnumbered_runner() {
        assert_eq!(READS_RUNNER_FORMATS, (0, 1));
        assert!(
            READS_RUNNER_FORMATS.0 <= saba_runner::RUNNER_FORMAT
                && saba_runner::RUNNER_FORMAT <= READS_RUNNER_FORMATS.1
        );
    }

    /// ★1: 러너 없이 데몬이 직접 문 켜진 서버가 있으면 교체하지 않는다. 꺼지면 막지 않는다
    #[tokio::test]
    async fn a_server_the_daemon_holds_without_a_runner_blocks_the_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = supervisor(tmp.path());
        let id = add(&mut sup, "직접 띄운 서버");
        let proc = std::sync::Arc::new(sleeper(tmp.path()).await);
        sup.managed_store.insert(&id, proc.clone()).await;

        let blockers = sup.core_swap_blockers(READS_RUNNER_FORMATS).await;
        assert_eq!(
            blockers,
            vec![CoreSwapBlocker {
                kind: CoreSwapBlockerKind::NonRunnerServer,
                instance_id: id.clone(),
                name: "직접 띄운 서버".into()
            }]
        );

        proc.force_kill().await.unwrap();
        for _ in 0..50 {
            if !proc.is_running() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            sup.core_swap_blockers(READS_RUNNER_FORMATS).await.is_empty(),
            "a stopped server does not block"
        );
    }

    /// ★2: 익스텐션이 붙인 로그 따라가기와 입출력 다리는 새 데몬이 다시 띄우지 않으므로 교체를 막는다
    #[tokio::test]
    async fn an_extension_bridge_blocks_the_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = supervisor(tmp.path());
        let follower_id = add(&mut sup, "컨테이너 로그");
        let bridge_id = add(&mut sup, "컨테이너 입출력");
        let mut follower = ManagedProcess::spawn_log_follower(
            "sh",
            &["-c".into(), "sleep 30".into()],
            tmp.path(),
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        follower.log_follower = true;
        // interactive 다리는 `spawn`으로 뜨지만 익스텐션 다리로 표시된다
        let mut bridge = sleeper(tmp.path()).await;
        bridge.log_follower = true;
        let (follower, bridge) = (std::sync::Arc::new(follower), std::sync::Arc::new(bridge));
        sup.managed_store.insert(&follower_id, follower.clone()).await;
        sup.managed_store.insert(&bridge_id, bridge.clone()).await;

        let mut blockers = sup.core_swap_blockers(READS_RUNNER_FORMATS).await;
        blockers.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            blockers,
            vec![
                CoreSwapBlocker {
                    kind: CoreSwapBlockerKind::ExtensionBridge,
                    instance_id: follower_id,
                    name: "컨테이너 로그".into()
                },
                CoreSwapBlocker {
                    kind: CoreSwapBlockerKind::ExtensionBridge,
                    instance_id: bridge_id,
                    name: "컨테이너 입출력".into()
                },
            ]
        );
        let _ = follower.force_kill().await;
        let _ = bridge.force_kill().await;
    }

    /// 러너가 쥔 서버는 새 코어가 그 러너 형식을 읽을 때만 막지 않는다. 형식 칸이 없는 옛 러너는 0이다
    #[tokio::test]
    async fn a_runner_outside_the_new_core_formats_blocks_the_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = supervisor(tmp.path());
        let now_id = add(&mut sup, "지금 러너");
        let old_id = add(&mut sup, "옛 러너");
        let (now_dir, old_dir) = (tmp.path().join("now"), tmp.path().join("old"));
        std::fs::create_dir_all(&now_dir).unwrap();
        std::fs::create_dir_all(&old_dir).unwrap();
        sup.managed_store
            .insert(&now_id, runner_held(&now_dir, Some(saba_runner::RUNNER_FORMAT)).await)
            .await;
        sup.managed_store
            .insert(&old_id, runner_held(&old_dir, None).await)
            .await;

        assert!(
            sup.core_swap_blockers((0, 1)).await.is_empty(),
            "the reading core takes both runners over"
        );
        assert_eq!(
            sup.core_swap_blockers((1, 2)).await,
            vec![CoreSwapBlocker {
                kind: CoreSwapBlockerKind::RunnerFormat,
                instance_id: old_id.clone(),
                name: "옛 러너".into()
            }]
        );
        let mut both = sup.core_swap_blockers((2, 3)).await;
        both.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            both.iter()
                .map(|b| (b.kind, b.instance_id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (CoreSwapBlockerKind::RunnerFormat, old_id.as_str()),
                (CoreSwapBlockerKind::RunnerFormat, now_id.as_str()),
            ]
        );
    }

    /// 이전 데몬에서 이미 파이프를 잃고 다시 붙은 서버는 교체로 더 잃을 것이 없어 막지 않는다 (막으면 그 서버가 꺼질 때까지 업데이트가 멈춘다)
    #[tokio::test]
    async fn a_server_that_already_lost_its_pipes_does_not_block() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = supervisor(tmp.path());
        let id = add(&mut sup, "다시 붙은 서버");
        let stub = ManagedProcess::create_reattached_stub(std::process::id(), tmp.path()).await;
        sup.managed_store.insert(&id, stub).await;
        assert!(sup.core_swap_blockers(READS_RUNNER_FORMATS).await.is_empty());
    }

    /// 업데이트 상태 응답의 `waiting_for` 항목과 같은 모양으로 직렬화된다
    #[test]
    fn a_blocker_serializes_as_a_waiting_for_entry() {
        let b = |kind| {
            serde_json::to_value(CoreSwapBlocker {
                kind,
                instance_id: "i".into(),
                name: "n".into(),
            })
            .unwrap()
        };
        assert_eq!(
            b(CoreSwapBlockerKind::NonRunnerServer),
            json!({ "kind": "non_runner_server", "instance_id": "i", "name": "n" })
        );
        assert_eq!(b(CoreSwapBlockerKind::ExtensionBridge)["kind"], "extension_bridge");
        assert_eq!(b(CoreSwapBlockerKind::RunnerFormat)["kind"], "runner_format");
    }
}
