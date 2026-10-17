//! 로컬 설치 버전 수집과 `installed-manifest.json` 관리

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::UpdateManager;

impl UpdateManager {
    /// 모든 컴포넌트의 현재 설치된 버전을 수집
    ///
    /// `installed-manifest.json` 을 우선하고, 없는 항목만 파일에서 감지한다.
    pub(crate) fn collect_local_versions(&self) -> HashMap<String, String> {
        let mut versions = Self::load_installed_manifest();

        versions
            .entry("saba-core".to_string())
            .or_insert_with(|| env!("CARGO_PKG_VERSION").to_string());

        if !versions.contains_key("cli") {
            if let Some(v) = read_cargo_version("saba-chan-cli") {
                versions.insert("cli".to_string(), v);
            }
        }
        if !versions.contains_key("gui") {
            if let Some(v) = read_package_json_version("saba-chan-gui") {
                versions.insert("gui".to_string(), v);
            }
        }
        if !versions.contains_key("updater") {
            if let Some(v) = read_cargo_version("updater") {
                versions.insert("updater".to_string(), v);
            }
        }

        // 모듈: modules/*/module.toml, 익스텐션: extensions/*/extension.toml
        for (dir, manifest_file, prefix) in [
            (&self.modules_dir, "module.toml", "module-"),
            (&self.extensions_dir, "extension.toml", "ext-"),
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some((name, version)) = read_toml_name_version(&path.join(manifest_file)) {
                    versions.entry(format!("{}{}", prefix, name)).or_insert(version);
                }
            }
        }

        tracing::debug!("[UpdateManager] Local versions: {:?}", versions);
        versions
    }

    // ── installed-manifest.json ──────────────────────────────

    /// 로컬 설치 매니페스트 로드 — 설치된 컴포넌트 버전 맵 반환
    pub fn load_installed_manifest() -> HashMap<String, String> {
        let path = crate::constants::resolve_installed_manifest_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<HashMap<String, String>>(&content).ok())
            .unwrap_or_default()
    }

    fn save_installed_manifest(versions: &HashMap<String, String>) -> Result<()> {
        let path = crate::constants::resolve_installed_manifest_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(versions)?)?;
        tracing::info!("[UpdateManager] Saved installed manifest: {} components -> {:?}", versions.len(), path);
        Ok(())
    }

    /// 특정 컴포넌트의 설치 버전을 기록
    pub(crate) fn update_installed_version(component_key: &str, version: &str) -> Result<()> {
        let mut manifest = Self::load_installed_manifest();
        manifest.insert(component_key.to_string(), version.to_string());
        Self::save_installed_manifest(&manifest)
    }

    // ── Windows "설치된 앱" 표시 버전 ──────────────────────

    /// Windows 레지스트리의 DisplayVersion을 코어 버전으로 갱신합니다.
    #[cfg(target_os = "windows")]
    pub(crate) fn update_registry_display_version(version: &str) -> Result<()> {
        use winreg::enums::*;
        use winreg::RegKey;

        const UNINSTALL_KEY: &str =
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan";

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey_with_flags(UNINSTALL_KEY, KEY_WRITE) {
            Ok(key) => {
                let clean_version = version.trim_start_matches('v');
                key.set_value("DisplayVersion", &clean_version)?;
                tracing::info!("[UpdateManager] Registry DisplayVersion updated to {}", clean_version);
            }
            Err(e) => {
                // 인스톨러 없이 설치된 경우 키가 없다
                tracing::debug!("[UpdateManager] Registry key not found, skipping DisplayVersion update: {}", e);
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn update_registry_display_version(_version: &str) -> Result<()> {
        Ok(())
    }
}

/// `<exe_dir>/<crate_dir>/Cargo.toml` 또는 CWD 기준 후보에서 `version` 을 읽는다.
fn read_cargo_version(crate_dir: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(crate_dir).join("Cargo.toml"));
    }
    candidates.push(PathBuf::from(crate_dir).join("Cargo.toml"));
    candidates.push(PathBuf::from("..").join(crate_dir).join("Cargo.toml"));

    candidates.iter().find_map(|p| {
        let content = std::fs::read_to_string(p).ok()?;
        toml_string_value(&content, "version")
    })
}

fn read_package_json_version(dir: &str) -> Option<String> {
    [PathBuf::from(dir).join("package.json"), PathBuf::from("..").join(dir).join("package.json")]
        .iter()
        .find_map(|p| {
            let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
            json.get("version")?.as_str().map(str::to_string)
        })
}

/// module.toml / extension.toml 에서 `(name, version)` 읽기
fn read_toml_name_version(path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    Some((toml_string_value(&content, "name")?, toml_string_value(&content, "version")?))
}

/// 첫 번째 `key = "value"` 줄의 값을 반환 (단순 라인 파서)
fn toml_string_value(content: &str, key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        if k.trim() != key {
            return None;
        }
        Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_string_value_reads_first_matching_key() {
        let content = "[package]\nname = \"mc\"\nversion_req = \"x\"\nversion = \"1.2.3\"\n";
        assert_eq!(toml_string_value(content, "name").as_deref(), Some("mc"));
        assert_eq!(toml_string_value(content, "version").as_deref(), Some("1.2.3"));
        assert_eq!(toml_string_value(content, "missing"), None);
    }
}
