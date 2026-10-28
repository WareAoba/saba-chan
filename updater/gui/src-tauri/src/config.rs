//! 업데이터 설정 관리
//!
//! 하드코딩 기본값을 사용합니다.

use saba_chan_updater_lib::UpdateConfig;

/// GUI 모드용 설정 로더
pub fn load_config_for_gui() -> UpdateConfig {
    UpdateConfig::default()
}
