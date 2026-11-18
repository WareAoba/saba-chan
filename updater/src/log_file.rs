//! 크기 상한이 있는 로그 파일 (데몬·GUI가 같이 쓴다)

use std::sync::{Arc, Mutex};

/// 크기 상한이 있는 로그 파일 (데몬 `<data>/logs/daemon.log`, GUI `<data>/logs/saba-chan.log`).
/// `cap` 바이트를 넘으면 `.1`로 넘기고 새로 쓴다
pub struct LogFile {
    path: std::path::PathBuf,
    cap: u64,
    file: Mutex<Option<(std::fs::File, u64)>>,
}

impl LogFile {
    pub fn open(path: &std::path::Path, cap: u64) -> Arc<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = Self::append(path);
        Arc::new(Self { path: path.to_path_buf(), cap, file: Mutex::new(file) })
    }

    fn append(path: &std::path::Path) -> Option<(std::fs::File, u64)> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some((file, len))
    }

    /// tracing `with_writer`에 넘길 쓰기 핸들
    pub fn writer(self: &Arc<Self>) -> LogFileWriter {
        LogFileWriter(self.clone())
    }

    /// 패닉 훅에서 부른다: tracing을 거치지 않고 바로 쓴다
    pub fn write_panic(&self, message: &str) {
        let line = format!("{} PANIC {}\n", chrono::Utc::now().to_rfc3339(), message);
        self.write_line(line.as_bytes());
    }

    /// 쓰기 실패는 삼킨다: 로그 때문에 프로그램이 멈추거나 죽지 않게
    fn write_line(&self, buf: &[u8]) {
        let Ok(mut guard) = self.file.lock() else { return };
        if guard.as_ref().is_some_and(|(_, len)| *len > 0 && len + buf.len() as u64 > self.cap) {
            *guard = None;
            let mut rolled = self.path.as_os_str().to_os_string();
            rolled.push(".1");
            if std::fs::rename(&self.path, rolled).is_err() {
                // 넘기지 못하면 비우고 새로 쓴다: 상한을 지키는 쪽이 낫다
                let _ = std::fs::OpenOptions::new().write(true).truncate(true).open(&self.path);
            }
        }
        if guard.is_none() {
            *guard = Self::append(&self.path);
        }
        if let Some((file, len)) = guard.as_mut() {
            if std::io::Write::write_all(file, buf).is_ok() {
                *len += buf.len() as u64;
            }
        }
    }
}

pub struct LogFileWriter(Arc<LogFile>);

impl std::io::Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_line(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 데몬 자신의 로그 파일: 크기 상한을 넘으면 `.1`로 넘기고, 패닉도 같은 파일에 남는다
    #[test]
    fn the_log_file_rolls_over_and_keeps_panics() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("logs").join("daemon.log");
        let log = LogFile::open(&path, 32);
        let mut w = log.writer();
        std::io::Write::write_all(&mut w, b"first line that is long enough to fill the cap..\n").unwrap();
        std::io::Write::write_all(&mut w, b"second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
        assert!(std::fs::read_to_string(path.with_extension("log.1")).unwrap().starts_with("first line"));
        log.write_panic("boom at src/x.rs:1");
        assert!(std::fs::read_to_string(&path).unwrap().contains("PANIC boom at src/x.rs:1"));
    }

    /// `.1`로 넘기지 못해도(이름 바꾸기 실패) 파일이 끝없이 커지지 않는다
    #[test]
    fn the_log_file_stays_capped_when_it_cannot_roll_over() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("daemon.log");
        let blocker = path.with_extension("log.1");
        std::fs::create_dir_all(blocker.join("busy")).unwrap();
        let log = LogFile::open(&path, 32);
        let mut w = log.writer();
        for _ in 0..20 {
            std::io::Write::write_all(&mut w, b"a line of about twenty\n").unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= 32, "the file must not grow past the cap");
    }
}
