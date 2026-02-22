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
    fn valid_transitions() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.state, State::Stopped);
        assert!(sm.transition(State::Starting).is_ok());
        assert!(sm.transition(State::Running).is_ok());
        assert!(sm.transition(State::Stopping).is_ok());
        assert!(sm.transition(State::Stopped).is_ok());
    }

    #[test]
    fn invalid_transition() {
        let mut sm = StateMachine::new();
        // cannot go directly from Stopped -> Running
        let res = sm.transition(State::Running);
        assert!(res.is_err());
    }
}
