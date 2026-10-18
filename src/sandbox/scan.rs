//! 악성 코드 검사: 스크립트는 AMSI, 실행 파일은 Defender `MpCmdRun`
//!
//! 검사기는 trait 뒤에 둔다 (테스트는 가짜 검사기를 쓴다). 결과는 `VerdictCache`에 sha256으로 남긴다.

use super::{sha256_of, Blocked, VerdictCache};
use std::path::{Path, PathBuf};

/// 검사 결과 한 건
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Clean,
    Detected(String),
    /// 검사기가 없다 (다른 백신, AMSI 공급자 없음 등). 건너뛴다
    Unavailable,
    /// 검사기는 있는데 이 파일 검사가 실패했다. 그 파일을 막고 나머지는 계속 본다
    Failed(String),
}

/// 검사기가 없다는 경고는 데몬 실행마다 한 번만
fn warn_once(flag: &std::sync::atomic::AtomicBool, message: &str) {
    if !flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!("[Sandbox] {}", message);
    }
}

static AMSI_MISSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DEFENDER_MISSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn scan_failed(file: &Path, detail: impl Into<String>) -> Blocked {
    Blocked { code: "scan_failed", file: Some(file.to_string_lossy().into_owned()), detail: Some(detail.into()) }
}

pub trait Scanner {
    /// 스크립트 내용 (`name`은 백신 기록용 파일 이름)
    fn scan_buffer(&self, name: &str, content: &[u8]) -> Verdict;
    /// 실행 파일
    fn scan_file(&self, path: &Path) -> Verdict;
}

/// AMSI 결과 값: 32768(`AMSI_RESULT_DETECTED`) 이상이면 찾은 것
#[cfg_attr(not(windows), allow(dead_code))]
pub fn amsi_detected(result: u32) -> bool {
    result >= 32768
}

/// `AmsiScanBuffer` 결과: `E_NOTIMPL`·`ERROR_NOT_READY`(백신이 꺼져 있음)는 공급자가 없는 것, 그 밖의 실패는 이 파일 검사 실패
#[cfg_attr(not(windows), allow(dead_code))]
pub fn amsi_verdict(hr: i32, result: u32) -> Verdict {
    const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;
    const NOT_READY: i32 = 0x8007_0015_u32 as i32;
    if hr == E_NOTIMPL || hr == NOT_READY {
        Verdict::Unavailable
    } else if hr < 0 {
        Verdict::Failed(format!("AMSI error 0x{:08x}", hr as u32))
    } else if amsi_detected(result) {
        Verdict::Detected("AMSI".into())
    } else {
        Verdict::Clean
    }
}

/// `MpCmdRun -Scan` 종료 코드: 2면 위협을 찾은 것
#[cfg_attr(not(windows), allow(dead_code))]
pub fn mpcmdrun_verdict(code: Option<i32>) -> Verdict {
    match code {
        Some(0) => Verdict::Clean,
        Some(2) => Verdict::Detected("Windows Defender".into()),
        _ => Verdict::Unavailable,
    }
}

/// AMSI로 볼 파일 (Python이 읽거나 셸이 실행할 수 있는 글)
const SCRIPT_SUFFIXES: &[&str] = &["py", "pyw", "pth", "ps1", "psm1", "bat", "cmd", "js", "vbs", "wsf"];
/// Defender로 볼 파일 (Python이 import로 불러오는 네이티브 코드와 압축 묶음)
const NATIVE_SUFFIXES: &[&str] = &["pyd", "dll", "so", "exe", "zip", "egg", "whl"];
/// 원본 없이 실행될 수 있어 검사할 수 없는 파일. `__pycache__` 밖의 `.pyc`는 막는다
/// (`__pycache__` 안의 것은 `PYTHONPYCACHEPREFIX` 때문에 읽히지 않는다)
const UNSCANNABLE_SUFFIXES: &[&str] = &["pyc", "pyo"];

#[derive(Default)]
struct ComponentFiles {
    scripts: Vec<PathBuf>,
    native: Vec<PathBuf>,
    unscannable: Vec<PathBuf>,
}

fn component_files(dir: &Path, out: &mut ComponentFiles) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if entry.file_name() != "__pycache__" {
                component_files(&path, out);
            }
        } else if kind.is_file() {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if SCRIPT_SUFFIXES.contains(&ext.as_str()) {
                out.scripts.push(path);
            } else if NATIVE_SUFFIXES.contains(&ext.as_str()) {
                out.native.push(path);
            } else if UNSCANNABLE_SUFFIXES.contains(&ext.as_str()) {
                out.unscannable.push(path);
            }
        }
    }
}

/// 구성요소 폴더를 검사한다. 스크립트는 AMSI, 네이티브 코드는 Defender로 보고 하나라도 찾으면
/// `malware_detected`. 원본 없는 바이트코드가 있으면 `unscannable_file`, 읽지 못하거나 검사가 실패한 파일이 있으면 `scan_failed`
pub fn scan_component(dir: &Path, scanner: &dyn Scanner, cache: &mut VerdictCache) -> Result<(), Blocked> {
    let mut files = ComponentFiles::default();
    component_files(dir, &mut files);
    files.unscannable.sort();
    if let Some(file) = files.unscannable.first() {
        return Err(Blocked { code: "unscannable_file", file: Some(file.to_string_lossy().into_owned()), detail: None });
    }
    files.scripts.sort();
    files.native.sort();
    let mut result = Ok(());
    for file in files.scripts {
        // 바뀌지 않은 파일은 다시 읽지 않고 저장한 결과를 쓴다
        let cached = saba_chan_updater_lib::integrity::cached_sha256(&file).ok().and_then(|sha| cache.get(&sha));
        let detected = match cached {
            Some(d) => d,
            None => {
                let bytes = match std::fs::read(&file) {
                    Ok(b) => b,
                    Err(e) => {
                        if result.is_ok() {
                            result = Err(scan_failed(&file, e.to_string()));
                        }
                        continue;
                    }
                };
                // 빈 파일(빈 __init__.py 등)은 검사할 내용이 없다. AMSI는 길이 0을 인자 오류로 돌려준다
                if bytes.is_empty() {
                    continue;
                }
                let sha = sha256_of(&bytes);
                match scanner.scan_buffer(&file.to_string_lossy(), &bytes) {
                    Verdict::Clean => {
                        cache.put(sha, false);
                        false
                    }
                    Verdict::Detected(by) => {
                        tracing::warn!("[Sandbox] {} flagged by {}", file.display(), by);
                        cache.put(sha, true);
                        true
                    }
                    Verdict::Unavailable => {
                        warn_once(&AMSI_MISSING, "no AMSI provider; scripts of unverified components are not scanned");
                        break;
                    }
                    Verdict::Failed(e) => {
                        tracing::warn!("[Sandbox] could not scan {}: {}", file.display(), e);
                        if result.is_ok() {
                            result = Err(scan_failed(&file, e));
                        }
                        continue;
                    }
                }
            }
        };
        if detected && result.is_ok() {
            result = Err(Blocked { code: "malware_detected", file: Some(file.to_string_lossy().into_owned()), detail: None });
        }
    }
    for file in files.native {
        if result.is_err() {
            break;
        }
        result = scan_executable(&file, scanner, cache);
    }
    result
}

/// 서버 실행 파일 검사 (처음 실행 전, 바뀐 뒤)
pub fn scan_executable(path: &Path, scanner: &dyn Scanner, cache: &mut VerdictCache) -> Result<(), Blocked> {
    // 큰 서버 파일을 시작마다 해시하지 않게 크기·수정 시각이 같으면 지난 값을 쓴다
    let sha = saba_chan_updater_lib::integrity::cached_sha256(path).map_err(|e| scan_failed(path, e))?;
    let detected = match cache.get(&sha) {
        Some(d) => d,
        None => match scanner.scan_file(path) {
            Verdict::Clean => {
                cache.put(sha, false);
                false
            }
            Verdict::Detected(by) => {
                tracing::warn!("[Sandbox] {} flagged by {}", path.display(), by);
                cache.put(sha, true);
                true
            }
            Verdict::Unavailable => {
                warn_once(&DEFENDER_MISSING, "Windows Defender (MpCmdRun) is unavailable; executables of unverified components are not scanned");
                return Ok(());
            }
            Verdict::Failed(e) => return Err(scan_failed(path, e)),
        },
    };
    if detected {
        return Err(Blocked { code: "malware_detected", file: Some(path.to_string_lossy().into_owned()), detail: None });
    }
    Ok(())
}

/// 이 PC의 검사기 (Windows: AMSI + MpCmdRun, 그 밖: 없음)
pub struct SystemScanner;

impl Scanner for SystemScanner {
    fn scan_buffer(&self, name: &str, content: &[u8]) -> Verdict {
        #[cfg(windows)]
        {
            amsi::scan(name, content)
        }
        #[cfg(not(windows))]
        {
            let _ = (name, content);
            Verdict::Unavailable
        }
    }

    fn scan_file(&self, path: &Path) -> Verdict {
        #[cfg(windows)]
        {
            let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
            let tool = program_files.join("Windows Defender").join("MpCmdRun.exe");
            if !tool.is_file() {
                return Verdict::Unavailable;
            }
            let mut cmd = std::process::Command::new(tool);
            cmd.args(["-Scan", "-ScanType", "3", "-File"]).arg(path).arg("-DisableRemediation");
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            mpcmdrun_verdict(cmd.output().ok().and_then(|o| o.status.code()))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Verdict::Unavailable
        }
    }
}

/// AMSI (amsi.dll을 실행 중에 불러온다: 없는 PC에서도 데몬이 뜬다)
#[cfg(windows)]
mod amsi {
    use super::{amsi_verdict, Verdict};
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    type HResult = i32;
    type Initialize = unsafe extern "system" fn(*const u16, *mut *mut c_void) -> HResult;
    type Uninitialize = unsafe extern "system" fn(*mut c_void);
    type OpenSession = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
    type CloseSession = unsafe extern "system" fn(*mut c_void, *mut c_void);
    type ScanBuffer = unsafe extern "system" fn(*mut c_void, *const c_void, u32, *const u16, *mut c_void, *mut u32) -> HResult;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn scan(name: &str, content: &[u8]) -> Verdict {
        use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
        let Ok(len) = u32::try_from(content.len()) else { return Verdict::Failed("file too large for AMSI".into()) };
        unsafe {
            let lib = LoadLibraryW(wide("amsi.dll").as_ptr());
            if lib.is_null() {
                return Verdict::Unavailable;
            }
            let get = |sym: &[u8]| GetProcAddress(lib, sym.as_ptr() as *const i8);
            let (init, uninit, open, close, scan) = (
                get(b"AmsiInitialize\0"),
                get(b"AmsiUninitialize\0"),
                get(b"AmsiOpenSession\0"),
                get(b"AmsiCloseSession\0"),
                get(b"AmsiScanBuffer\0"),
            );
            if [init, uninit, open, close, scan].iter().any(|p| p.is_null()) {
                return Verdict::Unavailable;
            }
            let init: Initialize = std::mem::transmute(init);
            let uninit: Uninitialize = std::mem::transmute(uninit);
            let open: OpenSession = std::mem::transmute(open);
            let close: CloseSession = std::mem::transmute(close);
            let scan: ScanBuffer = std::mem::transmute(scan);

            let mut ctx: *mut c_void = std::ptr::null_mut();
            if init(wide("saba-chan").as_ptr(), &mut ctx) < 0 || ctx.is_null() {
                return Verdict::Unavailable;
            }
            let mut session: *mut c_void = std::ptr::null_mut();
            let _ = open(ctx, &mut session);
            let mut result: u32 = 0;
            let hr = scan(ctx, content.as_ptr() as *const c_void, len, wide(name).as_ptr(), session, &mut result);
            if !session.is_null() {
                close(ctx, session);
            }
            uninit(ctx);
            amsi_verdict(hr, result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// "EICAR"가 든 내용만 찾는 가짜 검사기. 몇 번 불렸는지 센다
    struct Fake {
        calls: Cell<usize>,
        available: bool,
    }

    impl Scanner for Fake {
        fn scan_buffer(&self, _name: &str, content: &[u8]) -> Verdict {
            self.calls.set(self.calls.get() + 1);
            if !self.available {
                return Verdict::Unavailable;
            }
            if content.windows(5).any(|w| w == b"EICAR") {
                Verdict::Detected("fake".into())
            } else {
                Verdict::Clean
            }
        }
        fn scan_file(&self, path: &Path) -> Verdict {
            self.scan_buffer("", &std::fs::read(path).unwrap_or_default())
        }
    }

    #[test]
    fn result_codes_map_to_verdicts() {
        assert!(!amsi_detected(1) && amsi_detected(32768) && amsi_detected(40000));
        assert_eq!(mpcmdrun_verdict(Some(0)), Verdict::Clean);
        assert!(matches!(mpcmdrun_verdict(Some(2)), Verdict::Detected(_)));
        assert_eq!(mpcmdrun_verdict(Some(1)), Verdict::Unavailable);
        assert_eq!(mpcmdrun_verdict(None), Verdict::Unavailable);
        assert_eq!(amsi_verdict(0, 1), Verdict::Clean);
        assert!(matches!(amsi_verdict(0, 32768), Verdict::Detected(_)));
        assert_eq!(amsi_verdict(0x8000_4001_u32 as i32, 0), Verdict::Unavailable);
        assert_eq!(amsi_verdict(0x8007_0015_u32 as i32, 0), Verdict::Unavailable);
        assert!(matches!(amsi_verdict(0x8007_0005_u32 as i32, 0), Verdict::Failed(_)));
    }

    /// 한 파일 검사가 실패해도 공급자가 없는 것으로 보지 않는다: 그 파일을 막고 나머지도 본다
    #[test]
    fn a_failed_file_blocks_and_the_rest_are_still_scanned() {
        struct FailsOn(Cell<usize>);
        impl Scanner for FailsOn {
            fn scan_buffer(&self, name: &str, _content: &[u8]) -> Verdict {
                self.0.set(self.0.get() + 1);
                if name.ends_with("a.py") { Verdict::Failed("x".into()) } else { Verdict::Clean }
            }
            fn scan_file(&self, _path: &Path) -> Verdict {
                Verdict::Clean
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a.py", "b.py", "c.py"] {
            std::fs::write(tmp.path().join(f), f.as_bytes()).unwrap();
        }
        let scanner = FailsOn(Cell::new(0));
        let blocked = scan_component(tmp.path(), &scanner, &mut VerdictCache::in_memory()).unwrap_err();
        assert_eq!(blocked.code, "scan_failed");
        assert!(blocked.file.unwrap().ends_with("a.py"));
        assert_eq!(scanner.0.get(), 3);
    }

    #[test]
    fn empty_scripts_are_not_sent_to_the_scanner() {
        struct FailsOnEmpty;
        impl Scanner for FailsOnEmpty {
            fn scan_buffer(&self, _name: &str, content: &[u8]) -> Verdict {
                if content.is_empty() { Verdict::Failed("E_INVALIDARG".into()) } else { Verdict::Clean }
            }
            fn scan_file(&self, _path: &Path) -> Verdict {
                Verdict::Clean
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("__init__.py"), b"").unwrap();
        std::fs::write(tmp.path().join("lifecycle.py"), b"print(1)").unwrap();
        assert!(scan_component(tmp.path(), &FailsOnEmpty, &mut VerdictCache::in_memory()).is_ok());
    }

    /// 읽지 못하는 실행 파일은 통과시키지 않는다
    #[test]
    fn an_unreadable_executable_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake { calls: Cell::new(0), available: true };
        let blocked = scan_executable(&tmp.path().join("missing.exe"), &fake, &mut VerdictCache::in_memory()).unwrap_err();
        assert_eq!(blocked.code, "scan_failed");
    }

    #[test]
    fn flagged_scripts_block_and_unchanged_files_are_not_rescanned() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("lifecycle.py"), b"print('hi')").unwrap();
        std::fs::create_dir_all(tmp.path().join("__pycache__")).unwrap();
        std::fs::write(tmp.path().join("__pycache__").join("x.py"), b"EICAR").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"EICAR").unwrap();
        let fake = Fake { calls: Cell::new(0), available: true };
        let mut cache = VerdictCache::in_memory();
        assert_eq!(scan_component(tmp.path(), &fake, &mut cache), Ok(()));
        assert_eq!(scan_component(tmp.path(), &fake, &mut cache), Ok(()));
        assert_eq!(fake.calls.get(), 1);

        std::fs::write(tmp.path().join("bad.py"), b"x = 'EICAR'").unwrap();
        let blocked = scan_component(tmp.path(), &fake, &mut cache).unwrap_err();
        assert_eq!(blocked.code, "malware_detected");
        assert!(blocked.file.unwrap().ends_with("bad.py"));
        // 찾은 결과도 저장된다
        assert!(scan_component(tmp.path(), &fake, &mut cache).is_err());
        assert_eq!(fake.calls.get(), 2);

        // 검사기가 없으면 막지 않는다
        let none = Fake { calls: Cell::new(0), available: false };
        assert_eq!(scan_component(tmp.path(), &none, &mut VerdictCache::in_memory()), Ok(()));
        let exe = tmp.path().join("server.exe");
        std::fs::write(&exe, b"EICAR").unwrap();
        assert!(scan_executable(&exe, &fake, &mut cache).is_err());
        assert_eq!(scan_executable(&exe, &none, &mut VerdictCache::in_memory()), Ok(()));
    }

    #[test]
    fn native_code_is_scanned_and_sourceless_bytecode_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("lifecycle.py"), b"import helper").unwrap();
        std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
        std::fs::write(tmp.path().join("lib").join("helper.pyd"), b"EICAR").unwrap();
        let fake = Fake { calls: Cell::new(0), available: true };
        let blocked = scan_component(tmp.path(), &fake, &mut VerdictCache::in_memory()).unwrap_err();
        assert_eq!(blocked.code, "malware_detected");
        assert!(blocked.file.unwrap().ends_with("helper.pyd"));

        std::fs::write(tmp.path().join("lib").join("helper.pyd"), b"clean").unwrap();
        assert_eq!(scan_component(tmp.path(), &fake, &mut VerdictCache::in_memory()), Ok(()));
        // __pycache__ 밖의 .pyc는 검사할 수 없어 막는다. 안의 것은 읽히지 않으므로 둔다
        std::fs::create_dir_all(tmp.path().join("__pycache__")).unwrap();
        std::fs::write(tmp.path().join("__pycache__").join("lifecycle.cpython-312.pyc"), b"x").unwrap();
        assert_eq!(scan_component(tmp.path(), &fake, &mut VerdictCache::in_memory()), Ok(()));
        std::fs::write(tmp.path().join("helper.pyc"), b"x").unwrap();
        let blocked = scan_component(tmp.path(), &fake, &mut VerdictCache::in_memory()).unwrap_err();
        assert_eq!(blocked.code, "unscannable_file");
    }
}
