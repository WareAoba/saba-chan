//! 익스텐션 켜기·끄기·지우기와 의존성

use super::*;

impl ExtensionManager {
    /// 익스텐션 활성화 — 통합 dependencies 맵에서 의존성 전부 검증.
    pub fn enable(&mut self, ext_id: &str) -> Result<()> {
        let installed = HashMap::from([(crate::module_loader::CORE_COMPONENT.to_string(), APP_VERSION.to_string())]);
        self.enable_with_versions(ext_id, &installed)
    }

    /// 이 데몬에서 켤 수 없는 이유 (`min_app_version`이 더 높음)
    pub(super) fn incompatibility(m: &ExtensionManifest) -> Option<String> {
        app_version_problem(m.min_app_version.as_deref())
    }

    /// 컴포넌트 버전 정보를 함께 받아 dependencies를 검증하면서 활성화.
    /// `installed_versions`: 컴포넌트 키 → 설치된 버전 (예: "saba-core" → "0.3.0")
    ///
    /// dependencies 맵의 각 키를 먼저 discovered 익스텐션에서 찾고,
    /// 있으면 익스텐션 의존성(마운트+활성화+버전)으로, 없으면 컴포넌트 의존성(설치 버전)으로 처리.
    pub fn enable_with_versions(
        &mut self,
        ext_id: &str,
        installed_versions: &HashMap<String, String>,
    ) -> Result<()> {
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_found(ext_id).into());
        }

        require_app_version(ext_id, self.discovered[ext_id].manifest.min_app_version.as_deref())?;

        let deps = self.discovered[ext_id].manifest.dependencies.clone();
        for (dep_key, version_req) in &deps {
            if let Some(dep_ext) = self.discovered.get(dep_key) {
                // ── 익스텐션 의존성: discovered에 있으면 ext dep ──
                if !self.enabled.contains(dep_key) {
                    return Err(
                        ExtensionError::dependency_not_enabled(ext_id, dep_key).into()
                    );
                }
                // 버전 검증 ("*"면 스킵)
                if version_req != "*" && !saba_chan_updater_lib::version::satisfies(&dep_ext.manifest.version, version_req) {
                    return Err(
                        ExtensionError::component_version_unsatisfied(
                            ext_id, dep_key, version_req,
                            Some(&dep_ext.manifest.version),
                        ).into()
                    );
                }
            } else {
                // ── 비-익스텐션 컴포넌트 의존성 ──
                if version_req == "*" {
                    // 이름만 선언 → discovered에 없으면 마운트 안 된 익스텐션으로 간주
                    return Err(
                        ExtensionError::dependency_missing(ext_id, dep_key).into()
                    );
                }
                let installed = installed_versions.get(dep_key);
                let satisfied = installed.is_some_and(|v| saba_chan_updater_lib::version::satisfies(v, version_req));
                if !satisfied {
                    return Err(
                        ExtensionError::component_version_unsatisfied(
                            ext_id, dep_key, version_req,
                            installed.map(|s| s.as_str()),
                        ).into()
                    );
                }
            }
        }

        self.enabled.insert(ext_id.to_string());
        if let Err(e) = self.save_state() {
            self.enabled.remove(ext_id);
            return Err(e);
        }
        tracing::info!("Extension enabled: {}", ext_id);
        Ok(())
    }

    /// 모듈을 설치할 때 그 모듈이 요구하는 익스텐션을 켠다 (의존 익스텐션부터).
    /// `previously_required`(설치 전 판이 요구하던 것)는 사용자가 일부러 껐을 수 있으므로 다시 켜지 않는다.
    /// 다만 `newly_installed`(이번에 받은 것)는 켠다. 돌려주는 값: (새로 켠 것, 켜지 못한 것과 이유)
    pub fn enable_for_module(
        &mut self,
        required: &[String],
        previously_required: &[String],
        newly_installed: &[String],
    ) -> (Vec<String>, Vec<(String, anyhow::Error)>) {
        let mut enabled = Vec::new();
        let mut failed = Vec::new();
        for id in required {
            if previously_required.contains(id) && !newly_installed.contains(id) {
                continue;
            }
            if let Err(e) = self.enable_with_dependencies(id, &mut enabled, &mut Vec::new()) {
                failed.push((id.clone(), e));
            }
        }
        (enabled, failed)
    }

    /// 의존 익스텐션을 먼저 켜고 이것을 켠다. 새로 켠 것을 `enabled`에 차례대로 더한다
    fn enable_with_dependencies(&mut self, ext_id: &str, enabled: &mut Vec<String>, visiting: &mut Vec<String>) -> Result<()> {
        if self.enabled.contains(ext_id) || visiting.iter().any(|v| v == ext_id) {
            return Ok(());
        }
        visiting.push(ext_id.to_string());
        let deps: Vec<String> = self
            .discovered
            .get(ext_id)
            .map(|e| e.manifest.dependencies.keys().filter(|d| self.discovered.contains_key(*d)).cloned().collect())
            .unwrap_or_default();
        for dep in deps {
            self.enable_with_dependencies(&dep, enabled, visiting)?;
        }
        self.enable(ext_id)?;
        enabled.push(ext_id.to_string());
        Ok(())
    }

    /// 익스텐션 비활성화 — 역의존성 검사 + 인스턴스 사용 여부 검사
    pub fn disable(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        if !self.enabled.contains(ext_id) {
            // 이미 비활성 → no-op
            return Ok(());
        }

        self.ensure_unused(ext_id, active_ext_data)?;

        self.enabled.remove(ext_id);
        if let Err(e) = self.save_state() {
            self.enabled.insert(ext_id.to_string());
            return Err(e);
        }
        tracing::info!("Extension disabled: {}", ext_id);
        // 꺼진 익스텐션은 더 불리지 않는다
        crate::plugin::stop_workers(Some(&self.ext_dir_on_disk(ext_id)));
        Ok(())
    }

    /// 익스텐션 제거 — 비활성화 후 디렉토리 삭제
    pub fn remove(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        // 발견된 익스텐션인지 확인
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_found(ext_id).into());
        }
        // 꺼져 있어도 이 익스텐션을 쓰는 인스턴스가 있으면 지우지 않는다
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }
        // 활성화 상태면 먼저 비활성화 (의존성 검사 포함)
        if self.enabled.contains(ext_id) {
            self.disable(ext_id, active_ext_data)?;
        }
        // 발견 목록에서 제거
        self.discovered.remove(ext_id);
        // 디렉토리 삭제
        let ext_path = self.ext_dir_on_disk(ext_id);
        crate::plugin::stop_workers(Some(&ext_path));
        if ext_path.exists() {
            std::fs::remove_dir_all(&ext_path)
                .with_context(|| format!("Failed to remove extension directory: {}", ext_path.display()))?;
        }
        // extensionConfig에서 해당 익스텐션의 설정 제거
        // 폴더는 이미 지웠다: 남은 기록을 못 지워도 제거는 끝났다 (다시 발견되지 않는다)
        if self.extension_config.remove(ext_id).is_some() {
            if let Err(e) = self.save_extension_config() {
                tracing::warn!("Removed extension {} but could not save its config removal: {}", ext_id, e);
            }
        }
        if let Err(e) = self.save_state() {
            tracing::warn!("Removed extension {} but could not save the extension state: {}", ext_id, e);
        }
        tracing::info!("Extension removed: {}", ext_id);
        Ok(())
    }
}
