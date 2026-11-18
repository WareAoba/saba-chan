//! 로그 — <data>/logs/saba-chan.log(+ .1)와 stderr에 같이 쓴다

use saba_chan_updater_lib::log_file::LogFile;
use std::path::Path;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// GUI 로그 파일 상한. 넘으면 `.1`로 넘긴다 (둘이 합쳐 이 값의 두 배까지). GUI가 띄운 데몬의 출력도 여기에 옮겨 적는다
const GUI_LOG_CAP: u64 = 5 * 1024 * 1024;

/// 로그 파일을 열고 전역 구독자를 단다. 파일을 못 열면 stderr만 쓴다
pub fn init(data_dir: &Path) {
    let dir = data_dir.join("logs");
    remove_run_logs(&dir);
    let path = dir.join("saba-chan.log");
    let file = LogFile::open(&path, GUI_LOG_CAP);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_ansi(false).with_writer(move || file.writer()))
        .try_init();

    // 패닉도 로그 파일에 남긴다
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("Panic: {}", info);
        default_hook(info);
    }));

    tracing::info!("Log file: {}", path.display());
}

/// 0.1.14까지 실행마다 새로 만들던 `saba-chan-<시각>.log`를 지운다 (지우는 코드가 없어 계속 쌓였다)
fn remove_run_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("saba-chan-") && name.ends_with(".log") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
