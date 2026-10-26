//! 상주 GUI 감독 — 트레이는 데몬이 띄운 작은 GUI(`saba-chan-gui --tray`)가 든다
//!
//! 화면이 있는 환경(데스크톱 세션)에서만 띄운다. 서버·SSH·서비스 세션·원격 데몬에서는 띄우지 않는다.
//! 데몬이 살아 있는 동안 트레이를 든 GUI가 사라지면 다시 띄운다. 짧게 거듭 죽으면 간격을 늘린다.
//! 부트 모드(GUI·CLI·데몬 온리)와 상관없이 같다. 종료 요청·업데이터 기동 뒤에는 띄우지 않는다.

use std::time::{Duration, Instant};

/// 상주 GUI를 띄우는 인자
pub const TRAY_ARG: &str = "--tray";
/// 확인 간격
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// 데몬이 뜬 뒤 첫 확인까지 (데몬을 띄운 GUI가 먼저 등록할 틈)
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(3);
/// 이보다 짧게 살고 사라지면 실패로 센다
const STABLE_AFTER: Duration = Duration::from_secs(30);
const BACKOFF_BASE: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// 화면을 띄울 수 있는 데스크톱 세션인가.
/// Windows: 서비스 세션(0)이 아니고 대화형 창 스테이션(`WinSta0`)에 있다 (SSH·서비스는 다른 스테이션).
/// macOS: 늘 있다고 본다. Linux 등: `DISPLAY`나 `WAYLAND_DISPLAY`가 있다
pub fn has_desktop() -> bool {
    #[cfg(windows)]
    {
        windows_interactive()
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
    }
}

#[cfg(windows)]
fn windows_interactive() -> bool {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::StationsAndDesktops::{GetProcessWindowStation, GetUserObjectInformationW, UOI_NAME};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let mut session: u32 = 0;
    // SAFETY: 출력 포인터는 이 스택 변수다
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 || session == 0 {
        return false;
    }
    // SAFETY: 반환 핸들은 닫지 않는다(프로세스 창 스테이션). 버퍼 크기를 바이트로 넘긴다
    let station = unsafe { GetProcessWindowStation() };
    if station.is_null() {
        return false;
    }
    let mut name = [0u16; 64];
    let mut needed: u32 = 0;
    let ok = unsafe {
        GetUserObjectInformationW(
            station as _,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            (name.len() * 2) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return false;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case("WinSta0")
}

/// 테스트 하네스가 데몬만 띄울 때 끈다 (`SABA_NO_TRAY`가 비어 있지 않으면)
fn disabled_by_env() -> bool {
    std::env::var_os("SABA_NO_TRAY").is_some_and(|v| !v.is_empty())
}

/// 연속 실패 n번째 뒤 다음 시도까지 기다릴 시간 (5s, 10s, 20s … 최대 5분)
fn backoff(fails: u32) -> Duration {
    BACKOFF_BASE
        .saturating_mul(1u32 << fails.saturating_sub(1).min(16))
        .min(BACKOFF_MAX)
}

/// 되살리기 판단 상태
#[derive(Default)]
struct Respawn {
    /// 마지막으로 띄운 상주 GUI와 그 시각
    spawned: Option<(u32, Instant)>,
    fails: u32,
    not_before: Option<Instant>,
}

impl Respawn {
    /// 트레이를 든 GUI가 있다: 띄운 것이 충분히 살았으면 실패 기록을 지운다
    fn host_seen(&mut self, now: Instant) {
        if self
            .spawned
            .is_some_and(|(_, at)| now.duration_since(at) >= STABLE_AFTER)
        {
            self.spawned = None;
            self.fails = 0;
            self.not_before = None;
        }
    }

    /// 트레이를 든 GUI가 없다: 지금 띄워도 되는가. 직전에 띄운 것이 금방 사라졌으면 실패로 세고 기다린다
    fn may_spawn(&mut self, now: Instant) -> bool {
        if let Some((_, at)) = self.spawned.take() {
            if now.duration_since(at) < STABLE_AFTER {
                self.fails += 1;
                self.not_before = Some(now + backoff(self.fails));
            } else {
                self.fails = 0;
                self.not_before = None;
            }
        }
        self.not_before.is_none_or(|t| now >= t)
    }

    fn did_spawn(&mut self, pid: u32, now: Instant) {
        self.spawned = Some((pid, now));
    }

    /// 띄우지 못했다: 금방 사라진 것과 같이 센다
    fn spawn_failed(&mut self, now: Instant) {
        self.spawned = None;
        self.fails += 1;
        self.not_before = Some(now + backoff(self.fails));
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 트레이를 든 GUI가 살아 있는가: 등록된 GUI 중 프로세스가 살아 있는 것, 또는 이 감독이 띄운 그 프로세스
/// (`started`: PID와 띄운 시각. 등록 전이거나 등록을 잃었어도 센다. 같은 PID를 다른 프로세스가 받았으면 세지 않는다)
async fn host_alive(registry: &crate::ipc::ClientRegistry, started: Option<(u32, u64)>) -> bool {
    use crate::supervisor::process::{is_process_alive, is_same_process};
    let guis: Vec<u32> = registry
        .list()
        .await
        .into_iter()
        .filter(|(_, c)| c.kind == crate::ipc::ClientKind::Gui)
        .filter_map(|(_, c)| c.pid)
        .collect();
    guis.into_iter().any(is_process_alive) || started.is_some_and(|(pid, at)| is_same_process(pid, at))
}

/// 감독 루프. 화면이 없거나 GUI 실행 파일이 없으면 바로 끝난다
pub async fn run(server: crate::ipc::IPCServer) {
    if disabled_by_env() || !has_desktop() {
        tracing::info!("[Tray] No desktop session; the tray is not shown");
        return;
    }
    let Some(gui) = crate::boot_selector::find_interface_binary("saba-chan-gui") else {
        tracing::info!("[Tray] saba-chan-gui was not found next to the daemon; the tray is not shown");
        return;
    };
    let stop = server.stop_requested.clone();
    let mut state = Respawn::default();
    // 마지막으로 띄운 상주 GUI (PID, 띄운 시각)
    let mut started: Option<(u32, u64)> = None;
    tokio::select! {
        _ = stop.cancelled() => return,
        _ = tokio::time::sleep(FIRST_CHECK_DELAY) => {}
    }
    loop {
        if server.update_state.updater_launched() {
            return;
        }
        let now = Instant::now();
        if host_alive(&server.client_registry, started).await {
            state.host_seen(now);
        } else if state.may_spawn(now) {
            let env: Vec<(&str, String)> = crate::ipc::handlers::client::gui_ipc_base(
                &server.listen_addr,
                crate::ipc::handlers::client::configured_gui_port(),
            )
            .map(|b| ("IPC_BASE", b))
            .into_iter()
            .collect();
            match crate::boot_selector::spawn_gui_with(&gui, &[TRAY_ARG.to_string()], &env) {
                Ok(pid) => {
                    tracing::info!("[Tray] Started the tray app (pid {})", pid);
                    state.did_spawn(pid, now);
                    started = Some((pid, unix_now()));
                }
                Err(e) => {
                    tracing::warn!("[Tray] Could not start the tray app: {}", e);
                    state.spawn_failed(now);
                }
            }
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(CHECK_INTERVAL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_five_minutes() {
        assert_eq!(backoff(1), Duration::from_secs(5));
        assert_eq!(backoff(2), Duration::from_secs(10));
        assert_eq!(backoff(3), Duration::from_secs(20));
        assert_eq!(backoff(10), BACKOFF_MAX);
        assert_eq!(backoff(u32::MAX), BACKOFF_MAX);
    }

    #[test]
    fn the_first_missing_host_is_started_right_away() {
        let mut r = Respawn::default();
        assert!(r.may_spawn(Instant::now()));
    }

    #[test]
    fn a_host_that_keeps_dying_quickly_waits_longer_each_time() {
        let t0 = Instant::now();
        let mut r = Respawn::default();
        r.did_spawn(10, t0);
        // 5초 뒤 사라졌다: 실패 1, 5초 기다린다
        let t1 = t0 + Duration::from_secs(5);
        assert!(!r.may_spawn(t1));
        assert!(!r.may_spawn(t1 + Duration::from_secs(4)));
        assert!(r.may_spawn(t1 + Duration::from_secs(5)));
        r.did_spawn(11, t1 + Duration::from_secs(5));
        // 또 금방 사라졌다: 실패 2, 10초
        let t2 = t1 + Duration::from_secs(10);
        assert!(!r.may_spawn(t2));
        assert!(!r.may_spawn(t2 + Duration::from_secs(9)));
        assert!(r.may_spawn(t2 + Duration::from_secs(10)));
    }

    #[test]
    fn a_host_that_lived_long_enough_clears_the_failures() {
        let t0 = Instant::now();
        let mut r = Respawn::default();
        r.did_spawn(10, t0);
        assert!(!r.may_spawn(t0 + Duration::from_secs(1)));
        r.did_spawn(11, t0 + Duration::from_secs(6));
        // 살아 있는 것이 보이고 30초가 지났다
        r.host_seen(t0 + Duration::from_secs(40));
        assert_eq!(r.fails, 0);
        // 그 뒤 사라지면 바로 다시 띄운다
        assert!(r.may_spawn(t0 + Duration::from_secs(600)));
    }

    #[tokio::test]
    async fn the_started_host_counts_while_it_lives_even_unregistered() {
        let registry = crate::ipc::ClientRegistry::new();
        let me = std::process::id();
        // 등록하지 못했거나(토큰 어긋남) 레지스트리에서 빠졌어도(절전 뒤) 살아 있으면 또 띄우지 않는다
        assert!(host_alive(&registry, Some((me, unix_now()))).await);
        // 그 PID를 나중에 다른 프로세스가 받았으면(시작 시각이 띄운 때보다 늦다) 없는 것이다
        assert!(!host_alive(&registry, Some((me, 0))).await);
        assert!(!host_alive(&registry, None).await);
    }

    #[test]
    fn a_host_found_dead_after_a_long_life_is_started_again_at_once() {
        let t0 = Instant::now();
        let mut r = Respawn::default();
        r.did_spawn(10, t0);
        assert!(r.may_spawn(t0 + Duration::from_secs(3600)));
        assert_eq!(r.fails, 0);
    }
}
