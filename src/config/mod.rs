use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct GlobalConfig {
    pub ipc_socket: Option<String>,
    pub servers: Option<Vec<ServerInstance>>,
    pub updater: Option<UpdaterConfig>,
    /// Maximum number of log lines to keep per managed process (default: 10,000)
    pub log_buffer_size: Option<usize>,
}

/// [updater] 섹션 — 자동 업데이트 설정
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
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

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct ServerInstance {
    pub name: String,
    pub module: String,  // 사용할 모듈 이름 (minecraft, palworld 등)
    pub resource: Option<ResourceConfig>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct ResourceConfig {
    pub ram: Option<String>,
    pub cpu: Option<u32>,
}

impl GlobalConfig {
    pub fn load() -> anyhow::Result<Self> {
        let path = "config/global.toml";
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let cfg: Self = toml::from_str(&s)
                    .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path, e))?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("Config file {} not found, using defaults", path);
                Ok(Self {
                    ipc_socket: None,
                    servers: None,
                    updater: None,
                    log_buffer_size: None,
                })
            }
            Err(e) => Err(anyhow::anyhow!("Failed to read {}: {}", path, e)),
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
            servers: None,
            updater: None,
            log_buffer_size: None,
        };
        assert!(cfg.servers.is_none());
    }

    #[test]
    fn test_server_instance() {
        let instance = ServerInstance {
            name: "minecraft-main".to_string(),
            module: "minecraft".to_string(),
            resource: Some(ResourceConfig {
                ram: Some("8G".to_string()),
                cpu: Some(4),
            }),
        };
        assert_eq!(instance.name, "minecraft-main");
        assert_eq!(instance.module, "minecraft");
    }
}
