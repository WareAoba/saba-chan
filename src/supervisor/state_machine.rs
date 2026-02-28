//! Server instance state machine.
//!
//! NOTE: 이 모듈은 아직 Supervisor에 통합되지 않았습니다.
//! 각 서버 인스턴스의 상태를 이 StateMachine으로 관리할 예정이며,
//! 통합 전까지는 테스트 전용으로 유지됩니다.
#![allow(dead_code)]

use thiserror::Error;

// TODO: integrate with Supervisor — 각 서버 인스턴스의 상태를 이 StateMachine으로 관리
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Error, Debug)]
pub enum TransitionError {
    #[error("invalid transition: {0:?} -> {1:?}")]
    InvalidTransition(State, State),
}

pub struct StateMachine {
    pub state: State,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self { state: State::Stopped }
    }
}

impl StateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn can_transition(&self, to: &State) -> bool {
        matches!(
            (&self.state, to),
            (State::Stopped, State::Starting)
                | (State::Starting, State::Running)
                | (State::Starting, State::Crashed)
                | (State::Running, State::Stopping)
                | (State::Running, State::Crashed)
                | (State::Stopping, State::Stopped)
                | (State::Stopping, State::Crashed)
                | (State::Crashed, State::Stopped)
        )
    }

    pub fn transition(&mut self, to: State) -> Result<(), TransitionError> {
        if self.can_transition(&to) {
            tracing::info!("State transition: {:?} -> {:?}", self.state, to);
            self.state = to;
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition(self.state.clone(), to))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions_normal_lifecycle() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.state, State::Stopped);
        assert!(sm.transition(State::Starting).is_ok());
        assert!(sm.transition(State::Running).is_ok());
        assert!(sm.transition(State::Stopping).is_ok());
        assert!(sm.transition(State::Stopped).is_ok());
    }

    #[test]
    fn valid_transitions_crash_from_starting() {
        let mut sm = StateMachine::new();
        sm.transition(State::Starting).unwrap();
        assert!(sm.transition(State::Crashed).is_ok());
        assert_eq!(sm.state, State::Crashed);
        // 크래시 후 정지로 전이 가능
        assert!(sm.transition(State::Stopped).is_ok());
    }

    #[test]
    fn valid_transitions_crash_from_running() {
        let mut sm = StateMachine::new();
        sm.transition(State::Starting).unwrap();
        sm.transition(State::Running).unwrap();
        assert!(sm.transition(State::Crashed).is_ok());
        assert_eq!(sm.state, State::Crashed);
    }

    #[test]
    fn valid_transitions_crash_from_stopping() {
        let mut sm = StateMachine::new();
        sm.transition(State::Starting).unwrap();
        sm.transition(State::Running).unwrap();
        sm.transition(State::Stopping).unwrap();
        assert!(sm.transition(State::Crashed).is_ok());
    }

    /// 모든 불가능한 전이를 전수 검증
    #[test]
    fn exhaustive_invalid_transitions() {
        let invalid_pairs: Vec<(State, State)> = vec![
            (State::Stopped, State::Running),
            (State::Stopped, State::Stopping),
            (State::Stopped, State::Stopped),
            (State::Stopped, State::Crashed),
            (State::Starting, State::Stopped),
            (State::Starting, State::Starting),
            (State::Starting, State::Stopping),
            (State::Running, State::Starting),
            (State::Running, State::Running),
            (State::Running, State::Stopped),
            (State::Stopping, State::Starting),
            (State::Stopping, State::Running),
            (State::Stopping, State::Stopping),
            (State::Crashed, State::Starting),
            (State::Crashed, State::Running),
            (State::Crashed, State::Stopping),
            (State::Crashed, State::Crashed),
        ];

        for (from, to) in invalid_pairs {
            let mut sm = StateMachine { state: from.clone() };
            let result = sm.transition(to.clone());
            assert!(
                result.is_err(),
                "Transition {:?} -> {:?} should be invalid",
                from, to
            );
            // 상태가 변경되지 않아야 함
            assert_eq!(
                sm.state, from,
                "Failed transition must not mutate state"
            );
        }
    }

    #[test]
    fn can_transition_is_consistent_with_transition() {
        let all_states = vec![
            State::Stopped, State::Starting, State::Running,
            State::Stopping, State::Crashed,
        ];
        for from in &all_states {
            for to in &all_states {
                let sm = StateMachine { state: from.clone() };
                let can = sm.can_transition(to);
                let mut sm2 = StateMachine { state: from.clone() };
                let result = sm2.transition(to.clone());
                assert_eq!(
                    can, result.is_ok(),
                    "can_transition({:?}->{:?})={} but transition()={:?}",
                    from, to, can, result
                );
            }
        }
    }

    /// 재시작 사이클: Stopped → Starting → Running → Stopping → Stopped × 3
    #[test]
    fn restart_cycle_three_times() {
        let mut sm = StateMachine::new();
        for cycle in 0..3 {
            assert!(sm.transition(State::Starting).is_ok(), "Cycle {} start", cycle);
            assert!(sm.transition(State::Running).is_ok(), "Cycle {} run", cycle);
            assert!(sm.transition(State::Stopping).is_ok(), "Cycle {} stop", cycle);
            assert!(sm.transition(State::Stopped).is_ok(), "Cycle {} stopped", cycle);
        }
    }

    /// 크래시 복구 사이클
    #[test]
    fn crash_recovery_cycle() {
        let mut sm = StateMachine::new();
        // 정상 시작
        sm.transition(State::Starting).unwrap();
        sm.transition(State::Running).unwrap();
        // 크래시
        sm.transition(State::Crashed).unwrap();
        // 복구
        sm.transition(State::Stopped).unwrap();
        // 재시작
        sm.transition(State::Starting).unwrap();
        sm.transition(State::Running).unwrap();
        assert_eq!(sm.state, State::Running);
    }

    #[test]
    fn default_state_is_stopped() {
        let sm = StateMachine::default();
        assert_eq!(sm.state, State::Stopped);
        let sm2 = StateMachine::new();
        assert_eq!(sm2.state, State::Stopped);
    }

    #[test]
    fn transition_error_contains_states() {
        let mut sm = StateMachine::new();
        let err = sm.transition(State::Running).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Stopped"), "Error should mention source state: {}", msg);
        assert!(msg.contains("Running"), "Error should mention target state: {}", msg);
    }
}
