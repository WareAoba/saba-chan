//! 받을 수 있는 익스텐션: 릴리즈 목록, 업데이트 확인, 패키지 내려받기·풀기·설치

use super::*;

pub(super) const PACKAGE_DOWNLOAD_LIMIT: usize = 128 * 1024 * 1024;

const PACKAGE_LIMITS: crate::archive::Limits = crate::archive::Limits {
    what: "Extension package",
    entries: 10_000,
    expanded: 512 * 1024 * 1024,
};

/// 릴리즈 manifest의 `ext-<id>` 컴포넌트에서 만든 익스텐션 항목 (`from_release`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExtensionInfo {
    /// 익스텐션 ID (manifest 키에서 `ext-`를 뗀 것, 역직렬화 후 주입)
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 배포 패키지 다운로드 URL (.zip)
    #[serde(default)]
    pub download_url: String,
    /// 패키지 SHA-256 체크섬 (무결성 검증용)
    #[serde(default)]
    pub sha256: Option<String>,
    /// 최소 앱 버전 요구사항
    #[serde(default)]
    pub min_app_version: Option<String>,
    /// 이 데몬에서 설치할 수 없으면 그 이유 (`min_app_version` 미달). 데몬이 채운다 (M9)
    #[serde(default, skip_deserializing)]
    pub incompatible: Option<String>,
    /// 에셋 파일명 (예: "extension-docker.zip")
    #[serde(default)]
    pub asset: Option<String>,
    /// 설치 경로 (예: "extensions/docker")
    #[serde(default)]
    pub install_dir: Option<String>,
    /// 의존성 — 배열 또는 맵 형식 모두 허용
    #[serde(default, deserialize_with = "deserialize_dependencies")]
    pub dependencies: HashMap<String, String>,
    /// GUI 컴포넌트 포함 여부
    #[serde(default)]
    pub has_gui: bool,
    /// i18n 번역 파일 포함 여부
    #[serde(default)]
    pub has_i18n: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// 언어별 설명 (manifest `descriptions`). GUI가 앱 언어로 고른다
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    /// 아이콘이 있는가. 그림은 `/api/store/icons/ext-<id>`로 받는다
    #[serde(default)]
    pub has_icon: bool,
}

/// 업데이트 가용 정보 (로컬 버전 vs 원격 버전 비교 결과)
///
/// 업데이터의 `ComponentVersion`과 구조를 맞추어 향후 통합을 용이하게 함.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionUpdateInfo {
    pub id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub download_url: String,
    /// 새 버전이 이 데몬에서 돌지 않으면 그 이유 (M9)
    pub incompatible: Option<String>,
}

/// 익스텐션 zip을 `staging`에 풀고 맨 위 manifest.json을 읽어 `check`로 확인한다. 실패하면 `staging`을 지운다
pub(super) fn unpack_package(
    bytes: &[u8],
    staging: &std::path::Path,
    check: impl FnOnce(&ExtensionManifest) -> Result<()>,
) -> Result<ExtensionManifest> {
    let result = (|| -> Result<ExtensionManifest> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .context("Failed to read downloaded zip archive")?;
        crate::archive::check(&mut archive, &PACKAGE_LIMITS)?;
        crate::archive::extract(archive, staging, &PACKAGE_LIMITS).context("Failed to extract extension zip")?;
        let content = std::fs::read_to_string(staging.join("manifest.json"))
            .context("Package has no manifest.json at its root")?;
        let (manifest, _) = parse_manifest(&content)?;
        check(&manifest)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

impl ExtensionManager {
    /// 받을 수 있는 익스텐션 목록: 최신 릴리즈의 서명된 manifest에 든 `ext-*` 컴포넌트 (U9e).
    /// 네트워크 요청이므로 매니저 락을 놓고 부른다 (C12)
    pub async fn fetch_remote() -> Result<Vec<RemoteExtensionInfo>> {
        let catalog = saba_chan_updater_lib::relay::latest_catalog(None, &crate::ipc::updates::load_updater_config().channel)
            .await
            .context("Failed to fetch the release manifest")?;
        Ok(catalog.map(|m| Self::remote_list(&m)).unwrap_or_default())
    }

    /// 사바 스토리지 목록용: 데몬이 받아 둔 카탈로그(`store_cache`)에서. `refresh`면 새로 받는다
    pub async fn cached_remote(refresh: bool) -> Result<Vec<RemoteExtensionInfo>> {
        let channel = crate::ipc::updates::load_updater_config().channel;
        let catalog = crate::ipc::store_cache::store()
            .catalog(&channel, refresh)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch the release manifest: {}", e))?;
        Ok(catalog.map(|m| Self::remote_list(&m)).unwrap_or_default())
    }

    /// 카탈로그의 익스텐션 목록에 이 데몬에서 설치할 수 없는 이유를 채운다
    fn remote_list(manifest: &saba_chan_updater_lib::relay::ReleaseManifest) -> Vec<RemoteExtensionInfo> {
        let mut list = Self::from_release(manifest);
        for ext in &mut list {
            ext.incompatible = app_version_problem(ext.min_app_version.as_deref());
        }
        list
    }

    /// 릴리즈 manifest의 `ext-<id>` 컴포넌트를 익스텐션 목록으로 (meta에 이름·설명·의존성)
    pub fn from_release(manifest: &saba_chan_updater_lib::relay::ReleaseManifest) -> Vec<RemoteExtensionInfo> {
        manifest
            .addons(saba_chan_updater_lib::relay::EXTENSION_PREFIX)
            .into_iter()
            .filter_map(|(id, comp)| {
                let mut info = crate::ipc::store_cache::public_meta(comp.meta.as_ref());
                info.entry("name").or_insert_with(|| Value::String(id.to_string()));
                info.insert("version".into(), Value::String(comp.version.clone()));
                info.insert("download_url".into(), Value::String(comp.url.clone()));
                info.insert("sha256".into(), Value::String(comp.sha256.clone()));
                info.insert("asset".into(), Value::String(comp.filename.clone()));
                if let Some(dir) = &comp.install_dir {
                    info.insert("install_dir".into(), Value::String(dir.clone()));
                }
                match serde_json::from_value::<RemoteExtensionInfo>(Value::Object(info)) {
                    Ok(mut ext) => {
                        ext.id = id.to_string();
                        Some(ext)
                    }
                    Err(e) => {
                        tracing::warn!("Skipping extension '{}' in release manifest: {}", id, e);
                        None
                    }
                }
            })
            .collect()
    }

    /// 설치된 익스텐션 중 원격 버전보다 낮은 것의 업데이트 정보를 반환합니다.
    pub fn check_updates_against(
        &self,
        remote: &[RemoteExtensionInfo],
    ) -> Vec<ExtensionUpdateInfo> {
        let mut updates = Vec::new();
        for local in self.discovered.values() {
            if let Some(remote_ext) = remote.iter().find(|r| r.id == local.manifest.id) {
                // updater 크레이트의 SemVer를 사용하여 버전 비교
                let is_newer = match (
                    SemVer::parse(&remote_ext.version),
                    SemVer::parse(&local.manifest.version),
                ) {
                    (Some(remote_v), Some(local_v)) => remote_v.is_newer_than(&local_v),
                    // 파싱 실패 시 문자열 사전순 비교로 폴백
                    _ => remote_ext.version > local.manifest.version,
                };

                if is_newer {
                    updates.push(ExtensionUpdateInfo {
                        id: local.manifest.id.clone(),
                        installed_version: local.manifest.version.clone(),
                        latest_version: remote_ext.version.clone(),
                        download_url: remote_ext.download_url.clone(),
                        incompatible: app_version_problem(remote_ext.min_app_version.as_deref()),
                    });
                }
            }
        }
        updates
    }

    /// 익스텐션 패키지(zip)를 받는다. 락 없이 부르면 된다
    pub async fn download_package(download_url: &str) -> Result<Vec<u8>> {
        let response = reqwest::get(download_url)
            .await
            .with_context(|| format!("Failed to download extension from {}", download_url))?;
        if !response.status().is_success() {
            anyhow::bail!("Download failed: HTTP {}", response.status());
        }
        if response.content_length().is_some_and(|size| size > PACKAGE_DOWNLOAD_LIMIT as u64) {
            anyhow::bail!("Extension package exceeds the download size limit");
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.context("Failed to read download response body")? {
            if bytes.len().saturating_add(chunk.len()) > PACKAGE_DOWNLOAD_LIMIT {
                anyhow::bail!("Extension package exceeds the download size limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// 받은 zip을 검사하고 `extensions/.staging-<id>-<uuid>`에 푼다. 락 없이 부르면 된다.
    /// sha256(있으면), manifest 스키마, manifest id, min_app_version을 여기서 확인하므로
    /// 실패해도 설치된 익스텐션은 그대로다. 폴더는 호출마다 새로 만들어, 같은 id를 겹쳐 설치해도
    /// 돌려준 폴더에는 이 호출이 검사한 파일만 있다.
    pub fn stage_package(
        extensions_dir: &std::path::Path,
        ext_id: &str,
        bytes: &[u8],
        expected_sha256: Option<&str>,
    ) -> Result<PathBuf> {
        ensure_valid_extension_id(ext_id)?;
        if bytes.len() > PACKAGE_DOWNLOAD_LIMIT {
            anyhow::bail!("Extension package exceeds the download size limit");
        }
        if let Some(expected) = expected_sha256.filter(|s| !s.is_empty()) {
            use sha2::{Digest, Sha256};
            let actual = hex::encode(Sha256::digest(bytes));
            if !actual.eq_ignore_ascii_case(expected) {
                anyhow::bail!("sha256 mismatch for '{}': expected {}, got {}", ext_id, expected, actual);
            }
        }
        let staging = extensions_dir.join(format!(
            ".staging-{}-{}",
            Self::python_safe_dir_name(ext_id),
            uuid::Uuid::new_v4().simple()
        ));
        unpack_package(bytes, &staging, |manifest| {
            if manifest.id != ext_id {
                return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
            }
            require_app_version(ext_id, manifest.min_app_version.as_deref())
        })?;
        Ok(staging)
    }

    /// 검사를 마친 패키지 폴더로 익스텐션을 바꿔 끼운다 (새로 설치 또는 업데이트).
    /// 실행 중인 인스턴스가 쓰고 있으면 거부한다. 옛 폴더는 새 폴더가 자리를 잡은 뒤에 지운다.
    pub fn install_staged(
        &mut self,
        ext_id: &str,
        staged: &std::path::Path,
        running_ext_data: &[(&str, &HashMap<String, Value>)],
        official: bool,
    ) -> Result<()> {
        if let Err(e) = ensure_valid_extension_id(ext_id) {
            let _ = std::fs::remove_dir_all(staged);
            return Err(e);
        }
        let using = self.instances_using_ext(ext_id, running_ext_data);
        if !using.is_empty() {
            let _ = std::fs::remove_dir_all(staged);
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }
        let dest = self.ext_dir_on_disk(ext_id);
        if let Err(e) = Self::ensure_folder_is_free_for(&dest, ext_id) {
            let _ = std::fs::remove_dir_all(staged);
            return Err(e);
        }
        crate::utils::swap_dir(staged, &dest)?;
        Self::record_files(ext_id, &dest, official);
        self.mount(ext_id)?;
        tracing::info!("Extension '{}' installed", ext_id);
        Ok(())
    }

    /// 이 익스텐션이 의존하지만 아직 없는 익스텐션 (원격 목록에 있는 것만; `saba-core` 같은 컴포넌트는 제외)
    pub fn missing_dependencies(&self, ext_id: &str, remote: &[RemoteExtensionInfo]) -> Vec<String> {
        let Some(ext) = self.discovered.get(ext_id) else { return Vec::new() };
        let mut missing: Vec<String> = ext
            .manifest
            .dependencies
            .keys()
            .filter(|dep| !self.discovered.contains_key(*dep) && remote.iter().any(|r| &r.id == *dep))
            .cloned()
            .collect();
        missing.sort();
        missing
    }
}
