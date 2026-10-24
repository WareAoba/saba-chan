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
    /// 비밀값이 든 파일: 처음부터 주인만 읽고 쓴다 (Unix 0600, Windows는 폴더 ACL을 물려받는다)
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
    let _ = (path, mode);
    let mut file = options.open(tmp)?;
    file.write_all(contents)?;
    file.sync_all()
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
