//! 실행 중인 프로세스 확인 (업데이트 적용 전 종료 대기용)

/// 프로세스 체커 — 특정 프로세스가 실행 중인지 확인
pub struct ProcessChecker;

impl ProcessChecker {
    /// 프로세스 이름으로 실행 중인지 확인 (`tasklist`, 콘솔 창을 띄우지 않음)
    #[cfg(target_os = "windows")]
    pub fn is_running(process_name: &str) -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", process_name), "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(process_name))
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn is_running(process_name: &str) -> bool {
        std::process::Command::new("pgrep")
            .arg("-x")
            .arg(process_name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 프로세스 종료 대기. 제한 시간 안에 종료되면 true.
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
}
