//! 설치할 릴리즈: 업데이터와 같은 경로를 쓴다 (U10)
//!
//! relay가 주는 Ed25519 서명 manifest를 검증하고, 각 컴포넌트를 R2에서 받으며
//! SHA-256을 확인한다. 어떤 컴포넌트를 설치할지도 업데이터의 규칙
//! (`Component::try_from_manifest_key`)을 따르므로 `installer`처럼 업데이터가
//! 모르는 키는 설치하지 않는다.

use saba_chan_updater_lib::download::download_verified;
use saba_chan_updater_lib::relay::{self, ManifestComponent, ReleaseManifest, RelayClient};
use saba_chan_updater_lib::{Component, DownloadProgress};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 이 플랫폼의 최신 릴리즈 manifest (서명 검증 완료)
pub async fn fetch_latest() -> Result<ReleaseManifest, String> {
    let client = RelayClient::new(relay::DEFAULT_RELAY_URL);
    let platform = relay::current_platform();
    match client
        .fetch_verified_release(relay::DEFAULT_CHANNEL, &platform, "", &relay::anon_client_id())
        .await
    {
        Ok(Some(release)) => Ok(release.manifest),
        Ok(None) => Err(format!("No release is published for {}", platform)),
        Err(e) => Err(format!("Failed to fetch release: {}", e)),
    }
}

/// manifest가 요구하는 최소 설치기 버전
pub fn required_installer_version(manifest: &ReleaseManifest) -> Option<&str> {
    manifest.components.get("installer").map(|c| c.version.as_str())
}

/// 하나라도 빠지면 앱이 돌지 않는 컴포넌트 (패키징 스크립트의 `required`와 같다)
const REQUIRED: &[&str] = &["saba-core", "gui", "updater"];

/// 설치 폴더에 풀 컴포넌트 (키 순서로 정렬). 모듈·익스텐션은 고른 것만 데이터 폴더에 따로 설치한다.
/// 필수 컴포넌트가 빠진 릴리즈는 반쪽 설치가 되므로 오류다
pub fn installable(manifest: &ReleaseManifest) -> Result<Vec<(&String, &ManifestComponent)>, String> {
    let missing: Vec<&str> = REQUIRED.iter().copied().filter(|k| !manifest.components.contains_key(*k)).collect();
    if !missing.is_empty() {
        return Err(format!("The release is missing required components: {}", missing.join(", ")));
    }
    let mut list: Vec<_> = manifest
        .components
        .iter()
        .filter(|(key, _)| !key.starts_with(relay::MODULE_PREFIX) && !key.starts_with(relay::EXTENSION_PREFIX))
        .filter(|(key, _)| Component::try_from_manifest_key(key).is_some())
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

/// 모듈·익스텐션 하나를 받아(SHA-256 검증) `target`에 새로 푼다
pub async fn install_addon(key: &str, info: &ManifestComponent, target: &Path) -> Result<(), String> {
    let temp_dir = std::env::temp_dir().join("saba-chan-installer");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    let zip = temp_dir.join(&info.filename);
    download(info, &zip, |_, _| {}).await?;
    let result = install_addon_zip(key, &zip, target);
    let _ = std::fs::remove_file(&zip);
    result
}

/// 받은 zip을 `target`에 새로 풀고, 서명된 카탈로그에서 받은 것으로 기록한다.
/// 기록이 없으면 데몬이 "unverified"로 보고 격리해 실행한다 (데몬 카탈로그 설치와 같은 기록)
fn install_addon_zip(key: &str, zip: &Path, target: &Path) -> Result<(), String> {
    (|| {
        if target.exists() {
            std::fs::remove_dir_all(target)?;
        }
        std::fs::create_dir_all(target)?;
        saba_chan_updater_lib::extract_zip(zip, target, |_| Ok(())).map(|_| ())
    })()
    .map_err(|e| e.to_string())?;
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
    on_progress: impl Fn(u64, Option<u64>) + Send + 'static,
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("saba-chan-installer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let progress = Arc::new(Mutex::new(DownloadProgress::default()));
    let ticker = {
        let progress = progress.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let (received, total) = match progress.lock() {
                    Ok(p) => (p.bytes_received, p.total_bytes),
                    Err(_) => break,
                };
                on_progress(received, (total > 0).then_some(total));
            }
        })
    };
    let result = download_verified(&http, &info.url, dest, Some(&info.sha256), info.size_bytes, Some(&progress)).await;
    ticker.abort();
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
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
        let keys: Vec<_> = installable(&manifest).unwrap().into_iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["cli", "gui", "locales", "saba-core", "updater"], "installer, the legacy bot and add-ons are skipped");
        assert_eq!(required_installer_version(&manifest), Some("0.2.0"));

        let root = Path::new("/opt/saba");
        assert_eq!(target_dir(root, &component("1", Some("."))), root);
        assert_eq!(target_dir(root, &component("1", Some("locales"))), root.join("locales"));
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
        let err = installable(&manifest).unwrap_err();
        assert!(err.contains("gui") && err.contains("updater"), "{err}");
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
        zip.start_file("lifecycle.py", zip::write::FileOptions::default()).unwrap();
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
