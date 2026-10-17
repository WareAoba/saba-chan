//! 다운로드된 업데이트 적용
//!
//! 적용 직전에 스테이징 파일의 SHA-256 을 `pending.json` 의 해시로 다시 검증한다.
//! - 바이너리(코어·CLI·업데이터·포터블 GUI): 기존 파일을 `.old` 로 옮긴 뒤 교체, 실패 시 복원
//! - 모듈·익스텐션: 디렉터리 백업 → 교체, 실패 시 백업에서 복원

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::extract::{self, copy_dir_recursive};
use crate::staging::required_hash;
use crate::{integrity, ApplyComponentResult, Component, ProcessChecker, UpdateManager};

/// 여러 컴포넌트 적용 결과 (`apply_pending`)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyBatchOutcome {
    /// 적용에 성공한 manifest 키
    pub applied: Vec<String>,
    /// 실패한 manifest 키와 오류 메시지. 이 항목은 `pending.json` 에 남는다.
    pub failed: Vec<(String, String)>,
}

impl ApplyBatchOutcome {
    pub fn failed_keys(&self) -> Vec<String> {
        self.failed.iter().map(|(key, _)| key.clone()).collect()
    }
}

impl UpdateManager {
    /// `keys` 를 순서대로 적용한다. 개별 실패는 기록하고 다음 컴포넌트를 계속 적용한다.
    ///
    /// 끝나면 남은 대기 항목(실패 포함)으로 `pending.json` 을 다시 쓰고,
    /// 남은 항목이 없으면 스테이징 상태 파일을 모두 지운다.
    /// `on_progress(index, key)` 는 각 컴포넌트 적용 직전에 호출된다.
    pub async fn apply_pending(
        &mut self,
        keys: &[String],
        mut on_progress: impl FnMut(usize, &str),
    ) -> ApplyBatchOutcome {
        let mut outcome = ApplyBatchOutcome::default();
        for (index, key) in keys.iter().enumerate() {
            on_progress(index, key);
            match self.apply_single_component(&Component::from_manifest_key(key)).await {
                Ok(_) => {
                    tracing::info!("[Apply] {} ✓", key);
                    outcome.applied.push(key.clone());
                }
                Err(e) => {
                    tracing::error!("[Apply] {} failed: {:#}", key, e);
                    outcome.failed.push((key.clone(), format!("{:#}", e)));
                }
            }
        }

        if let Err(e) = self.save_pending_manifest() {
            tracing::warn!("[Apply] Failed to persist remaining pending components: {}", e);
        }
        if self.get_pending_components().is_empty() {
            self.clear_pending_manifest();
        }
        outcome
    }

    /// 다운로드된 단일 컴포넌트를 적용한다.
    pub async fn apply_single_component(&mut self, component: &Component) -> Result<ApplyComponentResult> {
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded && c.update_available)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Component {:?} is not ready for apply", component))?;
        let staged_path = comp.downloaded_path.as_deref()
            .ok_or_else(|| anyhow::anyhow!("No staged file for {:?}", component))?;
        let staged = Path::new(staged_path);
        let key = component.manifest_key();

        // 다운로드 이후 스테이징 파일이 바뀌지 않았는지 재검증
        let expected = required_hash(&key, comp.sha256.as_deref())?;
        integrity::verify_file(staged, &expected)
            .with_context(|| format!("staged file verification failed for {}", key))?;

        let (message, restart_needed) = match component {
            Component::Module(name) => {
                self.apply_module_update(name, staged)?;
                (format!("Module '{}' updated", name), false)
            }
            Component::Extension(name) => {
                self.apply_extension_update(name, staged)?;
                (format!("Extension '{}' updated", name), false)
            }
            Component::CoreDaemon => {
                self.apply_binary_update("saba-core", staged).await?;
                ("Saba-Core updated (restart required)".to_string(), true)
            }
            Component::Cli => {
                self.apply_binary_update("saba-cli", staged).await?;
                ("CLI updated".to_string(), false)
            }
            Component::Updater => {
                // Windows 는 실행 중인 exe 의 rename 을 허용한다
                self.apply_binary_update("saba-chan-updater", staged).await?;
                ("Updater updated".to_string(), false)
            }
            Component::Gui => {
                self.apply_gui_update(staged)?;
                ("GUI updated".to_string(), true)
            }
            Component::Locales => {
                extract::extract_to_directory(staged, &self.install_root.join("locales"))?;
                std::fs::remove_file(staged).ok();
                ("Locales updated".to_string(), false)
            }
        };

        self.mark_component_applied(component);

        if let Some(applied) = self.status.components.iter().find(|c| &c.component == component) {
            if let Err(e) = Self::update_installed_version(&key, &applied.current_version) {
                tracing::warn!("[UpdateManager] Failed to update installed manifest for {}: {}", key, e);
            }
            if matches!(component, Component::CoreDaemon) {
                if let Err(e) = Self::update_registry_display_version(&applied.current_version) {
                    tracing::warn!("[UpdateManager] Failed to update registry DisplayVersion: {}", e);
                }
            }
        }

        Ok(ApplyComponentResult {
            component: key,
            success: true,
            message,
            stopped_processes: Vec::new(),
            restart_needed,
        })
    }

    /// 컴포넌트의 적용 완료 상태를 표시
    fn mark_component_applied(&mut self, component: &Component) {
        for comp in self.status.components.iter_mut().filter(|c| &c.component == component) {
            comp.update_available = false;
            comp.downloaded = false;
            comp.downloaded_path = None;
            if let Some(ref latest) = comp.latest_version {
                comp.current_version = latest.clone();
            }
        }
    }

    pub(crate) fn apply_module_update(&self, module_name: &str, staged: &Path) -> Result<()> {
        Self::validate_component_name(module_name)?;
        let target_dir = extract::archive_entry_path(&self.modules_dir, module_name)?;
        let backup_dir = self.staging_dir.join(format!("{}_backup", module_name));
        apply_directory_update(staged, &target_dir, &backup_dir)?;
        tracing::info!("[Updater] Module '{}' updated successfully", module_name);
        Ok(())
    }

    fn apply_extension_update(&self, ext_name: &str, staged: &Path) -> Result<()> {
        Self::validate_component_name(ext_name)?;
        let target_dir = self.resolve_ext_dir(ext_name);
        let backup_dir = self.staging_dir.join(format!("{}_ext_backup", ext_name));
        apply_directory_update(staged, &target_dir, &backup_dir)?;
        tracing::info!("[Updater] Extension '{}' updated successfully", ext_name);
        Ok(())
    }

    /// install_root 의 바이너리를 교체한다. 실패하면 기존 파일을 복원한다.
    pub(crate) async fn apply_binary_update(&self, binary_name: &str, staged: &Path) -> Result<()> {
        let process = match binary_name {
            "saba-core" => Some("saba-core"),
            "saba-cli" => Some("saba-chan-cli"),
            _ => None,
        };
        if let Some(process) = process {
            let process = if cfg!(windows) { format!("{}.exe", process) } else { process.to_string() };
            if ProcessChecker::is_running(&process) {
                tracing::info!("[Updater] Waiting for {} to exit before applying update...", process);
                if !ProcessChecker::wait_for_exit(&process, 15).await {
                    tracing::warn!("[Updater] {} did not exit within timeout, attempting update anyway", process);
                }
            }
        }

        tracing::info!("[Updater] Applying binary update: {} in {}", binary_name, self.install_root.display());
        extract::extract_archive(staged, &self.install_root, true)?;
        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] Binary '{}' updated", binary_name);

        if binary_name == "saba-core" {
            self.install_shared_modules();
        }
        Ok(())
    }

    /// core zip 에 포함된 공유 Python 모듈(i18n.py, daemon_rcon.py)을
    /// `%APPDATA%/saba-chan/modules/` 로 옮긴다.
    fn install_shared_modules(&self) {
        const SHARED_MODULES: &[&str] = &["i18n.py", "daemon_rcon.py"];

        let modules_dir = crate::constants::resolve_data_dir().join("modules");
        let _ = std::fs::create_dir_all(&modules_dir);

        for name in SHARED_MODULES {
            let src = self.install_root.join(name);
            if !src.exists() {
                continue;
            }
            match std::fs::copy(&src, modules_dir.join(name)) {
                Ok(_) => {
                    tracing::info!("[Updater] Shared module installed: {}", name);
                    let _ = std::fs::remove_file(&src);
                }
                Err(e) => tracing::warn!("[Updater] Failed to install shared module {}: {}", name, e),
            }
        }
    }

    fn apply_gui_update(&self, staged: &Path) -> Result<()> {
        // 포터블 exe 모드: install_root/saba-chan-gui(.exe)
        if self.install_root.join(gui_exe_name()).exists() {
            tracing::info!("[Updater] GUI portable exe detected in {}", self.install_root.display());
            extract::extract_archive(staged, &self.install_root, true)?;
            std::fs::remove_file(staged).ok();
            tracing::info!("[Updater] GUI (portable exe) updated");
            return Ok(());
        }

        // 디렉터리 모드 (압축 해제된 Electron / 개발 환경)
        let gui_dir = self.find_gui_directory()?;
        let extract_dir = [gui_dir.join("build"), gui_dir.join("resources").join("app").join("build")]
            .into_iter()
            .find(|dir| dir.exists());
        let extract_dir = match extract_dir {
            Some(dir) => dir,
            None if gui_dir.join("src").exists() && gui_dir.join("package.json").exists() => {
                anyhow::bail!("GUI directory appears to be a source tree ({}). Refusing to overwrite.", gui_dir.display());
            }
            None => gui_dir,
        };
        tracing::info!("[Updater] Applying GUI update to dir: {}", extract_dir.display());
        extract::extract_archive(staged, &extract_dir, false)?;
        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] GUI updated successfully");
        Ok(())
    }

    pub(crate) fn find_gui_directory(&self) -> Result<PathBuf> {
        if self.install_root.join(gui_exe_name()).exists() {
            return Ok(self.install_root.clone());
        }
        if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
            if dir.join(gui_exe_name()).exists() {
                return Ok(dir);
            }
        }
        // 개발 환경 — CWD 기준 소스 구조
        for p in ["saba-chan-gui", "../saba-chan-gui"] {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
        }
        anyhow::bail!("GUI directory not found (checked: install_root={}, exe_dir, cwd)", self.install_root.display())
    }
}

pub(crate) fn gui_exe_name() -> &'static str {
    if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" }
}

/// 디렉터리 단위 업데이트: 백업 → 비우기 → 압축 해제. 실패하면 백업에서 복원한다.
///
/// 성공하면 백업과 스테이징 파일을 지운다. 복원에 실패하면 백업을 남기고 오류에 경로를 적는다.
pub(crate) fn apply_directory_update(staged: &Path, target_dir: &Path, backup_dir: &Path) -> Result<()> {
    if !extract::is_archive(staged) {
        anyhow::bail!("Unsupported package format (expected .zip or .tar.gz): {}", staged.display());
    }

    let had_previous = target_dir.exists();
    if had_previous {
        if backup_dir.exists() {
            std::fs::remove_dir_all(backup_dir)?;
        }
        copy_dir_recursive(target_dir, backup_dir)?;
    }

    let result = (|| -> Result<()> {
        if had_previous {
            extract::clean_dir(target_dir)?;
        }
        extract::extract_archive(staged, target_dir, false)
    })();

    if let Err(error) = result {
        if let Err(restore_error) = restore_directory(target_dir, backup_dir, had_previous) {
            return Err(error.context(format!(
                "rollback failed ({:#}); backup kept at {}",
                restore_error,
                backup_dir.display()
            )));
        }
        if had_previous {
            std::fs::remove_dir_all(backup_dir).ok();
        }
        return Err(error);
    }

    if had_previous {
        std::fs::remove_dir_all(backup_dir).ok();
    }
    std::fs::remove_file(staged).ok();
    Ok(())
}

/// 실패한 디렉터리 업데이트를 되돌린다. 대상 삭제가 실패해도 백업 복사를 시도한다.
fn restore_directory(target_dir: &Path, backup_dir: &Path, had_previous: bool) -> Result<()> {
    let removed = if target_dir.exists() { std::fs::remove_dir_all(target_dir) } else { Ok(()) };
    if had_previous {
        copy_dir_recursive(backup_dir, target_dir)?;
    }
    removed.with_context(|| format!("remove partially updated {}", target_dir.display()))?;
    Ok(())
}
