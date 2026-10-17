//! saba-chan CLI — 인터랙티브 TUI 모듈
//!
//! `main.rs`에서 `tui::run(client).await`로 호출된다.
//! 계층적 메뉴 내비게이션 + vim-like 편집기 + 레거시 커맨드 모드를 제공한다.
//!
//! ## 구조
//!
//! - [`app`] — `App` 상태, `Screen`/`InputMode`/`Focus` 열거형, 공유 타입
//! - [`event_loop`] — 메인 루프, 키/마우스 라우팅, 비동기 메시지 플러시
//! - [`commands`] — 레거시 `:command` 파서/실행기
//! - [`render`] — ratatui 렌더링 파이프라인
//! - [`screens`] — 화면별 메뉴 빌드·선택 핸들러
//! - [`theme`] — 색상 팔레트
//!
//! `run()`은 터미널 초기화·백그라운드 태스크 스폰·종료 정리만 담당하고,
//! 실제 이벤트 디스패치는 [`event_loop::run_event_loop`]로 위임한다.

pub mod app;
pub mod autocomplete;
pub mod commands;
pub mod event_loop;
pub mod overlay;
pub mod render;
pub mod screens;
pub mod session;
pub mod slash;
pub mod tabs;
pub mod theme;

use std::io::{self, stdout};
use std::time::Duration;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use app::*;
use crate::client::DaemonClient;
use crate::config;
use crate::process;

struct TerminalCleanup<F: Fn()>(F);

impl<F: Fn()> Drop for TerminalCleanup<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

// ═══════════════════════════════════════════════════════
// 엔트리포인트
// ═══════════════════════════════════════════════════════

pub async fn run(client: DaemonClient) -> anyhow::Result<()> {
    // 패닉 시 터미널 복원
    event_loop::install_panic_hook();

    // 터미널 초기화
    enable_raw_mode()?;
    let terminal_cleanup = TerminalCleanup(|| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            crossterm::cursor::Show,
        );
    });
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // 앱 상태 생성
    // NOTE: `App::new`는 내부적으로 blocking `reqwest::Client`를 생성/drop한다.
    // tokio 런타임 컨텍스트에서
    // 직접 호출하면 drop 시 "Cannot drop a runtime..." 패닉이 발생하므로
    // `spawn_blocking`으로 격리한다.
    let client_for_app = client.clone();
    let mut app = tokio::task::spawn_blocking(move || App::new(client_for_app)).await?;
    // Stage 5: 세션 로드 — 테마 및 히스토리 복원
    let session = session::Session::load();
    theme::Theme::set_mode(&session.theme);
    if !session.history.is_empty() {
        app.history = session.history.clone();
    }
    if !session.slash_history.is_empty() {
        app.slash_history = session.slash_history.clone();
    }
    app.session = Some(session);
    // 초기 메뉴 빌드
    app.menu_items = screens::build_menu(&app);

    // ── 백그라운드 태스크 ──────────────────────────────
    let (status_tx, mut status_rx) = mpsc::unbounded_channel::<Snapshot>();

    // 1) 상태 모니터 (1초마다 폴링)
    {
        let client = client.clone();
        let tx = status_tx.clone();
        tokio::spawn(async move {
            loop {
                let daemon = tokio::task::spawn_blocking(process::check_daemon_running).await.unwrap_or(false);
                // blocking reqwest client를 쓰는 설정 조회는 반드시 spawn_blocking으로 격리.
                // async 컨텍스트에서 직접 호출 시 drop 패닉.
                // 데몬이 죽었으면 bot_config/settings는 None(타임아웃).
                let (bot_config, settings_config) = tokio::task::spawn_blocking(|| {
                    let bot_cfg = config::load_bot_config().ok();
                    let settings_cfg = config::load_settings().ok();
                    (bot_cfg, settings_cfg)
                }).await.unwrap_or((None, None));
                let mut servers = Vec::new();
                if daemon {
                    if let Ok(list) = client.list_servers().await {
                        for s in list {
                            let ext_data = s.get("extension_data")
                                .and_then(|v| v.as_object())
                                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                                .unwrap_or_default();
                            servers.push(ServerInfo {
                                name: s["name"].as_str().unwrap_or("?").into(),
                                module: s["module"].as_str().unwrap_or("?").into(),
                                status: s["status"].as_str().unwrap_or("stopped").into(),
                                extension_data: ext_data,
                            });
                        }
                    }
                }
                let _ = tx.send(Snapshot { daemon, servers, bot_config, settings_config });
                tokio::time::sleep(Duration::from_millis(event_loop::app_refresh_interval())).await;
            }
        });
    }

    // 2) 하트비트 등록
    {
        let client = client.clone();
        let client_id = app.client_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Ok(id) = client.register_client("cli").await {
                let mut lock = client_id.lock().unwrap();
                *lock = Some(id.clone());
                drop(lock);
                let id_owned = id;
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        let _ = client.send_heartbeat(&id_owned).await;
                    }
                });
            }
        });
    }

    // 3) 자동 시작
    //
    // 외부에서 이미 떠 있던 데몬·봇은 절대 종료 시 건드리지 않는다.
    // 이 세션이 *실제로* 띄운 경우에만 `stop_*_on_exit` 플래그를 set하고,
    // 종료 정리 단계에서 그 플래그를 확인한다.
    {
        let buf = app.async_out.clone();
        let auto_start = app.settings.auto_start;
        let stop_daemon_flag = app.stop_daemon_on_exit.clone();
        tokio::spawn(async move {
            use std::sync::atomic::Ordering;
            if !auto_start { return; }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let daemon_running = tokio::task::spawn_blocking(process::check_daemon_running).await.unwrap_or(false);
            if !daemon_running {
                match tokio::task::spawn_blocking(process::start_daemon).await {
                    Ok(Ok(msg)) => {
                        // TUI가 직접 띄웠으므로 종료 시 함께 정리해도 안전.
                        stop_daemon_flag.store(true, Ordering::SeqCst);
                        push_out(&buf, msg.lines().map(|l| Out::Info(l.into())).collect());
                    }
                    Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("Auto-start Saba-Core failed: {}", e))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("Auto-start: {}", e))]),
                }
            }
        });
    }

    // 4) 익스텐션 슬롯 초기 로드 — GUI의 ExtensionProvider.useEffect()에 대응
    //    데몬이 올라올 때까지 대기 후, 활성 익스텐션 목록을 가져와
    //    CLI 슬롯 레지스트리를 빌드한다.
    {
        let client = client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            // 데몬이 올라올 때까지 최대 30초 대기
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if client.list_servers().await.is_ok() {
                    break;
                }
            }
            // 익스텐션 목록 로드 → EXT_SLOTS_INIT 프로토콜로 전달
            if let Ok(exts) = client.list_extensions().await {
                let data = serde_json::to_string(&exts).unwrap_or_default();
                push_out(&buf, vec![Out::Text(format!("EXT_SLOTS_INIT:{}", data))]);
            }
        });
    }

    // 5) 최근 탭 복원 (Stage 6)
    //    세션에 기록된 `recent_tabs`를 재개설한다. 데몬이 이미 올라와 있으면
    //    서버 존재 여부를 확인하여 삭제된 인스턴스는 건너뛴다. 데몬이 아직
    //    기동 중이면 검증 없이 복원하고, 이후 폴러가 조용히 재시도한다.
    {
        use std::collections::HashSet;
        let valid_ids: Option<HashSet<String>> = match tokio::time::timeout(
            Duration::from_millis(500),
            client.list_servers(),
        ).await {
            Ok(Ok(list)) => Some(
                list.iter()
                    .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect()
            ),
            _ => None,
        };
        let restored = app.restore_recent_tabs(valid_ids.as_ref());
        if restored > 0 {
            let msg = app.i18n.t_with("flash.restored_tabs", &[("count", &restored.to_string())]);
            app.flash(&msg);
        }
    }

    // ── 메인 이벤트 루프 ──────────────────────────────
    event_loop::run_event_loop(&mut app, &mut terminal, &client, &mut status_rx).await?;
    // ── 정리 ──────────────────────────────────────────
    // Stage 5: 세션 저장 — 테마 + 히스토리 + 최근 탭
    if let Some(mut s) = app.session.take() {
        s.theme = theme::Theme::mode_name().to_string();
        s.history = app.history.clone();
        s.slash_history = app.slash_history.clone();
        s.recent_tabs = app.tabs.tabs.iter().map(|t| match &t.source {
            tabs::TabSource::Server { id, name, module } => session::RecentTab::Server {
                id: id.clone(),
                name: name.clone(),
                module: module.clone(),
            },
            tabs::TabSource::Daemon => session::RecentTab::Daemon,
            tabs::TabSource::ExtProcess { name } => session::RecentTab::ExtProcess {
                name: name.clone(),
            },
        }).collect();
        let _ = s.save();
    }

    drop(terminal_cleanup);

    // 클라이언트 등록 해제
    let maybe_id = app.client_id.lock().unwrap().take();
    if let Some(id) = maybe_id {
        let _ = client.unregister_client(&id).await;
    }

    // 데몬 정지는 **이 TUI 세션이 직접 시작한 경우에만** 수행한다.
    // 외부 데몬을 단순 조회하려고 TUI를 띄웠다 빠져나올 때 운영 중인
    // 게임 서버가 함께 종료되는 사고를 막기 위함.
    use std::sync::atomic::Ordering;
    if app.stop_daemon_on_exit.load(Ordering::SeqCst) && process::check_daemon_running() {
        let _ = tokio::task::spawn_blocking(process::stop_daemon).await;
    }

    // 백그라운드 태스크(상태 폴링, 하트비트 등)가 남아있으면
    // tokio shutdown 시 blocking task 대기로 빈 콘솔 창이 잠시 보일 수 있으므로
    // 즉시 프로세스를 종료한다.
    let _ = io::stdout();
    std::process::exit(0);
}

#[cfg(test)]
mod terminal_cleanup_tests {
    use super::TerminalCleanup;
    use std::cell::Cell;

    fn fail_after_initialization(restored: &Cell<bool>) -> anyhow::Result<()> {
        let _cleanup = TerminalCleanup(|| restored.set(true));
        Err(anyhow::anyhow!("terminal operation failed"))
    }

    #[test]
    fn terminal_cleanup_runs_when_operation_returns_error() {
        let restored = Cell::new(false);
        assert!(fail_after_initialization(&restored).is_err());
        assert!(restored.get());
    }

    #[test]
    fn terminal_cleanup_runs_during_unwind() {
        let restored = Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _cleanup = TerminalCleanup(|| restored.set(true));
            panic!("terminal operation panicked");
        }));
        assert!(result.is_err());
        assert!(restored.get());
    }
}
