//! 인스턴스별 런타임 상태: 데몬이 도는 동안만 쌓이고 파일에 남지 않는 것 (정지 표시, 실행 세대, 재시작·종료 이력,
//! 접속자 수, 준비 알림, 정지 문, 명령 목록 기억 등). 감독자가 `Runtimes` 하나(`Supervisor::runtime`)를 갖고,
//! 인스턴스를 지우면 그 항목 하나만 지운다(`Runtimes::forget`). 지운 뒤 늦게 도착한 쓰기(진행 중이던 상태 질문·정지 등)가
//! 항목을 되살리지 않도록 지운 id는 기억한다 (인스턴스 id는 새로 만들 때마다 새 UUID라 다시 쓰이지 않는다).
//! 인스턴스마다 새 상태가 생기면 여기 칸을 더한다.
//!
//! 잠금은 맨 안쪽이다: 이 잠금을 쥔 채 다른 잠금을 잡거나 await하지 않는다 (닫는 함수 안에서 끝낸다)

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{crash, players, ready};

/// 인스턴스 하나의 런타임 상태
#[derive(Default)]
pub(crate) struct InstanceRuntime {
    /// 데몬이 정지를 보냈고 그 정지를 시킨 쪽. 이 상태로 끝나면 사용자 정지다 (A1)
    pub stop_requested: Option<crate::audit::Origin>,
    /// 정지 요청과 시작마다 오르는 실행 세대. 예약된 재시작은 세대가 그대로일 때만 실행한다
    pub run_generation: u64,
    /// 자동 재시작 시각 (한도 계산용)
    pub restart_history: VecDeque<Instant>,
    /// 종료 이력 (GET /api/instance/:id/events). `None`이면 아직 파일에서 읽지 않았다
    pub exit_events: Option<VecDeque<crash::ExitEvent>>,
    /// 정지한 때. 이 뒤 잠깐은 auto-detect가 다시 붙지 않는다
    pub stop_cooldown: Option<Instant>,
    /// 지금 시작 중이다. 시작은 락을 놓고 진행하므로 중복 시작을 이것으로 막는다 (C12, `StartGuard`)
    pub starting: bool,
    /// 마지막으로 켠 쪽. 켜진 뒤 곧 끝난 시작 실패(`instance.exit` `start_failed`)에 싣는다
    pub start_origin: Option<crate::audit::Origin>,
    /// 접속자 수와 응답 없음 기록 (`players.rs`)
    pub players: players::Players,
    /// 준비 알림 (`ready.rs`)
    pub ready: ready::Ready,
    /// 정지와 복원 전 백업 정리가 함께 쥐는 문 (`ipc/handlers/backup.rs` `stop_gate`)
    pub stop_gate: Arc<tokio::sync::Mutex<()>>,
    /// 모듈 `instance_commands()` 결과와 읽은 때 (`ipc/handlers/command.rs`)
    pub commands: Option<(Instant, crate::commands::InstanceCommands)>,
}

#[derive(Default)]
struct Inner {
    live: HashMap<String, InstanceRuntime>,
    /// 지운 인스턴스 id. 이 id에는 항목을 다시 만들지 않는다
    gone: HashSet<String>,
}

/// 인스턴스 id → 런타임 상태. 복제해도 같은 상태를 가리킨다
#[derive(Clone, Default)]
pub struct Runtimes(Arc<Mutex<Inner>>);

impl Runtimes {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 그 인스턴스의 상태를 고친다 (없으면 만든다). 지운 인스턴스면 빈 상태에 고치고 버린다
    pub(crate) fn with<R>(&self, instance_id: &str, f: impl FnOnce(&mut InstanceRuntime) -> R) -> R {
        let mut inner = self.lock();
        if inner.gone.contains(instance_id) {
            return f(&mut InstanceRuntime::default());
        }
        f(inner.live.entry(instance_id.to_string()).or_default())
    }

    /// 상태가 있을 때만 읽거나 고친다 (지우기·읽기는 항목을 새로 만들지 않는다)
    pub(crate) fn existing<R>(&self, instance_id: &str, f: impl FnOnce(&mut InstanceRuntime) -> R) -> Option<R> {
        self.lock().live.get_mut(instance_id).map(f)
    }

    /// 모든 인스턴스의 상태를 읽는다
    pub(crate) fn all<R>(&self, f: impl FnOnce(&HashMap<String, InstanceRuntime>) -> R) -> R {
        f(&self.lock().live)
    }

    /// 인스턴스를 지웠다: 그 상태를 모두 버리고, 늦게 도착한 쓰기가 되살리지 않게 한다
    pub fn forget(&self, instance_id: &str) {
        let mut inner = self.lock();
        inner.live.remove(instance_id);
        inner.gone.insert(instance_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 지운 뒤 늦게 도착한 쓰기(진행 중이던 상태 질문의 답, 겹친 정지의 쿨다운 등)는 지운 인스턴스의 상태를 되살리지 않는다
    #[test]
    fn a_late_write_does_not_bring_back_a_forgotten_instance() {
        let rt = Runtimes::default();
        rt.with("gone", |r| r.run_generation = 5);
        rt.forget("gone");
        rt.with("gone", |r| {
            r.run_generation += 1;
            r.stop_cooldown = Some(Instant::now());
        });
        assert_eq!(rt.existing("gone", |r| r.run_generation), None);
        assert!(rt.all(|map| map.is_empty()));
        // 다른 인스턴스는 그대로 만든다
        rt.with("other", |r| r.run_generation = 1);
        assert_eq!(rt.existing("other", |r| r.run_generation), Some(1));
    }
}
