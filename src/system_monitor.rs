//! 시스템 모니터: 기계 전체 CPU·RAM·디스크 IO·디스크 용량·네트워크와 그중 인스턴스 몫.
//!
//! 표본은 클라이언트가 `/api/system/metrics`를 부를 때만 뜬다(백그라운드 루프 없음).
//! 인스턴스 몫은 추적 중인 PID와 그 자손 프로세스를 합친 값이다.
//! - 디스크 IO: OS가 기계 전체 값을 따로 주지 않아 모든 프로세스의 IO 합을 전체로 쓴다.
//!   Windows의 프로세스 IO 카운터는 디스크 밖 IO(파이프 등)도 센다.
//! - 네트워크: 프로세스별 값을 싸게 얻을 길이 없어(Windows는 ETW·관리자 권한 필요) 기계 전체만 준다.
//! - 디스크 용량의 인스턴스 몫은 폴더 크기를 따로 훑어 10분 동안 재사용한다. 요약만 보는 요청은 훑지 않는다.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, System};

/// 이보다 잦은 요청은 직전 표본을 그대로 준다 (여러 클라이언트가 함께 봐도 표본은 하나)
const MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(900);
/// 직전 표본이 이보다 오래됐으면 짧은 간격으로 두 번 떠서 "지금" 값을 만든다
const STALE_AFTER: Duration = Duration::from_secs(10);
/// 인스턴스 폴더 크기 재사용 시간 (월드·백업 폴더는 파일이 수만 개라 자주 훑지 않는다)
const DIR_SIZE_TTL: Duration = Duration::from_secs(600);
/// 디스크 목록 갱신 간격
const DISK_LIST_TTL: Duration = Duration::from_secs(60);

/// 핸들러가 supervisor에서 복사해 넘기는 인스턴스 정보
#[derive(Debug, Clone)]
pub struct InstanceRoot {
    pub id: String,
    pub name: String,
    pub pid: Option<u32>,
    pub dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub sampled_at_ms: u64,
    /// 비율(초당 바이트, CPU %)을 잰 구간 길이
    pub window_ms: u64,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disk_io: DiskIoMetrics,
    pub network: NetworkMetrics,
    pub disks: Vec<DiskMetrics>,
    pub instances: Vec<InstanceMetrics>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuMetrics {
    pub cores: usize,
    /// 기계 전체 사용률 (0~100). 유휴 = 100 - total_percent
    pub total_percent: f32,
    /// 인스턴스가 아닌 프로세스 몫
    pub others_percent: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub others_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskIoMetrics {
    pub read_bps: u64,
    pub write_bps: u64,
    pub others_read_bps: u64,
    pub others_write_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkMetrics {
    pub rx_bps: u64,
    pub tx_bps: u64,
    /// 인스턴스별 네트워크는 아직 못 잰다 (항상 false)
    pub per_instance: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskMetrics {
    pub mount: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    /// 사용 중 - 인스턴스 폴더 합
    pub others_bytes: u64,
    pub instances: Vec<DiskShare>,
    /// 인스턴스 폴더 크기를 잰 시각 (아직 못 쟀으면 None)
    pub sizes_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskShare {
    pub id: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceMetrics {
    pub id: String,
    pub name: String,
    pub running: bool,
    pub process_count: usize,
    /// 기계 전체 기준 (0~100)
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
}

/// 프로세스 하나의 이번 구간 값 (sysinfo와 분리해 시험할 수 있게)
#[derive(Debug, Clone, Default)]
struct ProcSample {
    parent: Option<u32>,
    /// 코어 하나 기준 % (sysinfo 관례)
    cpu: f32,
    memory: u64,
    read: u64,
    written: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Usage {
    processes: usize,
    cpu: f32,
    memory: u64,
    read: u64,
    written: u64,
}

impl Usage {
    fn add(&mut self, p: &ProcSample) {
        self.processes += 1;
        self.cpu += p.cpu;
        self.memory += p.memory;
        self.read += p.read;
        self.written += p.written;
    }
}

/// 각 프로세스를 조상 중 가장 가까운 인스턴스 PID에 붙인다. 어느 인스턴스에도 안 붙으면 `others`.
/// 반환: (인스턴스 id → 합계, 나머지 합계)
fn attribute(procs: &HashMap<u32, ProcSample>, roots: &[(String, u32)]) -> (HashMap<String, Usage>, Usage) {
    let root_of: HashMap<u32, &str> = roots.iter().map(|(id, pid)| (*pid, id.as_str())).collect();
    let mut owner: HashMap<u32, Option<&str>> = HashMap::new();
    let mut by_instance: HashMap<String, Usage> = roots.iter().map(|(id, _)| (id.clone(), Usage::default())).collect();
    let mut others = Usage::default();

    for (&pid, sample) in procs {
        // 조상을 따라 올라가며 처음 만나는 인스턴스 PID가 주인. 고리(잘못된 parent)는 길이로 끊는다
        let mut chain = Vec::new();
        let mut cur = Some(pid);
        let found = loop {
            let Some(p) = cur else { break None };
            if let Some(known) = owner.get(&p) {
                break *known;
            }
            if let Some(id) = root_of.get(&p) {
                break Some(*id);
            }
            if chain.len() > 64 {
                break None;
            }
            chain.push(p);
            cur = procs.get(&p).and_then(|s| s.parent);
        };
        for p in chain {
            owner.insert(p, found);
        }
        owner.insert(pid, found);
        match found {
            Some(id) => by_instance.entry(id.to_string()).or_default().add(sample),
            None => others.add(sample),
        }
    }
    (by_instance, others)
}

/// `path`가 들어 있는 마운트 지점 (가장 긴 접두어)
fn mount_for<'a>(path: &Path, mounts: &'a [String]) -> Option<&'a str> {
    let norm = |p: &str| {
        let mut s = p.replace('\\', "/").to_lowercase();
        if !s.ends_with('/') {
            s.push('/');
        }
        s
    };
    let target = norm(&path.to_string_lossy());
    mounts
        .iter()
        .filter(|m| target.starts_with(&norm(m)))
        .max_by_key(|m| m.len())
        .map(|m| m.as_str())
}

/// 겹친 폴더는 바깥 것만 남긴다 (두 번 세지 않게)
fn outermost(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if out.iter().any(|o| d.starts_with(o)) {
            continue;
        }
        out.retain(|o| !o.starts_with(d));
        out.push(d.clone());
    }
    out
}

/// 폴더 크기 (심볼릭 링크는 따라가지 않는다)
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn per_sec(bytes: u64, window: Duration) -> u64 {
    let secs = window.as_secs_f64();
    if secs <= 0.0 {
        0
    } else {
        (bytes as f64 / secs) as u64
    }
}

struct Sampler {
    system: System,
    networks: Networks,
    disks: Disks,
    disks_listed_at: Option<Instant>,
    last_refresh: Option<Instant>,
    last: Option<(Instant, MetricsSnapshot)>,
}

#[derive(Default)]
struct DirSizes {
    measured_at: Option<(Instant, u64)>,
    running: bool,
    /// 인스턴스 id → (폴더, 바이트)
    sizes: HashMap<String, Vec<(PathBuf, u64)>>,
}

/// IPC 상태에 붙는 모니터. 복제해도 같은 표본을 나눠 쓴다.
#[derive(Clone)]
pub struct SystemMonitor {
    sampler: Arc<Mutex<Option<Sampler>>>,
    dir_sizes: Arc<Mutex<DirSizes>>,
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor {
    pub fn new() -> Self {
        // sysinfo 초기화는 첫 요청 때 한다: 아무도 안 보면 비용이 없다
        Self { sampler: Arc::new(Mutex::new(None)), dir_sizes: Arc::new(Mutex::new(DirSizes::default())) }
    }

    /// 표본을 뜬다. 동기 시스템 콜이므로 `spawn_blocking` 안에서 부른다.
    /// `measure_dirs`가 false면(요약만 보는 화면) 폴더 크기를 새로 재지 않고 마지막 값을 쓴다.
    pub fn sample(&self, roots: &[InstanceRoot], extra_dirs: &[PathBuf], measure_dirs: bool) -> MetricsSnapshot {
        if measure_dirs {
            self.refresh_dir_sizes_if_stale(roots);
        }
        let mut guard = self.sampler.lock().unwrap_or_else(|e| e.into_inner());
        let sampler = guard.get_or_insert_with(|| Sampler {
            system: System::new(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new(),
            disks_listed_at: None,
            last_refresh: None,
            last: None,
        });
        if let Some((at, snap)) = &sampler.last {
            if at.elapsed() < MIN_SAMPLE_INTERVAL {
                return snap.clone();
            }
        }
        let fresh = sampler.last_refresh.is_none_or(|t| t.elapsed() > STALE_AFTER);
        if fresh {
            // 비율은 두 번 뜬 차이로 구한다: 오래 쉬었으면 짧은 구간을 새로 만든다
            Self::refresh(sampler);
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
        }
        let window = sampler.last_refresh.map(|t| t.elapsed()).unwrap_or_default();
        Self::refresh(sampler);
        if sampler.disks_listed_at.is_none_or(|t| t.elapsed() > DISK_LIST_TTL) {
            sampler.disks.refresh_list();
            sampler.disks_listed_at = Some(Instant::now());
        } else {
            sampler.disks.refresh();
        }
        let sizes = self.dir_sizes.lock().unwrap_or_else(|e| e.into_inner());
        let snap = build_snapshot(sampler, roots, extra_dirs, &sizes, window);
        drop(sizes);
        sampler.last = Some((Instant::now(), snap.clone()));
        snap
    }

    fn refresh(sampler: &mut Sampler) {
        sampler.system.refresh_cpu_usage();
        sampler.system.refresh_memory();
        sampler
            .system
            .refresh_processes_specifics(ProcessRefreshKind::new().with_cpu().with_memory().with_disk_usage());
        sampler.networks.refresh();
        sampler.last_refresh = Some(Instant::now());
    }

    /// 폴더 크기가 오래됐으면 별도 스레드에서 다시 잰다 (응답은 기다리지 않는다)
    fn refresh_dir_sizes_if_stale(&self, roots: &[InstanceRoot]) {
        let mut state = self.dir_sizes.lock().unwrap_or_else(|e| e.into_inner());
        let stale = state.measured_at.is_none_or(|(t, _)| t.elapsed() > DIR_SIZE_TTL);
        if state.running || !stale {
            return;
        }
        state.running = true;
        drop(state);
        let jobs: Vec<(String, Vec<PathBuf>)> =
            roots.iter().map(|r| (r.id.clone(), outermost(&r.dirs))).collect();
        let shared = self.dir_sizes.clone();
        std::thread::spawn(move || {
            let sizes: HashMap<String, Vec<(PathBuf, u64)>> = jobs
                .into_iter()
                .map(|(id, dirs)| {
                    let measured = dirs.into_iter().map(|d| {
                        let size = dir_size(&d);
                        (d, size)
                    });
                    (id, measured.collect())
                })
                .collect();
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            state.sizes = sizes;
            state.measured_at = Some((Instant::now(), now_ms()));
            state.running = false;
        });
    }
}

fn build_snapshot(
    sampler: &Sampler,
    roots: &[InstanceRoot],
    extra_dirs: &[PathBuf],
    sizes: &DirSizes,
    window: Duration,
) -> MetricsSnapshot {
    let sys = &sampler.system;
    let cores = sys.cpus().len().max(1);
    let procs: HashMap<u32, ProcSample> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let io = p.disk_usage();
            (
                pid.as_u32(),
                ProcSample {
                    parent: p.parent().map(Pid::as_u32),
                    cpu: p.cpu_usage(),
                    memory: p.memory(),
                    read: io.read_bytes,
                    written: io.written_bytes,
                },
            )
        })
        .collect();
    let pids: Vec<(String, u32)> = roots
        .iter()
        .filter_map(|r| r.pid.filter(|p| procs.contains_key(p)).map(|p| (r.id.clone(), p)))
        .collect();
    let (by_instance, others) = attribute(&procs, &pids);

    let instances: Vec<InstanceMetrics> = roots
        .iter()
        .map(|r| {
            let u = by_instance.get(&r.id).cloned().unwrap_or_default();
            InstanceMetrics {
                id: r.id.clone(),
                name: r.name.clone(),
                running: u.processes > 0,
                process_count: u.processes,
                cpu_percent: u.cpu / cores as f32,
                memory_bytes: u.memory,
                disk_read_bps: per_sec(u.read, window),
                disk_write_bps: per_sec(u.written, window),
            }
        })
        .collect();

    let inst_cpu: f32 = instances.iter().map(|i| i.cpu_percent).sum();
    // 프로세스 합과 전체 값은 재는 시점이 조금 달라 어긋난다: 전체가 인스턴스 합보다 작아지지 않게 맞춘다
    let total_cpu = sys.global_cpu_info().cpu_usage().clamp(0.0, 100.0).max(inst_cpu.min(100.0));
    let inst_mem: u64 = instances.iter().map(|i| i.memory_bytes).sum();
    let total_mem = sys.total_memory();
    let used_mem = total_mem.saturating_sub(sys.available_memory()).max(inst_mem.min(total_mem));

    let others_read = per_sec(others.read, window);
    let others_write = per_sec(others.written, window);
    let disk_io = DiskIoMetrics {
        read_bps: others_read + instances.iter().map(|i| i.disk_read_bps).sum::<u64>(),
        write_bps: others_write + instances.iter().map(|i| i.disk_write_bps).sum::<u64>(),
        others_read_bps: others_read,
        others_write_bps: others_write,
    };

    let (mut rx, mut tx) = (0u64, 0u64);
    for (name, data) in sampler.networks.iter() {
        let lname = name.to_lowercase();
        if lname == "lo" || lname.contains("loopback") {
            continue;
        }
        rx += data.received();
        tx += data.transmitted();
    }

    MetricsSnapshot {
        sampled_at_ms: now_ms(),
        window_ms: window.as_millis() as u64,
        cpu: CpuMetrics { cores, total_percent: total_cpu, others_percent: (total_cpu - inst_cpu).max(0.0) },
        memory: MemoryMetrics { total_bytes: total_mem, used_bytes: used_mem, others_bytes: used_mem.saturating_sub(inst_mem) },
        disk_io,
        network: NetworkMetrics { rx_bps: per_sec(rx, window), tx_bps: per_sec(tx, window), per_instance: false },
        disks: build_disks(&sampler.disks, roots, extra_dirs, sizes),
        instances,
    }
}

/// 데이터 폴더나 인스턴스 폴더가 있는 디스크만 보여 준다
fn build_disks(disks: &Disks, roots: &[InstanceRoot], extra_dirs: &[PathBuf], sizes: &DirSizes) -> Vec<DiskMetrics> {
    let mut mounts: Vec<(String, u64, u64)> = disks
        .iter()
        .map(|d| (d.mount_point().to_string_lossy().to_string(), d.total_space(), d.available_space()))
        .filter(|(_, total, _)| *total > 0)
        .collect();
    mounts.sort_by(|a, b| a.0.cmp(&b.0));
    mounts.dedup_by(|a, b| a.0 == b.0);
    let names: Vec<String> = mounts.iter().map(|m| m.0.clone()).collect();

    let mut wanted: Vec<&str> = Vec::new();
    let all_dirs = roots.iter().flat_map(|r| r.dirs.iter()).chain(extra_dirs.iter());
    for dir in all_dirs {
        if let Some(m) = mount_for(dir, &names) {
            if !wanted.contains(&m) {
                wanted.push(m);
            }
        }
    }

    let mut out: Vec<DiskMetrics> = mounts
        .iter()
        .filter(|(m, _, _)| wanted.contains(&m.as_str()))
        .map(|(mount, total, avail)| {
            let mut shares: Vec<DiskShare> = Vec::new();
            for r in roots {
                let bytes: u64 = sizes
                    .sizes
                    .get(&r.id)
                    .map(|dirs| {
                        dirs.iter().filter(|(d, _)| mount_for(d, &names) == Some(mount.as_str())).map(|(_, b)| *b).sum()
                    })
                    .unwrap_or(0);
                if bytes > 0 {
                    shares.push(DiskShare { id: r.id.clone(), bytes });
                }
            }
            let used = total.saturating_sub(*avail);
            let inst: u64 = shares.iter().map(|s| s.bytes).sum();
            DiskMetrics {
                mount: mount.clone(),
                total_bytes: *total,
                available_bytes: *avail,
                others_bytes: used.saturating_sub(inst),
                instances: shares,
                sizes_at_ms: sizes.measured_at.map(|(_, ms)| ms),
            }
        })
        .collect();
    // 데이터 폴더가 있는 디스크가 먼저 (extra_dirs의 첫 항목이 데이터 폴더)
    let primary = extra_dirs.first().and_then(|d| mount_for(d, &names)).map(String::from);
    out.sort_by_key(|d| Some(&d.mount) != primary.as_ref());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parent: Option<u32>, cpu: f32, memory: u64, read: u64) -> ProcSample {
        ProcSample { parent, cpu, memory, read, written: 0 }
    }

    #[test]
    fn instance_share_covers_its_whole_process_tree_and_nothing_else() {
        // 1(init) ─ 10(runner) ─ 11(java) ─ 12(child)
        //         └ 20(other game, instance b)
        //         └ 30(browser)
        let procs: HashMap<u32, ProcSample> = [
            (1, p(None, 1.0, 10, 0)),
            (10, p(Some(1), 0.5, 5, 0)),
            (11, p(Some(10), 150.0, 2000, 300)),
            (12, p(Some(11), 10.0, 100, 50)),
            (20, p(Some(1), 40.0, 800, 0)),
            (30, p(Some(1), 20.0, 1500, 7)),
        ]
        .into();
        let roots = vec![("a".to_string(), 11), ("b".to_string(), 20), ("gone".to_string(), 99)];
        let (by, others) = attribute(&procs, &roots);

        assert_eq!(by["a"], Usage { processes: 2, cpu: 160.0, memory: 2100, read: 350, written: 0 });
        assert_eq!(by["b"].processes, 1);
        assert_eq!(by["gone"], Usage::default());
        // 러너(10)와 init은 인스턴스 몫이 아니다
        assert_eq!(others, Usage { processes: 3, cpu: 21.5, memory: 1515, read: 7, written: 0 });
    }

    #[test]
    fn parent_cycles_do_not_hang() {
        let procs: HashMap<u32, ProcSample> = [(5, p(Some(6), 1.0, 1, 0)), (6, p(Some(5), 1.0, 1, 0))].into();
        let (_, others) = attribute(&procs, &[]);
        assert_eq!(others.processes, 2);
    }

    #[test]
    fn paths_map_to_their_longest_mount() {
        let mounts = vec!["/".to_string(), "/data".to_string(), r"C:\".to_string(), r"D:\".to_string()];
        assert_eq!(mount_for(Path::new("/data/saba/instances/a"), &mounts), Some("/data"));
        assert_eq!(mount_for(Path::new("/database"), &mounts), Some("/"));
        assert_eq!(mount_for(Path::new(r"d:\Games\srv"), &mounts), Some(r"D:\"));
        assert_eq!(mount_for(Path::new(r"E:\x"), &[r"C:\".to_string()]), None);
    }

    #[test]
    fn nested_instance_dirs_are_counted_once() {
        let dirs = vec![PathBuf::from("/s/a/world"), PathBuf::from("/s/a"), PathBuf::from("/t")];
        assert_eq!(outermost(&dirs), vec![PathBuf::from("/s/a"), PathBuf::from("/t")]);
    }

    #[test]
    fn dir_size_sums_files_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/b.bin"), vec![0u8; 23]).unwrap();
        assert_eq!(dir_size(tmp.path()), 123);
        assert_eq!(dir_size(&tmp.path().join("missing")), 0);
    }

    #[test]
    fn sample_reports_live_machine_totals() {
        let monitor = SystemMonitor::new();
        let me = std::process::id();
        let roots = vec![InstanceRoot { id: "me".into(), name: "test".into(), pid: Some(me), dirs: vec![] }];
        let snap = monitor.sample(&roots, &[], false);
        assert!(snap.memory.total_bytes > 0);
        assert!(snap.memory.used_bytes >= snap.memory.others_bytes);
        assert!(snap.cpu.total_percent <= 100.0 && snap.cpu.total_percent >= snap.cpu.others_percent);
        assert!(snap.window_ms > 0);
        let inst = &snap.instances[0];
        assert!(inst.running && inst.memory_bytes > 0);
        // 바로 다시 부르면 같은 표본
        assert_eq!(monitor.sample(&roots, &[], false).sampled_at_ms, snap.sampled_at_ms);
        // 요약 요청은 폴더 크기를 재지 않는다
        assert!(monitor.dir_sizes.lock().unwrap().measured_at.is_none() && !monitor.dir_sizes.lock().unwrap().running);
    }
}
