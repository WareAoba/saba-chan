pub mod supervisor;
pub mod plugin;  // Keep - used for Python lifecycle integration
pub mod python_env;  // Python 가상환경 관리 (venv 부트스트랩, pip)
pub mod protocol;  // 새로운 프로토콜 통신 모듈
pub mod ipc;
pub mod config_store;  // 중앙 설정 저장소 (타입 안전 JSON 관리)
pub mod instance;
pub mod backup;  // 월드·세이브 백업 (A2)
pub mod module_paths;
pub mod scheduler;  // 인스턴스 예약 작업 (A3)
pub mod process_monitor;
pub mod system_monitor;
pub mod utils;
pub mod extension;  // 범용 익스텐션 시스템
pub mod validator;  // 설정값 타입 검증 및 포트 충돌 검사
pub mod boot_selector;  // BIOS 스타일 부트 선택기
pub mod daemon_log;  // 데몬 로그 버퍼 + tracing Layer
pub mod instance_lock;  // 데몬 단일 인스턴스 잠금
pub mod chat;  // 채팅 명령 처리기 (릴레이 에이전트, /api/chat/command)
pub mod commands;  // 게임 명령 해석 (/api/instance/:id/command)
pub mod module_audit;  // 모듈 Python 코드 정적 검사 (M12)
pub mod audit;
pub mod events;  // 데몬 이벤트 스트림 (P4, GET /api/events)
pub mod sandbox;  // 확인되지 않은 모듈·익스텐션 실행 제한 (Windows)
pub mod actions;  // 데몬 액션 카탈로그 (P2): 모든 표면이 읽는 기능 목록
pub mod launch_args;  // 실행 인자 미리보기: 추가 인자 ↔ 설정 필드
pub mod relay_agent;  // 클라우드 릴레이 ↔ 데몬 에이전트
pub mod remote;  // 사바쨩 웹: 릴레이 웹소켓으로 받는 원격 요청과 문지기
