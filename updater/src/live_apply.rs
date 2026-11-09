//! 설치기 `--apply`가 데몬을 바꾼 뒤 새 데몬이 건강한지 지켜보고, 아니면 이번 묶음을 되돌린다 (C절, 설계 9·13절)
//!
//! 흐름: 예전 데몬이 끝나길 기다림(`wait_for_previous`, 30초 넘으면 파일을 바꾸지 않음) → 코어가 바뀌는 묶음이면
//! 데이터 스냅샷·`.old` 목록·감시 표시(`LiveApply::prepare`) → 적용 → 다시 띄움 → `/health` 감시(`watch`) →
//! 건강하면 스냅샷과 `.old`를 지우고(`LiveApply::healthy`), 아니면 새 데몬을 끝내고 되돌린다(`LiveApply::roll_back`).

use super::*;
use crate::config_file::ConfigFile;
use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `--wait-pid`로 받은 예전 데몬이 끝나길 기다리는 상한. 넘으면 파일을 바꾸지 않는다
pub const WAIT_PID_TIMEOUT: Duration = Duration::from_secs(30);
/// 파일을 바꾸지 않기로 한 뒤 예전 데몬이 마저 끝나길 더 기다리는 상한. 끝나야 다시 띄운다
/// (데몬 종료는 릴레이 정리·`daemon.shutdown` hook 때문에 30초를 넘길 수 있다)
pub const LATE_EXIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// 다시 띄운 뒤 새 데몬이 `/health`에 목표 판으로 답하기까지의 상한.
/// 잠정값이다: PC 실측(모듈·익스텐션이 많은 설치의 기동 시간)으로 조정한다
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
/// 처음 건강하게 답한 뒤 그 프로세스가 더 살아 있어야 하는 시간 (곧바로 죽는 판을 거른다)
pub const SURVIVAL_WINDOW: Duration = Duration::from_secs(10);
/// `/health`를 묻는 간격
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 감시 중 표시 (staging 폴더). 새 데몬의 `cleanup_old_files`가 그동안 `.old`를 지우지 않게 한다
const WATCH_FILE: &str = "apply-watch.json";
/// 데이터 스냅샷 폴더 (`<data dir>/update-snapshot/<이전 판>/`)
pub const SNAPSHOT_DIR: &str = "update-snapshot";
/// 스냅샷에 넣는 데이터 폴더 바로 아래 파일: 설정과 설치 기록. 포함 목록이다 (제외 목록으로 바꾸지 않는다, 13절)
pub const SNAPSHOT_DATA_FILES: &[&str] = &["config.json", "installed-manifest.json", "installed-files.json", "official-components.json"];
/// 그중 설치 기록. 되돌릴 때 되돌리는 실행 파일의 항목만 되돌린다 (`merge_install_record`)
const SNAPSHOT_RECORD_FILES: &[&str] = &["installed-manifest.json", "installed-files.json", "official-components.json"];
/// 스냅샷에 넣는 인스턴스 폴더 바로 아래 파일
pub const SNAPSHOT_INSTANCE_ROOT_FILES: &[&str] = &["order.json"];
/// 스냅샷에 넣는 인스턴스마다의 파일 (인스턴스 기록). 러너 기록(`runner.json`)·`managed.pid`·서버 파일·백업은 넣지 않는다:
/// 되돌릴 때 그것까지 되돌리면 켜진 서버에 다시 붙지 못한다
pub const SNAPSHOT_INSTANCE_FILES: &[&str] = &["instance.json", "settings.json"];

// ═══════════════════════════════════════════════════════
// 예전 데몬 기다리기
// ═══════════════════════════════════════════════════════

/// `pid`가 `timeout` 안에 끝나면 true
pub fn wait_for_exit(pid: u32, timeout: Duration, poll: Duration, is_running: impl Fn(u32) -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !is_running(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// 예전 데몬(`--wait-pid`)이 끝나길 기다린다. 상한 안에 끝나지 않으면 파일을 바꾸지 않기로 하고
/// 완료 마커에 `deferred`(`wait_pid_timeout`)를 남긴 뒤 false. 받아 둔 목록은 그대로 둔다 (다음에 다시 적용한다)
pub fn wait_for_previous(pid: Option<u32>, timeout: Duration, is_running: impl Fn(u32) -> bool, marker_path: &Path) -> bool {
    let Some(pid) = pid else {
        // PID를 모르면 예전처럼 잠깐만 기다린다
        std::thread::sleep(Duration::from_millis(500));
        return true;
    };
    if wait_for_exit(pid, timeout, POLL_INTERVAL.min(timeout), is_running) {
        tracing::info!("[Apply] PID {} exited", pid);
        return true;
    }
    tracing::warn!("[Apply] PID {} is still running after {:?}; leaving the files as they are", pid, timeout);
    if let Err(e) = UpdateCompletionMarker::deferred("wait_pid_timeout").save_to(marker_path) {
        tracing::warn!("[Apply] Cannot record the deferred update: {}", e);
    }
    false
}

/// 파일을 바꾸지 않기로 한 뒤(`wait_for_previous`가 false) 다시 띄워도 되는가: 예전 데몬이 `timeout` 안에 마저 끝나면 true.
/// 끝나지 않으면 다시 띄우지 않는다: 새로 띄운 데몬은 단일 인스턴스 잠금에 막혀 끝나고, 그 뒤 예전 데몬마저 끝나면
/// 아무 데몬도 남지 않는다. 끝나지 않는 예전 데몬은 그대로 둔다
pub fn may_relaunch_unchanged(pid: Option<u32>, timeout: Duration, is_running: impl Fn(u32) -> bool) -> bool {
    let Some(pid) = pid else { return true };
    if wait_for_exit(pid, timeout, POLL_INTERVAL.min(timeout), is_running) {
        tracing::info!("[Apply] PID {} exited late; relaunching without the update", pid);
        return true;
    }
    tracing::warn!("[Apply] PID {} is still running after {:?}; not relaunching another daemon", pid, timeout);
    false
}

// ═══════════════════════════════════════════════════════
// 건강 확인
// ═══════════════════════════════════════════════════════

/// `/health`의 답 중 판정에 쓰는 것
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Health {
    pub version: String,
    pub pid: u32,
    /// 데몬 프로세스가 시작한 시각 (Unix ms). 이 필드가 없는 옛 데몬도 있다
    #[serde(default)]
    pub started_at_ms: Option<u64>,
}

/// 어느 데몬을 "새 데몬"으로 보는가. 200 응답만으로는 판단하지 않는다:
/// 예전 데몬이 아직 안 끝났거나 다른 데몬이 포트를 잡고 있어도 200이 온다
#[derive(Debug, Clone, PartialEq)]
pub enum HealthRule {
    /// 업데이터가 데몬을 직접 띄웠다: 그 PID여야 한다
    SpawnedPid(u32),
    /// 다른 것(GUI·서비스 관리자)이 데몬을 띄운다: 다시 띄운 뒤에 시작했고 예전 데몬이 아니어야 한다
    StartedAfter { since_ms: u64, old_pid: Option<u32> },
}

impl HealthRule {
    pub fn accepts(&self, health: &Health, target_version: &str) -> bool {
        same_version(&health.version, target_version) && self.is_new_process(health)
    }

    /// 판과 상관없이 다시 띄운 뒤의 데몬 프로세스인가 (시간 안에 건강해지지 않으면 끝낼 것)
    fn is_new_process(&self, health: &Health) -> bool {
        match self {
            Self::SpawnedPid(pid) => health.pid == *pid,
            Self::StartedAfter { since_ms, old_pid } => {
                Some(health.pid) != *old_pid && health.started_at_ms.is_some_and(|t| t >= *since_ms)
            }
        }
    }
}

fn same_version(a: &str, b: &str) -> bool {
    a.trim().trim_start_matches('v') == b.trim().trim_start_matches('v')
}

/// 지금 시각 (Unix ms). `HealthRule::StartedAfter`의 기준으로 다시 띄우기 직전에 잰다
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 감시 상한
#[derive(Debug, Clone, Copy)]
pub struct WatchLimits {
    pub timeout: Duration,
    pub survival: Duration,
    pub poll: Duration,
}

impl Default for WatchLimits {
    fn default() -> Self {
        Self { timeout: HEALTH_TIMEOUT, survival: SURVIVAL_WINDOW, poll: POLL_INTERVAL }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WatchOutcome {
    Healthy { pid: u32 },
    /// `pid`: 목표 판으로 답한 새 데몬, 시간이 다 됐으면 그사이 답한 새 프로세스 (끝낼 것). `reason`: `timeout`·`exited`·`replaced`
    Unhealthy { pid: Option<u32>, reason: &'static str },
}

/// 새 데몬을 보는 길. 실제는 `HttpProbe`, 테스트는 가짜
pub trait Probe {
    fn health(&self) -> impl Future<Output = Option<Health>> + Send;
    fn alive(&self, pid: u32) -> bool;
}

/// 새 데몬이 `rule`대로 목표 판으로 답하고, 그 뒤 생존 창 동안 살아 있으면 건강하다
pub async fn watch(probe: &impl Probe, target_version: &str, rule: &HealthRule, limits: WatchLimits) -> WatchOutcome {
    let deadline = Instant::now() + limits.timeout;
    // 목표 판으로 답하지 않았어도 다시 띄운 뒤의 프로세스가 답했으면 시간이 다 됐을 때 끝낸다
    let mut late = None;
    loop {
        let answer = probe.health().await;
        if let Some(h) = answer.as_ref().filter(|h| rule.is_new_process(h)) {
            late = Some(h.pid);
        }
        if let Some(health) = answer.filter(|h| rule.accepts(h, target_version)) {
            let pid = health.pid;
            let until = Instant::now() + limits.survival;
            loop {
                if !probe.alive(pid) {
                    tracing::warn!("[Apply] The new daemon (PID {}) exited within the survival window", pid);
                    return WatchOutcome::Unhealthy { pid: Some(pid), reason: "exited" };
                }
                // 잠깐 답하지 못하는 것은 봐준다. 다른 프로세스가 답하면 새 데몬이 포트를 잃은 것이다
                if let Some(other) = probe.health().await.filter(|h| h.pid != pid) {
                    tracing::warn!("[Apply] PID {} answers instead of the new daemon (PID {})", other.pid, pid);
                    return WatchOutcome::Unhealthy { pid: Some(pid), reason: "replaced" };
                }
                if Instant::now() >= until {
                    return WatchOutcome::Healthy { pid };
                }
                tokio::time::sleep(limits.poll).await;
            }
        }
        if Instant::now() >= deadline {
            return WatchOutcome::Unhealthy { pid: late, reason: "timeout" };
        }
        tokio::time::sleep(limits.poll).await;
    }
}

/// 이 PC 데몬의 `/health` (토큰 없이 열려 있다). 포트는 데몬과 같은 순서로 정한다:
/// `SABA_IPC_PORT` → `config.json`의 `gui.ipcPort` → 기본값
pub struct HttpProbe {
    url: String,
    http: reqwest::Client,
}

impl HttpProbe {
    pub fn local() -> Self {
        let port = std::env::var("SABA_IPC_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .or_else(|| {
                ConfigFile::default_location()
                    .peek_section("gui")
                    .get("ipcPort")
                    .and_then(|v| v.as_u64())
                    .and_then(|p| u16::try_from(p).ok())
            })
            .unwrap_or(crate::constants::DEFAULT_IPC_PORT);
        Self::at(&format!("http://127.0.0.1:{}", port))
    }

    pub fn at(base: &str) -> Self {
        // 시스템 프록시를 타지 않는다 (127.0.0.1)
        let http = crate::tls::http_client_builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap_or_default();
        Self { url: format!("{}/health", base.trim_end_matches('/')), http }
    }
}

/// 업데이터가 직접 띄운 프로세스 (`keep_child`). Unix에서 끝난 자식은 거두기 전까지 좀비로 남아 `kill -0`이 살아 있다고 답한다:
/// 그 PID는 이 손잡이로 끝났는지 보고 거둔다
static CHILDREN: std::sync::Mutex<Vec<std::process::Child>> = std::sync::Mutex::new(Vec::new());

/// 띄운 자식 프로세스를 맡아 두고 그 PID를 준다 (`process_running`이 끝났는지 본다)
pub fn keep_child(child: std::process::Child) -> u32 {
    let pid = child.id();
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner()).push(child);
    pid
}

/// 프로세스가 살아 있는가. 맡아 둔 자식이면 손잡이로 보고, 끝났으면 거둔다
pub fn process_running(pid: u32) -> bool {
    let mut children = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = children.iter().position(|c| c.id() == pid) {
        return match children[i].try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                // 이미 거뒀다: 남은 상태를 바로 돌려받는다
                let _ = children.remove(i).wait();
                false
            }
            Err(_) => ProcessChecker::is_pid_running(pid),
        };
    }
    drop(children);
    ProcessChecker::is_pid_running(pid)
}

impl Probe for HttpProbe {
    fn health(&self) -> impl Future<Output = Option<Health>> + Send {
        let request = self.http.get(&self.url).send();
        async move {
            let resp = request.await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            resp.json::<Health>().await.ok()
        }
    }

    fn alive(&self, pid: u32) -> bool {
        process_running(pid)
    }
}

/// 프로세스 하나를 끝낸다 (자식은 건드리지 않는다: 러너·게임 서버는 데몬과 따로 산다).
/// Unix는 TERM을 보내고 5초 안에 끝나지 않으면 KILL
pub fn stop_process(pid: u32) {
    if !process_running(pid) {
        return;
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new(crate::process::system_tool("taskkill"));
        let _ = crate::process::hide_console(&mut cmd).args(["/PID", &pid.to_string(), "/F"]).output();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output();
        if !wait_for_exit(pid, Duration::from_secs(5), Duration::from_millis(200), process_running) {
            let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).output();
        }
    }
    wait_for_exit(pid, Duration::from_secs(5), Duration::from_millis(200), process_running);
}

/// 데몬 단일 인스턴스 잠금(`<data dir>/daemon.lock`)을 쥔 쪽
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockHolder {
    Free,
    Pid(u32),
    /// 잠겨 있는데 쥔 PID를 읽지 못했다 (막 잠근 데몬이 아직 적지 않았다)
    Unknown,
}

/// 지금 데몬 잠금을 쥔 데몬. 잠금을 잠깐 걸어 보고 걸리면 곧바로 푼다 (데몬의 `instance_lock`과 같은 파일·같은 잠금)
pub fn daemon_lock_holder(data_dir: &Path) -> LockHolder {
    let path = data_dir.join("daemon.lock");
    let file = match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LockHolder::Free,
        // 열지 못했다(권한·폴더 등): 쥔 쪽을 모른다. 비어 있다고 보면 새 데몬이 쥔 채 되돌릴 수 있다
        Err(_) => return LockHolder::Unknown,
    };
    match file.try_lock() {
        Ok(()) => LockHolder::Free,
        Err(std::fs::TryLockError::WouldBlock) => std::fs::read_to_string(data_dir.join("daemon.lock.owner"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("pid=").and_then(|p| p.trim().parse().ok())))
            // 잠금을 막 쥔 데몬이 아직 적지 않았으면 끝난 예전 데몬의 PID가 남아 있다: 살아 있는 PID만 믿는다
            .filter(|pid| process_running(*pid))
            .map_or(LockHolder::Unknown, LockHolder::Pid),
        Err(std::fs::TryLockError::Error(_)) => LockHolder::Unknown,
    }
}

/// 새 데몬을 끝내는 차례 수 (서비스 관리자가 그사이 다시 띄울 수 있다)
const STOP_ROUNDS: usize = 3;

/// 되돌리기 전에 새 판 데몬을 모두 끝낸다: 감시가 본 새 프로세스(`seen`)와 데몬 잠금을 쥔 데몬 중 예전 데몬이 아닌 것.
/// GUI·서비스 관리자가 띄운 새 데몬은 업데이터가 PID를 모르므로 잠금으로 찾는다. 잠금을 쥔 새 데몬이 남지 않아야 true:
/// 그때만 파일과 스냅샷을 되돌린다(새 판 데몬이 잠금·포트를 쥔 채 이전 판 파일이 섞이지 않게). 러너·게임 서버는 건드리지 않는다
pub fn stop_new_daemons(seen: Option<u32>, old_pid: Option<u32>, holder: impl Fn() -> LockHolder, stop: impl Fn(u32)) -> bool {
    if let Some(pid) = seen.filter(|p| Some(*p) != old_pid) {
        stop(pid);
    }
    for round in 0..=STOP_ROUNDS {
        match holder() {
            LockHolder::Free => return true,
            LockHolder::Pid(pid) if Some(pid) == old_pid => return true,
            LockHolder::Pid(pid) if round < STOP_ROUNDS => {
                tracing::warn!("[Apply] Stopping the new daemon (PID {}) that holds the daemon lock", pid);
                stop(pid);
            }
            LockHolder::Unknown if round < STOP_ROUNDS => std::thread::sleep(Duration::from_millis(500)),
            _ => {}
        }
    }
    false
}

// ═══════════════════════════════════════════════════════
// 감시 중 표시
// ═══════════════════════════════════════════════════════

/// 감시를 시작한다고 적는다 (이 프로세스의 PID). 적지 못하면 오류: 그러면 새 데몬이 `.old`를 지워 되돌릴 수 없다
pub fn begin_watch(staging_dir: &Path) -> std::io::Result<()> {
    let body = serde_json::json!({ "pid": std::process::id() }).to_string();
    write_atomic(&staging_dir.join(WATCH_FILE), body.as_bytes())
}

/// 손으로 되살려야 하는 동안 `.old`를 붙잡아 둔다 (PID 없이 `hold`). 다음 적용의 `begin_watch`가 덮고, 그 적용이 끝나면 걷힌다
pub fn hold_backups(staging_dir: &Path) {
    let body = serde_json::json!({ "hold": true }).to_string();
    if let Err(e) = write_atomic(&staging_dir.join(WATCH_FILE), body.as_bytes()) {
        tracing::error!("[Apply] Cannot hold the .old backups for the manual recovery: {}", e);
    }
}

pub fn end_watch(staging_dir: &Path) {
    let _ = std::fs::remove_file(staging_dir.join(WATCH_FILE));
}

/// 업데이터가 아직 새 데몬을 지켜보는 중인가 (표시를 남긴 프로세스가 살아 있다), 또는 손으로 되살릴 때까지 붙잡아 뒀는가
/// (`hold_backups`). 그동안 `.old`는 되돌림에 쓴다
pub fn watch_in_progress(staging_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(staging_dir.join(WATCH_FILE)) else { return false };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    if value.get("hold").and_then(serde_json::Value::as_bool) == Some(true) {
        return true;
    }
    Some(value)
        .and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
        .and_then(|p| u32::try_from(p).ok())
        .is_some_and(ProcessChecker::is_pid_running)
}

// ═══════════════════════════════════════════════════════
// 데이터 스냅샷
// ═══════════════════════════════════════════════════════

/// 적용 직전에 뜬 설정·인스턴스 기록 사본. 새 데몬이 형식을 바꾼 뒤 죽어도 이전 데몬이 읽을 수 있게 되돌린다.
/// 데이터 폴더 안(원본과 같은 권한)에 두고, 성공을 확인하면 지운다 (설정 파일에는 토큰이 들어 있다)
#[derive(Debug)]
pub struct DataSnapshot {
    dir: PathBuf,
    data_dir: PathBuf,
    instances_dir: PathBuf,
}

impl DataSnapshot {
    pub fn take(data_dir: &Path, instances_dir: &Path, from_version: &str) -> std::io::Result<Self> {
        let root = data_dir.join(SNAPSHOT_DIR);
        let name: String = from_version
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+') { c } else { '_' })
            .collect();
        let name = if name.is_empty() || name.starts_with('.') { format!("v{name}") } else { name };
        // 사본마다 새 이름이다: 앞서 남긴 사본(되살리지 못한 것, 끊긴 실행의 것)을 같은 판이라고 지우지 않는다
        let stamp = now_ms();
        let name = (0..)
            .map(|n| if n == 0 { format!("{name}-{stamp}") } else { format!("{name}-{stamp}-{n}") })
            .find(|n| !root.join(n).exists() && !root.join(format!("{n}.partial")).exists())
            .unwrap_or_default();
        let dir = root.join(&name);
        // 다 뜬 뒤 이름을 바꾼다: `dir`이 있으면 언제나 온전한 사본이다
        let partial = root.join(format!("{name}.partial"));
        create_private_dir(&root)?;
        sweep_stale_partials(&root);
        // 뜨다 실패하면 반쯤 뜬 사본을 지운다: 다음 시도는 새 이름이라 실패할 때마다 쌓인다
        if let Err(e) = Self::copy_into(data_dir, instances_dir, &partial).and_then(|()| std::fs::rename(&partial, &dir)) {
            if let Err(cleanup) = std::fs::remove_dir_all(&partial)
                && cleanup.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("[Apply] Cannot remove the partial snapshot {}: {}", partial.display(), cleanup);
            }
            let _ = std::fs::remove_dir(&root); // 비었을 때만
            return Err(e);
        }
        tracing::info!("[Apply] Data snapshot taken at {}", dir.display());
        Ok(Self { dir, data_dir: data_dir.to_path_buf(), instances_dir: instances_dir.to_path_buf() })
    }

    /// 포함 목록의 파일을 `partial`에 뜬다
    fn copy_into(data_dir: &Path, instances_dir: &Path, partial: &Path) -> std::io::Result<()> {
        create_private_dir(partial)?;
        for file in SNAPSHOT_DATA_FILES {
            copy_if_file(&data_dir.join(file), &partial.join("data").join(file))?;
        }
        for file in SNAPSHOT_INSTANCE_ROOT_FILES {
            copy_if_file(&instances_dir.join(file), &partial.join("instances").join(file))?;
        }
        // 없는 것만 건너뛴다. 읽지 못하면(권한·입출력 오류) 사본이 모자라니 멈춘다
        if let Some(entries) = read_dir_if_exists(instances_dir)? {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                // `.saba-state`(실행 기록) 같은 점 폴더는 인스턴스가 아니다
                if name.to_string_lossy().starts_with('.') || !std::fs::metadata(entry.path())?.is_dir() {
                    continue;
                }
                for file in SNAPSHOT_INSTANCE_FILES {
                    copy_if_file(&entry.path().join(file), &partial.join("instances").join(&name).join(file))?;
                }
            }
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// 사본을 제자리에 되돌린다 (사본에 있는 파일만)
    /// 설치 기록(`SNAPSHOT_RECORD_FILES`)은 되돌리는 실행 파일의 항목만 되돌린다 (`merge_install_record`)
    pub fn restore(&self) -> std::io::Result<()> {
        let data_root = self.dir.join("data");
        restore_tree(&data_root, &self.data_dir, &|src, dst, copy| {
            let is_record = src.parent() == Some(data_root.as_path())
                && src.file_name().and_then(|n| n.to_str()).is_some_and(|n| SNAPSHOT_RECORD_FILES.contains(&n));
            if !is_record {
                return Ok(copy);
            }
            let current = match std::fs::read(dst) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(copy),
                Err(e) => return Err(e),
            };
            Ok(merge_install_record(&copy, &current).unwrap_or(copy))
        })?;
        restore_tree(&self.dir.join("instances"), &self.instances_dir, &|_, _, copy| Ok(copy))?;
        tracing::info!("[Apply] Data restored from {}", self.dir.display());
        Ok(())
    }

    /// 지우지 않고 남긴다 (옮기지 않는다: 옮기다 실패할 일이 없고, 다음 스냅샷은 새 이름이라 덮지 않는다)
    pub fn keep(self) -> KeptCopy {
        let copy = KeptCopy { dir: self.dir, data_dir: self.data_dir, instances_dir: self.instances_dir };
        tracing::error!("[Apply] The data snapshot is kept for a manual restore: {}", copy.restore_hint());
        copy
    }

    pub fn discard(self) {
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            tracing::warn!("[Apply] Cannot remove the data snapshot {}: {}", self.dir.display(), e);
        }
        if let Some(root) = self.dir.parent() {
            let _ = std::fs::remove_dir(root); // 비었을 때만
        }
    }
}

/// 남긴 사본과 손으로 되살릴 곳: 사본의 `data/`는 데이터 폴더에, `instances/`는 인스턴스 폴더에 덮어쓴다
#[derive(Debug, Clone, PartialEq)]
pub struct KeptCopy {
    pub dir: PathBuf,
    pub data_dir: PathBuf,
    pub instances_dir: PathBuf,
}

impl KeptCopy {
    /// 손으로 되살릴 실행 파일 백업(`.old`)을 사본 폴더(`exes/<n>/<제자리 파일 이름>`)로 옮긴다(다른 볼륨이면 복사). `.old`는 다음 적용이 비키며
    /// 덮고 정리가 지우므로, 안내는 이 사본을 가리킨다 (사본 폴더는 자동으로 지우지 않는다). 뜨지 못한 것은 `.old`를
    /// 그대로 가리키고 `false`를 돌려준다: 호출자는 `.old`를 붙잡아 둔다 (`hold_backups`)
    pub fn preserve_exes(&self, exes: Vec<(PathBuf, PathBuf)>) -> (Vec<(PathBuf, PathBuf)>, bool) {
        let mut all = true;
        let exes = exes
            .into_iter()
            .enumerate()
            .map(|(i, (backup, target))| {
                let Some(name) = target.file_name() else {
                    all = false;
                    return (backup, target);
                };
                let copy = self.dir.join("exes").join(i.to_string()).join(name);
                // 옮기기를 먼저 한다: 같은 볼륨이면 공간이 들지 않는다 (디스크가 찼을 때도). 다른 볼륨이면 복사한다
                let moved = copy.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&backup, &copy));
                match moved.or_else(|_| copy_whole(&backup, &copy)) {
                    Ok(()) => (copy, target),
                    Err(e) => {
                        tracing::error!("[Apply] Cannot keep a copy of {}: {}", backup.display(), e);
                        all = false;
                        (backup, target)
                    }
                }
            })
            .collect();
        (exes, all)
    }

    /// 마커·기록에 남기는 안내 (영어 원문, 화면은 번역한 문구를 쓴다)
    pub fn restore_hint(&self) -> String {
        format!(
            "copy {} into {} and {} into {}",
            self.dir.join("data").display(),
            self.data_dir.display(),
            self.dir.join("instances").display(),
            self.instances_dir.display()
        )
    }
}

/// 끊긴 실행(전원·강제 종료)이 남긴 반쯤 뜬 사본(`*.partial`)을 지운다. 한 시간이 지난 것만: 지금 뜨는 중인 것은 건드리지 않는다.
/// 온전한 사본은 지우지 않는다
fn sweep_stale_partials(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = entry.file_name().to_string_lossy().ends_with(".partial")
            && entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= STALE_PARTIAL_AGE));
        if stale && let Err(e) = std::fs::remove_dir_all(&path) {
            tracing::warn!("[Apply] Cannot remove the stale partial snapshot {}: {}", path.display(), e);
        }
    }
}

const STALE_PARTIAL_AGE: Duration = Duration::from_secs(60 * 60);

/// 주인만 여는 폴더 (Unix 0700). Windows는 데이터 폴더의 권한을 잇는다
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// 폴더를 연다. 없으면 None, 읽지 못하면 오류 (없는 것과 가른다)
fn read_dir_if_exists(dir: &Path) -> std::io::Result<Option<std::fs::ReadDir>> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(Some(entries)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// 파일이면 옮겨 뜬다. 없으면 건너뛰고, 읽지 못하면 오류
fn copy_if_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::metadata(from) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

/// 다 옮겨 뜬 뒤에만 그 이름에 둔다 (실패하면 반쯤 뜬 파일을 지운다)
fn copy_whole(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut partial = to.as_os_str().to_os_string();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let result = to
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::copy(from, &partial))
        .and_then(|_| std::fs::rename(&partial, to));
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// 되살릴 파일마다 쓸 내용을 정한다: (사본 경로, 제자리, 사본 내용) → 쓸 내용
type RestoreContents<'a> = &'a dyn Fn(&Path, &Path, Vec<u8>) -> std::io::Result<Vec<u8>>;

/// 사본의 파일을 제자리에 쓴다
fn restore_tree(from: &Path, to: &Path, contents: RestoreContents) -> std::io::Result<()> {
    // 사본에 그 갈래가 없으면(뜰 때 없던 것) 건너뛴다. 읽지 못하면 오류: 되돌림이 빠졌는데 성공으로 보지 않는다
    let Some(entries) = read_dir_if_exists(from)? else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if std::fs::metadata(&src)?.is_dir() {
            restore_tree(&src, &dst, contents)?;
        } else {
            let body = contents(&src, &dst, std::fs::read(&src)?)?;
            write_atomic(&dst, &body)?;
        }
    }
    Ok(())
}

/// 되돌리는 것(실행 파일: 코어·GUI·CLI·업데이터·설치기)의 항목은 사본에서, 나머지(모듈·익스텐션·번역, 모르는 키)는 지금 기록에서
/// 가져온다. 되돌림은 실행 파일만이라 나머지 파일은 새 판 그대로 남는다. 키→값 표와 키 목록 모두 다룬다. 읽지 못하면 None
fn merge_install_record(copy: &[u8], current: &[u8]) -> Option<Vec<u8>> {
    let rolled_back = |key: &str| {
        matches!(
            Component::try_from_manifest_key(key),
            Some(Component::CoreDaemon | Component::Cli | Component::Gui | Component::Updater | Component::Installer)
        )
    };
    let copy: serde_json::Value = serde_json::from_slice(copy).ok()?;
    let current: serde_json::Value = serde_json::from_slice(current).ok()?;
    let merged = match (copy, current) {
        (serde_json::Value::Object(copy), serde_json::Value::Object(current)) => serde_json::Value::Object(
            copy.into_iter()
                .filter(|(k, _)| rolled_back(k))
                .chain(current.into_iter().filter(|(k, _)| !rolled_back(k)))
                .collect(),
        ),
        (serde_json::Value::Array(copy), serde_json::Value::Array(current)) => serde_json::Value::Array(
            copy.into_iter()
                .filter(|k| k.as_str().is_some_and(rolled_back))
                .chain(current.into_iter().filter(|k| !k.as_str().is_some_and(rolled_back)))
                .collect(),
        ),
        _ => return None,
    };
    serde_json::to_vec_pretty(&merged).ok()
}

// ═══════════════════════════════════════════════════════
// 실패한 판과 보고
// ═══════════════════════════════════════════════════════

/// 되돌린 판을 업데이트 설정(`config.json`의 `updates.failed_release`)에 적는다. 완료 마커는 읽으면 지워지므로
/// 같은 판을 다시 자동 적용하지 않게 하는 기록은 여기에 둔다. 다른 키는 그대로 둔다
pub fn record_failed_release(config: &ConfigFile, version: &str) -> std::io::Result<()> {
    config
        .update(|doc| {
            let updates = doc.entry("updates").or_insert_with(|| serde_json::json!({}));
            if !updates.is_object() {
                *updates = serde_json::json!({});
            }
            updates["failed_release"] = serde_json::json!(version);
        })
        .map(|_| ())
}

/// 저장된 업데이트 설정 (없거나 깨졌으면 기본값)
pub fn saved_update_config(config: &ConfigFile) -> UpdateConfig {
    let mut section = config.section("updates");
    // 0.1.1 이름과 새 이름이 같이 있으면 별칭이 겹친다: 새 이름을 쓴다
    if let Some(obj) = section.as_object_mut().filter(|o| o.contains_key("relay_url")) {
        obj.remove("relay_base_url");
    }
    serde_json::from_value(section).unwrap_or_default()
}

/// 되돌렸다고 릴레이에 익명으로 한 번 알린다 (`POST /v1/updates/report`). 베타 판을 정식으로 올릴지 판단하는 데 쓴다.
/// 실패해도 무시한다
pub async fn report_rolled_back(relay_url: Option<&str>, version: &str, channel: &str) {
    let base = relay_url.unwrap_or(crate::relay::DEFAULT_RELAY_URL).trim_end_matches('/');
    let body = serde_json::json!({
        "version": version,
        "channel": channel,
        "platform": crate::relay::current_platform(),
        "outcome": "rolled_back",
    });
    let Ok(http) = crate::tls::http_client_builder().timeout(Duration::from_secs(5)).build() else { return };
    // 릴레이는 클라이언트 표시가 없는 요청을 받지 않는다 (`RelayClient`와 같은 헤더)
    let request = http.post(format!("{base}/v1/updates/report")).header(crate::relay::CLIENT_HEADER.0, crate::relay::CLIENT_HEADER.1);
    match request.json(&body).send().await {
        Ok(resp) if resp.status().is_success() => tracing::info!("[Apply] Reported the rollback of {}", version),
        Ok(resp) => tracing::debug!("[Apply] Rollback report answered {}", resp.status()),
        Err(e) => tracing::debug!("[Apply] Rollback report failed: {}", e),
    }
}

// ═══════════════════════════════════════════════════════
// 한 번의 적용 감시
// ═══════════════════════════════════════════════════════

/// 코어가 바뀌는 적용 하나. 적용 전에 `prepare`, 새 데몬을 지켜본 뒤 `healthy` 또는 `roll_back`,
/// 지켜볼 것이 없으면 `skip`으로 끝낸다
#[derive(Debug)]
pub struct LiveApply {
    staging_dir: PathBuf,
    before: ExeBackups,
    snapshot: DataSnapshot,
    pub from_version: String,
    /// 적용하는 코어 판 (`/health`로 견주고, 되돌리면 `failed_release`와 마커 `version`에 적는다)
    pub target_version: String,
    /// 적용하는 릴리즈 판 (받아 둔 릴리즈). 되돌림 보고는 이 판으로 한다: 부분 릴리즈는 코어 판이 릴리즈 판과 다르다
    pub release_version: Option<String>,
}

impl LiveApply {
    /// 받아 둔 것 중 이번에 적용할 코어가 있으면 준비한다: 데이터 스냅샷, 지금 있는 `.old` 목록, 감시 표시.
    /// 코어가 바뀌지 않으면 `Ok(None)`. 준비하지 못하면 `Err`: 적용하지 않는다 (설치기가 마커에 `deferred`(`snapshot_failed`)를 남긴다)
    pub fn prepare(manager: &mut UpdateManager, data_dir: &Path, instances_dir: &Path) -> Result<Option<Self>, String> {
        let Some((from_version, target_version)) = manager.staged_core_change() else { return Ok(None) };
        let release_version = manager.pending_release().map(|r| r.version);
        // 적용 전 백업 목록을 읽지 못하면 되돌릴 때 이번 것과 예전 것을 가를 수 없다
        let before = manager.try_exe_backups().map_err(|e| format!("Cannot list the exe backups: {e}"))?;
        Self::start(&manager.staging_dir, before, data_dir, instances_dir, from_version, target_version, release_version).map(Some)
    }

    /// 스냅샷을 뜨고 감시 표시를 남긴다. 스냅샷 없이 적용하면 새 데몬이 데이터를 바꾼 뒤 죽었을 때 실행 파일만 되돌아가고,
    /// 감시 표시가 없으면 새 데몬이 `.old`를 지워 되돌릴 수 없으므로 어느 쪽이든 실패하면 멈춘다
    pub fn start(
        staging_dir: &Path,
        before: ExeBackups,
        data_dir: &Path,
        instances_dir: &Path,
        from_version: String,
        target_version: String,
        release_version: Option<String>,
    ) -> Result<Self, String> {
        let snapshot = DataSnapshot::take(data_dir, instances_dir, &from_version).map_err(|e| format!("Cannot take the data snapshot: {e}"))?;
        if let Err(e) = begin_watch(staging_dir) {
            snapshot.discard();
            return Err(format!("Cannot mark the health watch: {e}"));
        }
        Ok(Self { staging_dir: staging_dir.to_path_buf(), before, snapshot, from_version, target_version, release_version })
    }

    /// 되돌림을 릴레이에 알릴 판: 릴리즈 판, 모르면 코어 판
    pub fn report_version(&self) -> &str {
        self.release_version.as_deref().unwrap_or(&self.target_version)
    }

    /// 새 데몬이 건강하다: 스냅샷과 이번 `.old`를 지운다
    pub fn healthy(self, manager: &UpdateManager) {
        self.snapshot.discard();
        end_watch(&self.staging_dir);
        manager.cleanup_old_files();
    }

    /// 지켜볼 것이 없다: 코어를 적용하지 못했거나(그 전에 멈췄거나 되돌려짐) 다시 띄운 것이 없다.
    /// `.old`는 예전처럼 다음 정리(`cleanup_old_files`)가 지운다
    pub fn skip(self) {
        self.snapshot.discard();
        end_watch(&self.staging_dir);
    }

    /// 새 데몬을 끝내지 못해 새 판을 그대로 둔다. 새 데몬이 이미 데이터를 바꿨을 수 있으니 사본은 남기고(`keep`) `.old`는
    /// 붙잡아 둔다(`hold_backups`). 완료 마커는 `success:false`·`result:"left_in_place"`·`reason`(감시 결과)과 되살리는 법으로 바꾼다:
    /// 새 데몬 끝내기 → 실행 파일(백업 → 제자리) → 데이터
    pub fn leave_in_place(self, manager: &UpdateManager, marker_path: &Path, reason: &str) -> Recovery {
        // 백업 목록을 읽지 못하면 어느 `.old`가 이번 것인지 모르니 모두 붙잡아 둔다
        let (exes, listed) = match manager.exe_rollbacks_since(&self.before) {
            Ok(exes) => (exes, true),
            Err(e) => {
                tracing::error!("[Apply] Cannot list the exe backups: {}", e);
                (Vec::new(), false)
            }
        };
        let missing_core = !has_core(&exes);
        let copy = self.snapshot.keep();
        let (exes, preserved) = copy.preserve_exes(exes);
        if !(listed && preserved) {
            hold_backups(&self.staging_dir);
        } else {
            end_watch(&self.staging_dir);
        }
        let recovery = Recovery { stop_first: true, missing_core, exes, copy, install_dir: manager.install_root.clone() };
        let previous = UpdateCompletionMarker::load_from(marker_path).unwrap_or_else(|| UpdateCompletionMarker::failure(String::new()));
        let marker = UpdateCompletionMarker {
            timestamp: chrono::Utc::now().to_rfc3339(),
            success: false,
            message: Some(format!("{} did not start properly and could not be stopped; to undo, {}", self.target_version, recovery.restore_hint())),
            result: Some("left_in_place".into()),
            reason: Some(reason.into()),
            version: Some(self.target_version.clone()),
            acknowledged: false,
            ..previous
        };
        if let Err(e) = marker.save_to(marker_path) {
            tracing::error!("[Apply] Cannot record the failed update: {}", e);
        }
        recovery
    }

    /// 새 데몬은 이미 끝냈다고 보고 되돌린다: 이번 묶음의 실행 파일 전부를 `.old`에서, 데이터를 스냅샷에서.
    /// 그다음 `failed_release`를 적고 완료 마커를 `rolled_back`으로 바꾼다 (이 순서: 스냅샷의 설정이 기록을 덮지 않게).
    /// 실행 파일(코어 백업이 없는 것 포함)이나 데이터를 되돌리지 못했으면 사본과 `.old`를 남기고 마커의 `reason`을
    /// `restore_failed`로 적은 뒤 되살리는 법(`Recovery`)을 준다: 섞인 판을 띄우지 않게 호출자는 다시 띄우지 않는다
    pub fn roll_back(self, manager: &UpdateManager, config: &ConfigFile, marker_path: &Path, reason: &str) -> Option<Recovery> {
        let mut listed = true;
        let (exes, missing_core) = match manager.exe_rollbacks_since(&self.before) {
            Err(e) => {
                tracing::error!("[Apply] Cannot list the exe backups to roll back: {}", e);
                listed = false;
                (Vec::new(), true)
            }
            Ok(plan) => {
                let missing_core = !has_core(&plan);
                match manager.roll_back_exes_since(&self.before) {
                    Ok(done) => {
                        tracing::warn!("[Apply] Rolled back {} ({}): {:?}", self.target_version, reason, done.restored);
                        (done.pending, missing_core)
                    }
                    Err(e) => {
                        tracing::error!("[Apply] Cannot roll back the exes: {}", e);
                        (plan, missing_core)
                    }
                }
            }
        };
        // 실행 파일을 되돌리지 못했어도 데이터는 되돌려 둔다 (사람이 실행 파일만 옮기면 되게). 사본은 그때 남긴다
        let data_restored = match self.snapshot.restore() {
            Ok(()) => true,
            Err(e) => {
                tracing::error!("[Apply] Cannot restore the data snapshot {}: {}", self.snapshot.path().display(), e);
                false
            }
        };
        if let Err(e) = record_failed_release(config, &self.target_version) {
            tracing::error!("[Apply] Cannot record the failed release {}: {}", self.target_version, e);
        }
        let previous = UpdateCompletionMarker::load_from(marker_path);
        if data_restored && exes.is_empty() && !missing_core {
            let marker = UpdateCompletionMarker::rolled_back(&self.target_version, reason, previous);
            if let Err(e) = marker.save_to(marker_path) {
                tracing::error!("[Apply] Cannot record the rollback: {}", e);
            }
            self.snapshot.discard();
            end_watch(&self.staging_dir);
            return None;
        }
        let copy = self.snapshot.keep();
        let (exes, preserved) = copy.preserve_exes(exes);
        if !(listed && preserved) {
            hold_backups(&self.staging_dir);
        } else {
            end_watch(&self.staging_dir);
        }
        let recovery = Recovery { stop_first: false, missing_core, exes, copy, install_dir: manager.install_root.clone() };
        let mut marker = UpdateCompletionMarker::rolled_back(&self.target_version, "restore_failed", previous);
        marker.message = Some(format!("Could not fully roll back {} ({}); {}", self.target_version, reason, recovery.restore_hint()));
        if let Err(e) = marker.save_to(marker_path) {
            tracing::error!("[Apply] Cannot record the rollback: {}", e);
        }
        Some(recovery)
    }
}

/// 실행 파일 되돌리기 결과: 되돌린 것과, 되돌리지 못해 손으로 옮길 것 (백업 → 제자리)
#[derive(Debug, Default, PartialEq)]
pub struct ExeRollback {
    pub restored: Vec<PathBuf>,
    pub pending: Vec<(PathBuf, PathBuf)>,
}

/// 되돌릴 목록에 코어(`saba-core`)가 있는가
fn has_core(exes: &[(PathBuf, PathBuf)]) -> bool {
    exes.iter().any(|(_, target)| target.file_stem().is_some_and(|n| n == "saba-core"))
}

/// 자동으로 끝내지 못해 손으로 되살릴 것. 차례: (새 데몬 끝내기) → 실행 파일(백업 → 제자리) → 데이터(`copy`)
#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    /// 새 판 데몬이 아직 떠 있다: 먼저 사바쨩과 saba-core를 끝내야 한다
    pub stop_first: bool,
    /// 손으로 옮길 실행 파일 (백업 → 제자리)
    pub exes: Vec<(PathBuf, PathBuf)>,
    /// 코어 백업을 찾지 못했다 (이전 판 코어는 다시 설치해야 한다)
    pub missing_core: bool,
    pub copy: KeptCopy,
    pub install_dir: PathBuf,
}

impl Recovery {
    /// 마커·기록에 남기는 안내 (영어 원문, 화면은 번역한 문구를 쓴다)
    pub fn restore_hint(&self) -> String {
        let mut steps = Vec::new();
        if self.stop_first {
            steps.push("quit Saba-chan and end every saba-core process".to_string());
        }
        for (backup, target) in &self.exes {
            steps.push(format!("replace {} with {}", target.display(), backup.display()));
        }
        if self.missing_core {
            steps.push(format!("no backup of saba-core was found in {}; reinstall the previous version", self.install_dir.display()));
        }
        steps.push(self.copy.restore_hint());
        steps.join("; then ")
    }
}

/// 설치 폴더의 실행 파일 백업과 그 파일의 정체 (적용 전후를 견줘 이번에 생긴 백업만 고른다)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExeBackups(Vec<(PathBuf, FileId)>);

/// 같은 이름을 다시 쓴 백업(`.old`를 지우고 새로 비켜 둠)도 다른 것으로 본다
type FileId = (u64, u64);

fn file_id(path: &Path) -> Option<FileId> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        Some((meta.len(), modified))
    }
}

impl UpdateManager {
    /// 받아 둔 것 중 이번에 적용할 코어의 (지금 판, 새 판). 적용 대상 목록이 있으면 거기 든 것만 본다 (목록은 지우지 않는다)
    pub fn staged_core_change(&mut self) -> Option<(String, String)> {
        self.load_pending_manifest().ok()?;
        let core = Component::CoreDaemon.manifest_key();
        let targets: Vec<String> = std::fs::read_to_string(self.staging_dir.join("apply-targets.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if !targets.is_empty() && !targets.contains(&core) {
            return None;
        }
        let comp = self.status.components.iter().find(|c| c.component == Component::CoreDaemon && c.downloaded && c.update_available)?;
        Some((comp.current_version.clone(), comp.latest_version.clone()?))
    }

    /// 지금 설치 폴더에 있는 실행 파일 백업 (`old_exe_backups`)
    pub fn exe_backups(&self) -> ExeBackups {
        ExeBackups(self.old_exe_backups().into_iter().filter_map(|(path, ..)| file_id(&path).map(|id| (path, id))).collect())
    }

    /// `exe_backups`와 같되 설치 폴더를 읽지 못하면 오류
    pub fn try_exe_backups(&self) -> std::io::Result<ExeBackups> {
        Ok(ExeBackups(self.try_old_exe_backups()?.into_iter().filter_map(|(path, ..)| file_id(&path).map(|id| (path, id))).collect()))
    }
}

impl ExeBackups {
    pub(crate) fn contains(&self, path: &Path, id: Option<FileId>) -> bool {
        self.0.iter().any(|(p, i)| p == path && Some(*i) == id)
    }
}

pub(crate) fn backup_id(path: &Path) -> Option<FileId> {
    file_id(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ★ 13절: 예전 데몬이 30초(상한) 뒤에도 살아 있으면 파일을 바꾸지 않는다고 정하고 마커에 `deferred`(`wait_pid_timeout`)를 남긴다
    #[test]
    fn a_previous_daemon_that_does_not_exit_defers_the_update() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("update-complete.json");
        assert_eq!(WAIT_PID_TIMEOUT, Duration::from_secs(30));

        assert!(!wait_for_previous(Some(42), Duration::from_millis(50), |_| true, &marker));
        let m = UpdateCompletionMarker::load_from(&marker).unwrap();
        assert_eq!(m.result.as_deref(), Some("deferred"));
        assert_eq!(m.reason.as_deref(), Some("wait_pid_timeout"));
        assert!(!m.success && m.updated_components.is_empty());

        std::fs::remove_file(&marker).unwrap();
        let polls = Mutex::new(0);
        let exits_later = |_| {
            let mut n = polls.lock().unwrap();
            *n += 1;
            *n < 3
        };
        assert!(wait_for_previous(Some(42), Duration::from_secs(5), exits_later, &marker));
        assert!(!marker.exists(), "nothing is recorded when the daemon exits in time");
    }

    // 파일을 바꾸지 않기로 한 뒤에는 예전 데몬이 끝나야 다시 띄운다. 끝나지 않으면 두 번째 데몬을 띄우지 않는다
    #[test]
    fn an_unchanged_relaunch_waits_for_the_previous_daemon_to_exit() {
        assert!(LATE_EXIT_TIMEOUT >= Duration::from_secs(120), "a daemon shutdown can take well over 30 s");
        assert!(may_relaunch_unchanged(None, Duration::from_millis(50), |_| true), "no PID: relaunch as before");
        assert!(!may_relaunch_unchanged(Some(42), Duration::from_millis(50), |_| true), "still running: leave it");
        let polls = Mutex::new(0);
        let exits_later = |_| {
            let mut n = polls.lock().unwrap();
            *n += 1;
            *n < 3
        };
        assert!(may_relaunch_unchanged(Some(42), Duration::from_secs(5), exits_later));
    }

    // 시간이 다 되면 그사이 답한 새 프로세스(판이 달라도)를 끝낼 것으로 넘긴다. 예전 데몬과 다시 띄우기 전에 뜬 것은 아니다
    #[tokio::test]
    async fn a_timeout_names_the_new_process_that_answered() {
        let rule = HealthRule::StartedAfter { since_ms: 1_000, old_pid: Some(77) };
        let starting = Script {
            answers: Mutex::new(vec![Some(health("0.1.13", 77, Some(10))), Some(health("0.1.13", 600, Some(1_500)))]),
            alive: Box::new(|_| true),
        };
        assert_eq!(watch(&starting, "0.1.14", &rule, limits()).await, WatchOutcome::Unhealthy { pid: Some(600), reason: "timeout" });
        let before = Script { answers: Mutex::new(vec![Some(health("0.1.13", 600, Some(900)))]), alive: Box::new(|_| true) };
        assert_eq!(watch(&before, "0.1.14", &rule, limits()).await, WatchOutcome::Unhealthy { pid: None, reason: "timeout" });
    }

    // 되돌리기 전: 감시가 본 새 프로세스와 잠금을 쥔 새 데몬을 끝내고, 잠금이 풀려야 되돌린다. 예전 데몬은 끝내지 않는다
    #[test]
    fn new_daemons_are_stopped_before_the_rollback() {
        // 서비스 관리자가 띄운 새 데몬(PID 600)이 잠금을 쥐고 있다
        let holder = Mutex::new(LockHolder::Pid(600));
        let stopped = Mutex::new(Vec::new());
        let gone = stop_new_daemons(None, Some(77), || *holder.lock().unwrap(), |pid| {
            stopped.lock().unwrap().push(pid);
            *holder.lock().unwrap() = LockHolder::Free;
        });
        assert!(gone);
        assert_eq!(*stopped.lock().unwrap(), vec![600]);

        // 감시가 본 것도 끝낸다. 예전 데몬 PID는 끝내지 않는다
        let stopped = Mutex::new(Vec::new());
        assert!(stop_new_daemons(Some(600), Some(77), || LockHolder::Free, |pid| stopped.lock().unwrap().push(pid)));
        assert!(stop_new_daemons(Some(77), Some(77), || LockHolder::Pid(77), |pid| stopped.lock().unwrap().push(pid)));
        assert_eq!(*stopped.lock().unwrap(), vec![600]);

        // 끝내도 계속 새 데몬이 잠금을 쥐면(끝나지 않거나 관리자가 다시 띄움) 되돌리지 않는다
        let stopped = Mutex::new(0);
        assert!(!stop_new_daemons(None, Some(77), || LockHolder::Pid(600), |_| *stopped.lock().unwrap() += 1));
        assert_eq!(*stopped.lock().unwrap(), STOP_ROUNDS);
    }

    // 업데이터가 띄운 데몬이 끝나면 좀비로 남아도 끝난 것으로 본다 (생존 창이 놓치지 않게)
    #[cfg(unix)]
    #[test]
    fn a_spawned_child_that_exited_is_not_alive() {
        let pid = keep_child(std::process::Command::new("sleep").arg("0.2").spawn().unwrap());
        assert!(process_running(pid));
        std::thread::sleep(Duration::from_millis(600));
        assert!(ProcessChecker::is_pid_running(pid), "an unreaped child still answers kill -0");
        assert!(!process_running(pid));
        assert!(!HttpProbe::at("http://127.0.0.1:9").alive(pid));
    }

    // 되돌림 보고는 릴레이가 받는 클라이언트 표시를 싣는다
    #[tokio::test]
    async fn the_rollback_report_carries_the_client_header() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = vec![0u8; 8192];
            // 머리와 본문이 따로 올 수 있다: 본문 끝까지 읽는다
            while !String::from_utf8_lossy(&req).contains("rolled_back") {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await;
            String::from_utf8_lossy(&req).to_lowercase()
        });
        report_rolled_back(Some(&base), "0.1.15", "beta").await;
        let request = server.await.unwrap();
        assert!(request.starts_with("post /v1/updates/report"), "{request}");
        assert!(request.contains("x-saba-client: 1"), "{request}");
        assert!(request.contains("\"version\":\"0.1.15\""), "{request}");
    }

    // 잠금을 쥔 데몬은 데몬과 같은 잠금 파일로 찾는다
    #[test]
    fn the_daemon_lock_holder_is_read_from_the_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Free, "no lock file");
        let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(tmp.path().join("daemon.lock")).unwrap();
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Free, "nobody holds it");
        lock.lock().unwrap();
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Unknown);
        let me = std::process::id();
        std::fs::write(tmp.path().join("daemon.lock.owner"), format!("pid={me}\naddr=127.0.0.1:57474\n")).unwrap();
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Pid(me));
        // 자체 리뷰: 잠금을 막 쥔 데몬이 아직 PID를 적지 않았으면 남은 것은 끝난 예전 데몬의 PID다. 그 PID로 보지 않는다
        let mut gone = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" }).args(if cfg!(windows) { &["/C", "exit"][..] } else { &[][..] }).spawn().unwrap();
        let gone_pid = gone.id();
        gone.wait().unwrap();
        std::fs::write(tmp.path().join("daemon.lock.owner"), format!("pid={gone_pid}\n")).unwrap();
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Unknown);
        drop(lock);
        assert_eq!(daemon_lock_holder(tmp.path()), LockHolder::Free);
        // Codex 4차 점검: 잠금 파일을 열지 못하면(없는 것이 아니라) 쥔 쪽을 모른다고 본다. 비어 있다고 보고 되돌리지 않는다
        let odd = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(odd.path().join("daemon.lock")).unwrap();
        assert_eq!(daemon_lock_holder(odd.path()), LockHolder::Unknown);
    }

    fn health(version: &str, pid: u32, started_at_ms: Option<u64>) -> Health {
        Health { version: version.into(), pid, started_at_ms }
    }

    // ★ 13절: 200 응답만으로 성공으로 보지 않는다. 판이 목표와 같고, 업데이터가 띄운 그 프로세스여야 한다
    #[test]
    fn only_the_new_daemon_the_updater_started_counts_as_healthy() {
        let spawned = HealthRule::SpawnedPid(500);
        assert!(spawned.accepts(&health("0.1.14", 500, None), "0.1.14"));
        assert!(!spawned.accepts(&health("0.1.13", 500, None), "0.1.14"), "an old version answering");
        assert!(!spawned.accepts(&health("0.1.14", 77, None), "0.1.14"), "another daemon holding the port");

        // 서비스 관리자·GUI가 띄우면 PID 대신 판과 기동 시각으로 본다
        let since = HealthRule::StartedAfter { since_ms: 1_000, old_pid: Some(77) };
        assert!(since.accepts(&health("0.1.14", 600, Some(1_500)), "0.1.14"));
        assert!(!since.accepts(&health("0.1.14", 600, Some(900)), "0.1.14"), "started before the relaunch");
        assert!(!since.accepts(&health("0.1.14", 77, Some(1_500)), "0.1.14"), "the previous daemon");
        assert!(!since.accepts(&health("0.1.14", 600, None), "0.1.14"), "no start time to compare");
        assert!(!since.accepts(&health("0.1.13", 600, Some(1_500)), "0.1.14"));
    }

    /// 정해진 답을 차례로 주는 가짜 데몬. 끝나면 마지막 답을 계속 준다
    struct Script {
        answers: Mutex<Vec<Option<Health>>>,
        alive: Box<dyn Fn(u32) -> bool + Send + Sync>,
    }

    impl Probe for Script {
        fn health(&self) -> impl Future<Output = Option<Health>> + Send {
            let mut a = self.answers.lock().unwrap();
            let next = if a.len() > 1 { a.remove(0) } else { a.first().cloned().flatten() };
            async move { next }
        }
        fn alive(&self, pid: u32) -> bool {
            (self.alive)(pid)
        }
    }

    fn limits() -> WatchLimits {
        WatchLimits { timeout: Duration::from_millis(300), survival: Duration::from_millis(100), poll: Duration::from_millis(10) }
    }

    // 새 데몬이 목표 판으로 답하고 생존 창 동안 살아 있으면 건강하다. 답이 늦어도 상한 안이면 된다
    #[tokio::test]
    async fn a_new_daemon_that_answers_and_stays_up_is_healthy() {
        let probe = Script {
            answers: Mutex::new(vec![None, Some(health("0.1.13", 77, None)), Some(health("0.1.14", 500, None))]),
            alive: Box::new(|_| true),
        };
        let out = watch(&probe, "0.1.14", &HealthRule::SpawnedPid(500), limits()).await;
        assert_eq!(out, WatchOutcome::Healthy { pid: 500 });
        assert_eq!(HEALTH_TIMEOUT, Duration::from_secs(90));
        assert_eq!(SURVIVAL_WINDOW, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn a_new_daemon_that_dies_or_never_answers_is_unhealthy() {
        let dies = Script { answers: Mutex::new(vec![Some(health("0.1.14", 500, None))]), alive: Box::new(|_| false) };
        assert_eq!(
            watch(&dies, "0.1.14", &HealthRule::SpawnedPid(500), limits()).await,
            WatchOutcome::Unhealthy { pid: Some(500), reason: "exited" }
        );
        // 예전 판이 계속 답하는 것은 새 데몬이 아니다
        let old = Script { answers: Mutex::new(vec![Some(health("0.1.13", 77, None))]), alive: Box::new(|_| true) };
        assert_eq!(
            watch(&old, "0.1.14", &HealthRule::SpawnedPid(500), limits()).await,
            WatchOutcome::Unhealthy { pid: None, reason: "timeout" }
        );
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn files_under(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    out.push(e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }

    // ★ 13절: 스냅샷은 포함 목록(설정·설치 기록·인스턴스 기록)만 담는다. 러너 기록·managed.pid·게임 월드·백업은 담지 않아,
    // 되돌려도 켜진 서버의 실행 기록은 새 것 그대로 남는다
    #[test]
    fn the_data_snapshot_holds_only_the_included_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), r#"{"gui":{"language":"ko"}}"#);
        write(&data.join("installed-manifest.json"), r#"{"saba-core":"0.1.13"}"#);
        write(&data.join("daemon.lock"), "1");
        write(&data.join("update-complete.json"), "{}");
        write(&inst.join("order.json"), "[]");
        write(&inst.join("a/instance.json"), "old instance");
        write(&inst.join("a/settings.json"), "old settings");
        write(&inst.join("a/server/world/level.dat"), "world");
        write(&inst.join("a/backups/1.zip"), "backup");
        write(&inst.join("a/run/runner.json"), "runner");
        write(&inst.join(".saba-state/a/managed.pid"), "123");
        write(&inst.join(".saba-state/a/run/runner.json"), "runner");

        let snap = DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        assert!(snap.path().starts_with(data.join(SNAPSHOT_DIR)));
        assert!(snap.path().file_name().unwrap().to_string_lossy().starts_with("0.1.13-"), "a new name per copy");
        assert_eq!(
            files_under(snap.path()),
            vec![
                "data/config.json",
                "data/installed-manifest.json",
                "instances/a/instance.json",
                "instances/a/settings.json",
                "instances/order.json",
            ]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(snap.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "only the owner can read the copy of the settings");
        }

        // 새 데몬이 바꾼 것
        write(&data.join("config.json"), r#"{"gui":{"language":"en"},"new":1}"#);
        write(&inst.join("a/instance.json"), "new format");
        write(&inst.join(".saba-state/a/managed.pid"), "456");
        write(&inst.join(".saba-state/a/run/runner.json"), "runner 2");

        snap.restore().unwrap();
        assert_eq!(std::fs::read_to_string(data.join("config.json")).unwrap(), r#"{"gui":{"language":"ko"}}"#);
        assert_eq!(std::fs::read_to_string(inst.join("a/instance.json")).unwrap(), "old instance");
        assert_eq!(std::fs::read_to_string(inst.join(".saba-state/a/managed.pid")).unwrap(), "456", "run state is not rolled back");
        assert_eq!(std::fs::read_to_string(inst.join(".saba-state/a/run/runner.json")).unwrap(), "runner 2");

        snap.discard();
        assert!(!data.join(SNAPSHOT_DIR).exists(), "removed once done");
    }

    // ★ 13절: 되돌리면 실패한 판을 업데이트 설정에 남긴다. 완료 마커를 읽고 지워도 기록은 남는다.
    // 스냅샷의 설정을 되돌린 뒤에 적으므로 되돌림이 기록을 덮지 않는다
    #[test]
    fn a_rollback_records_the_failed_release_outside_the_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        let config = ConfigFile::at(&data);
        config.set_section("updates", serde_json::json!({ "channel": "beta", "auto_apply": true })).unwrap();
        let marker = data.join("update-complete.json");

        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.install_root).unwrap();
        let (core, core_old) = if cfg!(windows) { ("saba-core.exe", "saba-core.exe.old") } else { ("saba-core", "saba-core.old") };
        std::fs::write(manager.install_root.join(core), b"0.1.13").unwrap();

        let live = LiveApply {
            staging_dir: manager.staging_dir.clone(),
            before: manager.exe_backups(),
            snapshot: DataSnapshot::take(&data, &inst, "0.1.13").unwrap(),
            from_version: "0.1.13".into(),
            target_version: "0.1.14".into(),
            release_version: Some("0.1.15".into()),
        };
        assert_eq!(live.report_version(), "0.1.15", "the relay counts rollbacks per release");
        begin_watch(&manager.staging_dir);
        // 적용과 새 데몬
        std::fs::rename(manager.install_root.join(core), manager.install_root.join(core_old)).unwrap();
        std::fs::write(manager.install_root.join(core), b"0.1.14").unwrap();
        UpdateCompletionMarker {
            release: Some(crate::marker::ReleaseSummary { version: "0.1.14".into(), ..Default::default() }),
            ..UpdateCompletionMarker::success(vec!["saba-core".into()])
        }
        .save_to(&marker)
        .unwrap();
        config.set_section("updates", serde_json::json!({ "channel": "stable" })).unwrap();

        live.roll_back(&manager, &config, &marker, "timeout");

        assert_eq!(std::fs::read(manager.install_root.join(core)).unwrap(), b"0.1.13");
        let m = UpdateCompletionMarker::load_from(&marker).unwrap();
        assert_eq!(m.result.as_deref(), Some("rolled_back"));
        assert_eq!(m.version.as_deref(), Some("0.1.14"));
        assert!(!m.success);
        assert_eq!(m.release.unwrap().version, "0.1.14", "patch notes stay");
        assert!(!data.join(SNAPSHOT_DIR).exists());
        assert!(!watch_in_progress(&manager.staging_dir));

        std::fs::remove_file(&marker).unwrap();
        let saved = saved_update_config(&config);
        assert_eq!(saved.failed_release.as_deref(), Some("0.1.14"));
        assert_eq!(saved.channel, "beta", "the snapshot restored the settings first");
        assert!(saved.auto_apply);
    }

    // Codex 리뷰: 데이터 스냅샷을 뜨지 못하면 적용하지 않는다. 새 데몬이 데이터를 바꾼 뒤 죽으면 실행 파일만 되돌아가기 때문이다
    #[test]
    fn a_failed_data_snapshot_stops_the_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        // 스냅샷 폴더 자리에 파일이 있어 폴더를 만들 수 없다
        write(&data.join(SNAPSHOT_DIR), "not a folder");
        let staging = tmp.path().join("staging");
        let err = LiveApply::start(&staging, ExeBackups::default(), &data, &data.join("instances"), "0.1.13".into(), "0.1.14".into(), None)
            .unwrap_err();
        assert!(err.contains("snapshot"), "{err}");
        assert!(!watch_in_progress(&staging), "nothing to watch when nothing is applied");
    }

    // Codex 리뷰: 되돌리다 데이터 복원이 실패하면 사본을 지우지 않고 남긴다 (손으로 되살릴 수단)
    #[test]
    fn a_failed_restore_keeps_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), r#"{"gui":{"language":"ko"}}"#);
        let config = ConfigFile::at(&data);
        let marker = data.join("update-complete.json");
        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.install_root).unwrap();
        let live =
            LiveApply::start(&manager.staging_dir, manager.exe_backups(), &data, &inst, "0.1.13".into(), "0.1.14".into(), None).unwrap();
        // 새 데몬이 설정 파일 자리를 비울 수 없는 폴더로 바꿨다: 복원이 실패한다
        std::fs::remove_file(data.join("config.json")).unwrap();
        write(&data.join("config.json").join("x"), "x");

        let kept_at = live.roll_back(&manager, &config, &marker, "timeout");

        // Codex 재리뷰: 복원 실패는 따로 알린다 (설치기는 이전 판을 다시 띄우지 않는다)
        let kept_at = kept_at.expect("the caller learns that the data was not restored");
        assert!(kept_at.copy.dir.join("data").join("config.json").is_file(), "{}", kept_at.copy.dir.display());
        // Codex 3차 리뷰: 손으로 되살릴 곳은 `data/`→데이터 폴더, `instances/`→인스턴스 폴더 (따로 둔 인스턴스 폴더 포함)
        assert_eq!((kept_at.copy.data_dir.as_path(), kept_at.copy.instances_dir.as_path()), (data.as_path(), inst.as_path()));
        let m = UpdateCompletionMarker::load_from(&marker).unwrap();
        assert_eq!((m.result.as_deref(), m.reason.as_deref()), (Some("rolled_back"), Some("restore_failed")));
        let message = m.message.unwrap_or_default();
        for place in [&kept_at.copy.dir, &data, &inst] {
            assert!(message.contains(&place.display().to_string()), "the marker names {}: {message}", place.display());
        }
        let kept = files_under(&data.join(SNAPSHOT_DIR));
        assert!(kept.iter().any(|f| f.ends_with("data/config.json")), "the copy is kept: {kept:?}");
        // 같은 판의 다음 스냅샷이 남긴 사본을 덮지 않는다
        DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        assert!(files_under(&data.join(SNAPSHOT_DIR)).iter().any(|f| f.ends_with("data/config.json")));
    }

    // Codex 재리뷰: 새 데몬을 끝내지 못해 되돌리지 않고 두면, 새 데몬이 이미 데이터를 바꿨을 수 있으니 사본을 남긴다
    #[test]
    fn leaving_the_new_daemon_in_place_keeps_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), "{}");
        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.install_root).unwrap();
        std::fs::create_dir_all(&manager.staging_dir).unwrap();
        let (core, core_old) = if cfg!(windows) { ("saba-core.exe", "saba-core.exe.old") } else { ("saba-core", "saba-core.old") };
        std::fs::write(manager.install_root.join(core), b"0.1.13").unwrap();
        let marker = data.join("update-complete.json");
        UpdateCompletionMarker::success(vec!["saba-core".into()]).save_to(&marker).unwrap();
        let live =
            LiveApply::start(&manager.staging_dir, manager.exe_backups(), &data, &inst, "0.1.13".into(), "0.1.14".into(), None).unwrap();
        std::fs::rename(manager.install_root.join(core), manager.install_root.join(core_old)).unwrap();
        std::fs::write(manager.install_root.join(core), b"0.1.14").unwrap();

        let recovery = live.leave_in_place(&manager, &marker, "timeout");

        assert!(recovery.copy.dir.join("data").join("config.json").is_file());
        // Codex 4차 리뷰: 안내는 새 데몬 끝내기 → 실행 파일 되돌리기 → 데이터 차례다
        assert!(recovery.stop_first);
        assert_eq!(recovery.exes, vec![(recovery.copy.dir.join("exes").join("0").join(core), manager.install_root.join(core))]);
        assert!(!recovery.missing_core);
        // Codex 3차 리뷰: 적용 성공으로 남지 않는다. 실패와 사본 자리를 적는다
        let m = UpdateCompletionMarker::load_from(&marker).unwrap();
        assert!(!m.success);
        assert_eq!((m.result.as_deref(), m.reason.as_deref()), (Some("left_in_place"), Some("timeout")));
        let message = m.message.unwrap_or_default();
        assert!(message.contains(&recovery.copy.dir.display().to_string()));
        assert!(message.contains("end every saba-core process"), "{message}");
    }

    // Codex 3차 리뷰: 읽지 못한 것(권한·입출력 오류)을 없는 것으로 넘기지 않는다. 없는 것만 건너뛴다
    #[test]
    fn a_snapshot_that_cannot_read_its_sources_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        // 인스턴스 폴더 자리가 폴더가 아니다: 읽지 못한다 (없는 것과 다르다)
        let inst = data.join("instances");
        write(&inst, "not a folder");
        assert!(DataSnapshot::take(&data, &inst, "0.1.13").is_err());
        // 인스턴스 폴더가 아예 없으면 건너뛴다
        assert!(DataSnapshot::take(&data, &tmp.path().join("none"), "0.1.13").is_ok());
    }

    #[test]
    fn a_restore_that_cannot_read_the_copy_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), "{}");
        let snap = DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        // 사본의 `data` 자리가 폴더가 아니게 됐다: 읽지 못한다
        std::fs::remove_dir_all(snap.path().join("data")).unwrap();
        write(&snap.path().join("data"), "broken");
        assert!(snap.restore().is_err());
    }

    // Codex 4차 리뷰: 남긴 사본은 같은 판의 다음 스냅샷이 지우지 않는다. 사본마다 새 이름이고, 끊긴 실행이 남긴 온전한 사본도 남긴다
    #[test]
    fn snapshots_never_delete_an_earlier_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), "{}");
        let first = DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        let first_dir = first.path().to_path_buf();
        let kept = first.keep();
        assert_eq!(kept.dir, first_dir, "keeping does not move the copy (a failed rename cannot lose it)");
        let second = DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        assert_ne!(second.path(), first_dir.as_path());
        assert!(first_dir.join("data").join("config.json").is_file(), "the kept copy is still there");
        second.discard();
        assert!(first_dir.join("data").join("config.json").is_file());
    }

    // Codex 4차 점검: 감시 표시를 남기지 못하면 새 데몬이 `.old`를 지워 되돌릴 수 없으니 적용하지 않는다
    #[test]
    fn an_apply_that_cannot_mark_the_watch_does_not_start() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        let staging = tmp.path().join("staging");
        write(&staging, "not a folder");
        assert!(LiveApply::start(&staging, ExeBackups::default(), &data, &data.join("instances"), "0.1.13".into(), "0.1.14".into(), None).is_err());
    }

    // Codex 4차 점검: 코어 실행 파일을 되돌리지 못하면 데이터만 되돌린 채 이전 판을 띄우지 않는다.
    // 남은 실행 파일(백업 → 제자리)을 안내에 싣고, `.old`는 지우지 않게 붙잡아 둔다
    #[test]
    fn a_rollback_without_the_core_backup_needs_a_manual_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("config.json"), "{}");
        let config = ConfigFile::at(&data);
        let marker = data.join("update-complete.json");
        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.install_root).unwrap();
        std::fs::create_dir_all(&manager.staging_dir).unwrap();
        let live =
            LiveApply::start(&manager.staging_dir, manager.exe_backups(), &data, &inst, "0.1.13".into(), "0.1.14".into(), None).unwrap();
        // 적용이 코어를 비켜 두지 않았다 (백업이 없다)
        let recovery = live.roll_back(&manager, &config, &marker, "timeout").expect("the old core is not back");
        assert!(recovery.copy.dir.join("data").join("config.json").is_file());
        let m = UpdateCompletionMarker::load_from(&marker).unwrap();
        assert_eq!(m.reason.as_deref(), Some("restore_failed"));
        assert!(m.message.unwrap_or_default().contains("saba-core"), "names the exe that was not restored");
        assert!(recovery.missing_core);
    }

    // 자체 리뷰: 되돌림은 실행 파일만이다. 감시 중에 새 데몬이 바꾼 모듈·익스텐션·번역은 그대로 남으므로
    // 설치 기록도 그 항목은 지금 것을 둔다 (옛 해시로 되돌리면 그 모듈이 "수정됨"으로 보여 다시는 자동 교체되지 않는다)
    #[test]
    fn a_restore_keeps_the_install_records_of_what_it_does_not_roll_back() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inst = data.join("instances");
        write(&data.join("installed-manifest.json"), r#"{"saba-core":"0.1.13","module-mc":"1.0.0"}"#);
        write(&data.join("installed-files.json"), r#"{"saba-core":{"/i/saba-core":"old"},"module-mc":{"/m/mc/a.py":"m1"}}"#);
        write(&data.join("official-components.json"), r#"["saba-core","module-mc"]"#);
        let snapshot = DataSnapshot::take(&data, &inst, "0.1.13").unwrap();
        // 새 데몬: 코어 기록은 적용이 바꿨고, 감시 중에 모듈을 바꾸고 익스텐션을 새로 깔았다
        write(&data.join("installed-manifest.json"), r#"{"saba-core":"0.1.14","module-mc":"1.1.0","ext-x":"2.0.0"}"#);
        write(&data.join("installed-files.json"), r#"{"saba-core":{"/i/saba-core":"new"},"module-mc":{"/m/mc/a.py":"m2"},"ext-x":{"/e/x/b.py":"x"}}"#);
        write(&data.join("official-components.json"), r#"["saba-core","module-mc","ext-x"]"#);

        snapshot.restore().unwrap();

        let read = |name: &str| serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(data.join(name)).unwrap()).unwrap();
        assert_eq!(read("installed-manifest.json"), serde_json::json!({"saba-core":"0.1.13","module-mc":"1.1.0","ext-x":"2.0.0"}));
        assert_eq!(
            read("installed-files.json"),
            serde_json::json!({"saba-core":{"/i/saba-core":"old"},"module-mc":{"/m/mc/a.py":"m2"},"ext-x":{"/e/x/b.py":"x"}})
        );
        let mut official: Vec<String> = serde_json::from_value(read("official-components.json")).unwrap();
        official.sort();
        assert_eq!(official, ["ext-x", "module-mc", "saba-core"]);
    }

    // Codex 5차 리뷰: 스냅샷을 뜨다 실패하면 반쯤 뜬 사본을 남기지 않는다 (다음 시도는 새 이름이라 쌓인다)
    #[test]
    fn a_failed_snapshot_leaves_no_partial_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        let inst = data.join("instances");
        write(&inst, "not a folder");
        for _ in 0..2 {
            assert!(DataSnapshot::take(&data, &inst, "0.1.13").is_err());
        }
        let left: Vec<_> = std::fs::read_dir(data.join(SNAPSHOT_DIR)).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    // 끊긴 실행이 남긴 오래된 `.partial`은 다음 스냅샷이 지운다. 온전한 사본과 막 뜨는 중인 것은 두고
    #[cfg(unix)]
    #[test]
    fn a_snapshot_sweeps_stale_partial_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        let root = data.join(SNAPSHOT_DIR);
        let (stale, fresh, whole) = (root.join("0.1.12-1.partial"), root.join("0.1.12-2.partial"), root.join("0.1.12-3"));
        for dir in [&stale, &fresh, &whole] {
            write(&dir.join("data").join("config.json"), "{}");
        }
        let old = SystemTime::now() - STALE_PARTIAL_AGE - Duration::from_secs(60);
        std::fs::File::open(&stale).unwrap().set_modified(old).unwrap();
        DataSnapshot::take(&data, &data.join("instances"), "0.1.13").unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists() && whole.exists());
    }

    // Codex 5차 리뷰: 안내하는 실행 파일 백업은 사본 폴더에 따로 뜬다. 다음 적용이 `.old`를 다시 비켜 두며 덮어도 남는다
    #[test]
    fn the_recovery_exes_survive_the_next_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.staging_dir).unwrap();
        let (core, core_old) = if cfg!(windows) { ("saba-core.exe", "saba-core.exe.old") } else { ("saba-core", "saba-core.old") };
        write(&manager.install_root.join(core), "0.1.13");
        let marker = data.join("update-complete.json");
        let live = LiveApply::start(&manager.staging_dir, manager.exe_backups(), &data, &data.join("instances"), "0.1.13".into(), "0.1.14".into(), None)
            .unwrap();
        std::fs::rename(manager.install_root.join(core), manager.install_root.join(core_old)).unwrap();
        write(&manager.install_root.join(core), "0.1.14");

        let recovery = live.leave_in_place(&manager, &marker, "timeout");

        let (from, to) = recovery.exes[0].clone();
        assert_eq!(to, manager.install_root.join(core));
        assert!(from.starts_with(&recovery.copy.dir), "{}", from.display());
        assert_eq!(from.file_name(), to.file_name(), "the copy has the name it goes back under");
        // 다음 적용이 `.old`를 새 백업으로 바꾸고 정리가 지운다
        std::fs::write(manager.install_root.join(core_old), b"0.1.14").unwrap();
        begin_watch(&manager.staging_dir).unwrap();
        end_watch(&manager.staging_dir);
        manager.cleanup_old_files();
        assert_eq!(std::fs::read(&from).unwrap(), b"0.1.13");
        let message = UpdateCompletionMarker::load_from(&marker).unwrap().message.unwrap_or_default();
        assert!(message.contains(&from.display().to_string()), "{message}");
    }

    // 백업 목록을 읽지 못하면 어느 `.old`가 이번 것인지 모르니 붙잡아 둔다
    #[test]
    fn backups_are_held_when_they_cannot_be_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        write(&data.join("config.json"), "{}");
        let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
        manager.install_root = tmp.path().join("install");
        manager.staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&manager.install_root).unwrap();
        std::fs::create_dir_all(&manager.staging_dir).unwrap();
        let marker = data.join("update-complete.json");
        let live = LiveApply::start(&manager.staging_dir, manager.exe_backups(), &data, &data.join("instances"), "0.1.13".into(), "0.1.14".into(), None)
            .unwrap();
        // 설치 폴더 자리가 폴더가 아니다: 읽지 못한다
        std::fs::remove_dir(&manager.install_root).unwrap();
        write(&manager.install_root, "not a folder");
        live.roll_back(&manager, &ConfigFile::at(&data), &marker, "timeout").expect("not rolled back");
        assert!(watch_in_progress(&manager.staging_dir));
    }

    // 사본 폴더에 뜨지 못하면 `.old`를 그대로 가리키고 붙잡아 둔다
    #[test]
    fn exes_that_cannot_be_copied_keep_pointing_at_the_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let copy = KeptCopy { dir: tmp.path().join("kept"), data_dir: tmp.path().into(), instances_dir: tmp.path().into() };
        write(&copy.dir.join("exes"), "not a folder");
        let backup = tmp.path().join("saba-core.old");
        write(&backup, "0.1.13");
        let (exes, all) = copy.preserve_exes(vec![(backup.clone(), tmp.path().join("saba-core"))]);
        assert!(!all);
        assert_eq!(exes[0].0, backup);
    }

    // Codex 6차 리뷰: 디스크가 차 복사하지 못해도 같은 볼륨이면 옮긴다 (공간이 들지 않는다). 그러면 붙잡을 필요가 없다
    #[test]
    fn recovery_exes_are_moved_into_the_kept_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let copy = KeptCopy { dir: tmp.path().join("kept"), data_dir: tmp.path().into(), instances_dir: tmp.path().into() };
        let backup = tmp.path().join("saba-core.old");
        write(&backup, "0.1.13");
        let (exes, all) = copy.preserve_exes(vec![(backup.clone(), tmp.path().join("saba-core"))]);
        assert!(all);
        assert!(!backup.exists(), "moved, not copied");
        assert_eq!(std::fs::read(&exes[0].0).unwrap(), b"0.1.13");
    }
}
