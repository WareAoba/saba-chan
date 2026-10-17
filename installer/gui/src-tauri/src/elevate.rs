//! 관리자 권한 상승 (UAC) — 설치 디렉토리 생성 전용
//!
//! 일반 권한 인스톨러가 설치 경로에 쓸 수 없으면 자기 자신을
//! `--elevate-mkdir <dir> --grant-sid <sid>` 로 "runas" 재실행한다.
//! 상승된 프로세스는 디렉토리를 만들고, **원래 사용자 SID** 에게만 권한을 준다.
//! (UAC 자격 증명 입력 시 상승 프로세스의 USERNAME 은 관리자 계정이므로 쓰지 않는다.)

use std::path::{Path, PathBuf};

pub const ELEVATE_MKDIR_ARG: &str = "--elevate-mkdir";
pub const GRANT_SID_ARG: &str = "--grant-sid";

const WRITE_PROBE_NAME: &str = ".saba-write-test";

/// 설치 디렉토리를 만들고 쓰기 가능한지 확인한다.
///
/// - `Ok(false)`: 쓰기 가능
/// - `Ok(true)`: 권한 부족 — 상승이 필요
/// - `Err`: 권한 이외의 실패 (경로 오류, 디스크 등). 상승으로 해결되지 않으므로 그대로 알린다.
pub fn install_dir_needs_elevation(dir: &Path) -> Result<bool, String> {
    use std::io::ErrorKind::PermissionDenied;

    match std::fs::create_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == PermissionDenied => return Ok(true),
        Err(e) => {
            return Err(format!(
                "Failed to create install directory {}: {}",
                dir.display(),
                e
            ))
        }
    }
    let probe = dir.join(WRITE_PROBE_NAME);
    match std::fs::write(&probe, b"test") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(false)
        }
        Err(e) if e.kind() == PermissionDenied => Ok(true),
        Err(e) => Err(format!(
            "Install directory {} is not writable: {}",
            dir.display(),
            e
        )),
    }
}

/// `--elevate-mkdir` 인자로 받은 경로를 검증한다.
///
/// ## 위협 모델
/// UAC 상승 후 `icacls /grant <sid>:(OI)(CI)F /T`를 수행하므로, 공격자가 UAC 동의를 얻어내면
/// 임의 디렉토리에 대해 재귀적 전체 권한을 부여할 수 있다.
/// 인스톨러의 정상 사용 시나리오(사용자 설치 경로 생성)에서만 동작하도록 제한한다.
///
/// ## 허용 규칙
/// - 절대 경로만 허용
/// - 경로 내 `..` 컴포넌트 금지 (symlink/traversal 회피)
/// - 루트 레벨(예: `C:\`) 직접 지정 금지 — 최소 2단계 이상
/// - 시스템 보호 디렉토리(Windows/Users\Default 등) 하위 금지
pub fn validate_elevate_mkdir_path(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);

    if !path.is_absolute() {
        return Err(format!("path must be absolute: {}", raw));
    }

    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("path must not contain '..': {}", raw));
    }

    // 시스템 보호 디렉토리 차단 (shallow 체크보다 먼저 — 더 구체적인 에러 메시지)
    #[cfg(windows)]
    {
        let protected = [
            r"C:\Windows",
            r"C:\Users\Default",
            r"C:\Users\All Users",
            r"C:\ProgramData\Microsoft",
        ];
        let raw_upper = raw.to_ascii_uppercase().replace('/', "\\");
        for p in &protected {
            let p_upper = p.to_ascii_uppercase();
            if raw_upper == p_upper || raw_upper.starts_with(&format!("{}\\", p_upper)) {
                return Err(format!("path is under a protected system directory: {}", raw));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let protected = ["/etc", "/bin", "/sbin", "/usr", "/boot", "/proc", "/sys"];
        for p in &protected {
            if raw == *p || raw.starts_with(&format!("{}/", p)) {
                return Err(format!("path is under a protected system directory: {}", raw));
            }
        }
    }

    // 루트만 있는 경로 차단
    // Windows: C:\Games\app = [Prefix, RootDir, Normal, Normal] → normal=2
    // Unix:    /home/x = [RootDir, Normal, Normal] → normal=2
    let normal_components = path
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();
    if normal_components < 2 {
        return Err(format!("path is too shallow (root or one-level): {}", raw));
    }

    Ok(path)
}

/// 문자열 SID 형식(`S-1-5-21-...`)인지 확인한다. icacls 인자로 넘기기 전 검증용.
pub fn is_valid_sid(sid: &str) -> bool {
    let Some(rest) = sid.strip_prefix("S-1-") else {
        return false;
    };
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Windows 명령줄 규칙(CommandLineToArgvW)에 맞게 인자 하나를 인용한다.
/// 끝이 `\` 인 경로도 닫는 따옴표를 삼키지 않는다.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// 상승 프로세스에 넘길 매개변수 문자열
pub fn elevate_mkdir_params(dir: &Path, sid: &str) -> String {
    [ELEVATE_MKDIR_ARG, &dir.to_string_lossy(), GRANT_SID_ARG, sid]
        .iter()
        .map(|a| quote_windows_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 현재 프로세스 사용자의 문자열 SID
#[cfg(windows)]
pub fn current_user_sid() -> Result<String, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("OpenProcessToken failed".into());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        // TOKEN_USER 는 포인터 정렬이 필요하므로 u64 버퍼를 쓴다.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err("GetTokenInformation(TokenUser) failed".into());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut sid_ptr: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_ptr) == 0 || sid_ptr.is_null() {
            return Err("ConvertSidToStringSidW failed".into());
        }
        let n = (0..).take_while(|&i| *sid_ptr.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_ptr, n));
        LocalFree(sid_ptr.cast());
        Ok(sid)
    }
}

/// UAC 프롬프트로 디렉토리를 생성하고 현재 사용자에게 쓰기 권한을 부여한다.
/// ShellExecuteExW("runas")로 자기 자신을 `--elevate-mkdir` 모드로 실행하므로
/// UAC 대화상자에 "Windows PowerShell"이 아닌 인스톨러 이름이 표시된다.
#[cfg(windows)]
pub fn elevate_create_dir(dir: &Path) -> Result<(), String> {
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    fn to_wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    let self_exe =
        std::env::current_exe().map_err(|e| format!("Failed to get current exe path: {}", e))?;
    let sid = current_user_sid()?;

    let verb = to_wide("runas".as_ref());
    let file = to_wide(self_exe.as_os_str());
    let params = to_wide(elevate_mkdir_params(dir, &sid).as_ref());

    let mut sei: SHELLEXECUTEINFOW = unsafe { zeroed() };
    sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.lpParameters = params.as_ptr();
    sei.nShow = 0; // SW_HIDE

    if unsafe { ShellExecuteExW(&mut sei) } == 0 {
        return Err("UAC elevation was cancelled or failed".into());
    }

    // 프로세스 완료 대기 (최대 30초)
    if !sei.hProcess.is_null() {
        unsafe {
            WaitForSingleObject(sei.hProcess, 30_000);
            CloseHandle(sei.hProcess);
        }
    }

    match install_dir_needs_elevation(dir) {
        Ok(false) => Ok(()),
        Ok(true) => Err("Directory exists but write permission was not granted".into()),
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
pub fn elevate_create_dir(_dir: &Path) -> Result<(), String> {
    Err("Elevation is only supported on Windows".to_string())
}

/// 상승된 프로세스 진입점. 프로세스 종료 코드를 반환한다.
pub fn run_elevated_mkdir(args: &[String]) -> i32 {
    let arg_after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let Some(dir_str) = arg_after(ELEVATE_MKDIR_ARG) else {
        eprintln!("elevate-mkdir: missing path argument");
        return 1;
    };
    let dir = match validate_elevate_mkdir_path(dir_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("elevate-mkdir: invalid path: {}", e);
            return 2;
        }
    };
    // 권한은 원래 사용자 SID 에게만 준다. 없으면 실패 — Users 그룹 전체 권한 폴백 금지.
    let sid = match arg_after(GRANT_SID_ARG) {
        Some(s) if is_valid_sid(s) => s,
        _ => {
            eprintln!("elevate-mkdir: missing or invalid {}", GRANT_SID_ARG);
            return 2;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("elevate-mkdir: failed to create dir: {}", e);
        return 1;
    }
    let icacls = std::process::Command::new("icacls")
        .arg(&dir)
        .args(["/grant", &format!("*{}:(OI)(CI)F", sid), "/T", "/Q"])
        .output();
    match icacls {
        Ok(out) if out.status.success() => 0,
        Ok(out) => {
            eprintln!(
                "elevate-mkdir: icacls failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            1
        }
        Err(e) => {
            eprintln!("elevate-mkdir: icacls spawn error: {}", e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_dir_does_not_need_elevation() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("new").join("install");
        assert_eq!(install_dir_needs_elevation(&dir), Ok(false));
        assert!(dir.is_dir());
        assert!(!dir.join(WRITE_PROBE_NAME).exists());
    }

    #[test]
    fn non_permission_failure_is_reported_not_elevated() {
        // 부모 경로가 파일이면 권한 문제가 아니므로 상승이 아니라 실제 에러를 보여야 한다.
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let result = install_dir_needs_elevation(&blocker.join("install"));
        assert!(result.is_err(), "got {:?}", result);
    }

    #[test]
    fn sid_validation() {
        assert!(is_valid_sid("S-1-5-21-1004336348-1177238915-682003330-512"));
        assert!(is_valid_sid("S-1-5-32"));
        assert!(!is_valid_sid("Users"));
        assert!(!is_valid_sid("S-1-5-21-abc"));
        assert!(!is_valid_sid("S-1-5-21:(OI)(CI)F"));
        assert!(!is_valid_sid("S-1-"));
    }

    #[test]
    fn quote_leaves_simple_args_alone() {
        assert_eq!(quote_windows_arg("--grant-sid"), "--grant-sid");
        assert_eq!(quote_windows_arg(r"C:\Games\saba\"), r"C:\Games\saba\");
        assert_eq!(quote_windows_arg(""), "\"\"");
    }

    #[test]
    fn quote_doubles_trailing_backslashes_inside_quotes() {
        assert_eq!(
            quote_windows_arg(r"C:\Program Files\Saba-chan\"),
            r#""C:\Program Files\Saba-chan\\""#
        );
        assert_eq!(quote_windows_arg(r#"a "b""#), r#""a \"b\"""#);
    }

    #[cfg(windows)]
    fn parse_windows_cmdline(cmdline: &str) -> Vec<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let wide: Vec<u16> = format!("prog.exe {}", cmdline)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut argc = 0i32;
        unsafe {
            let argv = CommandLineToArgvW(wide.as_ptr(), &mut argc);
            assert!(!argv.is_null());
            let args = (1..argc as usize)
                .map(|i| {
                    let p = *argv.add(i);
                    let n = (0..).take_while(|&j| *p.add(j) != 0).count();
                    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
                })
                .collect();
            LocalFree(argv.cast());
            args
        }
    }

    #[cfg(windows)]
    #[test]
    fn old_manual_quoting_breaks_trailing_backslash_path() {
        let dir = r"C:\Program Files\Saba-chan\";
        let old = format!("--elevate-mkdir \"{}\"", dir);
        assert_ne!(parse_windows_cmdline(&old), vec!["--elevate-mkdir", dir]);
    }

    #[cfg(windows)]
    #[test]
    fn elevate_params_round_trip_through_windows_parser() {
        let sid = "S-1-5-21-1-2-3-1001";
        for dir in [
            r"C:\Program Files\Saba-chan\",
            r"C:\Program Files\Saba-chan",
            r"D:\Games\saba",
            r"D:\it's a \\dir\\",
            r#"D:\odd"name"#,
        ] {
            let params = elevate_mkdir_params(Path::new(dir), sid);
            assert_eq!(
                parse_windows_cmdline(&params),
                vec![ELEVATE_MKDIR_ARG, dir, GRANT_SID_ARG, sid],
                "params={}",
                params
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn current_user_sid_is_valid() {
        let sid = current_user_sid().unwrap();
        assert!(is_valid_sid(&sid), "{}", sid);
    }

    #[test]
    fn elevated_mkdir_refuses_without_sid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b");
        let args = vec![
            "installer".to_string(),
            ELEVATE_MKDIR_ARG.to_string(),
            dir.to_string_lossy().to_string(),
        ];
        assert_eq!(run_elevated_mkdir(&args), 2);
        assert!(!dir.exists());
    }
}
