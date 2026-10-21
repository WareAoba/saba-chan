//! 전체 화면 TUI — 인자 없이 터미널에서 실행하면 열린다 (`saba-chan-cli tui`도 같다)
//!
//! 메뉴는 명령 표에서 만들고, 메뉴에서 고른 일은 같은 뜻의 명령 한 줄로 실행한다. 그래서 메뉴에서 되는 일은
//! 명령으로도 되고, 결과 창 제목에 그 명령이 남는다. 출력·진행·묻기 창구를 바꿔 끼워 명령의 출력은 결과 창에,
//! 진행은 아래 작업 줄에, 확인 질문은 화면 안 대화상자에 그린다. 상태는 몇 초마다 다시 읽는다.

mod app;
mod view;

use crate::ctx::Ctx;
use crate::error::{classify, Result};
use app::{Action, Answer, App, AskKind, Dialog, Job};
use clap::FromArgMatches;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// 상태를 다시 읽는 간격 (데몬 이벤트가 늘면 줄인다)
const REFRESH: Duration = Duration::from_secs(3);

enum Msg {
    Term(Event),
    Line(String),
    Progress(u64, crate::progress::Update),
    Ask(Dialog),
    Refreshed(Option<Value>, Vec<Value>),
    /// 몇 번째로 시작한 명령이 끝났는지 (취소 뒤 늦게 온 끝 소식이 새 명령을 끝내지 않게)
    Done(u64),
    /// 명령 작업이 패닉함. 기본 훅이 대체 화면 위에 직접 쓴 글자를 지우도록 화면을 통째로 다시 그린다
    Panicked,
    Tick,
}

/// 확인 질문을 화면 안 대화상자로
struct TuiAsker(UnboundedSender<Msg>);

impl TuiAsker {
    fn ask(&self, question: &str, kind: AskKind, cursor: usize) -> Option<Answer> {
        let (reply, rx) = std::sync::mpsc::channel();
        let dialog = Dialog { question: question.to_string(), kind, cursor, input: String::new(), reply };
        self.0.send(Msg::Ask(dialog)).ok()?;
        // 명령은 여러 스레드 런타임의 작업 하나에서 돈다: 답을 기다리는 동안 그 일꾼만 비켜 둔다
        tokio::task::block_in_place(|| rx.recv()).ok().flatten()
    }
}

impl crate::prompt::Asker for TuiAsker {
    fn confirm(&self, question: &str, default: bool) -> Option<bool> {
        match self.ask(question, AskKind::Confirm { default }, if default { 0 } else { 1 })? {
            Answer::Yes(y) => Some(y),
            _ => None,
        }
    }
    fn select(&self, question: &str, items: &[String], default: usize) -> Option<usize> {
        match self.ask(question, AskKind::Select { items: items.to_vec() }, default)? {
            Answer::Index(i) => Some(i),
            _ => None,
        }
    }
    fn text(&self, question: &str, default: Option<&str>, allow_empty: bool) -> Option<String> {
        let q = match default {
            Some(d) if !d.is_empty() => format!("{question} [{d}]"),
            _ => question.to_string(),
        };
        match self.ask(&q, AskKind::Text { allow_empty: allow_empty || default.is_some(), secret: false }, 0)? {
            Answer::Text(t) if t.is_empty() => default.map(str::to_string).or(Some(t)),
            Answer::Text(t) => Some(t),
            _ => None,
        }
    }
    fn secret(&self, question: &str) -> Option<String> {
        match self.ask(question, AskKind::Text { allow_empty: false, secret: true }, 0)? {
            Answer::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// 터미널을 원래대로 (끝날 때, 그리고 패닉 때)
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

pub async fn run(ctx: &Ctx) -> Result<()> {
    use std::io::IsTerminal;
    // `saba-chan-cli tui`로 불러도 인자 없을 때와 같은 조건에서만 연다
    if !ctx.ask.enabled || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(crate::error::CliError::Usage(t!("tui.needs_terminal")).into());
    }
    let mut ctx = ctx.clone();
    ctx.in_shell = true;
    let menu = app::menu_from(&crate::cli::localized());
    let mut app = App::new(menu);

    let (tx, mut rx) = unbounded_channel::<Msg>();
    // 명령 출력에는 색 코드를 넣지 않는다 (화면은 ratatui가 칠한다)
    let colors = (console::colors_enabled(), console::colors_enabled_stderr());
    console::set_colors_enabled(false);
    console::set_colors_enabled_stderr(false);
    let out_tx = tx.clone();
    let _out = crate::output::SinkGuard::install(Box::new(move |_, line| {
        let _ = out_tx.send(Msg::Line(line.to_string()));
    }));
    let progress_tx = tx.clone();
    let _progress = crate::progress::SinkGuard::install(Box::new(move |id, update| {
        let _ = progress_tx.send(Msg::Progress(id, update));
    }));
    let _asker = crate::prompt::AskerGuard::install(Box::new(TuiAsker(tx.clone())));

    // 화면을 그리는 이 스레드가 패닉할 때만 터미널을 되돌린다 (명령 작업의 패닉은 결과 창에 알린다)
    let prev_hook: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync> = std::panic::take_hook().into();
    let ui_thread = std::thread::current().id();
    {
        let prev_hook = prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == ui_thread {
                restore();
            }
            prev_hook(info);
        }));
    }
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;

    // 키·마우스·창 크기는 따로 읽어 넘긴다. 끝나면 멈춘다
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (tx, stop) = (tx.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    match event::read() {
                        Ok(e) => {
                            if tx.send(Msg::Term(e)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        })
    };
    let ticker = {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                if tx.send(Msg::Tick).is_err() {
                    break;
                }
                tokio::time::sleep(REFRESH).await;
            }
        })
    };

    let mut task: Option<tokio::task::AbortHandle> = None;
    let mut seq = 0u64;
    let mut refreshing = false;
    term.draw(|f| view::draw(f, &mut app))?;
    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::Term(Event::Key(k)) if k.kind == KeyEventKind::Press => match app.on_key(k) {
                Some(Action::Quit) => break,
                Some(Action::Cancel) => {
                    if let Some(t) = task.take() {
                        t.abort();
                    }
                    app.running = false;
                    app.jobs.clear();
                    app.push_line(&t!("tui.cancelled"));
                }
                Some(Action::Run(line)) => {
                    // 셸처럼 줄마다 포트를 다시 찾는다: 설정 포트를 바꾸거나 데몬을 다시 켰을 수 있다 (L62)
                    ctx.follow_daemon_port();
                    seq += 1;
                    task = start(&ctx, &mut app, &line, seq, tx.clone()).or(task);
                }
                None => {}
            },
            Msg::Term(Event::Mouse(m)) => {
                if let Some(Action::Run(line)) = app.on_mouse(m) {
                    ctx.follow_daemon_port();
                    seq += 1;
                    task = start(&ctx, &mut app, &line, seq, tx.clone()).or(task);
                }
            }
            Msg::Term(_) => {}
            Msg::Line(l) => {
                app.push_line(&l);
                app.scroll = 0;
            }
            Msg::Progress(id, u) => {
                use crate::progress::Update;
                match u {
                    Update::Start { message, .. } => {
                        app.jobs.insert(id, Job { message, ..Job::default() });
                    }
                    Update::Message(m) => {
                        if let Some(j) = app.jobs.get_mut(&id) {
                            j.message = m;
                        }
                    }
                    Update::Position(p) => {
                        if let Some(j) = app.jobs.get_mut(&id) {
                            j.position = p;
                        }
                    }
                    Update::Length(l) => {
                        if let Some(j) = app.jobs.get_mut(&id) {
                            j.length = Some(l);
                        }
                    }
                    Update::Done => {
                        app.jobs.remove(&id);
                    }
                }
            }
            Msg::Ask(d) => app.dialog = Some(d),
            Msg::Done(id) => {
                if id != seq {
                    continue;
                }
                task = None;
                app.running = false;
                // 명령이 상태를 바꿨을 수 있으니(데몬 포트까지) 곧바로 다시 읽는다
                ctx.follow_daemon_port();
                refreshing = refresh(&ctx, tx.clone(), refreshing);
            }
            Msg::Panicked => {
                app.push_line(&t!("tui.command_panicked"));
                app.scroll = 0;
                term.clear()?;
            }
            Msg::Tick => refreshing = refresh(&ctx, tx.clone(), refreshing),
            Msg::Refreshed(h, servers) => {
                refreshing = false;
                app.health = h;
                app.servers = servers;
            }
        }
        term.draw(|f| view::draw(f, &mut app))?;
    }

    stop.store(true, Ordering::Relaxed);
    ticker.abort();
    if let Some(t) = task {
        t.abort();
    }
    let _ = reader.join();
    drop(term);
    drop(_restore);
    console::set_colors_enabled(colors.0);
    console::set_colors_enabled_stderr(colors.1);
    std::panic::set_hook(Box::new(move |info| prev_hook(info)));
    Ok(())
}

/// 데몬 상태와 서버 목록을 다시 읽는다 (이미 읽는 중이면 건너뛴다)
fn refresh(ctx: &Ctx, tx: UnboundedSender<Msg>, busy: bool) -> bool {
    if busy {
        return true;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let health = ctx.api.health().await.ok();
        let servers = match health {
            Some(_) => ctx
                .api
                .servers(true)
                .await
                .ok()
                .and_then(|v| v.get("servers").and_then(Value::as_array).cloned())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let _ = tx.send(Msg::Refreshed(health, servers));
    });
    true
}

/// 명령 한 줄을 실행한다. 결과 창 제목에 그 명령을 남긴다
fn start(ctx: &Ctx, app: &mut App, line: &str, id: u64, tx: UnboundedSender<Msg>) -> Option<tokio::task::AbortHandle> {
    if app.running {
        return None;
    }
    let Some(words) = shlex::split(line) else {
        app.push_line(&t!("shell.bad_quotes"));
        return None;
    };
    if words.is_empty() {
        return None;
    }
    if let Some(why) = app::refuse(&words) {
        app.push_line(&why);
        return None;
    }
    let words = app::adapt(words);
    let shown = shlex::try_join(words.iter().map(String::as_str)).unwrap_or_else(|_| line.to_string());
    app.push_line("");
    app.push_line(&format!("› {shown}"));
    app.last_command = Some(shown);
    app.running = true;
    app.scroll = 0;
    let ctx = ctx.clone();
    let work = tokio::spawn(async move {
        if let Err(e) = one(&ctx, &words).await {
            ctx.out.error(&classify(&e));
        }
        crate::output::flush();
    });
    let abort = work.abort_handle();
    // 명령이 패닉해도 결과 창에 알리고 "실행 중"을 푼다
    tokio::spawn(async move {
        if work.await.is_err_and(|e| e.is_panic()) {
            let _ = tx.send(Msg::Panicked);
        }
        let _ = tx.send(Msg::Done(id));
    });
    Some(abort)
}

/// 셸과 같은 규칙으로 한 줄을 실행: 명령이 아니면 디스코드와 같은 채팅 처리기로
async fn one(ctx: &Ctx, words: &[String]) -> Result<()> {
    let top: Vec<String> = crate::cli::command_words().into_iter().filter(|w| !w.contains(' ')).collect();
    if !(top.contains(&words[0]) || words[0] == "help" || words[0].starts_with('-')) {
        return crate::cmd::chat(ctx, &words.join(" ")).await;
    }
    let argv = std::iter::once("saba-chan-cli".to_string()).chain(words.iter().cloned());
    let matches = match crate::cli::parse(argv.map(Into::into).collect()) {
        Ok(m) => m,
        Err(e) => {
            // 사용법 오류·도움말도 결과 창에 (터미널에 직접 찍으면 화면이 깨진다)
            say!("{}", e.render());
            return Ok(());
        }
    };
    let cli = crate::cli::Cli::from_arg_matches(&matches)?;
    let mut ctx = ctx.clone();
    if let Some(port) = cli.global.port {
        ctx = ctx.on_port(port);
    }
    if cli.global.json || cli.global.json_stream {
        ctx.out = crate::output::Out { json: true };
        ctx.ask.enabled = false;
    }
    if cli.global.yes {
        ctx.ask.yes = true;
    }
    match cli.cmd {
        Some(cmd) => crate::cmd::dispatch(&ctx, cmd).await,
        None => Ok(()),
    }
}
