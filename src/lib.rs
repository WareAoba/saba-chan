pub mod supervisor;
pub mod plugin;  // Keep - used for Python lifecycle integration
pub mod python_env;  // Python 가상환경 관리 (venv 부트스트랩, pip)
pub mod protocol;  // 새로운 프로토콜 통신 모듈
pub mod ipc;
pub mod config;
pub mod instance;
pub mod process_monitor;
pub mod utils;
pub mod extension;  // 범용 익스텐션 시스템
pub mod validator;  // 설정값 타입 검증 및 포트 충돌 검사
