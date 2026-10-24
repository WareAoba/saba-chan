//! 파일 하나를 통째로 바꾸는 저장. 같은 폴더의 임시 파일에 다 쓰고 디스크에 내린 뒤 이름을 바꾼다.
//! 쓰다 꺼져도 예전 파일이나 새 파일 중 하나가 온전히 남고, 다른 쪽이 반쯤 쓴 파일을 읽지 않는다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 새 파일의 권한
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 있던 파일의 권한을 잇는다 (없으면 기본값)
    Plain,
    /// 비밀값이 든 파일: 처음부터 주인만 읽고 쓴다 (Unix 0600, Windows는 폴더 ACL을 잇지 않는 보호 DACL)
    Secret,
}

/// `path`를 `contents`로 바꾼다. 폴더가 없으면 만든다
pub fn write(path: &Path, contents: &[u8], mode: Mode) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = temp_path(dir, path);
    let result = write_temp(&tmp, path, contents, mode).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    sync_dir(dir);
    Ok(())
}

/// 같은 프로세스·다른 프로세스가 동시에 써도 겹치지 않는 임시 이름
fn temp_path(dir: &Path, path: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{}.{}-{}.tmp", name, std::process::id(), seq))
}

fn write_temp(tmp: &Path, path: &Path, contents: &[u8], mode: Mode) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        match mode {
            Mode::Secret => {
                options.mode(0o600);
            }
            Mode::Plain => {
                if let Ok(meta) = std::fs::metadata(path) {
                    options.mode(meta.permissions().mode() & 0o7777);
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    let mut file = options.open(tmp)?;
    // 이름을 바꿔도 임시 파일의 ACL이 그대로 간다: 폴더에 준 격리 실행 권한이 비밀 파일로 상속되지 않게 미리 끊는다
    #[cfg(windows)]
    if mode == Mode::Secret {
        restrict_to_user(tmp)?;
    }
    file.write_all(contents)?;
    file.sync_all()
}

/// 폴더·파일의 DACL을 상속 없이 현재 사용자·SYSTEM·Administrators 전체 권한으로 바꾼다 (Windows).
/// 폴더면 안의 항목도 이 권한을 물려받는다. 다른 계정이나 격리 실행에 준 권한이 위에서 내려오지 않는다
#[cfg(windows)]
pub fn restrict_to_user(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{LocalFree, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, OPEN_EXISTING, READ_CONTROL, WRITE_DAC};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut needed = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut needed);
        let mut info = vec![0u8; needed as usize];
        if GetTokenInformation(token.as_raw_handle(), TokenUser, info.as_mut_ptr().cast(), needed, &mut needed) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let user = std::ptr::read_unaligned(info.as_ptr() as *const TOKEN_USER);
        let mut sid = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let user_sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)").encode_utf16().chain(Some(0)).collect();
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, null_mut()) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let (mut present, mut defaulted, mut dacl) = (0, 0, null_mut());
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) == 0 || present == 0 {
            let error = std::io::Error::last_os_error();
            LocalFree(descriptor);
            return Err(error);
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = CreateFileW(wide.as_ptr(), WRITE_DAC | READ_CONTROL, 7, null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, null_mut());
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            LocalFree(descriptor);
            return Err(error);
        }
        let handle = OwnedHandle::from_raw_handle(handle);
        let rc = SetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        );
        LocalFree(descriptor);
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc as i32));
        }
    }
    Ok(())
}

/// Unix에는 상속 ACL이 없다: 할 일이 없다
#[cfg(not(windows))]
pub fn restrict_to_user(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// 이름 바꾸기까지 디스크에 남긴다 (Unix만. 실패해도 저장은 끝난 것이다)
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn replaces_the_whole_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(&path, b"a much longer old content").unwrap();

        write(&path, b"new", Mode::Plain).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn creates_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x/y/z.json");

        write(&path, b"{}", Mode::Plain).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
    }

    #[test]
    fn a_failed_rename_keeps_the_old_file_and_removes_the_temp() {
        let dir = tempfile::tempdir().unwrap();
        // 폴더 자리에는 파일로 바꿔 넣을 수 없다
        let path = dir.path().join("taken");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), b"kept").unwrap();

        assert!(write(&path, b"new", Mode::Plain).is_err());

        assert_eq!(std::fs::read(path.join("inside")).unwrap(), b"kept");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn concurrent_writers_never_leave_a_mixed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.json");
        let bodies: Vec<Vec<u8>> = (0..8u8).map(|i| vec![b'a' + i; 64 * 1024]).collect();
        std::thread::scope(|s| {
            for body in &bodies {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..20 {
                        write(path, body, Mode::Plain).unwrap();
                    }
                });
            }
        });

        let last = std::fs::read(&path).unwrap();
        assert!(bodies.contains(&last), "file is one writer's whole content");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn secret_files_are_owner_only_and_plain_files_keep_their_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("token");
        std::fs::write(&secret, b"old").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        let plain = dir.path().join("server.properties");
        std::fs::write(&plain, b"old").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o640)).unwrap();

        write(&secret, b"new", Mode::Secret).unwrap();
        write(&plain, b"new", Mode::Plain).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&secret), 0o600);
        assert_eq!(mode(&plain), 0o640);
    }
}
