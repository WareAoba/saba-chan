//! 월드·세이브 백업 (A2)
//!
//! 모듈 `[backup] paths`가 가리키는 폴더·파일을 `<인스턴스 폴더>/backups/<시각>-<종류>.zip` 하나로 묶는다.
//! zip 안에는 `saba-backup.json`(항목 번호 → 원래 절대 경로)과 `<번호>/...` 파일이 들어 있어
//! 복원 때 원래 자리로 돌려놓는다. 이 파일의 함수는 모두 블로킹이다 (호출자가 spawn_blocking).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "saba-backup.json";

/// 백업 종류. `Manual`만 정리(prune)에서 빠진다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupKind {
    Manual,
    Scheduled,
    PreUpdate,
    PreRestore,
}

impl BackupKind {
    fn as_str(self) -> &'static str {
        match self {
            BackupKind::Manual => "manual",
            BackupKind::Scheduled => "scheduled",
            BackupKind::PreUpdate => "pre-update",
            BackupKind::PreRestore => "pre-restore",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".zip")?;
        [Self::Manual, Self::Scheduled, Self::PreUpdate, Self::PreRestore]
            .into_iter()
            .find(|k| stem.ends_with(&format!("-{}", k.as_str())))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub name: String,
    pub kind: BackupKind,
    /// 유닉스 시각 (초)
    pub created: u64,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    paths: Vec<ManifestEntry>,
}

#[derive(Serialize, Deserialize)]
struct ManifestEntry {
    entry: String,
    path: PathBuf,
    is_file: bool,
}

/// 백업 폴더
pub fn backups_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join("backups")
}

/// `[backup] paths` 패턴을 실제 경로로. `{설정 이름}`은 `lookup`으로 바꾸고(값이 비면 그 경로는 뺀다),
/// 상대 경로는 `base`(서버 작업 폴더) 기준이다. 지금 없는 경로도 남긴다(복원이 되살릴 수 있는 범위). 백업은 있는 것만 담는다
pub fn fill_paths(patterns: &[String], base: &Path, lookup: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let placeholder = regex::Regex::new(r"\{([A-Za-z0-9_]+)\}").expect("valid regex");
    let mut out: Vec<PathBuf> = Vec::new();
    for pattern in patterns {
        let mut missing = false;
        let filled = placeholder.replace_all(pattern, |c: &regex::Captures| {
            match lookup(&c[1]).filter(|v| !v.is_empty()) {
                Some(v) => v,
                None => {
                    missing = true;
                    String::new()
                }
            }
        });
        if missing {
            continue;
        }
        let path = base.join(filled.as_ref());
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// 새 백업을 만든다. 같은 초에 두 번 만들면 이름 뒤에 번호를 붙인다
pub fn create(dir: &Path, kind: BackupKind, paths: &[PathBuf]) -> Result<BackupInfo> {
    if paths.is_empty() {
        bail!("nothing to back up: none of the module's backup paths exist");
    }
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    // 밀리초까지 넣어 이름순이 곧 만든 순서가 되게 한다 (같은 초에 만든 복원 전 백업이 더 오래돼 보이지 않게)
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let name = unique_name(dir, &stamp, kind);
    let final_path = dir.join(&name);
    let partial = dir.join(format!("{}.partial", name));
    let written = write_zip(&partial, paths);
    if let Err(e) = written {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    fs::rename(&partial, &final_path)?;
    info(&final_path).context("read new backup")
}

/// `stamp`로 시작하는 백업이 이미 있으면(종류가 달라도) 번호를 붙인다: 이름순이 만든 순서가 되게
fn unique_name(dir: &Path, stamp: &str, kind: BackupKind) -> String {
    let taken: Vec<String> = fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    let used = |s: &str| taken.iter().any(|name| name.starts_with(&format!("{}-", s)));
    let mut candidate = stamp.to_string();
    let mut n = 2;
    while used(&candidate) {
        // 두 자리로 채워 10번째도 9번째 뒤에 온다
        candidate = format!("{}{:02}", stamp, n);
        n += 1;
    }
    format!("{}-{}.zip", candidate, kind.as_str())
}

fn write_zip(target: &Path, paths: &[PathBuf]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(target)?);
    let opts = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut manifest = Manifest { version: 1, paths: Vec::new() };
    for (i, path) in paths.iter().enumerate() {
        let entry = i.to_string();
        let is_file = path.is_file();
        if is_file {
            zip.start_file(&entry, opts)?;
            io::copy(&mut File::open(path)?, &mut zip)?;
        } else {
            zip.add_directory(format!("{}/", entry), opts)?;
            add_dir(&mut zip, path, &entry, opts)?;
        }
        manifest.paths.push(ManifestEntry { entry, path: path.clone(), is_file });
    }
    zip.start_file(MANIFEST, opts)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;
    zip.finish()?.sync_all()?;
    Ok(())
}

fn add_dir(zip: &mut zip::ZipWriter<File>, dir: &Path, prefix: &str, opts: zip::write::FileOptions) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = format!("{}/{}", prefix, item.file_name().to_string_lossy());
        let kind = item.file_type()?;
        if kind.is_dir() {
            zip.add_directory(format!("{}/", name), opts)?;
            add_dir(zip, &item.path(), &name, opts)?;
        } else if kind.is_file() {
            zip.start_file(&name, opts)?;
            // 서버가 쓰는 중이라 잠긴 파일(Windows)은 건너뛰지 않고 실패로 알린다: 반쪽 백업을 믿지 않게
            io::copy(&mut File::open(item.path()).with_context(|| format!("read {}", item.path().display()))?, zip)?;
        }
    }
    Ok(())
}

fn info(path: &Path) -> Option<BackupInfo> {
    let name = path.file_name()?.to_str()?.to_string();
    let kind = BackupKind::from_name(&name)?;
    let meta = fs::metadata(path).ok()?;
    let created = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(BackupInfo { name, kind, created, size: meta.len() })
}

/// 새것부터
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let mut out: Vec<BackupInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| info(&e.path()))
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// API로 받은 이름이 백업 폴더 안의 백업 파일만 가리키게
fn checked(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.contains(['/', '\\']) || name.starts_with('.') || BackupKind::from_name(name).is_none() {
        bail!("invalid backup name: {}", name);
    }
    let path = dir.join(name);
    if !path.is_file() {
        bail!("backup not found: {}", name);
    }
    Ok(path)
}

pub fn delete(dir: &Path, name: &str) -> Result<()> {
    fs::remove_file(checked(dir, name)?)?;
    Ok(())
}

/// 자동 백업(수동 제외)을 새것 `keep`개만 남긴다. `protect`는 넘쳐도 지우지 않는다 (방금 만든 백업, 복원 중인 백업). 지운 이름을 돌려준다
pub fn prune(dir: &Path, keep: usize, protect: &[&str]) -> Vec<String> {
    // 종류마다 따로 센다: 업데이트 전·복원 전 안전 백업이 예약 백업을 밀어내지 않게
    let mut seen: std::collections::HashMap<BackupKind, usize> = std::collections::HashMap::new();
    list(dir)
        .into_iter()
        .filter(|b| b.kind != BackupKind::Manual)
        .filter(|b| {
            let n = seen.entry(b.kind).or_default();
            *n += 1;
            *n > keep
        })
        .filter(|b| !protect.contains(&b.name.as_str()))
        .filter(|b| fs::remove_file(dir.join(&b.name)).is_ok())
        .map(|b| b.name)
        .collect()
}

/// 백업이 돌려놓을 경로 (백업할 때의 절대 경로)
pub fn backed_up_paths(dir: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let mut zip = zip::ZipArchive::new(File::open(checked(dir, name)?)?)?;
    let manifest: Manifest = serde_json::from_reader(zip.by_name(MANIFEST).context("not a saba-chan backup")?)?;
    Ok(manifest.paths.into_iter().map(|e| e.path).collect())
}

/// `path`가 `roots` 중 하나의 안(그 자체 포함)이고 `..`가 없는가 (글자 경로로 본다)
pub fn is_inside(path: &Path, roots: &[PathBuf]) -> bool {
    !path.components().any(|c| matches!(c, std::path::Component::ParentDir)) && roots.iter().any(|r| path.starts_with(r))
}

/// 백업을 원래 자리로 돌려놓는다. 서버는 꺼져 있어야 한다 (호출자가 확인).
/// 경로마다 기존 것을 옆으로 치워 두고 풀어 넣는다. 치워 둔 것은 모두 성공한 뒤에 지우고,
/// 도중에 실패하면 이미 풀어 넣은 경로까지 거꾸로 모두 되돌린다
pub fn restore(dir: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let mut zip = zip::ZipArchive::new(File::open(checked(dir, name)?)?)?;
    let manifest: Manifest = serde_json::from_reader(zip.by_name(MANIFEST).context("not a saba-chan backup")?)?;
    // (경로, 기존 것을 치워 둔 곳). 실패하면 이 목록을 거꾸로 되돌린다
    let mut touched: Vec<(&Path, Option<PathBuf>)> = Vec::new();
    let mut failed = None;
    for entry in &manifest.paths {
        let aside = entry.path.exists().then(|| free_aside_path(&entry.path));
        if let Some(aside) = &aside {
            if let Err(e) = fs::rename(&entry.path, aside) {
                failed = Some(anyhow::Error::from(e).context(format!("move aside {}", entry.path.display())));
                break;
            }
        }
        touched.push((&entry.path, aside));
        if let Err(e) = extract_entry(&mut zip, entry) {
            failed = Some(e.context(format!("restore {}", entry.path.display())));
            break;
        }
    }
    if let Some(e) = failed {
        // 되돌리지 못한 경로는 오류에 붙인다 (원래 내용은 `.saba-restore-old`와 복원 전 안전 백업에 남아 있다)
        let mut stuck = Vec::new();
        for (path, aside) in touched.into_iter().rev() {
            remove_any(path);
            if let Some(aside) = aside {
                if let Err(r) = fs::rename(&aside, path) {
                    tracing::warn!("Could not put {} back after a failed restore: {}", path.display(), r);
                    stuck.push(aside.display().to_string());
                }
            }
        }
        if !stuck.is_empty() {
            return Err(e.context(format!("could not undo the restore; the original files are kept at {}", stuck.join(", "))));
        }
        return Err(e);
    }
    for (_, aside) in &touched {
        if let Some(aside) = aside {
            remove_any(aside);
        }
    }
    Ok(touched.into_iter().map(|(p, _)| p.to_path_buf()).collect())
}

fn aside_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".saba-restore-old");
    path.with_file_name(name)
}

/// 치워 둘 자리. 지난 복원이 남긴 것(지우지 못했거나 되돌리지 못한 원본)은 건드리지 않고 다음 번호를 쓴다
fn free_aside_path(path: &Path) -> PathBuf {
    let base = aside_path(path);
    (1..)
        .map(|n| if n == 1 { base.clone() } else { PathBuf::from(format!("{}.{}", base.display(), n)) })
        .find(|p| !p.exists() && p.symlink_metadata().is_err())
        .expect("a free name")
}

fn remove_any(path: &Path) {
    let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
}

fn extract_entry(zip: &mut zip::ZipArchive<File>, entry: &ManifestEntry) -> Result<()> {
    if entry.is_file {
        if let Some(parent) = entry.path.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut zip.by_name(&entry.entry)?, &mut File::create(&entry.path)?)?;
        return Ok(());
    }
    fs::create_dir_all(&entry.path)?;
    let prefix = PathBuf::from(&entry.entry);
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        // enclosed_name: `..`나 절대 경로로 폴더 밖에 쓰는 항목을 막는다
        let Some(rel) = file.enclosed_name().and_then(|p| p.strip_prefix(&prefix).ok()).map(Path::to_path_buf) else {
            continue;
        };
        let out = entry.path.join(rel);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            io::copy(&mut file, &mut File::create(&out)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(root: &Path) -> PathBuf {
        let w = root.join("world");
        fs::create_dir_all(w.join("region")).unwrap();
        fs::write(w.join("level.dat"), b"level").unwrap();
        fs::write(w.join("region/r.0.0.mca"), b"chunks").unwrap();
        w
    }

    #[test]
    fn fill_replaces_settings_and_drops_unset_ones() {
        let tmp = tempfile::tempdir().unwrap();
        world(tmp.path());
        let patterns = vec!["{level_name}".to_string(), "{level_name}_nether".to_string(), "{unset}/x".to_string()];
        let got = fill_paths(&patterns, tmp.path(), |k| (k == "level_name").then(|| "world".to_string()));
        assert_eq!(got, vec![tmp.path().join("world"), tmp.path().join("world_nether")]);
    }

    #[test]
    fn create_restore_roundtrip_and_rollback_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let props = tmp.path().join("server.properties");
        fs::write(&props, b"a=1").unwrap();
        let dir = tmp.path().join("backups");
        let b = create(&dir, BackupKind::Manual, &[w.clone(), props.clone()]).unwrap();
        assert!(b.name.ends_with("-manual.zip"));

        fs::write(w.join("level.dat"), b"broken").unwrap();
        fs::write(w.join("new.dat"), b"x").unwrap();
        fs::write(&props, b"a=2").unwrap();
        let restored = restore(&dir, &b.name).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level");
        assert_eq!(fs::read(w.join("region/r.0.0.mca")).unwrap(), b"chunks");
        assert!(!w.join("new.dat").exists(), "restore replaces the folder, not merges");
        assert_eq!(fs::read(&props).unwrap(), b"a=1");
        assert!(!aside_path(&w).exists());
    }

    #[test]
    fn a_failed_restore_puts_every_path_back() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let props = tmp.path().join("server.properties");
        fs::write(&props, b"a=1").unwrap();
        let dir = tmp.path().join("backups");
        let b = create(&dir, BackupKind::Manual, &[w.clone(), props.clone()]).unwrap();
        // 두 번째 항목의 파일이 빠진 백업: 첫 경로는 풀리고 두 번째에서 실패한다
        let broken = dir.join("20000101-000000-000-manual.zip");
        {
            let mut src = zip::ZipArchive::new(File::open(dir.join(&b.name)).unwrap()).unwrap();
            let mut out = zip::ZipWriter::new(File::create(&broken).unwrap());
            for i in 0..src.len() {
                let f = src.by_index(i).unwrap();
                if f.name() != "1" {
                    out.raw_copy_file(f).unwrap();
                }
            }
            out.finish().unwrap();
        }
        fs::write(w.join("level.dat"), b"current").unwrap();
        fs::write(&props, b"a=2").unwrap();
        assert!(restore(&dir, "20000101-000000-000-manual.zip").is_err());
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"current", "the first path must be rolled back too");
        assert_eq!(fs::read(&props).unwrap(), b"a=2");
        assert!(!aside_path(&w).exists() && !aside_path(&props).exists());
    }

    #[test]
    fn prune_keeps_manual_and_newest_auto() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let manual = create(&dir, BackupKind::Manual, &[w.clone()]).unwrap();
        let autos: Vec<_> = (0..3).map(|_| create(&dir, BackupKind::Scheduled, &[w.clone()]).unwrap().name).collect();
        let removed = prune(&dir, 1, &[]);
        assert_eq!(removed.len(), 2);
        let left: Vec<_> = list(&dir).into_iter().map(|b| b.name).collect();
        assert!(left.contains(&manual.name));
        assert!(left.contains(autos.iter().max().unwrap()));
    }

    /// 같은 밀리초에 종류가 다른 백업을 만들어도 나중 것이 이름순으로 뒤에 온다
    #[test]
    fn a_backup_made_in_the_same_millisecond_still_sorts_as_newer() {
        let tmp = tempfile::tempdir().unwrap();
        let stamp = "20260101-000000-000";
        let first = unique_name(tmp.path(), stamp, BackupKind::Scheduled);
        fs::write(tmp.path().join(&first), b"").unwrap();
        let second = unique_name(tmp.path(), stamp, BackupKind::PreRestore);
        assert!(second > first, "{second} should sort after {first}");
        fs::write(tmp.path().join(&second), b"").unwrap();
        let third = unique_name(tmp.path(), stamp, BackupKind::Scheduled);
        assert!(third > second, "{third} should sort after {second}");
    }

    /// 안전 백업(업데이트 전·복원 전)은 예약 백업과 따로 센다: 업데이트를 몇 번 하면 예약 백업이 밀려 지워지지 않게
    #[test]
    fn safety_backups_do_not_push_out_scheduled_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let scheduled = create(&dir, BackupKind::Scheduled, &[w.clone()]).unwrap().name;
        let updates: Vec<_> = (0..3).map(|_| create(&dir, BackupKind::PreUpdate, &[w.clone()]).unwrap().name).collect();
        let removed = prune(&dir, 2, &[]);
        assert_eq!(removed, vec![updates[0].clone()], "only the oldest pre-update backup goes");
        assert!(dir.join(&scheduled).exists());
    }

    /// 지난 복원이 치워 둔 것(`.saba-restore-old`)이 남아 있어도 복원된다. 남은 것은 지우지 않는다: 되돌리지 못한 원본일 수 있다
    #[test]
    fn a_leftover_from_an_earlier_restore_does_not_block_the_next() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let made = create(&dir, BackupKind::Manual, &[w.clone()]).unwrap().name;
        let leftover = aside_path(&w);
        fs::create_dir_all(&leftover).unwrap();
        fs::write(leftover.join("kept.txt"), b"original").unwrap();
        fs::write(w.join("level.dat"), b"changed").unwrap();
        restore(&dir, &made).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level");
        assert_eq!(fs::read(leftover.join("kept.txt")).unwrap(), b"original");
    }

    #[test]
    fn prune_never_removes_the_backup_being_restored() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let oldest = create(&dir, BackupKind::Scheduled, &[w.clone()]).unwrap().name;
        let safety = create(&dir, BackupKind::PreRestore, &[w.clone()]).unwrap().name;
        let removed = prune(&dir, 1, &[&oldest]);
        assert!(removed.is_empty(), "keep=1 would drop the oldest auto backup, but it is the restore target");
        let left: Vec<_> = list(&dir).into_iter().map(|b| b.name).collect();
        assert!(left.contains(&oldest) && left.contains(&safety));
        assert!(restore(&dir, &oldest).is_ok());
    }

    #[test]
    fn prune_with_keep_zero_still_keeps_the_new_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let old = create(&dir, BackupKind::Scheduled, &[w.clone()]).unwrap().name;
        let safety = create(&dir, BackupKind::PreRestore, &[w.clone()]).unwrap().name;
        assert_eq!(prune(&dir, 0, &[&safety]), vec![old]);
        assert!(dir.join(&safety).exists());
    }

    #[test]
    fn names_from_the_api_stay_inside_the_backup_folder() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["../x-manual.zip", "..\\x-manual.zip", "x.zip", ".hidden-manual.zip"] {
            assert!(delete(tmp.path(), bad).is_err(), "{}", bad);
        }
        assert!(create(tmp.path(), BackupKind::Manual, &[]).is_err());
    }
}
