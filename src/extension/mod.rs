//! 범용 익스텐션 시스템
//!
//! `extensions/` 디렉토리의 서브디렉토리를 스캔하여 `manifest.json`을 파싱하고,
//! 데몬/서버 수명주기 Hook을 Python 모듈로 디스패치합니다.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use saba_chan_updater_lib::config_file::ConfigFile;
use saba_chan_updater_lib::version::SemVer;

pub mod event_hooks;
mod discovery;
mod enable;
mod error;
mod hooks;
mod manifest;
mod remote;
mod state;

pub use error::*;
pub use hooks::*;
pub use manifest::*;
pub use remote::*;

pub use saba_chan_updater_lib::component_dirs::extension_dir;

// ═══════════════════════════════════════════════════════════════
//  API 응답용 타입
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionListItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(skip)]
    pub descriptions: HashMap<String, String>,
    pub author: String,
    pub enabled: bool,
    pub hooks: Vec<String>,
    /// npm package.json 스타일 의존성 (이름 → 버전 요구사항)
    pub dependencies: HashMap<String, String>,
    pub gui: Option<GuiManifest>,
    pub cli: Option<CliManifest>,
    /// 데이터 slot (`cli.slots`를 통일 이름으로, 값은 늘 배열). 인터페이스는 이것만 읽는다 (Q-7)
    #[serde(default)]
    pub slots: serde_json::Map<String, Value>,
    pub instance_fields: HashMap<String, FieldDef>,
    pub config_fields: HashMap<String, FieldDef>,
    /// 익스텐션 디렉토리에 icon.png가 존재하는지 여부
    #[serde(default)]
    pub has_icon: bool,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    /// 이 데몬 버전에서 켤 수 없는 이유 (min_app_version 미충족 등)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incompatible: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// 신뢰 표시: verified(서명된 카탈로그에서 받았고 그대로) · modified · unverified (M12)
    pub trust: saba_chan_updater_lib::integrity::Trust,
    /// GUI 번들이 있을 때만: 데몬이 번들을 서빙하는가 (`gui_allowed`). false면 GUI가 사용자에게 묻고
    /// `POST /api/sandbox/extension-gui/ext-<id>`로 기록한다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_allowed: Option<bool>,
    /// 사용자가 고른 판 (`GET /api/extensions`가 채운다)
    #[serde(flatten)]
    pub addon: saba_chan_updater_lib::addon_pins::AddonChoice,
}

/// GUI 번들을 서빙해도 되는가: 확인된 익스텐션이거나, 사용자가 이 PC에서 지금 파일 그대로 허락했다
/// (`sandbox.extension_gui`). 번들은 GUI 웹뷰에서 데몬 API 전체를 부를 수 있어 AppContainer 밖이다
pub fn gui_allowed(key: &str, dir: &std::path::Path) -> bool {
    saba_chan_updater_lib::integrity::trust_of(key, dir) == saba_chan_updater_lib::integrity::Trust::Verified
        || crate::sandbox::consented(crate::sandbox::Consent::ExtensionGui, key, dir)
}

// ═══════════════════════════════════════════════════════════════
//  ExtensionManager
// ═══════════════════════════════════════════════════════════════

pub struct ExtensionManager {
    extensions_dir: PathBuf,
    discovered: HashMap<String, DiscoveredExtension>,
    enabled: HashSet<String>,
    /// 익스텐션 글로벌 설정 (`config.json`의 `extensions.settings`)
    /// 구조: ext_id → { key → value }
    extension_config: HashMap<String, HashMap<String, Value>>,
    /// 활성 목록·글로벌 설정을 저장하는 설정 파일 (`extensions` 네임스페이스)
    config_file: ConfigFile,
    /// 마지막 스캔에서 로드하지 못한 익스텐션
    load_errors: Vec<ExtensionLoadError>,
}

/// 필수 익스텐션 검사 결과 (`ExtensionManager::missing_required`)
#[derive(Debug, Default, PartialEq)]
pub struct MissingExtensions {
    /// 켜지지 않았거나 디스크에 설치되지 않은 것
    pub missing: Vec<String>,
    /// `missing` 중 켜져 있지만 디스크에 설치되지 않은 것
    pub not_installed: Vec<String>,
}

impl ExtensionManager {
    /// 새 ExtensionManager 생성. `extensions_dir`은 `extensions/` 디렉토리 경로.
    pub fn new(extensions_dir: &str) -> Self {
        Self::with_config_file(extensions_dir, ConfigFile::default_location())
    }

    /// 커스텀 state 경로를 지정한 생성자 (테스트 격리용)
    #[cfg(test)]
    pub fn new_isolated(extensions_dir: &str) -> Self {
        Self::with_config_file(extensions_dir, ConfigFile::at(std::path::Path::new(extensions_dir)))
    }

    fn with_config_file(extensions_dir: &str, config_file: ConfigFile) -> Self {
        let extensions_dir = PathBuf::from(extensions_dir);

        // extensions/ 디렉토리가 없으면 생성 (최초 실행 대응)
        if !extensions_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&extensions_dir) {
                tracing::warn!("Failed to create extensions directory: {}", e);
            }
        }

        let mut mgr = Self {
            extensions_dir,
            discovered: HashMap::new(),
            enabled: HashSet::new(),
            extension_config: HashMap::new(),
            config_file,
            load_errors: Vec::new(),
        };
        mgr.load_persisted();
        mgr
    }

    // ═══════════════════════════════════════════════════════════════
    //  의존성 검증
    // ═══════════════════════════════════════════════════════════════

    /// 활성 여부 확인
    #[cfg(test)]
    pub fn is_enabled(&self, ext_id: &str) -> bool {
        self.enabled.contains(ext_id)
    }

    /// 활성화되어 있고 **디스크에 실제로 설치된** 익스텐션 ID 집합을 반환합니다.
    /// `enabled_set()`과 달리, `discover()`로 발견된 익스텐션만 포함합니다.
    /// 모듈 필수 익스텐션 검증 시 이 메서드를 사용하세요.
    pub fn installed_and_enabled_set(&self) -> HashSet<String> {
        self.enabled
            .iter()
            .filter(|ext_id| self.discovered.contains_key(ext_id.as_str()))
            .cloned()
            .collect()
    }

    /// 활성화되어 있지만 디스크에 설치되지 않은 익스텐션 ID 목록을 반환합니다.
    pub fn enabled_but_not_installed(&self) -> Vec<String> {
        self.enabled
            .iter()
            .filter(|ext_id| !self.discovered.contains_key(ext_id.as_str()))
            .cloned()
            .collect()
    }

    /// `required` 중 쓸 수 없는 익스텐션 (켜지지 않았거나 디스크에 없음). 순서는 `required`를 따른다
    pub fn missing_required(&self, required: &[String]) -> MissingExtensions {
        let ready = self.installed_and_enabled_set();
        let enabled_not_installed = self.enabled_but_not_installed();
        let missing: Vec<String> = required.iter().filter(|ext_id| !ready.contains(ext_id.as_str())).cloned().collect();
        let not_installed = missing.iter().filter(|id| enabled_not_installed.contains(id)).cloned().collect();
        MissingExtensions { missing, not_installed }
    }

    /// 발견된 전체 익스텐션 목록 (활성 상태 포함)
    pub fn list(&self) -> Vec<ExtensionListItem> {
        self.discovered
            .values()
            .map(|ext| {
                let m = &ext.manifest;
                let has_icon = ext.dir.join("icon.png").is_file();
                ExtensionListItem {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.clone(),
                    description: m.description.clone(),
                    descriptions: m.descriptions.clone(),
                    author: m.author.clone(),
                    enabled: self.enabled.contains(&m.id),
                    hooks: m.hooks.keys().cloned().collect(),
                    dependencies: m.dependencies.clone(),
                    gui: m.gui.clone(),
                    cli: m.cli.clone(),
                    slots: m.cli.as_ref().map(normalized_slots).unwrap_or_default(),
                    instance_fields: m.instance_fields.clone(),
                    config_fields: m.config_fields.clone(),
                    has_icon,
                    schema_version: m.schema_version,
                    min_app_version: m.min_app_version.clone(),
                    incompatible: Self::incompatibility(m),
                    warnings: ext.warnings.clone(),
                    trust: saba_chan_updater_lib::integrity::trust_of(&format!("ext-{}", m.id), &ext.dir),
                    gui_allowed: m
                        .gui
                        .as_ref()
                        .and_then(|g| g.bundle.as_ref())
                        .map(|_| gui_allowed(&format!("ext-{}", m.id), &ext.dir)),
                    addon: Default::default(),
                }
            })
            .collect()
    }

    /// 활성 익스텐션의 GUI 매니페스트 목록
    pub fn gui_manifests(&self) -> Vec<(&str, &GuiManifest)> {
        self.discovered
            .values()
            .filter(|ext| self.enabled.contains(&ext.manifest.id))
            .filter_map(|ext| {
                ext.manifest
                    .gui
                    .as_ref()
                    .map(|gui| (ext.manifest.id.as_str(), gui))
            })
            .collect()
    }

    /// 익스텐션 파일 절대 경로
    /// 발견된 익스텐션 (manifest + 폴더)
    pub fn discovered(&self, ext_id: &str) -> Option<&DiscoveredExtension> {
        self.discovered.get(ext_id)
    }

    pub fn extension_file_path(&self, ext_id: &str, relative: &str) -> Option<PathBuf> {
        self.discovered.get(ext_id).map(|ext| ext.dir.join(relative))
    }

    pub fn extensions_dir(&self) -> &std::path::Path {
        &self.extensions_dir
    }

    /// 익스텐션 폴더를 옮긴 뒤 새 폴더를 보게 하고 다시 읽는다 (`/api/system/paths/move`)
    pub fn set_extensions_dir(&mut self, extensions_dir: PathBuf) -> Result<Vec<String>> {
        self.extensions_dir = extensions_dir;
        self.rescan()
    }

}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests;
