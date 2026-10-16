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

/// 설치할 컴포넌트 (키 순서로 정렬)
pub fn installable(manifest: &ReleaseManifest) -> Vec<(&String, &ManifestComponent)> {
    let mut list: Vec<_> = manifest
        .components
        .iter()
        .filter(|(key, _)| Component::try_from_manifest_key(key).is_some())
        .collect();
    list.sort_by(|a, b| a.0.cmp(b.0));
    list
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
    let result = download_verified(&http, &info.url, dest, Some(&info.sha256), Some(&progress)).await;
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
        }
    }

    #[test]
    fn installs_what_the_updater_knows_and_reads_installer_requirement() {
        let mut components = HashMap::new();
        for key in ["saba-core", "gui", "cli", "updater", "locales", "installer", "discord_bot"] {
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
        let keys: Vec<_> = installable(&manifest).into_iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["cli", "gui", "locales", "saba-core", "updater"], "installer and the legacy bot are skipped");
        assert_eq!(required_installer_version(&manifest), Some("0.2.0"));

        let root = Path::new("/opt/saba");
        assert_eq!(target_dir(root, &component("1", Some("."))), root);
        assert_eq!(target_dir(root, &component("1", Some("locales"))), root.join("locales"));
    }
}
