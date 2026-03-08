//! 업데이터 설정 관리
//!
//! 하드코딩 기본값을 사용합니다.

use anyhow::Result;
use saba_chan_updater_lib::UpdateConfig;

/// 설정 로드 — 항상 하드코딩 기본값 반환
pub fn load_updater_config() -> Result<UpdateConfig> {
    Ok(UpdateConfig::default())
}

/// GUI 모드용 설정 로더
pub fn load_config_for_gui() -> UpdateConfig {
    UpdateConfig::default()
}

/// install_root 기반 설정 로드 — 기본값 + install_root 오버라이드
pub fn load_config_from_root(root: &str) -> UpdateConfig {
    UpdateConfig {
        install_root: Some(root.to_string()),
        ..Default::default()
    }
}

/// config set — 설정 값은 내장이므로 no-op (경고 메시지 출력)
pub fn set_config_value(key: &str, value: &str) -> Result<()> {
    tracing::warn!(
        "[Config] set_config_value({}, {}) ignored — config is embedded",
        key,
        value
    );
    eprintln!("⚠ 설정이 코드에 내장되어 있어 변경이 반영되지 않습니다.");
    eprintln!("  key={}, value={}", key, value);
    Ok(())
}
