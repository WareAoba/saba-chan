//! 실제 디스크 IO (기계 전체, 초당 바이트)
//!
//! 프로세스 IO 카운터는 캐시에서 읽은 것까지 세서 실제 디스크보다 훨씬 크게 나온다.
//! 그래서 전체 디스크 IO는 OS의 물리 디스크 카운터로 잰다.
//! - Windows: PDH `\PhysicalDisk(_Total)\Disk Read Bytes/sec`·`Disk Write Bytes/sec`
//! - Linux: `/proc/diskstats`의 섹터 수 (파티션·loop·dm 같은 겹치는 장치는 뺀다)
//! - 그 밖: 없음 (`None`)
//!
//! 둘 다 두 번 잰 차이로 비율을 낸다: 첫 `refresh`는 `None`이다.

pub struct PhysicalIo {
    inner: imp::Counter,
}

impl PhysicalIo {
    pub fn new() -> Self {
        Self { inner: imp::Counter::new() }
    }

    /// 직전 `refresh` 이후의 (읽기, 쓰기) 초당 바이트. 잴 수 없으면 `None`
    pub fn refresh(&mut self) -> Option<(u64, u64)> {
        self.inner.refresh()
    }
}

/// `/proc/diskstats`에서 실제 디스크만 골라 (읽은 바이트, 쓴 바이트) 누계를 낸다.
/// 필드: major minor name reads merged sectors_read ms writes merged sectors_written ...
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_diskstats(text: &str, is_disk: impl Fn(&str) -> bool) -> (u64, u64) {
    const SECTOR: u64 = 512; // 커널은 장치 섹터 크기와 상관없이 512바이트 단위로 센다
    let mut read = 0u64;
    let mut written = 0u64;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !is_disk(f[2]) {
            continue;
        }
        read += f[5].parse::<u64>().unwrap_or(0) * SECTOR;
        written += f[9].parse::<u64>().unwrap_or(0) * SECTOR;
    }
    (read, written)
}

/// 다른 장치 위에 얹힌 가상 장치 (같은 IO를 두 번 센다)
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_layered(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"].iter().any(|p| name.starts_with(p))
}

#[cfg(target_os = "linux")]
mod imp {
    use std::time::Instant;

    pub struct Counter {
        last: Option<(Instant, u64, u64)>,
    }

    impl Counter {
        pub fn new() -> Self {
            Self { last: None }
        }

        pub fn refresh(&mut self) -> Option<(u64, u64)> {
            let text = std::fs::read_to_string("/proc/diskstats").ok()?;
            // /sys/block에 있는 이름만 디스크 전체다 (파티션은 그 아래에 있다)
            let (read, written) = super::parse_diskstats(&text, |name| {
                !super::is_layered(name) && std::path::Path::new("/sys/block").join(name).exists()
            });
            let now = Instant::now();
            let prev = self.last.replace((now, read, written));
            let (t, r, w) = prev?;
            let secs = now.duration_since(t).as_secs_f64();
            if secs <= 0.0 {
                return None;
            }
            Some(((read.saturating_sub(r) as f64 / secs) as u64, (written.saturating_sub(w) as f64 / secs) as u64))
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ptr::null_mut;
    use winapi::um::pdh::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    };

    pub struct Counter {
        query: PDH_HQUERY,
        read: PDH_HCOUNTER,
        write: PDH_HCOUNTER,
        primed: bool,
    }

    // PDH 핸들은 스레드에 묶이지 않는다. 접근은 SystemMonitor의 Mutex가 한 번에 하나로 막는다
    unsafe impl Send for Counter {}

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl Counter {
        pub fn new() -> Self {
            let mut c = Self { query: null_mut(), read: null_mut(), write: null_mut(), primed: false };
            unsafe {
                if PdhOpenQueryW(null_mut(), 0, &mut c.query) != 0 {
                    c.query = null_mut();
                    return c;
                }
                // English 이름: 윈도우 표시 언어와 상관없이 같은 카운터
                let r = PdhAddEnglishCounterW(c.query, wide(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec").as_ptr(), 0, &mut c.read);
                let w = PdhAddEnglishCounterW(c.query, wide(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec").as_ptr(), 0, &mut c.write);
                if r != 0 || w != 0 {
                    PdhCloseQuery(c.query);
                    c.query = null_mut();
                }
            }
            c
        }

        pub fn refresh(&mut self) -> Option<(u64, u64)> {
            if self.query.is_null() {
                return None;
            }
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                // 비율 카운터는 두 번째 수집부터 값이 있다
                if !std::mem::replace(&mut self.primed, true) {
                    return None;
                }
                Some((value(self.read)?, value(self.write)?))
            }
        }
    }

    unsafe fn value(counter: PDH_HCOUNTER) -> Option<u64> {
        let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
        // CStatus 0(VALID_DATA)·1(NEW_DATA)은 둘 다 쓸 수 있는 값이다
        if PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, null_mut(), &mut v) != 0 || v.CStatus > 1 {
            return None;
        }
        Some(v.u.doubleValue().max(0.0) as u64)
    }

    impl Drop for Counter {
        fn drop(&mut self) {
            if !self.query.is_null() {
                unsafe { PdhCloseQuery(self.query) };
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub struct Counter;

    impl Counter {
        pub fn new() -> Self {
            Self
        }

        pub fn refresh(&mut self) -> Option<(u64, u64)> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats_counts_whole_disks_once() {
        let text = "\
   8       0 sda 100 0 2000 0 50 0 4000 0 0 0 0
   8       1 sda1 90 0 1800 0 40 0 3000 0 0 0 0
   7       0 loop0 5 0 80 0 0 0 0 0 0 0 0
 253       0 dm-0 70 0 1500 0 30 0 2500 0 0 0 0
 259       0 nvme0n1 10 0 100 0 5 0 60 0 0 0 0
 259       1 nvme0n1p1 9 0 90 0 4 0 50 0 0 0 0
";
        let disks = ["sda", "loop0", "dm-0", "nvme0n1"];
        let (r, w) = parse_diskstats(text, |n| disks.contains(&n) && !is_layered(n));
        assert_eq!((r, w), ((2000 + 100) * 512, (4000 + 60) * 512));
    }

    #[test]
    fn first_refresh_has_no_rate_and_second_never_panics() {
        let mut io = PhysicalIo::new();
        assert!(io.refresh().is_none());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = io.refresh();
    }
}
