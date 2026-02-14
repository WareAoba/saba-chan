//! 포그라운드 적용 — GUI/CLI 종료 후 업데이터가 파일 수정
//!
//! ## 2단계 업데이트 플로우
//! 1. 백그라운드: 버전 체크 + 다운로드 (worker.rs)
//! 2. 포그라운드: GUI/CLI 종료 → 업데이터가 파일 적용 → 재시작
//!
//! ## 적용 모드
//! - **모듈 적용**: 바로 진행 (프로세스 중단 불필요)
//! - **데몬 적용**: 데몬 중지 → 적용 → 재시작
//! - **GUI/CLI 적용**: 업데이터가 셀프 업데이트 수행

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};

use crate::{Component, UpdateManager, ApplyResult, ApplyComponentResult};

/// 적용 전 준비 상태
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPreparation {
    /// 적용할 컴포넌트 목록
    pub components: Vec<String>,
    /// GUI/CLI 재시작 필요 여부
    pub requires_restart: bool,
    /// 데몬 재시작 필요 여부
    pub requires_daemon_restart: bool,
    /// 셀프 업데이트 필요 여부 (업데이터 자체 업데이트)
    pub requires_self_update: bool,
    /// 예상 소요 시간 (초)
    pub estimated_seconds: u32,
}

/// 적용 진행 상태
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyProgress {
    pub phase: ApplyPhase,
    pub current_component: Option<String>,
    pub total: usize,
    pub done: usize,
    pub message: String,
}

/// 적용 단계
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApplyPhase {
    /// 준비 중
    Preparing,
    /// 프로세스 종료 대기
    WaitingForProcesses,
    /// 파일 적용 중
    Applying,
    /// 재시작 중
    Restarting,
    /// 완료
    Completed,
    /// 실패
    Failed,
}

/// 포그라운드 적용 매니저
pub struct ForegroundApplier {
    manager: Arc<RwLock<UpdateManager>>,
    /// GUI 실행 파일 경로
    gui_executable: Option<PathBuf>,
    /// CLI 실행 파일 경로
    cli_executable: Option<PathBuf>,
    /// 현재 진행 상태
    progress: Arc<RwLock<Option<ApplyProgress>>>,
}

impl ForegroundApplier {
    pub fn new(manager: Arc<RwLock<UpdateManager>>) -> Self {
        Self {
            manager,
            gui_executable: None,
            cli_executable: None,
            progress: Arc::new(RwLock::new(None)),
        }
    }

    /// GUI 실행 파일 설정
    pub fn set_gui_executable(&mut self, path: PathBuf) {
        self.gui_executable = Some(path);
    }

    /// CLI 실행 파일 설정
    pub fn set_cli_executable(&mut self, path: PathBuf) {
        self.cli_executable = Some(path);
    }

    /// 적용 준비 상태 확인
    pub async fn prepare(&self) -> ApplyPreparation {
        let mgr = self.manager.read().await;
        let pending = mgr.get_pending_components();

        let component_names: Vec<String> = pending
            .iter()
            .map(|c| c.component.display_name())
            .collect();

        let requires_restart = pending
            .iter()
            .any(|c| matches!(c.component, Component::Gui | Component::Cli));

        let requires_daemon_restart = pending
            .iter()
            .any(|c| matches!(c.component, Component::CoreDaemon));

        let requires_self_update = requires_restart; // GUI/CLI 업데이트 시 셀프 업데이트 필요

        // 예상 시간: 컴포넌트당 5초 + 재시작 시 10초
        let estimated_seconds = (pending.len() as u32 * 5)
            + if requires_restart { 10 } else { 0 }
            + if requires_daemon_restart { 10 } else { 0 };

        ApplyPreparation {
            components: component_names,
            requires_restart,
            requires_daemon_restart,
            requires_self_update,
            estimated_seconds,
        }
    }

    /// 현재 진행 상태 조회
    pub async fn get_progress(&self) -> Option<ApplyProgress> {
        self.progress.read().await.clone()
    }

    /// 진행 상태 업데이트
    async fn update_progress(&self, progress: ApplyProgress) {
        *self.progress.write().await = Some(progress);
    }

    /// 모듈만 적용 (프로세스 중단 불필요)
    pub async fn apply_modules_only(&self) -> Result<Vec<String>, String> {
        self.update_progress(ApplyProgress {
            phase: ApplyPhase::Applying,
            current_component: None,
            total: 0,
            done: 0,
            message: "모듈 업데이트 적용 중...".to_string(),
        }).await;

        let mut applied = Vec::new();
        let mut mgr = self.manager.write().await;

        let modules: Vec<Component> = mgr
            .get_pending_components()
            .iter()
            .filter(|c| matches!(c.component, Component::Module(_)))
            .map(|c| c.component.clone())
            .collect();

        let total = modules.len();
        for (idx, module) in modules.iter().enumerate() {
            self.update_progress(ApplyProgress {
                phase: ApplyPhase::Applying,
                current_component: Some(module.display_name()),
                total,
                done: idx,
                message: format!("{} 적용 중...", module.display_name()),
            }).await;

            match mgr.apply_single_component(module).await {
                Ok(result) if result.success => {
                    applied.push(module.display_name());
                }
                Ok(result) => {
                    tracing::warn!("[Apply] Module {} failed: {}", module.display_name(), result.message);
                }
                Err(e) => {
                    tracing::error!("[Apply] Module {} error: {}", module.display_name(), e);
                }
            }
        }

        self.update_progress(ApplyProgress {
            phase: ApplyPhase::Completed,
            current_component: None,
            total,
            done: applied.len(),
            message: format!("{} 모듈 업데이트 완료", applied.len()),
        }).await;

        Ok(applied)
    }

    /// 전체 적용 (GUI/CLI 종료 필요)
    /// 
    /// 이 함수는 업데이터 CLI에서 호출됩니다.
    /// GUI/CLI가 이미 종료된 상태에서 실행됩니다.
    pub async fn apply_all(&self) -> Result<ApplyResult, String> {
        self.update_progress(ApplyProgress {
            phase: ApplyPhase::Preparing,
            current_component: None,
            total: 0,
            done: 0,
            message: "업데이트 준비 중...".to_string(),
        }).await;

        let mut mgr = self.manager.write().await;

        // 적용 실행
        self.update_progress(ApplyProgress {
            phase: ApplyPhase::Applying,
            current_component: None,
            total: 0,
            done: 0,
            message: "업데이트 적용 중...".to_string(),
        }).await;

        let result = mgr.apply_updates().await.map_err(|e| e.to_string())?;

        let apply_result = ApplyResult {
            results: result.iter().map(|name| ApplyComponentResult {
                component: name.clone(),
                success: true,
                message: format!("{} 업데이트 완료", name),
                stopped_processes: Vec::new(),
                restart_needed: false,
            }).collect(),
            daemon_restart_script: None,
            self_update_components: Vec::new(),
        };

        self.update_progress(ApplyProgress {
            phase: ApplyPhase::Completed,
            current_component: None,
            total: result.len(),
            done: result.len(),
            message: "업데이트 적용 완료!".to_string(),
        }).await;

        Ok(apply_result)
    }

    /// GUI 재시작
    pub fn relaunch_gui(&self, after_update: bool) -> Result<(), String> {
        let exe = self.gui_executable.as_ref()
            .ok_or("GUI 실행 파일이 설정되지 않았습니다")?;

        let mut cmd = Command::new(exe);
        if after_update {
            cmd.arg("--after-update");
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x00000008 | 0x01000000 | 0x00000200);
        }

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("GUI 재시작 실패: {}", e))?;

        Ok(())
    }

    /// CLI 재시작
    pub fn relaunch_cli(&self, args: &[String], after_update: bool) -> Result<(), String> {
        let exe = self.cli_executable.as_ref()
            .ok_or("CLI 실행 파일이 설정되지 않았습니다")?;

        let mut cmd = Command::new(exe);
        cmd.args(args);
        if after_update {
            cmd.arg("--after-update");
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x00000008 | 0x01000000 | 0x00000200);
        }

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("CLI 재시작 실패: {}", e))?;

        Ok(())
    }
}

/// 셀프 업데이트 실행기
/// 
/// GUI/CLI가 자신을 업데이트할 때 사용합니다.
/// 1. 업데이터 프로세스 스폰
/// 2. 현재 프로세스 종료
/// 3. 업데이터가 파일 교체 후 재시작
pub struct SelfUpdater {
    /// 업데이터 실행 파일 경로
    updater_exe: PathBuf,
    /// 적용할 컴포넌트
    component: Component,
    /// 스테이징 경로
    staged_path: PathBuf,
    /// 재시작할 실행 파일
    relaunch_exe: Option<PathBuf>,
    /// 재시작 인자
    relaunch_args: Vec<String>,
}

impl SelfUpdater {
    pub fn new(updater_exe: PathBuf, component: Component, staged_path: PathBuf) -> Self {
        Self {
            updater_exe,
            component,
            staged_path,
            relaunch_exe: None,
            relaunch_args: Vec::new(),
        }
    }

    /// 재시작 설정
    pub fn set_relaunch(&mut self, exe: PathBuf, args: Vec<String>) {
        self.relaunch_exe = Some(exe);
        self.relaunch_args = args;
    }

    /// 셀프 업데이트 실행
    /// 
    /// 이 함수 호출 후 현재 프로세스는 종료되어야 합니다.
    pub fn execute(&self) -> Result<(), String> {
        let mut cmd = Command::new(&self.updater_exe);
        
        cmd.arg("--cli")
            .arg("apply")
            .arg("--component")
            .arg(self.component.manifest_key())
            .arg("--staged")
            .arg(&self.staged_path);

        if let Some(ref relaunch_exe) = self.relaunch_exe {
            cmd.arg("--relaunch")
                .arg(relaunch_exe);
            
            for arg in &self.relaunch_args {
                cmd.arg("--relaunch-arg").arg(arg);
            }
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            // DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP
            cmd.creation_flags(0x00000008 | 0x01000000 | 0x00000200);
        }

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("업데이터 실행 실패: {}", e))?;

        tracing::info!("[SelfUpdate] Updater spawned, current process should exit now");
        Ok(())
    }
}

/// 프로세스 체커 — 특정 프로세스가 실행 중인지 확인
pub struct ProcessChecker;

impl ProcessChecker {
    /// 프로세스 이름으로 실행 중인지 확인
    #[cfg(target_os = "windows")]
    pub fn is_running(process_name: &str) -> bool {
        use std::process::Command;
        
        let output = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", process_name)])
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(process_name)
            }
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub fn is_running(process_name: &str) -> bool {
        use std::process::Command;
        
        let output = Command::new("pgrep")
            .arg("-x")
            .arg(process_name)
            .output();

        match output {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// 프로세스 종료 대기
    pub async fn wait_for_exit(process_name: &str, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            if !Self::is_running(process_name) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        false
    }

    /// 사바쨩 GUI가 실행 중인지 확인
    pub fn is_gui_running() -> bool {
        #[cfg(target_os = "windows")]
        {
            Self::is_running("saba-chan-gui.exe")
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self::is_running("saba-chan-gui")
        }
    }

    /// 사바쨩 CLI가 실행 중인지 확인
    pub fn is_cli_running() -> bool {
        #[cfg(target_os = "windows")]
        {
            Self::is_running("saba-chan-cli.exe")
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self::is_running("saba-chan-cli")
        }
    }

    /// 사바쨩 데몬이 실행 중인지 확인
    pub fn is_daemon_running() -> bool {
        #[cfg(target_os = "windows")]
        {
            Self::is_running("saba-chan.exe")
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self::is_running("saba-chan")
        }
    }
}
