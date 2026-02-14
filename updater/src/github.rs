//! GitHub Release API 클라이언트
//!
//! GitHub REST API를 사용하여 릴리스 정보를 가져오고,
//! 릴리스 에셋에 포함된 manifest.json으로 컴포넌트별 버전을 결정합니다.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// GitHub Release 응답 (필요한 필드만)
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub published_at: Option<String>,
    pub html_url: String,
    pub assets: Vec<GitHubAsset>,
}

/// GitHub Release Asset
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    pub content_type: Option<String>,
}

/// manifest.json — 릴리스에 포함되는 컴포넌트별 버전 매핑
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    /// 릴리스 전체 버전
    pub release_version: String,
    /// 컴포넌트별 정보
    pub components: HashMap<String, ComponentInfo>,
}

/// manifest.json 내 각 컴포넌트 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub version: String,
    /// 릴리스 에셋 파일명 (None이면 이번 릴리스에 해당 바이너리 없음 — 버전만 기록)
    #[serde(default)]
    pub asset: Option<String>,
    /// 선택: 에셋 SHA256 해시
    pub sha256: Option<String>,
    /// 선택: 설치 디렉터리 (install_root 기준 상대경로)
    pub install_dir: Option<String>,
    /// 버전 의존성: 이 컴포넌트가 요구하는 다른 컴포넌트의 최소 버전
    /// 예: { "core_daemon": ">=0.3.0" } — GUI 0.3.0은 CoreDaemon 0.3.0 이상 필요
    #[serde(default)]
    pub requires: Option<HashMap<String, String>>,
}

/// 여러 릴리즈를 횡단 탐색하여 결정된 컴포넌트의 최적 다운로드 소스
///
/// 릴리즈마다 모든 컴포넌트가 포함되는 것은 아니므로,
/// 각 컴포넌트의 에셋이 실제로 존재하는 릴리즈를 개별적으로 찾아야 한다.
///
/// 예: 릴리즈 v0.5.0에 GUI만 포함 → 데몬 0.4.0은 v0.4.0 릴리즈에서 찾아옴
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedComponent {
    /// 소스 코드 기준 최신 버전 (최신 manifest 기준)
    pub latest_version: String,
    /// 에셋이 포함된 릴리즈 태그 (예: "v0.4.0")
    pub source_release_tag: String,
    /// 에셋 다운로드 URL
    pub download_url: String,
    /// 에셋 파일명
    pub asset_name: String,
    /// 설치 디렉터리 (install_root 기준 상대경로)
    pub install_dir: Option<String>,
    /// SHA256 해시
    pub sha256: Option<String>,
    /// 의존성 정보
    pub requires: Option<HashMap<String, String>>,
}

/// GitHub API 클라이언트
pub struct GitHubClient {
    owner: String,
    repo: String,
    http: reqwest::Client,
    /// API 베이스 URL (기본: "https://api.github.com")
    /// 로컬 mock 서버 테스트 시 "http://127.0.0.1:9876" 등으로 오버라이드
    base_url: String,
}

impl GitHubClient {
    pub fn new(owner: &str, repo: &str) -> Self {
        Self::with_base_url(owner, repo, None)
    }

    /// base_url을 오버라이드할 수 있는 생성자 (테스트/mock 서버용)
    pub fn with_base_url(owner: &str, repo: &str, base_url: Option<&str>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("saba-chan-updater/1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client for updater");

        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            http,
            base_url: base_url
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("https://api.github.com")
                .trim_end_matches('/')
                .to_string(),
        }
    }

    /// 모든 릴리스 가져오기 (최대 per_page개)
    pub async fn fetch_releases(&self, per_page: u32) -> Result<Vec<GitHubRelease>> {
        let url = format!(
            "{}/repos/{}/{}/releases?per_page={}",
            self.base_url, self.owner, self.repo, per_page
        );

        let response = self.http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error ({}): {}", status, body);
        }

        let releases: Vec<GitHubRelease> = response.json().await?;
        Ok(releases)
    }

    /// 최신 릴리스 가져오기 (프리릴리스 제외)
    pub async fn fetch_latest_release(&self) -> Result<GitHubRelease> {
        let url = format!(
            "{}/repos/{}/{}/releases/latest",
            self.base_url, self.owner, self.repo
        );

        let response = self.http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error ({}): {}", status, body);
        }

        let release: GitHubRelease = response.json().await?;
        Ok(release)
    }

    /// 릴리스 에셋에서 manifest.json 다운로드 및 파싱
    pub async fn fetch_manifest(&self, release: &GitHubRelease) -> Result<ReleaseManifest> {
        let manifest_asset = release.assets.iter()
            .find(|a| a.name == "manifest.json")
            .ok_or_else(|| anyhow::anyhow!(
                "Release '{}' does not contain manifest.json", release.tag_name
            ))?;

        let response = self.http
            .get(&manifest_asset.browser_download_url)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download manifest.json: {}", response.status());
        }

        let manifest: ReleaseManifest = response.json().await?;
        Ok(manifest)
    }

    /// 에셋 바이너리 다운로드 → Vec<u8>
    pub async fn download_asset(&self, asset: &GitHubAsset) -> Result<Vec<u8>> {
        tracing::info!("Downloading asset: {} ({} bytes)", asset.name, asset.size);

        let response = self.http
            .get(&asset.browser_download_url)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download {}: {}", asset.name, response.status());
        }

        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// 에셋을 파일로 다운로드 (스트리밍)
    pub async fn download_asset_to_file(
        &self,
        asset: &GitHubAsset,
        dest: &std::path::Path,
    ) -> Result<()> {
        tracing::info!("Downloading {} → {}", asset.name, dest.display());

        let response = self.http
            .get(&asset.browser_download_url)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download {}: {}", asset.name, response.status());
        }

        let bytes = response.bytes().await?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, &bytes)?;

        tracing::info!("Downloaded {} ({} bytes)", asset.name, bytes.len());
        Ok(())
    }

    /// 여러 릴리즈를 횡단하여 각 컴포넌트의 최적 다운로드 소스를 결정
    ///
    /// ## 알고리즘
    /// 1. 최신 릴리즈 manifest에서 모든 컴포넌트의 **최신 버전**을 확인
    /// 2. 에셋이 포함된 컴포넌트는 바로 resolved
    /// 3. 에셋이 없는 컴포넌트(이번 릴리즈에 빌드 안 됨)는 이전 릴리즈를
    ///    거슬러 올라가며 해당 버전의 에셋을 찾음
    ///
    /// ## 효율성
    /// - releases 목록은 1회 API 호출로 전부 가져옴
    /// - manifest는 필요한 릴리즈만 선별적으로 다운로드
    /// - 이미 resolved된 컴포넌트는 건너뜀
    pub async fn resolve_components_across_releases(
        &self,
        releases: &[GitHubRelease],
        include_prerelease: bool,
    ) -> Result<(ReleaseManifest, HashMap<String, ResolvedComponent>)> {

        // draft 제외, prerelease 옵션 적용, 최신순 정렬된 릴리즈 필터
        let valid_releases: Vec<&GitHubRelease> = releases.iter()
            .filter(|r| !r.draft)
            .filter(|r| include_prerelease || !r.prerelease)
            .collect();

        if valid_releases.is_empty() {
            anyhow::bail!("No suitable releases found");
        }

        // ── 1단계: 최신 릴리즈의 manifest에서 최신 버전 맵 구축 ──
        let latest_release = valid_releases[0];
        let latest_manifest = self.fetch_manifest(latest_release).await?;

        // 컴포넌트 키 → 최신 버전 (manifest 전체에서)
        let mut target_versions: HashMap<String, String> = HashMap::new();
        for (key, info) in &latest_manifest.components {
            target_versions.insert(key.clone(), info.version.clone());
        }

        // ── 2단계: 최신 릴리즈에서 에셋이 있는 컴포넌트 바로 resolve ──
        let mut resolved: HashMap<String, ResolvedComponent> = HashMap::new();

        for (key, info) in &latest_manifest.components {
            if let Some(ref asset_name) = info.asset {
                if let Some(asset) = latest_release.assets.iter().find(|a| &a.name == asset_name) {
                    resolved.insert(key.clone(), ResolvedComponent {
                        latest_version: info.version.clone(),
                        source_release_tag: latest_release.tag_name.clone(),
                        download_url: asset.browser_download_url.clone(),
                        asset_name: asset_name.clone(),
                        install_dir: info.install_dir.clone(),
                        sha256: info.sha256.clone(),
                        requires: info.requires.clone(),
                    });
                }
            }
        }

        // ── 3단계: 미해결 컴포넌트 → 이전 릴리즈 순회 ──
        let unresolved_keys: Vec<String> = target_versions.keys()
            .filter(|k| !resolved.contains_key(*k))
            .cloned()
            .collect();

        if !unresolved_keys.is_empty() {
            tracing::info!(
                "[Resolver] {} 컴포넌트가 최신 릴리즈에 에셋 없음, 이전 릴리즈 탐색: {:?}",
                unresolved_keys.len(), unresolved_keys
            );

            // 이전 릴리즈를 순회하며 에셋 탐색
            for older_release in valid_releases.iter().skip(1) {
                if unresolved_keys.iter().all(|k| resolved.contains_key(k)) {
                    break; // 모두 해결됨
                }

                // 이 릴리즈에 manifest.json이 있는지 먼저 확인 (에셋 목록으로)
                let has_manifest = older_release.assets.iter().any(|a| a.name == "manifest.json");
                if !has_manifest {
                    continue;
                }

                let older_manifest = match self.fetch_manifest(older_release).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            "[Resolver] {} manifest 로드 실패: {}",
                            older_release.tag_name, e
                        );
                        continue;
                    }
                };

                for key in &unresolved_keys {
                    if resolved.contains_key(key) {
                        continue;
                    }

                    if let Some(info) = older_manifest.components.get(key) {
                        // 필요한 버전과 동일한지 확인
                        let target_ver = &target_versions[key];
                        if &info.version == target_ver {
                            // 에셋이 있는지 확인
                            if let Some(ref asset_name) = info.asset {
                                if let Some(asset) = older_release.assets.iter().find(|a| &a.name == asset_name) {
                                    tracing::info!(
                                        "[Resolver] {} v{} → 릴리즈 {} 에서 발견",
                                        key, info.version, older_release.tag_name
                                    );
                                    resolved.insert(key.clone(), ResolvedComponent {
                                        latest_version: info.version.clone(),
                                        source_release_tag: older_release.tag_name.clone(),
                                        download_url: asset.browser_download_url.clone(),
                                        asset_name: asset_name.clone(),
                                        install_dir: info.install_dir.clone(),
                                        sha256: info.sha256.clone(),
                                        requires: info.requires.clone(),
                                    });
                                }
                            }
                        }
                        // 정확히 target_ver와 일치하지 않더라도,
                        // 이전 버전이 로컬보다 높으면 "최선의 다운로드 가능 버전"으로 사용할 수 있다.
                        // 하지만 요구사항상 "최신 버전의 바이너리를 발견하면 받아옴"이므로
                        // target_ver와 일치하는 경우만 resolve한다.
                    }
                }
            }

            // 여전히 미해결 컴포넌트 로깅
            for key in &unresolved_keys {
                if !resolved.contains_key(key) {
                    tracing::warn!(
                        "[Resolver] {} v{} → 에셋을 포함한 릴리즈를 찾지 못함",
                        key, target_versions[key]
                    );
                }
            }
        }

        Ok((latest_manifest, resolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest() {
        let json = r#"{
            "release_version": "0.2.0",
            "components": {
                "core_daemon": {
                    "version": "0.2.0",
                    "asset": "core_daemon-windows-x64.zip",
                    "sha256": "abc123",
                    "install_dir": "."
                },
                "cli": {
                    "version": "0.2.0",
                    "asset": "saba-cli-windows-x64.zip",
                    "sha256": null,
                    "install_dir": null
                },
                "gui": {
                    "version": "0.1.5"
                },
                "module-minecraft": {
                    "version": "2.1.0",
                    "asset": "module-minecraft.zip",
                    "sha256": null,
                    "install_dir": "modules/minecraft"
                }
            }
        }"#;

        let manifest: ReleaseManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.release_version, "0.2.0");
        assert_eq!(manifest.components.len(), 4);
        assert_eq!(
            manifest.components["core_daemon"].asset.as_deref(),
            Some("core_daemon-windows-x64.zip")
        );
        assert_eq!(
            manifest.components["core_daemon"].install_dir.as_deref(),
            Some(".")
        );
        // gui: asset 없음 — 이번 릴리스에 바이너리 미포함
        assert_eq!(manifest.components["gui"].asset, None);
        assert_eq!(manifest.components["gui"].version, "0.1.5");
        assert_eq!(
            manifest.components["module-minecraft"].install_dir.as_deref(),
            Some("modules/minecraft")
        );
    }

    #[test]
    fn parse_manifest_partial_release() {
        // 릴리즈에 일부 컴포넌트만 포함된 경우 (walk-back 시나리오)
        let json = r#"{
            "release_version": "0.5.0",
            "components": {
                "core_daemon": {
                    "version": "0.4.0",
                    "asset": null,
                    "sha256": null,
                    "install_dir": "."
                },
                "gui": {
                    "version": "0.5.0",
                    "asset": "saba-chan-gui-windows-x64.zip",
                    "sha256": null,
                    "install_dir": "saba-chan-gui"
                }
            }
        }"#;

        let manifest: ReleaseManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.release_version, "0.5.0");

        // core_daemon: 버전 정보는 있지만 에셋은 없음
        assert_eq!(manifest.components["core_daemon"].version, "0.4.0");
        assert_eq!(manifest.components["core_daemon"].asset, None);

        // gui: 에셋 포함
        assert_eq!(manifest.components["gui"].version, "0.5.0");
        assert_eq!(
            manifest.components["gui"].asset.as_deref(),
            Some("saba-chan-gui-windows-x64.zip")
        );
    }

    #[test]
    fn resolved_component_serialization() {
        let rc = ResolvedComponent {
            latest_version: "0.4.0".to_string(),
            source_release_tag: "v0.4.0".to_string(),
            download_url: "https://example.com/daemon.zip".to_string(),
            asset_name: "core_daemon-windows-x64.zip".to_string(),
            install_dir: Some(".".to_string()),
            sha256: None,
            requires: None,
        };

        let json = serde_json::to_string(&rc).unwrap();
        let deserialized: ResolvedComponent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.latest_version, "0.4.0");
        assert_eq!(deserialized.source_release_tag, "v0.4.0");
        assert_eq!(deserialized.asset_name, "core_daemon-windows-x64.zip");
    }
}
