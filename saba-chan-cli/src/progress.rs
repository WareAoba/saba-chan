//! 진행 표시 (stderr). 터미널이 아니면 보이지 않는다.
//!
//! 진행 표시도 출력처럼 바꿔 끼울 수 있다: 창구가 끼워져 있으면 막대를 그리지 않고 변화를 창구로 넘긴다
//! (TUI가 작업 줄에 그린다).

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Spinner,
    /// 0-100
    Percent,
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Start { kind: Kind, message: String },
    Message(String),
    Position(u64),
    Length(u64),
    Done,
}

/// 진행 표시 번호와 변화를 받는다
pub type Sink = Box<dyn Fn(u64, Update) + Send + Sync>;

static SINK: std::sync::RwLock<Option<Sink>> = std::sync::RwLock::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn send(id: u64, update: Update) -> bool {
    match SINK.read().ok().as_ref().and_then(|s| s.as_ref()) {
        Some(sink) => {
            sink(id, update);
            true
        }
        None => false,
    }
}

/// 창구를 바꿔 끼운다. 놓으면 터미널 막대로 돌아간다 (`--json-stream`, 나중에 TUI가 끼운다)
#[must_use]
pub struct SinkGuard(());

impl SinkGuard {
    pub fn install(sink: Sink) -> Self {
        if let Ok(mut s) = SINK.write() {
            *s = Some(sink);
        }
        Self(())
    }
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = SINK.write() {
            *s = None;
        }
    }
}

/// 진행 표시 하나. 끝나면 `finish_and_clear`, 잊어도 놓을 때 끝난다
pub struct Progress {
    id: u64,
    /// 창구가 없을 때 그리는 터미널 막대
    bar: Option<ProgressBar>,
    done: AtomicBool,
}

impl Progress {
    fn start(kind: Kind, msg: &str, bar: impl FnOnce() -> ProgressBar) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let routed = send(
            id,
            Update::Start {
                kind,
                message: msg.to_string(),
            },
        );
        Self {
            id,
            bar: (!routed).then(bar),
            done: AtomicBool::new(false),
        }
    }

    fn update(&self, update: Update, draw: impl FnOnce(&ProgressBar)) {
        match &self.bar {
            Some(bar) => draw(bar),
            None => {
                send(self.id, update);
            }
        }
    }

    pub fn set_message(&self, msg: impl Into<String>) {
        let msg = msg.into();
        self.update(Update::Message(msg.clone()), |b| b.set_message(msg));
    }

    pub fn set_position(&self, pos: u64) {
        self.update(Update::Position(pos), |b| b.set_position(pos));
    }

    pub fn set_length(&self, len: u64) {
        self.update(Update::Length(len), |b| b.set_length(len));
    }

    pub fn finish_and_clear(&self) {
        if !self.done.swap(true, Ordering::Relaxed) {
            self.update(Update::Done, ProgressBar::finish_and_clear);
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if self.bar.is_none() && !self.done.swap(true, Ordering::Relaxed) {
            send(self.id, Update::Done);
        }
    }
}

/// `--json-stream`에서 같은 막대의 위치를 내보내는 최소 간격
const GAP: Duration = Duration::from_millis(250);

/// 진행 변화 하나를 JSON 한 줄로
fn line(id: u64, update: &Update) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({ "type": "progress", "id": id });
    match update {
        Update::Start { kind, message } => {
            let kind = match kind {
                Kind::Spinner => "spinner",
                Kind::Percent => "percent",
                Kind::Bytes => "bytes",
            };
            v["kind"] = json!(kind);
            v["message"] = json!(message);
        }
        Update::Message(m) => v["message"] = json!(m),
        Update::Position(p) => v["position"] = json!(p),
        Update::Length(l) => v["length"] = json!(l),
        Update::Done => v["done"] = json!(true),
    }
    v
}

/// 막대마다 위치를 마지막으로 내보낸 때. 끝나면 지운다
#[derive(Default)]
struct Gate(std::collections::HashMap<u64, std::time::Instant>);

impl Gate {
    fn pass(&mut self, id: u64, update: &Update, now: std::time::Instant) -> bool {
        match update {
            Update::Position(_) => {
                if self.0.get(&id).is_some_and(|last| now.duration_since(*last) < GAP) {
                    return false;
                }
                self.0.insert(id, now);
                true
            }
            Update::Done => {
                self.0.remove(&id);
                true
            }
            _ => true,
        }
    }
}

/// `--json-stream`: 진행을 막대 대신 stdout에 JSON 한 줄씩 낸다
pub fn stream_lines() -> SinkGuard {
    let gate = std::sync::Mutex::new(Gate::default());
    SinkGuard::install(Box::new(move |id, update| {
        let pass = gate
            .lock()
            .map(|mut g| g.pass(id, &update, std::time::Instant::now()))
            .unwrap_or(true);
        if pass {
            say!("{}", line(id, &update));
        }
    }))
}

fn target() -> ProgressDrawTarget {
    ProgressDrawTarget::stderr()
}

pub fn spinner(msg: &str) -> Progress {
    Progress::start(Kind::Spinner, msg, || {
        let pb = ProgressBar::with_draw_target(None, target());
        pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    })
}

/// 0-100 퍼센트 막대
pub fn percent_bar(msg: &str) -> Progress {
    Progress::start(Kind::Percent, msg, || {
        let pb = ProgressBar::with_draw_target(Some(100), target());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/blue}] {pos:>3}% {msg}")
                .unwrap()
                .progress_chars("━╸ "),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(120));
        pb
    })
}

/// 바이트 막대
pub fn bytes_bar(msg: &str) -> Progress {
    Progress::start(Kind::Bytes, msg, || {
        let pb = ProgressBar::with_draw_target(Some(0), target());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {msg}")
                .unwrap()
                .progress_chars("━╸ "),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(120));
        pb
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--json-stream`: 진행 변화가 JSON 한 줄씩. 위치는 자주 바뀌어 사이를 두고 내보낸다
    #[test]
    fn stream_lines_describe_progress_and_skip_rapid_positions() {
        assert_eq!(
            line(
                3,
                &Update::Start {
                    kind: Kind::Bytes,
                    message: "down".into()
                }
            ),
            serde_json::json!({ "type": "progress", "id": 3, "kind": "bytes", "message": "down" })
        );
        assert_eq!(
            line(3, &Update::Position(5)),
            serde_json::json!({ "type": "progress", "id": 3, "position": 5 })
        );
        assert_eq!(
            line(3, &Update::Done),
            serde_json::json!({ "type": "progress", "id": 3, "done": true })
        );

        let mut gate = Gate::default();
        let t0 = std::time::Instant::now();
        assert!(gate.pass(1, &Update::Position(1), t0));
        assert!(!gate.pass(1, &Update::Position(2), t0 + Duration::from_millis(50)));
        assert!(
            gate.pass(2, &Update::Position(1), t0 + Duration::from_millis(50)),
            "each bar has its own gap"
        );
        assert!(
            gate.pass(1, &Update::Message("x".into()), t0 + Duration::from_millis(60)),
            "only positions wait"
        );
        assert!(gate.pass(1, &Update::Position(3), t0 + GAP));
        assert!(gate.pass(1, &Update::Done, t0 + GAP));
    }

    /// 창구가 끼워져 있으면 막대 대신 변화가 창구로 간다. 끝내기를 잊어도 놓을 때 끝을 알린다
    #[test]
    fn progress_goes_to_the_installed_sink() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u64, Update)>::new()));
        let rec = seen.clone();
        let _guard = SinkGuard::install(Box::new(move |id, u| rec.lock().unwrap().push((id, u))));

        let bar = bytes_bar("down");
        bar.set_length(10);
        bar.set_position(4);
        bar.set_message("mod-a");
        bar.finish_and_clear();
        drop(spinner("wait"));

        let got = seen.lock().unwrap().clone();
        let a = got[0].0;
        let mine: Vec<Update> = got.iter().filter(|(id, _)| *id == a).map(|(_, u)| u.clone()).collect();
        assert_eq!(
            mine,
            vec![
                Update::Start {
                    kind: Kind::Bytes,
                    message: "down".into()
                },
                Update::Length(10),
                Update::Position(4),
                Update::Message("mod-a".into()),
                Update::Done,
            ]
        );
        let b: Vec<&Update> = got.iter().filter(|(id, _)| *id != a).map(|(_, u)| u).collect();
        assert_eq!(
            b,
            vec![
                &Update::Start {
                    kind: Kind::Spinner,
                    message: "wait".into()
                },
                &Update::Done
            ]
        );
    }
}
