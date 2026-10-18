//! 적용 대기 목록(pending), 설치된 버전 기록(installed-manifest), 설치된 앱 표시 버전

use super::*;


/// 받아 둔 업데이트의 릴리즈 요약 (staging 폴더)
const PENDING_RELEASE: &str = "pending-release.json";

impl UpdateManager {
    /// 적용 대기 중인 개별 컴포넌트를 반환
    pub fn get_pending_components(&self) -> Vec<&ComponentVersion> {
        self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .collect()
    }

    /// 다운로드 완료된 컴포넌트 정보를 staging 디렉터리에 매니페스트로 저장합니다.
    /// 업데이터 --apply 모드에서 이 매니페스트를 읽어 네트워크 없이 적용할 수 있습니다.
    pub fn save_pending_manifest(&self) -> Result<()> {
        let pending: Vec<&ComponentVersion> = self.status.components.iter()
            .filter(|c| c.downloaded && c.downloaded_path.is_some())
            .collect();

        if pending.is_empty() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.staging_dir)?;
        let manifest_path = self.staging_dir.join("pending.json");
        let json = serde_json::to_string_pretty(&pending)?;
        std::fs::write(&manifest_path, json)?;
        // 적용 결과에 실을 릴리즈 버전·패치노트 (pending.json 형식은 예전 업데이터와 맞춰 그대로 둔다)
        match &self.latest_release {
            Some(release) => {
                let summary = crate::marker::ReleaseSummary::from(release);
                std::fs::write(self.staging_dir.join(PENDING_RELEASE), serde_json::to_string_pretty(&summary)?)?;
            }
            // 예전 릴리즈 요약이 이번 대상에 붙지 않게
            None => {
                let _ = std::fs::remove_file(self.staging_dir.join(PENDING_RELEASE));
            }
        }
        tracing::info!("[UpdateManager] Saved pending manifest: {} components → {:?}", pending.len(), manifest_path);
        Ok(())
    }

    /// 받아 둔 업데이트의 릴리즈 (버전·패치노트). 업데이터가 완료 마커에 싣는다
    pub fn pending_release(&self) -> Option<crate::marker::ReleaseSummary> {
        let text = std::fs::read_to_string(self.staging_dir.join(PENDING_RELEASE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// staging 디렉터리의 매니페스트를 읽어 컴포넌트 상태를 복원합니다.
    /// 네트워크 없이 apply_updates()를 실행할 수 있게 해줍니다.
    pub fn load_pending_manifest(&mut self) -> Result<usize> {
        let manifest_path = self.staging_dir.join("pending.json");
        if !manifest_path.exists() {
            anyhow::bail!("No pending manifest found at {:?}", manifest_path);
        }

        let content = std::fs::read_to_string(&manifest_path)?;
        let components: Vec<ComponentVersion> = serde_json::from_str(&content)?;

        // 실제 파일 존재 여부 재확인
        let mut valid = Vec::new();
        for mut comp in components {
            if let Some(ref path) = comp.downloaded_path {
                if std::path::Path::new(path).exists() {
                    comp.downloaded = true;
                    comp.update_available = true;
                    valid.push(comp);
                } else {
                    tracing::warn!("[UpdateManager] Staged file missing: {}", path);
                }
            }
        }

        let count = valid.len();
        self.status.components = valid;
        tracing::info!("[UpdateManager] Loaded pending manifest: {} components", count);
        Ok(count)
    }

    /// pending 매니페스트와 받아 둔 압축 파일 삭제 (적용이 끝난 뒤, 성공·실패 모두)
    pub fn clear_pending_manifest(&self) {
        let manifest_path = self.staging_dir.join("pending.json");
        if manifest_path.exists() {
            let _ = std::fs::remove_file(&manifest_path);
        }
        let targets_path = self.staging_dir.join("apply-targets.json");
        if targets_path.exists() {
            let _ = std::fs::remove_file(&targets_path);
        }
        let _ = std::fs::remove_file(self.staging_dir.join(PENDING_RELEASE));
        // pending이 없으면 받아 둔 압축 파일도 쓸모없다 (적용에 실패한 것까지). 다음 확인 때 다시 받는다
        if let Ok(entries) = std::fs::read_dir(&self.staging_dir) {
            for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
                let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if name.ends_with(".zip") || name.ends_with(".tar.gz") || name.ends_with(".part") || name.ends_with(".partial") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// 업데이터 exe가 적용해야 할 컴포넌트 키 목록을 저장합니다.
    /// 데몬이 apply_updates 응답의 needs_updater 목록을 기록하여,
    /// GUI가 CLI 인자로 일부만 전달해도 업데이터가 정확한 대상을 알 수 있도록 합니다.
    pub fn save_updater_apply_targets(&self, keys: &[String]) -> Result<()> {
        std::fs::create_dir_all(&self.staging_dir)?;
        let path = self.staging_dir.join("apply-targets.json");
        let json = serde_json::to_string_pretty(keys)?;
        std::fs::write(&path, json)?;
        tracing::info!("[UpdateManager] Saved apply targets: {:?} → {:?}", keys, path);
        Ok(())
    }

    /// 데몬이 저장한 업데이터 적용 대상 목록을 읽습니다.
    /// 파일을 읽은 후 자동 삭제합니다.
    pub fn load_updater_apply_targets(&self) -> Result<Vec<String>> {
        let path = self.staging_dir.join("apply-targets.json");
        if !path.exists() {
            anyhow::bail!("No apply-targets.json found");
        }
        let content = std::fs::read_to_string(&path)?;
        let keys: Vec<String> = serde_json::from_str(&content)?;
        std::fs::remove_file(&path).ok();
        tracing::info!("[UpdateManager] Loaded apply targets: {:?}", keys);
        Ok(keys)
    }

    // ══════════════════════════════════════════════════════
    // 로컬 설치 매니페스트 (installed-manifest.json)
    // ══════════════════════════════════════════════════════

    /// installed-manifest.json 경로 — constants 모듈에 위임
    pub(crate) fn installed_manifest_path() -> PathBuf {
        crate::constants::resolve_installed_manifest_path()
    }

    /// 로컬 설치 매니페스트 로드 — 설치된 컴포넌트 버전 맵 반환
    pub fn load_installed_manifest() -> HashMap<String, String> {
        let manifest: HashMap<String, String> =
            crate::read_json(&Self::installed_manifest_path()).ok().flatten().unwrap_or_default();
        tracing::debug!("[UpdateManager] Loaded installed manifest: {} components", manifest.len());
        manifest
    }

    /// 고쳐 쓰기 전에 읽는다. 읽을 수 없는 기록을 새 값 몇 개로 덮어쓰면 다른 컴포넌트의 버전이 사라진다
    fn read_installed_manifest(path: &Path) -> Result<HashMap<String, String>> {
        Ok(crate::read_json(path).map_err(anyhow::Error::msg)?.unwrap_or_default())
    }

    /// 로컬 설치 매니페스트 저장
    pub fn save_installed_manifest(versions: &HashMap<String, String>) -> Result<()> {
        Self::save_installed_manifest_at(&Self::installed_manifest_path(), versions)
    }

    fn save_installed_manifest_at(path: &Path, versions: &HashMap<String, String>) -> Result<()> {
        let json = serde_json::to_string_pretty(versions)?;
        crate::write_atomic(path, json.as_bytes())?;
        tracing::info!("[UpdateManager] Saved installed manifest: {} components -> {:?}", versions.len(), path);
        Ok(())
    }

    /// 특정 컴포넌트의 설치 버전을 업데이트하고 매니페스트 저장
    pub fn update_installed_version(component_key: &str, version: &str) -> Result<()> {
        Self::update_installed_version_at(&Self::installed_manifest_path(), component_key, version)
    }

    fn update_installed_version_at(path: &Path, component_key: &str, version: &str) -> Result<()> {
        let mut manifest = Self::read_installed_manifest(path)?;
        manifest.insert(component_key.to_string(), version.to_string());
        Self::save_installed_manifest_at(path, &manifest)
    }

    /// apply 성공 후 적용된 컴포넌트들의 버전을 일괄 업데이트
    pub fn update_installed_versions_batch(&self, applied_keys: &[String]) -> Result<()> {
        let mut manifest = Self::read_installed_manifest(&Self::installed_manifest_path())?;
        let mut updated = false;

        for comp in &self.status.components {
            let key = comp.component.manifest_key();
            if applied_keys.iter().any(|a| a == &comp.component.display_name() || a == &key) {
                if let Some(ref _ver) = comp.latest_version {
                    // apply 후 current_version이 이미 latest로 업데이트되어 있음
                    manifest.insert(key.clone(), comp.current_version.clone());
                    updated = true;
                    tracing::info!("[UpdateManager] Updated installed version: {} -> {}", key, comp.current_version);
                }
            }
        }

        if updated {
            Self::save_installed_manifest(&manifest)?;
        }
        Ok(())
    }

    // ══════════════════════════════════════════════════════
    // 레지스트리 버전 갱신 (Windows "설치된 앱" 표시)
    // ══════════════════════════════════════════════════════

    /// 이 설치본의 Windows "설치된 앱" DisplayVersion을 코어 버전으로 갱신한다 (다른 설치본 항목은 건드리지 않는다)
    pub fn update_registry_display_version(&self, version: &str) -> Result<()> {
        match uninstall_entry::set_display_version(&self.install_root, version) {
            Ok(true) => tracing::info!("[UpdateManager] Registry DisplayVersion updated to {}", version),
            Ok(false) => tracing::debug!("[UpdateManager] No uninstall entry for {}; DisplayVersion left alone", self.install_root.display()),
            Err(e) => tracing::warn!("[UpdateManager] DisplayVersion update failed: {}", e),
        }
        Ok(())
    }

    // ══════════════════════════════════════════════════════
    // 버전 의존성 확인
    // ══════════════════════════════════════════════════════
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 읽을 수 없는 설치 버전 기록을 새 키 하나만으로 덮어쓰면 다른 컴포넌트의 버전이 모두 사라진다
    #[test]
    fn an_unreadable_installed_manifest_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("installed-manifest.json");
        std::fs::write(&path, b"{\"gui\": \"0.1").unwrap();
        assert!(UpdateManager::update_installed_version_at(&path, "saba-core", "0.2.0").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"gui\": \"0.1");

        std::fs::remove_file(&path).unwrap();
        UpdateManager::update_installed_version_at(&path, "saba-core", "0.2.0").unwrap();
        UpdateManager::update_installed_version_at(&path, "gui", "0.2.1").unwrap();
        let saved: HashMap<String, String> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.len(), 2);
    }
}
