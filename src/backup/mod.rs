//! 월드·세이브 백업 (A2)
//!
//! 모듈 `[backup] paths`가 가리키는 폴더·파일을 `<인스턴스 폴더>/backups/<시각>-<종류>.zip` 하나로 적는다.
//! 판 2(MK16, 0.1.9~)는 증분이다: 파일 내용은 `backups/objects/<해시 앞 두 글자>/<sha256>`에 한 번만 두고(deflate),
//! zip에는 `saba-backup-v2.json`(항목 번호 → 원래 절대 경로, 파일마다 상대 경로·해시·크기·수정 시각)만 담는다.
//! 지난 백업과 크기·수정 시각이 같은 파일은 다시 읽지 않는다. 백업을 지우면 어느 백업도 쓰지 않는 내용을 치운다.
//! 판 1(0.1.8까지)은 zip 안에 `saba-backup.json`과 `<번호>/...` 파일을 담는다. 읽기·복원은 두 판 모두 한다.
//! 0.1.8 데몬은 판 2의 매니페스트 이름을 몰라 복원을 거절한다(빈 폴더로 바꿔 넣지 않는다).
//! 폴더 안의 심볼릭 링크는 가리키는 곳을 담지 않고 `links`에 링크로만 적는다.
//! 이 파일의 함수는 모두 블로킹이다 (호출자가 spawn_blocking).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "saba-backup.json";
/// 판 2 매니페스트와 목록용 요약 (요약은 큰 파일 목록을 읽지 않고 크기를 보이려고 따로 둔다)
const MANIFEST_V2: &str = "saba-backup-v2.json";
const SUMMARY_V2: &str = "saba-backup-v2-summary.json";
/// 판 2의 내용 저장소 (백업 폴더 안)
const OBJECTS: &str = "objects";
/// 이보다 최근에 바뀐 파일은 크기·수정 시각이 같아도 다시 읽는다 (같은 시각 안에 또 바뀌었을 수 있다)
const MTIME_SLACK_NS: u64 = 2_000_000_000;

/// 백업 종류. `Manual`만 정리(prune)에서 빠진다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupKind {
    Manual,
    Scheduled,
    PreUpdate,
    PreRestore,
    /// 게임이 스스로 만든 자동 백업(스냅샷)을 보관한 것. 수동처럼 정리하지 않는다
    Snapshot,
}

impl BackupKind {
    fn as_str(self) -> &'static str {
        match self {
            BackupKind::Manual => "manual",
            BackupKind::Scheduled => "scheduled",
            BackupKind::PreUpdate => "pre-update",
            BackupKind::PreRestore => "pre-restore",
            BackupKind::Snapshot => "snapshot",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".zip")?;
        [Self::Manual, Self::Scheduled, Self::PreUpdate, Self::PreRestore, Self::Snapshot]
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
    /// 담은 내용의 크기 (복원하면 쓰는 바이트. 판 1은 zip 파일 크기)
    pub size: u64,
    /// 이 백업이 만들어질 때 디스크에 새로 쓴 바이트 (앞 백업과 같은 내용은 다시 쓰지 않는다. 판 1은 zip 파일 크기)
    pub stored: u64,
    /// zip으로 열리는가. 열리지 않는 백업은 복원할 수 없고, 지울 때까지 쓰지 않는 내용의 정리(GC)를 멈춘다
    pub readable: bool,
    /// 만든 시각 (유닉스 나노초, 판 2는 요약의 시작 시각, 판 1은 파일 수정 시각). 이름은 지역 시각이라 서머타임 끝에 뒤바뀔 수 있어 순서는 이것으로 정한다 (D-B12)
    #[serde(skip)]
    made: u64,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    paths: Vec<ManifestEntry>,
    /// 백업한 폴더 안의 심볼릭 링크. 가리키는 곳은 담지 않고 링크만 되살린다 (D-B13)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    links: Vec<LinkEntry>,
}

/// 판 2 매니페스트
#[derive(Serialize, Deserialize)]
struct ManifestV2 {
    version: u32,
    paths: Vec<ManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    links: Vec<LinkEntry>,
    files: Vec<StoredFile>,
    /// 폴더 항목 안의 하위 폴더 (빈 폴더도 되살린다)
    #[serde(default)]
    dirs: Vec<StoredDir>,
    /// 백업을 시작한 시각 (유닉스 나노초). 이보다 조금 앞서 바뀐 파일은 다음 백업이 다시 읽는다
    started: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredFile {
    /// `paths`의 항목 번호
    entry: String,
    /// 그 항목 폴더 안의 상대 경로(`/`로 잇는다). 파일 항목이면 빈 문자열
    rel: String,
    hash: String,
    size: u64,
    /// 수정 시각 (유닉스 나노초)
    mtime: u64,
    /// 내용 파일(압축)의 크기. 다음 백업이 이어 쓰기 전에 그 파일이 그대로인지 본다
    #[serde(default)]
    stored_size: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredDir {
    entry: String,
    rel: String,
}

/// 목록에 보이는 크기 (`BackupInfo`)
#[derive(Serialize, Deserialize, Default)]
struct SummaryV2 {
    size: u64,
    stored: u64,
    #[serde(default)]
    started: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct LinkEntry {
    path: PathBuf,
    target: PathBuf,
    /// 폴더를 가리키는 링크 (Windows는 폴더·파일 링크를 따로 만든다)
    dir: bool,
}

/// 이보다 오래된 `*.partial`은 강제 종료가 남긴 것으로 본다 (쓰는 중이면 수정 시각이 계속 새로워진다)
const STALE_PARTIAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

#[derive(Serialize, Deserialize, Clone)]
struct ManifestEntry {
    entry: String,
    path: PathBuf,
    is_file: bool,
    /// 다른 자리에서 읽어 `path`로 되돌리는 항목(게임 스냅샷)이면 읽은 곳. 그 크기·시각은 `path` 파일의 재사용 기록이 아니다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<PathBuf>,
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

/// 백업 폴더마다 하나: 만들기·지우기·정리·복원이 내용 저장소를 함께 쓰므로 한 번에 하나만 한다
/// (정리가 만드는 중인 백업의 새 내용을 지우거나 복원이 읽는 내용을 지우지 않게)
fn dir_lock(dir: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    type Locks = std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>;
    static LOCKS: std::sync::LazyLock<std::sync::Mutex<Locks>> = std::sync::LazyLock::new(Default::default);
    LOCKS.lock().unwrap_or_else(|e| e.into_inner()).entry(dir.to_path_buf()).or_default().clone()
}

/// 새 백업을 만든다. 같은 초에 두 번 만들면 이름 뒤에 번호를 붙인다
pub fn create(dir: &Path, kind: BackupKind, paths: &[PathBuf]) -> Result<BackupInfo> {
    create_excluding(dir, kind, paths, &[])
}

/// 서버가 쓰는 중이라 읽지 못한 파일 (Windows 공유·잠금 위반). 호출자는 `backup_file_locked`로 알린다
#[derive(Debug)]
pub struct LockedFile {
    pub path: PathBuf,
}

impl std::fmt::Display for LockedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is in use by another process", self.path.display())
    }
}

impl std::error::Error for LockedFile {}

/// 읽기 오류. 다른 프로세스가 잠근 파일이면 `LockedFile` (ERROR_SHARING_VIOLATION 32, ERROR_LOCK_VIOLATION 33)
fn read_error(e: io::Error, path: &Path) -> anyhow::Error {
    if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
        return LockedFile { path: path.to_path_buf() }.into();
    }
    anyhow::Error::new(e).context(format!("read {}", path.display()))
}

/// `exclude`: 폴더 안에서 담지 않을 파일 이름 패턴 (`*` 가능. 예: 서버가 쥐고 있는 `session.lock`)
pub fn create_excluding(dir: &Path, kind: BackupKind, paths: &[PathBuf], exclude: &[String]) -> Result<BackupInfo> {
    let entries: Vec<(PathBuf, PathBuf)> = paths.iter().map(|p| (p.clone(), p.clone())).collect();
    create_from(dir, kind, &entries, exclude)
}

/// `entries`: (읽을 곳, 복원할 곳). 게임 스냅샷처럼 다른 자리의 내용을 원래 자리로 되돌릴 백업을 만든다
pub fn create_from(dir: &Path, kind: BackupKind, entries: &[(PathBuf, PathBuf)], exclude: &[String]) -> Result<BackupInfo> {
    if entries.is_empty() {
        bail!("nothing to back up: none of the module's backup paths exist");
    }
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let lock = dir_lock(dir);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    clear_stale_partials(dir);
    // 밀리초까지 넣어 이름순이 곧 만든 순서가 되게 한다 (같은 초에 만든 복원 전 백업이 더 오래돼 보이지 않게)
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let name = unique_name(dir, &stamp, kind);
    let final_path = dir.join(&name);
    let partial = dir.join(format!("{}.partial", name));
    let exclude: Vec<glob::Pattern> = exclude.iter().filter_map(|p| glob::Pattern::new(p).ok()).collect();
    let written = write_v2(dir, &partial, entries, &exclude);
    if let Err(e) = written {
        let _ = fs::remove_file(&partial);
        // 이번에 넣은 내용은 어느 백업도 쓰지 않는다
        collect_garbage(dir);
        return Err(e);
    }
    fs::rename(&partial, &final_path)?;
    info(&final_path).context("read new backup")
}

/// 강제 종료로 남은 반쪽 백업 파일을 지운다 (D-B13)
fn clear_stale_partials(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let stale = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_PARTIAL);
        if stale && path.extension().is_some_and(|e| e == "partial") {
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!("Could not remove leftover partial backup {}: {}", path.display(), e);
            }
        }
    }
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

fn now_ns() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn mtime_ns(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn object_path(dir: &Path, hash: &str) -> Result<PathBuf> {
    if !is_hash(hash) {
        bail!("backup content id is not valid");
    }
    Ok(dir.join(OBJECTS).join(&hash[..2]).join(hash))
}

/// 매니페스트의 상대 경로를 그 항목 폴더 안의 경로로 (`..`·절대 경로·드라이브는 받지 않는다)
fn inner_path(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path: PathBuf = rel.split('/').collect();
    if rel.is_empty() || !rel_path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        bail!("backup entry path is not valid: {}", rel);
    }
    Ok(base.join(rel_path))
}

/// 가장 새 판 2 백업이 적은 파일 (원래 절대 경로 → 크기, 수정 시각, 해시). 그 백업 직전에 바뀐 파일은 뺀다
fn known_files(dir: &Path) -> std::collections::HashMap<PathBuf, (u64, u64, String, u64)> {
    let newest = list(dir).into_iter().find_map(|b| read_v2(&dir.join(&b.name)).ok().flatten());
    let Some(m) = newest else { return Default::default() };
    let mut out = std::collections::HashMap::new();
    for f in m.files.iter().filter(|f| f.mtime.saturating_add(MTIME_SLACK_NS) < m.started) {
        let Some(entry) = m.paths.iter().find(|p| p.entry == f.entry && p.from.is_none()) else { continue };
        let path = if f.rel.is_empty() { entry.path.clone() } else if let Ok(p) = inner_path(&entry.path, &f.rel) { p } else { continue };
        out.insert(path, (f.size, f.mtime, f.hash.clone(), f.stored_size));
    }
    out
}

/// 파일 내용을 저장소에 넣는다: (해시, 읽은 바이트, 내용 파일 크기, 새로 쓴 파일). 같은 내용이 이미 있으면 새로 쓰지 않는다.
/// 이미 있는 파일의 크기가 방금 만든 것과 다르면 망가진 것으로 보고 바꿔 넣는다 (망가진 내용을 다음 백업이 물려받지 않게).
/// 디스크에 내리는 일(fsync)은 부른 쪽이 백업을 마치기 전에 한꺼번에 한다
fn put_object(dir: &Path, path: &Path) -> Result<(String, u64, u64, Option<PathBuf>)> {
    use sha2::Digest;
    use std::io::{Read, Write};
    let objects = dir.join(OBJECTS);
    fs::create_dir_all(&objects)?;
    let tmp = objects.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    let written = (|| -> Result<(String, u64)> {
        // 서버가 쓰는 중이라 잠긴 파일(Windows)은 건너뛰지 않고 실패로 알린다: 반쪽 백업을 믿지 않게
        let mut src = File::open(path).map_err(|e| read_error(e, path))?;
        let mut out = flate2::write::DeflateEncoder::new(File::create(&tmp)?, flate2::Compression::default());
        let mut hasher = sha2::Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut read = 0u64;
        loop {
            let n = src.read(&mut buf).map_err(|e| read_error(e, path))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            read += n as u64;
        }
        out.finish()?;
        Ok((hex::encode(hasher.finalize()), read))
    })();
    let (hash, read) = match written {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let target = object_path(dir, &hash)?;
    let added = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == added) {
        let _ = fs::remove_file(&tmp);
        return Ok((hash, read, added, None));
    }
    if target.is_file() {
        tracing::warn!("Backup content {} was damaged; replacing it", &hash[..12]);
    }
    let moved = target.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::rename(&tmp, &target));
    if let Err(e) = moved {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok((hash, read, added, Some(target)))
}

/// 판 2 백업 쓰기 중의 상태
struct V2Writer<'a> {
    dir: &'a Path,
    /// 백업 폴더의 실제 경로. 백업할 폴더 안에 있으면 담지 않는다 (지난 백업을 다시 담아 백업마다 커지지 않게)
    own_dir: Option<PathBuf>,
    known: std::collections::HashMap<PathBuf, (u64, u64, String, u64)>,
    exclude: &'a [glob::Pattern],
    /// 이번에 새로 쓴 내용 파일 (매니페스트를 쓰기 전에 한꺼번에 디스크에 내린다)
    written: Vec<PathBuf>,
    manifest: ManifestV2,
    summary: SummaryV2,
}

impl V2Writer<'_> {
    fn add_file(&mut self, path: &Path, entry: &str, rel: &str) -> Result<()> {
        let meta = fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
        let mtime = mtime_ns(&meta);
        // 이어 쓰는 내용 파일은 크기가 적어 둔 그대로일 때만 (망가졌으면 다시 읽어 바꿔 넣는다)
        let reuse = match self.known.get(path) {
            Some((size, known_mtime, hash, stored_size)) if *size == meta.len() && *known_mtime == mtime && fs::metadata(object_path(self.dir, hash)?).is_ok_and(|m| m.is_file() && m.len() == *stored_size) => {
                Some((hash.clone(), *size, *stored_size))
            }
            _ => None,
        };
        let (hash, size, stored_size) = match reuse {
            Some(v) => v,
            None => {
                let (hash, read, stored_size, written) = put_object(self.dir, path)?;
                if let Some(written) = written {
                    self.summary.stored += stored_size;
                    self.written.push(written);
                }
                (hash, read, stored_size)
            }
        };
        self.summary.size += size;
        self.manifest.files.push(StoredFile { entry: entry.to_string(), rel: rel.to_string(), hash, size, mtime, stored_size });
        Ok(())
    }

    fn add_dir(&mut self, dir: &Path, entry: &str, rel: &str) -> Result<()> {
        for item in fs::read_dir(dir)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() { name } else { format!("{}/{}", rel, name) };
            let kind = item.file_type()?;
            if kind.is_dir() {
                if self.own_dir.is_some() && fs::canonicalize(item.path()).ok() == self.own_dir {
                    continue;
                }
                self.manifest.dirs.push(StoredDir { entry: entry.to_string(), rel: child.clone() });
                self.add_dir(&item.path(), entry, &child)?;
            } else if kind.is_symlink() {
                let path = item.path();
                self.manifest.links.push(LinkEntry { target: fs::read_link(&path)?, dir: path.is_dir(), path });
            } else if kind.is_file() {
                if self.exclude.iter().any(|p| p.matches(&item.file_name().to_string_lossy())) {
                    continue;
                }
                self.add_file(&item.path(), entry, &child)?;
            }
        }
        Ok(())
    }
}

fn write_v2(dir: &Path, target: &Path, entries: &[(PathBuf, PathBuf)], exclude: &[glob::Pattern]) -> Result<()> {
    let started = now_ns();
    let mut w = V2Writer {
        dir,
        own_dir: fs::canonicalize(dir).ok(),
        known: known_files(dir),
        exclude,
        written: Vec::new(),
        manifest: ManifestV2 { version: 2, paths: Vec::new(), links: Vec::new(), files: Vec::new(), dirs: Vec::new(), started },
        summary: SummaryV2 { started, ..Default::default() },
    };
    for (i, (source, path)) in entries.iter().enumerate() {
        let entry = i.to_string();
        let is_file = source.is_file();
        let links_before = w.manifest.links.len();
        if is_file {
            w.add_file(source, &entry, "")?;
        } else {
            w.add_dir(source, &entry, "")?;
        }
        // 다른 자리에서 읽은 폴더 안의 링크는 복원 자리 아래로 옮겨 적는다 (그 안을 가리키는 대상도)
        if source != path {
            let moved = |p: &Path| p.strip_prefix(source).ok().map(|rel| path.join(rel));
            for link in &mut w.manifest.links[links_before..] {
                if let Some(p) = moved(&link.path) {
                    link.path = p;
                }
                if let Some(t) = moved(&link.target) {
                    link.target = t;
                }
            }
        }
        let from = (source != path).then(|| source.clone());
        w.manifest.paths.push(ManifestEntry { entry, path: path.clone(), is_file, from });
    }
    sync_all(&w.written)?;
    let mut zip = zip::ZipWriter::new(File::create(target)?);
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    zip.start_file(SUMMARY_V2, opts)?;
    serde_json::to_writer(&mut zip, &w.summary)?;
    zip.start_file(MANIFEST_V2, opts)?;
    serde_json::to_writer(&mut zip, &w.manifest)?;
    zip.finish()?.sync_all()?;
    Ok(())
}

/// 새로 쓴 내용 파일을 디스크에 내린다. Windows는 쓰기 권한 핸들이어야 내릴 수 있다. 파일마다 차례로 기다리면 작은 파일이 많은 서버의 첫 백업이 아주 느려서 몇 갈래로 나눠 한다
fn sync_all(files: &[PathBuf]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let lanes = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let chunk = files.len().div_ceil(lanes);
    std::thread::scope(|scope| {
        let handles: Vec<_> = files.chunks(chunk).map(|part| scope.spawn(move || part.iter().try_for_each(|f| fs::OpenOptions::new().write(true).open(f).and_then(|f| f.sync_all())))).collect();
        handles.into_iter().try_for_each(|h| h.join().unwrap_or_else(|_| Err(io::Error::other("sync thread panicked"))))
    })?;
    Ok(())
}

/// 판 2 매니페스트. 판 1이면 `None`
fn read_v2(path: &Path) -> Result<Option<ManifestV2>> {
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    let found = zip.by_name(MANIFEST_V2);
    match found {
        Ok(file) => Ok(Some(serde_json::from_reader(file)?)),
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 폴더에 있는 백업 이름 꼴의 파일 (목록이나 항목을 하나라도 읽지 못하면 오류). 그 이름의 폴더는 백업이 아니다
fn backup_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Ok(name) = entry.file_name().into_string() {
            if BackupKind::from_name(&name).is_some() && !entry.path().is_dir() {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

/// 어느 백업도 쓰지 않는 내용과 끝나지 않은 임시 파일을 치운다. 호출자가 `dir_lock`을 쥐고 있어야 한다.
/// 백업 하나라도 읽지 못하면 무엇을 쓰는지 모르니 아무것도 지우지 않는다
fn collect_garbage(dir: &Path) {
    let objects = dir.join(OBJECTS);
    if !objects.is_dir() {
        return;
    }
    // 목록이 아니라 폴더의 백업 이름을 모두 본다: 목록에서 빠진(열지 못한) 백업의 내용도 지우지 않게
    let mut used = std::collections::HashSet::new();
    let names = match backup_names(dir) {
        Ok(names) => names,
        Err(e) => {
            tracing::warn!("Not clearing unused backup content: could not list {}: {}", dir.display(), e);
            return;
        }
    };
    for name in names {
        match read_v2(&dir.join(&name)) {
            Ok(Some(m)) => used.extend(m.files.into_iter().map(|f| f.hash)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Not clearing unused backup content: could not read {}: {}", name, e);
                return;
            }
        }
    }
    for shard in fs::read_dir(&objects).into_iter().flatten().flatten() {
        let path = shard.path();
        let name = shard.file_name().to_string_lossy().into_owned();
        if name.starts_with(".tmp-") {
            let _ = fs::remove_file(&path);
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        for object in fs::read_dir(&path).into_iter().flatten().flatten() {
            let hash = object.file_name().to_string_lossy().into_owned();
            if !used.contains(&hash) {
                if let Err(e) = fs::remove_file(object.path()) {
                    tracing::warn!("Could not remove unused backup content {}: {}", object.path().display(), e);
                }
            }
        }
        let _ = fs::remove_dir(&path); // 비었을 때만 지워진다
    }
}

/// 판 1 백업 쓰기 (0.1.8까지의 모양. 시험이 옛 백업을 만드는 데 쓴다)
#[cfg(test)]
fn write_zip(target: &Path, paths: &[PathBuf]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(target)?);
    let opts = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut manifest = Manifest { version: 1, paths: Vec::new(), links: Vec::new() };
    for (i, path) in paths.iter().enumerate() {
        let entry = i.to_string();
        let is_file = path.is_file();
        if is_file {
            zip.start_file(&entry, opts)?;
            io::copy(&mut File::open(path)?, &mut zip)?;
        } else {
            zip.add_directory(format!("{}/", entry), opts)?;
            add_dir(&mut zip, path, &entry, opts, &mut manifest.links)?;
        }
        manifest.paths.push(ManifestEntry { entry, path: path.clone(), is_file, from: None });
    }
    zip.start_file(MANIFEST, opts)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;
    zip.finish()?.sync_all()?;
    Ok(())
}

#[cfg(test)]
fn add_dir(zip: &mut zip::ZipWriter<File>, dir: &Path, prefix: &str, opts: zip::write::FileOptions, links: &mut Vec<LinkEntry>) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = format!("{}/{}", prefix, item.file_name().to_string_lossy());
        let kind = item.file_type()?;
        if kind.is_dir() {
            zip.add_directory(format!("{}/", name), opts)?;
            add_dir(zip, &item.path(), &name, opts, links)?;
        } else if kind.is_symlink() {
            let path = item.path();
            links.push(LinkEntry { target: fs::read_link(&path)?, dir: path.is_dir(), path });
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
    let meta = match fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return None,
        // 대상이 없는 링크: 정리를 멈추니 열 수 없는 백업으로 보인다
        Err(_) => {
            fs::symlink_metadata(path).ok()?;
            let made = name_time_ns(&name).unwrap_or(0);
            return Some(BackupInfo { name, kind, created: made / 1_000_000_000, size: 0, stored: 0, readable: false, made });
        }
    };
    let modified = mtime_ns(&meta);
    // 판 2는 요약을 읽는다. 판 1(요약 없음)이나 읽지 못하면 zip 크기.
    // 열리고 매니페스트가 있어야 읽을 수 있다 (매니페스트 내용까지 읽지는 않는다: 목록마다 모두 풀면 느리다)
    let zip = File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok());
    let readable = zip.as_ref().is_some_and(|z| z.file_names().any(|n| n == MANIFEST_V2 || n == MANIFEST));
    let summary = zip.and_then(|mut zip| zip.by_name(SUMMARY_V2).ok().and_then(|f| serde_json::from_reader::<_, SummaryV2>(f).ok()));
    // 판 1은 0.1.8까지 만든 것이다: 수정 시각이 새로 찍힌 사본(옮겼다 되돌린 것)도 이름의 시각보다 새것으로 보지 않는다
    let v1_made = name_time_ns(&name).map_or(modified, |named| named.min(modified));
    let (size, stored, made) = summary.map_or((meta.len(), meta.len(), v1_made), |s| (s.size, s.stored, if s.started > 0 { s.started } else { modified }));
    Some(BackupInfo { name, kind, created: made / 1_000_000_000, size, stored, readable, made })
}

/// 이름 앞의 지역 시각(`YYYYMMDD-HHMMSS`)을 유닉스 나노초로
fn name_time_ns(name: &str) -> Option<u64> {
    let naive = chrono::NaiveDateTime::parse_from_str(name.get(..15)?, "%Y%m%d-%H%M%S").ok()?;
    let local = naive.and_local_timezone(chrono::Local).earliest()?;
    u64::try_from(local.timestamp_nanos_opt()?).ok()
}

/// 새것부터
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let mut out: Vec<BackupInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| info(&e.path()))
        .collect();
    out.sort_by(|a, b| b.made.cmp(&a.made).then_with(|| b.name.cmp(&a.name)));
    out
}

/// API로 받은 이름이 백업 폴더 안의 백업 파일만 가리키게
fn checked(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.contains(['/', '\\']) || name.starts_with('.') || BackupKind::from_name(name).is_none() {
        bail!("invalid backup name: {}", name);
    }
    let path = dir.join(name);
    // 대상이 없는 링크도 목록에 보이므로 받는다 (지우면 링크만 지운다)
    if !path.is_file() && !path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("backup not found: {}", name);
    }
    Ok(path)
}

pub fn delete(dir: &Path, name: &str) -> Result<()> {
    let lock = dir_lock(dir);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    fs::remove_file(checked(dir, name)?)?;
    collect_garbage(dir);
    Ok(())
}

/// 자동 백업(수동 제외)을 새것 `keep`개만 남긴다. `protect`는 넘쳐도 지우지 않는다 (방금 만든 백업, 복원 중인 백업). 지운 이름을 돌려준다
pub fn prune(dir: &Path, keep: usize, protect: &[&str]) -> Vec<String> {
    let lock = dir_lock(dir);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    // 종류마다 따로 센다: 업데이트 전·복원 전 안전 백업이 예약 백업을 밀어내지 않게
    let mut seen: std::collections::HashMap<BackupKind, usize> = std::collections::HashMap::new();
    let removed: Vec<String> = list(dir)
        .into_iter()
        .filter(|b| !matches!(b.kind, BackupKind::Manual | BackupKind::Snapshot))
        .filter(|b| {
            let n = seen.entry(b.kind).or_default();
            *n += 1;
            *n > keep
        })
        .filter(|b| !protect.contains(&b.name.as_str()))
        .filter(|b| fs::remove_file(dir.join(&b.name)).is_ok())
        .map(|b| b.name)
        .collect();
    if !removed.is_empty() {
        collect_garbage(dir);
    }
    removed
}

/// 백업이 돌려놓을 경로 (백업할 때의 절대 경로)
pub fn backed_up_paths(dir: &Path, name: &str) -> Result<Vec<PathBuf>> {
    Ok(open(dir, name)?.paths().iter().map(|e| e.path.clone()).collect())
}

/// 읽은 백업 (두 판)
enum Opened {
    V1(zip::ZipArchive<File>, Manifest),
    V2(ManifestV2),
}

impl Opened {
    fn paths(&self) -> &[ManifestEntry] {
        match self {
            Opened::V1(_, m) => &m.paths,
            Opened::V2(m) => &m.paths,
        }
    }

    fn links(&self) -> &[LinkEntry] {
        match self {
            Opened::V1(_, m) => &m.links,
            Opened::V2(m) => &m.links,
        }
    }
}

fn open(dir: &Path, name: &str) -> Result<Opened> {
    let path = checked(dir, name)?;
    if let Some(m) = read_v2(&path)? {
        return Ok(Opened::V2(m));
    }
    let mut zip = zip::ZipArchive::new(File::open(&path)?)?;
    let manifest: Manifest = serde_json::from_reader(zip.by_name(MANIFEST).context("not a saba-chan backup")?)?;
    Ok(Opened::V1(zip, manifest))
}

/// 링크를 통과하지 않고 실제 경로가 허용 루트 안에 있는가. 없는 대상은 기존 조상부터 확인한다
pub fn is_inside(path: &Path, roots: &[PathBuf]) -> bool {
    crate::module_paths::is_inside(path, roots)
}

/// 복원 결과
#[derive(Debug)]
pub struct Restored {
    /// 돌려놓은 경로
    pub paths: Vec<PathBuf>,
    /// 되살리지 못한 링크 수 (Windows 권한 등). 나머지 복원은 끝났다 (D-B14)
    pub links_missed: usize,
}

/// 백업을 원래 자리로 돌려놓는다. 서버는 꺼져 있어야 한다 (호출자가 확인).
/// 경로마다 기존 것을 옆으로 치워 두고 풀어 넣는다. 치워 둔 것은 모두 성공한 뒤에 지우고,
/// 도중에 실패하면 이미 풀어 넣은 경로까지 거꾸로 모두 되돌린다
pub fn restore(dir: &Path, name: &str) -> Result<Restored> {
    let lock = dir_lock(dir);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut opened = open(dir, name)?;
    let entries: Vec<ManifestEntry> = opened.paths().to_vec();
    let links: Vec<LinkEntry> = opened.links().to_vec();
    // (경로, 기존 것을 치워 둔 곳). 실패하면 이 목록을 거꾸로 되돌린다
    let mut touched: Vec<(&Path, Option<PathBuf>)> = Vec::new();
    let mut failed = None;
    let mut links_missed = 0;
    for entry in &entries {
        let aside = entry.path.exists().then(|| free_aside_path(&entry.path));
        if let Some(aside) = &aside {
            if let Err(e) = fs::rename(&entry.path, aside) {
                failed = Some(anyhow::Error::from(e).context(format!("move aside {}", entry.path.display())));
                break;
            }
        }
        touched.push((&entry.path, aside));
        let extracted = match &mut opened {
            Opened::V1(zip, _) => extract_entry(zip, entry),
            Opened::V2(m) => extract_v2(dir, m, entry),
        };
        if let Err(e) = extracted {
            failed = Some(e.context(format!("restore {}", entry.path.display())));
            break;
        }
        // 링크는 그 폴더 안에 있던 것만, 링크 자체로 되살린다. 만들지 못하면(Windows 권한 등) 알리고 나머지 복원은 이어 간다
        for link in links.iter().filter(|l| !entry.is_file && is_inside(&l.path, std::slice::from_ref(&entry.path))) {
            if let Err(e) = make_link(link) {
                tracing::warn!("Could not restore link {} -> {}: {}", link.path.display(), link.target.display(), e);
                links_missed += 1;
            }
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
    Ok(Restored { paths: touched.into_iter().map(|(p, _)| p.to_path_buf()).collect(), links_missed })
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

fn make_link(link: &LinkEntry) -> io::Result<()> {
    if link.path.symlink_metadata().is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "something is already there"));
    }
    if let Some(parent) = link.path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    return std::os::unix::fs::symlink(&link.target, &link.path);
    #[cfg(windows)]
    return if link.dir {
        std::os::windows::fs::symlink_dir(&link.target, &link.path)
    } else {
        std::os::windows::fs::symlink_file(&link.target, &link.path)
    };
    #[cfg(not(any(unix, windows)))]
    Err(io::Error::new(io::ErrorKind::Unsupported, "links are not supported here"))
}

fn remove_any(path: &Path) {
    let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
}

/// 저장소의 내용 하나를 `out`에 쓴다. 해시·크기가 맞지 않으면(망가진 내용) 실패한다
fn write_object(dir: &Path, file: &StoredFile, out: &Path) -> Result<()> {
    use sha2::Digest;
    use std::io::{Read, Write};
    let object = object_path(dir, &file.hash)?;
    let mut src = flate2::read::DeflateDecoder::new(File::open(&object).context("backup content is missing")?);
    let mut dst = File::create(out)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        let n = src.read(&mut buf).context("backup content is damaged")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n])?;
        written += n as u64;
    }
    if written != file.size || hex::encode(hasher.finalize()) != file.hash {
        bail!("backup content is damaged");
    }
    Ok(())
}

fn extract_v2(dir: &Path, m: &ManifestV2, entry: &ManifestEntry) -> Result<()> {
    let files = m.files.iter().filter(|f| f.entry == entry.entry);
    if entry.is_file {
        let file = m.files.iter().find(|f| f.entry == entry.entry && f.rel.is_empty()).context("backup content is missing")?;
        if let Some(parent) = entry.path.parent() {
            fs::create_dir_all(parent)?;
        }
        return write_object(dir, file, &entry.path);
    }
    fs::create_dir_all(&entry.path)?;
    for d in m.dirs.iter().filter(|d| d.entry == entry.entry) {
        fs::create_dir_all(inner_path(&entry.path, &d.rel)?)?;
    }
    for file in files {
        let out = inner_path(&entry.path, &file.rel)?;
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        write_object(dir, file, &out)?;
    }
    Ok(())
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

    #[test]
    fn contract_paths_restore_rejects_an_intermediate_link_even_to_an_allowed_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("server");
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = root.join("linked");
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&real).output().unwrap().status.success());
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(is_inside(&root.join("new/deleted/world"), std::slice::from_ref(&root)));
        assert!(!is_inside(&link.join("deleted/world"), &[root]));
    }

    /// 0.1.8까지 만든 모양(판 1)의 백업
    fn v1_backup(dir: &Path, paths: &[PathBuf]) -> BackupInfo {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join("20100101-000000-000-manual.zip");
        write_zip(&path, paths).unwrap();
        info(&path).unwrap()
    }

    fn objects(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir.join(OBJECTS)).into_iter().flatten().flatten().flat_map(|s| fs::read_dir(s.path()).into_iter().flatten().flatten()).map(|o| o.file_name().to_string_lossy().into_owned()).collect();
        out.sort();
        out
    }

    fn backed_up_files(dir: &Path, name: &str) -> Vec<String> {
        read_v2(&dir.join(name)).unwrap().unwrap().files.into_iter().map(|f| f.rel).collect()
    }

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

    // D-B13: 강제 종료로 남은 반쪽 파일(`*.zip.partial`)은 다음 백업 때 지운다. 지금 쓰는 중일 수 있는 새 것은 둔다
    #[test]
    fn a_new_backup_clears_partial_files_left_by_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("20200101-000000-000-scheduled.zip.partial");
        fs::write(&stale, b"half").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        File::options().write(true).open(&stale).unwrap().set_modified(old).unwrap();
        let fresh = dir.join("20990101-000000-000-manual.zip.partial");
        fs::write(&fresh, b"being written").unwrap();
        create(&dir, BackupKind::Manual, &[w]).unwrap();
        assert!(!stale.exists(), "a partial file left by a crash must not pile up");
        assert!(fresh.exists(), "a partial file that may still be written stays");
    }

    // D-B13: 백업할 폴더 안의 심볼릭 링크를 말없이 빼면, 복원이 폴더를 바꿔 넣은 뒤 링크가 사라졌다
    #[cfg(unix)]
    #[test]
    fn links_inside_a_backed_up_folder_come_back_on_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let outside = tmp.path().join("shared");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, w.join("shared")).unwrap();
        std::os::unix::fs::symlink("level.dat", w.join("region/latest")).unwrap();
        let dir = tmp.path().join("backups");
        let b = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap();
        restore(&dir, &b.name).unwrap();
        assert_eq!(fs::read_link(w.join("shared")).unwrap(), outside);
        assert_eq!(fs::read_link(w.join("region/latest")).unwrap(), Path::new("level.dat"));
        assert!(outside.is_dir(), "restoring a link never touches where it points");
    }

    /// 되살리지 못한 링크는 경고만 남기고 끝나지 않는다: 복원 결과에 그 수를 실어 사용자가 안다 (D-B14)
    #[cfg(unix)]
    #[test]
    fn a_link_that_cannot_come_back_is_counted_in_the_result() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        std::os::unix::fs::symlink("level.dat", w.join("latest")).unwrap();
        let dir = tmp.path().join("backups");
        let b = v1_backup(&dir, std::slice::from_ref(&w));
        // 파일 아래를 가리키는 링크 자리: 만들 수 없다
        let crafted = dir.join("20000101-000000-000-manual.zip");
        {
            let mut src = zip::ZipArchive::new(File::open(dir.join(&b.name)).unwrap()).unwrap();
            let mut manifest: Manifest = serde_json::from_reader(src.by_name(MANIFEST).unwrap()).unwrap();
            manifest.links.push(LinkEntry { path: w.join("level.dat/inside"), target: "x".into(), dir: false });
            let mut out = zip::ZipWriter::new(File::create(&crafted).unwrap());
            for i in 0..src.len() {
                let f = src.by_index(i).unwrap();
                if f.name() != MANIFEST {
                    out.raw_copy_file(f).unwrap();
                }
            }
            out.start_file(MANIFEST, zip::write::FileOptions::default()).unwrap();
            std::io::Write::write_all(&mut out, &serde_json::to_vec(&manifest).unwrap()).unwrap();
            out.finish().unwrap();
        }
        let done = restore(&dir, "20000101-000000-000-manual.zip").unwrap();
        assert_eq!(done.paths, vec![w.clone()]);
        assert_eq!(done.links_missed, 1);
        assert_eq!(fs::read_link(w.join("latest")).unwrap(), Path::new("level.dat"), "the other link still comes back");
    }

    #[test]
    fn excluded_file_names_are_left_out_of_the_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        fs::write(w.join("session.lock"), b"held by the server").unwrap();
        fs::write(w.join("region/r.0.0.mca.tmp"), b"x").unwrap();
        let dir = tmp.path().join("backups");
        let b = create_excluding(&dir, BackupKind::Manual, std::slice::from_ref(&w), &["session.lock".into(), "*.tmp".into()]).unwrap();
        let files = backed_up_files(&dir, &b.name);
        assert!(files.iter().any(|f| f == "level.dat"), "{files:?}");
        assert!(!files.iter().any(|f| f.ends_with("session.lock") || f.ends_with(".tmp")), "{files:?}");
    }

    /// 다른 자리에서 읽은 백업(게임 스냅샷)의 크기·시각은 복원 자리 파일의 재사용 기록이 아니다:
    /// 다음 백업이 지금 파일 대신 스냅샷 내용을 적으면 안전 백업으로도 되돌릴 수 없다 (Codex P1)
    #[test]
    fn a_backup_read_from_elsewhere_is_not_reused_for_the_live_file() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let snap = tmp.path().join("snap");
        fs::create_dir_all(&snap).unwrap();
        fs::write(snap.join("level.dat"), b"SNAP!").unwrap();
        fs::write(w.join("level.dat"), b"LIVE!").unwrap();
        // 같은 크기·같은 수정 시각 (오래전)
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for p in [snap.join("level.dat"), w.join("level.dat")] {
            File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        }
        let dir = tmp.path().join("backups");
        create_from(&dir, BackupKind::Snapshot, &[(snap.join("level.dat"), w.join("level.dat"))], &[]).unwrap();
        let live = create(&dir, BackupKind::PreRestore, std::slice::from_ref(&w)).unwrap();
        fs::write(w.join("level.dat"), b"other").unwrap();
        restore(&dir, &live.name).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"LIVE!");
    }

    /// 다른 자리에서 읽은 폴더 안의 링크도 복원 자리에 되살린다 (Codex P2)
    #[cfg(windows)]
    #[test]
    fn links_inside_a_folder_read_from_elsewhere_come_back_at_the_restore_place() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snap").join("Players");
        fs::create_dir_all(snap.join("real")).unwrap();
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(snap.join("latest")).arg(snap.join("real")).output().unwrap().status.success());
        let live = tmp.path().join("world").join("Players");
        fs::create_dir_all(&live).unwrap();
        let dir = tmp.path().join("backups");
        let b = create_from(&dir, BackupKind::Snapshot, &[(snap.clone(), live.clone())], &[]).unwrap();
        let done = restore(&dir, &b.name).unwrap();
        assert_eq!(done.links_missed, 0);
        assert!(fs::symlink_metadata(live.join("latest")).is_ok(), "the link comes back under the restore place");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_the_server_holds_open_fails_with_the_locked_error() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let lock = w.join("session.lock");
        // 다른 프로세스가 읽지도 못하게 연 파일 (게임 서버가 잠근 것과 같다)
        let _held = fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(&lock).unwrap();
        let err = create(&tmp.path().join("backups"), BackupKind::Manual, std::slice::from_ref(&w)).unwrap_err();
        let locked = err.downcast_ref::<LockedFile>().expect("locked error");
        assert_eq!(locked.path, lock);
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
        assert_eq!(restored.paths.len(), 2);
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
        let b = v1_backup(&dir, &[w.clone(), props.clone()]);
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

    /// MK16: 바뀌지 않은 파일은 다시 쓰지 않고, 어느 백업이든 그때 내용으로 돌아온다. 0.1.8은 판 2를 백업으로 보지 않는다
    #[test]
    fn unchanged_files_are_stored_once_and_every_backup_restores_its_own_content() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        fs::create_dir_all(w.join("empty")).unwrap();
        let props = tmp.path().join("server.properties");
        fs::write(&props, b"a=1").unwrap();
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 60);
        for f in [w.join("level.dat"), w.join("region/r.0.0.mca"), props.clone()] {
            File::options().write(true).open(f).unwrap().set_modified(hour_ago).unwrap();
        }
        let dir = tmp.path().join("backups");
        let paths = [w.clone(), props.clone()];
        let first = create(&dir, BackupKind::Manual, &paths).unwrap();
        assert_eq!(first.size, 5 + 6 + 3);
        assert!(first.stored > 0);
        let mut zip = zip::ZipArchive::new(File::open(dir.join(&first.name)).unwrap()).unwrap();
        assert!(zip.by_name(MANIFEST).is_err(), "a 0.1.8 daemon must refuse it instead of restoring empty folders");

        let second = create(&dir, BackupKind::Scheduled, &paths).unwrap();
        assert_eq!((second.size, second.stored), (first.size, 0));
        fs::write(w.join("level.dat"), b"level 2").unwrap();
        let third = create(&dir, BackupKind::Scheduled, &paths).unwrap();
        assert!(third.stored > 0 && third.stored < first.stored, "only the changed file is written: {third:?}");
        assert_eq!(objects(&dir).len(), 4);

        fs::remove_dir_all(w.join("empty")).unwrap();
        restore(&dir, &first.name).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level");
        assert_eq!(fs::read(w.join("region/r.0.0.mca")).unwrap(), b"chunks");
        assert!(w.join("empty").is_dir(), "empty folders come back too");
        restore(&dir, &third.name).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level 2");
        assert_eq!(fs::read(&props).unwrap(), b"a=1");
    }

    /// 지우거나 정리하면 남은 백업이 쓰지 않는 내용만 치운다. 읽지 못하는 백업이 있으면 아무것도 치우지 않는다
    #[test]
    fn deleting_backups_clears_only_content_no_backup_uses() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let old = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        let before = objects(&dir);
        fs::write(w.join("level.dat"), b"level 2").unwrap();
        let new = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        assert_eq!(objects(&dir).len(), 3);
        fs::write(dir.join(OBJECTS).join(".tmp-left"), b"half").unwrap();

        assert_eq!(prune(&dir, 1, &[]), vec![old]);
        let after = objects(&dir);
        assert_eq!(after.len(), 2, "the old level.dat goes, the shared chunk file stays");
        assert!(before.iter().filter(|h| after.contains(h)).count() == 1);
        assert!(!dir.join(OBJECTS).join(".tmp-left").exists());
        fs::write(w.join("level.dat"), b"broken").unwrap();
        restore(&dir, &new).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level 2");

        fs::write(dir.join("20000101-000000-000-manual.zip"), b"not a zip").unwrap();
        delete(&dir, &new).unwrap();
        assert_eq!(objects(&dir).len(), 2, "an unreadable backup might use any content");
        delete(&dir, "20000101-000000-000-manual.zip").unwrap();
        assert!(objects(&dir).is_empty());
    }

    /// 망가진 내용이 있으면 복원이 실패하고 이미 풀어 넣은 경로도 되돌린다
    #[test]
    fn damaged_content_fails_the_restore_and_rolls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let props = tmp.path().join("server.properties");
        fs::write(&props, b"a=1").unwrap();
        let dir = tmp.path().join("backups");
        let b = create(&dir, BackupKind::Manual, &[w.clone(), props.clone()]).unwrap();
        let m = read_v2(&dir.join(&b.name)).unwrap().unwrap();
        let props_hash = &m.files.iter().find(|f| f.entry == "1").unwrap().hash;
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut enc, b"a=9").unwrap();
        fs::write(object_path(&dir, props_hash).unwrap(), enc.finish().unwrap()).unwrap();
        fs::write(w.join("level.dat"), b"current").unwrap();
        fs::write(&props, b"a=2").unwrap();
        assert!(restore(&dir, &b.name).is_err());
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"current");
        assert_eq!(fs::read(&props).unwrap(), b"a=2");
    }

    /// 0.1.8까지 만든 백업도 그대로 목록에 보이고 복원된다
    #[test]
    fn backups_from_before_still_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let b = v1_backup(&dir, std::slice::from_ref(&w));
        assert_eq!(b.size, b.stored);
        create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap();
        fs::write(w.join("level.dat"), b"broken").unwrap();
        restore(&dir, &b.name).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level");
        assert_eq!(backed_up_paths(&dir, &b.name).unwrap(), vec![w]);
    }

    /// D-B12: 이름은 지역 시각이라 서머타임이 끝나는 시간에는 나중 백업의 이름이 앞설 수 있다. 순서는 만든 시각으로 정한다
    #[test]
    fn backups_are_ordered_by_when_they_were_made_not_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let earlier = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        let later = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        // 시계가 한 시간 뒤로 간 뒤 만든 것처럼 이름만 앞으로
        let renamed = "20000101-000000-000-scheduled.zip";
        fs::rename(dir.join(&later), dir.join(renamed)).unwrap();
        assert_eq!(list(&dir).into_iter().map(|b| b.name).collect::<Vec<_>>(), vec![renamed.to_string(), earlier.clone()]);
        assert_eq!(prune(&dir, 1, &[]), vec![earlier]);
    }

    /// 리뷰 603: 열지 못하는 백업이 있으면 그 백업이 쓰는 내용을 지우지 않는다
    #[cfg(unix)]
    #[test]
    fn a_backup_that_cannot_be_opened_keeps_its_content() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let first = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        fs::write(w.join("level.dat"), b"changed").unwrap();
        let second = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        // 다른 프로그램이 잡고 있어 열리지 않는 것처럼: 그 자리를 갈 곳 없는 링크로
        let aside = tmp.path().join("aside.zip");
        fs::rename(dir.join(&first), &aside).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("gone.zip"), dir.join(&first)).unwrap();
        delete(&dir, &second).unwrap();
        fs::remove_file(dir.join(&first)).unwrap();
        fs::rename(&aside, dir.join(&first)).unwrap();
        fs::write(w.join("level.dat"), b"broken").unwrap();
        restore(&dir, &first).unwrap();
        assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level");
    }

    /// 리뷰 603: 망가진 내용 파일은 같은 내용을 다시 읽는 다음 백업이 바꿔 넣는다 (그대로 물려받지 않는다)
    #[test]
    fn a_damaged_content_file_is_replaced_by_the_next_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        // 바뀌지 않은 파일로 다시 읽지 않고 이어 쓰는 경로도 지나게
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        File::options().write(true).open(w.join("level.dat")).unwrap().set_modified(old).unwrap();
        let dir = tmp.path().join("backups");
        let first = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        let m = read_v2(&dir.join(&first)).unwrap().unwrap();
        let level = &m.files.iter().find(|f| f.rel == "level.dat").unwrap().hash;
        fs::write(object_path(&dir, level).unwrap(), b"damaged content").unwrap();
        let second = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        for name in [&first, &second] {
            fs::write(w.join("level.dat"), b"broken").unwrap();
            restore(&dir, name).unwrap();
            assert_eq!(fs::read(w.join("level.dat")).unwrap(), b"level", "{name}");
        }
    }

    /// 리뷰 603: 판 1은 모두 0.1.8까지 만든 것이다. 수정 시각이 새로 찍힌 사본도 이름의 시각보다 새것으로 보지 않는다
    #[test]
    fn a_copied_back_old_backup_does_not_push_out_newer_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let a = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        let copied = "20100101-000000-000-scheduled.zip";
        write_zip(&dir.join(copied), std::slice::from_ref(&w)).unwrap();
        let c = create(&dir, BackupKind::Scheduled, std::slice::from_ref(&w)).unwrap().name;
        assert_eq!(list(&dir).into_iter().map(|b| b.name).collect::<Vec<_>>(), vec![c.clone(), a, copied.to_string()]);
        assert_eq!(prune(&dir, 2, &[&c]), vec![copied.to_string()]);
    }

    #[test]
    fn new_content_is_flushed_on_every_platform() {
        // Windows는 쓰기 권한이 있는 핸들이어야 디스크에 내릴 수 있다
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("object");
        fs::write(&file, b"x").unwrap();
        sync_all(&[file]).unwrap();
    }

    #[test]
    fn content_is_kept_when_the_backup_folder_cannot_be_listed() {
        let tmp = tempfile::tempdir().unwrap();
        // 폴더가 아니라 파일이면 목록을 읽지 못한다
        let not_a_folder = tmp.path().join("backups");
        fs::write(&not_a_folder, b"").unwrap();
        assert!(backup_names(&not_a_folder).is_err());
        let dir = tmp.path().join("ok");
        let w = world(tmp.path());
        let made = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        assert_eq!(backup_names(&dir).unwrap(), vec![made]);
    }

    /// 603 P3-7: 백업 폴더가 백업할 폴더 안에 있으면 지난 백업(내용 파일·zip)을 다시 담아 백업마다 커졌다
    #[test]
    fn the_backup_folder_inside_a_backed_up_folder_is_left_out() {
        let tmp = tempfile::tempdir().unwrap();
        world(tmp.path());
        let dir = backups_dir(tmp.path());
        let paths = [tmp.path().to_path_buf()];
        let first = create(&dir, BackupKind::Manual, &paths).unwrap();
        let second = create(&dir, BackupKind::Manual, &paths).unwrap();
        assert_eq!(second.size, first.size, "the second backup holds the same server files only");
        assert_eq!(second.stored, 0);
    }

    /// 603 P3-8·619 P3-1: 열지 못하는 백업은 쓰지 않는 내용의 정리를 멈춘다. 목록에 표시해 사용자가 알고 지울 수 있게 한다.
    /// 백업 이름을 단 폴더는 백업이 아니다: 목록에도 없고 정리를 막지도 않는다
    #[test]
    fn backups_that_cannot_be_opened_are_marked_and_folders_are_not_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let w = world(tmp.path());
        let dir = tmp.path().join("backups");
        let good = create(&dir, BackupKind::Manual, std::slice::from_ref(&w)).unwrap().name;
        fs::write(dir.join("notes-manual.zip"), b"not a zip").unwrap();
        fs::create_dir_all(dir.join("20200101-000000-000-scheduled.zip")).unwrap();
        let listed: Vec<(String, bool)> = list(&dir).into_iter().map(|b| (b.name, b.readable)).collect();
        assert!(listed.contains(&(good.clone(), true)), "{listed:?}");
        assert!(listed.contains(&("notes-manual.zip".to_string(), false)), "{listed:?}");
        assert_eq!(listed.len(), 2, "{listed:?}");

        delete(&dir, "notes-manual.zip").unwrap();
        delete(&dir, &good).unwrap();
        assert!(objects(&dir).is_empty(), "a folder named like a backup must not keep unused content");
    }

    /// 리뷰 630 P3-2: 매니페스트가 없는 zip과 대상이 없는 링크도 정리를 멈추거나 복원할 수 없으니 열 수 없다고 표시한다
    #[test]
    fn a_zip_without_a_manifest_and_a_dangling_link_are_marked_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("backups");
        fs::create_dir_all(&dir).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(dir.join("20200101-000000-000-manual.zip")).unwrap());
        zip.start_file("other.txt", zip::write::FileOptions::default()).unwrap();
        zip.finish().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("gone.zip"), dir.join("20200102-000000-000-manual.zip")).unwrap();
        let listed: Vec<(String, bool)> = list(&dir).into_iter().map(|b| (b.name, b.readable)).collect();
        assert!(listed.contains(&("20200101-000000-000-manual.zip".to_string(), false)), "{listed:?}");
        #[cfg(unix)]
        {
            assert!(listed.contains(&("20200102-000000-000-manual.zip".to_string(), false)), "{listed:?}");
            // 목록에 보인 것은 지울 수 있다 (링크만 지운다)
            delete(&dir, "20200102-000000-000-manual.zip").unwrap();
            assert!(fs::symlink_metadata(dir.join("20200102-000000-000-manual.zip")).is_err());
        }
    }
}
