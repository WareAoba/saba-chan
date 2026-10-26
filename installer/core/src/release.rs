//! 설치할 릴리즈: 업데이터와 같은 경로를 쓴다 (U10)
//!
//! relay가 주는 Ed25519 서명 manifest를 검증하고, 각 컴포넌트를 R2에서 받으며
//! SHA-256을 확인한다. 어떤 컴포넌트를 설치할지도 업데이터의 규칙
//! (`Component::try_from_manifest_key`)을 따르므로 업데이터가 모르는 키는 설치하지 않는다.
//! `installer`는 받지 않는다. 호출자가 넘긴 제거기 exe(`InstallOptions.uninstaller_exe`)를 설치 폴더에 복사한다(`registry.rs`).

use saba_chan_updater_lib::download::download_verified;
use saba_chan_updater_lib::relay::{self, ManifestComponent, ReleaseManifest, RelayClient};
use saba_chan_updater_lib::{Component, DownloadProgress};
use crate::Profile;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// relay 서명을 검증한 릴리즈 manifest. `fetch_latest`만 만들 수 있어, 설치 호출자가 검증을 건너뛸 수 없다
/// (받은 모듈·익스텐션은 공식으로 기록된다)
#[derive(Debug, Clone)]
pub struct VerifiedRelease(ReleaseManifest);

impl std::ops::Deref for VerifiedRelease {
    type Target = ReleaseManifest;
    fn deref(&self) -> &ReleaseManifest {
        &self.0
    }
}

/// 이 플랫폼의 최신 릴리즈 manifest (서명 검증 완료)
pub async fn fetch_latest() -> Result<VerifiedRelease, String> {
    let client = RelayClient::new(relay::DEFAULT_RELAY_URL);
    let platform = relay::current_platform();
    match client
        .fetch_verified_catalog(relay::DEFAULT_CHANNEL, &platform, &saba_chan_updater_lib::versions::bundled().release, &relay::anon_client_id())
        .await
    {
        Ok(Some(release)) => Ok(VerifiedRelease(release.manifest)),
        Ok(None) => Err(format!("No release is published for {}", platform)),
        Err(e) => Err(format!("Failed to fetch release: {}", e)),
    }
}

/// manifest가 요구하는 최소 설치기 버전
pub fn required_installer_version(manifest: &ReleaseManifest) -> Option<&str> {
    manifest.components.get("installer").map(|c| c.version.as_str())
}

/// 하나라도 빠지면 앱이 돌지 않는 컴포넌트: versions.toml의 `required = true` (패키징 스크립트도 이 값을 읽는다).
/// 이 구성에 들지 않는 것(`profiles`)은 뺀다
fn required_components(profile: Profile) -> Vec<String> {
    let versions: toml::Table = include_str!("../../../versions.toml").parse().unwrap_or_default();
    let Some(components) = versions.get("components").and_then(|c| c.as_table()) else {
        return Vec::new();
    };
    components
        .iter()
        .filter(|(_, c)| c.get("required").and_then(|r| r.as_bool()) == Some(true))
        .filter(|(k, _)| profile.includes(k))
        .map(|(k, _)| k.clone())
        .collect()
}

/// 설치 폴더에 풀 컴포넌트 (키 순서로 정렬). 모듈·익스텐션은 고른 것만 데이터 폴더에 따로 설치한다.
/// 필수 컴포넌트가 빠진 릴리즈는 반쪽 설치가 되므로 빠진 키 목록을 오류로 돌려준다. 이 구성에 들지 않는 것은 받지 않는다
pub fn installable(manifest: &ReleaseManifest, profile: Profile) -> Result<Vec<(&String, &ManifestComponent)>, Vec<String>> {
    let missing: Vec<String> =
        required_components(profile).into_iter().filter(|k| !manifest.components.contains_key(k)).collect();
    if !missing.is_empty() {
        return Err(missing);
    }
    let mut list: Vec<_> = manifest
        .components
        .iter()
        .filter(|(key, _)| !key.starts_with(relay::MODULE_PREFIX) && !key.starts_with(relay::EXTENSION_PREFIX))
        .filter(|(key, _)| {
            Component::try_from_manifest_key(key).is_some_and(|c| c != Component::Installer)
        })
        .filter(|(key, _)| profile.includes(key))
        .collect();
    list.sort_by(|a, b| a.0.cmp(b.0));
    Ok(list)
}

/// 설치 화면의 모듈 목록 (`module-*` 컴포넌트와 그 meta)
pub fn modules(manifest: &ReleaseManifest) -> Vec<crate::ModuleInfo> {
    manifest
        .addons(relay::MODULE_PREFIX)
        .into_iter()
        .map(|(name, comp)| {
            let meta = comp.meta.as_ref();
            let text = |key: &str| meta.and_then(|m| m.get(key)).and_then(|v| v.as_str()).map(str::to_string);
            crate::ModuleInfo {
                id: name.to_string(),
                name: text("display_name").or_else(|| text("game_name")).unwrap_or_else(|| name.to_string()),
                description: text("description").unwrap_or_default(),
                icon: text("icon").unwrap_or_default(),
                required_extensions: meta
                    .and_then(|m| m.get("requires_extensions"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// 받은 zip을 두는 임시 폴더. 설치기 창과 CLI가 함께 돌아도 서로의 파일을 지우지 않게 프로세스마다 따로 쓴다
pub(crate) fn work_dir() -> PathBuf {
    std::env::temp_dir().join(format!("{}{}", WORK_DIR_PREFIX, std::process::id()))
}

/// `work_dir`의 이름 앞부분 (제거기가 남은 폴더를 찾는다)
pub(crate) const WORK_DIR_PREFIX: &str = "saba-chan-installer-";

/// 모듈·익스텐션 하나를 받아(SHA-256 검증) `target`에 새로 푼다
/// 크레이트 안에서만 쓴다: `info`는 `VerifiedRelease`에서 꺼낸 값이어야 공식으로 기록해도 된다
pub(crate) async fn install_addon(key: &str, info: &ManifestComponent, target: &Path) -> Result<(), String> {
    let temp_dir = work_dir();
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    let zip = temp_dir.join(&info.filename);
    download_with_limit(info, &zip, Some(128 * 1024 * 1024), |_, _| {}).await?;
    let result = install_addon_zip(key, &zip, target);
    let _ = std::fs::remove_file(&zip);
    result
}

/// 받은 zip을 `target`에 새로 풀고, 서명된 카탈로그에서 받은 것으로 기록한다.
/// 기록이 없으면 데몬이 "unverified"로 보고 격리해 실행한다 (데몬 카탈로그 설치와 같은 기록).
/// 옆 폴더에 다 푼 뒤에 바꿔 넣는다: 풀다가 실패하면(깨진 zip, 디스크 부족) 있던 폴더를 그대로 둔다
fn install_addon_zip(key: &str, zip: &Path, target: &Path) -> Result<(), String> {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = target.with_file_name(format!(".{}.new-{}", name, std::process::id()));
    let old = target.with_file_name(format!(".{}.old-{}", name, std::process::id()));
    let extracted = (|| {
        if staged.exists() {
            std::fs::remove_dir_all(&staged)?;
        }
        std::fs::create_dir_all(&staged)?;
        saba_chan_updater_lib::extract_zip(zip, &staged, |_| Ok(())).map(|_| ())
    })();
    if let Err(e) = extracted {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e.to_string());
    }
    let swapped = (|| {
        if target.exists() {
            std::fs::rename(target, &old)?;
        }
        if let Err(e) = std::fs::rename(&staged, target) {
            // 새 폴더를 못 넣었으면 있던 폴더를 되돌린다
            if old.exists() {
                let _ = std::fs::rename(&old, target);
            }
            return Err(e);
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staged);
    swapped.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    saba_chan_updater_lib::integrity::record_dir(key, target)
        .and_then(|_| saba_chan_updater_lib::integrity::set_official(key, true))
        .map_err(|e| format!("Could not record installed files: {}", e))
}

/// 컴포넌트가 풀릴 폴더
pub fn target_dir(install_root: &Path, info: &ManifestComponent) -> PathBuf {
    match info.install_dir.as_deref() {
        Some(d) if !d.is_empty() && d != "." => install_root.join(d),
        _ => install_root.to_path_buf(),
    }
}

/// 컴포넌트 하나를 `dest`로 받는다. 해시가 다르면 파일을 지우고 실패한다.
/// 받는 동안 `on_progress(received, total)`를 0.2초마다 부른다.
pub async fn download(
    info: &ManifestComponent,
    dest: &Path,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    download_with_limit(info, dest, None, on_progress).await
}

async fn download_with_limit(
    info: &ManifestComponent,
    dest: &Path,
    limit: Option<u64>,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("saba-chan-installer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let progress = Arc::new(Mutex::new(DownloadProgress::default()));
    // 같은 작업 안에서 받기와 함께 돈다: 진행 콜백이 호출한 쪽의 것을 빌려 쓸 수 있다
    let ticker = async {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let Ok((received, total)) = progress.lock().map(|p| (p.bytes_received, p.total_bytes)) else {
                // 진행을 못 읽어도 받기는 끝까지 간다
                return std::future::pending::<()>().await;
            };
            on_progress(received, (total > 0).then_some(total));
        }
    };
    let max_bytes = match (info.size_bytes, limit) {
        (Some(size), Some(limit)) => Some(size.min(limit)),
        (Some(size), None) => Some(size),
        (None, limit) => limit,
    };
    let result = tokio::select! {
        r = download_verified(&http, &info.url, dest, Some(&info.sha256), max_bytes, Some(&progress)) => r,
        _ = ticker => unreachable!("the ticker never finishes"),
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    // L15(c): 빌드에 담긴 versions.toml을 못 읽으면 필수 목록이 비어 반쪽 릴리스를 막지 못한다
    #[test]
    fn the_bundled_versions_file_names_the_required_components() {
        let mut required = super::required_components(super::Profile::Desktop);
        required.sort();
        assert_eq!(required, ["gui", "saba-core", "updater"]);
        // 헤드리스 설치는 GUI 없이 돈다
        let mut required = super::required_components(super::Profile::Headless);
        required.sort();
        assert_eq!(required, ["saba-core", "updater"]);
    }

    use super::*;
    use std::collections::HashMap;

    fn component(version: &str, install_dir: Option<&str>) -> ManifestComponent {
        ManifestComponent {
            version: version.into(),
            filename: "x.zip".into(),
            size_bytes: None,
            sha256: "0".repeat(64),
            url: "https://example.invalid/x.zip".into(),
            install_dir: install_dir.map(Into::into),
            requires: None,
            meta: None,
        }
    }

    #[test]
    fn installs_what_the_updater_knows_and_reads_installer_requirement() {
        let mut components = HashMap::new();
        for key in ["saba-core", "gui", "cli", "updater", "locales", "installer", "discord_bot", "module-palworld", "ext-steamcmd"] {
            components.insert(key.to_string(), component("0.2.0", None));
        }
        let manifest = ReleaseManifest {
            version: "0.2.0".into(),
            channel: "stable".into(),
            platform: "windows-x64".into(),
            released_at: None,
            is_critical: false,
            components,
        };
        let keys: Vec<_> = installable(&manifest, Profile::Desktop).unwrap().into_iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["cli", "gui", "locales", "saba-core", "updater"], "installer, the legacy bot and add-ons are skipped");
        let keys: Vec<_> = installable(&manifest, Profile::Headless).unwrap().into_iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["cli", "locales", "saba-core", "updater"], "a headless install skips the GUI");
        assert_eq!(required_installer_version(&manifest), Some("0.2.0"));

        let root = Path::new("/opt/saba");
        assert_eq!(target_dir(root, &component("1", Some("."))), root);
        assert_eq!(target_dir(root, &component("1", Some("locales"))), root.join("locales"));
    }

    /// 풀다가 실패하면(깨진 zip, 디스크 부족) 이미 설치돼 있던 모듈·익스텐션을 그대로 둔다
    #[test]
    fn a_failed_extraction_keeps_the_installed_addon() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("modules").join("palworld");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("lifecycle.py"), b"old").unwrap();
        let zip = tmp.path().join("broken.zip");
        std::fs::write(&zip, b"not a zip").unwrap();

        assert!(install_addon_zip("module-palworld", &zip, &target).is_err());
        assert_eq!(std::fs::read(target.join("lifecycle.py")).unwrap(), b"old");
        let left: Vec<_> = std::fs::read_dir(target.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, ["palworld"], "no staging folder is left behind");
    }

    /// 데몬·GUI·업데이터가 빠진 릴리즈는 반쪽 설치가 된다. 설치하지 않고 실패한다
    #[test]
    fn a_release_missing_a_required_component_is_refused() {
        let mut components = HashMap::new();
        for key in ["saba-core", "cli", "locales"] {
            components.insert(key.to_string(), component("0.2.0", None));
        }
        let manifest = ReleaseManifest {
            version: "0.2.0".into(),
            channel: "stable".into(),
            platform: "windows-x64".into(),
            released_at: None,
            is_critical: false,
            components,
        };
        assert_eq!(installable(&manifest, Profile::Desktop).unwrap_err(), ["gui", "updater"]);
        assert_eq!(installable(&manifest, Profile::Headless).unwrap_err(), ["updater"]);
    }

    /// 깨끗한 설치로 받은 공식 모듈이 데몬에서 "verified"로 보여야 격리·검사 없이 돈다
    #[test]
    fn an_installed_addon_is_recorded_as_official() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("saba-installer-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("SABA_DATA_DIR", &data);

        let zip_path = root.join("module.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zip.start_file("lifecycle.py", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"print('hi')").unwrap();
        zip.finish().unwrap();

        let target = data.join("modules").join("palworld");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("old.py"), "stale").unwrap();
        install_addon_zip("module-palworld", &zip_path, &target).unwrap();

        assert!(!target.join("old.py").exists(), "the folder is replaced, not merged");
        assert_eq!(
            saba_chan_updater_lib::integrity::trust_of("module-palworld", &target),
            saba_chan_updater_lib::integrity::Trust::Verified
        );
        std::env::remove_var("SABA_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }
}
