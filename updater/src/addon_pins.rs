//! 사용자가 고른 모듈·익스텐션 판 (`<데이터 폴더>/addon-pins.json`)
//!
//! 키(`module-<이름>`, `ext-<id>`)마다 고정(`pinned`)과, 고른 판으로 바꾸기를 켜진 서버가 꺼질 때까지 미룬 것(`pending_version`)을 둔다.
//! 업데이트 확인(`UpdateManager::check_for_updates_hinted`)이 고정 규칙(`rule`)으로 자동 교체를 막거나 고정을 풀고,
//! 데몬의 판 고르기 API와 모듈·익스텐션 목록이 같은 파일을 읽는다

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::version::SemVer;
use crate::{relay, unmet_requires, Component, ComponentVersion, DependencyIssue, ManifestComponent, ReleaseManifest, UpdateManager};

/// 고정: `version`을 두고, 카탈로그가 `until_newer_than`보다 새 판을 낼 때까지 자동 교체가 이 키를 건드리지 않는다
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonPin {
    pub version: String,
    pub until_newer_than: String,
}

/// 한 키에 대해 사용자가 고른 것
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<AddonPin>,
    /// 고른 판으로 바꾸기가 켜진 서버를 기다린다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_version: Option<String>,
}

impl AddonChoice {
    fn is_empty(&self) -> bool {
        self.pinned.is_none() && self.pending_version.is_none()
    }

    /// 목록·상태에 싣는 것 (`installed`: 깔린 판). 깔린 판과 같은 미뤄 둔 판은 끝난 기다림이고,
    /// 고른 판도 기다리는 판도 아닌 판이 깔린 고정(고른 뒤 지우고 다시 받기)은 다음 확인이 풀 것이라 싣지 않는다 (`rule`의 Drop)
    pub fn as_installed(mut self, installed: &str) -> Self {
        self.pending_version = self.pending_version.filter(|v| v != installed);
        let pending = self.pending_version.as_deref();
        self.pinned = self.pinned.filter(|p| p.version == installed || pending == Some(p.version.as_str()));
        self
    }
}

pub fn path() -> PathBuf {
    crate::constants::resolve_data_dir().join("addon-pins.json")
}

/// 읽고 고쳐 쓰는 사이에 다른 쓰기가 끼지 않게
static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn load() -> BTreeMap<String, AddonChoice> {
    load_at(&path())
}

fn load_at(path: &Path) -> BTreeMap<String, AddonChoice> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn get(key: &str) -> AddonChoice {
    load().remove(key).unwrap_or_default()
}

/// 한 키를 고친다. 비면 그 키를 지운다
pub fn update(key: &str, change: impl FnOnce(&mut AddonChoice)) -> std::io::Result<()> {
    update_at(&path(), key, change)
}

fn update_at(path: &Path, key: &str, change: impl FnOnce(&mut AddonChoice)) -> std::io::Result<()> {
    let _write = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load_at(path);
    let mut choice = all.remove(key).unwrap_or_default();
    change(&mut choice);
    if !choice.is_empty() {
        all.insert(key.to_string(), choice);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&all).map_err(std::io::Error::other)?;
    crate::write_atomic(path, text.as_bytes())
}

/// 고정한 키를 확인이 어떻게 다루는가
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRule {
    /// 고른 판이 깔려 있다: 자동 교체가 건드리지 않는다
    Keep,
    /// 고른 판으로 바꾸기가 미뤄져 있다: 확인이 그 판을 받을 것으로 둔다
    Switch,
    /// 고정을 푼다: 고른 판이 내려졌거나, `until_newer_than`보다 새 판이 나왔거나, 고른 뒤 다른 판이 깔렸다
    Drop,
}

/// `current`: 깔린 판, `offered`: 카탈로그가 이 코어에 내는 가장 새 판, `pinned_yanked`: 고른 판이 내려졌는가,
/// `pending`: 미뤄 둔 바꾸기의 판
pub fn rule(pin: &AddonPin, current: &str, offered: &str, pinned_yanked: bool, pending: Option<&str>) -> PinRule {
    let newer = SemVer::parse(offered).zip(SemVer::parse(&pin.until_newer_than)).is_some_and(|(o, u)| o.is_newer_than(&u));
    if pinned_yanked || newer {
        PinRule::Drop
    } else if current == pin.version {
        PinRule::Keep
    } else if pending == Some(pin.version.as_str()) {
        PinRule::Switch
    } else {
        PinRule::Drop
    }
}

/// 확인이 고정 규칙으로 정한 키 (`UpdateManager::apply_pins`)
#[derive(Debug, Default)]
pub(crate) struct PinnedKeys {
    /// 자동 교체가 건드리지 않는다
    pub keep: std::collections::HashSet<String>,
    /// 고른 판(manifest에 넣어 둔 것)으로 바꾼다
    pub switch: std::collections::HashSet<String>,
}

/// 판 고르기가 상태에 넣은 것. 받거나 바꾸지 못하면 `UpdateManager::unstage_addon`으로 되돌린다
#[derive(Debug)]
pub struct StagedAddon {
    key: String,
    status: Option<ComponentVersion>,
    entry: Option<ManifestComponent>,
    had_manifest: bool,
}

impl UpdateManager {
    /// 모듈·익스텐션 판을 고르는 기준인 코어 판 (확인과 같다: 설치 기록, 없으면 이 빌드의 것)
    pub fn addon_core(&self) -> String {
        let mut versions = Self::load_installed_manifest();
        Self::fill_bundled_versions(&mut versions);
        versions.remove("saba-core").filter(|v| !v.is_empty()).unwrap_or_else(relay::core_version)
    }

    /// 한 모듈·익스텐션의 판 전부 (지금 코어·채널, 서명 확인). 업데이트 관리자를 잡지 않고 묻는다
    pub fn addon_versions(&self, key: &str) -> impl std::future::Future<Output = Result<relay::AddonVersions, relay::RelayError>> + Send + 'static {
        let (client, core, channel, key) = (self.relay_client(), self.addon_core(), self.config.channel.clone(), key.to_string());
        async move { client.fetch_addon_versions(&key, &core, &channel).await }
    }

    /// 디스크에 있는 모듈·익스텐션의 판
    pub fn installed_addon_version(&self, key: &str) -> Option<String> {
        self.disk_addon_versions().remove(key)
    }

    /// `requires` 중 코어 말고 지금 설치된 것이 못 맞추는 것 (코어는 릴레이가 이미 걸렀다)
    pub fn unmet_addon_requires(&self, key: &str, requires: &HashMap<String, String>) -> Vec<DependencyIssue> {
        let others: HashMap<String, String> = requires.iter().filter(|(k, _)| k.as_str() != "saba-core").map(|(k, v)| (k.clone(), v.clone())).collect();
        unmet_requires(key, &others, &self.dependency_versions())
    }

    /// 고른 판을 받을 것으로 둔다: 자동 교체와 같은 `download_component`·적용이 이 판을 받아 바꾼다
    pub fn stage_addon(&mut self, key: &str, entry: &ManifestComponent) -> StagedAddon {
        let component = Component::from_manifest_key(key);
        let status = self.status.components.iter().position(|c| c.component == component).map(|i| self.status.components.remove(i));
        let had_manifest = self.cached_manifest.is_some();
        let manifest = self.cached_manifest.get_or_insert_with(|| ReleaseManifest {
            version: String::new(),
            channel: String::new(),
            platform: String::new(),
            released_at: None,
            is_critical: false,
            components: HashMap::new(),
            platforms: HashMap::new(),
        });
        let previous_entry = manifest.components.insert(key.to_string(), entry.clone());
        let current = self.installed_addon_version(key).or_else(|| status.as_ref().map(|s| s.current_version.clone())).unwrap_or_default();
        self.status.components.push(ComponentVersion {
            component,
            current_version: current,
            latest_version: Some(entry.version.clone()),
            update_available: true,
            download_url: Some(entry.url.clone()),
            asset_name: Some(entry.filename.clone()),
            release_notes: None,
            published_at: None,
            downloaded: false,
            downloaded_path: None,
            staged_sha256: None,
            requires: entry.requires.clone(),
            installed: true,
            repair: false,
        });
        StagedAddon { key: key.to_string(), status, entry: previous_entry, had_manifest }
    }

    /// `stage_addon` 전으로 되돌린다 (받은 파일은 지운다)
    pub fn unstage_addon(&mut self, staged: StagedAddon) {
        let component = Component::from_manifest_key(&staged.key);
        if let Some(i) = self.status.components.iter().position(|c| c.component == component) {
            let removed = self.status.components.remove(i);
            if let Some(path) = removed.downloaded_path.filter(|_| removed.downloaded) {
                let _ = std::fs::remove_file(path);
            }
        }
        if let Some(previous) = staged.status {
            self.status.components.push(previous);
        }
        if !staged.had_manifest {
            self.cached_manifest = None;
        } else if let Some(manifest) = self.cached_manifest.as_mut() {
            match staged.entry {
                Some(entry) => {
                    manifest.components.insert(staged.key, entry);
                }
                None => {
                    manifest.components.remove(&staged.key);
                }
            }
        }
    }

    /// 고른 판이 이미 깔려 있다(고정 풀기, 깔린 판 고르기): 그 키에 받아 둔 다른 판(켜진 서버를 기다리던 바꾸기)을 걷어 내어
    /// 서버가 꺼진 뒤 자동 교체가 바꾸지 않게 한다. 받은 파일은 지운다
    pub fn settle_addon(&mut self, key: &str, entry: &ManifestComponent) {
        let component = Component::from_manifest_key(key);
        let Some(status) = self.status.components.iter_mut().find(|c| c.component == component && c.update_available) else { return };
        if let Some(path) = status.downloaded_path.take().filter(|_| status.downloaded) {
            let _ = std::fs::remove_file(path);
        }
        status.update_available = false;
        status.downloaded = false;
        status.staged_sha256 = None;
        status.latest_version = Some(entry.version.clone());
        status.requires = entry.requires.clone();
        if let Some(manifest) = self.cached_manifest.as_mut() {
            manifest.components.insert(key.to_string(), entry.clone());
        }
    }

    /// 확인 한 번에 고정 규칙을 적용한다: 고정을 풀 것은 풀고, 미뤄 둔 바꾸기는 고른 판을 다시 찾아 `manifest`에 넣는다
    pub(crate) async fn apply_pins(
        &self,
        client: &relay::RelayClient,
        manifest: &mut ReleaseManifest,
        local: &HashMap<String, String>,
        yanked: Option<&relay::AddonCatalog>,
        core: &str,
    ) -> PinnedKeys {
        let mut keys = PinnedKeys::default();
        for (key, choice) in load() {
            let Some(pin) = choice.pinned else { continue };
            // 카탈로그가 이 키를 내지 않으면(목록을 읽지 못함) 이번에는 손대지 않는다
            let Some(offered) = manifest.components.get(&key).map(|c| c.version.clone()) else { continue };
            let current = local.get(&key).map(String::as_str).unwrap_or_default();
            let pinned_yanked = yanked.is_some_and(|c| c.is_yanked(&key, &pin.version));
            let entry = match rule(&pin, current, &offered, pinned_yanked, choice.pending_version.as_deref()) {
                PinRule::Keep => {
                    keys.keep.insert(key);
                    continue;
                }
                PinRule::Drop => None,
                PinRule::Switch => match client.fetch_addon_versions(&key, core, &self.config.channel).await {
                    Ok(list) => list.find(&pin.version).cloned(),
                    Err(e) => {
                        tracing::warn!("[Updater] Could not look up the chosen version {} of {}: {}", pin.version, key, e);
                        keys.keep.insert(key);
                        continue;
                    }
                },
            };
            match entry {
                Some(entry) => {
                    manifest.components.insert(key.clone(), entry);
                    keys.switch.insert(key);
                }
                None => {
                    tracing::info!("[Updater] Releasing the pin of {} to {}", key, pin.version);
                    if let Err(e) = update(&key, |c| *c = AddonChoice::default()) {
                        tracing::warn!("[Updater] Could not release the pin of {}: {}", key, e);
                    }
                }
            }
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(version: &str, until: &str) -> AddonPin {
        AddonPin { version: version.into(), until_newer_than: until.into() }
    }

    /// 고정은 카탈로그가 바꾸기 전 판보다 새 판을 낼 때까지 자동 교체를 막는다. 고른 판이 내려지면 내림 규칙이 이긴다
    #[test]
    fn a_pin_holds_until_a_newer_version_than_the_replaced_one_or_a_yank() {
        let p = pin("0.1.12", "0.1.13");
        assert_eq!(rule(&p, "0.1.12", "0.1.13", false, None), PinRule::Keep);
        assert_eq!(rule(&p, "0.1.12", "0.1.14", false, None), PinRule::Drop);
        assert_eq!(rule(&p, "0.1.12", "0.1.13", true, None), PinRule::Drop);
        // 켜진 서버를 기다리는 바꾸기는 확인이 다시 받을 것으로 둔다
        assert_eq!(rule(&p, "0.1.13", "0.1.13", false, Some("0.1.12")), PinRule::Switch);
        // 고른 뒤 다른 판이 깔렸으면(지우고 다시 받기, 손으로 넣기) 고정을 푼다
        assert_eq!(rule(&p, "0.1.13", "0.1.13", false, None), PinRule::Drop);
    }

    /// 목록·상태에는 고른 뒤 다른 판이 깔린 고정(지우고 다시 받기)을 싣지 않는다: 다음 확인이 풀 고정이다 (`rule`의 Drop).
    /// 깔린 판과 같은 미뤄 둔 판은 끝난 기다림이다
    #[test]
    fn a_pin_left_behind_by_another_installed_version_is_not_shown() {
        let choice = |pending: Option<&str>| AddonChoice { pinned: Some(pin("0.1.12", "0.1.13")), pending_version: pending.map(Into::into) };
        assert_eq!(choice(None).as_installed("0.1.12"), choice(None));
        assert_eq!(choice(Some("0.1.12")).as_installed("0.1.13"), choice(Some("0.1.12")), "the chosen version waits for a server");
        assert_eq!(choice(Some("0.1.12")).as_installed("0.1.12"), choice(None));
        assert_eq!(choice(None).as_installed("0.1.13"), AddonChoice::default(), "reinstalled after the pin");
    }

    /// 고정과 미뤄 둔 판은 파일에 남아 다시 켜도 그대로고, 둘 다 없어진 키는 지운다
    #[test]
    fn choices_survive_a_restart_and_an_emptied_key_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("addon-pins.json");
        update_at(&file, "module-minecraft", |c| {
            c.pinned = Some(pin("0.1.12", "0.1.13"));
            c.pending_version = Some("0.1.12".into());
        })
        .unwrap();
        let loaded = load_at(&file);
        assert_eq!(loaded["module-minecraft"].pinned, Some(pin("0.1.12", "0.1.13")));
        assert_eq!(loaded["module-minecraft"].pending_version.as_deref(), Some("0.1.12"));
        update_at(&file, "module-minecraft", |c| *c = AddonChoice::default()).unwrap();
        assert!(load_at(&file).is_empty());
    }
}
