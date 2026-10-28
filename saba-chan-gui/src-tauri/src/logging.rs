//! 로그 — <data>/logs/saba-chan-<ts>.log와 stderr에 같이 쓴다

use std::path::Path;
use std::sync::Mutex;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// 로그 파일을 열고 전역 구독자를 단다. 파일을 못 열면 stderr만 쓴다
pub fn init(data_dir: &Path) {
    let dir = data_dir.join("logs");
    let file = std::fs::create_dir_all(&dir).ok().and_then(|_| {
        // main.js와 같은 이름: ISO 시각에서 :·. 를 -로, 밀리초는 뺀다 (UTC)
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
        let path = dir.join(format!("saba-chan-{ts}.log"));
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
            .map(|f| (path, f))
    });
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (path, file_layer) = match file {
        Some((p, f)) => (Some(p), Some(fmt::layer().with_ansi(false).with_writer(Mutex::new(f)))),
        None => (None, None),
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .try_init();

    // 패닉도 로그 파일에 남긴다
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("Panic: {}", info);
        default_hook(info);
    }));

    if let Some(p) = &path {
        tracing::info!("Log file: {}", p.display());
    }
}
