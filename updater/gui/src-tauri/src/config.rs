//! 업데이터 설정 파일 관리
//!
//! `updater.toml` 또는 `config/global.toml` [updater] 섹션에서 설정을 로드합니다.

use anyhow::Result;
use saba_chan_updater_lib::UpdateConfig;
use std::path::PathBuf;

/// 설정 파일 경로 결정
pub fn config_file_path() -> PathBuf {
    // 1. 실행 파일 옆 config/updater.toml
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cfg = dir.join("config").join("updater.toml");
            if cfg.exists() {
                return cfg;
            }
            let cfg = dir.join("updater.toml");
            if cfg.exists() {
                return cfg;
            }
        }
    }

    // 2. CWD의 config/updater.toml
    let cwd_cfg = PathBuf::from("config").join("updater.toml");
    if cwd_cfg.exists() {
        return cwd_cfg;
    }

    // 3. 기본: config/updater.toml (생성용)
    PathBuf::from("config").join("updater.toml")
}

/// TOML 파일에서 설정 로드 (없으면 기본값)
pub fn load_updater_config() -> Result<UpdateConfig> {
    let path = config_file_path();

    if !path.exists() {
        // global.toml에서 [updater] 섹션 읽기 시도
        if let Some(gp) = find_global_toml() {
            if let Ok(content) = std::fs::read_to_string(&gp) {
                if let Ok(parsed) = content.parse::<toml::Value>() {
                    if let Some(updater) = parsed.get("updater") {
                        return Ok(parse_config(updater));
                    }
                }
            }
        }
        return Ok(UpdateConfig::default());
    }

    let content = std::fs::read_to_string(&path)?;
    let parsed: toml::Value = content.parse()?;
    Ok(parse_config(&parsed))
}

/// GUI 모드에서도 사용하는 설정 로더 (기존 load_config 대체)
pub fn load_config_for_gui() -> UpdateConfig {
    load_updater_config().unwrap_or_default()
}

/// 지정된 install_root 경로의 config/updater.toml 또는 config/global.toml에서 설정 로드
/// --apply --install-root <path> 모드에서 사용 (portable exe가 임시 폴더에서 실행될 때)
pub fn load_config_from_root(root: &str) -> UpdateConfig {
    let root_path = PathBuf::from(root);

    // 1. root/config/updater.toml
    let updater_toml = root_path.join("config").join("updater.toml");
    if updater_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&updater_toml) {
            if let Ok(parsed) = content.parse::<toml::Value>() {
                tracing::info!("[Config] Loaded from install_root: {:?}", updater_toml);
                return parse_config(&parsed);
            }
        }
    }

    // 2. root/config/global.toml [updater] 섹션
    let global_toml = root_path.join("config").join("global.toml");
    if global_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&global_toml) {
            if let Ok(parsed) = content.parse::<toml::Value>() {
                if let Some(updater) = parsed.get("updater") {
                    tracing::info!("[Config] Loaded [updater] from install_root global.toml: {:?}", global_toml);
                    return parse_config(updater);
                }
            }
        }
    }

    tracing::warn!("[Config] No config found in install_root: {}", root);
    // fallback: 기본 경로에서 시도
    load_config_for_gui()
}

pub fn parse_config(val: &toml::Value) -> UpdateConfig {
    let mut cfg = UpdateConfig::default();
    if let Some(v) = val.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = val.get("github_owner").and_then(|v| v.as_str()) {
        cfg.github_owner = v.to_string();
    }
    if let Some(v) = val.get("github_repo").and_then(|v| v.as_str()) {
        cfg.github_repo = v.to_string();
    }
    if let Some(v) = val.get("check_interval_hours").and_then(|v| v.as_integer()) {
        cfg.check_interval_hours = v as u32;
    }
    if let Some(v) = val.get("auto_download").and_then(|v| v.as_bool()) {
        cfg.auto_download = v;
    }
    if let Some(v) = val.get("auto_apply").and_then(|v| v.as_bool()) {
        cfg.auto_apply = v;
    }
    if let Some(v) = val.get("include_prerelease").and_then(|v| v.as_bool()) {
        cfg.include_prerelease = v;
    }
    if let Some(v) = val.get("install_root").and_then(|v| v.as_str()) {
        cfg.install_root = Some(v.to_string());
    }
    if let Some(v) = val.get("api_base_url").and_then(|v| v.as_str()) {
        cfg.api_base_url = Some(v.to_string());
    }
    cfg
}

/// config set <key> <value>
pub fn set_config_value(key: &str, value: &str) -> Result<()> {
    let path = config_file_path();

    let mut table: toml::value::Table = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        content.parse::<toml::Value>()?
            .as_table()
            .cloned()
            .unwrap_or_default()
    } else {
        toml::value::Table::new()
    };

    let toml_val: toml::Value = match key {
        "enabled" | "auto_download" | "auto_apply" | "include_prerelease" => {
            toml::Value::Boolean(value.parse::<bool>().map_err(|_| {
                anyhow::anyhow!("Invalid boolean value: '{}' (use true/false)", value)
            })?)
        }
        "check_interval_hours" => {
            toml::Value::Integer(value.parse::<i64>().map_err(|_| {
                anyhow::anyhow!("Invalid integer value: '{}'", value)
            })?)
        }
        "github_owner" | "github_repo" | "install_root" | "api_base_url" => {
            toml::Value::String(value.to_string())
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: '{}'\nAvailable: enabled, github_owner, github_repo, \
                check_interval_hours, auto_download, auto_apply, include_prerelease, install_root, api_base_url",
                key
            );
        }
    };

    table.insert(key.to_string(), toml_val);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(&toml::Value::Table(table))?;
    std::fs::write(&path, content)?;

    Ok(())
}

fn find_global_toml() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("config").join("global.toml");
            if p.exists() {
                return Some(p);
            }
        }
    }
    let p = PathBuf::from("config").join("global.toml");
    if p.exists() {
        return Some(p);
    }
    None
}
