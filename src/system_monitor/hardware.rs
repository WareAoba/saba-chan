//! 시스템 모니터의 하드웨어 값: CPU 클럭·온도, RAM 클럭, GPU 사용률·클럭·온도, 디스크 회전 속도.
//!
//! 모두 OS가 기본으로 주는 것만 쓴다(드라이버를 깔지 않는다). 못 읽으면 `None`이고 화면은 "모름"으로 보인다.
//! - Windows
//!   - CPU 클럭: PDH `\Processor Information(_Total)\Processor Frequency`(기본 클럭) × `% Processor Performance`
//!     (작업 관리자의 "속도"와 같은 계산)
//!   - CPU 온도: PDH `\Thermal Zone Information(*)\Temperature`(켈빈, 가장 높은 영역). 없으면 sysinfo 부품 온도
//!     (WMI ACPI 온도, 보통 관리자 권한이 있어야 나온다)
//!   - GPU 사용률: PDH `\GPU Engine(*)\Utilization Percentage`를 엔진 종류마다 더해 가장 바쁜 종류 (제조사 상관없음)
//!   - GPU 이름·클럭·온도: NVIDIA 드라이버가 함께 까는 `nvml.dll`이 있을 때만 (NVIDIA)
//!   - RAM 클럭·디스크 종류와 회전 속도: 숨긴 PowerShell 한 번(`Win32_PhysicalMemory`·`Get-PhysicalDisk`).
//!     값이 거의 바뀌지 않아 `PROBE_TTL`마다만 다시 묻는다
//! - Linux: CPU 클럭은 sysinfo(`/proc/cpuinfo`), 온도는 sysinfo 부품(hwmon). GPU·RAM 클럭·회전 속도는 없다
//!
//! 읽기는 표본을 뜰 때만 한다(백그라운드 루프 없음). 첫 PDH 비율 값은 두 번째 수집부터 나온다.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// RAM·디스크 정보를 다시 묻는 간격 (디스크를 꽂고 빼도 이 안에 따라온다)
#[cfg_attr(not(windows), allow(dead_code))]
const PROBE_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuReading {
    pub name: Option<String>,
    pub util_percent: Option<f32>,
    pub clock_mhz: Option<u32>,
    pub temp_c: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub cpu_clock_mhz: Option<u32>,
    pub cpu_temp_c: Option<f32>,
    pub memory_clock_mhz: Option<u32>,
    pub gpus: Vec<GpuReading>,
    /// 드라이브 문자(대문자 한 글자) → 회전 속도. SSD는 0
    pub disk_rpm: HashMap<char, u32>,
}

/// GPU 엔진 인스턴스 이름(`pid_1234_luid_0x0_0x1_phys_0_eng_0_engtype_3D`)과 값에서 GPU별 사용률.
/// 같은 GPU(`luid`)의 같은 엔진 종류는 더하고(프로세스마다 따로 나온다), 그중 가장 바쁜 종류를 그 GPU 값으로 한다.
/// 반환은 luid 순서
#[cfg_attr(not(windows), allow(dead_code))]
pub fn gpu_engine_util(items: &[(String, f64)]) -> Vec<(String, f32)> {
    let mut by: HashMap<(String, String), f64> = HashMap::new();
    for (name, value) in items {
        let lower = name.to_lowercase();
        let Some(luid) = field(&lower, "luid_", "_phys") else { continue };
        let Some(kind) = lower.split("engtype_").nth(1) else { continue };
        *by.entry((luid.to_string(), kind.to_string())).or_default() += value.max(0.0);
    }
    let mut gpus: HashMap<String, f64> = HashMap::new();
    for ((luid, _), v) in by {
        let e = gpus.entry(luid).or_default();
        *e = e.max(v);
    }
    let mut out: Vec<(String, f32)> = gpus.into_iter().map(|(l, v)| (l, v.min(100.0) as f32)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn field<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &s[s.find(start)? + start.len()..];
    Some(&rest[..rest.find(end)?])
}

/// 기본 클럭(MHz)과 성능 %에서 지금 클럭
#[cfg_attr(not(windows), allow(dead_code))]
pub fn effective_clock(base_mhz: f64, performance_percent: f64) -> Option<u32> {
    (base_mhz > 0.0 && performance_percent > 0.0).then(|| (base_mhz * performance_percent / 100.0).round() as u32)
}

/// 온도 영역(켈빈) 중 가장 높은 것을 섭씨로. 0K·말이 안 되는 값은 뺀다
#[cfg_attr(not(windows), allow(dead_code))]
pub fn hottest_zone_c(kelvins: &[f64]) -> Option<f32> {
    kelvins
        .iter()
        .map(|k| k - 273.15)
        .filter(|c| (1.0..150.0).contains(c))
        .fold(None, |m: Option<f64>, c| Some(m.map_or(c, |m| m.max(c))))
        .map(|c| c as f32)
}

/// 부품 온도 중 CPU 것 (이름으로 고른다). CPU로 보이는 것이 없으면 ACPI 영역 중 가장 높은 것
pub fn cpu_component_temp(components: &[(String, Option<f32>)]) -> Option<f32> {
    let valid = |t: &Option<f32>| t.filter(|c| (1.0..150.0).contains(c));
    let cpu_like = |l: &str| {
        let l = l.to_lowercase();
        ["cpu", "package", "tctl", "tdie", "k10temp", "coretemp", "core "].iter().any(|k| l.contains(k))
    };
    let max = |it: &mut dyn Iterator<Item = f32>| it.fold(None, |m: Option<f32>, c| Some(m.map_or(c, |m| m.max(c))));
    max(&mut components.iter().filter(|(l, _)| cpu_like(l)).filter_map(|(_, t)| valid(t)))
        .or_else(|| max(&mut components.iter().filter(|(l, _)| l.to_lowercase().contains("acpi")).filter_map(|(_, t)| valid(t))))
}

/// 숨긴 PowerShell 조회 결과 (`PROBE_SCRIPT`)
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    #[serde(default)]
    pub mem_clock: Option<u32>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub disks: Vec<ProbeDisk>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct ProbeDisk {
    #[serde(default)]
    pub media: Option<String>,
    #[serde(default)]
    pub rpm: Option<u64>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub letters: Vec<String>,
}

/// ConvertTo-Json은 항목이 하나뿐인 배열을 값 하나로 낼 때가 있다: 둘 다 받는다
fn one_or_many<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
        None(()),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(de)? {
        Some(OneOrMany::Many(v)) => v,
        Some(OneOrMany::One(v)) => vec![v],
        Some(OneOrMany::None(())) | None => Vec::new(),
    })
}

/// RAM 설정 속도(가장 큰 것)와 물리 디스크마다 종류·회전 속도·드라이브 문자.
/// `Get-PhysicalDisk`의 `DeviceId`는 디스크 번호라 `Get-Partition -DiskNumber`로 드라이브 문자를 잇는다
#[cfg_attr(not(windows), allow(dead_code))]
pub const PROBE_SCRIPT: &str = concat!(
    "$ErrorActionPreference='SilentlyContinue';",
    "$m=(Get-CimInstance Win32_PhysicalMemory | Measure-Object -Property ConfiguredClockSpeed -Maximum).Maximum;",
    "$d=@(Get-PhysicalDisk | ForEach-Object { $n=$_.DeviceId; [pscustomobject]@{ media=[string]$_.MediaType; rpm=$_.SpindleSpeed;",
    " letters=@(Get-Partition -DiskNumber $n | Where-Object DriveLetter | ForEach-Object { [string]$_.DriveLetter }) } });",
    "[pscustomobject]@{ memClock=$m; disks=$d } | ConvertTo-Json -Depth 4 -Compress"
);

/// 조회 결과를 드라이브 문자 → 회전 속도로. SSD는 0, 회전 속도를 모르는 HDD(0·u32 최대값)는 뺀다
pub fn disk_rpm_map(probe: &Probe) -> HashMap<char, u32> {
    let mut out = HashMap::new();
    for d in &probe.disks {
        let ssd = d.media.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("ssd") || m == "4");
        let rpm = match d.rpm {
            _ if ssd => Some(0),
            Some(r) if r > 0 && r < u32::MAX as u64 => Some(r as u32),
            _ => None,
        };
        let Some(rpm) = rpm else { continue };
        for l in &d.letters {
            if let Some(c) = l.chars().next().filter(char::is_ascii_alphabetic) {
                out.insert(c.to_ascii_uppercase(), rpm);
            }
        }
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_probe(text: &str) -> Option<Probe> {
    serde_json::from_str(text.trim()).ok()
}

/// 표본기 하나에 하나. 핸들(PDH·NVML)을 들고 있다
pub struct Hardware {
    inner: imp::Inner,
    /// sysinfo 부품 (온도). 처음 한 번 목록을 만들고, 비어 있으면 다시 보지 않는다
    components: Option<sysinfo::Components>,
    probe: Option<(Instant, Probe)>,
    probe_running: bool,
    probe_rx: Option<std::sync::mpsc::Receiver<Option<Probe>>>,
}

impl Hardware {
    pub fn new() -> Self {
        Self { inner: imp::Inner::new(), components: None, probe: None, probe_running: false, probe_rx: None }
    }

    /// 지금 값. `system`은 CPU 클럭을 sysinfo로 읽는 OS에서 쓴다
    pub fn read(&mut self, system: &sysinfo::System) -> Reading {
        let live = self.inner.read(system);
        let cpu_temp_c = live.cpu_temp_c.or_else(|| self.component_temp());
        self.poll_probe();
        let probe = self.probe.as_ref().map(|(_, p)| p);
        Reading {
            cpu_clock_mhz: live.cpu_clock_mhz,
            cpu_temp_c,
            memory_clock_mhz: probe.and_then(|p| p.mem_clock).filter(|m| *m > 0),
            gpus: live.gpus,
            disk_rpm: probe.map(disk_rpm_map).unwrap_or_default(),
        }
    }

    fn component_temp(&mut self) -> Option<f32> {
        let comps = self.components.get_or_insert_with(sysinfo::Components::new_with_refreshed_list);
        if comps.list().is_empty() {
            return None;
        }
        comps.refresh(false);
        let list: Vec<(String, Option<f32>)> = comps.iter().map(|c| (c.label().to_string(), c.temperature())).collect();
        cpu_component_temp(&list)
    }

    /// RAM·디스크 조회를 별도 스레드에서 돌리고, 끝난 결과를 받는다 (표본은 기다리지 않는다)
    fn poll_probe(&mut self) {
        if let Some(rx) = &self.probe_rx {
            if let Ok(result) = rx.try_recv() {
                self.probe_rx = None;
                self.probe_running = false;
                // 실패해도 시각은 남겨 PROBE_TTL 동안 다시 띄우지 않는다
                self.probe = Some((Instant::now(), result.unwrap_or_default()));
            }
        }
        let stale = self.probe.as_ref().is_none_or(|(at, _)| at.elapsed() > PROBE_TTL);
        if stale && !self.probe_running && imp::PROBE_SUPPORTED {
            let (tx, rx) = std::sync::mpsc::channel();
            self.probe_running = true;
            self.probe_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(imp::run_probe());
            });
        }
    }
}

/// OS별 지금 값 (조회 결과 제외)
#[derive(Default)]
struct Live {
    cpu_clock_mhz: Option<u32>,
    cpu_temp_c: Option<f32>,
    gpus: Vec<GpuReading>,
}

#[cfg(windows)]
mod imp {
    use super::{GpuReading, Live, Probe};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue,
        PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA,
    };

    pub const PROBE_SUPPORTED: bool = true;
    /// 100%를 넘는 값도 그대로 (부스트 중 `% Processor Performance`는 100을 넘는다)
    const PDH_FMT_NOCAP100: u32 = 0x0000_8000;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Inner {
        query: isize,
        freq: isize,
        perf: isize,
        thermal: isize,
        gpu: isize,
        nvml: Option<super::nvml::Nvml>,
    }

    // PDH·NVML 핸들은 스레드에 묶이지 않는다. 접근은 SystemMonitor의 Mutex가 한 번에 하나로 막는다
    unsafe impl Send for Inner {}

    impl Inner {
        pub fn new() -> Self {
            let mut s = Self { query: 0, freq: 0, perf: 0, thermal: 0, gpu: 0, nvml: super::nvml::Nvml::load() };
            unsafe {
                if PdhOpenQueryW(null(), 0, &mut s.query) != 0 {
                    s.query = 0;
                    return s;
                }
                // 카운터마다 따로 더한다: 없는 카운터(가상 머신의 온도 영역 등)가 있어도 나머지는 쓴다
                let add = |path: &str, out: &mut isize| {
                    if PdhAddEnglishCounterW(s.query, wide(path).as_ptr(), 0, out) != 0 {
                        *out = 0;
                    }
                };
                let (mut freq, mut perf, mut thermal, mut gpu) = (0, 0, 0, 0);
                add(r"\Processor Information(_Total)\Processor Frequency", &mut freq);
                add(r"\Processor Information(_Total)\% Processor Performance", &mut perf);
                add(r"\Thermal Zone Information(*)\Temperature", &mut thermal);
                add(r"\GPU Engine(*)\Utilization Percentage", &mut gpu);
                (s.freq, s.perf, s.thermal, s.gpu) = (freq, perf, thermal, gpu);
            }
            s
        }

        pub fn read(&mut self, _system: &sysinfo::System) -> Live {
            let mut live = Live::default();
            if self.query != 0 && unsafe { PdhCollectQueryData(self.query) } == 0 {
                unsafe {
                    if let (Some(base), Some(perf)) = (single(self.freq, 0), single(self.perf, PDH_FMT_NOCAP100)) {
                        live.cpu_clock_mhz = super::effective_clock(base, perf);
                    }
                    let zones: Vec<f64> = array(self.thermal).into_iter().map(|(_, v)| v).collect();
                    live.cpu_temp_c = super::hottest_zone_c(&zones);
                    let util = super::gpu_engine_util(&array(self.gpu));
                    live.gpus = util.into_iter().map(|(_, u)| GpuReading { util_percent: Some(u), ..Default::default() }).collect();
                }
            }
            // NVIDIA가 있으면 NVML 카드만 쓴다: PDH GPU(luid)와 NVML 카드를 짝지을 길이 없어 섞지 않는다
            if let Some(nv) = &self.nvml {
                let cards = nv.read();
                if !cards.is_empty() {
                    live.gpus = cards;
                }
            }
            live
        }
    }

    unsafe fn single(counter: isize, extra: u32) -> Option<f64> {
        if counter == 0 {
            return None;
        }
        unsafe {
            let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
            if PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE | extra, null_mut(), &mut v) != 0 || v.CStatus > 1 {
                return None;
            }
            Some(v.Anonymous.doubleValue)
        }
    }

    /// 와일드카드 카운터의 (인스턴스 이름, 값)들
    unsafe fn array(counter: isize) -> Vec<(String, f64)> {
        if counter == 0 {
            return Vec::new();
        }
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut size, &mut count, null_mut());
            if r != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            // 이름 문자열이 항목 뒤에 붙어 오므로 바이트 크기만큼, 항목 정렬에 맞춰 잡는다
            let n = (size as usize).div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![std::mem::zeroed(); n];
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut size, &mut count, buf.as_mut_ptr()) != 0 {
                return Vec::new();
            }
            buf.iter()
                .take(count as usize)
                .filter(|it| it.FmtValue.CStatus <= 1 && !it.szName.is_null())
                .map(|it| {
                    let mut len = 0;
                    while *it.szName.add(len) != 0 {
                        len += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                    (name, it.FmtValue.Anonymous.doubleValue)
                })
                .collect()
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            if self.query != 0 {
                unsafe { PdhCloseQuery(self.query) };
            }
        }
    }

    /// 숨긴 PowerShell로 RAM·디스크를 묻는다 (창을 띄우지 않는다)
    pub fn run_probe() -> Option<Probe> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", super::PROBE_SCRIPT])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        out.status.success().then(|| super::parse_probe(&String::from_utf8_lossy(&out.stdout))).flatten()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Live, Probe};

    pub const PROBE_SUPPORTED: bool = false;

    pub struct Inner;

    impl Inner {
        pub fn new() -> Self {
            Self
        }

        /// CPU 클럭은 sysinfo 코어 값의 평균 (`refresh_cpu_frequency` 뒤). 온도는 부품 온도로 (호출한 쪽)
        pub fn read(&mut self, system: &sysinfo::System) -> Live {
            let freqs: Vec<u64> = system.cpus().iter().map(|c| c.frequency()).filter(|f| *f > 0).collect();
            let cpu_clock_mhz = (!freqs.is_empty()).then(|| (freqs.iter().sum::<u64>() / freqs.len() as u64) as u32);
            Live { cpu_clock_mhz, ..Default::default() }
        }
    }

    pub fn run_probe() -> Option<Probe> {
        None
    }
}

/// NVIDIA 관리 라이브러리(NVML)를 드라이버의 `nvml.dll`에서 직접 부른다 (크레이트를 더하지 않는다)
#[cfg(windows)]
mod nvml {
    use super::GpuReading;
    use std::ffi::c_void;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

    type Device = *mut c_void;
    #[repr(C)]
    struct Utilization {
        gpu: u32,
        memory: u32,
    }
    type CountFn = unsafe extern "C" fn(*mut u32) -> i32;
    type HandleFn = unsafe extern "C" fn(u32, *mut Device) -> i32;
    type NameFn = unsafe extern "C" fn(Device, *mut u8, u32) -> i32;
    type TempFn = unsafe extern "C" fn(Device, i32, *mut u32) -> i32;
    type ClockFn = unsafe extern "C" fn(Device, i32, *mut u32) -> i32;
    type UtilFn = unsafe extern "C" fn(Device, *mut Utilization) -> i32;
    const NVML_TEMPERATURE_GPU: i32 = 0;
    const NVML_CLOCK_GRAPHICS: i32 = 0;

    pub struct Nvml {
        count: CountFn,
        handle: HandleFn,
        name: NameFn,
        temp: TempFn,
        clock: ClockFn,
        util: UtilFn,
    }

    impl Nvml {
        /// 드라이버가 System32에 까는 nvml.dll만 본다 (다른 폴더의 같은 이름 DLL을 올리지 않게)
        pub fn load() -> Option<Self> {
            unsafe {
                let path: Vec<u16> = "nvml.dll".encode_utf16().chain(std::iter::once(0)).collect();
                let lib = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
                if lib.is_null() {
                    return None;
                }
                let sym = |name: &[u8]| GetProcAddress(lib, name.as_ptr());
                let init = std::mem::transmute::<unsafe extern "system" fn() -> isize, unsafe extern "C" fn() -> i32>(sym(b"nvmlInit_v2\0")?);
                let me = Self {
                    count: std::mem::transmute::<unsafe extern "system" fn() -> isize, CountFn>(sym(b"nvmlDeviceGetCount_v2\0")?),
                    handle: std::mem::transmute::<unsafe extern "system" fn() -> isize, HandleFn>(sym(b"nvmlDeviceGetHandleByIndex_v2\0")?),
                    name: std::mem::transmute::<unsafe extern "system" fn() -> isize, NameFn>(sym(b"nvmlDeviceGetName\0")?),
                    temp: std::mem::transmute::<unsafe extern "system" fn() -> isize, TempFn>(sym(b"nvmlDeviceGetTemperature\0")?),
                    clock: std::mem::transmute::<unsafe extern "system" fn() -> isize, ClockFn>(sym(b"nvmlDeviceGetClockInfo\0")?),
                    util: std::mem::transmute::<unsafe extern "system" fn() -> isize, UtilFn>(sym(b"nvmlDeviceGetUtilizationRates\0")?),
                };
                // 라이브러리는 데몬이 끝날 때까지 둔다 (함수 포인터가 남아 있다)
                (init() == 0).then_some(me)
            }
        }

        pub fn read(&self) -> Vec<GpuReading> {
            unsafe {
                let mut n = 0u32;
                if (self.count)(&mut n) != 0 {
                    return Vec::new();
                }
                (0..n.min(8))
                    .filter_map(|i| {
                        let mut dev: Device = std::ptr::null_mut();
                        if (self.handle)(i, &mut dev) != 0 {
                            return None;
                        }
                        let mut buf = [0u8; 96];
                        let name = ((self.name)(dev, buf.as_mut_ptr(), buf.len() as u32) == 0).then(|| {
                            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                            String::from_utf8_lossy(&buf[..end]).trim().to_string()
                        });
                        let mut t = 0u32;
                        let temp_c = ((self.temp)(dev, NVML_TEMPERATURE_GPU, &mut t) == 0).then_some(t as f32);
                        let mut c = 0u32;
                        let clock_mhz = ((self.clock)(dev, NVML_CLOCK_GRAPHICS, &mut c) == 0).then_some(c);
                        let mut u = Utilization { gpu: 0, memory: 0 };
                        let util_percent = ((self.util)(dev, &mut u) == 0).then_some(u.gpu as f32);
                        Some(GpuReading { name: name.filter(|n| !n.is_empty()), util_percent, clock_mhz, temp_c })
                    })
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_util_sums_processes_per_engine_and_takes_the_busiest_engine() {
        let items = vec![
            ("pid_10_luid_0x0_0x1A_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_20_luid_0x0_0x1A_phys_0_eng_0_engtype_3D".to_string(), 25.0),
            ("pid_20_luid_0x0_0x1A_phys_0_eng_5_engtype_VideoDecode".to_string(), 40.0),
            ("pid_30_luid_0x0_0x2B_phys_0_eng_1_engtype_Copy".to_string(), 7.5),
            ("junk".to_string(), 99.0),
        ];
        assert_eq!(gpu_engine_util(&items), vec![("0x0_0x1a".to_string(), 55.0), ("0x0_0x2b".to_string(), 7.5)]);
        // 합이 100을 넘으면 100
        let busy = vec![("pid_1_luid_0x1_phys_0_eng_0_engtype_3D".to_string(), 80.0), ("pid_2_luid_0x1_phys_0_eng_0_engtype_3D".to_string(), 60.0)];
        assert_eq!(gpu_engine_util(&busy)[0].1, 100.0);
    }

    #[test]
    fn clock_is_base_times_performance_and_zones_are_kelvin() {
        assert_eq!(effective_clock(3700.0, 128.5), Some(4755));
        assert_eq!(effective_clock(0.0, 100.0), None);
        assert_eq!(hottest_zone_c(&[300.15, 330.15, 0.0]), Some(57.0));
        assert_eq!(hottest_zone_c(&[]), None);
    }

    #[test]
    fn cpu_temp_prefers_cpu_labels_then_acpi() {
        let list = vec![("nvme Composite".to_string(), Some(45.0)), ("k10temp Tctl".to_string(), Some(61.5)), ("acpitz".to_string(), Some(70.0))];
        assert_eq!(cpu_component_temp(&list), Some(61.5));
        let acpi = vec![("ACPI\\ThermalZone\\TZ00_0".to_string(), Some(38.0)), ("gpu".to_string(), Some(80.0))];
        assert_eq!(cpu_component_temp(&acpi), Some(38.0));
        assert_eq!(cpu_component_temp(&[("cpu".to_string(), None)]), None);
    }

    #[test]
    fn probe_maps_drive_letters_to_rpm() {
        let text = r#"{"memClock":6000,"disks":[{"media":"SSD","rpm":0,"letters":["C"]},{"media":"HDD","rpm":7200,"letters":["D","e"]},{"media":"Unspecified","rpm":4294967295,"letters":["F"]},{"media":"HDD","rpm":null,"letters":[]}]}"#;
        let probe = parse_probe(text).unwrap();
        assert_eq!(probe.mem_clock, Some(6000));
        let map = disk_rpm_map(&probe);
        assert_eq!(map.get(&'C'), Some(&0));
        assert_eq!(map.get(&'D'), Some(&7200));
        assert_eq!(map.get(&'E'), Some(&7200));
        assert_eq!(map.get(&'F'), None);
        // 항목이 하나라 ConvertTo-Json이 배열 대신 값 하나를 내도 받는다
        let one = parse_probe(r#"{"memClock":null,"disks":{"media":"HDD","rpm":5400,"letters":"G"}}"#).unwrap();
        assert_eq!(one.mem_clock, None);
        assert_eq!(disk_rpm_map(&one).get(&'G'), Some(&5400));
    }
}
