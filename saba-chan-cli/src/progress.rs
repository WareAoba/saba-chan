//! 진행 표시 (stderr). 터미널이 아니면 보이지 않는다.

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::time::Duration;

fn target() -> ProgressDrawTarget {
    ProgressDrawTarget::stderr()
}

pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(None, target());
    pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

/// 0-100 퍼센트 막대
pub fn percent_bar(msg: &str) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(Some(100), target());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/blue}] {pos:>3}% {msg}")
            .unwrap()
            .progress_chars("━╸ "),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(120));
    pb
}

/// 바이트 막대
pub fn bytes_bar(msg: &str) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(Some(0), target());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {msg}")
            .unwrap()
            .progress_chars("━╸ "),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(120));
    pb
}
