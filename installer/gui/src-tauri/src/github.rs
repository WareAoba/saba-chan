//! GitHub Release API — 릴리스 정보 페치 및 에셋 다운로드

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 릴리스 정보 (간략)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: Option<String>,
    pub prerelease: bool,
    pub published_at: Option<String>,
    pub assets: Vec<AssetInfo>,
}

/// 에셋 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

/// release-manifest.json 구조
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub release_version: String,
    pub components: HashMap<String, ComponentInfo>,
}

/// manifest.json 내 각 컴포넌트
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub version: String,
    #[serde(default)]
    pub asset: Option<String>,
    pub sha256: Option<String>,
    pub install_dir: Option<String>,
}

/// GitHub에서 릴리스 목록 페치 (최대 20개, 드래프트 제외)
pub async fn fetch_releases(owner: &str, repo: &str) -> Result<Vec<ReleaseInfo>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases?per_page=20",
        owner, repo
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {} : {}", status, body);
    }

    let gh_releases: Vec<GitHubReleaseRaw> = resp.json().await?;

    let releases: Vec<ReleaseInfo> = gh_releases
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| ReleaseInfo {
            tag_name: r.tag_name,
            name: r.name,
            prerelease: r.prerelease,
            published_at: r.published_at,
            assets: r
                .assets
                .into_iter()
                .map(|a| AssetInfo {
                    name: a.name,
                    size: a.size,
                    browser_download_url: a.browser_download_url,
                })
                .collect(),
        })
        .collect();

    Ok(releases)
}

/// 특정 릴리즈에서 manifest.json 에셋 다운로드 및 파싱
pub async fn fetch_manifest(
    owner: &str,
    repo: &str,
    tag: &str,
) -> Result<ReleaseManifest> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/tags/{}",
        owner, repo, tag
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch release {}: {}", tag, resp.status());
    }

    let release: GitHubReleaseRaw = resp.json().await?;

    // release-manifest.json 또는 manifest.json 에셋 찾기
    let manifest_asset = release
        .assets
        .iter()
        .find(|a| a.name == "release-manifest.json" || a.name == "manifest.json")
        .ok_or_else(|| anyhow::anyhow!("No manifest.json found in release {}", tag))?;

    // 매니페스트 다운로드
    let manifest_resp = client
        .get(&manifest_asset.browser_download_url)
        .header("Accept", "application/octet-stream")
        .send()
        .await?;

    let body = manifest_resp.text().await?;
    let manifest: ReleaseManifest = serde_json::from_str(&body)?;

    Ok(manifest)
}

/// 특정 릴리즈/에셋 파일의 다운로드 URL 가져오기
pub async fn get_asset_download_url(
    owner: &str,
    repo: &str,
    tag: &str,
    asset_name: &str,
) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/tags/{}",
        owner, repo, tag
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch release {}: {}", tag, resp.status());
    }

    let release: GitHubReleaseRaw = resp.json().await?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| anyhow::anyhow!("Asset '{}' not found in release {}", asset_name, tag))?;

    Ok(asset.browser_download_url.clone())
}

/// 리포지토리 zipball 다운로드 (모듈 설치용)
pub async fn download_repo_zipball(owner: &str, repo: &str, dest: &Path) -> Result<()> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/zipball",
        owner, repo
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to download repo zipball: {}", resp.status());
    }

    let bytes = resp.bytes().await?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &bytes)?;

    Ok(())
}

/// 에셋 다운로드 → 파일 저장
pub async fn download_asset(url: &str, dest: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Download failed: {} → {}", url, resp.status());
    }

    let bytes = resp.bytes().await?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &bytes)?;

    Ok(())
}

// ── 내부 타입 ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GitHubReleaseRaw {
    tag_name: String,
    name: Option<String>,
    prerelease: bool,
    draft: bool,
    published_at: Option<String>,
    assets: Vec<GitHubAssetRaw>,
}

#[derive(Debug, Deserialize)]
struct GitHubAssetRaw {
    name: String,
    size: u64,
    browser_download_url: String,
}
