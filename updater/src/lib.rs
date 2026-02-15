//! �E�E���E���� �E��E� + �E��E�����E�/�E��E��E�E EGitHub �E��E��E�E�E��E�E
//!
//! �E�어 �E��E�, CLI, GUI, �E��E�E�E� �E��E� �E�����E�트�E� �E��E�����E�다.
//! �E��E�����E��E��E�E�E�E�작���여 �E�촁E�E��E�를 �E�리����E�다.
//!
//! ## �E�작 �E��E�E
//! - **�E��E��E��E��E�E*: �E��E�된 �E�E��(�E��E� 3�E�각E�E��E�E�E�동 �E����, �E�그 �E�력
//! - **GUI �E��E�E*: IPC �E�드����E�����E� ������ GUI�E��E �E�E���E����/�E��E���� ���인/�E����E
//! - **CLI �E�력**: `update` �E�브커�E��E�에�E�E�E�E��E�E�력
//!
//! ## �E�E�����첁E(v2)
//! �E��E��E��E��E�와 ����E��E��E��E�E�E�업�E�E�E�E�����E�E�E��:
//! - **�E��E��E��E��E�E(worker.rs)**: �E�E��E�E����, �E��E��E�드  EGUI/CLI �E���� �E�E�� �E�E��
//! - **����E��E��E��E�E(foreground.rs)**: �E�E��  EGUI/CLI �E�E��E���E�E�E���E�����E� ���일 �E�적E
//! - **���E(queue.rs)**: �E��E�E�E��E��E�드 �E�청�E�E�E�차 �E�리, �E��E�도 �E��E�E
//! - **�E�러 (error.rs)**: �E�����E�크 �E�김, ����E�E���E�E�E� �E��E�E�E��E�
//! - **IPC (ipc.rs)**: GUI/CLI ↁE�E�E���E���� ����E�
//!
//! ## �E��E�����E� �E��E�E
//! �E�����E�트�E� �E�컬�E�E�E�E���E� �E��E��E�롁E���정하�E�,
//! �E��E��E�드/�E��E�를 �E��E�����E�다. �E�렉터�E��E�E`install_root` �E��E��E�E���E�.
//!
//! ## GitHub �E��E��E�E�E��E�
//! �E�E�E��E��E�에 `manifest.json` ���일�E� �������E�어�E� ����E�다:
//! ```json
//! {
//!   "release_version": "0.2.0",
//!   "components": {
//!     "saba-core": { "version": "0.2.0", "asset": "saba-core-windows-x64.zip", "install_dir": "." },
//!     "cli":         { "version": "0.2.0", "asset": "saba-cli-windows-x64.zip", "install_dir": "." },
//!     "gui":         { "version": "0.2.0", "asset": "saba-chan-gui-windows-x64.zip", "install_dir": "saba-chan-gui" },
//!     "module-minecraft": { "version": "2.1.0", "asset": "module-minecraft.zip", "install_dir": "modules/minecraft" },
//!     "module-palworld":  { "version": "1.0.1", "asset": "module-palworld.zip", "install_dir": "modules/palworld" }
//!   }
//! }
//! ```

// ══════════════════════════════════════════════════════╁E
// �E��E�E
// ══════════════════════════════════════════════════════╁E

pub mod error;
pub mod foreground;
pub mod github;
pub mod ipc;
pub mod queue;
pub mod scheduler;
pub mod version;
pub mod worker;

#[cfg(test)]
mod tests;

// Re-exports for convenience
pub use error::{UpdaterError, RecoveryStrategy, NetworkChecker, ErrorContext};
pub use foreground::{ForegroundApplier, SelfUpdater, ProcessChecker, ApplyPhase, ApplyProgress, ApplyPreparation};
pub use github::{ResolvedComponent, ReleaseManifest, ComponentInfo, GitHubRelease};
pub use ipc::{DaemonIpcClient, StateFile, UpdateCompletionMarker, UpdateSummary, UpdaterCommand, UpdaterResponse};
pub use queue::{DownloadQueue, DownloadRequest, DownloadResult, QueueStatus};
pub use worker::{BackgroundWorker, BackgroundTask, WorkerEvent, WorkerStatus, AutoCheckScheduler};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use github::{GitHubClient};
use version::SemVer;

// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・
// 繝ｻ・�E�繝ｻ繝ｻ螟仰繝ｻ繝ｻ
// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・

/// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ�E�繝ｻ・�E�・代・繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ・�E�繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    CoreDaemon,
    Cli,
    Gui,
    Module(String),
    DiscordBot,
}

impl Component {
    /// manifest.json繝ｻ繝ｻ蠁E��・�E�繝ｻ繝ｻ繝ｻ�E�蠍ｹ繝ｻ
    pub fn manifest_key(&self) -> String {
        match self {
            Component::CoreDaemon => "saba-core".to_string(),
            Component::Cli => "cli".to_string(),
            Component::Gui => "gui".to_string(),
            Component::Module(name) => format!("module-{}", name),
            Component::DiscordBot => "discord_bot".to_string(),
        }
    }

    /// manifest 蠁E��・�E�繝ｻ蟾昴・ Component 繝ｻ・�E�繝ｻ繝ｻ
    pub fn from_manifest_key(key: &str) -> Self {
        match key {
            "saba-core" => Component::CoreDaemon,
            "cli" => Component::Cli,
            "gui" => Component::Gui,
            "discord_bot" => Component::DiscordBot,
            k if k.starts_with("module-") => {
                Component::Module(k.strip_prefix("module-").unwrap().to_string())
            }
            other => Component::Module(other.to_string()),
        }
    }

    /// 繝ｻ・�E�繝ｻ隴�E�謫�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ繝ｻ螢�E�譬�E� 繝ｻ・�E�繝ｻ繝ｻ
    pub fn display_name(&self) -> String {
        match self {
            Component::CoreDaemon => "Core Daemon".to_string(),
            Component::Cli => "CLI".to_string(),
            Component::Gui => "GUI".to_string(),
            Component::Module(name) => format!("Module: {}", name),
            Component::DiscordBot => "Discord Bot".to_string(),
        }
    }
}

/// 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentVersion {
    pub component: Component,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    /// 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E繝ｻ繝ｻ・�E�繝ｻ繝ｻ・�E�繝ｻ�E�
    pub downloaded: bool,
    /// 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉悶・繝ｻ逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ繝ｻ(繝ｻ繝ｻ蝨�E� 繝ｻ�E�繝ｻ・�E� 繝ｻ繝ｻ
    pub downloaded_path: Option<String>,
    /// 繝ｻ諛搾�E��E�・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ菫�E�謐ｮ繝ｻ・�E� 繝ｻ螢�E�譬�E�繝ｻ�E� 繝ｻ・�E�繝ｻ�E� (false 遶翫・繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ繝ｻ・代・蝗�E
    pub installed: bool,
}

/// 繝ｻ繝ｻ・�E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�/繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub last_check: Option<String>,
    pub next_check: Option<String>,
    pub components: Vec<ComponentVersion>,
    pub checking: bool,
    pub error: Option<String>,
}

/// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ蜩�E� 繝ｻ繝ｻ繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgress {
    /// 繝ｻ繝ｻ・�E�・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ・�E�繝ｻ�E�
    pub complete: bool,
    /// 蠍ｸ繝ｻ讀�E� 繝ｻ蜊鍋�E�・繝ｻ蜊捺筁E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E
    pub current_component: Option<String>,
    /// 繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ
    pub total: usize,
    /// 繝ｻ繝ｻ・�E�隱�E�鬁E�E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ
    pub done: usize,
    /// 繝ｻ・�E�繝ｻ菫�E�鬁E�E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ・�E�繝ｻ繝ｻ
    pub installed_components: Vec<String>,
    /// 繝ｻ蟁E���E��E� 繝ｻ諛阪・ 繝ｻ繝ｻ
    pub errors: Vec<String>,
}

/// 버전 의존성 확인 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCheck {
    /// 확인 대상 컴포넌트 키
    pub component: String,
    /// 모든 의존성이 충족되었는지 여부
    pub satisfied: bool,
    /// 충족되지 않은 의존성 목록
    pub issues: Vec<DependencyIssue>,
}

/// 충족되지 않은 개별 의존성 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyIssue {
    /// 필요한 컴포넌트 키 (예: "saba-core")
    pub required_component: String,
    /// 필요한 최소 버전 (예: ">=0.3.0")
    pub required_version: String,
    /// 현재 설치된 버전 (None이면 미설치)
    pub installed_version: Option<String>,
    /// 사람이 읽을 수 있는 메시지
    pub message: String,
}


/// 繝ｻ繝ｻ・�E�・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ繝ｻ (繝ｻ・�E�繝ｻ・�E�螂晢�E��E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStatus {
    /// 繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ蛟第筈繝ｻ�E� 繝ｻ・�E�繝ｻ�E� (繝ｻ閧�E�迚�E繝ｻ・�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�)
    pub is_fresh_install: bool,
    /// 繝ｻ繝ｻ・�E�・�E� 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ
    pub total_components: usize,
    /// 繝ｻ・�E�繝ｻ菫�E�鬁E�E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ
    pub installed_components: usize,
    /// 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ邁E��・�E�・�E�
    pub components: Vec<ComponentInstallInfo>,
    /// 蠍ｸ繝ｻ讀�E� 繝ｻ繝ｻ蜩�E� 繝ｻ蜊捺筁E繝ｻ・�E�繝ｻ繝ｻ(繝ｻ貁E��諢阪・・�E�)
    pub progress: Option<InstallProgress>,
}

/// 繝ｻ鄂ｹ・�E�繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ・�E�繝ｻ繝ｻ繝ｻ邁E��・�E�・�E�
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInstallInfo {
    pub component: Component,
    pub display_name: String,
    pub installed: bool,
}

// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・
// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ蝨�E� 繝ｻ・�E�繝ｻ・�E� 螟仰繝ｻ繝ｻ(2-flow 繝ｻ繝ｻ縺・�E�帑ｽ懶�E��E�繝ｻ
// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・

/// 繝ｻ鄂ｹ・�E�繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ蝨�E� 繝ｻ・�E�繝ｻ・�E�
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyComponentResult {
    /// 컴포넌트 manifest 키 (예: "module-minecraft", "saba-core")
    pub component: String,
    /// 繝ｻ繝ｻ蝨�E� 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ�E�
    pub success: bool,
    /// 繝ｻ・�E�繝ｻ・�E� 繝ｻ閧�E�莠�E�繝ｻ�E�
    pub message: String,
    /// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�繝ｻ・�E� 繝ｻ繝ｻ邏�E繝ｻ蜊難�E��E��E�繝ｻ繝ｻ蠏薙・・�E�諛坂塙繝ｻ・�E� (繝ｻ鄂ｹ・�E�繝ｻ繝ｻ・�E�繝ｻ・�E�螂難�E��E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ
    pub stopped_processes: Vec<String>,
    /// 繝ｻ・�E�繝ｻ諛肴�E��E� ・代・蝗�E繝ｻ・�E�繝ｻ�E�
    pub restart_needed: bool,
}

/// 繝ｻ繝ｻ・�E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ蝨�E� 繝ｻ・�E�繝ｻ・�E�
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    /// 繝ｻ鄂ｹ・�E�繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ蝨�E� 繝ｻ・�E�繝ｻ・�E�
    pub results: Vec<ApplyComponentResult>,
    /// 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ諛肴�E��E�繝ｻ・�E� 繝ｻ・�E�蠁E���E��E�繝ｻ・�E�蟁E���E��E� 繝ｻ・�E�繝ｻ繝ｻ(CoreDaemon 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ
    pub daemon_restart_script: Option<String>,
    /// GUI/CLI 繝ｻ蟾晢�E��E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�繝ｻ�E� ・代・蝗茨�E�代・繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ・�E�繝ｻ繝ｻ(self-update flow)
    pub self_update_components: Vec<String>,
}

/// GUI/CLI 繝ｻ蟾晢�E��E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ邁E��・�E�・�E� (繝ｻ・�E�螂難�E��E�蟁E��蛟第匿繝ｻ・�E� 遶翫・繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E� ・托ｽ�E�繝ｻ諛堺�E��E�蠏薙・蝨�E�)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateInfo {
    /// 繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E� 繝ｻ・�E�蠏ゅ・逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ繝ｻ
    pub updater_executable: String,
    /// 繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E�繝ｻ繝ｻ繝ｻ繝ｻ蟲�E�・托｣�E� 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ謾�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ
    pub args: Vec<String>,
    /// 繝ｻ�E�繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E
    pub component: String,
    /// 繝ｻ・�E�螂幁E���E�謫�E�繝ｻ邁E��鬁E�E逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ繝ｻ
    pub staged_path: Option<String>,
}

/// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ・�E�繝ｻ繝ｻ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// 繝ｻ・�E�蠁E���E��E� 繝ｻ繝ｻ・�E�・�E� (繝ｻ諛�E�E��E�繝ｻ 繝ｻ・�E�繝ｻ・�E� 3 遶翫・・台�E��E�・�E�・�E� 8蟾舌�E
    pub check_interval_hours: u32,
    /// 繝ｻ・�E�蠁E���E��E� 蠑｡繝ｻ繝ｻ蟁E���E�舌�E・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E
    pub auto_download: bool,
    /// 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E蠑｡繝ｻ繝ｻ蟁E���E�舌�E・�E�繝ｻ繝ｻ繝ｻ繝ｻ蝨�E� (繝ｻ・�E�繝ｻ螢�E�・�E�繝ｻ遯�E�繝ｻ繝ｻ閧�E�迚�ECLI/GUI繝ｻ繝ｻ繝ｻ・�E�繝ｻ諛肴�E��E� ・代・蝗�E
    pub auto_apply: bool,
    /// GitHub 繝ｻ�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ隴�E�謔帙�E繝ｻ
    pub github_owner: String,
    /// GitHub 繝ｻ�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ
    pub github_repo: String,
    /// 蠏薙・・�E�・�E�繝ｻ・�E�繝ｻ・�E�繝ｻ・�E� 蟁E���E��E�・托ｽ�E� 繝ｻ・�E�繝ｻ�E�
    pub include_prerelease: bool,
    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E� 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� (繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ・�E�螂晢�E��E� 繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�, 繝ｻ・�E�繝ｻ・�E�: 繝ｻ・�E�蠏ゅ・逕ｯ隴�E�謾�E� 繝ｻ繝ｻ
    pub install_root: Option<String>,
    /// API 繝ｻ・�E�繝ｻ・�E�繝ｻ・�E� URL 繝ｻ・�E�繝ｻ繝ｻ謾�E�繝ｻ・�E�繝ｻ繝ｻ(螂幁E���E�萓�E�E�厁E���E�繝ｻ・�E� 繝ｻ諛搾�E��E�・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ鄂ｹ・�E�繝ｻ
    /// 繝ｻ繝ｻ "http://127.0.0.1:9876" 遶翫・GitHub API 繝ｻ�E�繝ｻ・�E� 繝ｻ・�E� URL 繝ｻ・�E�繝ｻ・�E�
    #[serde(default)]
    pub api_base_url: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_hours: 3,
            auto_download: false,
            auto_apply: false,
            github_owner: String::new(),
            github_repo: "saba-chan".to_string(),
            include_prerelease: false,
            install_root: None,
            api_base_url: None,
        }
    }
}

// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・
// UpdateManager
// 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・

/// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ・�E�繝ｻ貁E��・�E��E� 遯�E�繝ｻ繝ｻ・�E�繝ｻ・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�/繝ｻ・�E�繝ｻ・�E�螂晢�E��E� 繝ｻ諛搾�E��E�繝ｻ謾�E� 繝ｻ蜊灘�E��E�
pub struct UpdateManager {
    pub config: UpdateConfig,
    /// 繝ｻ貁E��・�E��E�繝ｻ繝ｻ繝ｻ・�E�蠁E���E��E� 繝ｻ・�E�繝ｻ・�E�
    status: UpdateStatus,
    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ
    modules_dir: PathBuf,
    /// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E繝ｻ繝ｻ莠�E� 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E�
    staging_dir: PathBuf,
    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E� 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� (繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ・�E�螂晢�E��E� 繝ｻ繝ｻ繝ｻ・�E�繝ｻ�E�)
    install_root: PathBuf,
    /// 캐시된 최신 릴리즈 정보
    cached_release: Option<GitHubRelease>,
    /// 캐시된 최신 manifest
    cached_manifest: Option<ReleaseManifest>,
    /// fetch한 전체 릴리즈 목록 (walk-back 탐색용)
    cached_releases: Vec<GitHubRelease>,
    /// 릴리즈 횡단 탐색 결과: 각 컴포넌트별 최적 다운로드 소스
    /// key = manifest key ("saba-core", "cli", "gui", ...)
    resolved_components: HashMap<String, ResolvedComponent>,
    /// 설치 진행 상태
    install_progress: Option<InstallProgress>,
}

impl UpdateManager {
    pub fn new(config: UpdateConfig, modules_dir: &str) -> Self {
        // staging 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E�: %APPDATA%/saba-chan/updates/ 繝ｻ蟁E���E��E� ./updates/
        let staging_dir = Self::resolve_staging_dir();

        // install_root: config 繝ｻ�E�繝ｻ繝ｻ繝ｻ蟁E���E��E� 繝ｻ・�E�蠏ゅ・逕ｯ隴�E�謾�E� 繝ｻ繝ｻ
        let install_root = config.install_root.as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."))
            });

        Self {
            config,
            status: UpdateStatus {
                last_check: None,
                next_check: None,
                components: Vec::new(),
                checking: false,
                error: None,
            },
            modules_dir: PathBuf::from(modules_dir),
            staging_dir,
            install_root,
            cached_release: None,
            cached_manifest: None,
            cached_releases: Vec::new(),
            resolved_components: HashMap::new(),
            install_progress: None,
        }
    }

    fn resolve_staging_dir() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("saba-chan").join("updates"))
                .unwrap_or_else(|_| PathBuf::from("./updates"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".cache").join("saba-chan").join("updates"))
                .unwrap_or_else(|_| PathBuf::from("./updates"))
        }
    }

    /// 蠍ｸ繝ｻ讀�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ繝ｻ 繝ｻ・�E�蟾舌�E
    pub fn get_status(&self) -> UpdateStatus {
        self.status.clone()
    }

    /// GitHub API 蠁E���E��E�繝ｻ・�E�繝ｻ・�E�繝ｻ・�E�蟁E���E��E� 繝ｻ譎ｧ笏ｳ (api_base_url 繝ｻ・�E�繝ｻ繝ｻ謾�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ�E�繝ｻ繝ｻ
    fn create_client(&self) -> GitHubClient {
        GitHubClient::with_base_url(
            &self.config.github_owner,
            &self.config.github_repo,
            self.config.api_base_url.as_deref(),
        )
    }

    /// 蠍ｸ繝ｻ讀�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�蟾舌�E
    pub fn get_config(&self) -> UpdateConfig {
        self.config.clone()
    }

    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�
    pub fn update_config(&mut self, new_config: UpdateConfig) {
        // install_root override: config에 install_root가 설정되면 실제 경로 갱신
        if let Some(ref root) = new_config.install_root {
            let new_root = std::path::PathBuf::from(root);
            if new_root != self.install_root {
                tracing::info!("[UpdateManager] install_root updated: {:?} -> {:?}", self.install_root, new_root);
                self.install_root = new_root;
            }
        }
        self.config = new_config;
    }


    // ─── 업데이트 확인 ────────────────────────────────────────────────────────

    /// GitHub에서 릴리즈 + 모듈 리포를 확인하여 컴포넌트별 업데이트 여부를 반환한다.
    ///
    /// ## 핵심 로직 (walk-back)
    /// 릴리즈마다 모든 컴포넌트가 포함되지 않을 수 있으므로,
    /// 여러 릴리즈를 거슬러 올라가며 필요한 에셋을 찾는다.
    ///
    /// 1. 최신 릴리즈의 manifest.json → 최신 버전 확인
    /// 2. 에셋이 없는 컴포넌트 → 이전 릴리즈 순회하며 탐색
    /// 3. 각 컴포넌트별로 실제 에셋이 존재하는 릴리즈 기록 (`resolved_components`)
    pub async fn check_for_updates(&mut self) -> Result<UpdateStatus> {
        if self.config.github_owner.is_empty() || self.config.github_repo.is_empty() {
            anyhow::bail!("GitHub owner/repo not configured");
        }

        self.status.checking = true;
        self.status.error = None;

        let local_versions = self.collect_local_versions();
        let mut components = Vec::new();

        // ══ 1. 코어 리포 체크 (saba-core, cli, gui, updater, discord_bot) ══
        let core_client = self.create_client();
        match self.check_core_repo(&core_client, &local_versions).await {
            Ok(core_components) => {
                components.extend(core_components);
            }
            Err(e) => {
                tracing::error!("[Updater] Core repo check failed: {}", e);
                self.status.checking = false;
                self.status.error = Some(format!("Core repo check failed: {}", e));
                return Err(e);
            }
        }

        // ══ 2. 모듈 리포 개별 체크 ══
        let module_repos = self.discover_module_repos();
        for (module_name, module_repo) in &module_repos {
            let module_client = GitHubClient::with_base_url(
                &self.config.github_owner,
                module_repo,
                self.config.api_base_url.as_deref(),
            );
            match self.check_module_repo(&module_client, module_name, &local_versions).await {
                Ok(Some(cv)) => components.push(cv),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("[Updater] Module '{}' check failed: {}", module_name, e);
                }
            }
        }

        // 타임스탬프 갱신
        let now = chrono_now_iso();
        let next = chrono_add_hours_iso(&now, self.config.check_interval_hours);

        self.status = UpdateStatus {
            last_check: Some(now),
            next_check: Some(next),
            components,
            checking: false,
            error: None,
        };

        Ok(self.status.clone())
    }

    /// 코어 리포에서 릴리즈를 횡단 탐색하여 컴포넌트별 업데이트 정보를 반환한다.
    ///
    /// ## Walk-back 알고리즘
    /// 1. 릴리즈 목록 fetch (30개)
    /// 2. `resolve_components_across_releases`로 각 컴포넌트의 최적 다운로드 소스 결정
    /// 3. 로컬 버전과 비교하여 `ComponentVersion` 목록 생성
    async fn check_core_repo(
        &mut self,
        client: &GitHubClient,
        local_versions: &HashMap<String, String>,
    ) -> Result<Vec<ComponentVersion>> {
        let releases = client.fetch_releases(30).await?;

        // 핵심: 여러 릴리즈를 횡단하여 각 컴포넌트의 에셋 소스를 결정
        let (manifest, resolved) = client.resolve_components_across_releases(
            &releases,
            self.config.include_prerelease,
        ).await?;

        // 캐시 갱신
        let latest_release = releases.iter()
            .filter(|r| !r.draft)
            .find(|r| self.config.include_prerelease || !r.prerelease)
            .cloned();
        self.cached_release = latest_release;
        self.cached_manifest = Some(manifest.clone());
        self.cached_releases = releases;
        self.resolved_components = resolved.clone();

        // ComponentVersion 빌드
        let mut components = Vec::new();
        for (key, info) in &manifest.components {
            // 모듈은 별도 리포에서 처리
            if key.starts_with("module-") {
                continue;
            }

            let component = Component::from_manifest_key(key);
            let current = local_versions.get(key).cloned().unwrap_or_default();
            let update_available = self.compare_versions(&info.version, &current);
            let installed = self.is_component_installed(&component);

            // resolved_components에서 다운로드 URL 조회
            // (최신 릴리즈에 에셋이 없으면 이전 릴리즈에서 찾은 URL이 들어있음)
            let (download_url, asset_name) = if let Some(rc) = resolved.get(key) {
                (Some(rc.download_url.clone()), Some(rc.asset_name.clone()))
            } else {
                (None, None)
            };

            let release_notes = self.cached_release.as_ref().and_then(|r| r.body.clone());
            let published_at = self.cached_release.as_ref().and_then(|r| r.published_at.clone());

            components.push(ComponentVersion {
                component,
                current_version: current,
                latest_version: Some(info.version.clone()),
                update_available,
                download_url,
                asset_name,
                release_notes,
                published_at,
                downloaded: false,
                downloaded_path: None,
                installed,
            });
        }

        Ok(components)
    }

    async fn check_module_repo(
        &self,
        client: &GitHubClient,
        module_name: &str,
        local_versions: &HashMap<String, String>,
    ) -> Result<Option<ComponentVersion>> {
        let releases = client.fetch_releases(5).await?;

        let release = match releases.iter()
            .filter(|r| !r.draft)
            .find(|r| self.config.include_prerelease || !r.prerelease)
        {
            Some(r) => r,
            None => return Ok(None),
        };

        let module_key = format!("module-{}", module_name);
        let component = Component::Module(module_name.to_string());
        let current = local_versions.get(&module_key).cloned().unwrap_or_default();

        // ���그�E��E �E�E��E�E�춁E "v1.2.0" ↁE"1.2.0"
        let latest_version = release.tag_name.trim_start_matches('v').to_string();

        let update_available = self.compare_versions(&latest_version, &current);
        let installed = self.is_component_installed(&component);

        // �E��E ���일 �E��E� (module-{name}.zip �E�는 {name}.zip)
        let asset = release.assets.iter()
            .find(|a| a.name == format!("module-{}.zip", module_name)
                    || a.name == format!("{}.zip", module_name));

        let download_url = asset.map(|a| a.browser_download_url.clone());
        let asset_name = asset.map(|a| a.name.clone());

        // 체크 시에는 항상 downloaded=false 로 시작
        Ok(Some(ComponentVersion {
            component,
            current_version: current,
            latest_version: Some(latest_version),
            update_available,
            download_url,
            asset_name,
            release_notes: release.body.clone(),
            published_at: release.published_at.clone(),
            downloaded: false,
            downloaded_path: None,
            installed,
        }))
    }

    /// module.toml�E�E[update] �E��E�에�E�E�E��E�볁E�E�포 �E�보 �E�직E
    fn discover_module_repos(&self) -> Vec<(String, String)> {
        let mut repos = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.modules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let module_toml = path.join("module.toml");
                    if module_toml.exists() {
                        if let Ok(content) = std::fs::read_to_string(&module_toml) {
                            if let Ok(parsed) = content.parse::<toml::Value>() {
                                let name = parsed.get("module")
                                    .and_then(|m| m.get("name"))
                                    .and_then(|v| v.as_str());
                                let repo = parsed.get("update")
                                    .and_then(|u| u.get("github_repo"))
                                    .and_then(|v| v.as_str());
                                if let (Some(name), Some(repo)) = (name, repo) {
                                    repos.push((name.to_string(), repo.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
        repos
    }

    /// SemVer �E�E��E������
    fn compare_versions(&self, latest: &str, current: &str) -> bool {
        let latest_ver = SemVer::parse(latest);
        let current_ver = SemVer::parse(current);
        match (&latest_ver, &current_ver) {
            (Some(l), Some(c)) => l.is_newer_than(c),
            (Some(_), None) => true, // �E�컬 �E�E��E�E�보 �E�E���E� �E�E���E���� ���E��
            _ => false,
        }
    }

    /// staging �E�렉터�E��E��E �E��E��E�드 �E�E�E ���인
    #[allow(dead_code)]
    fn check_staged_status(&self, asset_name: Option<&str>) -> (bool, Option<String>) {
        match asset_name {
            Some(name) => {
                let staged_path = self.staging_dir.join(name);
                let exists = staged_path.exists();
                (exists, if exists { Some(staged_path.to_string_lossy().to_string()) } else { None })
            }
            None => (false, None),
        }
    }


    // 隨渉隨渉隨渉 繝ｻ諛搾�E��E�・�E� 繝ｻ繝ｻ・�E�繝ｻ繝ｻ蛟托�E��E�繝ｻ隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ蠍ｸ繝ｻ讀�E� 繝ｻ繝ｻ・�E�繝ｻ謠・繝ｻ蛟托�E��E�繝ｻ
    fn collect_local_versions(&self) -> HashMap<String, String> {
        // 1. 설치 매니페스트 우선 로드 (가장 신뢰할 수 있는 소스)
        let mut versions = Self::load_installed_manifest();

        // 2. 매니페스트에 없는 컴포넌트는 기존 방법으로 감지 (폴백)
        if !versions.contains_key("saba-core") {
            versions.insert(
                "saba-core".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            );
        }

        if !versions.contains_key("cli") {
            if let Some(v) = self.read_cargo_version("saba-chan-cli") {
                versions.insert("cli".to_string(), v);
            }
        }

        if !versions.contains_key("gui") {
            if let Some(v) = self.read_package_json_version("saba-chan-gui") {
                versions.insert("gui".to_string(), v);
            }
        }

        if !versions.contains_key("discord_bot") {
            if let Some(v) = self.read_package_json_version("discord_bot") {
                versions.insert("discord_bot".to_string(), v);
            }
        }

        // 모듈: modules/*/module.toml에서 감지
        if let Ok(entries) = std::fs::read_dir(&self.modules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let module_toml = path.join("module.toml");
                    if let Some((name, version)) = self.read_module_version(&module_toml) {
                        let key = format!("module-{}", name);
                        versions.entry(key).or_insert(version);
                    }
                }
            }
        }

        tracing::debug!("[UpdateManager] Local versions: {:?}", versions);
        versions
    }

    fn read_cargo_version(&self, crate_dir: &str) -> Option<String> {
        // 繝ｻ・�E�蠏ゅ・逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ�E�繝ｻ・�E�繝ｻ繝ｻ蠏薙・・�E�諛搾�E��E�譎ｨ謨�E� 繝ｻ・�E�蟁E���E��E� 螟句�E�昴΁E
        let candidates = vec![
            PathBuf::from(crate_dir).join("Cargo.toml"),
            PathBuf::from("..").join(crate_dir).join("Cargo.toml"),
        ];

        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let p = dir.join(crate_dir).join("Cargo.toml");
                if p.exists() {
                    return self.parse_cargo_toml_version(&p);
                }
            }
        }

        for p in candidates {
            if let Some(v) = self.parse_cargo_toml_version(&p) {
                return Some(v);
            }
        }
        None
    }

    fn parse_cargo_toml_version(&self, path: &Path) -> Option<String> {
        let content = std::fs::read_to_string(path).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("version") && trimmed.contains('=') {
                let value = trimmed.split('=').nth(1)?.trim();
                let version = value.trim_matches('"').trim_matches('\'');
                return Some(version.to_string());
            }
        }
        None
    }

    fn read_package_json_version(&self, dir: &str) -> Option<String> {
        let candidates = vec![
            PathBuf::from(dir).join("package.json"),
            PathBuf::from("..").join(dir).join("package.json"),
        ];

        for p in candidates {
            if let Ok(content) = std::fs::read_to_string(&p) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(v) = json.get("version").and_then(|v| v.as_str()) {
                        return Some(v.to_string());
                    }
                }
            }
        }
        None
    }

    fn read_module_version(&self, module_toml: &Path) -> Option<(String, String)> {
        let content = std::fs::read_to_string(module_toml).ok()?;
        let mut name = None;
        let mut version = None;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") && trimmed.contains('=') {
                let val = trimmed.split('=').nth(1)?.trim().trim_matches('"');
                name = Some(val.to_string());
            }
            if trimmed.starts_with("version") && trimmed.contains('=') {
                let val = trimmed.split('=').nth(1)?.trim().trim_matches('"');
                version = Some(val.to_string());
            }
            if name.is_some() && version.is_some() {
                break;
            }
        }

        match (name, version) {
            (Some(n), Some(v)) => Some((n, v)),
            _ => None,
        }
    }

    // 隨渉隨渉隨渉 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�繝ｻ�E� 繝ｻ�E�繝ｻ・�E�・代・繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ・�E� 繝ｻ・�E�螂幁E���E�謫�E�繝ｻ繝ｻ繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E
    /// 업데이트 가능한 모든 컴포넌트를 staging 디렉터리로 다운로드
    ///
    /// resolved_components를 활용하여 각 컴포넌트의 에셋이 실제로 존재하는
    /// 릴리즈에서 다운로드한다 (최신 릴리즈에 없을 수 있음).
    pub async fn download_available_updates(&mut self) -> Result<Vec<String>> {
        std::fs::create_dir_all(&self.staging_dir)?;

        let mut downloaded = Vec::new();

        // 업데이트 가능하고 아직 다운로드하지 않은 컴포넌트 목록
        let to_download: Vec<(String, String, String)> = self.status.components.iter()
            .filter(|c| c.update_available && !c.downloaded)
            .filter_map(|c| {
                let key = c.component.manifest_key();
                // resolved_components에서 실제 다운로드 소스 조회
                self.resolved_components.get(&key).map(|rc| {
                    (key, rc.download_url.clone(), rc.asset_name.clone())
                })
            })
            .collect();

        for (key, url, asset_name) in &to_download {
            // URL에서 직접 다운로드 (특정 릴리즈의 에셋 URL)
            let dest = self.staging_dir.join(asset_name);
            tracing::info!("[Updater] Downloading {} from resolved source", key);

            // resolved_components에 저장된 URL로 직접 다운로드
            let response = reqwest::get(url).await?;
            if !response.status().is_success() {
                anyhow::bail!("Failed to download {}: {}", asset_name, response.status());
            }
            let bytes = response.bytes().await?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &bytes)?;
            tracing::info!("[Updater] Downloaded {} ({} bytes)", asset_name, bytes.len());

            downloaded.push(asset_name.clone());
        }

        // 상태 업데이트: downloaded 플래그 설정
        for comp in &mut self.status.components {
            if let Some(ref name) = comp.asset_name {
                if downloaded.contains(name) {
                    comp.downloaded = true;
                    comp.downloaded_path = Some(
                        self.staging_dir.join(name).to_string_lossy().to_string()
                    );
                }
            }
        }

        Ok(downloaded)
    }

    /// 특정 컴포넌트만 다운로드
    ///
    /// resolved_components를 조회하여 에셋이 포함된 릴리즈에서 다운로드.
    /// 최신 릴리즈에 에셋이 없어도 이전 릴리즈에서 자동으로 찾아온다.
    pub async fn download_component(&mut self, component: &Component) -> Result<String> {
        std::fs::create_dir_all(&self.staging_dir)?;

        let comp_status = self.status.components.iter()
            .find(|c| &c.component == component)
            .ok_or_else(|| anyhow::anyhow!("Component {:?} not found in status", component))?;

        if !comp_status.update_available {
            anyhow::bail!("No update available for {}", component.display_name());
        }

        let key = component.manifest_key();
        let rc = self.resolved_components.get(&key)
            .ok_or_else(|| anyhow::anyhow!(
                "No resolved download source for {} — 에셋을 포함한 릴리즈를 찾지 못함",
                component.display_name()
            ))?;

        let dest = self.staging_dir.join(&rc.asset_name);

        tracing::info!(
            "[Updater] Downloading {} v{} from release {}",
            key, rc.latest_version, rc.source_release_tag
        );

        // resolved URL에서 직접 다운로드
        let response = reqwest::get(&rc.download_url).await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to download {}: {}", rc.asset_name, response.status());
        }
        let bytes = response.bytes().await?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &bytes)?;

        let asset_name = rc.asset_name.clone();

        // 상태 업데이트
        if let Some(comp) = self.status.components.iter_mut().find(|c| &c.component == component) {
            comp.downloaded = true;
            comp.downloaded_path = Some(dest.to_string_lossy().to_string());
        }

        Ok(asset_name)
    }
    // 隨渉隨渉隨渉 繝ｻ繝ｻ蝨�E� 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉悶・繝ｻ繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�繝ｻ・�E� 繝ｻ繝ｻ蝨�E�
    ///
    /// ## 繝ｻ・�E�繝ｻ蛟代℁E�E托ｽ�E�
    /// - **繝ｻ・�E�繝ｻ繝ｻ*: 繝ｻ鬢謎ｺ�E� 逕ｯ隴�E�謾�E� 繝ｻ蟾晢�E��E�・�E� 蠑｡繝ｻ繝ｻ・�E�繝ｻ鄂ｹ邉�E繝ｻ�E�繝ｻ・�E�
    /// - **GUI/CLI**: 逕ｯ隴�E�謾�E� 繝ｻ蟾晢�E��E�・�E� (繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�蠏ゅ・繝ｻ繝ｻ繝ｻ蛟代・)
    /// - **繝ｻ閧�E�迚�E繝ｻ・�E�繝ｻ・�E�**: 繝ｻ蟁E���E��E�・�E� 繝ｻ蟾昜ｾ�E�繝ｻ�E� 繝ｻ蟾晢�E��E�・�E� 繝ｻ荳・�E��E� 遶翫・繝ｻ・�E�繝ｻ諛肴�E��E� 繝ｻ・�E�蠁E���E��E�繝ｻ・�E�蟁E���E��E� 繝ｻ譎ｧ笏ｳ
    pub async fn apply_updates(&mut self) -> Result<Vec<String>> {
        let all_keys: Vec<String> = self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .map(|c| c.component.manifest_key())
            .collect();
        self.apply_components(&all_keys).await
    }

    /// 지정한 컴포넌트만 적용 (빈 목록이면 전체 적용)
    pub async fn apply_components(&mut self, keys: &[String]) -> Result<Vec<String>> {
        let mut applied = Vec::new();

        let components: Vec<ComponentVersion> = self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .filter(|c| keys.is_empty() || keys.contains(&c.component.manifest_key()))
            .cloned()
            .collect();

        for comp in &components {
            let staged_path = comp.downloaded_path.as_ref()
                .ok_or_else(|| anyhow::anyhow!("No staged file for {:?}", comp.component))?;

            match &comp.component {
                Component::Module(name) => {
                    self.apply_module_update(name, staged_path).await?;
                    applied.push(comp.component.display_name());
                }
                Component::Cli => {
                    self.apply_binary_update("saba-cli", staged_path).await?;
                    applied.push(comp.component.display_name());
                }
                Component::Gui => {
                    self.apply_gui_update(staged_path).await?;
                    applied.push(comp.component.display_name());
                }
                Component::CoreDaemon => {
                    // Updater exe can directly replace daemon binary
                    self.apply_binary_update("saba-core", staged_path).await?;
                    applied.push(comp.component.display_name());
                }
                Component::DiscordBot => {
                    self.apply_discord_bot_update(staged_path).await?;
                    applied.push(comp.component.display_name());
                }
            }
        }

        // 繝ｻ繝ｻ蝨�E� 繝ｻ繝ｻ・�E�隱�E�鬁E�E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ繝ｻ繝ｻ繝ｻ 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�
        for comp in &mut self.status.components {
            if applied.iter().any(|a| a.starts_with(&comp.component.display_name())) {
                comp.update_available = false;
                comp.downloaded = false;
                comp.downloaded_path = None;
                if let Some(ref latest) = comp.latest_version {
                    comp.current_version = latest.clone();
                }
            }
        }

        // 적용 성공한 컴포넌트들의 버전을 로컬 매니페스트에 기록
        if !applied.is_empty() {
            if let Err(e) = self.update_installed_versions_batch(&applied) {
                tracing::warn!("[UpdateManager] Failed to update installed manifest: {}", e);
            }
        }

        Ok(applied)
    }

    // 隨渉隨渉隨渉 2-flow 繝ｻ繝ｻ縺・�E�帑ｽ懶�E��E�繝ｻ 繝ｻ鄂ｹ・�E�繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ蝨�E� 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 繝ｻ繝ｻ蝨�E� 繝ｻ�E�繝ｻ・�E� 繝ｻ蜊捺筁E繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ・�E�繝ｻ繝ｻ繝ｻ蛟�E轁E
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
        tracing::info!("[UpdateManager] Saved pending manifest: {} components → {:?}", pending.len(), manifest_path);
        Ok(())
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

    /// pending 매니페스트 파일 삭제 (적용 완료 후)
    pub fn clear_pending_manifest(&self) {
        let manifest_path = self.staging_dir.join("pending.json");
        if manifest_path.exists() {
            let _ = std::fs::remove_file(&manifest_path);
        }
    }

    // ══════════════════════════════════════════════════════
    // 로컬 설치 매니페스트 (installed-manifest.json)
    // ══════════════════════════════════════════════════════

    /// installed-manifest.json 경로 (설치된 각 컴포넌트 버전 추적)
    fn installed_manifest_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("saba-chan").join("installed-manifest.json"))
                .unwrap_or_else(|_| PathBuf::from("installed-manifest.json"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".saba-chan").join("installed-manifest.json"))
                .unwrap_or_else(|_| PathBuf::from("installed-manifest.json"))
        }
    }

    /// 로컬 설치 매니페스트 로드 — 설치된 컴포넌트 버전 맵 반환
    pub fn load_installed_manifest() -> HashMap<String, String> {
        let path = Self::installed_manifest_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(manifest) = serde_json::from_str::<HashMap<String, String>>(&content) {
                tracing::debug!("[UpdateManager] Loaded installed manifest: {} components", manifest.len());
                return manifest;
            }
        }
        HashMap::new()
    }

    /// 로컬 설치 매니페스트 저장
    pub fn save_installed_manifest(versions: &HashMap<String, String>) -> Result<()> {
        let path = Self::installed_manifest_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(versions)?;
        std::fs::write(&path, json)?;
        tracing::info!("[UpdateManager] Saved installed manifest: {} components -> {:?}", versions.len(), path);
        Ok(())
    }

    /// 특정 컴포넌트의 설치 버전을 업데이트하고 매니페스트 저장
    pub fn update_installed_version(component_key: &str, version: &str) -> Result<()> {
        let mut manifest = Self::load_installed_manifest();
        manifest.insert(component_key.to_string(), version.to_string());
        Self::save_installed_manifest(&manifest)
    }

    /// apply 성공 후 적용된 컴포넌트들의 버전을 일괄 업데이트
    pub fn update_installed_versions_batch(&self, applied_keys: &[String]) -> Result<()> {
        let mut manifest = Self::load_installed_manifest();
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
    // 버전 의존성 확인
    // ══════════════════════════════════════════════════════

    /// 컴포넌트의 버전 의존성을 확인합니다.
    /// 서버 매니페스트의 `requires` 필드를 기반으로 설치된 버전과 비교합니다.
    ///
    /// 예: GUI 0.3.0 → requires: { "saba-core": ">=0.3.0" }
    ///     → saba-core이 0.3.0 미만이면 DependencyIssue 반환
    pub fn check_dependencies(&self, component_key: &str) -> DependencyCheck {
        let installed = Self::load_installed_manifest();
        let mut issues = Vec::new();

        // 캐시된 서버 매니페스트에서 requires 정보 조회
        if let Some(ref manifest) = self.cached_manifest {
            if let Some(info) = manifest.components.get(component_key) {
                if let Some(ref requires) = info.requires {
                    for (dep_key, min_version_str) in requires {
                        let dep_version = installed.get(dep_key);
                        let satisfied = dep_version.is_some_and(|v| {
                            // ">=" 접두사 제거 후 SemVer 비교
                            let min_clean = min_version_str.trim_start_matches(">=").trim();
                            match (SemVer::parse(v), SemVer::parse(min_clean)) {
                                (Some(installed_v), Some(required_v)) => installed_v >= required_v,
                                _ => false,
                            }
                        });

                        if !satisfied {
                            issues.push(DependencyIssue {
                                required_component: dep_key.clone(),
                                required_version: min_version_str.clone(),
                                installed_version: dep_version.cloned(),
                                message: format!(
                                    "{} requires {} {} but {} is installed",
                                    component_key, dep_key, min_version_str,
                                    dep_version.map_or("not installed".to_string(), |v| v.clone())
                                ),
                            });
                        }
                    }
                }
            }
        }

        DependencyCheck {
            component: component_key.to_string(),
            satisfied: issues.is_empty(),
            issues,
        }
    }

    /// 모든 업데이트 가능한 컴포넌트의 의존성을 일괄 확인합니다.
    pub fn check_all_dependencies(&self) -> Vec<DependencyCheck> {
        self.status.components.iter()
            .filter(|c| c.update_available)
            .map(|c| self.check_dependencies(&c.component.manifest_key()))
            .collect()
    }


    /// 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ繝ｻ�E�・托ｽ�E� 逕ｯ隴�E�謾�E� 繝ｻ繝ｻ蝨�E�繝ｻ繝ｻ繝ｻ蛟�E陶 (蠏薙・・�E�諛坂塙繝ｻ・�E� 繝ｻ�E�繝ｻ・�E�繝ｻ繝ｻ蠍ｸ・�E�繝ｻ諛肴�E��E� 繝ｻ繝ｻ譯�E�)
    ///
    /// Flow 1 (繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�): IPC ・托ｽ�E�繝ｻ・�E�繝ｻ・�E�繝ｻ�E� 蠏薙・・�E�諛坂塙繝ｻ・�E� 繝ｻ蜊難�E��E��E� 蠑｡繝ｻ繝ｻ・�E� 繝ｻ閧�E�繝ｻ繝ｻ繝ｻ蠍ｸ・�E�繝ｻ繝ｻ
    /// Flow 2 (GUI/CLI): 繝ｻ・�E�蠁E��・�E�, self-update flow繝ｻ繝ｻ繝ｻ繝ｻ蜉�E
    pub async fn apply_single_component(&mut self, component: &Component) -> Result<ApplyComponentResult> {
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded && c.update_available)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Component {:?} is not ready for apply", component))?;

        let staged_path = comp.downloaded_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No staged file for {:?}", component))?;

        let result = match component {
            Component::Module(name) => {
                self.apply_module_update(name, staged_path).await?;
                ApplyComponentResult {
                    component: component.manifest_key(),
                    success: true,
                    message: format!("Module '{}' updated", name),
                    stopped_processes: Vec::new(), // IPC ・托ｽ�E�繝ｻ・�E�繝ｻ・�E�繝ｻ�E� 繝ｻ繝ｻ螳・
                    restart_needed: true,
                }
            }
            Component::CoreDaemon => {
                // Windows: 실행 중인 exe를 .exe.old로 rename 후 새 바이너리 추출
                self.apply_binary_update("saba-core", staged_path).await?;
                ApplyComponentResult {
                    component: component.manifest_key(),
                    success: true,
                    message: "Core Daemon updated (restart required)".to_string(),
                    stopped_processes: Vec::new(),
                    restart_needed: true,
                }
            }
            Component::Cli => {
                self.apply_binary_update("saba-cli", staged_path).await?;
                ApplyComponentResult {
                    component: component.manifest_key(),
                    success: true,
                    message: "CLI updated".to_string(),
                    stopped_processes: Vec::new(),
                    restart_needed: false,
                }
            }
            Component::Gui => {
                // GUI는 업데이터 exe를 통한 self-update flow 필요
                return Ok(ApplyComponentResult {
                    component: component.manifest_key(),
                    success: false,
                    message: "GUI requires self-update flow".to_string(),
                    stopped_processes: Vec::new(),
                    restart_needed: false,
                });
            }
            Component::DiscordBot => {
                self.apply_discord_bot_update(staged_path).await?;
                ApplyComponentResult {
                    component: component.manifest_key(),
                    success: true,
                    message: "Discord Bot updated".to_string(),
                    stopped_processes: Vec::new(),
                    restart_needed: false,
                }
            }
        };

        // 繝ｻ繝ｻ蝨�E� 繝ｻ繝ｻ・�E�繝ｻ遶翫・繝ｻ繝ｻ繝ｻ 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E�
        self.mark_component_applied(component);

        Ok(result)
    }

    /// 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ繝ｻ蝨�E� 繝ｻ繝ｻ・�E�繝ｻ蠑｡繝ｻ繝ｻ繝ｻ繝ｻ 繝ｻ・�E�繝ｻ・�E�
    pub fn mark_component_applied(&mut self, component: &Component) {
        for comp in &mut self.status.components {
            if &comp.component == component {
                comp.update_available = false;
                comp.downloaded = false;
                comp.downloaded_path = None;
                if let Some(ref latest) = comp.latest_version {
                    comp.current_version = latest.clone();
                }
            }
        }
    }

    /// GUI/CLI 繝ｻ蟾晢�E��E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ邁E��・�E�・�E� 繝ｻ蛟�E轁E(繝ｻ・�E�螂難�E��E�蟁E��蛟第匿繝ｻ・�E�繝ｻ�E� 繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E�繝ｻ・�E� 繝ｻ・�E�蟁E���E��E�・台�E��E�・�E�・�E� 繝ｻ繝ｻ・�E�驢占村繝ｻ・�E� 繝ｻ繝ｻ蠕ｩ 繝ｻ邁E��・�E�・�E�)
    pub fn get_self_update_info(&self, component: &Component) -> Result<SelfUpdateInfo> {
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded)
            .ok_or_else(|| anyhow::anyhow!("Component {:?} not downloaded", component))?;

        // 繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E� CLI 繝ｻ・�E�繝ｻ繝ｻ繝ｻ閧�E�・�E�繝ｻ install_root/updater/cli/ 繝ｻ蟁E���E��E� 繝ｻ蜻�E�謐ｩ 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E�
        let updater_exe = self.find_updater_executable()?;

        let staged_path = comp.downloaded_path.clone();
        let component_key = component.manifest_key();

        Ok(SelfUpdateInfo {
            updater_executable: updater_exe,
            args: vec![
                "apply".to_string(),
                "--component".to_string(),
                component_key.clone(),
                "--install-root".to_string(),
                self.install_root.display().to_string(),
            ],
            component: component_key,
            staged_path,
        })
    }

    /// 繝ｻ繝ｻ鬲めE�E・�E�螂難�E��E� CLI 繝ｻ・�E�蠏ゅ・逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ繝ｻ螟句�E�昴΁E
    fn find_updater_executable(&self) -> Result<String> {
        // 繝ｻ・�E�繝ｻ・�E�繝ｻ諛肴�E�: install_root/saba-chan-updater(.exe)
        let candidates = if cfg!(target_os = "windows") {
            vec![
                self.install_root.join("saba-chan-updater.exe"),
            ]
        } else {
            vec![
                self.install_root.join("saba-chan-updater"),
            ]
        };

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.display().to_string());
            }
        }

        // 繝ｻ鄂ｹ・�E�繝ｻ蠍ｹ菫�E�・�E�・�E�: target/release 繝ｻ蟁E���E��E� target/debug
        let dev_candidates = if cfg!(target_os = "windows") {
            vec![
                PathBuf::from("updater/gui/src-tauri/target/release/saba-chan-updater.exe"),
                PathBuf::from("updater/gui/src-tauri/target/debug/saba-chan-updater.exe"),
                PathBuf::from("target/release/saba-chan-updater.exe"),
                PathBuf::from("target/debug/saba-chan-updater.exe"),
            ]
        } else {
            vec![
                PathBuf::from("updater/gui/src-tauri/target/release/saba-chan-updater"),
                PathBuf::from("updater/gui/src-tauri/target/debug/saba-chan-updater"),
                PathBuf::from("target/release/saba-chan-updater"),
                PathBuf::from("target/debug/saba-chan-updater"),
            ]
        };

        for candidate in &dev_candidates {
            if candidate.exists() {
                return Ok(candidate.canonicalize()?.display().to_string());
            }
        }

        // 繝ｻ・�E� 繝ｻ・�E�繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ蛟�E轁E(蠍ｸ・�E�繝ｻ諛肴�E��E�繝ｻ�E� 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ�E� 蠍ｹ闖ｩ謾�E�)
        Ok(candidates[0].display().to_string())
    }

    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ蝨�E� 遯�E�繝ｻzip ・托ｽ�E�繝ｻ繝ｻ蠑｡繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� 繝ｻ蟾晢�E��E�・�E�
    async fn apply_module_update(&self, module_name: &str, staged_path: &str) -> Result<()> {
        let target_dir = self.modules_dir.join(module_name);
        let staged = Path::new(staged_path);

        tracing::info!("[Updater] Applying module update: {} 遶翫・{}", module_name, target_dir.display());

        // 繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ
        let backup_dir = self.staging_dir.join(format!("{}_backup", module_name));
        if target_dir.exists() {
            if backup_dir.exists() {
                std::fs::remove_dir_all(&backup_dir)?;
            }
            self.copy_dir_recursive(&target_dir, &backup_dir)?;
        }

        // zip ・托ｽ�E�繝ｻ繝ｻ
        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::open(staged)?;
            let mut archive = zip::ZipArchive::new(file)?;

            // 繝ｻ・�E�繝ｻ・�E� 逕ｯ隴�E�謾�E� 繝ｻ・�E�繝ｻ繝ｻ蠑｡繝ｻ繝ｻ螢�E�・�E�繝ｻ繝ｻ闖ｩ・�E�繝ｻ・托ｽ�E�繝ｻ繝ｻ
            if target_dir.exists() {
                // __pycache__繝ｻ�E� 繝ｻ蜻�E�謐ｩ 繝ｻ蟾昜ｺ�E�繝ｻ繝ｻ繝ｻ諛榊献・台�E��E�・�E�・�E� 繝ｻ・�E�繝ｻ繝ｻ
                self.clean_module_dir(&target_dir)?;
            }

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                let out_path = target_dir.join(&name);

                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut outfile = std::fs::File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                }
            }
        } else {
            // zip繝ｻ・�E� 繝ｻ繝ｻ譌ｧ 繝ｻ・�E�繝ｻ・�E� 逕ｯ隴�E�謾�E� 遶翫・繝ｻ・�E�繝ｻ�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�
            std::fs::copy(staged, &target_dir)?;
        }

        // 繝ｻ・�E�螂幁E���E�謫�E�繝ｻ繝ｻ逕ｯ隴�E�謾�E� 繝ｻ邁E��・�E�・�E�
        std::fs::remove_file(staged).ok();

        tracing::info!("[Updater] Module '{}' updated successfully", module_name);
        Ok(())
    }

    /// Windows에서 실행 중인 .exe를 rename하기 위한 재시도 로직
    /// 프로세스가 파일을 해제할 때까지 지수 백오프로 최대 max_retries번 재시도
    fn rename_with_retry(from: &Path, to: &Path, max_retries: u32) -> Result<()> {
        // 기존 백업 파일이 있으면 먼저 삭제 시도
        if to.exists() {
            std::fs::remove_file(to).ok();
        }

        let mut last_err = None;
        for attempt in 0..=max_retries {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < max_retries {
                        let delay = std::time::Duration::from_millis(200 * 2u64.pow(attempt));
                        tracing::warn!(
                            "[Updater] rename {} -> {} failed (attempt {}/{}), retrying in {:?}...",
                            from.display(), to.display(), attempt + 1, max_retries + 1, delay
                        );
                        std::thread::sleep(delay);
                    }
                }
            }
        }
        Err(anyhow::anyhow!(
            "Failed to rename {} -> {} after {} attempts: {}",
            from.display(), to.display(), max_retries + 1,
            last_err.unwrap()
        ))
    }

    async fn apply_binary_update(&self, binary_name: &str, staged_path: &str) -> Result<()> {
        let staged = Path::new(staged_path);

        let exe_dir = self.install_root.clone();

        // Windows: 대상 프로세스가 실행 중이라면 종료를 대기
        #[cfg(target_os = "windows")]
        {
            let process_names: Vec<&str> = match binary_name {
                n if n.contains("daemon") || n.contains("core") => vec!["saba-chan.exe"],
                n if n.contains("cli") => vec!["saba-chan-cli.exe"],
                n if n.contains("gui") => vec!["saba-chan-gui.exe"],
                _ => vec![],
            };
            for proc in &process_names {
                if ProcessChecker::is_running(proc) {
                    tracing::info!("[Updater] Waiting for {} to exit before applying update...", proc);
                    let exited = ProcessChecker::wait_for_exit(proc, 15).await;
                    if !exited {
                        tracing::warn!("[Updater] {} did not exit within timeout, attempting update anyway", proc);
                    }
                }
            }
        }

        tracing::info!("[Updater] Applying binary update: {} in {}", binary_name, exe_dir.display());

        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::open(staged)?;
            let mut archive = zip::ZipArchive::new(file)?;

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                let out_path = exe_dir.join(&name);

                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    // Windows繝ｻ蟾昴・ 繝ｻ・�E�繝ｻ・�E� 繝ｻ蜊捺筁E.exe繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ�E�繝ｻ・�E� 蠑｡繝ｻ繝ｻ蟾晢�E��E�・�E�
                    // Windows: 실행 중인 .exe를 .old로 rename (재시도 포함)
                    if out_path.exists() && out_path.extension().map(|e| e == "exe").unwrap_or(false) {
                        let backup = out_path.with_extension("exe.old");
                        if let Err(e) = Self::rename_with_retry(&out_path, &backup, 5) {
                            tracing::error!("[Updater] Cannot replace {}: {}", out_path.display(), e);
                            anyhow::bail!("Cannot replace {}: {}. Is the process still running?", out_path.display(), e);
                        }
                    }
                    let mut outfile = std::fs::File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                }
            }
        }

        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] Binary '{}' updated", binary_name);
        Ok(())
    }

    /// GUI 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ繝ｻ蝨�E�
    async fn apply_gui_update(&self, staged_path: &str) -> Result<()> {
        let staged = Path::new(staged_path);

        // Portable exe mode: install_root/saba-chan-gui.exe
        let portable_exe = self.install_root.join("saba-chan-gui.exe");
        if portable_exe.exists() {
            tracing::info!("[Updater] GUI portable exe detected at {}", portable_exe.display());
            if staged.extension().map(|e| e == "zip").unwrap_or(false) {
                let file = std::fs::File::open(staged)?;
                let mut archive = zip::ZipArchive::new(file)?;
                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i)?;
                    let name = entry.name().to_string();
                    if name.contains("..") { continue; }
                    let out_path = self.install_root.join(&name);
                    if entry.is_dir() {
                        std::fs::create_dir_all(&out_path)?;
                    } else {
                        if let Some(parent) = out_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        if out_path.exists() && out_path.extension().map(|e| e == "exe").unwrap_or(false) {
                            let backup = out_path.with_extension("exe.old");
                            if let Err(e) = Self::rename_with_retry(&out_path, &backup, 5) {
                                tracing::error!("[Updater] Cannot replace GUI exe {}: {}", out_path.display(), e);
                                anyhow::bail!("Cannot replace {}: {}. Is the GUI still running?", out_path.display(), e);
                            }
                        }
                        let mut outfile = std::fs::File::create(&out_path)?;
                        std::io::copy(&mut entry, &mut outfile)?;
                    }
                }
            }
            std::fs::remove_file(staged).ok();
            tracing::info!("[Updater] GUI (portable exe) updated");
            return Ok(());
        }

        // Directory mode fallback (unpacked Electron / dev)
        let gui_dir = self.find_gui_directory()?;
        let extract_dir = {
            let build_dir = gui_dir.join("build");
            if build_dir.exists() {
                build_dir
            } else {
                let res_build = gui_dir.join("resources").join("app").join("build");
                if res_build.exists() {
                    res_build
                } else {
                    if gui_dir.join("src").exists() && gui_dir.join("package.json").exists() {
                        anyhow::bail!(
                            "GUI directory appears to be a source tree ({}). Refusing to overwrite.",
                            gui_dir.display()
                        );
                    }
                    gui_dir.clone()
                }
            }
        };
        tracing::info!("[Updater] Applying GUI update to dir: {}", extract_dir.display());
        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::open(staged)?;
            let mut archive = zip::ZipArchive::new(file)?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                if name.contains("..") { continue; }
                let out_path = extract_dir.join(&name);
                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut outfile = std::fs::File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                }
            }
        }
        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] GUI updated successfully");
        Ok(())
    }

    /// 繝ｻ閧�E�迚�E繝ｻ・�E�繝ｻ・�E� 繝ｻ蟾晢�E��E�・�E� 繝ｻ繝ｻ鬲めE�E・�E�蟁E���E��E� 繝ｻ�E�繝ｻ繝ｻ(繝ｻ・�E�繝ｻ諛肴�E��E� 蠑｡繝ｻ繝ｻ繝ｻ蝨�E�)
    #[allow(dead_code)]
    async fn prepare_daemon_update(&self, staged_path: &str) -> Result<Option<String>> {
        let staged = Path::new(staged_path);
        let daemon_exe_name = if cfg!(target_os = "windows") { "saba-core.exe" } else { "saba-core" };
        let exe_path = self.install_root.join(daemon_exe_name);
        let exe_dir = self.install_root.clone();

        let result_script_path: String;

        // Windows: 繝ｻ・�E�繝ｻ諛肴�E��E� 繝ｻ繝ｻ繝ｻ・�E�蠏る�E�戊�E PowerShell 繝ｻ・�E�蠁E���E��E�繝ｻ・�E�蟁E���E��E� 繝ｻ譎ｧ笏ｳ
        #[cfg(target_os = "windows")]
        {
            let script_path = exe_dir.join("_update_daemon.ps1");
            let script = format!(
                r#"# saba-chan daemon auto-update script
# Wait for the daemon process to exit
Start-Sleep -Seconds 2

$exePath = "{exe}"
$stagedPath = "{staged}"
$backupPath = "$exePath.old"

# Backup current executable
if (Test-Path $exePath) {{
    Move-Item -Force $exePath $backupPath
}}

# Extract update
if ($stagedPath -like "*.zip") {{
    Expand-Archive -Path $stagedPath -DestinationPath "{exe_dir}" -Force
}} else {{
    Copy-Item -Force $stagedPath $exePath
}}

# Restart daemon
Start-Process -FilePath $exePath
Remove-Item -Force $stagedPath -ErrorAction SilentlyContinue
Remove-Item -Force $MyInvocation.MyCommand.Source -ErrorAction SilentlyContinue
"#,
                exe = exe_path.display(),
                staged = staged.display(),
                exe_dir = exe_dir.display(),
            );

            std::fs::write(&script_path, script)?;
            tracing::info!(
                "[Updater] Daemon update prepared 遯�E�繝ｻrun {} after stopping daemon",
                script_path.display()
            );
            result_script_path = script_path.display().to_string();
        }

        #[cfg(not(target_os = "windows"))]
        {
            let script_path = exe_dir.join("_update_daemon.sh");
            let script = format!(
                r#"#!/bin/bash
# saba-chan daemon auto-update script
sleep 2

EXE="{exe}"
STAGED="{staged}"

# Backup
cp "$EXE" "$EXE.old"

# Extract or copy
if [[ "$STAGED" == *.zip ]]; then
    unzip -o "$STAGED" -d "{exe_dir}"
else
    cp "$STAGED" "$EXE"
    chmod +x "$EXE"
fi

# Restart
"$EXE" &
rm -f "$STAGED"
rm -f "$0"
"#,
                exe = exe_path.display(),
                staged = staged.display(),
                exe_dir = exe_dir.display(),
            );

            std::fs::write(&script_path, &script)?;
            // 繝ｻ・�E�蠏ゅ・繝ｻ驢仙ｾ�E� 繝ｻ�E�繝ｻ・�E�
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&script_path)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&script_path, perms)?;
            }

            tracing::info!(
                "[Updater] Daemon update prepared 遯�E�繝ｻrun {} after stopping daemon",
                script_path.display()
            );
            result_script_path = script_path.display().to_string();
        }

        Ok(Some(result_script_path))
    }

    // 隨渉隨渉隨渉 繝ｻ・�E�蟁E�・�E�繝ｻ・�E�蟁E�・�E� 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    fn find_gui_directory(&self) -> Result<PathBuf> {
        // 1) install_root 기준 (컴파일된 배포 환경에서 가장 정확)
        let from_root = self.install_root.join("saba-chan-gui");
        if from_root.exists() {
            return Ok(from_root);
        }

        // 2) exe 기준
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let gui = dir.join("saba-chan-gui");
                if gui.exists() {
                    return Ok(gui);
                }
            }
        }

        // 3) CWD 기준 (개발 환경)
        for p in &["saba-chan-gui", "../saba-chan-gui"] {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
        }

        // 4) Portable exe fallback: if saba-chan-gui.exe exists in install_root, return install_root itself
        let gui_exe_name = if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" };
        if self.install_root.join(gui_exe_name).exists() {
            tracing::info!("[Updater] GUI portable exe found at install_root, returning install_root as gui_dir");
            return Ok(self.install_root.clone());
        }

        anyhow::bail!("GUI directory not found (checked: install_root={}, exe_dir, cwd)", self.install_root.display())
    }

    fn find_discord_bot_directory(&self) -> Result<PathBuf> {
        // 1) install_root 기준 (컴파일된 배포 환경에서 가장 정확)
        let from_root = self.install_root.join("discord_bot");
        if from_root.exists() {
            return Ok(from_root);
        }

        // 2) exe 기준
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let bot = dir.join("discord_bot");
                if bot.exists() {
                    return Ok(bot);
                }
            }
        }

        // 3) CWD 기준 (개발 환경)
        for p in &["discord_bot", "../discord_bot"] {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
        }

        anyhow::bail!("Discord Bot directory not found")
    }

    async fn apply_discord_bot_update(&self, staged_path: &str) -> Result<()> {
        let target_dir = self.find_discord_bot_directory()
            .unwrap_or_else(|_| {
                // If not found, create next to exe or in current dir
                if let Ok(exe) = std::env::current_exe() {
                    exe.parent().map(|p| p.join("discord_bot")).unwrap_or_else(|| PathBuf::from("discord_bot"))
                } else {
                    PathBuf::from("discord_bot")
                }
            });
        let staged = Path::new(staged_path);

        tracing::info!("[Updater] Applying Discord Bot update -> {}", target_dir.display());

        // Backup existing
        let backup_dir = self.staging_dir.join("discord_bot_backup");
        if target_dir.exists() {
            if backup_dir.exists() {
                std::fs::remove_dir_all(&backup_dir)?;
            }
            self.copy_dir_recursive(&target_dir, &backup_dir)?;
        }

        // Extract zip
        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::open(staged)?;
            let mut archive = zip::ZipArchive::new(file)?;

            if target_dir.exists() {
                self.clean_module_dir(&target_dir)?;
            } else {
                std::fs::create_dir_all(&target_dir)?;
            }

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                let out_path = target_dir.join(&name);

                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut outfile = std::fs::File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                }
            }
        } else {
            std::fs::copy(staged, &target_dir)?;
        }

        // Clean staged file
        std::fs::remove_file(staged).ok();

        tracing::info!("[Updater] Discord Bot updated successfully");
        Ok(())
    }

    fn clean_module_dir(&self, dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // __pycache__, .git 繝ｻ・�E�繝ｻ�E� 繝ｻ・�E�繝ｻ螢�E�諤・
            if name_str == "__pycache__" || name_str.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            let path = entry.path();
            let dest = dst.join(entry.file_name());
            if path.is_dir() {
                self.copy_dir_recursive(&path, &dest)?;
            } else {
                std::fs::copy(&path, &dest)?;
            }
        }
        Ok(())
    }

    // 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・
    // 繝ｻ・�E�繝ｻ・�E�螂晢�E��E�繝ｻ・�E� 繝ｻ・�E�繝ｻ・�E�
    // 隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨顔ｵ�E�豁E��隨翫・

    /// 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ�E� 繝ｻ諛搾�E��E�・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ菫�E�謐ｮ繝ｻ・�E� 繝ｻ螢�E�譬�E�繝ｻ�E� 蠍ｹ闖ｩ謾�E�
    pub fn is_component_installed(&self, component: &Component) -> bool {
        match component {
            Component::CoreDaemon => {
                // 繝ｻ閧�E�迚�E繝ｻ・�E�繝ｻ・�E�繝ｻ�E� 繝ｻ蟁E���E��E�・�E� 繝ｻ蟾昜ｾ�E�繝ｻ・�E�繝ｻ�E�繝ｻ繝ｻ・托ｽ�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ菫�E�蜍｢
                true
            }
            Component::Cli => {
                let exe_name = if cfg!(windows) { "saba-chan-cli.exe" } else { "saba-chan-cli" };
                self.install_root.join(exe_name).exists()
                    || PathBuf::from("saba-chan-cli").join("target").exists()
            }
            Component::Gui => {
                // Portable exe OR directory
                let exe_name = if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" };
                if self.install_root.join(exe_name).exists() {
                    return true;
                }
                self.find_gui_directory().ok().map(|d| d.exists()).unwrap_or(false)
            }
            Component::Module(name) => {
                let module_dir = self.modules_dir.join(name);
                module_dir.join("module.toml").exists()
            }
            Component::DiscordBot => {
                // discord_bot 디렉토리에 index.js + package.json 존재 확인
                self.find_discord_bot_directory().ok().map(|d| d.join("index.js").exists()).unwrap_or(false)
            }
        }
    }

    /// 繝ｻ繝ｻ・�E�・�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ繝ｻ 繝ｻ閧�E�蟷�E� 繝ｻ・�E�蟾舌�E
    pub fn get_install_status(&self) -> InstallStatus {
        let components: Vec<(Component, bool)> = vec![
            (Component::CoreDaemon, self.is_component_installed(&Component::CoreDaemon)),
            (Component::Cli, self.is_component_installed(&Component::Cli)),
            (Component::Gui, self.is_component_installed(&Component::Gui)),
            (Component::DiscordBot, self.is_component_installed(&Component::DiscordBot)),
        ];

        // 繝ｻ・�E�繝ｻ螢�E�雎�E・繝ｻ蠍ｹ闖ｩ謾�E� 遯�E�繝ｻmanifest繝ｻ蟾昴・ 繝ｻ隱�E�・�E�・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E�
        let mut module_components = Vec::new();
        if let Some(ref manifest) = self.cached_manifest {
            for key in manifest.components.keys() {
                if key.starts_with("module-") {
                    let comp = Component::from_manifest_key(key);
                    let installed = self.is_component_installed(&comp);
                    module_components.push((comp, installed));
                }
            }
        } else {
            // manifest 繝ｻ繝ｻ諢阪・・�E� 繝ｻ諛搾�E��E�・�E� modules/ 繝ｻ・�E�繝ｻ繝ｻ
            if let Ok(entries) = std::fs::read_dir(&self.modules_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let comp = Component::Module(name.clone());
                        let installed = self.is_component_installed(&comp);
                        module_components.push((comp, installed));
                    }
                }
            }
        }

        let all: Vec<(Component, bool)> = components.into_iter()
            .chain(module_components)
            .collect();

        let total = all.len();
        let installed_count = all.iter().filter(|(_, i)| *i).count();
        let is_fresh = installed_count <= 1; // 繝ｻ閧�E�迚�E繝ｻ・�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ菫�E�蜍｢

        InstallStatus {
            is_fresh_install: is_fresh,
            total_components: total,
            installed_components: installed_count,
            components: all.into_iter().map(|(c, i)| ComponentInstallInfo {
                component: c.clone(),
                display_name: c.display_name(),
                installed: i,
            }).collect(),
            progress: self.install_progress.clone(),
        }
    }

    /// 繝ｻ繝ｻ・�E�繝ｻ繝ｻ・�E�繝ｻ菫�E�謐ｮ繝ｻ�E� 繝ｻ蝟懈昁E繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ・�E� 繝ｻ・�E�繝ｻ・�E�螂晢�E��E� (繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ繝ｻ
    ///
    /// 蠖ｧ蟁E���E��E�繝ｻ manifest 蟁E��蛟托�E��E�繝ｻ遶翫・繝ｻ・�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E繝ｻ譎､・�E�繝ｻ遶翫・繝ｻ・�E�繝ｻ・�E�繝ｻ鄂ｹ邉�E遶翫・繝ｻ闖ｩ・�E�繝ｻ・托ｽ�E�繝ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ
    /// 미설치된 필수 컴포넌트를 설치하는 초기 설치 (릴리즈 횡단 탐색 지원)
    ///
    /// resolved_components를 활용하여 에셋이 포함된 릴리즈에서 개별 다운로드.
    pub async fn fresh_install(&mut self, components_filter: Option<Vec<String>>) -> Result<InstallProgress> {
        if self.config.github_owner.is_empty() || self.config.github_repo.is_empty() {
            anyhow::bail!("GitHub owner/repo not configured — cannot install");
        }

        let client = self.create_client();

        // 릴리즈 목록 fetch & 횡단 탐색
        let releases = client.fetch_releases(30).await?;
        let (manifest, resolved) = client.resolve_components_across_releases(
            &releases,
            self.config.include_prerelease,
        ).await?;

        let latest_release = releases.iter()
            .filter(|r| !r.draft)
            .find(|r| self.config.include_prerelease || !r.prerelease)
            .cloned();
        self.cached_release = latest_release;
        self.cached_manifest = Some(manifest.clone());
        self.cached_releases = releases;
        self.resolved_components = resolved.clone();

        // 설치 대상 필터
        let targets: Vec<(String, github::ComponentInfo)> = manifest.components.iter()
            .filter(|(key, _)| {
                if let Some(ref filter) = components_filter {
                    filter.iter().any(|f| f == *key)
                } else {
                    true
                }
            })
            .filter(|(key, _)| {
                let comp = Component::from_manifest_key(key);
                if comp == Component::CoreDaemon {
                    return false;
                }
                !self.is_component_installed(&comp)
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if targets.is_empty() {
            let progress = InstallProgress {
                complete: true,
                current_component: None,
                total: 0,
                done: 0,
                installed_components: vec![],
                errors: vec![],
            };
            self.install_progress = Some(progress.clone());
            return Ok(progress);
        }

        let total = targets.len();
        let mut installed = Vec::new();
        let mut errors = Vec::new();

        self.install_progress = Some(InstallProgress {
            complete: false,
            current_component: None,
            total,
            done: 0,
            installed_components: vec![],
            errors: vec![],
        });

        std::fs::create_dir_all(&self.staging_dir)?;

        for (idx, (key, info)) in targets.iter().enumerate() {
            let component = Component::from_manifest_key(key);
            let comp_label = component.display_name();

            if let Some(ref mut prog) = self.install_progress {
                prog.current_component = Some(comp_label.clone());
                prog.done = idx;
            }

            tracing::info!("[Installer] ({}/{}) Installing {}...", idx + 1, total, comp_label);

            // resolved_components에서 다운로드 소스 조회
            let rc = match resolved.get(key) {
                Some(rc) => rc,
                None => {
                    let err = format!("{}: 에셋을 포함한 릴리즈를 찾지 못함", comp_label);
                    tracing::warn!("[Installer] {}", err);
                    errors.push(err);
                    continue;
                }
            };

            let staged_path = self.staging_dir.join(&rc.asset_name);

            // resolved URL에서 다운로드
            tracing::info!(
                "[Installer] {} v{} ← release {}",
                key, rc.latest_version, rc.source_release_tag
            );
            let download_result: Result<()> = async {
                let response = reqwest::get(&rc.download_url).await?;
                if !response.status().is_success() {
                    anyhow::bail!("HTTP {}", response.status());
                }
                let bytes = response.bytes().await?;
                std::fs::write(&staged_path, &bytes)?;
                Ok(())
            }.await;

            if let Err(e) = download_result {
                let err = format!("Download failed for {}: {}", comp_label, e);
                tracing::error!("[Installer] {}", err);
                errors.push(err);
                continue;
            }

            // 설치 디렉터리 결정 & 압축 해제
            let install_dir = self.resolve_install_dir(&component, info.install_dir.as_deref());

            if let Err(e) = self.extract_to_directory(&staged_path, &install_dir).await {
                let err = format!("Extraction failed for {}: {}", comp_label, e);
                tracing::error!("[Installer] {}", err);
                errors.push(err);
                continue;
            }

            std::fs::remove_file(&staged_path).ok();

            installed.push(comp_label.clone());
            let dir_path = install_dir.to_string_lossy();
            tracing::info!("[Installer] {} installed to {}", comp_label, dir_path);
        }

        // 기본 config 파일 생성 (필요하면)
        self.ensure_default_config().ok();

        let progress = InstallProgress {
            complete: true,
            current_component: None,
            total,
            done: installed.len(),
            installed_components: installed,
            errors,
        };
        self.install_progress = Some(progress.clone());

        Ok(progress)
    }

    /// 특정 컴포넌트를 단일 설치 (릴리즈 횡단 탐색 지원)
    pub async fn install_component(&mut self, component: &Component) -> Result<String> {
        if self.config.github_owner.is_empty() || self.config.github_repo.is_empty() {
            anyhow::bail!("GitHub owner/repo not configured");
        }

        if self.is_component_installed(component) {
            anyhow::bail!("{} is already installed", component.display_name());
        }

        let client = self.create_client();
        let key = component.manifest_key();

        // resolved_components가 있으면 그것을 사용, 없으면 릴리즈를 새로 fetch
        if self.resolved_components.is_empty() {
            let releases = client.fetch_releases(30).await?;
            let (manifest, resolved) = client.resolve_components_across_releases(
                &releases,
                self.config.include_prerelease,
            ).await?;
            let latest_release = releases.iter()
                .filter(|r| !r.draft)
                .find(|r| self.config.include_prerelease || !r.prerelease)
                .cloned();
            self.cached_release = latest_release;
            self.cached_manifest = Some(manifest);
            self.cached_releases = releases;
            self.resolved_components = resolved;
        }

        let rc = self.resolved_components.get(&key)
            .ok_or_else(|| anyhow::anyhow!(
                "Component '{}' 에 대한 에셋을 어떤 릴리즈에서도 찾지 못함", key
            ))?;

        let manifest = self.cached_manifest.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No cached manifest"))?;
        let info = manifest.components.get(&key)
            .ok_or_else(|| anyhow::anyhow!("Component '{}' not found in manifest", key))?;

        // resolved URL에서 직접 다운로드
        std::fs::create_dir_all(&self.staging_dir)?;
        let staged_path = self.staging_dir.join(&rc.asset_name);

        tracing::info!(
            "[Installer] Downloading {} v{} from release {}",
            key, rc.latest_version, rc.source_release_tag
        );

        let response = reqwest::get(&rc.download_url).await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to download {}: {}", rc.asset_name, response.status());
        }
        let bytes = response.bytes().await?;
        std::fs::write(&staged_path, &bytes)?;

        let install_dir = self.resolve_install_dir(component, info.install_dir.as_deref());
        self.extract_to_directory(&staged_path, &install_dir).await?;
        std::fs::remove_file(&staged_path).ok();

        tracing::info!("[Installer] {} installed to {}", component.display_name(), install_dir.display());
        Ok(install_dir.to_string_lossy().to_string())
    }
    /// 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ蜩�E� 繝ｻ繝ｻ繝ｻ 繝ｻ・�E�蟾舌�E
    pub fn get_install_progress(&self) -> Option<InstallProgress> {
        self.install_progress.clone()
    }

    // 隨渉隨渉隨渉 繝ｻ・�E�繝ｻ・�E�螂晢�E��E�繝ｻ・�E� 繝ｻ・�E�蟁E�・�E�繝ｻ・�E�蟁E�・�E� 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 繝ｻ・�E�蟁E���E��E�繝ｻ驢先要E��ｻ繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ
    fn resolve_install_dir(&self, component: &Component, manifest_dir: Option<&str>) -> PathBuf {
        // manifest繝ｻ繝ｻinstall_dir繝ｻ・�E� 繝ｻ貁E��諢阪・・�E� install_root 繝ｻ・�E�繝ｻ�E� 繝ｻ繝ｻ蜉�E・・�E�繝ｻ繝ｻ
        if let Some(dir) = manifest_dir {
            return self.install_root.join(dir);
        }

        // 繝ｻ・�E�繝ｻ・�E� 繝ｻ諛搾�E��E�繝ｻ
        match component {
            Component::CoreDaemon => self.install_root.clone(),
            Component::Cli => self.install_root.clone(),
            Component::Gui => self.install_root.join("saba-chan-gui"),
            Component::Module(name) => self.install_root.join("modules").join(name),
            Component::DiscordBot => self.install_root.join("discord_bot"),
        }
    }

    /// zip(繝ｻ蟁E���E��E� 繝ｻ・�E�繝ｻ・�E� 逕ｯ隴�E�謾�E�)繝ｻ繝ｻ繝ｻ�E�繝ｻ繝ｻ繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E�繝ｻ繝ｻ繝ｻ閧�E�・�E�繝ｻ
    async fn extract_to_directory(&self, staged: &Path, target_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(target_dir)?;

        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            let file = std::fs::File::open(staged)?;
            let mut archive = zip::ZipArchive::new(file)?;

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                let out_path = target_dir.join(&name);

                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut outfile = std::fs::File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut outfile)?;
                }
            }
        } else {
            // 繝ｻ・�E�繝ｻ・�E� 逕ｯ隴�E�謾�E� 遶翫・target_dir 繝ｻ貁E��荵�E� 繝ｻ・�E�繝ｻ・�E�
            let file_name = staged.file_name().unwrap_or_default();
            std::fs::copy(staged, target_dir.join(file_name))?;
        }

        Ok(())
    }

    /// 繝ｻ諛搾�E��E�繝ｻ繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ繝ｻ・�E�繝ｻ・�E� 繝ｻ・�E�繝ｻ繝ｻ逕ｯ隴�E�謾�E�繝ｻ・�E� 繝ｻ繝ｻ諢阪・・�E� 繝ｻ譎ｧ笏ｳ
    fn ensure_default_config(&self) -> Result<()> {
        let config_dir = self.install_root.join("config");
        let global_toml = config_dir.join("global.toml");

        if !global_toml.exists() {
            std::fs::create_dir_all(&config_dir)?;
            let default_config = format!(
                r#"ipc_socket = "./ipc.sock"

[updater]
enabled = true
check_interval_hours = 3
auto_download = false
auto_apply = false
github_owner = "{owner}"
github_repo = "{repo}"
include_prerelease = false
"#,
                owner = self.config.github_owner,
                repo = self.config.github_repo,
            );
            std::fs::write(&global_toml, default_config)?;
            tracing::info!("[Installer] Created default config at {}", global_toml.display());
        }

        // modules 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� 繝ｻ譎ｧ笏ｳ
        let modules_dir = self.install_root.join("modules");
        std::fs::create_dir_all(&modules_dir)?;

        // locales 繝ｻ鄂ｷ・�E�鬧補肇繝ｻ・�E� 繝ｻ譎ｧ笏ｳ
        let locales_dir = self.install_root.join("locales");
        std::fs::create_dir_all(&locales_dir)?;

        Ok(())
    }

    // 隨渉隨渉隨渉 螂幁E���E�萓�E�E�厁E���E� 蠏ｭ・�E�蟁E���E��E� (pub) 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

    /// 螂幁E���E�萓�E�E�厁E���E� 繝ｻ繝ｻ蝨�E�: extract_to_directory繝ｻ・�E� 繝ｻ・�E�繝ｻ�E�繝ｻ蟾昴・ 蠍ｸ・�E�繝ｻ繝ｻ
    #[doc(hidden)]
    pub async fn extract_to_directory_for_test(&self, staged: &Path, target: &Path) {
        self.extract_to_directory(staged, target).await.unwrap();
    }

    /// 螂幁E���E�萓�E�E�厁E���E� 繝ｻ繝ｻ蝨�E�: resolve_install_dir繝ｻ・�E� 繝ｻ・�E�繝ｻ�E�繝ｻ蟾昴・ 蠍ｸ・�E�繝ｻ繝ｻ
    #[doc(hidden)]
    pub fn resolve_install_dir_for_test(&self, component: &Component, manifest_dir: Option<&str>) -> PathBuf {
        self.resolve_install_dir(component, manifest_dir)
    }
}

// 隨渉隨渉隨渉 繝ｻ諛�E�E��E�繝ｻ繝ｻ・�E�蟁E�・�E� (chrono 繝ｻ繝ｻ謫�E�) 隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉隨渉

/// 蠍ｸ繝ｻ讀�E� 繝ｻ諛�E�E��E�繝ｻ謠・ISO 8601 蠍ｸ闖ｩ莠�E�繝ｻ・�E�繝ｻ繝ｻ繝ｻ蛟�E轁E
fn chrono_now_iso() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(now.as_secs())
}

/// hours 繝ｻ諛�E�E��E�繝ｻ蠑｡繝ｻ謾�E� ISO 8601 繝ｻ・�E�繝ｻ蟾晞ｾ・
fn chrono_add_hours_iso(_iso: &str, hours: u32) -> String {
    // 繝ｻ繝ｻ蜊ｿ・代・繝ｻ・�E�蠍ｸ繝ｻ 蠍ｸ繝ｻ讀�E� UNIX timestamp + hours * 3600
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let future = now.as_secs() + (hours as u64 * 3600);
    format_unix_timestamp(future)
}

fn format_unix_timestamp(secs: u64) -> String {
    // 繝ｻ繝ｻ蜊ｿ・代・UTC 繝ｻ・�E�繝ｻ繝ｻ繝ｻ諛�E�E��E�繝ｻ蟁E���E��E�繝ｻ・�E�逕ｯ繝ｻ
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Unix epoch (1970-01-01) 繝ｻ・�E�繝ｻ�E� 繝ｻ・�E�繝ｻ繝ｻ繝ｻ繝ｻ縺・
    let (year, month, day) = days_to_date(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    // 繝ｻ繝ｻ蜊ｿ・代・繝ｻ・�E�繝ｻ荳・�E�・�E�繝ｻ・�E�繝ｻ・�E� 繝ｻ�E�蠍ｹ繝ｻ
    let mut y = 1970;
    let mut remaining = days as i64;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let days_in_months: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for (i, &dim) in days_in_months.iter().enumerate() {
        if remaining < dim {
            m = i;
            break;
        }
        remaining -= dim;
    }

    (y as u64, (m + 1) as u64, (remaining + 1) as u64)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

