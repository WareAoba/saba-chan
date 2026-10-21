//! 이름·PID로 프로세스가 떠 있는지 본다 (적용 전 GUI·CLI 종료 대기)

/// 콘솔 프로그램(tasklist·taskkill·powershell 등)을 창 없이 띄운다. Windows가 아니면 아무것도 안 한다.
/// GUI 프로그램(인스톨러·업데이터)이 콘솔 프로그램을 부르면 Windows는 부를 때마다 새 콘솔 창을 연다.
pub fn hide_console(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Windows 시스템 도구(powershell·taskkill·tasklist·tar·cmd·icacls)를 띄울 경로. 다른 OS는 이름 그대로다.
/// 이름만 주면 실행 파일이 있는 폴더부터 찾으므로, 같은 이름의 다른 파일이 아니라 시스템 폴더의 것을 띄운다.
pub fn system_tool(name: &str) -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("windir"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        system_tool_in(&root, name)
    }
    #[cfg(not(target_os = "windows"))]
    std::path::PathBuf::from(name)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn system_tool_in(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let sys = root.join("System32");
    match name {
        "powershell" => sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
        _ => sys.join(format!("{name}.exe")),
    }
}

/// 프로세스 체커 — 특정 프로세스가 실행 중인지 확인
pub struct ProcessChecker;

impl ProcessChecker {
    /// 프로세스 이름으로 실행 중인지 확인
    #[cfg(target_os = "windows")]
    pub fn is_running(process_name: &str) -> bool {
        use std::process::Command;

        let output = hide_console(Command::new(system_tool("tasklist")).args(["/FI", &format!("IMAGENAME eq {}", process_name)])).output();

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
            hide_console(Command::new(system_tool("tasklist")).args(["/FI", &format!("PID eq {}", pid), "/NH"]))
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

#[cfg(test)]
mod tests {
    use super::*;

    // 시스템 도구는 이름으로 찾지 않고 Windows 시스템 폴더의 절대 경로로 띄운다
    #[test]
    fn system_tools_resolve_inside_the_windows_folder() {
        let root = std::path::Path::new(r"D:\Win");
        let sys = root.join("System32");
        assert_eq!(system_tool_in(root, "taskkill"), sys.join("taskkill.exe"));
        assert_eq!(system_tool_in(root, "tar"), sys.join("tar.exe"));
        assert_eq!(system_tool_in(root, "powershell"), sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
    }
}
