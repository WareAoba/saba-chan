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

/// 창구를 바꿔 끼운다. 놓으면 터미널 막대로 돌아간다 (TUI가 끼운다. 그 전까지는 시험만 쓴다)
#[must_use]
#[cfg_attr(not(test), allow(dead_code))]
pub struct SinkGuard(());

#[cfg_attr(not(test), allow(dead_code))]
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
