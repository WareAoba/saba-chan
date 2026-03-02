//! 글로벌 설정 — 컴파일 타임 내장 기본값
//!
//! 이전에는 config/global.toml 파일에서 읽었지만,
//! 값이 사실상 고정이므로 코드에 직접 내장합니다.

/// 글로벌 설정 (하드코딩 기본값)
#[derive(Debug, Clone)]
pub struct GlobalConfig {
    pub ipc_socket: String,
    pub log_buffer_size: usize,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            ipc_socket: "./ipc.sock".to_string(),
            log_buffer_size: 10_000,
        }
    }
}

impl GlobalConfig {
    /// 기본 설정을 반환합니다 (항상 성공).
    pub fn load() -> anyhow::Result<Self> {
        Ok(Self::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_config_default() {
        let cfg = GlobalConfig::default();
        assert_eq!(cfg.ipc_socket, "./ipc.sock");
        assert_eq!(cfg.log_buffer_size, 10_000);
    }
}
