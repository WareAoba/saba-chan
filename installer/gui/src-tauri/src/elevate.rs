//! 관리자 권한 상승 (UAC) — 설치 폴더 만들기 전용
//!
//! 쓸 수 없는 설치 폴더는 자기 자신을 `--elevate-mkdir <폴더> --grant-sid <SID>`로 관리자 권한으로 다시 띄워 만들고,
//! 상승 전 사용자에게 그 폴더의 전체 권한을 준다 (`run`). 앱 전체를 재시작하지 않는다

use std::path::Path;
use saba_chan_installer_core::uninstall;

// ═══════════════════════════════════════════════════════
// 관리자 권한 상승 (UAC) — 디렉토리 생성 전용
// ═══════════════════════════════════════════════════════

/// Windows 명령줄 인자 하나로 따옴표를 씌운다 (CommandLineToArgvW 규칙: `"` 앞과 끝의 `\`는 두 배로)
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// UAC 프롬프트로 디렉토리를 생성하고 현재 사용자에게 쓰기 권한을 부여한다.
/// 앱 전체를 재시작하지 않고 디렉토리만 상승 처리한다.
/// ShellExecuteExW("runas")로 자기 자신을 --elevate-mkdir 모드로 실행하므로
/// UAC 대화상자에 "Windows PowerShell"이 아닌 인스톨러 이름이 표시된다.
#[cfg(target_os = "windows")]
#[cfg_attr(not(feature = "window"), allow(dead_code))]
pub(crate) fn elevate_create_dir(dir: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let self_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?;

    let verb = to_wide("runas");
    let file = to_wide(&self_exe.to_string_lossy());
    let sid = current_user_sid()?;
    let params = to_wide(&format!("--elevate-mkdir {} --grant-sid {}", quote_arg(&dir.to_string_lossy()), quote_arg(&sid)));

    let mut sei: SHELLEXECUTEINFOW = unsafe { zeroed() };
    sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.lpParameters = params.as_ptr();
    sei.nShow = 0; // SW_HIDE

    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 {
        return Err("UAC elevation was cancelled or failed".into());
    }

    // 프로세스 완료 대기 (최대 30초)
    if !sei.hProcess.is_null() {
        unsafe {
            WaitForSingleObject(sei.hProcess, 30_000);
            CloseHandle(sei.hProcess);
        }
    }

    if dir.exists() {
        // 쓰기 권한 확인
        let probe = dir.join(".saba-write-test");
        if std::fs::write(&probe, b"test").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Ok(());
        }
        return Err("Directory exists but write permission was not granted".into());
    }

    Err("Directory creation was cancelled or failed".into())
}

#[cfg(target_os = "windows")]
fn current_user_sid() -> Result<String, String> {
    use std::{os::windows::io::FromRawHandle, ptr::null_mut};
    use windows_sys::Win32::{Foundation::LocalFree, Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser}, Security::Authorization::ConvertSidToStringSidW, System::Threading::{GetCurrentProcess, OpenProcessToken}};
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 { return Err("Cannot open current process token".into()); }
        let _token = std::fs::File::from_raw_handle(token);
        let mut size = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        let mut data = vec![0u8; size as usize];
        if GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size) == 0 { return Err("Cannot read current user SID".into()); }
        let user = &*(data.as_ptr() as *const TOKEN_USER);
        let mut text = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 { return Err("Cannot format current user SID".into()); }
        let mut len = 0;
        while *text.add(len) != 0 { len += 1; }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(sid)
    }
}

fn valid_user_sid(sid: &str) -> bool {
    let mut parts = sid.split('-');
    if parts.next() != Some("S") || parts.next() != Some("1") { return false; }
    let decimal = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit());
    let Some(authority) = parts.next().filter(|v| decimal(v)).and_then(|v| v.parse::<u64>().ok()) else { return false };
    let subauthorities: Vec<_> = parts.collect();
    authority <= 0x0000_ffff_ffff_ffff
        && !subauthorities.is_empty()
        && subauthorities.len() <= 15
        && subauthorities.iter().all(|v| decimal(v) && v.parse::<u32>().is_ok())
}

/// 관리자 권한으로 만들 폴더 검사: 드라이브 절대 경로, `.`·`..` 없음, 두 단계 이상 깊이,
/// Windows 폴더 밖, 보호된 시스템 폴더(Program Files, ProgramData, Users, 사용자 폴더) 자체가 아님.
/// 이 폴더 전체에 사용자 전체 권한을 주므로 사바쨩 전용 하위 폴더만 받는다
fn check_elevated_target(dir: &Path) -> Result<(), String> {
    check_elevated_target_in(&dir.to_string_lossy(), &|name| std::env::var(name).ok())
}

fn check_elevated_target_in(dir: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let lower = norm(dir);
    let bytes = lower.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err("path must be an absolute drive path".into());
    }
    let parts: Vec<&str> = lower[3..].split('\\').collect();
    if parts.iter().any(|c| c.is_empty() || *c == "." || *c == "..") {
        return Err("path must not contain . or ..".into());
    }
    if parts.len() < 2 {
        return Err("refusing a drive root or a top-level folder".into());
    }
    if let Some(sys) = env("SystemRoot").or_else(|| env("WINDIR")).map(|v| norm(&v)) {
        if !sys.is_empty() && (lower == sys || lower.starts_with(&format!("{}\\", sys))) {
            return Err("refusing a path inside the Windows folder".into());
        }
    }
    let users = env("SystemDrive").map(|d| format!("{}\\Users", d));
    let protected = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "USERPROFILE", "PUBLIC"]
        .into_iter()
        .filter_map(env)
        .chain(users);
    if protected.map(|p| norm(&p)).any(|p| !p.is_empty() && p == lower) {
        return Err("refusing a protected system folder".into());
    }
    Ok(())
}

fn is_link_or_reparse_point(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// 권한을 줄 폴더의 내용 검사: 링크·정션을 거치지 않고, 아직 없거나, 비었거나, 사바쨩 설치 폴더(설치 표시나 실행 파일)여야 한다.
/// `icacls /T`가 하위 전체에 권한을 주므로 다른 파일이 든 기존 폴더는 받지 않는다
fn check_elevated_contents(dir: &Path) -> Result<(), String> {
    // 정션·심볼릭 링크를 따라가면 다른 폴더(System32 등)에 권한이 걸린다: 자신과 위 폴더 모두 확인
    if dir.ancestors().any(is_link_or_reparse_point) {
        return Err("refusing a path through a link or junction".into());
    }
    if !dir.exists()
        || saba_chan_installer_core::is_empty_dir(dir)
        || dir.join(uninstall::INSTALL_MARKER).is_file()
        || uninstall::looks_like_install_dir(dir)
    {
        Ok(())
    } else {
        Err("refusing an existing folder that is not a Saba-chan install".into())
    }
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(feature = "window"), allow(dead_code))]
pub(crate) fn elevate_create_dir(_dir: &Path) -> Result<(), String> {
    Err("Elevation is only supported on Windows".to_string())
}

/// `--elevate-mkdir <path> --grant-sid <sid>`: UAC 상승된 상태에서 폴더를 만들고 권한을 준 뒤의 종료 코드.
/// `elevate_create_dir`가 ShellExecuteW("runas")로 자기 자신을 다시 띄울 때 들어온다
pub(crate) fn run(args: &[String]) -> i32 {
    let pos = args.iter().position(|a| a == "--elevate-mkdir").unwrap_or(0);
    if let Some(dir_str) = args.get(pos + 1) {
        let dir = Path::new(dir_str);
        if let Err(e) = check_elevated_target(dir).and_then(|_| check_elevated_contents(dir)) {
            eprintln!("elevate-mkdir: {}", e);
            return 1;
        }
        // 디렉토리 생성
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("elevate-mkdir: failed to create dir: {}", e);
            return 1;
        }
        // 상승 전 사용자에게 전체 권한 부여
        let Some(sid) = args.iter().position(|a| a == "--grant-sid").and_then(|i| args.get(i + 1)).cloned().filter(|s| valid_user_sid(s)) else {
            eprintln!("elevate-mkdir: missing or invalid user SID");
            return 1;
        };
        if let Err(e) = check_elevated_target(dir).and_then(|_| check_elevated_contents(dir)) {
            eprintln!("elevate-mkdir: {}", e);
            return 1;
        }
        let icacls = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new(saba_chan_updater_lib::process::system_tool("icacls")))
            .args([
                dir_str.as_str(),
                "/grant",
                &format!("*{}:(OI)(CI)F", sid),
                "/Q",
            ])
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
    } else {
        eprintln!("elevate-mkdir: missing path argument");
        1
    }
}

#[cfg(test)]
mod tests {
    // L15(a): `\`로 끝나는 경로는 그대로 따옴표로 감싸면 `\"`가 되어 다음 인자까지 먹는다
    #[test]
    fn elevation_arguments_survive_trailing_backslashes_and_quotes() {
        assert_eq!(quote_arg(r"C:\Program Files\Saba-chan"), r#""C:\Program Files\Saba-chan""#);
        assert_eq!(quote_arg(r"D:\"), r#""D:\\""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }

    use super::*;

    #[test]
    fn elevation_accepts_only_numeric_user_sids() {
        assert!(valid_user_sid("S-1-5-21-123-456-789-1001"));
        assert!(!valid_user_sid("DOMAIN\\user"));
        assert!(!valid_user_sid("S-1-5-21-123-x"));
        assert!(!valid_user_sid("S-1-5-21-4294967296"));
    }

    /// 맥의 임시 폴더(/var/folders)는 링크(/var → /private/var) 아래다: 링크를 거절하는 검사라 실제 경로로 시험한다
    fn real_temp_dir() -> std::path::PathBuf {
        let temp = std::env::temp_dir();
        if cfg!(unix) { std::fs::canonicalize(&temp).unwrap() } else { temp }
    }

    #[test]
    fn elevation_grants_only_on_a_new_empty_or_saba_chan_folder() {
        let root = real_temp_dir().join(format!("saba-elevate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(check_elevated_contents(&root.join("new")).is_ok(), "a folder that does not exist yet");
        assert!(check_elevated_contents(&root).is_ok(), "an empty folder");

        std::fs::write(root.join("report.docx"), b"").unwrap();
        assert!(check_elevated_contents(&root).is_err(), "a folder with someone else's files");

        std::fs::write(root.join(uninstall::INSTALL_MARKER), b"").unwrap();
        assert!(check_elevated_contents(&root).is_ok(), "a folder the installer made");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn elevation_refuses_a_link_or_a_folder_below_a_link() {
        let root = real_temp_dir().join(format!("saba-elevate-link-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        assert!(check_elevated_contents(&root.join("link")).is_err(), "the folder itself is a link");
        assert!(check_elevated_contents(&root.join("link").join("Saba-chan")).is_err(), "a parent is a link");
        assert!(check_elevated_contents(&root.join("real").join("Saba-chan")).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn elevation_only_creates_a_dedicated_folder_below_a_parent() {
        let env = |name: &str| match name {
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_string()),
            "ProgramData" => Some(r"C:\ProgramData".to_string()),
            "SystemDrive" => Some("C:".to_string()),
            "USERPROFILE" => Some(r"C:\Users\me".to_string()),
            _ => None,
        };
        for refused in [
            r"C:\Program Files",
            r"c:\program files\",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users",
            r"C:\Users\me",
            r"C:\Windows\Saba-chan",
            r"C:\Games",
            r"C:\",
            r"C:\Program Files\..\Users\x",
            r"Saba-chan",
        ] {
            assert!(check_elevated_target_in(refused, &env).is_err(), "{refused} must be refused");
        }
        for allowed in [r"C:\Program Files\Saba-chan", r"D:\Games\Saba-chan", r"C:\Users\me\Saba-chan"] {
            assert!(check_elevated_target_in(allowed, &env).is_ok(), "{allowed} must be allowed");
        }
    }
}
