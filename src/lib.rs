pub mod supervisor;
pub mod plugin;  // Keep - used for Python lifecycle integration
pub mod python_env;  // Python 가상환경 관리 (venv 부트스트랩, pip)
pub mod protocol;  // 새로운 프로토콜 통신 모듈
pub mod ipc;
pub mod config_store;  // 중앙 설정 저장소 (타입 안전 JSON 관리)
pub mod instance;
pub mod process_monitor;
pub mod utils;
pub mod extension;  // 범용 익스텐션 시스템
pub mod validator;  // 설정값 타입 검증 및 포트 충돌 검사
pub mod boot_selector;  // BIOS 스타일 부트 선택기
pub mod daemon_log;  // 데몬 로그 버퍼 + tracing Layer
pub mod runtime_preflight; // 데몬 기동 시 자체 의존성(node/python/bot npm) 검사 + 자동 복구
pub mod cloud_agent;  // 릴레이 서버 long-poll/heartbeat 에이전트 (클라우드 모드)
