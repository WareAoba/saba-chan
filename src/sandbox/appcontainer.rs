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

/// 사바쨩이 만든 프로필 이름인가 (구성요소별 프로필과 예전 공용 프로필)
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_saba_profile(name: &str) -> bool {
    name.starts_with(PROFILE_PREFIX) || name == PROFILE_PREFIX.trim_end_matches('.')
}

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
    /// 모든 구성요소가 함께 읽는 폴더(Python·모듈·익스텐션·Java 런타임): 프로필 SID가 아니라 ALL APPLICATION PACKAGES에 한 번 준다.
    /// 프로필마다 주면 그 폴더 DACL에 구성요소 수만큼 ACE가 쌓여, 64KB 한도를 넘으면 ACL을 쓰지 못한다(os error 87)
    pub shared: Vec<(PathBuf, Access)>,
    /// 받은 폴더 안에서 가리는 데몬 항목(실제 경로). 리눅스는 bwrap이 빈 파일(`empty_file`)·빈 폴더로 덮는다.
    /// Windows는 상속을 끊어 막으므로 쓰지 않는다
    pub hidden: Vec<PathBuf>,
    /// 데몬 파일을 덮는 빈 파일 (없으면 /dev/null: 읽으면 권한 거부이고 `is_file()`이 거짓이다)
    pub empty_file: Option<PathBuf>,
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

    /// `push`와 같지만 모든 구성요소가 함께 읽는 폴더로 둔다 (`shared`)
    pub fn push_shared(&mut self, path: &Path, access: Access, data_dir: &Path, home: Option<&Path>) {
        let before = self.entries.len();
        self.push(path, access, data_dir, home);
        if self.entries.len() > before {
            let entry = self.entries[before].clone();
            self.shared.push(entry);
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

/// 런타임 기록에서 데몬이 ALL APPLICATION PACKAGES에 더한 함께 읽는 권한의 키. 런타임(없어도 되는 것)은 쓰지 않게 되면 거두고,
/// 나머지(Python·모듈·익스텐션 폴더)는 제거기(`reset`)만 거둔다
#[cfg_attr(not(windows), allow(dead_code))]
const SHARED_RUNTIMES: &str = "*all-app-packages*";
#[cfg_attr(not(windows), allow(dead_code))]
const SHARED_FIXED: &str = "*all-app-packages-fixed*";

/// 런타임 읽기 권한 기록 (`<data_dir>/sandbox-grants.json`: 프로필 → 폴더)
#[cfg_attr(not(windows), allow(dead_code))]
fn runtime_record_path() -> PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join("sandbox-grants.json")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn read_runtime_record(path: &Path) -> BTreeMap<String, BTreeSet<PathBuf>> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn write_runtime_record(path: &Path, record: &BTreeMap<String, BTreeSet<PathBuf>>) {
    let saved = serde_json::to_vec_pretty(record)
        .map_err(std::io::Error::other)
        .and_then(|bytes| saba_atomic_file::write(path, &bytes, saba_atomic_file::Mode::Plain));
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
pub use imp::{adopt_parent, clear_low_label_of, clear_low_labels, grant, release, reset, revoke_stale_runtimes, spawn, Contained};
#[cfg(windows)]
pub(crate) use imp::check_existing_path;

#[cfg(windows)]
mod imp {
    use super::{Access, Grants};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::ffi::{OsStr, OsString};
    use std::ffi::c_void;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
    use std::path::Path;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{LocalFree, SetHandleInformation, FALSE, HANDLE, HANDLE_FLAG_INHERIT, TRUE};
    use windows_sys::Win32::Security::Authorization::{
        GetSecurityInfo, SetEntriesInAclW, SetNamedSecurityInfoW, SetSecurityInfo, EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
        TRUSTEE_IS_WELL_KNOWN_GROUP,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, AddAce, AddMandatoryAce, AllocateAndInitializeSid, EqualSid, GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation,
        InitializeAcl, InitializeSecurityDescriptor, SetKernelObjectSecurity, SetSecurityDescriptorSacl, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION, ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, INHERITED_ACE, LABEL_SECURITY_INFORMATION, NO_INHERITANCE, OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SECURITY_DESCRIPTOR, SECURITY_CREATOR_SID_AUTHORITY, SECURITY_MANDATORY_LABEL_AUTHORITY,
        SE_DACL_PROTECTED, SID_AND_ATTRIBUTES, SID_IDENTIFIER_AUTHORITY, SUB_CONTAINERS_AND_OBJECTS_INHERIT, UNPROTECTED_DACL_SECURITY_INFORMATION, SYSTEM_MANDATORY_LABEL_ACE, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, OPEN_EXISTING, READ_CONTROL,
        SYNCHRONIZE, WRITE_DAC, WRITE_OWNER,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::SystemServices::{
        ACCESS_ALLOWED_ACE_TYPE, SECURITY_CREATOR_OWNER_RIGHTS_RID, SECURITY_DESCRIPTOR_REVISION, SECURITY_MANDATORY_LOW_RID, SECURITY_MANDATORY_MEDIUM_RID, SE_GROUP_ENABLED,
        SYSTEM_MANDATORY_LABEL_ACE_TYPE, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken,
        ResumeThread, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
        EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };
    #[allow(clippy::upper_case_acronyms)]
    type DWORD = u32;
    #[allow(clippy::upper_case_acronyms)]
    type PACL = *mut ACL;

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
            let create = GetProcAddress(lib, c"CreateAppContainerProfile".as_ptr().cast());
            let derive = GetProcAddress(lib, c"DeriveAppContainerSidFromAppContainerName".as_ptr().cast());
            let (Some(create), Some(derive)) = (create, derive) else {
                return Err("AppContainer is not supported on this Windows".into());
            };
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
        let auth = SID_IDENTIFIER_AUTHORITY { Value: APP_PACKAGE_AUTHORITY };
        let mut sid: PSID = null_mut();
        if unsafe { AllocateAndInitializeSid(&auth, 2, sub0, sub1, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
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
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
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
            if h == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return Err(last_error(&format!("open {}", path.display())));
            }
            let handle = OwnedHandle::from_raw_handle(h as RawHandle);
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(h, &mut info) == 0 {
                return Err(last_error(&format!("inspect {}", path.display())));
            }
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                // 동기화 폴더(OneDrive 등)의 클라우드 파일도 재분석 지점이다. 막는 것은 같지만 원인을 따로 알린다
                match reparse_tag(h) {
                    Some(tag) if tag & 0xFFFF_0FFF == 0x9000_001A => {
                        return Err(format!("{} {}", path.display(), crate::sandbox::SYNC_FOLDER_MARK));
                    }
                    // 이름 대리 표시가 없는 태그(압축 WOF·중복 제거 등)는 다른 곳을 가리키지 않는 같은 파일이다
                    Some(tag) if tag & 0x2000_0000 == 0 => {}
                    _ => return Err(format!("{} is a link", path.display())),
                }
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

    pub(crate) fn check_existing_path(path: &Path) -> Result<(), String> {
        open_checked(path, 0).map(|_| ())
    }

    /// 재분석 지점의 태그 (클라우드 파일 `IO_REPARSE_TAG_CLOUD`는 `0x9000_?01A` 묶음, 링크류는 이름 대리 비트 `0x2000_0000`)
    fn reparse_tag(handle: HANDLE) -> Option<u32> {
        use windows_sys::Win32::Storage::FileSystem::{FileAttributeTagInfo, GetFileInformationByHandleEx};
        // winapi 0.3에 없는 구조체 (fileapi.h FILE_ATTRIBUTE_TAG_INFO)
        #[repr(C)]
        #[allow(non_snake_case)]
        struct FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: DWORD,
            ReparseTag: DWORD,
        }
        unsafe {
            let mut info: FILE_ATTRIBUTE_TAG_INFO = std::mem::zeroed();
            (GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as DWORD,
            ) != 0)
                .then_some(info.ReparseTag)
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
            let rc = SetEntriesInAclW(1, &ea, dacl, &mut new_dacl);
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
            let rc = SetEntriesInAclW(1, &ea, dacl, &mut new_dacl);
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
        integrity_sid(&SID, SECURITY_MANDATORY_LOW_RID as DWORD)
    }

    fn medium_integrity() -> Result<PSID, String> {
        static SID: std::sync::OnceLock<Result<Sid, String>> = std::sync::OnceLock::new();
        integrity_sid(&SID, SECURITY_MANDATORY_MEDIUM_RID as DWORD)
    }

    fn integrity_sid(cell: &'static std::sync::OnceLock<Result<Sid, String>>, rid: DWORD) -> Result<PSID, String> {
        cell.get_or_init(|| {
            let auth = SECURITY_MANDATORY_LABEL_AUTHORITY;
            let mut sid: PSID = null_mut();
            if unsafe { AllocateAndInitializeSid(&auth, 1, rid, 0, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
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
            let auth = SECURITY_CREATOR_SID_AUTHORITY;
            let mut sid: PSID = null_mut();
            if unsafe { AllocateAndInitializeSid(&auth, 1, SECURITY_CREATOR_OWNER_RIGHTS_RID as DWORD, 0, 0, 0, 0, 0, 0, 0, &mut sid) } == 0 {
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
                        header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE as u8
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
        // 붙이기 전부터 있던 파일(가져온 서버의 설정 파일 등)에도 상속 표시를 맞춘다: 없으면 격리 실행이 고치지 못한다
        let failed = sync_inherited_low(path, true);
        if !failed.is_empty() {
            tracing::warn!("[Sandbox] {} existing item(s) under {} did not get the low label (first: {})", failed.len(), path.display(),
                failed.first().map(|(p, e)| format!("{}: {e}", p.display())).unwrap_or_default());
        }
        Ok(())
    }

    /// 폴더에 붙이거나 뗀 낮은 수준 표시를 그 전부터 있던 하위 항목에 맞춘다 (`low`: 상속 표시를 둘지). 고치지 못한 항목
    /// SetSecurityInfo는 DACL과 달리 표시(레이블)를 이미 있는 하위 항목에 퍼뜨리지 않는다(PC 실측): 붙인 뒤에도 원래 파일은
    /// 중간 수준이라 격리 실행이 고치지 못하고, 뗀 뒤에도 하위에 상속 표시가 남는다. 각 항목에 직접 붙은 표시(실행 파일의
    /// 중간 수준 등)는 그대로 두고 상속 표시만 넣거나 뺀다. 링크·정션은 따라가지도 고치지도 않는다
    fn sync_inherited_low(dir: &Path, low: bool) -> Vec<(PathBuf, String)> {
        use std::os::windows::fs::MetadataExt;
        let gone = |p: &Path| matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
        let mut failed = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(path) = pending.pop() {
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    failed.push((path, format!("list: {e}")));
                    continue;
                }
            };
            for entry in entries.flatten() {
                let item = entry.path();
                let attributes = entry.metadata().map(|m| m.file_attributes()).unwrap_or(0);
                let is_dir = match entry.file_type() {
                    Ok(t) if t.is_symlink() || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && reparse_refusal(&item, attributes).is_some() => continue,
                    Ok(t) => t.is_dir(),
                    Err(_) if gone(&item) => continue,
                    Err(e) => {
                        failed.push((item, format!("read the type: {e}")));
                        continue;
                    }
                };
                match set_inherited_low(&item, is_dir, low) {
                    Ok(()) => {}
                    Err(_) if gone(&item) => continue,
                    Err(e) => failed.push((item.clone(), e)),
                }
                if is_dir {
                    pending.push(item);
                }
            }
        }
        failed
    }

    /// 한 항목의 상속받은 낮은 수준 표시를 넣거나(`low`) 뺀다. 다른 표시 ACE는 그대로 둔다. 이미 맞으면 쓰지 않는다.
    /// SetKernelObjectSecurity는 상속 계산 없이 준 그대로 쓴다 (SetSecurityInfo는 상속 표시를 지운다)
    fn set_inherited_low(path: &Path, is_dir: bool, low: bool) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let low_sid = low_integrity()?;
        let handle = open_for_relabel(path)?;
        let h = handle.as_raw_handle() as HANDLE;
        unsafe {
            let mut sacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(h, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), &mut sacl, &mut sd);
            if rc != 0 {
                return Err(format!("read label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            // 남길 ACE(상속받은 낮은 수준 표시 말고 전부)의 원래 바이트
            let mut keep: Vec<Vec<u8>> = Vec::new();
            let mut has_inherited_low = false;
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
                    let inherited_low = header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE as u8
                        && DWORD::from(header.AceFlags) & INHERITED_ACE != 0
                        && EqualSid(&label.SidStart as *const DWORD as PSID, low_sid) != 0;
                    if inherited_low {
                        has_inherited_low = true;
                    } else {
                        keep.push(std::slice::from_raw_parts(ace as *const u8, usize::from(header.AceSize)).to_vec());
                    }
                }
            }
            LocalFree(sd);
            if has_inherited_low == low {
                return Ok(());
            }
            // ACL 머리 + 남길 ACE + 새 표시 ACE(머리·마스크 8바이트 + SID). 8바이트 정렬
            let label_size = 8 + GetLengthSid(low_sid) as usize;
            let size = std::mem::size_of::<ACL>() + keep.iter().map(Vec::len).sum::<usize>() + label_size;
            let mut buf = vec![0u64; size.div_ceil(8)];
            let acl = buf.as_mut_ptr() as *mut ACL;
            if InitializeAcl(acl, (buf.len() * 8) as DWORD, DWORD::from(ACL_REVISION)) == 0 {
                return Err(last_error("InitializeAcl"));
            }
            for ace in &keep {
                if AddAce(acl, DWORD::from(ACL_REVISION), u32::MAX, ace.as_ptr() as *const c_void, ace.len() as DWORD) == 0 {
                    return Err(last_error("AddAce"));
                }
            }
            if low {
                // 위 폴더의 (OI)(CI) 표시를 물려받은 모양: 폴더는 (OI)(CI)(I), 파일은 (I)
                let flags = INHERITED_ACE | if is_dir { (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as DWORD } else { 0 };
                if AddMandatoryAce(acl, DWORD::from(ACL_REVISION), flags, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, low_sid) == 0 {
                    return Err(last_error("AddMandatoryAce"));
                }
            }
            let mut desc: SECURITY_DESCRIPTOR = std::mem::zeroed();
            let desc_ptr = &mut desc as *mut SECURITY_DESCRIPTOR as PSECURITY_DESCRIPTOR;
            if InitializeSecurityDescriptor(desc_ptr, SECURITY_DESCRIPTOR_REVISION) == 0 || SetSecurityDescriptorSacl(desc_ptr, TRUE, acl, FALSE) == 0 {
                return Err(last_error("build the label"));
            }
            if SetKernelObjectSecurity(h, LABEL_SECURITY_INFORMATION, desc_ptr) == 0 {
                return Err(last_error(&format!("write label of {}", path.display())));
            }
        }
        Ok(())
    }

    /// 경로에 걸린 낮은 수준 표시: `None` 없음(또는 앞선 다른 수준 표시), `Some(true)` 위 폴더에서 상속받은 것, `Some(false)` 직접 붙은 것
    fn low_label(path: &Path) -> Result<Option<bool>, String> {
        let low = low_integrity()?;
        Ok(first_label(path, |sid, flags, _| unsafe { EqualSid(sid, low) != 0 }.then_some(flags & super::ACE_INHERITED != 0))?.flatten())
    }

    /// 이 파일에 직접 붙은 중간 수준 표시(`label_medium`이 붙인 모양)가 그 경로의 수준인가
    fn own_medium_label(path: &Path) -> Result<bool, String> {
        let medium = medium_integrity()?;
        Ok(first_label(path, |sid, flags, mask| {
            let same = unsafe { EqualSid(sid, medium) != 0 };
            same && flags & super::ACE_INHERITED == 0 && mask == SYSTEM_MANDATORY_LABEL_NO_WRITE_UP
        })?
        .unwrap_or(false))
    }

    /// 경로의 처음 나오는 표시 ACE(그 경로의 수준. 직접 붙인 표시가 상속받은 표시보다 앞에 온다)를 `f(SID, 플래그, 마스크)`로 본다
    fn first_label<T>(path: &Path, f: impl FnOnce(PSID, u8, u32) -> T) -> Result<Option<T>, String> {
        use std::os::windows::io::AsRawHandle;
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
                    if header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE as u8 {
                        found = Some(f(&label.SidStart as *const DWORD as PSID, header.AceFlags, label.Mask));
                        break;
                    }
                }
            }
            LocalFree(sd);
            Ok(found)
        }
    }

    /// `dir`과 그 아래의 실행 파일(`.exe`·`.com`)에 걸린 낮은 수준 표시를 중간 수준 표시로 덮는다. 낮은 수준 표시가
    /// 걸린 실행 파일로 띄운 프로세스는 낮은 무결성으로 돌아 서버(Java 등)가 뜨지 못하므로, 서버를 띄우기 전에 부른다.
    /// 프로세스 수준은 실행 파일 표시만 따르므로 폴더와 다른 파일의 표시는 그대로 둔다: 폴더 표시를 걷으면 4만 개
    /// 파일에 상속 계산이 퍼지고, 다음 격리 실행이 다시 붙이며 또 퍼진다(첫 시작이 90초 넘게 걸리던 원인).
    /// 덮은 파일 수를 돌려준다. 하나에서 실패해도 나머지를 마저 보고, 낮은 수준이 남았거나 표시를 읽지 못한 실행 파일을
    /// 모아 Err로 돌려준다(읽지 못하면 표시가 남았는지 알 수 없다).
    /// 볼륨이 보안 정보를 두지 않으면(FAT 등) 표시가 있을 수 없으므로 아무것도 하지 않는다
    pub fn clear_low_labels(dir: &Path) -> Result<usize, Vec<(PathBuf, String)>> {
        let _guard = acl_lock();
        walk_executables(dir, clear_one)
    }

    /// 인스턴스를 지우며 폴더의 낮은 수준 표시를 뗀 뒤: 서버를 띄우려고 실행 파일에 직접 붙인 중간 수준 표시(`clear_low_labels`)도
    /// 뗀다. 위 폴더에 표시가 없으면 중간 수준이 기본이라 할 일은 같고, 흔적만 남는다. 뗀 파일 수
    fn drop_medium_labels(dir: &Path) -> Result<usize, Vec<(PathBuf, String)>> {
        walk_executables(dir, unlabel_medium_one)
    }

    /// `dir`과 그 아래(링크는 따라가지 않는다)의 실행 파일마다 `visit(파일, 센 수, 실패)`. 볼륨이 보안 정보를 두지 않으면 0
    fn walk_executables(dir: &Path, mut visit: impl FnMut(&Path, &mut usize, &mut Vec<(PathBuf, String)>)) -> Result<usize, Vec<(PathBuf, String)>> {
        // 실제 경로(`\\?\` 긴 이름)에서 시작한다: 짧은 이름(8.3)이 섞인 경로는 열 때의 경로 확인과 맞지 않는다
        let dir = std::fs::canonicalize(dir).map_err(|e| vec![(dir.to_path_buf(), format!("resolve: {e}"))])?;
        match persistent_acls(&dir) {
            Ok(false) => return Ok(0),
            Ok(true) => {}
            Err(e) => return Err(vec![(dir, e)]),
        }
        let mut cleared = 0;
        let mut failed = Vec::new();
        use std::os::windows::fs::MetadataExt;
        if let Some(e) = std::fs::metadata(&dir).ok().and_then(|m| reparse_refusal(&dir, m.file_attributes())) {
            return Err(vec![(dir, e)]);
        }
        let mut pending = vec![dir];
        while let Some(path) = pending.pop() {
            // 링크는 따라가지 않는다 (폴더 밖을 고치지 않게)
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let attributes = entry.metadata().map(|m| m.file_attributes()).unwrap_or(0);
                        match entry.file_type() {
                            Ok(t) if t.is_symlink() => {}
                            Ok(_) if reparse_refusal(&entry.path(), attributes).map(|e| failed.push((entry.path(), e))).is_some() => {}
                            Ok(t) if t.is_dir() => pending.push(entry.path()),
                            Ok(_) if is_executable(&entry.path()) => visit(&entry.path(), &mut cleared, &mut failed),
                            Ok(_) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => failed.push((entry.path(), format!("read the type: {e}"))),
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // 목록을 읽지 못하면 그 안에 실행 파일이 남았는지 알 수 없다
                Err(e) => failed.push((path.clone(), format!("list: {e}"))),
            }
        }
        if failed.is_empty() {
            Ok(cleared)
        } else {
            Err(failed)
        }
    }

    /// 띄울 파일 하나에 걸린 낮은 수준 표시를 덮는다. 확장자가 `.exe`·`.com`이 아닌 PE도 Windows가 그대로 띄우므로,
    /// 폴더 걷기가 보지 않는 실행 대상 자체를 따로 덮는다 (L38). 덮었으면 true
    pub fn clear_low_label_of(path: &Path) -> Result<bool, Vec<(PathBuf, String)>> {
        let _guard = acl_lock();
        let path = std::fs::canonicalize(path).map_err(|e| vec![(path.to_path_buf(), format!("resolve: {e}"))])?;
        match persistent_acls(&path) {
            Ok(false) => return Ok(false),
            Ok(true) => {}
            Err(e) => return Err(vec![(path, e)]),
        }
        use std::os::windows::fs::MetadataExt;
        if let Some(e) = std::fs::metadata(&path).ok().and_then(|m| reparse_refusal(&path, m.file_attributes())) {
            return Err(vec![(path, e)]);
        }
        let (mut cleared, mut failed) = (0, Vec::new());
        clear_one(&path, &mut cleared, &mut failed);
        if failed.is_empty() {
            Ok(cleared > 0)
        } else {
            Err(failed)
        }
    }

    /// 재분석 지점이 동기화 폴더의 클라우드 파일·폴더(또는 링크)면 막는 까닭. 실행 파일이 아니어도 막는다
    /// (사용자 결정: 동기화 폴더 아래 서버는 지원하지 않는다). 압축(WOF) 같은 다른 재분석 지점은 막지 않고 보통처럼
    /// 다룬다. 목록의 속성으로 거르므로 다른 파일은 열지 않는다. 그 사이 지워졌으면 막지 않는다
    fn reparse_refusal(path: &Path, attributes: u32) -> Option<String> {
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            return None;
        }
        match open_checked(path, 0) {
            Ok(_) => None,
            Err(_) if matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound) => None,
            Err(e) => Some(e),
        }
    }

    /// Windows가 프로그램으로 띄우는 파일 (확장자, 대소문자 무시)
    fn is_executable(path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"))
    }

    /// 한 실행 파일에 걸린 낮은 수준 표시를 중간 수준 표시로 덮고 결과를 센다. 목록을 읽은 뒤에 지워진 경로(업데이트 중
    /// 바뀐 파일 등)는 표시가 남을 곳이 없으므로 실패로 치지 않는다
    fn clear_one(path: &Path, cleared: &mut usize, failed: &mut Vec<(PathBuf, String)>) {
        let gone = || matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
        match low_label(path) {
            Ok(Some(_)) => match label_medium(path).and_then(|()| match low_label(path)? {
                None => Ok(()),
                Some(_) => Err("the low label is still in effect".to_string()),
            }) {
                Ok(()) => *cleared += 1,
                Err(_) if gone() => {}
                Err(e) => failed.push((path.to_path_buf(), e)),
            },
            Ok(None) => {}
            Err(_) if gone() => {}
            Err(e) => failed.push((path.to_path_buf(), format!("read the label: {e}"))),
        }
    }

    /// 한 실행 파일에 직접 붙은 중간 수준 표시를 떼고(빈 표시, 위 폴더에서 다시 물려받는다) 센다. 떼고도 낮은 수준이면 실패다
    fn unlabel_medium_one(path: &Path, removed: &mut usize, failed: &mut Vec<(PathBuf, String)>) {
        use std::os::windows::io::AsRawHandle;
        let gone = || matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
        let unlabel = || -> Result<bool, String> {
            if !own_medium_label(path)? {
                return Ok(false);
            }
            let mut buf = [0u32; 4];
            let acl = buf.as_mut_ptr() as *mut ACL;
            if unsafe { InitializeAcl(acl, std::mem::size_of_val(&buf) as DWORD, DWORD::from(ACL_REVISION)) } == 0 {
                return Err(last_error("InitializeAcl"));
            }
            let handle = open_for_relabel(path)?;
            let rc = unsafe { SetSecurityInfo(handle.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), acl) };
            if rc != 0 {
                return Err(format!("remove label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            match low_label(path)? {
                None => Ok(true),
                Some(_) => Err("a low label came back from the folder".to_string()),
            }
        };
        match unlabel() {
            Ok(true) => *removed += 1,
            Ok(false) => {}
            Err(_) if gone() => {}
            Err(e) => failed.push((path.to_path_buf(), e)),
        }
    }

    /// 경로의 짧은 이름(8.3)을 긴 이름으로 편다. 링크·정션은 따라가지 않는다. 실패하면 그대로
    fn long_path(path: &Path) -> PathBuf {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
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
        use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};
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
        Ok(flags & windows_sys::Win32::System::SystemServices::FILE_PERSISTENT_ACLS != 0)
    }

    /// 폴더에 직접 붙인 낮은 수준 표시(`label_low`)를 떼어 표시가 없던 처음 상태로 둔다 (하위가 물려받은 표시도 뗀다).
    /// 직접 붙은 표시가 없으면 아무것도 하지 않는다. 떼었으면 true
    fn unlabel_low(path: &Path) -> Result<bool, String> {
        use std::os::windows::io::AsRawHandle;
        if low_label(path)? != Some(false) {
            return Ok(false);
        }
        // ACE 없는 ACL 머리만
        let mut buf = [0u32; 4];
        let acl = buf.as_mut_ptr() as *mut ACL;
        unsafe {
            if InitializeAcl(acl, std::mem::size_of_val(&buf) as DWORD, DWORD::from(ACL_REVISION)) == 0 {
                return Err(last_error("InitializeAcl"));
            }
        }
        let handle = open_for_relabel(path)?;
        let rc = unsafe { SetSecurityInfo(handle.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), acl) };
        if rc != 0 {
            return Err(format!("remove label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
        }
        // 하위 항목이 물려받은 표시도 뗀다 (SetSecurityInfo는 표시를 하위로 다시 계산하지 않는다)
        let failed = sync_inherited_low(path, false);
        if let Some((p, e)) = failed.first() {
            return Err(format!("{} item(s) under {} kept the low label (first: {}: {e})", failed.len(), path.display(), p.display()));
        }
        Ok(true)
    }

    /// 이 파일에만 중간 수준 표시를 직접 붙인다. 직접 붙인 표시가 상속받은 표시보다 앞에 와서 그 파일의 수준이 되고,
    /// 위 폴더 표시가 다시 퍼져도 남는다. 상속받은 낮은 수준 표시는 그 뒤에 그대로 있다
    fn label_medium(path: &Path) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let medium = medium_integrity()?;
        // ACL 머리 + 표시 ACE 하나 + SID(하위 권한 1개)
        let mut buf = [0u32; 16];
        let acl = buf.as_mut_ptr() as *mut ACL;
        unsafe {
            if InitializeAcl(acl, std::mem::size_of_val(&buf) as DWORD, DWORD::from(ACL_REVISION)) == 0
                || AddMandatoryAce(acl, DWORD::from(ACL_REVISION), 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, medium) == 0
            {
                return Err(last_error("AddMandatoryAce"));
            }
        }
        let handle = open_for_relabel(path)?;
        let rc = unsafe { SetSecurityInfo(handle.as_raw_handle() as HANDLE, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), acl) };
        if rc != 0 {
            return Err(format!("write label of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
        }
        Ok(())
    }

    #[cfg(test)]
    mod label_tests {
        /// `exe`로 띄운 프로세스의 무결성 수준 RID (0x1000 낮음, 0x2000 중간). 멈춘 채 띄워 보고 바로 끝낸다
        fn launch_integrity(exe: &std::path::Path) -> u32 {
            use super::*;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Security::{TokenIntegrityLevel, TOKEN_MANDATORY_LABEL};
            unsafe {
                let app = wide(exe.as_os_str());
                let mut line = wide(std::ffi::OsStr::new("probe --list"));
                let mut si: windows_sys::Win32::System::Threading::STARTUPINFOW = std::mem::zeroed();
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
                    let count = *windows_sys::Win32::Security::GetSidSubAuthorityCount(label.Label.Sid);
                    *windows_sys::Win32::Security::GetSidSubAuthority(label.Label.Sid, DWORD::from(count) - 1)
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

        /// 다른 위 폴더 아래 있다가 원래 자리로 옮겨 온 폴더(가져오기를 거둔 서버 폴더)는 예전 위 폴더의 권한·낮은 수준 표시를
        /// 남기지 않고 지금 위 폴더에서 다시 물려받는다
        #[test]
        fn a_folder_moved_back_inherits_from_its_new_parent_again() {
            use super::*;
            let tmp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(tmp.path()).unwrap();
            let (instance, home) = (root.join("instance"), root.join("home"));
            std::fs::create_dir_all(instance.join("server").join("Config")).unwrap();
            std::fs::create_dir_all(home.join("fresh")).unwrap();
            std::fs::write(home.join("fresh.ini"), b"x").unwrap();
            let file = instance.join("server").join("Config").join("settings.ini");
            std::fs::write(&file, b"x").unwrap();
            let sid = all_app_packages().unwrap();
            allow(&instance, sid, FILE_GENERIC_READ, true).unwrap();
            label_low(&instance).unwrap();

            let back = home.join("server");
            std::fs::rename(instance.join("server"), &back).unwrap();
            let file = back.join("Config").join("settings.ini");
            // 지금 위 폴더 아래 새로 만든 것이 물려받는 만큼이 기준이다 (임시 폴더 자체의 ACE와 상관없이)
            let fresh = |dir: bool| ace_flags(&home.join(if dir { "fresh" } else { "fresh.ini" }), sid).unwrap().len();
            assert!(ace_flags(&file, sid).unwrap().len() > fresh(false), "the rename kept the old inherited ACE");

            adopt_parent(&back).unwrap();
            for (path, dir) in [(&back, true), (&back.join("Config"), true), (&file, false)] {
                assert_eq!(ace_flags(path, sid).unwrap().len(), fresh(dir), "{}", path.display());
                assert_eq!(low_label(path).unwrap(), low_label(&home.join("fresh.ini")).unwrap(), "{}", path.display());
            }
        }

        /// 폴더를 읽은 뒤 지워진 파일(서버 로그 회전 등) 때문에 시작이 막히지 않는다
        #[test]
        fn a_path_gone_during_the_walk_is_not_a_failure() {
            let tmp = tempfile::tempdir().unwrap();
            let (mut cleared, mut failed) = (0, Vec::new());
            super::clear_one(&tmp.path().join("rotated.log"), &mut cleared, &mut failed);
            assert!(failed.is_empty(), "{failed:?}");
        }

        /// 가져온 서버 폴더: 표시를 붙이기 전부터 있던 파일도 낮은 수준이 되어 격리 실행이 고칠 수 있고,
        /// 표시를 떼면 그 사이 생긴 파일까지 하위의 상속 표시가 모두 사라진다
        #[test]
        fn files_that_were_there_before_follow_the_folder_label_both_ways() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            std::fs::create_dir_all(dir.join("Config")).unwrap();
            let before = dir.join("Config").join("settings.ini");
            std::fs::write(&before, b"x").unwrap();
            super::label_low(&dir).unwrap();
            assert_eq!(super::low_label(&before).unwrap(), Some(true));
            assert_eq!(super::low_label(&dir.join("Config")).unwrap(), Some(true));
            let after = dir.join("Config").join("made-later.json");
            std::fs::write(&after, b"x").unwrap();
            assert_eq!(super::low_label(&after).unwrap(), Some(true), "new files still inherit");

            assert!(super::unlabel_low(&dir).unwrap());
            for path in [&dir, &dir.join("Config"), &before, &after] {
                assert_eq!(super::low_label(path).unwrap(), None, "{}", path.display());
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

        /// 가져온 서버를 빼면(`release`) 서버를 띄우려고 실행 파일에 붙인 중간 수준 표시도 남지 않는다
        #[test]
        fn releasing_a_folder_leaves_no_label_on_its_executables() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            let exe = dir.join("bin").join("server.exe");
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"x").unwrap();
            super::label_low(&dir).unwrap();
            super::clear_low_labels(&dir).unwrap();
            assert!(super::own_medium_label(&exe).unwrap());

            super::release(&[], &[dir.clone()]).unwrap();
            assert!(!super::own_medium_label(&exe).unwrap());
            assert_eq!(super::low_label(&exe).unwrap(), None);
        }

        /// 위 폴더가 낮은 수준 표시를 물려주면(다른 격리 폴더 안의 서버) 빼도 실행 파일의 중간 수준 표시를 남긴다:
        /// 떼면 서버가 낮은 무결성으로 떠 뜨지 못한다
        #[test]
        fn releasing_a_folder_under_a_low_folder_keeps_its_executables_at_medium() {
            let tmp = tempfile::tempdir().unwrap();
            let outer = std::fs::canonicalize(tmp.path()).unwrap();
            let dir = outer.join("server");
            let exe = dir.join("server.exe");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&exe, b"x").unwrap();
            super::label_low(&outer).unwrap();
            super::clear_low_labels(&dir).unwrap();
            assert!(super::own_medium_label(&exe).unwrap());

            super::release(&[], &[dir.clone()]).unwrap();
            assert!(super::own_medium_label(&exe).unwrap());
            assert_eq!(super::low_label(&exe).unwrap(), None, "the medium label still decides the level");
        }

        /// L38: 확장자가 `.exe`가 아닌 실행 대상(PE)은 폴더 걷기가 보지 않는다. 그 파일을 띄우기 전에 따로 덮는다
        #[test]
        fn a_launch_target_without_an_exe_extension_also_runs_at_medium_integrity() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            super::label_low(&dir).unwrap();
            let target = dir.join("probe.bin");
            std::fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 0, "the walk only looks at .exe and .com");
            assert_eq!(launch_integrity(&target), 0x1000);
            assert!(super::clear_low_label_of(&target).unwrap());
            assert!(launch_integrity(&target) >= 0x2000);
            assert!(!super::clear_low_label_of(&target).unwrap(), "nothing left to clear");
        }

        /// 격리 실행에 쓰기를 준 폴더(낮은 수준 표시)에 받은 서버 실행 파일은, 서버를 띄우기 전에 중간 수준이 된다.
        /// 폴더와 다른 파일의 표시는 그대로 둔다: 다음 격리 실행이 4만 개 파일에 표시를 다시 퍼뜨리지 않고(첫 시작 90초),
        /// 모듈은 서버 설정 파일을 계속 쓸 수 있다
        #[test]
        fn only_executables_lose_the_low_label_before_launch() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            let bin = dir.join("server").join("java").join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            super::label_low(&dir).unwrap();
            super::label_low(&dir.join("server")).unwrap();
            let exe = bin.join("java.exe");
            std::fs::write(&exe, b"x").unwrap();
            let ini = dir.join("server").join("server.ini");
            std::fs::write(&ini, b"x").unwrap();
            assert_eq!(super::low_label(&exe).unwrap(), Some(true));
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 1);
            assert_eq!(super::low_label(&exe).unwrap(), None);
            assert_eq!(super::low_label(&ini).unwrap(), Some(true), "the module still writes its settings");
            assert_eq!(super::low_label(&dir).unwrap(), Some(false), "the folder keeps its label");
            // 폴더 표시가 다시 하위로 퍼져도(다른 데몬·도구가 다시 붙임) 실행 파일은 중간 수준 그대로다
            let out = std::process::Command::new(saba_chan_updater_lib::process::system_tool("icacls")).arg(&dir).args(["/setintegritylevel", "(OI)(CI)L"]).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
            assert_eq!(super::low_label(&ini).unwrap(), Some(true));
            assert_eq!(super::low_label(&exe).unwrap(), None);
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 0, "nothing left to clear");
        }

        // 함께 읽는 폴더(Python 등)에 구성요소마다 ACE를 더하면 DACL이 64KB 한도에 닿아 "os error 87"로 격리 실행이
        // 모두 멈췄다(이 PC에서 1,363개). 그 폴더는 ALL APPLICATION PACKAGES에 한 번만 준다
        #[test]
        fn a_shared_folder_gets_one_ace_whatever_the_number_of_components() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            let mut grants = super::Grants::default();
            grants.entries.push((dir.clone(), super::Access::Read));
            grants.shared.push((dir.clone(), super::Access::Read));
            for name in ["saba-chan.untrusted.test-shared-a", "saba-chan.untrusted.test-shared-b"] {
                super::apply(name, &grants).unwrap();
                let sid = super::container_sid(name).unwrap();
                assert!(!super::has_own_ace(&dir, sid).unwrap(), "{name} gets no ACE of its own");
                let _ = super::delete_profile(name);
            }
            assert!(super::has_own_ace(&dir, super::all_app_packages().unwrap()).unwrap(), "one ACE for every component");
        }

        // 압축된 게임 폴더(compact /exe, WOF)의 파일도 재분석 지점이지만 링크가 아니다: 막지 않고, 실행 파일은 덮는다
        #[test]
        fn compressed_server_files_do_not_block_the_start() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = std::fs::canonicalize(tmp.path()).unwrap();
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            super::label_low(&dir).unwrap();
            let exe = bin.join("server.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
            std::fs::write(dir.join("data.pak"), vec![0u8; 1 << 20]).unwrap();
            // compact는 `\\?\` 경로를 받지 않는다: 실행 파일 하나를 보통 경로로 넘긴다
            let plain_exe = tmp.path().join("bin").join("server.exe");
            let out = std::process::Command::new(saba_chan_updater_lib::process::system_tool("compact")).args(["/c", "/exe:xpress4k"]).arg(&plain_exe).output().unwrap();
            let said = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(out.status.success(), "{said}");
            // WOF 필터가 붙어 있으면 재분석 지점 속성은 어떤 열기로도 보이지 않는다(CI 러너에서 확인). 그래도 압축된
            // 실행 파일의 표시를 걷고 서버가 중간 수준으로 뜨는지를 본다 (필터가 없는 PC에서는 재분석 지점 그대로 지난다)
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 1);
            assert_eq!(super::low_label(&exe).unwrap(), None);
            assert!(launch_integrity(&exe) >= 0x2000, "a compressed server starts at the daemon's level");
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
            let made = std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd"))
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
                let out = std::process::Command::new(saba_chan_updater_lib::process::system_tool("icacls")).arg(path).args(args).output().unwrap();
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
            assert!(super::open_checked(&exe, windows_sys::Win32::Storage::FileSystem::WRITE_OWNER).is_err(), "the file must not grant WRITE_OWNER, as on a real PC");
            assert_eq!(launch_integrity(&exe), 0x1000);
            assert_eq!(super::clear_low_labels(&dir).unwrap(), 1);
            assert_eq!(super::low_label(&exe).unwrap(), None);
            for file in [&bin.join("jli.dll"), &server.join("server.jar")] {
                assert_eq!(super::low_label(file).unwrap(), Some(true), "{}", file.display());
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

    /// 이 실행에서 기록할 권한: 프로필의 런타임 읽기, 데몬이 모든 앱 패키지에 더한 함께 읽는 권한
    fn record_grants(profile: &str, runtimes: Vec<PathBuf>, shared_added: Vec<(PathBuf, bool)>) {
        if !runtimes.is_empty() || !shared_added.is_empty() {
            let file = super::runtime_record_path();
            let mut record = super::read_runtime_record(&file);
            let mut changed = false;
            let mut add = |key: &str, paths: Vec<PathBuf>| {
                let known = record.entry(key.to_string()).or_default();
                let before = known.len();
                known.extend(paths);
                changed |= known.len() != before;
            };
            add(profile, runtimes);
            let (shared_runtimes, shared_fixed): (Vec<_>, Vec<_>) = shared_added.into_iter().partition(|(_, optional)| *optional);
            add(super::SHARED_RUNTIMES, shared_runtimes.into_iter().map(|(p, _)| p).collect());
            add(super::SHARED_FIXED, shared_fixed.into_iter().map(|(p, _)| p).collect());
            record.retain(|_, paths| !paths.is_empty());
            if changed {
                super::write_runtime_record(&file, &record);
            }
        }
    }

    fn apply(profile: &str, grants: &Grants) -> Result<PSID, String> {
        let _guard = acl_lock();
        let sid = container_sid(profile)?;
        let everyone = all_app_packages()?;
        let mut runtimes = Vec::new();
        let mut shared_added: Vec<(PathBuf, bool)> = Vec::new();
        for (path, access) in &grants.entries {
            // 상속되는 권한·표시는 하위 파일 전부에 퍼진다: 큰 폴더는 처음 한 번 오래 걸릴 수 있어 남긴다
            let started = std::time::Instant::now();
            let _slow = SlowGrant { path, access: *access, started };
            let optional = grants.optional.contains(&(path.clone(), *access));
            let shared = grants.shared.contains(&(path.clone(), *access));
            let existing = match grant_one(path, if shared { everyone } else { sid }, everyone, *access) {
                Ok(existing) => existing,
                Err(e) if optional => {
                    tracing::warn!("[Sandbox] skipping {}: {}", path.display(), e);
                    continue;
                }
                Err(e) => {
                    // 앞에서 더한 권한은 기록해 둔다 (거둘 수 있게)
                    record_grants(profile, std::mem::take(&mut runtimes), std::mem::take(&mut shared_added));
                    return Err(e);
                }
            };
            // 없어도 되는 읽기 권한(런타임 폴더)은 이 SID의 명시 ACE(지금 더한 것, 예전 데몬이 더한 것)를 기록해 두었다가
            // 쓰지 않게 되면 거둔다. 상속받았거나 모든 앱 패키지로 읽을 수 있는 폴더는 이 폴더에서 거둘 것이 없다
            // 함께 쓰는 폴더의 권한은 한 구성요소가 거두면 다른 구성요소도 잃으므로 기록하지 않는다
            // 그 대신 데몬이 모든 앱 패키지에 더한 것은 따로 기록해, 쓰지 않게 된 런타임이나 제거기(`reset`)에서 거둔다
            if shared && existing == super::Existing::None {
                shared_added.push((path.clone(), optional));
            }
            if optional && !shared && *access == Access::Read {
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
        record_grants(profile, runtimes, shared_added);
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
            // 함께 읽는 고정 폴더는 런타임이 아니라 여기서 거두지 않는다 (제거기만)
            if profile == super::SHARED_FIXED {
                continue;
            }
            let sid = match if profile == super::SHARED_RUNTIMES { all_app_packages() } else { container_sid(profile) } {
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

    /// 제거기용 (`saba-core sandbox reset`): `labeled` 폴더의 낮은 무결성 표시를 덮고, 사바쨩 프로필마다
    /// `targets`와 기록된 런타임 폴더에서 그 SID의 명시 ACE를 거둔 뒤 프로필을 지운다
    /// (프로필 폴더 `%LOCALAPPDATA%\Packages\<이름>`과 레지스트리 매핑도 함께 지워진다). 지운 프로필 수
    pub fn reset(targets: &[PathBuf], labeled: &[PathBuf]) -> Result<usize, Vec<String>> {
        let mut failed: Vec<String> = Vec::new();
        for dir in labeled.iter().filter(|d| d.is_dir()) {
            if let Err(errs) = clear_low_labels(dir) {
                failed.extend(errs.into_iter().map(|(p, e)| format!("{}: {}", p.display(), e)));
            }
        }
        let _guard = acl_lock();
        let file = super::runtime_record_path();
        let record = super::read_runtime_record(&file);
        let profiles = saba_profiles();
        for profile in &profiles {
            let sid = match container_sid(profile) {
                Ok(sid) => sid,
                Err(e) => {
                    failed.push(format!("{}: {}", profile, e));
                    continue;
                }
            };
            let recorded = record.get(profile).into_iter().flatten();
            let failed_before = failed.len();
            for path in targets.iter().chain(recorded).filter(|p| p.exists()) {
                // 이 SID의 명시 ACE가 있는 폴더만 고친다 (남의 폴더 DACL을 괜히 다시 쓰지 않게)
                match has_own_ace(path, sid) {
                    Ok(false) => {}
                    Ok(true) => {
                        if let Err(e) = remove_ace(path, sid) {
                            failed.push(e);
                        }
                    }
                    Err(e) => failed.push(e),
                }
            }
            // 거두지 못한 ACE가 있으면 프로필을 남긴다: 다음 reset은 남은 프로필로 다시 찾는다
            if failed.len() > failed_before {
                continue;
            }
            if let Err(e) = delete_profile(profile) {
                failed.push(e);
            }
        }
        // 데몬이 모든 앱 패키지에 더한 함께 읽는 권한도 거둔다 (기록된 것만: 원래 있던 권한은 두고)
        let shared = [super::SHARED_RUNTIMES, super::SHARED_FIXED].into_iter().flat_map(|key| record.get(key).into_iter().flatten());
        match all_app_packages() {
            Ok(everyone) => {
                for path in shared.filter(|p| p.exists()) {
                    match has_own_ace(path, everyone) {
                        Ok(false) => {}
                        Ok(true) => {
                            if let Err(e) = remove_ace(path, everyone) {
                                failed.push(e);
                            }
                        }
                        Err(e) => failed.push(e),
                    }
                }
            }
            Err(e) => failed.push(e),
        }
        if failed.is_empty() {
            let _ = std::fs::remove_file(&file);
            Ok(profiles.len())
        } else {
            Err(failed)
        }
    }

    /// 인스턴스를 지울 때 (`super::release`): `labeled` 폴더의 낮은 무결성 표시를 덮고, 사바쨩 프로필마다
    /// `targets`에서 그 SID의 명시 ACE를 거둔다. 프로필과 런타임 기록은 그대로 둔다
    pub fn release(targets: &[PathBuf], labeled: &[PathBuf]) -> Result<(), Vec<String>> {
        let mut failed: Vec<String> = Vec::new();
        for dir in labeled.iter().filter(|d| d.is_dir()) {
            if let Err(errs) = clear_low_labels(dir) {
                failed.extend(errs.into_iter().map(|(p, e)| format!("{}: {}", p.display(), e)));
            }
            let _guard = acl_lock();
            if let Err(e) = std::fs::canonicalize(dir).map_err(|e| format!("{}: resolve: {e}", dir.display())).and_then(|d| unlabel_low(&d)) {
                failed.push(e);
                continue;
            }
            // 폴더 표시를 뗀 뒤에야 뗄 수 있다 (먼저 떼면 실행 파일이 낮은 수준을 다시 물려받는다).
            // 위 폴더에서 물려받은 낮은 수준 표시가 남아 있으면 중간 수준 표시가 서버를 중간으로 띄우는 것이라 둔다
            match std::fs::canonicalize(dir).map_err(|e| format!("{}: resolve: {e}", dir.display())).and_then(|d| low_label(&d)) {
                Ok(None) => {}
                Ok(Some(_)) => continue,
                Err(e) => {
                    failed.push(e);
                    continue;
                }
            }
            if let Err(errs) = drop_medium_labels(dir) {
                failed.extend(errs.into_iter().map(|(p, e)| format!("{}: {}", p.display(), e)));
            }
        }
        let _guard = acl_lock();
        for profile in saba_profiles() {
            let sid = match container_sid(&profile) {
                Ok(sid) => sid,
                Err(e) => {
                    failed.push(format!("{}: {}", profile, e));
                    continue;
                }
            };
            for path in targets.iter().filter(|p| p.exists()) {
                match has_own_ace(path, sid) {
                    Ok(false) => {}
                    Ok(true) => {
                        if let Err(e) = remove_ace(path, sid) {
                            failed.push(e);
                        }
                    }
                    Err(e) => failed.push(e),
                }
            }
        }
        if failed.is_empty() { Ok(()) } else { Err(failed) }
    }

    /// 이 사용자의 AppContainer 매핑 중 사바쨩 프로필 이름 (`HKCU\...\AppContainer\Mappings\<SID>`의 `Moniker`)
    fn saba_profiles() -> Vec<String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        const MAPPINGS: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings";
        let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(MAPPINGS) else { return Vec::new() };
        let mut names: Vec<String> = root
            .enum_keys()
            .flatten()
            .filter_map(|sid| root.open_subkey(&sid).ok()?.get_value::<String, _>("Moniker").ok())
            .filter(|m| super::is_saba_profile(m))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// 폴더 DACL에 `sid`의 명시(상속받지 않은) ACE가 있는가
    fn has_own_ace(path: &Path, sid: PSID) -> Result<bool, String> {
        Ok(ace_flags(path, sid)?.iter().any(|flags| flags & super::ACE_INHERITED == 0))
    }

    /// 경로 DACL에서 `sid` 허용 ACE들의 플래그 (상속받은 것 포함)
    fn ace_flags(path: &Path, sid: PSID) -> Result<Vec<u8>, String> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            let dir = open_checked(path, 0)?;
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(dir.as_raw_handle() as HANDLE, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                return Err(format!("read ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let mut found = Vec::new();
            let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            if !dacl.is_null()
                && GetAclInformation(dacl, &mut info as *mut _ as *mut c_void, std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD, AclSizeInformation) != 0
            {
                for i in 0..info.AceCount {
                    let mut ace: *mut c_void = null_mut();
                    if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                        continue;
                    }
                    let header = &*(ace as *const ACE_HEADER);
                    if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
                        continue;
                    }
                    let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
                    if EqualSid(&allowed.SidStart as *const DWORD as PSID, sid) != 0 {
                        found.push(header.AceFlags);
                    }
                }
            }
            LocalFree(sd);
            Ok(found)
        }
    }

    /// 다른 위 폴더 아래 있다가 옮겨 온 폴더가 지금 위 폴더에서 다시 물려받게 한다. 같은 볼륨 안 이름 바꾸기는 예전 위 폴더에서
    /// 물려받은 ACE·낮은 수준 표시를 그대로 들고 오고 원래 자리의 상속 ACE는 잃는다. 직접 붙은 ACE만 남긴 DACL을 상속을 열어
    /// 경로로 쓰면(SetNamedSecurityInfoW) 지금 위 폴더의 상속 ACE가 하위까지 다시 계산된다(핸들로 쓰면 위 폴더를 보지 않는다).
    /// 상속을 끊은 폴더(보호 DACL)는 DACL을 그대로 둔다. 낮은 수준 표시는 지금 위 폴더에 있을 때만 물려받은 모양으로 둔다
    pub fn adopt_parent(path: &Path) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let _guard = acl_lock();
        unsafe {
            let read = open_checked(path, 0)?;
            let mut dacl: PACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let rc = GetSecurityInfo(read.as_raw_handle() as HANDLE, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                return Err(format!("read ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
            }
            let (mut control, mut revision): (u16, u32) = (0, 0);
            let protected = GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0 && control & SE_DACL_PROTECTED != 0;
            let mut keep: Vec<Vec<u8>> = Vec::new();
            let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            if !dacl.is_null()
                && GetAclInformation(dacl, &mut info as *mut _ as *mut c_void, std::mem::size_of::<ACL_SIZE_INFORMATION>() as DWORD, AclSizeInformation) != 0
            {
                for i in 0..info.AceCount {
                    let mut ace: *mut c_void = null_mut();
                    if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                        continue;
                    }
                    let header = &*(ace as *const ACE_HEADER);
                    if DWORD::from(header.AceFlags) & INHERITED_ACE == 0 {
                        keep.push(std::slice::from_raw_parts(ace as *const u8, usize::from(header.AceSize)).to_vec());
                    }
                }
            }
            LocalFree(sd);
            if !protected {
                let size = std::mem::size_of::<ACL>() + keep.iter().map(Vec::len).sum::<usize>();
                let mut buf = vec![0u64; size.div_ceil(8)];
                let acl = buf.as_mut_ptr() as *mut ACL;
                if InitializeAcl(acl, (buf.len() * 8) as DWORD, DWORD::from(ACL_REVISION)) == 0 {
                    return Err(last_error("InitializeAcl"));
                }
                for ace in &keep {
                    if AddAce(acl, DWORD::from(ACL_REVISION), u32::MAX, ace.as_ptr() as *const c_void, ace.len() as DWORD) == 0 {
                        return Err(last_error("AddAce"));
                    }
                }
                let name = wide(path.as_os_str());
                let rc = SetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION, null_mut(), null_mut(), acl, null_mut());
                if rc != 0 {
                    return Err(format!("inherit ACL of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32)));
                }
            }
            drop(read);
        }
        let low = match path.parent() {
            Some(parent) => low_label(parent)?.is_some(),
            None => false,
        };
        set_inherited_low(path, true, low)?;
        let failed = sync_inherited_low(path, low);
        if let Some((p, e)) = failed.first() {
            return Err(format!("{} item(s) under {} kept the old label (first: {}: {e})", failed.len(), path.display(), p.display()));
        }
        Ok(())
    }

    /// AppContainer 프로필을 지운다 (userenv `DeleteAppContainerProfile`. 없으면 이미 지워진 것)
    fn delete_profile(profile: &str) -> Result<(), String> {
        type DeleteProfile = unsafe extern "system" fn(*const u16) -> i32;
        unsafe {
            let lib = LoadLibraryW(wide(OsStr::new("userenv.dll")).as_ptr());
            if lib.is_null() {
                return Err(last_error("userenv.dll"));
            }
            let Some(delete) = GetProcAddress(lib, c"DeleteAppContainerProfile".as_ptr().cast()) else {
                return Err("AppContainer is not supported on this Windows".into());
            };
            let delete: DeleteProfile = std::mem::transmute(delete);
            let name = wide(OsStr::new(profile));
            let hr = delete(name.as_ptr());
            if hr < 0 {
                return Err(format!("delete AppContainer profile {}: 0x{:08x}", profile, hr as u32));
            }
        }
        Ok(())
    }

    fn pipe() -> Result<(OwnedHandle, OwnedHandle), String> {
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD, lpSecurityDescriptor: null_mut(), bInheritHandle: TRUE };
        let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
        if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
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
            let mut capability = SID_AND_ATTRIBUTES { Sid: internet, Attributes: SE_GROUP_ENABLED as DWORD };
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
                &si.StartupInfo,
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
