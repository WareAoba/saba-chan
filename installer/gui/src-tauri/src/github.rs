//! GitHub에서 모듈·익스텐션을 받는다 (U9e로 R2에 옮기기 전까지).
//! 코어 컴포넌트는 `release.rs`가 relay+R2로 받는다.

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

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
