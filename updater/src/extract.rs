//! 안전한 압축 해제와 파일 교체
//!
//! - 모든 엔트리 경로를 쓰기 전에 검증한다 (zip-slip, 절대 경로, 드라이브 문자, 심볼릭 링크 차단).
//! - `replace_existing` 모드에서는 기존 파일을 `<name>.old` 로 옮긴 뒤 새 파일을 쓴다.
//!   실패하면 새로 쓴 파일을 지우고 `.old` 를 되돌린다. 성공하면 `.old` 를 지운다
//!   (실행 중인 exe 처럼 지울 수 없는 파일은 다음 업데이트 때 덮어쓴다).

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// saba-chan 실행 파일 이름 (Unix 실행 권한 부여 대상)
const KNOWN_BINARIES: &[&str] = &[
    "saba-core",
    "saba-chan-cli",
    "saba-chan-gui",
    "saba-chan-updater",
    "saba-chan-installer",
];

pub(crate) fn is_zip(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "zip")
}

pub(crate) fn is_tar_gz(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".tar.gz"))
}

pub(crate) fn is_archive(path: &Path) -> bool {
    is_zip(path) || is_tar_gz(path)
}

/// `target_dir` 아래의 안전한 경로로 `name` 을 결합한다.
///
/// `..`, 절대 경로, 드라이브 문자/ADS(`:`), 경로상의 심볼릭 링크를 거부한다.
pub(crate) fn archive_entry_path(target_dir: &Path, name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    let relative = Path::new(&normalized);
    if normalized.is_empty() || normalized.contains(':') || relative.components().any(|part| {
        !matches!(part, std::path::Component::Normal(_) | std::path::Component::CurDir)
    }) {
        anyhow::bail!("Unsafe archive path: {}", name);
    }

    let mut path = target_dir.to_path_buf();
    for part in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(part) = part {
            path.push(part.as_os_str());
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("Archive path contains a symbolic link: {}", path.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

/// zip / tar.gz 를 `target_dir` 에 푼다. 그 외 형식은 거부한다.
///
/// `replace_existing` 이 true 이면 실패 시 기존 파일을 모두 복원한다.
pub(crate) fn extract_archive(staged: &Path, target_dir: &Path, replace_existing: bool) -> Result<()> {
    std::fs::create_dir_all(target_dir)?;
    let mut replacer = FileReplacer::new(replace_existing);
    let result = if is_zip(staged) {
        extract_zip(staged, target_dir, &mut replacer)
    } else if is_tar_gz(staged) {
        extract_tar_gz(staged, target_dir, &mut replacer)
    } else {
        Err(anyhow::anyhow!("Unsupported package format (expected .zip or .tar.gz): {}", staged.display()))
    };

    match result {
        Ok(()) => {
            replacer.commit();
            Ok(())
        }
        Err(error) => match replacer.rollback() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!("rollback failed: {:#}", rollback_error))),
        },
    }
}

/// 설치용: 압축 파일은 `target_dir` 에 풀고, 단일 파일은 `target_dir` 안에 복사한다.
pub(crate) fn extract_to_directory(staged: &Path, target_dir: &Path) -> Result<()> {
    if is_archive(staged) {
        return extract_archive(staged, target_dir, false);
    }
    let file_name = staged
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid staged file: {}", staged.display()))?;
    std::fs::create_dir_all(target_dir)?;
    std::fs::copy(staged, archive_entry_path(target_dir, &file_name.to_string_lossy())?)?;
    Ok(())
}

fn extract_zip(staged: &Path, target_dir: &Path, replacer: &mut FileReplacer) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(staged)?)?;

    // 1차: 모든 엔트리를 쓰기 전에 검증
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        archive_entry_path(target_dir, entry.name())?;
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            anyhow::bail!("Archive links are unsupported: {}", entry.name());
        }
    }

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let out_path = archive_entry_path(target_dir, entry.name())?;
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        replacer.prepare(&out_path)?;
        let mut outfile = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut outfile)
            .with_context(|| format!("extract {}", entry.name()))?;
        set_executable_if_binary(&out_path)?;
    }
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, target_dir: &Path, replacer: &mut FileReplacer) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let open = || -> Result<Archive<GzDecoder<std::fs::File>>> {
        Ok(Archive::new(GzDecoder::new(std::fs::File::open(archive_path)?)))
    };

    // 1차: 모든 엔트리를 쓰기 전에 검증 (일반 파일·디렉터리만 허용)
    for entry in open()?.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        archive_entry_path(target_dir, &path.to_string_lossy())?;
        let kind = entry.header().entry_type();
        if !kind.is_dir() && !kind.is_file() {
            anyhow::bail!("Unsupported tar entry: {}", path.display());
        }
    }

    for entry in open()?.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.into_owned();
        let out_path = archive_entry_path(target_dir, &entry_path.to_string_lossy())?;
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        replacer.prepare(&out_path)?;
        entry.unpack(&out_path)
            .with_context(|| format!("extract {}", entry_path.display()))?;
        set_executable_if_binary(&out_path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable_if_binary(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if is_known_binary(path) {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable_if_binary(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn is_known_binary(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| KNOWN_BINARIES.contains(&name))
}

/// 교체 중인 파일의 백업 경로 (`saba-core.exe` → `saba-core.exe.old`)
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".old");
    PathBuf::from(name)
}

/// 압축 해제 중 덮어쓴 파일을 추적하여 실패 시 되돌린다.
struct FileReplacer {
    enabled: bool,
    touched: HashSet<PathBuf>,
    /// (원래 경로, 백업 경로)
    backups: Vec<(PathBuf, PathBuf)>,
    /// 이전에 없던 새 파일
    created: Vec<PathBuf>,
}

impl FileReplacer {
    fn new(enabled: bool) -> Self {
        Self { enabled, touched: HashSet::new(), backups: Vec::new(), created: Vec::new() }
    }

    /// `path` 에 쓰기 직전에 호출한다. 기존 파일은 백업으로 옮긴다.
    fn prepare(&mut self, path: &Path) -> Result<()> {
        if !self.enabled || !self.touched.insert(path.to_path_buf()) {
            return Ok(());
        }
        if path.exists() {
            let backup = backup_path(path);
            // Windows: 실행 중인 exe 는 덮어쓸 수 없지만 rename 은 가능
            rename_with_retry(path, &backup, 5)?;
            self.backups.push((path.to_path_buf(), backup));
        } else {
            self.created.push(path.to_path_buf());
        }
        Ok(())
    }

    /// 새로 쓴 파일을 지우고 백업을 원래 위치로 복원한다.
    fn rollback(self) -> Result<()> {
        let mut errors = Vec::new();
        for path in &self.created {
            if let Err(e) = std::fs::remove_file(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    errors.push(format!("remove {}: {}", path.display(), e));
                }
            }
        }
        for (original, backup) in self.backups.iter().rev() {
            if original.exists() {
                if let Err(e) = std::fs::remove_file(original) {
                    errors.push(format!("remove {}: {}", original.display(), e));
                    continue;
                }
            }
            if let Err(e) = std::fs::rename(backup, original) {
                errors.push(format!("restore {}: {}", original.display(), e));
            } else {
                tracing::info!("[Updater] Restored {}", original.display());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("; "))
        }
    }

    /// 성공 시 백업 정리 (실행 중이라 지울 수 없는 파일은 남겨 둔다)
    fn commit(self) {
        for (_, backup) in &self.backups {
            if let Err(e) = std::fs::remove_file(backup) {
                tracing::debug!("[Updater] Cannot remove {} (still in use?): {}", backup.display(), e);
            }
        }
    }
}

/// Windows 에서 파일 잠금이 풀릴 때까지 지수 백오프로 rename 을 재시도한다.
pub(crate) fn rename_with_retry(from: &Path, to: &Path, max_retries: u32) -> Result<()> {
    // 이전 업데이트에서 남은 백업 파일 제거
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
        last_err.expect("at least one attempt")
    ))
}

/// 심볼릭 링크를 거부하며 디렉터리를 재귀 복사한다 (백업용).
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if std::fs::symlink_metadata(src)?.file_type().is_symlink() {
        anyhow::bail!("Cannot back up a symbolic link: {}", src.display());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            anyhow::bail!("Cannot back up a symbolic link: {}", path.display());
        }
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

/// 디렉터리 내용을 비운다. `__pycache__` 와 `.` 으로 시작하는 항목은 남긴다.
pub(crate) fn clean_dir(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "__pycache__" || name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}
