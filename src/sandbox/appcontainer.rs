//! 확인되지 않은 모듈·익스텐션의 Python을 AppContainer 안에서 돌린다 (Windows)
//!
//! - 프로필은 구성요소마다 따로 둔다(`profile_for`). 폴더 권한이 프로필 SID에 남으므로, 한 구성요소가 받은
//!   권한을 다른 구성요소가 쓰지 못하게 하려면 SID가 달라야 한다. 능력은 `internetClient`(밖으로 내려받기)만 준다.
//!   그래서 안에서는 포트를 열 수 없고, LAN과 루프백(데몬 API)에도 닿지 않는다.
//! - 폴더 권한: Python 설치·구성요소·공용 유틸리티는 읽기, 그 인스턴스 폴더·바이트코드 캐시·임시 폴더는
//!   읽기/쓰기. 데이터 폴더 자체(토큰·`config.json`)와 그 위, 드라이브 루트, 사용자 폴더 루트에는 주지 않는다.
//! - 프로세스는 Job Object에 넣는다. 시간 초과면 Job째 끝내고, 데몬 쪽 핸들이 닫혀도 남은 것은 끝난다.
//!
//! 준비가 하나라도 실패하면 격리 없이 돌리지 않고 `sandbox_unavailable`을 돌려준다
//! (사용자가 그 구성요소를 격리 없이 돌리기로 한 경우만 예외, `super::consent`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// AppContainer 프로필 이름 접두어. 예전의 공용 프로필 `saba-chan.untrusted`는 더 쓰지 않는다
const PROFILE_PREFIX: &str = "saba-chan.untrusted.";

/// 구성요소 키(`module-<이름>`·`ext-<이름>`)를 짧게 줄인 꼬리표 (프로필 이름 64자 한도, 폴더 이름에도 쓴다)
pub fn component_tag(key: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(key.as_bytes()))[..16].to_string()
}

/// 그 구성요소의 AppContainer 프로필 이름
pub fn profile_for(key: &str) -> String {
    format!("{PROFILE_PREFIX}{}", component_tag(key))
}

/// 폴더에 줄 권한
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// 그 폴더의 목록만 (안의 파일은 읽지 못한다. 상속하지 않는다)
    List,
    /// 읽기·실행 (하위로 상속)
    Read,
    /// 읽기·쓰기·지우기 (하위로 상속)
    Write,
}

/// 한 번의 실행에 필요한 폴더 권한
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Grants {
    pub entries: Vec<(PathBuf, Access)>,
    /// 주지 못해도 실행은 계속하는 폴더 (찾아 둔 런타임처럼 없어도 되는 것)
    pub optional: Vec<(PathBuf, Access)>,
}

/// 권한을 줄 수 없는 위치인가: 데이터 폴더와 그 위, 드라이브 루트, 사용자 폴더와 그 위
fn forbidden(path: &Path, data_dir: &Path, home: Option<&Path>) -> bool {
    root_or_home(path, home) || data_dir.starts_with(path)
}

/// 드라이브 루트이거나 사용자 폴더와 그 위인가 (목록 권한도 주지 않는다)
fn root_or_home(path: &Path, home: Option<&Path>) -> bool {
    use std::path::Component;
    let normal = path.components().filter(|c| matches!(c, Component::Normal(_))).count();
    normal == 0 || home.is_some_and(|h| h.starts_with(path))
}

/// 경로 자신이나 그 위 폴더 중 하나가 심볼릭 링크·정션인가
fn through_link(path: &Path) -> bool {
    path.ancestors().any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
}

impl Grants {
    /// 넣으려는 폴더를 실제 경로로 바꾸고, 줄 수 없는 위치는 버린다
    pub fn push(&mut self, path: &Path, access: Access, data_dir: &Path, home: Option<&Path>) {
        let Ok(real) = std::fs::canonicalize(path) else { return };
        if !real.is_dir() {
            return;
        }
        // List는 목록만이라 데이터 폴더 자신에도 줄 수 있다 (안의 파일은 상속받지 않는다).
        // 쓰기는 링크를 거친 경로에 주지 않는다: 격리된 구성요소가 쓰기를 받은 폴더 안에 정션을 만들어 두면,
        // 다음 실행 때 그 경로의 쓰기 권한과 낮은 수준 표시가 정션이 가리키는 다른 폴더로 간다.
        // 읽기는 정션 아래 Python 설치(scoop 등)도 쓰므로 막지 않는다
        let blocked = match access {
            Access::List => root_or_home(&real, home),
            Access::Read => forbidden(&real, data_dir, home),
            Access::Write => forbidden(&real, data_dir, home) || through_link(path),
        };
        if blocked {
            tracing::warn!("[Sandbox] not granting {:?} on {}", access, real.display());
            return;
        }
        if !self.entries.iter().any(|(p, a)| p == &real && *a == access) {
            self.entries.push((real, access));
        }
    }

    /// `push`와 같지만, 권한을 주지 못하면(관리자가 설치한 폴더 등) 경고만 하고 그 폴더 없이 띄운다
    pub fn push_optional(&mut self, path: &Path, access: Access, data_dir: &Path, home: Option<&Path>) {
        let before = self.entries.len();
        self.push(path, access, data_dir, home);
        if self.entries.len() > before {
            let entry = self.entries[before].clone();
            self.optional.push(entry);
        }
    }
}

/// venv의 기반 Python 폴더 (`pyvenv.cfg`의 `home`). venv 실행 파일은 이 폴더의 python을 띄운다
pub fn venv_home(python_exe: &Path) -> Option<PathBuf> {
    let venv = python_exe.parent()?.parent()?;
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).ok()?;
    cfg.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim().eq_ignore_ascii_case("home")).then(|| PathBuf::from(value.trim()))
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Windows 명령줄 규칙(MSVC `CommandLineToArgvW`)에 맞게 인자 하나를 덧붙인다
pub fn push_arg(arg: &[u16], out: &mut Vec<u16>) {
    const QUOTE: u16 = b'"' as u16;
    const SLASH: u16 = b'\\' as u16;
    if !out.is_empty() {
        out.push(b' ' as u16);
    }
    let plain = !arg.is_empty() && !arg.iter().any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'\n' as u16 || c == 0x0b || c == QUOTE);
    if plain {
        out.extend_from_slice(arg);
        return;
    }
    out.push(QUOTE);
    let mut slashes = 0usize;
    for &c in arg {
        if c == SLASH {
            slashes += 1;
            continue;
        }
        if c == QUOTE {
            out.extend(std::iter::repeat_n(SLASH, slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(SLASH, slashes));
        }
        out.push(c);
        slashes = 0;
    }
    out.extend(std::iter::repeat_n(SLASH, slashes * 2));
    out.push(QUOTE);
}

#[cfg_attr(not(windows), allow(dead_code))]
/// 자식에게 줄 환경: 데몬 환경에 `Command`에서 바꾼 값을 얹는다 (이름은 대소문자 무시)
pub fn merge_env(base: Vec<(OsString, OsString)>, changes: Vec<(OsString, Option<OsString>)>) -> Vec<(OsString, OsString)> {
    let same = |a: &OsString, b: &OsString| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    let mut env = base;
    for (key, value) in changes {
        env.retain(|(k, _)| !same(k, &key));
        if let Some(value) = value {
            env.push((key, value));
        }
    }
    env.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    env
}

/// 폴더 DACL의 ACE 하나를 보는 데 필요한 것만 (`existing_grant`가 Windows API 없이 판정하도록)
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy)]
pub struct AceView {
    pub allowed: bool,
    pub mask: u32,
    pub flags: u8,
    pub who: Who,
}

/// ACE의 SID가 누구인가
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// 이 프로필의 SID
    Own,
    /// ALL APPLICATION PACKAGES
    Packages,
    Other,
}

/// 폴더에 원하는 권한이 이미 있는가
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    None,
    /// 데몬이 더하는 모양 그대로인 이 SID의 명시 ACE가 있다 (예전 데몬이 더한 것도): 데몬이 거둘 수 있다
    Own,
    /// 상속받았거나, ALL APPLICATION PACKAGES거나, 데몬이 더하지 않은 모양의 ACE로 이미 읽을 수 있다: 데몬이 거두지 않는다
    Elsewhere,
}

#[cfg_attr(not(windows), allow(dead_code))]
const ACE_OBJECT_INHERIT: u8 = 0x1;
#[cfg_attr(not(windows), allow(dead_code))]
const ACE_CONTAINER_INHERIT: u8 = 0x2;
#[cfg_attr(not(windows), allow(dead_code))]
const ACE_INHERITED: u8 = 0x10;

#[cfg_attr(not(windows), allow(dead_code))]
pub fn existing_grant(aces: impl IntoIterator<Item = AceView>, mask: u32, inherit: bool) -> Existing {
    let want = if inherit { ACE_OBJECT_INHERIT | ACE_CONTAINER_INHERIT } else { 0 };
    let mut found = Existing::None;
    for ace in aces {
        if !ace.allowed || ace.who == Who::Other || ace.mask & mask != mask || ace.flags & want != want {
            continue;
        }
        // 데몬이 더하는 모양(같은 권한, 같은 상속 표시) 그대로인 것만 데몬 것으로 본다. 관리자가 따로 준 넓은 권한은
        // 거두면(그 SID의 명시 ACE를 모두 뺀다) 함께 사라지므로 기록하지 않는다
        if ace.who == Who::Own && ace.mask == mask && ace.flags == want {
            return Existing::Own;
        }
        found = Existing::Elsewhere;
    }
    found
}

#[cfg_attr(not(windows), allow(dead_code))]
/// 지금 쓰지 않는 런타임 읽기 권한: 기록된 것 중 `daemon`(모든 프로필이 쓰는 데몬 런타임)과 그 프로필의 `keep`에 없는 것
pub fn stale_runtimes(
    record: &BTreeMap<String, BTreeSet<PathBuf>>,
    daemon: &BTreeSet<PathBuf>,
    keep: &BTreeMap<String, BTreeSet<PathBuf>>,
) -> BTreeMap<String, Vec<PathBuf>> {
    let none = BTreeSet::new();
    record
        .iter()
        .map(|(profile, paths)| {
            let kept = keep.get(profile).unwrap_or(&none);
            (profile.clone(), paths.iter().filter(|p| !daemon.contains(*p) && !kept.contains(*p)).cloned().collect::<Vec<_>>())
        })
        .filter(|(_, stale)| !stale.is_empty())
        .collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
/// 런타임 읽기 권한 기록 (`<data_dir>/sandbox-grants.json`: 프로필 → 폴더)
fn runtime_record_path() -> PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join("sandbox-grants.json")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn read_runtime_record(path: &Path) -> BTreeMap<String, BTreeSet<PathBuf>> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn write_runtime_record(path: &Path, record: &BTreeMap<String, BTreeSet<PathBuf>>) {
    let tmp = path.with_extension("json.tmp");
    let saved = serde_json::to_vec_pretty(record)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = saved {
        tracing::warn!("[Sandbox] could not save {}: {}", path.display(), e);
    }
}

/// 핸들의 실제 경로(`GetFinalPathNameByHandleW`, `\\?\` 형식)가 열려던 경로와 같은 곳인가.
/// `\\?\`·`\\?\UNC\` 머리, `/`, 끝 구분자, 대소문자를 맞춰 비교한다 (중간 폴더가 정션이면 다르다)
#[cfg_attr(not(windows), allow(dead_code))]
fn same_final_path(final_path: &str, requested: &str) -> bool {
    fn plain(p: &str) -> String {
        let p = p.replace('/', "\\");
        let p = match (p.strip_prefix(r"\\?\UNC\"), p.strip_prefix(r"\\?\")) {
            (Some(unc), _) => format!(r"\\{unc}"),
            (None, Some(rest)) => rest.to_string(),
            (None, None) => p,
        };
        let trimmed = p.trim_end_matches('\\');
        // 드라이브 루트(`C:\`)는 구분자를 남긴다
        if trimmed.len() == 2 && trimmed.ends_with(':') { format!("{trimmed}\\") } else { trimmed.to_string() }
    }
    plain(final_path).to_lowercase() == plain(requested).to_lowercase()
}

#[cfg(windows)]
pub use imp::{clear_low_labels, grant, revoke_stale_runtimes, spawn, Contained};

#[cfg(windows)]
mod imp {
    use super::{Access, Grants};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::ffi::{OsStr, OsString};
    use winapi::ctypes::c_void;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
    use std::path::Path;
    use std::ptr::null_mut;
    use winapi::shared::minwindef::{DWORD, FALSE, TRUE};
    use winapi::um::accctrl::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_INHERITANCE, REVOKE_ACCESS, SE_FILE_OBJECT, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TRUSTEE_IS_SID,
        TRUSTEE_IS_WELL_KNOWN_GROUP,
    };
    use winapi::um::aclapi::{GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo};
    use winapi::um::fileapi::{CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION, OPEN_EXISTING};
    use winapi::um::handleapi::SetHandleInformation;
    use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject};
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
    use winapi::um::namedpipeapi::CreatePipe;
    use winapi::um::processthreadsapi::{
        CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread,
        TerminateProcess, UpdateProcThreadAttribute, PROCESS_INFORMATION,
    };
    use winapi::um::securitybaseapi::{AddMandatoryAce, AllocateAndInitializeSid, EqualSid, GetAce, GetAclInformation, GetLengthSid, GetTokenInformation, InitializeAcl};
    use winapi::um::synchapi::WaitForSingleObject;
    use winapi::um::winbase::{
        LocalFree, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, HANDLE_FLAG_INHERIT,
        INFINITE, STARTF_USESTDHANDLES, STARTUPINFOEXW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    use winapi::um::winnt::{
        AclSizeInformation, JobObjectExtendedLimitInformation, ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_ACE_TYPE, ACE_HEADER,
        ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE, HANDLE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, OBJECT_INHERIT_ACE, PACL, PSECURITY_DESCRIPTOR,
        PSID, SECURITY_CAPABILITIES, SE_GROUP_ENABLED, SID_AND_ATTRIBUTES, SID_IDENTIFIER_AUTHORITY, SYNCHRONIZE, ACL, ACL_REVISION,
        LABEL_SECURITY_INFORMATION, SECURITY_MANDATORY_LABEL_AUTHORITY, SECURITY_MANDATORY_LOW_RID, SYSTEM_MANDATORY_LABEL_ACE,
        SYSTEM_MANDATORY_LABEL_ACE_TYPE, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC, WRITE_OWNER, SECURITY_CREATOR_OWNER_RIGHTS_RID,
        SECURITY_CREATOR_SID_AUTHORITY, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };

    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
    const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x0002_0009;
    /// SECURITY_APP_PACKAGE_AUTHORITY (S-1-15)
    const APP_PACKAGE_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 15];

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error(what: &str) -> String {
        format!("{}: {}", what, std::io::Error::last_os_error())
    }

    /// 데몬이 끝날 때까지 쓰는 SID (해제하지 않는다)
    struct Sid(PSID);
    unsafe impl Send for Sid {}
    unsafe impl Sync for Sid {}

    type CreateProfile = unsafe extern "system" fn(*const u16, *const u16, *const u16, *mut c_void, DWORD, *mut PSID) -> i32;
    type DeriveSid = unsafe extern "system" fn(*const u16, *mut PSID) -> i32;

    /// 프로필 SID. userenv의 함수는 실행 중에 찾는다 (없는 Windows에서는 `sandbox_unavailable`)
    fn container_sid(profile: &str) -> Result<PSID, String> {
        static SIDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Sid>>> = std::sync::OnceLock::new();
        let mut sids = SIDS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sid) = sids.get(profile) {
            return Ok(sid.0);
        }
        let sid = unsafe {
            let lib = LoadLibraryW(wide(OsStr::new("userenv.dll")).as_ptr());
            if lib.is_null() {
                return Err(last_error("userenv.dll"));
            }
            let create = GetProcAddress(lib, c"CreateAppContainerProfile".as_ptr());
            let derive = GetProcAddress(lib, c"DeriveAppContainerSidFromAppContainerName".as_ptr());
            if create.is_null() || derive.is_null() {
                return Err("AppContainer is not supported on this Windows".into());
            }
            let create: CreateProfile = std::mem::transmute(create);
            let derive: DeriveSid = std::mem::transmute(derive);
            let name = wide(OsStr::new(profile));
            let desc = wide(OsStr::new("saba-chan unverified module or extension"));
            let mut sid: PSID = null_mut();
            let hr = create(name.as_ptr(), name.as_ptr(), desc.as_ptr(), null_mut(), 0, &mut sid);
            if hr < 0 || sid.is_null() {
                // 이미 있으면 (HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)) 이름에서 SID를 얻는다
                sid = null_mut();
                let hr2 = derive(name.as_ptr(), &mut sid);
                if hr2 < 0 || sid.is_null() {
                    return Err(format!("AppContainer profile: 0x{:08x} / 0x{:08x}", hr as u32, hr2 as u32));
                }
            }
            sid
        };
        sids.insert(profile.to_string(), Sid(sid));
        Ok(sid)
    }

    /// S-1-15-3-1 (internetClient), S-1-15-2-1 (ALL APPLICATION PACKAGES)
    fn well_known(sub0: DWORD, sub1: DWORD) -> Result<PSID, String> {
        let mut auth = SID_IDENTIFIER_AUTHORITY { Value: APP_PACKAGE_AUTHORITY };
        let mut sid: PSID = null_mut();
        if unsafe { AllocateAndInitializeSid(&mut auth, 2, sub0, sub1, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
            return Err(last_error("AllocateAndInitializeSid"));
        }
        Ok(sid)
    }

    fn internet_client() -> Result<PSID, String> {
        static SID: std::sync::OnceLock<Result<Sid, String>> = std::sync::OnceLock::new();
        SID.get_or_init(|| well_known(3, 1).map(Sid)).as_ref().map(|s| s.0).map_err(Clone::clone)
    }

    fn all_app_packages() -> Result<PSID, String> {
        static SID: std::sync::OnceLock<Result<Sid, String>> = std::sync::OnceLock::new();
        SID.get_or_init(|| well_known(2, 1).map(Sid)).as_ref().map(|s| s.0).map_err(Clone::clone)
    }

    fn rights(access: Access) -> (DWORD, bool) {
        match access {
            Access::List => (FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE, false),
            Access::Read => (FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, true),
            Access::Write => (FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE | FILE_DELETE_CHILD, true),
        }
    }

    /// DACL에 이미 그 권한을 주는 허용 ACE가 있는가 (프로필 SID나 ALL APPLICATION PACKAGES)
    unsafe fn existing(dacl: PACL, sid: PSID, everyone: PSID, mask: DWORD, inherit: bool) -> super::Existing {
        use super::{AceView, Who};
        if dacl.is_null() {
            return super::Existing::Elsewhere; // DACL이 없으면 누구나 접근할 수 있다
        }
        let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
        if GetAclInformation(dacl, &mut info as *mut _ as *mut c_void, std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD, AclSizeInformation) == 0 {
            return super::Existing::None;
        }
        let aces = (0..info.AceCount).filter_map(|i| {
            let mut ace: *mut c_void = null_mut();
            if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                return None;
            }
            let header = &*(ace as *const ACE_HEADER);
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                return Some(AceView { allowed: false, mask: 0, flags: header.AceFlags, who: Who::Other });
            }
            let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
            let ace_sid = &allowed.SidStart as *const DWORD as PSID;
            let who = if EqualSid(ace_sid, sid) != 0 {
                Who::Own
            } else if EqualSid(ace_sid, everyone) != 0 {
                Who::Packages
            } else {
                Who::Other
            };
            Some(AceView { allowed: true, mask: allowed.Mask, flags: header.AceFlags, who })
        });
        super::existing_grant(aces.collect::<Vec<_>>(), mask, inherit)
    }

    /// ACL을 읽거나 고칠 폴더를 연다. 경로 마지막이 링크면 따라가지 않고, 연 폴더의 실제 경로가 `push`가 정한
    /// 경로와 다르면(그 사이 위 폴더가 정션으로 바뀜) 거절한다. 이후 읽기·쓰기는 이 핸들로 해서 이름이 바뀌어도
    /// 같은 폴더다. 고칠 권한(`WRITE_DAC` 등)은 고쳐야 할 때만 청한다 (이미 허용된 시스템 Python 폴더처럼 고칠 수
    /// 없는 곳도 읽기만으로 지나간다)
    fn open_checked(path: &Path, access: DWORD) -> Result<OwnedHandle, String> {
        let name = wide(path.as_os_str());
        unsafe {
            let h = CreateFileW(
                name.as_ptr(),
                READ_CONTROL | access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            );
            if h == winapi::um::handleapi::INVALID_HANDLE_VALUE {
                return Err(last_error(&format!("open {}", path.display())));
            }
            let handle = OwnedHandle::from_raw_handle(h as RawHandle);
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(h, &mut info) == 0 {
                return Err(last_error(&format!("inspect {}", path.display())));
            }
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                // 동기화 폴더(OneDrive 등)의 클라우드 파일도 재분석 지점이다. 막는 것은 같지만 원인을 따로 알린다
                if cloud_file(h) {
                    return Err(format!("{} {}", path.display(), crate::sandbox::SYNC_FOLDER_MARK));
                }
                return Err(format!("{} is a link", path.display()));
            }
            // 버퍼가 작으면 필요한 길이(끝 NUL 포함)를 돌려준다: 그만큼 늘려 한 번 더
            let mut buf = vec![0u16; 1024];
            let mut n = GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as DWORD, 0) as usize;
            if n >= buf.len() {
                buf.resize(n, 0);
                n = GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as DWORD, 0) as usize;
            }
            if n == 0 || n >= buf.len() {
                return Err(last_error(&format!("resolve {}", path.display())));
            }
            let real = OsString::from_wide(&buf[..n]);
            // 짧은 이름(8.3, `RUNNER~1`)은 긴 이름으로 펴서 비교한다 (정션은 펴지 않는다)
            if !super::same_final_path(&real.to_string_lossy(), &long_path(path).to_string_lossy()) {
                return Err(format!("{} now resolves to {}", path.display(), real.to_string_lossy()));
            }
            Ok(handle)
        }
    }

    /// 재분석 지점의 태그가 클라우드 파일(`IO_REPARSE_TAG_CLOUD`, `0x9000_?01A` 묶음)인지
    fn cloud_file(handle: HANDLE) -> bool {
        use winapi::um::minwinbase::FileAttributeTagInfo;
        use winapi::um::winbase::GetFileInformationByHandleEx;
        // winapi 0.3에 없는 구조체 (fileapi.h FILE_ATTRIBUTE_TAG_INFO)
        #[repr(C)]
        #[allow(non_snake_case)]
        struct FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: DWORD,
            ReparseTag: DWORD,
        }
        unsafe {
            let mut info: FILE_ATTRIBUTE_TAG_INFO = std::mem::zeroed();
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as DWORD,
            ) != 0
                && info.ReparseTag & 0xFFFF_0FFF == 0x9000_001A
        }
    }

    /// 권한이 이미 있으면 그대로 두고 무엇으로 있는지를, 없으면 ACE를 더하고 `None`을 돌려준다
    fn grant_one(path: &Path, sid: PSID, everyone: PSID, access: Access) -> Result<super::Existing, String> {
        use std::os::windows::io::AsRawHandle;
        let (mask, inherit) = rights(access);
        unsafe {
            let read = open_checked(path, 0)?;
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(read.as_raw_handle() as HANDLE, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                return Err(format!("read ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let existing = existing(dacl, sid, everyone, mask, inherit);
            LocalFree(sd);
            if existing != super::Existing::None {
                return Ok(existing);
            }
        }
        allow(path, sid, mask, inherit).map(|()| super::Existing::None)
    }

    /// 폴더 DACL에서 `sid`의 ACE를 모두 뺀다 (하위로 상속된 것도 함께 빠진다)
    fn remove_ace(path: &Path, sid: PSID) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            let dir = open_checked(path, WRITE_DAC)?;
            let h = dir.as_raw_handle() as HANDLE;
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                return Err(format!("read ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
            ea.grfAccessMode = REVOKE_ACCESS;
            ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
            ea.Trustee.TrusteeType = TRUSTEE_IS_WELL_KNOWN_GROUP;
            ea.Trustee.ptstrName = sid as *mut u16;
            let mut new_dacl: PACL = null_mut();
            let rc = SetEntriesInAclW(1, &mut ea, dacl, &mut new_dacl);
            if rc != 0 {
                LocalFree(sd);
                return Err(format!("build ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let rc = SetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), new_dacl, null_mut());
            LocalFree(new_dacl as *mut c_void);
            LocalFree(sd);
            if rc != 0 {
                return Err(format!("write ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
        }
        Ok(())
    }

    /// 폴더 DACL에 `sid` 허용 ACE를 더한다
    fn allow(path: &Path, sid: PSID, mask: DWORD, inherit: bool) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            let dir = open_checked(path, WRITE_DAC)?;
            let h = dir.as_raw_handle() as HANDLE;
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                return Err(format!("read ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
            ea.grfAccessPermissions = mask;
            ea.grfAccessMode = GRANT_ACCESS;
            ea.grfInheritance = if inherit { SUB_CONTAINERS_AND_OBJECTS_INHERIT } else { NO_INHERITANCE };
            ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
            ea.Trustee.TrusteeType = TRUSTEE_IS_WELL_KNOWN_GROUP;
            ea.Trustee.ptstrName = sid as *mut u16;
            let mut new_dacl: PACL = null_mut();
            let rc = SetEntriesInAclW(1, &mut ea, dacl, &mut new_dacl);
            if rc != 0 {
                LocalFree(sd);
                return Err(format!("build ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let rc = SetSecurityInfo(h, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), new_dacl, null_mut());
            LocalFree(new_dacl as *mut c_void);
            LocalFree(sd);
            if rc != 0 {
                return Err(format!("write ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
        }
        Ok(())
    }

    /// 낮은 무결성 수준 S-1-16-4096
    fn low_integrity() -> Result<PSID, String> {
        static SID: std::sync::OnceLock<Result<Sid, String>> = std::sync::OnceLock::new();
        SID.get_or_init(|| {
            let mut auth = SID_IDENTIFIER_AUTHORITY { Value: SECURITY_MANDATORY_LABEL_AUTHORITY };
            let mut sid: PSID = null_mut();
            if unsafe { AllocateAndInitializeSid(&mut auth, 1, SECURITY_MANDATORY_LOW_RID, 0, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
                return Err(last_error("AllocateAndInitializeSid"));
            }
            Ok(Sid(sid))
        })
        .as_ref()
        .map(|s| s.0)
        .map_err(Clone::clone)
    }

    /// OWNER RIGHTS S-1-3-4. 이 SID의 ACE가 있으면 소유자의 암묵적 권한(READ_CONTROL·WRITE_DAC) 대신 그 ACE를 쓴다
    fn owner_rights() -> Result<PSID, String> {
        static SID: std::sync::OnceLock<Result<Sid, String>> = std::sync::OnceLock::new();
        SID.get_or_init(|| {
            let mut auth = SID_IDENTIFIER_AUTHORITY { Value: SECURITY_CREATOR_SID_AUTHORITY };
            let mut sid: PSID = null_mut();
            if unsafe { AllocateAndInitializeSid(&mut auth, 1, SECURITY_CREATOR_OWNER_RIGHTS_RID, 0, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
                return Err(last_error("AllocateAndInitializeSid"));
            }
            Ok(Sid(sid))
        })
        .as_ref()
        .map(|s| s.0)
        .map_err(Clone::clone)
    }

    /// 데몬을 돌리는 사용자 SID (토큰에서 한 번 읽어 둔다)
    fn current_user() -> Result<&'static [u8], String> {
        static SID: std::sync::OnceLock<Result<Vec<u8>, String>> = std::sync::OnceLock::new();
        use std::os::windows::io::AsRawHandle;
        SID.get_or_init(|| unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(last_error("OpenProcessToken"));
            }
            let token = OwnedHandle::from_raw_handle(token as RawHandle);
            let mut buf = vec![0u8; 256];
            let mut len: DWORD = 0;
            if GetTokenInformation(token.as_raw_handle() as HANDLE, TokenUser, buf.as_mut_ptr() as *mut c_void, buf.len() as DWORD, &mut len) == 0 {
                return Err(last_error("GetTokenInformation"));
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let sid = user.User.Sid;
            let n = GetLengthSid(sid) as usize;
            Ok(std::slice::from_raw_parts(sid as *const u8, n).to_vec())
        })
        .as_ref()
        .map(Vec::as_slice)
        .map_err(Clone::clone)
    }

    /// 표시를 바꿀 핸들(`WRITE_OWNER`). 사용자가 C:\ 바로 아래 만든 폴더(C:\Games 등)는 상속받은 "수정" 권한뿐이라
    /// 소유자에게도 WRITE_OWNER가 없다. 소유자가 데몬 사용자일 때만, 소유자의 암묵적 WRITE_DAC으로 그 폴더에
    /// OWNER RIGHTS ACE를 잠깐 더해 핸들을 열고 원래 DACL로 되돌린다 (ACE는 남지 않는다)
    fn open_for_relabel(path: &Path) -> Result<OwnedHandle, String> {
        use std::os::windows::io::AsRawHandle;
        let first = match open_checked(path, WRITE_OWNER) {
            Ok(dir) => return Ok(dir),
            Err(e) => e,
        };
        unsafe {
            let read = open_checked(path, 0)?;
            let mut owner: PSID = null_mut();
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(
                read.as_raw_handle() as HANDLE,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut sd,
            );
            if rc != 0 {
                return Err(format!("{first} (read owner: {})", std::io::Error::from_raw_os_error(rc as i32)));
            }
            let mine = current_user().is_ok_and(|me| !owner.is_null() && EqualSid(owner, me.as_ptr() as PSID) != 0);
            if !mine {
                LocalFree(sd);
                return Err(format!("{first} (owner is not the daemon user)"));
            }
            if let Err(e) = owner_rights().and_then(|s| allow(path, s, READ_CONTROL | WRITE_DAC | WRITE_OWNER, false)) {
                LocalFree(sd);
                return Err(format!("{first} ({e})"));
            }
            let dir = open_checked(path, WRITE_DAC | WRITE_OWNER);
            // 열었든 못 열었든 더한 ACE를 걷어 낸다 (원래 DACL 그대로)
            let restore = match &dir {
                Ok(d) => SetSecurityInfo(d.as_raw_handle() as HANDLE, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), dacl, null_mut()),
                Err(_) => match open_checked(path, WRITE_DAC) {
                    Ok(d) => SetSecurityInfo(d.as_raw_handle() as HANDLE, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), dacl, null_mut()),
                    Err(_) => 1,
                },
            };
            LocalFree(sd);
            if restore != 0 {
                tracing::warn!("[Sandbox] could not remove the temporary owner ACE on {}", path.display());
            }
            dir
        }
    }

    /// AppContainer 프로세스는 낮은 무결성 수준으로 돈다. 보통 폴더는 표시가 없어 중간 수준(쓰기 금지)으로 보므로
    /// DACL이 허용해도 쓰기가 막힌다: 쓰기를 준 폴더에는 낮은 수준 표시를 하위까지 상속되게 붙인다
    fn label_low(path: &Path) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let low = low_integrity()?;
        let flags = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as DWORD;
        unsafe {
            let read = open_checked(path, 0)?;
            let mut sacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(read.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), &mut sacl, &mut sd);
            if rc != 0 {
                return Err(format!("read label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let already = !sacl.is_null() && {
                let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
                GetAclInformation(sacl, &mut info as *mut _ as *mut c_void, std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD, AclSizeInformation) != 0
                    && (0..info.AceCount).any(|i| {
                        let mut ace: *mut c_void = null_mut();
                        if GetAce(sacl, i, &mut ace) == 0 || ace.is_null() {
                            return false;
                        }
                        let header = &*(ace as *const ACE_HEADER);
                        let label = &*(ace as *const SYSTEM_MANDATORY_LABEL_ACE);
                        header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE
                            && DWORD::from(header.AceFlags) & flags == flags
                            && EqualSid(&label.SidStart as *const DWORD as PSID, low) != 0
                    })
            };
            LocalFree(sd);
            if already {
                return Ok(());
            }
            // ACL 머리 + 표시 ACE 하나 + SID(하위 권한 1개)
            let mut buf = [0u32; 16];
            let acl = buf.as_mut_ptr() as *mut ACL;
            if InitializeAcl(acl, std::mem::size_of_val(&buf) as DWORD, DWORD::from(ACL_REVISION)) == 0
                || AddMandatoryAce(acl, DWORD::from(ACL_REVISION), flags, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, low) == 0
            {
                return Err(last_error("AddMandatoryAce"));
            }
            let dir = open_for_relabel(path)?;
            let rc = SetSecurityInfo(dir.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), acl);
            if rc != 0 {
                return Err(format!("write label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
        }
        Ok(())
    }

    /// 경로의 낮은 수준 표시: `None` 없음, `Some(true)` 위 폴더에서 상속받은 것, `Some(false)` 직접 붙은 것
    fn low_label(path: &Path) -> Result<Option<bool>, String> {
        use std::os::windows::io::AsRawHandle;
        let low = low_integrity()?;
        unsafe {
            let read = open_checked(path, 0)?;
            let mut sacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(read.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), &mut sacl, &mut sd);
            if rc != 0 {
                return Err(format!("read label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let mut found = None;
            let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            if !sacl.is_null()
                && GetAclInformation(sacl, &mut info as *mut _ as *mut c_void, std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD, AclSizeInformation) != 0
            {
                for i in 0..info.AceCount {
                    let mut ace: *mut c_void = null_mut();
                    if GetAce(sacl, i, &mut ace) == 0 || ace.is_null() {
                        continue;
                    }
                    let header = &*(ace as *const ACE_HEADER);
                    let label = &*(ace as *const SYSTEM_MANDATORY_LABEL_ACE);
                    if header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE && EqualSid(&label.SidStart as *const DWORD as PSID, low) != 0 {
                        found = Some(header.AceFlags & super::ACE_INHERITED != 0);
                        break;
                    }
                }
            }
            LocalFree(sd);
            Ok(found)
        }
    }

    /// `label_low`가 붙인 낮은 수준 표시를 `dir`과 그 아래 폴더·파일에서 걷는다. 상속받은 표시도 함께 빠진다.
    /// 낮은 수준 표시가 붙은 실행 파일로 띄운 프로세스는 낮은 무결성으로 돌아 서버(Java 등)가 뜨지 못하므로,
    /// 서버를 띄우기 전에 부른다. 걷은 폴더·파일 수를 돌려준다. 하나에서 실패해도 나머지를 마저 걷고, 표시가 있는데
    /// 걷지 못했거나 표시를 읽지 못한 경로를 모아 Err로 돌려준다(읽지 못하면 표시가 남았는지 알 수 없다).
    /// 볼륨이 보안 정보를 두지 않으면(FAT 등) 표시가 있을 수 없으므로 아무것도 하지 않는다
    pub fn clear_low_labels(dir: &Path) -> Result<usize, Vec<(PathBuf, String)>> {
        let _guard = acl_lock();
        // 실제 경로(`\\?\` 긴 이름)에서 시작한다: 짧은 이름(8.3)이 섞인 경로는 열 때의 경로 확인과 맞지 않는다
        let dir = std::fs::canonicalize(dir).map_err(|e| vec![(dir.to_path_buf(), format!("resolve: {e}"))])?;
        match persistent_acls(&dir) {
            Ok(false) => return Ok(0),
            Ok(true) => {}
            Err(e) => return Err(vec![(dir, e)]),
        }
        let mut cleared = 0;
        let mut failed = Vec::new();
        let mut pending = vec![dir];
        while let Some(path) = pending.pop() {
            // 위 폴더부터 걷으므로 여기서 남은 상속 표시는 다른 곳(sandbox-temp 등)에서 옮겨 온 폴더의 것이다:
            // 비우면 지금 위 폴더 기준으로 다시 계산되어 빠진다. 위 폴더에 진짜 표시가 있어 남으면 실패로 본다
            clear_one(&path, &mut cleared, &mut failed);
            // 링크는 따라가지 않는다 (폴더 밖을 고치지 않게)
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        match entry.file_type() {
                            Ok(t) if t.is_symlink() => {}
                            Ok(t) if t.is_dir() => pending.push(entry.path()),
                            // 폴더 표시를 걷어도 이미 있는 파일의 상속 표시는 파일에 WRITE_OWNER가 있을 때만 다시
                            // 계산된다. C:\ 아래 폴더처럼 "수정" 권한만 상속받은 파일은 그대로 남으므로 파일마다 걷는다
                            Ok(_) => clear_one(&entry.path(), &mut cleared, &mut failed),
                            Err(e) => failed.push((entry.path(), format!("read the type: {e}"))),
                        }
                    }
                }
                Err(e) => tracing::warn!("[Sandbox] could not list {}: {}", path.display(), e),
            }
        }
        if failed.is_empty() {
            Ok(cleared)
        } else {
            Err(failed)
        }
    }

    /// 한 경로(폴더나 파일)의 낮은 수준 표시를 걷고 결과를 센다
    fn clear_one(path: &Path, cleared: &mut usize, failed: &mut Vec<(PathBuf, String)>) {
        match low_label(path) {
            Ok(Some(_)) => match clear_label(path).and_then(|()| match low_label(path)? {
                None => Ok(()),
                Some(_) => Err("the label is inherited from a parent folder".to_string()),
            }) {
                Ok(()) => *cleared += 1,
                Err(e) => failed.push((path.to_path_buf(), e)),
            },
            Ok(None) => {}
            Err(e) => failed.push((path.to_path_buf(), format!("read the label: {e}"))),
        }
    }

    /// 경로의 짧은 이름(8.3)을 긴 이름으로 편다. 링크·정션은 따라가지 않는다. 실패하면 그대로
    fn long_path(path: &Path) -> PathBuf {
        use std::os::windows::ffi::OsStringExt;
        use winapi::um::fileapi::GetLongPathNameW;
        let name = wide(path.as_os_str());
        let mut buf = vec![0u16; 1024];
        unsafe {
            let mut n = GetLongPathNameW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as DWORD) as usize;
            if n >= buf.len() {
                buf.resize(n, 0);
                n = GetLongPathNameW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as DWORD) as usize;
            }
            if n == 0 || n >= buf.len() {
                return path.to_path_buf();
            }
            PathBuf::from(OsString::from_wide(&buf[..n]))
        }
    }

    /// 경로가 있는 볼륨이 파일 보안 정보(ACL·표시)를 두는가 (NTFS·ReFS는 예, FAT·exFAT는 아니오)
    fn persistent_acls(path: &Path) -> Result<bool, String> {
        use winapi::um::fileapi::{GetVolumeInformationW, GetVolumePathNameW};
        let name = wide(path.as_os_str());
        let mut root = vec![0u16; name.len().max(260) + 1];
        let mut flags: DWORD = 0;
        unsafe {
            if GetVolumePathNameW(name.as_ptr(), root.as_mut_ptr(), root.len() as DWORD) == 0 {
                return Err(last_error(&format!("find the volume of {}", path.display())));
            }
            if GetVolumeInformationW(root.as_ptr(), null_mut(), 0, null_mut(), null_mut(), &mut flags, null_mut(), 0) == 0 {
                return Err(last_error(&format!("read the volume of {}", path.display())));
            }
        }
        Ok(flags & winapi::um::winnt::FILE_PERSISTENT_ACLS != 0)
    }

    /// 표시 목록을 비운다 (ACL 머리만)
    fn clear_label(path: &Path) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let mut buf = [0u32; 4];
        let acl = buf.as_mut_ptr() as *mut ACL;
        if unsafe { InitializeAcl(acl, std::mem::size_of_val(&buf) as DWORD, DWORD::from(ACL_REVISION)) } == 0 {
            return Err(last_error("InitializeAcl"));
        }
        let handle = open_for_relabel(path)?;
        let rc = unsafe { SetSecurityInfo(handle.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), acl) };
        if rc != 0 {
            return Err(format!("clear label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
        }
        Ok(())
    }

    #[cfg(test)]
    mod label_tests {
        /// `exe`로 띄운 프로세스의 무결성 수준 RID (0x1000 낮음, 0x2000 중간). 멈춘 채 띄워 보고 바로 끝낸다
        fn launch_integrity(exe: &std::path::Path) -> u32 {
            use super::*;
            use std::os::windows::io::AsRawHandle;
            use winapi::um::winnt::{TokenIntegrityLevel, TOKEN_MANDATORY_LABEL};
            unsafe {
                let app = wide(exe.as_os_str());
                let mut line = wide(std::ffi::OsStr::new("probe --list"));
                let mut si: winapi::um::processthreadsapi::STARTUPINFOW = std::mem::zeroed();
                si.cb = std::mem::size_of_val(&si) as DWORD;
                let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
                assert_ne!(
                    CreateProcessW(app.as_ptr(), line.as_mut_ptr(), null_mut(), null_mut(), FALSE, CREATE_SUSPENDED | CREATE_NO_WINDOW, null_mut(), null_mut(), &mut si, &mut pi),
                    0,
                    "{}",
                    last_error("CreateProcessW")
                );
                let process = OwnedHandle::from_raw_handle(pi.hProcess as RawHandle);
                let _thread = OwnedHandle::from_raw_handle(pi.hThread as RawHandle);
                let mut token: HANDLE = null_mut();
                let opened = OpenProcessToken(process.as_raw_handle() as HANDLE, TOKEN_QUERY, &mut token);
                let mut buf = vec![0u8; 256];
                let mut len: DWORD = 0;
                let read = opened != 0
                    && GetTokenInformation(token, TokenIntegrityLevel, buf.as_mut_ptr() as *mut c_void, buf.len() as DWORD, &mut len) != 0;
                let rid = if read {
                    let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
                    let count = *winapi::um::securitybaseapi::GetSidSubAuthorityCount(label.Label.Sid);
                    *winapi::um::securitybaseapi::GetSidSubAuthority(label.Label.Sid, DWORD::from(count) - 1)
                } else {
                    0
                };
                if !token.is_null() {
                    drop(OwnedHandle::from_raw_handle(token as RawHandle));
                }
                TerminateProcess(process.as_raw_handle() as HANDLE, 1);
                assert!(read, "read the integrity level");
                rid
            }
        }

        /// 낮은 수준 표시를 상속한 실행 파일은 낮은 무결성으로 뜬다(서버가 DLL을 못 읽고 끝나던 원인). 걷으면 중간으로 뜬다
        #[test]
        fn a_server_in_a_folder_the_sandbox_wrote_runs_at_medium_integrity_after_the_labels_go() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            super::label_low(&dir).unwrap();
            let exe = dir.join("probe.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
            assert_eq!(launch_integrity(&exe), 0x1000, "the inherited low label makes the launch low integrity");
            super::clear_low_labels(&dir).unwrap();
            assert!(launch_integrity(&exe) >= 0x2000, "after the labels go the server starts at the daemon's level");
        }

        /// 격리 실행에 쓰기를 준 폴더(낮은 수준 표시)에 받은 서버 파일도, 서버를 띄우기 전에 표시가 걷힌다
        #[test]
        fn server_files_lose_the_low_label_before_launch() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            let bin = dir.join("server").join("java").join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            super::label_low(&dir).unwrap();
            super::label_low(&dir.join("server")).unwrap();
            let exe = bin.join("java.exe");
            std::fs::write(&exe, b"x").unwrap();
            assert_eq!(super::low_label(&exe).unwrap(), Some(true));
            // `server`는 이미 상속받은 표시가 있어 직접 붙지 않았다: 위 폴더만 걷으면 함께 빠진다
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 1);
            assert_eq!(super::low_label(&exe).unwrap(), None);
            assert_eq!(super::low_label(&dir.join("server")).unwrap(), None);
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 0, "nothing left to clear");
        }

        // 데몬은 인스턴스 폴더를 `C:\..` 형식으로 넘긴다 (`\\?\` 형식이 아니다). 실제 PC에서는 이 경로를
        // 여는 확인이 모두 "다른 곳"으로 판정해 표시를 하나도 걷지 못했다
        #[test]
        fn labels_go_when_the_folder_is_given_as_a_plain_path() {
            let tmp = tempfile::tempdir().unwrap();
            let plain = tmp.path().to_path_buf();
            assert!(!plain.to_string_lossy().starts_with(r"\\?\"), "{}", plain.display());
            let real = std::fs::canonicalize(&plain).unwrap();
            std::fs::create_dir_all(plain.join("server").join("bin")).unwrap();
            super::label_low(&real).unwrap();
            let exe = real.join("server").join("bin").join("java.exe");
            std::fs::write(&exe, b"x").unwrap();
            assert_eq!(super::low_label(&exe).unwrap(), Some(true));
            assert_eq!(super::low_label(&plain.join("server")).unwrap(), Some(true), "a plain path opens");
            assert_eq!(super::clear_low_labels(&plain).unwrap(), 1);
            assert_eq!(super::low_label(&exe).unwrap(), None);
        }

        // 중간 폴더가 정션이면 다른 곳이다: 그 경로로는 표시를 읽거나 고치지 않는다
        #[test]
        fn a_path_through_a_junction_is_refused() {
            let tmp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(tmp.path()).unwrap();
            std::fs::create_dir_all(root.join("target").join("sub")).unwrap();
            let target = root.join("target");
            let link = tmp.path().join("link");
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .unwrap();
            assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
            let err = super::low_label(&link.join("sub")).unwrap_err();
            assert!(err.contains("resolves to"), "{err}");
        }

        // 표시가 붙은 폴더(sandbox-temp 등)에서 받은 뒤 서버 폴더로 옮긴 파일은 상속 표시를 그대로 갖고 온다
        #[test]
        fn files_moved_in_from_a_labelled_folder_lose_the_label_too() {
            let tmp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(tmp.path()).unwrap();
            let (staging, dir) = (root.join("staging"), root.join("server"));
            std::fs::create_dir_all(staging.join("jre").join("bin")).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            super::label_low(&staging).unwrap();
            let exe = staging.join("jre").join("bin").join("java.exe");
            std::fs::write(&exe, b"x").unwrap();
            std::fs::rename(staging.join("jre"), dir.join("jre")).unwrap();
            let moved = dir.join("jre").join("bin").join("java.exe");
            assert_eq!(super::low_label(&moved).unwrap(), Some(true), "a move keeps the inherited label");
            assert!(super::clear_low_labels(&dir).unwrap() >= 1);
            assert_eq!(super::low_label(&moved).unwrap(), None);
        }

        // 실제 PC의 인스턴스 폴더(C:\ 아래)는 상속받은 "수정" 권한뿐이라 파일에 WRITE_OWNER가 없다. 이때 폴더 표시를
        // 걷어도 이미 있는 파일의 상속 표시는 다시 계산되지 않아 남았다(Java가 낮은 무결성으로 떠 DLL을 못 읽음)
        #[test]
        fn files_without_write_owner_lose_the_inherited_label() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap().join("instance");
            std::fs::create_dir_all(&dir).unwrap();
            let icacls = |path: &std::path::Path, args: &[&str]| {
                let out = std::process::Command::new("icacls").arg(path).args(args).output().unwrap();
                (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
            };
            // 준비하는 동안은 Administrators 모든 권한도 둔다: 관리자 토큰(CI)이 만든 항목은 소유자가 Administrators라
            // 그 권한 없이는 폴더에 표시를 붙이지 못한다
            let (ok, out) = icacls(&dir, &["/inheritance:r", "/grant:r", "*S-1-5-11:(OI)(CI)M", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"]);
            assert!(ok, "{out}");
            let server = dir.join("server");
            std::fs::create_dir_all(&server).unwrap();
            super::label_low(&server).unwrap();
            let bin = server.join("java").join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let exe = bin.join("probe.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
            std::fs::write(bin.join("jli.dll"), b"x").unwrap();
            std::fs::write(server.join("server.jar"), b"x").unwrap();
            // 그다음 실제 PC와 같게 맞춘다: 소유자는 데몬 사용자(관리자 토큰이면 Administrators였다), 권한은
            // C:\ 아래 새 폴더처럼 인증된 사용자 수정·SYSTEM 모든 권한뿐이라 파일에 WRITE_OWNER가 없다.
            // 제한된 토큰(실제 PC)에서는 소유자가 이미 데몬 사용자라 소유자 바꾸기가 거부돼도 그대로다
            let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
            let _ = icacls(&dir, &["/setowner", &user, "/T", "/C", "/Q"]);
            let (ok, out) = icacls(&dir, &["/remove:g", "*S-1-5-32-544"]);
            assert!(ok, "{out}");
            assert_eq!(super::low_label(&exe).unwrap(), Some(true));
            assert!(super::open_checked(&exe, winapi::um::winnt::WRITE_OWNER).is_err(), "the file must not grant WRITE_OWNER, as on a real PC");
            assert_eq!(launch_integrity(&exe), 0x1000);
            super::clear_low_labels(&dir).unwrap();
            for file in [&exe, &bin.join("jli.dll"), &server.join("server.jar")] {
                assert_eq!(super::low_label(file).unwrap(), None, "{}", file.display());
            }
            assert!(launch_integrity(&exe) >= 0x2000, "the server starts at the daemon's level");
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 0, "nothing left to clear");
        }
    }

    /// 띄우지 않고 그 프로필에 권한만 준다 (사용자가 정한 런타임 등)
    pub fn grant(profile: &str, grants: &Grants) -> Result<(), String> {
        apply(profile, grants).map(|_| ())
    }

    /// ACL 고치기는 읽고-바꾸고-쓰기라, 두 실행이 같은 폴더(Python 설치 등)에 동시에 주면 한쪽 ACE가 사라진다.
    /// 프로필이 구성요소마다 달라 겹칠 수 있으므로 데몬 안에서 한 번에 하나씩 (런타임 기록도 이 잠금 안에서)
    fn acl_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 오래 걸린 권한 주기를 로그에 남긴다 (반복의 끝에서, 실패로 빠져나가도)
    struct SlowGrant<'a> {
        path: &'a Path,
        access: Access,
        started: std::time::Instant,
    }

    impl Drop for SlowGrant<'_> {
        fn drop(&mut self) {
            let took = self.started.elapsed();
            if took >= std::time::Duration::from_millis(500) {
                tracing::info!("[Sandbox] granting {:?} on {} took {} ms", self.access, self.path.display(), took.as_millis());
            }
        }
    }

    fn apply(profile: &str, grants: &Grants) -> Result<PSID, String> {
        let _guard = acl_lock();
        let sid = container_sid(profile)?;
        let everyone = all_app_packages()?;
        let mut runtimes = Vec::new();
        for (path, access) in &grants.entries {
            // 상속되는 권한·표시는 하위 파일 전부에 퍼진다: 큰 폴더는 처음 한 번 오래 걸릴 수 있어 남긴다
            let started = std::time::Instant::now();
            let _slow = SlowGrant { path, access: *access, started };
            let optional = grants.optional.contains(&(path.clone(), *access));
            let existing = match grant_one(path, sid, everyone, *access) {
                Ok(existing) => existing,
                Err(e) if optional => {
                    tracing::warn!("[Sandbox] skipping {}: {}", path.display(), e);
                    continue;
                }
                Err(e) => return Err(e),
            };
            // 없어도 되는 읽기 권한(런타임 폴더)은 이 SID의 명시 ACE(지금 더한 것, 예전 데몬이 더한 것)를 기록해 두었다가
            // 쓰지 않게 되면 거둔다. 상속받았거나 모든 앱 패키지로 읽을 수 있는 폴더는 이 폴더에서 거둘 것이 없다
            if optional && *access == Access::Read {
                let state = match existing {
                    super::Existing::None => "granted",
                    super::Existing::Own => "already granted to this profile",
                    super::Existing::Elsewhere => "already readable (inherited or all app packages), not recorded",
                };
                tracing::info!("[Sandbox] runtime {} for {}: {}", path.display(), profile, state);
                if existing != super::Existing::Elsewhere {
                    runtimes.push(path.clone());
                }
            }
            // 표시를 지원하지 않는 볼륨(FAT·일부 네트워크 드라이브 등)도 있다: 실행은 막지 않고, 그 폴더 쓰기만 안 될 수 있다
            if *access == Access::Write {
                if let Err(e) = label_low(path) {
                    tracing::warn!("[Sandbox] could not mark {} writable for low integrity: {}", path.display(), e);
                }
            }
        }
        if !runtimes.is_empty() {
            let file = super::runtime_record_path();
            let mut record = super::read_runtime_record(&file);
            let known = record.entry(profile.to_string()).or_default();
            let before = known.len();
            known.extend(runtimes);
            if known.len() != before {
                super::write_runtime_record(&file, &record);
            }
        }
        Ok(sid)
    }

    /// 기록된 런타임 읽기 권한 중 `daemon`·`keep`에 없는 것을 거둔다 (`super::stale_runtimes`).
    /// 폴더가 남아 있는데 거두지 못한 것은 기록에 두고 다음 시작 때 다시 시도한다
    pub fn revoke_stale_runtimes(daemon: &BTreeSet<PathBuf>, keep: &BTreeMap<String, BTreeSet<PathBuf>>) {
        let _guard = acl_lock();
        let file = super::runtime_record_path();
        let mut record = super::read_runtime_record(&file);
        let stale = super::stale_runtimes(&record, daemon, keep);
        if stale.is_empty() {
            return;
        }
        for (profile, paths) in &stale {
            let sid = match container_sid(profile) {
                Ok(sid) => sid,
                Err(e) => {
                    tracing::warn!("[Sandbox] could not revoke runtimes of {}: {}", profile, e);
                    continue;
                }
            };
            for path in paths {
                // 거뒀거나 폴더가 없어졌으면(ACE도 함께 사라짐) 기록에서 빼고, 아니면 다음 시작 때 다시 시도한다
                let done = match remove_ace(path, sid) {
                    Ok(()) => true,
                    Err(_) if !path.exists() => true,
                    Err(e) => {
                        tracing::warn!("[Sandbox] could not revoke {} from {}: {}", path.display(), profile, e);
                        false
                    }
                };
                if let (true, Some(known)) = (done, record.get_mut(profile)) {
                    known.remove(path);
                }
            }
        }
        record.retain(|_, paths| !paths.is_empty());
        super::write_runtime_record(&file, &record);
    }

    fn pipe() -> Result<(OwnedHandle, OwnedHandle), String> {
        let mut sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD, lpSecurityDescriptor: null_mut(), bInheritHandle: TRUE };
        let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
        if unsafe { CreatePipe(&mut r, &mut w, &mut sa, 0) } == 0 {
            return Err(last_error("CreatePipe"));
        }
        unsafe { Ok((OwnedHandle::from_raw_handle(r as RawHandle), OwnedHandle::from_raw_handle(w as RawHandle))) }
    }

    fn not_inherited(h: &OwnedHandle) {
        use std::os::windows::io::AsRawHandle;
        unsafe { SetHandleInformation(h.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 0) };
    }

    /// AppContainer 안에서 돈 프로세스
    pub struct Contained {
        process: OwnedHandle,
        job: OwnedHandle,
        pub stdin: Option<std::fs::File>,
        pub stdout: Option<std::fs::File>,
        pub stderr: Option<std::fs::File>,
    }

    impl Contained {
        /// 끝날 때까지 기다린다 (막히는 호출이므로 `spawn_blocking`에서)
        pub fn wait_handle(&self) -> usize {
            use std::os::windows::io::AsRawHandle;
            self.process.as_raw_handle() as usize
        }

        /// Job째 끝낸다 (Python과 그것이 띄운 것 모두)
        pub fn kill(&self) {
            use std::os::windows::io::AsRawHandle;
            unsafe { TerminateJobObject(self.job.as_raw_handle() as HANDLE, 1) };
        }
    }

    /// `wait_handle`의 프로세스가 끝날 때까지 기다려 종료 코드를 돌려준다
    pub fn wait(handle: usize) -> std::io::Result<u32> {
        unsafe {
            WaitForSingleObject(handle as HANDLE, INFINITE);
            let mut code: DWORD = 0;
            if GetExitCodeProcess(handle as HANDLE, &mut code) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(code)
        }
    }

    /// `profile`의 SID에 폴더 권한을 주고 그 AppContainer 안에서 띄운다. 표준 입출력은 파이프로 잇는다
    pub fn spawn(
        program: &OsStr,
        args: &[OsString],
        cwd: Option<&Path>,
        env: &[(OsString, OsString)],
        profile: &str,
        grants: &Grants,
    ) -> Result<Contained, String> {
        let sid = apply(profile, grants)?;
        let internet = internet_client()?;

        let (in_r, in_w) = pipe()?;
        let (out_r, out_w) = pipe()?;
        let (err_r, err_w) = pipe()?;
        for h in [&in_w, &out_r, &err_r] {
            not_inherited(h);
        }

        let mut command_line = Vec::new();
        super::push_arg(&program.encode_wide().collect::<Vec<_>>(), &mut command_line);
        for arg in args {
            super::push_arg(&arg.encode_wide().collect::<Vec<_>>(), &mut command_line);
        }
        command_line.push(0);
        let mut env_block: Vec<u16> = Vec::new();
        for (k, v) in env {
            env_block.extend(k.encode_wide());
            env_block.push(b'=' as u16);
            env_block.extend(v.encode_wide());
            env_block.push(0);
        }
        if env_block.is_empty() {
            env_block.push(0);
        }
        env_block.push(0);
        let program_w = wide(program);
        let cwd_w = cwd.map(|d| wide(d.as_os_str()));

        unsafe {
            use std::os::windows::io::AsRawHandle;
            let mut capability = SID_AND_ATTRIBUTES { Sid: internet, Attributes: SE_GROUP_ENABLED };
            let mut caps: SECURITY_CAPABILITIES = std::mem::zeroed();
            caps.AppContainerSid = sid;
            caps.Capabilities = &mut capability;
            caps.CapabilityCount = 1;
            let mut inherit: [HANDLE; 3] = [in_r.as_raw_handle() as HANDLE, out_w.as_raw_handle() as HANDLE, err_w.as_raw_handle() as HANDLE];

            let mut size: usize = 0;
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size);
            let mut list_buf = vec![0u8; size];
            let list = list_buf.as_mut_ptr() as _;
            if InitializeProcThreadAttributeList(list, 2, 0, &mut size) == 0 {
                return Err(last_error("InitializeProcThreadAttributeList"));
            }
            let updated = UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                &mut caps as *mut _ as *mut c_void,
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                null_mut(),
                null_mut(),
            ) != 0
                && UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                    inherit.as_mut_ptr() as *mut c_void,
                    std::mem::size_of_val(&inherit),
                    null_mut(),
                    null_mut(),
                ) != 0;
            if !updated {
                let e = last_error("UpdateProcThreadAttribute");
                DeleteProcThreadAttributeList(list);
                return Err(e);
            }

            let mut si: STARTUPINFOEXW = std::mem::zeroed();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = inherit[0];
            si.StartupInfo.hStdOutput = inherit[1];
            si.StartupInfo.hStdError = inherit[2];
            si.lpAttributeList = list;
            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
            let created = CreateProcessW(
                program_w.as_ptr(),
                command_line.as_mut_ptr(),
                null_mut(),
                null_mut(),
                TRUE,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                env_block.as_mut_ptr() as *mut c_void,
                cwd_w.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                &mut si.StartupInfo,
                &mut pi,
            );
            let create_error = last_error("CreateProcessW");
            DeleteProcThreadAttributeList(list);
            // 자식 쪽 끝은 여기서 닫는다 (자식이 가진 사본만 남는다)
            drop((in_r, out_w, err_w));
            if created == FALSE {
                return Err(create_error);
            }
            let process = OwnedHandle::from_raw_handle(pi.hProcess as RawHandle);
            let thread = OwnedHandle::from_raw_handle(pi.hThread as RawHandle);

            let job = CreateJobObjectW(null_mut(), std::ptr::null());
            if job.is_null() {
                let e = last_error("CreateJobObjectW");
                TerminateProcess(pi.hProcess, 1);
                return Err(e);
            }
            let job = OwnedHandle::from_raw_handle(job as RawHandle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let job_ok = SetInformationJobObject(
                job.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                &mut limits as *mut _ as *mut c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
            ) != 0
                && AssignProcessToJobObject(job.as_raw_handle() as HANDLE, pi.hProcess) != 0;
            if !job_ok {
                let e = last_error("Job Object");
                TerminateProcess(pi.hProcess, 1);
                return Err(e);
            }
            ResumeThread(thread.as_raw_handle() as HANDLE);
            drop(thread);

            Ok(Contained {
                process,
                job,
                stdin: Some(std::fs::File::from(in_w)),
                stdout: Some(std::fs::File::from(out_r)),
                stderr: Some(std::fs::File::from(err_r)),
            })
        }
    }
}

#[cfg(windows)]
pub use imp::wait;

#[cfg(test)]
mod tests {
    use super::*;

    /// 데몬은 `C:\..` 형식으로 넘기고 GetFinalPathNameByHandleW는 `\\?\C:\..`를 돌려준다: 같은 곳이다.
    /// 다른 곳을 가리키면(중간 폴더가 정션) 다르다
    #[test]
    fn a_verbatim_final_path_names_the_same_place_as_the_plain_one() {
        assert!(same_final_path(r"\\?\C:\Users\me\saba\instances\x\server", r"C:\Users\me\saba\instances\x\server"));
        assert!(same_final_path(r"\\?\C:\Users\me\x", r"\\?\C:\Users\me\x"));
        assert!(same_final_path(r"\\?\C:\Users\Me\X", r"c:/users/me/x/"));
        assert!(same_final_path(r"\\?\UNC\nas\share\x", r"\\nas\share\x"));
        assert!(same_final_path(r"\\?\C:\", r"C:\"));
        assert!(!same_final_path(r"\\?\D:\elsewhere\server", r"C:\Users\me\x\server"));
        assert!(!same_final_path(r"\\?\C:\Users\me\x\server2", r"C:\Users\me\x\server"));
    }

    #[test]
    fn runtimes_no_one_uses_any_more_are_the_stale_ones() {
        let set = |ps: &[&str]| ps.iter().map(PathBuf::from).collect::<BTreeSet<_>>();
        let record = BTreeMap::from([("a".to_string(), set(&["/jdk17", "/jdk21", "/mine"])), ("b".to_string(), set(&["/jdk21"]))]);
        let daemon = set(&["/jdk21"]);
        let keep = BTreeMap::from([("a".to_string(), set(&["/mine"]))]);
        assert_eq!(stale_runtimes(&record, &daemon, &keep), BTreeMap::from([("a".to_string(), vec![PathBuf::from("/jdk17")])]));
        // 사용자가 정한 런타임을 더 쓰지 않으면 그것도 거둔다
        assert_eq!(stale_runtimes(&record, &daemon, &BTreeMap::new())["a"], vec![PathBuf::from("/jdk17"), PathBuf::from("/mine")]);
    }

    #[test]
    fn only_this_profiles_own_explicit_ace_counts_as_a_grant_to_record() {
        const RX: u32 = 0xA0;
        let ace = |who, mask, flags| AceView { allowed: true, mask, flags, who };
        let oi_ci = ACE_OBJECT_INHERIT | ACE_CONTAINER_INHERIT;
        // 예전 데몬이 더한 이 SID의 명시 ACE는 기록해서 거둘 수 있게 한다
        assert_eq!(existing_grant([ace(Who::Other, RX, oi_ci), ace(Who::Own, RX, oi_ci)], RX, true), Existing::Own);
        // 위 폴더에서 상속받았거나 모든 앱 패키지로 읽을 수 있으면 이 폴더에서 거둘 것이 없다
        assert_eq!(existing_grant([ace(Who::Own, RX, oi_ci | ACE_INHERITED)], RX, true), Existing::Elsewhere);
        assert_eq!(existing_grant([ace(Who::Packages, RX, oi_ci)], RX, true), Existing::Elsewhere);
        // 둘 다 있으면 명시 ACE가 우선
        assert_eq!(existing_grant([ace(Who::Packages, RX, oi_ci), ace(Who::Own, RX, oi_ci)], RX, true), Existing::Own);
        // 데몬이 더하지 않은 모양(더 넓은 권한, 다른 상속 표시)이면 거두지 않는다
        assert_eq!(existing_grant([ace(Who::Own, RX | 0x116, oi_ci)], RX, true), Existing::Elsewhere);
        assert_eq!(existing_grant([ace(Who::Own, RX, oi_ci | 0x4)], RX, true), Existing::Elsewhere);
        // 권한이 모자라거나, 상속되지 않거나, 다른 SID거나, 거부 ACE면 없는 것
        assert_eq!(existing_grant([ace(Who::Own, 0x20, oi_ci)], RX, true), Existing::None);
        assert_eq!(existing_grant([ace(Who::Own, RX, 0)], RX, true), Existing::None);
        assert_eq!(existing_grant([ace(Who::Other, RX, oi_ci)], RX, true), Existing::None);
        assert_eq!(existing_grant([AceView { allowed: false, mask: RX, flags: oi_ci, who: Who::Own }], RX, true), Existing::None);
    }

    fn line(args: &[&str]) -> String {
        let mut out = Vec::new();
        for a in args {
            push_arg(&a.encode_utf16().collect::<Vec<_>>(), &mut out);
        }
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn arguments_are_quoted_like_the_msvc_runtime_reads_them() {
        assert_eq!(line(&["python.exe", "lifecycle.py", "start"]), "python.exe lifecycle.py start");
        assert_eq!(line(&[r"C:\Program Files\py\python.exe", ""]), r#""C:\Program Files\py\python.exe" """#);
        assert_eq!(line(&[r#"a"b"#]), r#""a\"b""#);
        assert_eq!(line(&[r"dir with space\"]), r#""dir with space\\""#);
        assert_eq!(line(&[r#"x\"y z"#]), r#""x\\\"y z""#);
        assert_eq!(line(&[r"no\space\"]), r"no\space\");
    }

    #[test]
    fn environment_changes_replace_by_name_ignoring_case() {
        let os = |s: &str| OsString::from(s);
        let env = merge_env(
            vec![(os("Path"), os("a")), (os("SABA_TOKEN_PATH"), os("t")), (os("HOME"), os("h"))],
            vec![(os("PATH"), Some(os("b"))), (os("saba_token_path"), None), (os("TEMP"), Some(os("x")))],
        );
        assert_eq!(env, vec![(os("HOME"), os("h")), (os("PATH"), os("b")), (os("TEMP"), os("x"))]);
    }

    #[test]
    fn data_dir_its_parents_roots_and_home_are_never_granted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let data = home.join("data");
        let inst = data.join("instances").join("1");
        let modules = data.join("modules");
        for d in [&inst, &modules] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut g = Grants::default();
        g.push(&inst, Access::Write, &data, Some(&home));
        g.push(&modules, Access::Read, &data, Some(&home));
        g.push(&data, Access::Read, &data, Some(&home));
        g.push(&home, Access::Write, &data, Some(&home));
        g.push(Path::new("/"), Access::Read, &data, Some(&home));
        g.push(&data.join("missing"), Access::Read, &data, Some(&home));
        g.push(&data, Access::List, &data, Some(&home));
        // 목록도 드라이브 루트·사용자 폴더와 그 위에는 주지 않는다
        g.push(&home, Access::List, &data, Some(&home));
        g.push(Path::new("/"), Access::List, &data, Some(&home));
        g.push(&inst, Access::Write, &data, Some(&home));
        assert_eq!(g.entries, vec![(inst.clone(), Access::Write), (modules.clone(), Access::Read), (data.clone(), Access::List)]);
    }

    #[test]
    fn venv_home_is_read_from_pyvenv_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Scripts")).unwrap();
        std::fs::write(tmp.path().join("pyvenv.cfg"), "home = C:\\py\\base\ninclude-system-site-packages = false\n").unwrap();
        assert_eq!(venv_home(&tmp.path().join("Scripts").join("python.exe")), Some(PathBuf::from("C:\\py\\base")));
        assert_eq!(venv_home(Path::new("python")), None);
    }
}
