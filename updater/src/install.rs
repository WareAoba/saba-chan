//! 단일 컴포넌트 신규 설치 / 재설치

use anyhow::Result;
use std::path::PathBuf;

use crate::apply::gui_exe_name;
use crate::extract::{self, archive_entry_path};
use crate::{integrity, Component, UpdateManager};

fn exe_name(base: &str) -> String {
    if cfg!(windows) { format!("{}.exe", base) } else { base.to_string() }
}

impl UpdateManager {
    /// 컴포넌트가 설치되어 있는지 확인
    pub(crate) fn is_component_installed(&self, component: &Component) -> bool {
        if Self::validate_component(component).is_err() {
            return false;
        }
        match component {
            // 코어 데몬은 무조건 설치된 것으로 판단
            Component::CoreDaemon => true,
            Component::Cli => {
                self.install_root.join(exe_name("saba-chan-cli")).exists()
                    || PathBuf::from("saba-chan-cli").join("target").exists()
            }
            Component::Gui => {
                self.install_root.join(gui_exe_name()).exists() || self.find_gui_directory().is_ok()
            }
            Component::Module(name) => self.modules_dir.join(name).join("module.toml").exists(),
            Component::Extension(name) => {
                let ext_dir = self.resolve_ext_dir(name);
                ["extension.toml", "__init__.py", "manifest.json"]
                    .iter()
                    .any(|f| ext_dir.join(f).exists())
            }
            Component::Updater => self.install_root.join(exe_name("saba-chan-updater")).exists(),
            Component::Locales => self.install_root.join("locales").join("en").exists(),
        }
    }

    /// 특정 컴포넌트를 단일 설치한다. 이미 설치되어 있으면 실패한다.
    pub async fn install_component(&mut self, component: &Component) -> Result<String> {
        self.install_component_internal(component, None, false).await
    }

    /// 이미 설치된 컴포넌트도 덮어쓰기 재설치한다 (인스톨러 GUI 전용 경로).
    pub async fn reinstall_component_with_progress(
        &mut self,
        component: &Component,
        progress: Option<integrity::ProgressCallback>,
    ) -> Result<String> {
        self.install_component_internal(component, progress, true).await
    }

    async fn install_component_internal(
        &mut self,
        component: &Component,
        progress: Option<integrity::ProgressCallback>,
        force: bool,
    ) -> Result<String> {
        Self::validate_component(component)?;
        if self.config.relay_base_url.trim().is_empty() {
            anyhow::bail!("relay_base_url not configured");
        }

        if self.is_component_installed(component) {
            if !force {
                anyhow::bail!("{} is already installed", component.display_name());
            }
            tracing::info!("[Installer] {} already installed — overwrite 재설치 진행", component.display_name());
        }

        if self.cached_manifest.is_none() {
            self.fetch_latest_manifest_via_relay().await?;
        }

        let key = component.manifest_key();
        let rc = self.resolved_components.get(&key)
            .ok_or_else(|| anyhow::anyhow!("Component '{}' 에 대한 자산 URL이 매니페스트에 없음", key))?
            .clone();

        let staged_path = self.download_resolved(&key, &rc, progress).await?;
        let install_dir = self.resolve_install_dir(component, rc.install_dir.as_deref())?;
        let result = extract::extract_to_directory(&staged_path, &install_dir);
        std::fs::remove_file(&staged_path).ok();
        result?;

        tracing::info!("[Installer] {} installed to {}", component.display_name(), install_dir.display());
        Ok(install_dir.to_string_lossy().to_string())
    }

    /// 컴포넌트의 설치 디렉터리를 결정한다.
    /// 매니페스트의 `install_dir` 이 있으면 install_root 하위로 결합한다.
    pub(crate) fn resolve_install_dir(&self, component: &Component, manifest_dir: Option<&str>) -> Result<PathBuf> {
        Self::validate_component(component)?;
        if let Some(dir) = manifest_dir {
            // 익스텐션은 Python import 호환을 위해 하이픈을 언더스코어로 바꾼다
            if matches!(component, Component::Extension(_)) {
                return archive_entry_path(&self.install_root, &dir.replace('-', "_"));
            }
            return archive_entry_path(&self.install_root, dir);
        }

        let target = match component {
            Component::CoreDaemon | Component::Cli | Component::Gui | Component::Updater => self.install_root.clone(),
            Component::Module(name) => self.modules_dir.join(name),
            Component::Extension(name) => self.resolve_ext_dir(name),
            Component::Locales => self.install_root.join("locales"),
        };
        archive_entry_path(&target, ".")
    }
}
