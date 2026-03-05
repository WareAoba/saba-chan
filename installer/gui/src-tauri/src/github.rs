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

/// 리포지토리 zipball 다운로드 (모듈 설치용) — 스트리밍 + 진행률 콜백
pub async fn download_repo_zipball(owner: &str, repo: &str, dest: &Path) -> Result<()> {
    download_repo_zipball_with_progress(
        owner,
        repo,
        dest,
        None::<Box<dyn Fn(u64, Option<u64>) + Send>>,
    )
    .await
}

/// 리포지토리 zipball 다운로드 (스트리밍 + 진행률 콜백)
pub async fn download_repo_zipball_with_progress<F>(
    owner: &str,
    repo: &str,
    dest: &Path,
    on_progress: Option<F>,
) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;

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

    let total = resp.content_length();

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::File::create(dest)?;
    let mut received: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        std::io::Write::write_all(&mut file, &chunk)?;
        received += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            cb(received, total);
        }
    }

    Ok(())
}

/// 에셋 다운로드 → 파일 저장 (스트리밍 + 진행률 콜백)
pub async fn download_asset(url: &str, dest: &Path) -> Result<()> {
    download_asset_with_progress(url, dest, None::<Box<dyn Fn(u64, Option<u64>) + Send>>).await
}

/// 에셋 다운로드 → 파일 저장 (스트리밍 + 진행률 콜백)
///
/// `on_progress(received_bytes, total_bytes)` — total_bytes가 None이면 Content-Length가 없음
pub async fn download_asset_with_progress<F>(
    url: &str,
    dest: &Path,
    on_progress: Option<F>,
) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Download failed: {} → {}", url, resp.status());
    }

    let total = resp.content_length();

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::File::create(dest)?;
    let mut received: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        std::io::Write::write_all(&mut file, &chunk)?;
        received += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            cb(received, total);
        }
    }

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

// ══════════════════════════════════════════════════════
//  원격 모듈 메타데이터 페치 (module.toml 파싱)
// ══════════════════════════════════════════════════════

/// 원격 모듈 리포지토리 내 특정 모듈의 module.toml 가져오기
pub async fn fetch_module_toml(
    owner: &str,
    modules_repo: &str,
    module_name: &str,
) -> Result<String> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/main/{}/module.toml",
        owner, modules_repo, module_name
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch module.toml for {}: {}", module_name, resp.status());
    }

    Ok(resp.text().await?)
}

/// 원격 모듈 리포지토리의 목록(디렉토리) 가져오기
/// GitHub Contents API를 사용하여 루트 디렉토리의 서브디렉토리만 반환
pub async fn fetch_module_list(owner: &str, modules_repo: &str) -> Result<Vec<String>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/",
        owner, modules_repo
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch module list: {}", resp.status());
    }

    let items: Vec<GitHubContentsItem> = resp.json().await?;

    let module_dirs: Vec<String> = items
        .into_iter()
        .filter(|item| {
            item.item_type == "dir"
                && !item.name.starts_with('_')
                && !item.name.starts_with('.')
                && item.name != "docs"
        })
        .map(|item| item.name)
        .collect();

    Ok(module_dirs)
}

#[derive(Debug, Deserialize)]
struct GitHubContentsItem {
    name: String,
    #[serde(rename = "type")]
    item_type: String,
}

/// 원격 익스텐션 매니페스트 페치
pub async fn fetch_extensions_manifest(
    owner: &str,
    extensions_repo: &str,
) -> Result<serde_json::Value> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/main/manifest.json",
        owner, extensions_repo
    );

    let client = reqwest::Client::builder()
        .user_agent("saba-chan-installer/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch extensions manifest: {}", resp.status());
    }

    let manifest: serde_json::Value = resp.json().await?;
    Ok(manifest)
}

/// 인스톨러 최소 요구 버전을 릴리스 매니페스트에서 확인
/// installer 컴포넌트가 존재하면 그 version을, 없으면 None 반환
pub fn get_installer_version_from_manifest(manifest: &ReleaseManifest) -> Option<String> {
    manifest
        .components
        .get("installer")
        .map(|c| c.version.clone())
}
