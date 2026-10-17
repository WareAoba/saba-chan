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

fn script_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if entry.file_name() != "__pycache__" {
                script_files(&path, out);
            }
        } else if kind.is_file() {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if SCRIPT_SUFFIXES.contains(&ext.as_str()) {
                out.push(path);
            }
        }
    }
}

/// 구성요소 폴더의 스크립트를 검사한다. 하나라도 찾으면 `malware_detected`
pub fn scan_component(dir: &Path, scanner: &dyn Scanner, cache: &mut VerdictCache) -> Result<(), Blocked> {
    let mut files = Vec::new();
    script_files(dir, &mut files);
    files.sort();
    let mut result = Ok(());
    for file in files {
        let Ok(bytes) = std::fs::read(&file) else { continue };
        let sha = sha256_of(&bytes);
        let detected = match cache.get(&sha) {
            Some(d) => d,
            None => match scanner.scan_buffer(&file.to_string_lossy(), &bytes) {
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
                    tracing::info!("[Sandbox] no AMSI provider; skipping script scan");
                    return result;
                }
            },
        };
        if detected && result.is_ok() {
            result = Err(Blocked { code: "malware_detected", file: Some(file.to_string_lossy().into_owned()), detail: None });
        }
    }
    result
}

/// 서버 실행 파일 검사 (처음 실행 전, 바뀐 뒤)
pub fn scan_executable(path: &Path, scanner: &dyn Scanner, cache: &mut VerdictCache) -> Result<(), Blocked> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    let sha = sha256_of(&bytes);
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
            Verdict::Unavailable => return Ok(()),
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
    use super::{amsi_detected, Verdict};
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
        let Ok(len) = u32::try_from(content.len()) else { return Verdict::Unavailable };
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
            if hr < 0 {
                // E_NOTIMPL 등: 공급자가 없다
                return Verdict::Unavailable;
            }
            if amsi_detected(result) {
                Verdict::Detected("AMSI".into())
            } else {
                Verdict::Clean
            }
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
}
