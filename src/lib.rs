pub mod supervisor;
pub mod plugin;  // Keep - used for Python lifecycle integration
pub mod python_env;  // Python 가상환경 관리 (venv 부트스트랩, pip)
pub mod protocol;  // 새로운 프로토콜 통신 모듈
pub mod ipc;
pub mod config;
pub mod instance;
pub mod process_monitor;
pub mod utils;
pub mod docker;  // Docker Compose 통합
