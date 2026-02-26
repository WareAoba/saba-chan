use serde::Deserialize;
use anyhow::Context;

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Deserialize 전용 — TOML 설정 파일의 모든 필드를 매핑
pub struct GlobalConfig {
    pub ipc_socket: Option<String>,
    pub updater: Option<UpdaterConfig>,
    /// Maximum number of log lines to keep per managed process (default: 10,000)
    pub log_buffer_size: Option<usize>,
}

/// [updater] 섹션 — 자동 업데이트 설정
#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Deserialize 전용 — updater.toml 필드 매핑
pub struct UpdaterConfig {
    pub enabled: Option<bool>,
    pub check_interval_hours: Option<u32>,
    pub auto_download: Option<bool>,
    pub auto_apply: Option<bool>,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub include_prerelease: Option<bool>,
    pub install_root: Option<String>,
    pub api_base_url: Option<String>,
}

const CONFIG_PATH: &str = "config/global.toml";

impl GlobalConfig {
    pub fn load() -> anyhow::Result<Self> {
        match std::fs::read_to_string(CONFIG_PATH) {
            Ok(s) => {
                let cfg: Self = toml::from_str(&s)
                    .with_context(|| format!("Failed to parse {}", CONFIG_PATH))?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("Config file {} not found, using defaults", CONFIG_PATH);
                Ok(Self {
                    ipc_socket: None,
                    updater: None,
                    log_buffer_size: None,
                })
            }
            Err(e) => Err(e).with_context(|| format!("Failed to read {}", CONFIG_PATH)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_config_default() {
        let cfg = GlobalConfig {
            ipc_socket: None,
            updater: None,
            log_buffer_size: None,
        };
        assert!(cfg.updater.is_none());
    }
}
