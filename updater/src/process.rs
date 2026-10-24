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
/// 시스템 폴더는 OS에 묻는다(`GetSystemDirectoryW`). 사용자가 바꿀 수 있는 SystemRoot·windir 환경변수는 보지 않는다.
pub fn system_tool(name: &str) -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        system_tool_in(system_directory(), name)
    }
    #[cfg(not(target_os = "windows"))]
    std::path::PathBuf::from(name)
}

#[cfg(target_os = "windows")]
fn system_directory() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 260];
    // SAFETY: buf는 길이를 함께 넘기는 쓰기 버퍼다. 0이나 길이 이상이 돌아오면 쓰지 않는다
    let len = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| std::ffi::OsString::from_wide(&buf[..len]).into())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn system_tool_in(sys: Option<std::path::PathBuf>, name: &str) -> std::path::PathBuf {
    let sys = sys.filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows\System32"));
    match name {
        "powershell" => sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
        _ => sys.join(format!("{name}.exe")),
    }
}

/// 프로세스 체커 — 특정 프로세스가 실행 중인지 확인
pub struct ProcessChecker;

impl ProcessChecker {
    /// 프로세스 이름(실행 파일 이름, 대소문자 무시)으로 실행 중인지 확인.
    /// 종료 대기에서 0.5초마다 부르므로 tasklist를 띄우지 않고 프로세스 목록을 OS에 직접 묻는다 (띄울 때마다 콘솔 창이 깜빡였다)
    #[cfg(target_os = "windows")]
    pub fn is_running(process_name: &str) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };

        // SAFETY: 스냅숏 핸들은 이 함수 안에서만 쓰고 닫는다. entry는 dwSize를 채운 뒤 넘긴다
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more && !found {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found = String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(process_name);
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        found
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

    /// PID로 실행 중인지 확인 (이름이 같은 다른 프로세스와 헷갈리지 않는다).
    /// Windows는 tasklist를 띄우지 않고 프로세스를 직접 연다. 열 권한이 없으면 살아 있는 것으로 본다
    #[cfg(target_os = "windows")]
    pub fn is_pid_running(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

        // SAFETY: 연 핸들은 이 함수 안에서만 쓰고 닫는다
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let alive = unsafe { GetExitCodeProcess(handle, &mut code) } != 0 && code == STILL_ACTIVE as u32;
        unsafe { CloseHandle(handle) };
        alive
    }

    #[cfg(not(target_os = "windows"))]
    pub fn is_pid_running(pid: u32) -> bool {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).output().map(|o| o.status.success()).unwrap_or(false)
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
        let sys = std::path::PathBuf::from(r"D:\Win\system32");
        assert_eq!(system_tool_in(Some(sys.clone()), "taskkill"), sys.join("taskkill.exe"));
        assert_eq!(system_tool_in(Some(sys.clone()), "tar"), sys.join("tar.exe"));
        assert_eq!(system_tool_in(Some(sys.clone()), "powershell"), sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
    }

    // 업데이트 중 종료 대기는 0.5초마다 확인한다. 확인마다 tasklist를 띄우면 그만큼 콘솔 창이 깜빡였다:
    // 프로세스 확인은 다른 프로그램을 띄우지 않고 OS에 직접 묻는다
    #[cfg(windows)]
    #[test]
    fn process_checks_ask_windows_directly() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_str().unwrap();
        assert!(ProcessChecker::is_running(name));
        // tasklist의 IMAGENAME 필터처럼 대소문자를 가리지 않는다
        assert!(ProcessChecker::is_running(&name.to_uppercase()));
        assert!(!ProcessChecker::is_running("saba-no-such-process.exe"));
        // 이름의 일부만 같은 것은 같은 프로세스가 아니다
        assert!(!ProcessChecker::is_running(&name[1..]));

        assert!(ProcessChecker::is_pid_running(std::process::id()));
        let mut child = std::process::Command::new(system_tool("cmd")).args(["/C", "exit"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        drop(child);
        assert!(!ProcessChecker::is_pid_running(pid));
    }

    // 시스템 폴더를 묻지 못했거나 빈 답이면 상대 경로가 되지 않게 기본 위치로 간다 (환경변수는 보지 않는다)
    #[test]
    fn an_unknown_system_folder_falls_back_to_the_default() {
        let default = std::path::PathBuf::from(r"C:\Windows\System32");
        assert_eq!(system_tool_in(None, "cmd"), default.join("cmd.exe"));
        assert_eq!(system_tool_in(Some(std::path::PathBuf::new()), "cmd"), default.join("cmd.exe"));
    }
}
