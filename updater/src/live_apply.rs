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
        if !same_version(&health.version, target_version) {
            return false;
        }
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
    /// `pid`: 목표 판으로 답한 새 데몬 (끝낼 것). `reason`: `timeout`·`exited`·`replaced`
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
    loop {
        if let Some(health) = probe.health().await.filter(|h| rule.accepts(h, target_version)) {
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
            return WatchOutcome::Unhealthy { pid: None, reason: "timeout" };
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
        ProcessChecker::is_pid_running(pid)
    }
}

/// 프로세스 하나를 끝낸다 (자식은 건드리지 않는다: 러너·게임 서버는 데몬과 따로 산다).
/// Unix는 TERM을 보내고 5초 안에 끝나지 않으면 KILL
pub fn stop_process(pid: u32) {
    if !ProcessChecker::is_pid_running(pid) {
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
        if !wait_for_exit(pid, Duration::from_secs(5), Duration::from_millis(200), ProcessChecker::is_pid_running) {
            let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).output();
        }
    }
    wait_for_exit(pid, Duration::from_secs(5), Duration::from_millis(200), ProcessChecker::is_pid_running);
}

// ═══════════════════════════════════════════════════════
// 감시 중 표시
// ═══════════════════════════════════════════════════════

/// 감시를 시작한다고 적는다 (이 프로세스의 PID)
pub fn begin_watch(staging_dir: &Path) {
    let body = serde_json::json!({ "pid": std::process::id() }).to_string();
    if let Err(e) = write_atomic(&staging_dir.join(WATCH_FILE), body.as_bytes()) {
        tracing::warn!("[Apply] Cannot mark the health watch: {}", e);
    }
}

pub fn end_watch(staging_dir: &Path) {
    let _ = std::fs::remove_file(staging_dir.join(WATCH_FILE));
}

/// 업데이터가 아직 새 데몬을 지켜보는 중인가 (표시를 남긴 프로세스가 살아 있다). 그동안 `.old`는 되돌림에 쓴다
pub fn watch_in_progress(staging_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(staging_dir.join(WATCH_FILE)) else { return false };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
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
        let dir = root.join(&name);
        // 다 뜬 뒤 이름을 바꾼다: `dir`이 있으면 언제나 온전한 사본이다
        let partial = root.join(format!("{name}.partial"));
        for stale in [&partial, &dir] {
            if stale.exists() {
                std::fs::remove_dir_all(stale)?;
            }
        }
        create_private_dir(&root)?;
        create_private_dir(&partial)?;
        for file in SNAPSHOT_DATA_FILES {
            copy_if_file(&data_dir.join(file), &partial.join("data").join(file))?;
        }
        for file in SNAPSHOT_INSTANCE_ROOT_FILES {
            copy_if_file(&instances_dir.join(file), &partial.join("instances").join(file))?;
        }
        if let Ok(entries) = std::fs::read_dir(instances_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                // `.saba-state`(실행 기록) 같은 점 폴더는 인스턴스가 아니다
                if name.to_string_lossy().starts_with('.') || !entry.path().is_dir() {
                    continue;
                }
                for file in SNAPSHOT_INSTANCE_FILES {
                    copy_if_file(&entry.path().join(file), &partial.join("instances").join(&name).join(file))?;
                }
            }
        }
        std::fs::rename(&partial, &dir)?;
        tracing::info!("[Apply] Data snapshot taken at {}", dir.display());
        Ok(Self { dir, data_dir: data_dir.to_path_buf(), instances_dir: instances_dir.to_path_buf() })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// 사본을 제자리에 되돌린다 (사본에 있는 파일만)
    pub fn restore(&self) -> std::io::Result<()> {
        restore_tree(&self.dir.join("data"), &self.data_dir)?;
        restore_tree(&self.dir.join("instances"), &self.instances_dir)?;
        tracing::info!("[Apply] Data restored from {}", self.dir.display());
        Ok(())
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

fn copy_if_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_file() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

fn restore_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            restore_tree(&src, &dst)?;
        } else {
            write_atomic(&dst, &std::fs::read(&src)?)?;
        }
    }
    Ok(())
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
    match http.post(format!("{base}/v1/updates/report")).json(&body).send().await {
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
    snapshot: Option<DataSnapshot>,
    pub from_version: String,
    pub target_version: String,
}

impl LiveApply {
    /// 받아 둔 것 중 이번에 적용할 코어가 있으면 준비한다: 데이터 스냅샷, 지금 있는 `.old` 목록, 감시 표시
    pub fn prepare(manager: &mut UpdateManager, data_dir: &Path, instances_dir: &Path) -> Option<Self> {
        let (from_version, target_version) = manager.staged_core_change()?;
        let snapshot = match DataSnapshot::take(data_dir, instances_dir, &from_version) {
            Ok(s) => Some(s),
            Err(e) => {
                // 스냅샷 없이도 실행 파일은 되돌릴 수 있다
                tracing::error!("[Apply] Cannot take the data snapshot: {}", e);
                None
            }
        };
        begin_watch(&manager.staging_dir);
        Some(Self { staging_dir: manager.staging_dir.clone(), before: manager.exe_backups(), snapshot, from_version, target_version })
    }

    /// 새 데몬이 건강하다: 스냅샷과 이번 `.old`를 지운다
    pub fn healthy(self, manager: &UpdateManager) {
        if let Some(s) = self.snapshot {
            s.discard();
        }
        end_watch(&self.staging_dir);
        manager.cleanup_old_files();
    }

    /// 지켜볼 것이 없다: 코어를 적용하지 못했거나(그 전에 멈췄거나 되돌려짐) 다시 띄운 것이 없다.
    /// `.old`는 예전처럼 다음 정리(`cleanup_old_files`)가 지운다
    pub fn skip(self) {
        if let Some(s) = self.snapshot {
            s.discard();
        }
        end_watch(&self.staging_dir);
    }

    /// 새 데몬은 이미 끝냈다고 보고 되돌린다: 이번 묶음의 실행 파일 전부를 `.old`에서, 데이터를 스냅샷에서.
    /// 그다음 `failed_release`를 적고 완료 마커를 `rolled_back`으로 바꾼다 (이 순서: 스냅샷의 설정이 기록을 덮지 않게)
    pub fn roll_back(self, manager: &UpdateManager, config: &ConfigFile, marker_path: &Path, reason: &str) {
        let restored = manager.roll_back_exes_since(&self.before);
        tracing::warn!("[Apply] Rolled back {} ({}): {:?}", self.target_version, reason, restored);
        if let Some(snapshot) = &self.snapshot {
            if let Err(e) = snapshot.restore() {
                tracing::error!("[Apply] Cannot restore the data snapshot {}: {}", snapshot.path().display(), e);
            }
        }
        if let Err(e) = record_failed_release(config, &self.target_version) {
            tracing::error!("[Apply] Cannot record the failed release {}: {}", self.target_version, e);
        }
        let marker = UpdateCompletionMarker::rolled_back(&self.target_version, reason, UpdateCompletionMarker::load_from(marker_path));
        if let Err(e) = marker.save_to(marker_path) {
            tracing::error!("[Apply] Cannot record the rollback: {}", e);
        }
        if let Some(s) = self.snapshot {
            s.discard();
        }
        end_watch(&self.staging_dir);
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
        assert_eq!(snap.path(), data.join(SNAPSHOT_DIR).join("0.1.13"));
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
            snapshot: Some(DataSnapshot::take(&data, &inst, "0.1.13").unwrap()),
            from_version: "0.1.13".into(),
            target_version: "0.1.14".into(),
        };
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
}
