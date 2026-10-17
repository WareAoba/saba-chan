//! 인스턴스(서버) 단위 HTTP 핸들러.
//!
//! 토픽별로 파일이 분리되어 있다. 라우팅은 [`crate::ipc::routes`] 의
//! `instance_routes()` 가 담당하며, 여기서는 핸들러 함수만 모아 재공개한다.
//!
//! 외부에서 `crate::ipc::handlers::instance::*` 경로로 사용하던 함수들은
//! 동일 경로로 그대로 사용 가능하다 (`pub use` 로 평탄화).
//!
//! # 파일 구성
//! - [`crud`]       — 목록/조회/순서변경/삭제 (`list_instances`, `reorder_instances`,
//!   `get_instance`, `delete_instance`)
//! - [`create`]     — 인스턴스 신규 생성 (`create_instance`) — 모듈 정보 조회,
//!   네이티브/컨테이너/마이그레이션 분기, 백그라운드 provisioning 호출 포함
//! - [`provision`]  — 프로비저닝 진행 상태 (`get_provision_progress`,
//!   `dismiss_provision_progress`)
//! - [`settings`]   — 설정 업데이트 (`update_instance_settings`)
//! - [`update`]     — 게임 서버 빌드 업데이트 (`check_server_update`,
//!   `apply_server_update`)

pub mod create;
pub mod crud;
pub mod provision;
pub mod settings;
pub mod update;

pub use create::create_instance;
pub use crud::{delete_instance, get_instance, list_instances, reorder_instances};
pub use provision::{dismiss_provision_progress, get_provision_progress};
pub use settings::update_instance_settings;
pub use update::{apply_server_update, check_server_update};
