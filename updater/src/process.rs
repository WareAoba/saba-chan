//! 이름·PID로 프로세스가 떠 있는지 본다 (적용 전 GUI·CLI 종료 대기)

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

    /// PID로 실행 중인지 확인 (이름이 같은 다른 프로세스와 헷갈리지 않는다)
    pub fn is_pid_running(pid: u32) -> bool {
        use std::process::Command;
        if cfg!(target_os = "windows") {
            Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().any(|w| w == pid.to_string()))
                .unwrap_or(false)
        } else {
            Command::new("kill").args(["-0", &pid.to_string()]).output().map(|o| o.status.success()).unwrap_or(false)
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
        crate::GUI_EXE_NAMES.iter().any(|n| Self::is_running(n))
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
}
