//! 설정 › 저장 공간: 이 PC의 내장 디스크와 사바쨩 파일이 차지하는 공간, 서버 폴더를 다른 드라이브로 옮기기
//!
//! - 디스크 목록은 부를 때마다 읽는다(싸다). 이동식·네트워크·읽기 전용 드라이브는 뺀다
//! - 폴더 크기는 오래 걸려 별도 스레드에서 재고, 다 잰 값을 [`Storage`]에 둔다 (`GET /api/system/storage`)
//! - 서버 폴더 옮기기는 [`plan_server_move`]가 원본·대상을 정하고, IPC 작업이 `relocate_watched`로 옮긴다

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::instance::ServerInstance;

/// 이만큼 지난 크기는 다시 잰다
const SIZES_TTL: Duration = Duration::from_secs(300);
/// 옮긴 뒤 대상 드라이브에 남겨 둘 공간
pub const SPARE_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Volume {
    pub mount: String,
    pub name: String,
    /// "ssd" | "hdd" | "unknown"
    pub kind: &'static str,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// 내장 디스크로 볼 것인가: 이동식·읽기 전용·크기 0은 뺀다. 네트워크 파일 시스템은 sysinfo가 대부분 빼지만 남은 것도 뺀다
pub fn is_internal(removable: bool, read_only: bool, total_bytes: u64, file_system: &str) -> bool {
    const NETWORK: [&str; 7] = ["nfs", "nfs4", "cifs", "smbfs", "smb3", "afpfs", "webdav"];
    let fs = file_system.to_ascii_lowercase();
    !removable && !read_only && total_bytes > 0 && !NETWORK.contains(&fs.as_str()) && !fs.starts_with("fuse.sshfs")
}

/// 이 PC의 내장 디스크 (마운트 이름순)
pub fn internal_volumes() -> Vec<Volume> {
    let Ok(disks) = sysinfo::Disks::new_with_refreshed_list() else {
        return Vec::new();
    };
    let mut out: Vec<Volume> = disks
        .iter()
        .filter(|d| {
            is_internal(
                d.is_removable(),
                d.is_read_only(),
                d.total_space(),
                &d.file_system().to_string_lossy(),
            )
        })
        .map(|d| Volume {
            mount: d.mount_point().to_string_lossy().into_owned(),
            name: d.name().to_string_lossy().into_owned(),
            kind: crate::system_monitor::disk_kind(d),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
        })
        .collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out.dedup_by(|a, b| a.mount == b.mount);
    // 유닉스는 한 장치가 여러 자리에 붙는다(바인드 마운트): 가장 짧은 자리 하나만
    if !cfg!(windows) {
        let mut seen: Vec<(String, u64)> = Vec::new();
        out.sort_by_key(|v| v.mount.len());
        out.retain(|v| {
            let key = (v.name.clone(), v.total_bytes);
            let new = !seen.contains(&key);
            seen.push(key);
            new
        });
        out.sort_by(|a, b| a.mount.cmp(&b.mount));
    }
    out
}

/// 모든 디스크의 (마운트 자리, 남은 공간). 공간 확인용이라 내장이 아닌 것도 넣는다
/// (macOS는 `/`가 읽기 전용이고 `/var`·`/Users`가 다른 자리에 붙는다)
pub fn all_disks() -> Vec<(String, u64)> {
    let Ok(disks) = sysinfo::Disks::new_with_refreshed_list() else {
        return Vec::new();
    };
    disks.iter().map(|d| (d.mount_point().to_string_lossy().into_owned(), d.available_space())).collect()
}

/// 경로가 놓일 디스크의 (마운트 자리, 남은 공간). 아직 없는 경로는 있는 위 폴더로, 링크는 풀어서 본다. 모르면 None
pub fn space_at(path: &Path, disks: &[(String, u64)]) -> Option<(String, u64)> {
    let existing = path.ancestors().find(|p| p.exists())?;
    let real = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    let mounts: Vec<String> = disks.iter().map(|(m, _)| m.clone()).collect();
    let mount = crate::system_monitor::mount_for(&real, &mounts)?;
    disks.iter().find(|(m, _)| m == mount).cloned()
}

/// 사바쨩 파일의 갈래 (디스크 막대의 색)
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Servers,
    Backups,
    Components,
    App,
}

/// 사바쨩 파일 하나 (화면의 한 줄)
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    /// "server" | "backups" | "modules" | "extensions" | "app"
    pub kind: &'static str,
    pub category: Category,
    /// 서버면 인스턴스 id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// 있는 디스크 (`volumes[].mount`). 내장 디스크가 아니면 None
    pub mount: Option<String>,
    pub bytes: u64,
}

/// 잴 것: 폴더와 그 안에서 빼고 셀 폴더 (다른 줄이 따로 센다)
#[derive(Debug, Clone)]
pub struct Plan {
    pub instances: Vec<PlannedInstance>,
    pub modules: Option<PathBuf>,
    pub extensions: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub install_dir: Option<PathBuf>,
    pub instances_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PlannedInstance {
    pub id: String,
    pub name: String,
    pub instance_dir: PathBuf,
    pub server_dir: PathBuf,
}

/// 다 잰 사바쨩 파일
#[derive(Debug, Clone, Default)]
pub struct Measured {
    pub items: Vec<Item>,
    pub at_ms: Option<u64>,
}

/// 폴더 크기. `skip` 안은 세지 않는다 (링크는 따라가지 않는다)
fn size_without(path: &Path, skip: &[PathBuf]) -> u64 {
    if skip.is_empty() {
        return crate::utils::dir_size(path);
    }
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let p = entry.path();
            if meta.is_dir() {
                if !skip.iter().any(|s| saba_chan_updater_lib::same_path(s, &p)) {
                    stack.push(p);
                }
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

fn inside(path: &Path, dir: &Path) -> bool {
    saba_chan_updater_lib::paths_overlap(path, dir) && path.components().count() >= dir.components().count()
}

/// 잰다 (막는 작업: 별도 스레드에서). `mounts`: 내장 디스크 마운트
pub fn measure(plan: &Plan, mounts: &[String]) -> Vec<Item> {
    let mount = |p: &Path| crate::system_monitor::mount_for(p, mounts).map(str::to_string);
    let mut items = Vec::new();
    let mut backups: HashMap<Option<String>, u64> = HashMap::new();
    let mut counted: Vec<PathBuf> = Vec::new();
    for inst in &plan.instances {
        let backups_dir = crate::backup::backups_dir(&inst.instance_dir);
        let backup_bytes = crate::utils::dir_size(&backups_dir);
        *backups.entry(mount(&inst.instance_dir)).or_default() += backup_bytes;
        // 인스턴스 폴더(백업 빼고)와 그 밖의 서버 폴더. 다른 서버와 함께 쓰는 폴더는 한 번만 센다
        let mut bytes = size_without(&inst.instance_dir, &[backups_dir]);
        if !inside(&inst.server_dir, &inst.instance_dir)
            && !counted
                .iter()
                .any(|c| saba_chan_updater_lib::paths_overlap(c, &inst.server_dir))
        {
            bytes += crate::utils::dir_size(&inst.server_dir);
            counted.push(inst.server_dir.clone());
        }
        items.push(Item {
            kind: "server",
            category: Category::Servers,
            id: Some(inst.id.clone()),
            name: Some(inst.name.clone()),
            path: Some(inst.server_dir.clone()),
            mount: mount(&inst.server_dir),
            bytes,
        });
    }
    let mut backups: Vec<_> = backups.into_iter().filter(|(_, b)| *b > 0).collect();
    backups.sort_by(|a, b| a.0.cmp(&b.0));
    items.extend(backups.into_iter().map(|(mount, bytes)| Item {
        kind: "backups",
        category: Category::Backups,
        id: None,
        name: None,
        path: None,
        mount,
        bytes,
    }));
    for (kind, dir) in [("modules", &plan.modules), ("extensions", &plan.extensions)] {
        if let Some(dir) = dir {
            items.push(Item {
                kind,
                category: Category::Components,
                id: None,
                name: None,
                path: Some(dir.clone()),
                mount: mount(dir),
                bytes: crate::utils::dir_size(dir),
            });
        }
    }
    // 프로그램·데이터: 설치 폴더와 데이터 폴더(위에서 센 폴더 빼고)
    let skip: Vec<PathBuf> = [
        Some(plan.instances_dir.clone()),
        plan.modules.clone(),
        plan.extensions.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut app: Vec<(Option<String>, u64)> = vec![(mount(&plan.data_dir), size_without(&plan.data_dir, &skip))];
    if let Some(install) = plan.install_dir.as_ref().filter(|i| !inside(i, &plan.data_dir)) {
        let m = mount(install);
        let bytes = size_without(install, &skip);
        match app.iter_mut().find(|(am, _)| *am == m) {
            Some(slot) => slot.1 += bytes,
            None => app.push((m, bytes)),
        }
    }
    items.extend(app.into_iter().filter(|(_, b)| *b > 0).map(|(mount, bytes)| Item {
        kind: "app",
        category: Category::App,
        id: None,
        name: None,
        path: None,
        mount,
        bytes,
    }));
    items
}

/// 디스크마다 사바쨩 파일을 갈래별로 더한다 (화면의 막대)
pub fn per_volume(items: &[Item], mount: &str) -> HashMap<Category, u64> {
    let mut sums = HashMap::new();
    for item in items.iter().filter(|i| i.mount.as_deref() == Some(mount)) {
        *sums.entry(item.category).or_default() += item.bytes;
    }
    sums
}

/// 옮기기 작업 하나 (`GET /api/system/storage/move`)
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MoveJob {
    /// "server" | "modules" | "extensions"
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub target: PathBuf,
    /// "copy" | "verify" | "moving"(진행을 알 수 없는 옮기기) | "done" | "failed" | "canceled"
    pub phase: &'static str,
    pub done_bytes: u64,
    /// 모르면 0
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 옮긴 뒤 원래 자리에서 지우지 못한 것
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_behind: Vec<PathBuf>,
}

impl MoveJob {
    pub fn finished(&self) -> bool {
        matches!(self.phase, "done" | "failed" | "canceled")
    }
}

/// 잰 값과 재는 중인지, 옮기기 작업 (데몬 하나에 하나)
#[derive(Clone, Default)]
pub struct Storage {
    inner: Arc<Mutex<State>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
struct State {
    measured: Measured,
    measured_at: Option<Instant>,
    running: bool,
    job: Option<MoveJob>,
    /// `invalidate`마다 오른다. 재기 전과 다르면 그 측정은 옮기기 중간 값이라 "방금 잼"으로 두지 않는다
    generation: u64,
}

impl Storage {
    /// 지금 잰 값. 오래됐거나 `refresh`면 별도 스레드에서 다시 잰다(기다리지 않는다). 두 번째 값: 재는 중인가
    pub fn get(&self, refresh: bool, plan: impl FnOnce() -> Plan, mounts: Vec<String>) -> (Measured, bool) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let stale = refresh || state.measured_at.is_none_or(|t| t.elapsed() > SIZES_TTL);
        if stale && !state.running {
            state.running = true;
            let plan = plan();
            let shared = self.inner.clone();
            let generation = state.generation;
            std::thread::spawn(move || {
                let items = measure(&plan, &mounts);
                let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                state.measured = Measured {
                    items,
                    at_ms: Some(now_ms()),
                };
                if state.generation == generation {
                    state.measured_at = Some(Instant::now());
                }
                state.running = false;
            });
        }
        (state.measured.clone(), state.running)
    }

    /// 마지막으로 잰 값 (다시 재지 않는다)
    pub fn measured(&self) -> Measured {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).measured.clone()
    }

    /// 파일을 옮겼다: 다음 요청에 다시 잰다
    pub fn invalidate(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.measured_at = None;
        state.generation += 1;
    }

    /// 새 옮기기 작업을 건다. 다른 작업이 진행 중이면 false
    pub fn begin(&self, job: MoveJob) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.job.as_ref().is_some_and(|j| !j.finished()) {
            return false;
        }
        self.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        state.job = Some(job);
        true
    }

    pub fn job(&self) -> Option<MoveJob> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).job.clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut MoveJob)) {
        if let Some(job) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).job.as_mut() {
            f(job);
        }
    }

    /// 진행 중인 복사를 멈추게 한다 (복사·검사 중에만 듣는다)
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn canceled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 폴더 이름으로 쓸 서버 이름 (글자·숫자·`-`·`_`만, 나머지는 `-`). 비면 id
pub fn folder_name(name: &str, id: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        id.to_string()
    } else {
        cleaned
    }
}

/// 드라이브를 고르면 옮길 자리: 인스턴스 폴더가 있는 드라이브면 인스턴스 폴더 안(`subdir`, 직접 고른 폴더·가져온 서버는
/// `instance_mount`를 넘기지 않아 늘 바깥), 아니면
/// `<드라이브>/saba-chan/servers/<서버 이름>` (그 이름이 있으면 `-2`, `-3`…)
pub fn default_server_target(
    mount: &Path,
    instance_dir: &Path,
    instance_mount: Option<&Path>,
    subdir: &str,
    name: &str,
    id: &str,
) -> PathBuf {
    if instance_mount.is_some_and(|m| saba_chan_updater_lib::same_path(m, mount)) {
        return instance_dir.join(subdir);
    }
    let base = mount.join(saba_chan_updater_lib::constants::APP_NAME).join("servers");
    let stem = folder_name(name, id);
    let mut target = base.join(&stem);
    let mut n = 2;
    while std::fs::symlink_metadata(&target).is_ok() {
        target = base.join(format!("{stem}-{n}"));
        n += 1;
    }
    target
}

/// `path`가 `from` 안이면 같은 자리의 `to` 안 경로
pub fn rebase(path: &str, from: &Path, to: &Path) -> Option<String> {
    let rest = Path::new(path).strip_prefix(from).ok()?;
    Some(
        if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        }
        .to_string_lossy()
        .into_owned(),
    )
}

/// 서버 폴더를 옮긴 뒤의 인스턴스: 서버 폴더 안을 가리키던 경로(작업 폴더, 실행 파일, 모듈 설정 값)를 새 폴더로
pub fn rebase_instance(instance: &mut ServerInstance, from: &Path, to: &Path) {
    instance.working_dir = Some(to.to_string_lossy().into_owned());
    if let Some(moved) = instance.executable_path.as_deref().and_then(|p| rebase(p, from, to)) {
        instance.executable_path = Some(moved);
    }
    for value in instance.module_settings.values_mut() {
        if let Some(moved) = value.as_str().and_then(|p| rebase(p, from, to)) {
            *value = serde_json::Value::String(moved);
        }
    }
}

/// 옮길 수 없는 까닭 (`storage_<코드>`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub detail: String,
}

impl Refused {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// 서버 폴더 옮기기 계획
#[derive(Debug, Clone, PartialEq)]
pub struct ServerMove {
    pub source: PathBuf,
    pub target: PathBuf,
    /// 옮긴 뒤에도 사바쨩이 지우지 않는 폴더(직접 고른 폴더·가져온 서버)인가
    pub linked: bool,
}

/// 원본·대상을 정하고 겹침을 본다 (파일 시스템은 원본이 있는지만 본다. 비었는지·쓸 수 있는지는 `relocate`가 본다).
/// `others`: 다른 인스턴스의 인스턴스 폴더와 서버 폴더, `install_dir`: 사바쨩 설치 폴더
pub fn plan_server_move(
    server_dir: &Path,
    instance_dir: &Path,
    target: &Path,
    others: &[PathBuf],
    install_dir: Option<&Path>,
) -> Result<ServerMove, Refused> {
    let shown = || target.display().to_string();
    if !target.is_absolute()
        || target
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Refused::new("not_absolute", shown()));
    }
    // 인스턴스 폴더 자체(또는 그 위)가 서버 폴더면 백업·기록까지 옮기게 된다
    if saba_chan_updater_lib::same_path(server_dir, instance_dir) || inside(instance_dir, server_dir) {
        return Err(Refused::new("not_movable", server_dir.display().to_string()));
    }
    if !server_dir.is_dir() {
        return Err(Refused::new("no_server_folder", server_dir.display().to_string()));
    }
    if target.parent().is_none_or(|p| {
        p.components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count()
            == 0
    }) {
        return Err(Refused::new("root", shown()));
    }
    if saba_chan_updater_lib::same_path(server_dir, target) {
        return Err(Refused::new("same_as_current", shown()));
    }
    if saba_chan_updater_lib::paths_overlap(server_dir, target) {
        return Err(Refused::new("overlaps_current", shown()));
    }
    if others
        .iter()
        .any(|o| saba_chan_updater_lib::paths_overlap(o, target) || saba_chan_updater_lib::paths_overlap(o, server_dir))
    {
        return Err(Refused::new("overlaps_other", shown()));
    }
    if install_dir.is_some_and(|i| saba_chan_updater_lib::paths_overlap(i, target)) {
        return Err(Refused::new("overlaps_install", shown()));
    }
    let linked = crate::instance::migration::linked_folder(Some(&server_dir.to_string_lossy()), instance_dir).is_some();
    // 데이터 폴더 안은 이 인스턴스 폴더 안만 (다른 곳은 사바쨩이 지우거나 덮어쓴다).
    // 직접 고른 폴더·가져온 서버는 인스턴스 폴더 안에도 두지 않는다 (인스턴스를 지울 때 함께 지워진다)
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    if (inside(target, &data_dir) && !inside(target, instance_dir)) || (linked && inside(target, instance_dir)) {
        return Err(Refused::new("overlaps_data", shown()));
    }
    Ok(ServerMove {
        source: server_dir.to_path_buf(),
        target: target.to_path_buf(),
        linked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 재는 사이 옮기기가 끝나 무효화했으면, 늦게 끝난 측정을 "방금 잼"으로 두지 않고 다음 요청에 다시 잰다
    #[test]
    fn a_measurement_started_before_an_invalidation_is_not_kept_as_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..3000 {
            std::fs::write(tmp.path().join(format!("f{i}")), b"x").unwrap();
        }
        let plan = || Plan {
            instances: vec![],
            modules: None,
            extensions: None,
            data_dir: tmp.path().to_path_buf(),
            install_dir: None,
            instances_dir: tmp.path().join("instances"),
        };
        let storage = Storage::default();
        assert!(storage.get(false, plan, vec![]).1, "the first request starts measuring");
        storage.invalidate();
        while storage.inner.lock().unwrap().running {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(storage.get(false, plan, vec![]).1, "measured again after the move");
    }

    /// 아직 없는 자리·링크를 거친 자리도 그 디스크로 찾는다 (macOS의 `/var` → `/private/var`)
    #[test]
    fn space_is_found_through_missing_folders_and_links() {
        let tmp = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(tmp.path()).unwrap();
        let disks = vec![("/nowhere".to_string(), 1), (real.to_string_lossy().into_owned(), 42)];
        assert_eq!(space_at(&tmp.path().join("a").join("b"), &disks).map(|d| d.1), Some(42));
        assert_eq!(space_at(&tmp.path().join("a"), &[("/nowhere".to_string(), 1)]), None);
    }


    #[test]
    fn only_fixed_local_drives_count_as_internal() {
        assert!(is_internal(false, false, 500, "NTFS"));
        assert!(is_internal(false, false, 500, "ext4"));
        assert!(!is_internal(true, false, 500, "exFAT"), "USB·SD");
        assert!(!is_internal(false, true, 500, "squashfs"), "read-only image");
        assert!(!is_internal(false, false, 0, "NTFS"));
        for fs in ["nfs4", "CIFS", "smbfs", "fuse.sshfs"] {
            assert!(!is_internal(false, false, 500, fs), "{fs}");
        }
    }

    #[test]
    fn a_server_moves_back_into_its_instance_folder_or_under_saba_chan_on_another_drive() {
        let tmp = tempfile::tempdir().unwrap();
        let c = tmp.path().join("c");
        let d = tmp.path().join("d");
        let instance_dir = c.join("instances").join("abc");
        assert_eq!(
            default_server_target(&c, &instance_dir, Some(&c), "server", "생존 서버", "abc"),
            instance_dir.join("server")
        );

        let first = default_server_target(&d, &instance_dir, Some(&c), "server", "생존 서버/1", "abc");
        assert_eq!(first, d.join("saba-chan").join("servers").join("생존-서버-1"));
        std::fs::create_dir_all(&first).unwrap();
        assert_eq!(
            default_server_target(&d, &instance_dir, Some(&c), "server", "생존 서버/1", "abc"),
            d.join("saba-chan").join("servers").join("생존-서버-1-2")
        );
        assert_eq!(
            default_server_target(&d, &instance_dir, Some(&c), "server", "///", "abc"),
            d.join("saba-chan").join("servers").join("abc")
        );
    }

    #[test]
    fn moving_a_server_folder_rewrites_paths_that_pointed_into_it() {
        let from = std::env::temp_dir().join("old").join("server");
        let to = std::env::temp_dir().join("new").join("pal");
        let mut inst = ServerInstance::new("pal", "palworld");
        inst.working_dir = Some(from.to_string_lossy().into_owned());
        inst.executable_path = Some(from.join("bin").join("Pal.exe").to_string_lossy().into_owned());
        inst.module_settings.insert(
            "cache_dir".into(),
            serde_json::json!(from.join("cache").to_string_lossy()),
        );
        inst.module_settings
            .insert("java".into(), serde_json::json!("C:/java/bin/java.exe"));
        inst.module_settings.insert("port".into(), serde_json::json!(8211));
        rebase_instance(&mut inst, &from, &to);
        assert_eq!(inst.working_dir.as_deref(), Some(to.to_string_lossy().as_ref()));
        assert_eq!(
            inst.executable_path.as_deref(),
            Some(to.join("bin").join("Pal.exe").to_string_lossy().as_ref())
        );
        assert_eq!(
            inst.module_settings["cache_dir"],
            serde_json::json!(to.join("cache").to_string_lossy())
        );
        assert_eq!(inst.module_settings["java"], serde_json::json!("C:/java/bin/java.exe"));
        assert_eq!(inst.module_settings["port"], serde_json::json!(8211));
        // 이름만 비슷한 옆 폴더는 바꾸지 않는다
        assert_eq!(rebase(&format!("{}2", from.display()), &from, &to), None);
    }

    #[test]
    fn a_server_move_is_refused_where_it_would_take_or_hit_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let instance_dir = tmp.path().join("instances").join("abc");
        let server = instance_dir.join("server");
        std::fs::create_dir_all(&server).unwrap();
        let other = tmp.path().join("instances").join("other");
        let to = |p: &str| tmp.path().join("d").join(p);
        let code = |src: &Path, target: &Path| {
            plan_server_move(src, &instance_dir, target, std::slice::from_ref(&other), None)
                .map(|m| m.linked)
                .map_err(|e| e.code)
        };

        assert_eq!(code(&server, &to("srv")), Ok(false));
        assert_eq!(
            code(&instance_dir, &to("srv")),
            Err("not_movable"),
            "the instance folder holds backups and records"
        );
        assert_eq!(code(&server, Path::new("relative")), Err("not_absolute"));
        assert_eq!(code(&server, &server.join("x")), Err("overlaps_current"));
        assert_eq!(code(&server, &other.join("x")), Err("overlaps_other"));
        assert_eq!(code(&server, &server), Err("same_as_current"));
        assert_eq!(code(&instance_dir.join("gone"), &to("srv")), Err("no_server_folder"));

        // 직접 고른 폴더(사바쨩이 만들지 않은 바깥 폴더)는 옮겨도 사바쨩 것이 되지 않는다
        let picked = tmp.path().join("mine");
        std::fs::create_dir_all(&picked).unwrap();
        assert_eq!(code(&picked, &to("srv")), Ok(true));
        // 인스턴스 폴더 안으로 들이면 인스턴스를 지울 때 함께 지워진다
        assert_eq!(code(&picked, &instance_dir.join("server2")), Err("overlaps_data"));
        saba_chan_updater_lib::component_dirs::mark_owned(&picked).unwrap();
        assert_eq!(
            code(&picked, &to("srv")),
            Ok(false),
            "a folder Saba-chan moved out stays Saba-chan's"
        );
    }

    #[test]
    fn sizes_are_split_by_kind_and_drive_without_counting_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let (c, d) = (tmp.path().join("c"), tmp.path().join("d"));
        let data = c.join("data");
        let instances = data.join("instances");
        let write = |p: PathBuf, n: usize| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![0u8; n]).unwrap();
        };
        write(data.join("config.json"), 10);
        write(data.join("modules").join("mc").join("module.toml"), 20);
        write(instances.join("a").join("server").join("world.dat"), 100);
        write(instances.join("a").join("backups").join("1.zip"), 30);
        write(d.join("srv-b").join("world.dat"), 200);
        write(instances.join("b").join("instance.json"), 5);
        let plan = Plan {
            instances: vec![
                PlannedInstance {
                    id: "a".into(),
                    name: "A".into(),
                    instance_dir: instances.join("a"),
                    server_dir: instances.join("a").join("server"),
                },
                PlannedInstance {
                    id: "b".into(),
                    name: "B".into(),
                    instance_dir: instances.join("b"),
                    server_dir: d.join("srv-b"),
                },
            ],
            modules: Some(data.join("modules")),
            extensions: None,
            data_dir: data.clone(),
            install_dir: None,
            instances_dir: instances.clone(),
        };
        let mounts = vec![c.to_string_lossy().into_owned(), d.to_string_lossy().into_owned()];
        let items = measure(&plan, &mounts);
        let get = |kind: &str, id: Option<&str>| {
            items
                .iter()
                .find(|i| i.kind == kind && i.id.as_deref() == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(get("server", Some("a")).bytes, 100);
        assert_eq!(get("server", Some("b")).bytes, 205);
        assert_eq!(get("server", Some("b")).mount.as_deref(), Some(mounts[1].as_str()));
        assert_eq!(get("backups", None).bytes, 30);
        assert_eq!(get("modules", None).bytes, 20);
        assert_eq!(
            get("app", None).bytes,
            10,
            "the data folder minus instances and modules"
        );
        let c_sums = per_volume(&items, &mounts[0]);
        assert_eq!(
            (
                c_sums[&Category::Servers],
                c_sums[&Category::Backups],
                c_sums[&Category::Components],
                c_sums[&Category::App]
            ),
            (100, 30, 20, 10)
        );
    }
}
