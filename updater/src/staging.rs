//! 다운로드와 스테이징 상태 파일 관리
//!
//! - `pending.json` — 다운로드 완료 후 적용 대기 중인 컴포넌트 (데몬 → 업데이터 exe)
//! - `apply-targets.json` — 업데이터 exe 가 적용할 컴포넌트 키 목록 (데몬 → 업데이터 exe)

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{integrity, Component, ComponentVersion, ResolvedComponent, UpdateManager};

const PENDING_FILE: &str = "pending.json";
const APPLY_TARGETS_FILE: &str = "apply-targets.json";

/// 매니페스트 SHA-256 을 필수로 파싱한다. 해시가 없으면 다운로드하지 않는다 (fail closed).
pub(crate) fn required_hash(key: &str, sha256: Option<&str>) -> Result<integrity::FileHash> {
    let hex = sha256
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("매니페스트에 '{}' 의 sha256 이 없음 — 검증할 수 없어 거부", key))?;
    Ok(integrity::FileHash::parse_hex(hex)?)
}

pub(crate) fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("saba-chan-updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| anyhow::anyhow!("HTTP 클라이언트 생성 실패: {}", e))
}

impl UpdateManager {
    /// 업데이트 가능한 모든 컴포넌트를 staging 디렉터리로 다운로드한다.
    /// 개별 실패는 로그만 남기고 다음 컴포넌트를 계속 시도한다.
    pub async fn download_available_updates(&mut self) -> Result<Vec<String>> {
        std::fs::create_dir_all(&self.staging_dir)?;

        let to_download: Vec<Component> = self.status.components.iter()
            .filter(|c| c.update_available && !c.downloaded)
            .filter(|c| self.resolved_components.contains_key(&c.component.manifest_key()))
            .map(|c| c.component.clone())
            .collect();

        let mut downloaded = Vec::new();
        for component in to_download {
            match self.download_component(&component).await {
                Ok(asset_name) => downloaded.push(asset_name),
                Err(e) => tracing::warn!("[Updater] 컴포넌트 {} 다운로드 실패: {}", component.display_name(), e),
            }
        }
        Ok(downloaded)
    }

    /// 특정 컴포넌트만 다운로드한다.
    ///
    /// 매니페스트의 SHA-256 이 필수다. 해시가 없거나 불일치하면 실패하고 파일은 삭제된다.
    pub async fn download_component(&mut self, component: &Component) -> Result<String> {
        Self::validate_component(component)?;

        let comp_status = self.status.components.iter()
            .find(|c| &c.component == component)
            .ok_or_else(|| anyhow::anyhow!("Component {:?} not found in status", component))?;
        if !comp_status.update_available {
            anyhow::bail!("No update available for {}", component.display_name());
        }

        let key = component.manifest_key();
        let rc = self.resolved_components.get(&key)
            .ok_or_else(|| anyhow::anyhow!("No resolved download source for {}", component.display_name()))?
            .clone();

        {
            let mut prog = self.download_progress.lock().unwrap();
            *prog = crate::DownloadProgress { component: Some(key.clone()), active: true, ..Default::default() };
        }
        let progress_handle = Arc::clone(&self.download_progress);
        let progress_cb: integrity::ProgressCallback = Box::new(move |downloaded, total| {
            if let Ok(mut prog) = progress_handle.lock() {
                prog.bytes_received = downloaded;
                if prog.total_bytes == 0 && total > 0 {
                    prog.total_bytes = total;
                }
            }
        });

        let result = self.download_resolved(&key, &rc, Some(progress_cb)).await;
        self.download_progress.lock().unwrap().active = false;

        let comp = self.status.components.iter_mut().find(|c| &c.component == component);
        match result {
            Ok(dest) => {
                if let Some(comp) = comp {
                    comp.downloaded = true;
                    comp.downloaded_path = Some(dest.to_string_lossy().to_string());
                    comp.sha256 = rc.sha256.clone();
                }
                Ok(rc.asset_name)
            }
            Err(e) => {
                if let Some(comp) = comp {
                    comp.downloaded = false;
                    comp.downloaded_path = None;
                }
                Err(e)
            }
        }
    }

    /// 해석된 소스를 staging 디렉터리로 받아 SHA-256 을 검증한다. 반환값은 저장 경로.
    pub(crate) async fn download_resolved(
        &self,
        key: &str,
        rc: &ResolvedComponent,
        progress: Option<integrity::ProgressCallback>,
    ) -> Result<PathBuf> {
        let expected = required_hash(key, rc.sha256.as_deref())?;
        Self::validate_component_name(&rc.asset_name)?;
        std::fs::create_dir_all(&self.staging_dir)?;
        let dest = crate::extract::archive_entry_path(&self.staging_dir, &rc.asset_name)?;

        tracing::info!("[Updater] Downloading {} v{} from release {}", key, rc.latest_version, rc.source_release_tag);
        integrity::download_verified(&http_client()?, &rc.download_url, &dest, Some(&expected), progress)
            .await
            .map_err(|e| anyhow::anyhow!("다운로드/검증 실패 ({}): {}", rc.asset_name, e))?;
        Ok(dest)
    }

    // ── pending.json / apply-targets.json ───────────────────

    /// 적용 대기 중인 개별 컴포넌트를 반환
    pub fn get_pending_components(&self) -> Vec<&ComponentVersion> {
        self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .collect()
    }

    /// 다운로드 완료된 컴포넌트를 `pending.json` 에 저장한다.
    /// 대기 중인 컴포넌트가 없으면 오래된 `pending.json` 을 삭제한다.
    pub fn save_pending_manifest(&self) -> Result<()> {
        let pending: Vec<&ComponentVersion> = self.status.components.iter()
            .filter(|c| c.downloaded && c.downloaded_path.is_some())
            .collect();
        let path = self.staging_dir.join(PENDING_FILE);

        if pending.is_empty() {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            return Ok(());
        }

        std::fs::create_dir_all(&self.staging_dir)?;
        std::fs::write(&path, serde_json::to_string_pretty(&pending)?)?;
        tracing::info!("[UpdateManager] Saved pending manifest: {} components → {:?}", pending.len(), path);
        Ok(())
    }

    /// `pending.json` 을 읽어 컴포넌트 상태를 복원한다 (네트워크 불필요).
    /// 스테이징 파일이 사라진 항목은 제외한다.
    pub fn load_pending_manifest(&mut self) -> Result<usize> {
        let path = self.staging_dir.join(PENDING_FILE);
        if !path.exists() {
            anyhow::bail!("No pending manifest found at {:?}", path);
        }

        let components: Vec<ComponentVersion> = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let valid: Vec<ComponentVersion> = components
            .into_iter()
            .filter_map(|mut comp| {
                let staged = comp.downloaded_path.as_deref()?;
                if !std::path::Path::new(staged).exists() {
                    tracing::warn!("[UpdateManager] Staged file missing: {}", staged);
                    return None;
                }
                comp.downloaded = true;
                comp.update_available = true;
                Some(comp)
            })
            .collect();

        let count = valid.len();
        self.status.components = valid;
        tracing::info!("[UpdateManager] Loaded pending manifest: {} components", count);
        Ok(count)
    }

    /// `pending.json` 과 `apply-targets.json` 삭제 (모두 적용된 뒤)
    pub fn clear_pending_manifest(&self) {
        for name in [PENDING_FILE, APPLY_TARGETS_FILE] {
            let _ = std::fs::remove_file(self.staging_dir.join(name));
        }
    }

    /// 업데이터 exe 가 적용해야 할 컴포넌트 키 목록을 저장한다.
    pub fn save_updater_apply_targets(&self, keys: &[String]) -> Result<()> {
        std::fs::create_dir_all(&self.staging_dir)?;
        let path = self.staging_dir.join(APPLY_TARGETS_FILE);
        std::fs::write(&path, serde_json::to_string_pretty(keys)?)?;
        tracing::info!("[UpdateManager] Saved apply targets: {:?} → {:?}", keys, path);
        Ok(())
    }

    /// 데몬이 저장한 업데이터 적용 대상 목록을 읽고 파일을 삭제한다.
    pub fn load_updater_apply_targets(&self) -> Result<Vec<String>> {
        let path = self.staging_dir.join(APPLY_TARGETS_FILE);
        if !path.exists() {
            anyhow::bail!("No apply-targets.json found");
        }
        let keys: Vec<String> = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        std::fs::remove_file(&path).ok();
        tracing::info!("[UpdateManager] Loaded apply targets: {:?}", keys);
        Ok(keys)
    }
}
