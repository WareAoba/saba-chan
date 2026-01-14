use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct GlobalConfig {
    pub ipc_socket: Option<String>,
    pub servers: Option<Vec<ServerInstance>>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct ServerInstance {
    pub name: String,
    pub module: String,  // 사용할 모듈 이름 (minecraft, palworld 등)
    pub resource: Option<ResourceConfig>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct ResourceConfig {
    pub ram: Option<String>,
    pub cpu: Option<u32>,
}

impl GlobalConfig {
    pub fn load() -> anyhow::Result<Self> {
        let s = std::fs::read_to_string("config/global.toml").unwrap_or_default();
        let cfg: Self = toml::from_str(&s).unwrap_or(Self {
            ipc_socket: None,
            servers: None,
        });
        Ok(cfg)
    }

    #[allow(dead_code)]
    pub fn get_server(&self, name: &str) -> Option<ServerInstance> {
        self.servers
            .as_ref()
            .and_then(|servers| servers.iter().find(|s| s.name == name).cloned())
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
