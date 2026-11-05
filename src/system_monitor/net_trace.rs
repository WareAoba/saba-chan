//! 포트·서버별 네트워크 (시스템 모니터 고급 옵션, 기본 꺼짐. Windows만)
//!
//! Windows는 프로세스·포트별 네트워크 양을 알려 주는 카운터가 없다. 커널 네트워크 ETW 공급자
//! (Microsoft-Windows-Kernel-Network)의 실시간 세션을 열어 패킷 이벤트마다 (PID, 로컬 포트, 바이트)를 더한다.
//! - 권한: 이 세션은 관리자, "Performance Log Users" 그룹(S-1-5-32-559), 시스템 서비스만 열 수 있다
//!   (Microsoft StartTrace 문서). 데몬은 보통 권한이므로, 옵션을 켤 때 관리자 확인 한 번으로 지금 사용자를
//!   그 그룹에 넣는다(`saba-core net-trace grant <SID>`). 그룹은 다음 Windows 로그인부터 토큰에 들어간다.
//! - 부하: 옵션이 켜져 있고 누군가 모니터를 펼쳐 둔 동안(`sizes=true` 요청)만 세션을 연다.
//!   `IDLE_STOP` 동안 펼친 요청이 없으면 닫는다. 실측(2026-11-06, 16코어 PC, 초당 이벤트 약 1,450개):
//!   코어 하나 기준 CPU 0.08%.
//! - 주소·내용은 남기지 않는다. (PID, 프로토콜, 로컬 포트)별 바이트 합만 들고 있다가 표본을 뜰 때 비운다.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::port_check::{Protocol, PublicPort};

/// 펼친 요청이 이만큼 없으면 세션을 닫는다
const IDLE_STOP: Duration = Duration::from_secs(10);
/// 세션을 못 열었으면 이만큼 지나야 다시 시도한다 (권한이 없는데 매초 열어 보지 않게)
const RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum Dir {
    Rx,
    Tx,
}

/// Kernel-Network 이벤트 id → (프로토콜, 방향). 데이터가 오간 이벤트만 센다.
/// 10/11 TCP IPv4 보냄/받음, 26/27 TCP IPv6, 42/43 UDP IPv4, 58/59 UDP IPv6 (IPv6는 IPv4 + 16).
/// 10·11·42·43·58은 PC 실측으로 이름("TCP Data sent" 등)과 필드를 확인했다
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn classify(event_id: u16) -> Option<(Protocol, Dir)> {
    match event_id {
        10 | 26 => Some((Protocol::Tcp, Dir::Tx)),
        11 | 27 => Some((Protocol::Tcp, Dir::Rx)),
        42 | 58 => Some((Protocol::Udp, Dir::Tx)),
        43 | 59 => Some((Protocol::Udp, Dir::Rx)),
        _ => None,
    }
}

type Key = (u32, Protocol, u16);
/// 센 바이트: (PID, 프로토콜, 로컬 포트) → (받은, 보낸)
pub(crate) type Counts = HashMap<Key, (u64, u64)>;
/// `NetTrace::take` 결과: 센 바이트와 그 구간, 아니면 `NetTraceMetrics.status`
pub(crate) type Taken = Result<(Counts, Duration), &'static str>;

/// (PID, 프로토콜, 로컬 포트) → (받은, 보낸) 바이트
#[derive(Debug, Default)]
pub(crate) struct NetCounters {
    map: HashMap<Key, (u64, u64)>,
}

impl NetCounters {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn add(&mut self, pid: u32, proto: Protocol, port: u16, dir: Dir, bytes: u64) {
        let e = self.map.entry((pid, proto, port)).or_default();
        match dir {
            Dir::Rx => e.0 += bytes,
            Dir::Tx => e.1 += bytes,
        }
    }

    fn take(&mut self) -> Counts {
        std::mem::take(&mut self.map)
    }
}

/// 서버가 밖에 여는 포트 하나의 사용량
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PortTraffic {
    pub instance_id: String,
    /// 모듈 설정 필드 이름 (예: "port", "rcon_port")
    pub field: String,
    pub port: u16,
    pub protocol: Protocol,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// `MetricsSnapshot.net_trace`
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NetTraceMetrics {
    /// "off"(옵션 꺼짐) | "idle"(펼친 화면이 아니라 세지 않음) | "on" | "unsupported"(Windows 아님)
    /// | "needs_grant"(권한 없음) | "needs_relogin"(그룹에는 들었고 다시 로그인하면 됨) | "error"
    pub status: &'static str,
    /// 아래 값을 잰 구간 (on일 때만 의미가 있다)
    pub window_ms: u64,
    /// ETW로 센 기계 전체 (루프백 포함이라 네트워크 카드 값과 다를 수 있다)
    pub rx_bps: u64,
    pub tx_bps: u64,
    /// 인스턴스가 아닌 프로세스 몫
    pub others_rx_bps: u64,
    pub others_tx_bps: u64,
    pub ports: Vec<PortTraffic>,
}

impl NetTraceMetrics {
    pub fn status(status: &'static str) -> Self {
        Self {
            status,
            window_ms: 0,
            rx_bps: 0,
            tx_bps: 0,
            others_rx_bps: 0,
            others_tx_bps: 0,
            ports: Vec::new(),
        }
    }
}

/// 선언한 포트 하나에 오간 바이트
#[derive(Debug, PartialEq)]
pub(crate) struct PortBytes {
    pub instance_id: String,
    pub field: String,
    pub port: u16,
    pub protocol: Protocol,
    pub rx: u64,
    pub tx: u64,
}

/// 나눈 결과: 인스턴스 id → (받은, 보낸) 바이트, 포트별 바이트, 나머지
#[derive(Debug, Default, PartialEq)]
pub(crate) struct NetSplit {
    pub by_instance: HashMap<String, (u64, u64)>,
    pub ports: Vec<PortBytes>,
    pub others: (u64, u64),
}

/// 센 바이트를 인스턴스(그 PID 트리)와 그 인스턴스가 선언한 포트로 나눈다.
/// `owner`: PID → 인스턴스 id (`attribute`가 만든 것). 거기 없는 PID(이미 끝난 프로세스 포함)는 나머지
pub(crate) fn split(counts: &Counts, owner: &HashMap<u32, String>, ports: &[(String, Vec<PublicPort>)]) -> NetSplit {
    let mut out = NetSplit::default();
    let mut port_bytes: HashMap<(&str, Protocol, u16), (u64, u64)> = HashMap::new();
    for (&(pid, proto, port), &(rx, tx)) in counts {
        match owner.get(&pid) {
            Some(id) => {
                let e = out.by_instance.entry(id.clone()).or_default();
                e.0 += rx;
                e.1 += tx;
                let p = port_bytes.entry((id.as_str(), proto, port)).or_default();
                p.0 += rx;
                p.1 += tx;
            }
            None => {
                out.others.0 += rx;
                out.others.1 += tx;
            }
        }
    }
    for (id, declared) in ports {
        for p in declared {
            let (rx, tx) = port_bytes
                .get(&(id.as_str(), p.protocol, p.port))
                .copied()
                .unwrap_or_default();
            out.ports.push(PortBytes {
                instance_id: id.clone(),
                field: p.field.clone(),
                port: p.port,
                protocol: p.protocol,
                rx,
                tx,
            });
        }
    }
    out
}

/// 권한 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// 지금 토큰으로 세션을 열 수 있다 (관리자이거나 그룹이 토큰에 있다)
    Ok,
    /// 그룹에 들었지만 지금 로그인 토큰에는 아직 없다
    NeedsRelogin,
    NeedsGrant,
    Unsupported,
}

impl Access {
    /// `admin`: 관리자로 돌고 있다. `group_in_token`: 그룹이 지금 토큰에 있다. `member`: 계정이 그룹에 들었다.
    /// `provider`: 공급자 보안 설정에 이 계정 허용이 있는지 (못 읽으면 None: 세션을 열어 보고 판단한다)
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn from_parts(admin: bool, group_in_token: bool, member: bool, provider: Option<bool>) -> Self {
        if admin {
            return Access::Ok;
        }
        match (provider, group_in_token, member) {
            (Some(false), _, _) => Access::NeedsGrant,
            (_, true, _) => Access::Ok,
            (_, false, true) => Access::NeedsRelogin,
            (_, false, false) => Access::NeedsGrant,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Access::Ok => "ok",
            Access::NeedsRelogin => "needs_relogin",
            Access::NeedsGrant => "needs_grant",
            Access::Unsupported => "unsupported",
        }
    }
}

/// 공급자 보안 설정(SDDL)에서 그 SID에 준 허용 ACE를 뺀다 (되돌리기). 다른 ACE는 그대로 둔다
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn strip_sid_aces(sddl: &str, sid: &str) -> String {
    let mut out = String::with_capacity(sddl.len());
    let mut rest = sddl;
    while let Some(start) = rest.find('(') {
        let Some(len) = rest[start..].find(')') else { break };
        let ace = &rest[start + 1..start + len];
        out.push_str(&rest[..start]);
        let ours = ace.starts_with("A;") && ace.rsplit(';').next().is_some_and(|s| s.eq_ignore_ascii_case(sid));
        if !ours {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// SDDL에 그 SID의 허용 ACE가 있는지
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn sddl_allows(sddl: &str, sid: &str) -> bool {
    sddl.split('(').skip(1).filter_map(|a| a.split(')').next()).any(|ace| {
        let parts: Vec<&str> = ace.split(';').collect();
        parts.first() == Some(&"A") && parts.last().is_some_and(|s| s.eq_ignore_ascii_case(sid))
    })
}

/// 지금 사용자의 권한 상태
pub fn access() -> Access {
    platform::access()
}

/// 세션을 여는 데 실패한 까닭
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum StartError {
    Denied,
    Unsupported,
    Other(String),
}

struct State {
    session: Option<platform::Session>,
    counters: Arc<Mutex<NetCounters>>,
    last_want: Option<Instant>,
    last_take: Instant,
    failed: Option<(Instant, &'static str)>,
}

/// 세션 하나를 들고 있는 수집기. 복제해도 같은 세션을 나눠 쓴다
#[derive(Clone)]
pub(crate) struct NetTrace {
    state: Arc<Mutex<State>>,
}

impl Default for NetTrace {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                session: None,
                counters: Arc::default(),
                last_want: None,
                last_take: Instant::now(),
                failed: None,
            })),
        }
    }
}

/// 세션을 계속 둘지: 마지막 펼친 요청 뒤 `IDLE_STOP`이 지나지 않았으면 둔다
pub(crate) fn keep_running(last_want: Option<Instant>, now: Instant) -> bool {
    last_want.is_some_and(|t| now.duration_since(t) < IDLE_STOP)
}

impl NetTrace {
    /// 표본 하나. `enabled`: 옵션, `want`: 펼친 화면의 요청인지.
    /// 세고 있으면 지난번부터 센 바이트와 그 구간을 준다. 아니면 상태 문자열
    pub(crate) fn take(&self, enabled: bool, want: bool) -> Taken {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if !enabled {
            st.session = None;
            st.last_want = None;
            return Err("off");
        }
        if want {
            st.last_want = Some(now);
        }
        if !keep_running(st.last_want, now) {
            st.session = None;
            return Err("idle");
        }
        if st.session.is_none() {
            if let Some((at, why)) = st.failed
                && now.duration_since(at) < RETRY_AFTER
            {
                return Err(why);
            }
            st.counters.lock().unwrap_or_else(|e| e.into_inner()).take();
            match platform::start(st.counters.clone()) {
                Ok(s) => {
                    st.session = Some(s);
                    st.failed = None;
                    st.last_take = now;
                    self.spawn_idle_stopper();
                }
                Err(e) => {
                    let why = match e {
                        StartError::Unsupported => "unsupported",
                        StartError::Denied => match access() {
                            Access::NeedsRelogin => "needs_relogin",
                            _ => "needs_grant",
                        },
                        StartError::Other(detail) => {
                            tracing::warn!("net trace: {}", detail);
                            "error"
                        }
                    };
                    st.failed = Some((now, why));
                    return Err(why);
                }
            }
        }
        let window = now.duration_since(st.last_take);
        st.last_take = now;
        let counts = st.counters.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok((counts, window))
    }

    /// 권한을 바꾼 뒤 바로 다시 시도하게 실패 기록을 지운다
    pub(crate) fn forget_failure(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).failed = None;
    }

    /// 세션을 열면 하나 띄운다. 펼친 요청이 끊기면 세션을 닫고 끝난다
    fn spawn_idle_stopper(&self) {
        let weak = Arc::downgrade(&self.state);
        std::thread::spawn(move || loop {
            std::thread::sleep(IDLE_STOP / 2);
            let Some(state) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            if st.session.is_none() {
                return;
            }
            if !keep_running(st.last_want, Instant::now()) {
                st.session = None;
                return;
            }
        });
    }
}

/// 관리자 확인 창을 띄워 지금 사용자를 그룹에 넣거나(`grant`) 뺀다
pub fn elevate(grant: bool) -> Result<(), String> {
    platform::elevate(grant)
}

/// `saba-core net-trace grant|revoke <SID>`: 관리자 권한으로 다시 띄워졌을 때 그룹을 바꾼다
pub fn group_tool(args: &[String]) -> i32 {
    platform::group_tool(args)
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) struct Session;

    pub(super) fn start(_counters: Arc<Mutex<NetCounters>>) -> Result<Session, StartError> {
        Err(StartError::Unsupported)
    }

    pub(super) fn access() -> Access {
        Access::Unsupported
    }

    pub(super) fn elevate(_grant: bool) -> Result<(), String> {
        Err("unsupported".into())
    }

    pub(super) fn group_tool(_args: &[String]) -> i32 {
        eprintln!("net-trace: Windows only");
        2
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use ferrisetw::native::EvntraceNativeError;
    use ferrisetw::parser::Parser;
    use ferrisetw::provider::Provider;
    use ferrisetw::schema_locator::SchemaLocator;
    use ferrisetw::trace::{TraceError, UserTrace};
    use ferrisetw::EventRecord;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::NetworkManagement::NetManagement::{
        NetApiBufferFree, NetLocalGroupAddMembers, NetLocalGroupDelMembers, NetLocalGroupGetMembers,
        LOCALGROUP_MEMBERS_INFO_0, MAX_PREFERRED_LENGTH,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::System::Diagnostics::Etw::{EventAccessControl, EventSecurityAddDACL, TRACELOG_GUID_ENABLE};
    use windows_sys::Win32::Security::{
        CheckTokenMembership, EqualSid, GetTokenInformation, LookupAccountSidW, TokenUser, PSID, SID_NAME_USE,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    /// Microsoft-Windows-Kernel-Network
    const KERNEL_NETWORK: &str = "7dd42a49-5329-4832-8dfd-43d979153a88";
    const KERNEL_NETWORK_GUID: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0x7dd42a49_5329_4832_8dfd_43d979153a88);
    /// 공급자별 ETW 보안 설정이 있는 곳 (값 이름 = 중괄호 없는 GUID, REG_BINARY 자체 상대 보안 서술자)
    const WMI_SECURITY_KEY: &str = r"SYSTEM\CurrentControlSet\Control\WMI\Security";
    const SESSION_NAME: &str = "saba-chan-net";
    const PERF_LOG_USERS: &str = "S-1-5-32-559";
    const ADMINISTRATORS: &str = "S-1-5-32-544";
    /// NetLocalGroupAddMembers: 이미 들어 있다 / NetLocalGroupDelMembers: 원래 없다
    const ERROR_MEMBER_IN_ALIAS: u32 = 1378;
    const ERROR_MEMBER_NOT_IN_ALIAS: u32 = 1377;
    /// StartTraceW가 돌려주는 HRESULT (0x80070005, 접근 거부)
    const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;

    pub(super) struct Session {
        _trace: UserTrace,
    }

    pub(super) fn start(counters: Arc<Mutex<NetCounters>>) -> Result<Session, StartError> {
        let build = || {
            let counters = counters.clone();
            let cb = move |record: &EventRecord, locator: &SchemaLocator| {
                let Some((proto, dir)) = classify(record.event_id()) else {
                    return;
                };
                let Ok(schema) = locator.event_schema(record) else {
                    return;
                };
                let p = Parser::create(record, &schema);
                let (Ok(pid), Ok(size), Ok(sport)) = (
                    p.try_parse::<u32>("PID"),
                    p.try_parse::<u32>("size"),
                    p.try_parse::<u16>("sport"),
                ) else {
                    return;
                };
                // 포트는 네트워크 바이트 순서로 온다. sport는 보낼 때도 받을 때도 이 PC 쪽 포트다 (실측)
                let port = u16::from_be(sport);
                counters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .add(pid, proto, port, dir, size as u64);
            };
            let provider = Provider::by_guid(KERNEL_NETWORK).add_callback(cb).build();
            UserTrace::new()
                .named(SESSION_NAME.to_string())
                .enable(provider)
                .start_and_process()
        };
        let trace = match build() {
            // 데몬이 죽어 지난 세션이 남았다: 닫고 한 번 더
            Err(TraceError::EtwNativeError(EvntraceNativeError::AlreadyExist)) => {
                let _ = ferrisetw::trace::stop_trace_by_name(SESSION_NAME);
                build()
            }
            other => other,
        };
        if trace.is_err() {
            // 공급자를 켜다 거부되면 ferrisetw는 만든 세션을 남긴다 (PC 실측): 다음 시도가 AlreadyExist가 되지 않게 닫는다
            let _ = ferrisetw::trace::stop_trace_by_name(SESSION_NAME);
        }
        match trace {
            Ok(t) => Ok(Session { _trace: t }),
            Err(TraceError::EtwNativeError(EvntraceNativeError::IoError(e)))
                if matches!(e.raw_os_error(), Some(5) | Some(E_ACCESSDENIED)) =>
            {
                Err(StartError::Denied)
            }
            Err(e) => Err(StartError::Other(format!("{:?}", e))),
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 문자열 SID → LocalFree로 풀 PSID
    struct OwnedSid(PSID);
    impl OwnedSid {
        fn parse(s: &str) -> Option<Self> {
            let w = wide(s);
            let mut sid: PSID = null_mut();
            (unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) } != 0).then_some(Self(sid))
        }
    }
    impl Drop for OwnedSid {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0 as _) };
        }
    }

    fn token_has(sid: &str) -> bool {
        let Some(sid) = OwnedSid::parse(sid) else { return false };
        let mut is_member = 0;
        unsafe { CheckTokenMembership(null_mut(), sid.0, &mut is_member) != 0 && is_member != 0 }
    }

    fn current_user_sid() -> Option<String> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut size = 0;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
            let mut data = vec![0u8; size as usize];
            let ok = GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size) != 0;
            CloseHandle(token);
            if !ok {
                return None;
            }
            let user = &*(data.as_ptr() as *const TOKEN_USER);
            let mut text = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
                return None;
            }
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text as _);
            Some(sid)
        }
    }

    /// 그룹 이름 (Windows 언어마다 다르다)
    fn group_name(group: &OwnedSid) -> Option<Vec<u16>> {
        let mut name = vec![0u16; 256];
        let mut domain = vec![0u16; 256];
        let (mut nlen, mut dlen) = (name.len() as u32, domain.len() as u32);
        let mut use_: SID_NAME_USE = 0;
        let ok = unsafe {
            LookupAccountSidW(
                null(),
                group.0,
                name.as_mut_ptr(),
                &mut nlen,
                domain.as_mut_ptr(),
                &mut dlen,
                &mut use_,
            )
        };
        if ok == 0 {
            return None;
        }
        name.truncate(nlen as usize);
        name.push(0);
        Some(name)
    }

    /// 계정 데이터베이스에서 그 사용자가 그룹에 들었는지 (토큰과 달리 로그인 전에도 바로 보인다)
    fn is_member(user: &str) -> bool {
        let (Some(group), Some(user)) = (OwnedSid::parse(PERF_LOG_USERS), OwnedSid::parse(user)) else {
            return false;
        };
        let Some(name) = group_name(&group) else { return false };
        let mut buf: *mut u8 = null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        let status = unsafe {
            NetLocalGroupGetMembers(
                null(),
                name.as_ptr(),
                0,
                &mut buf,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                null_mut(),
            )
        };
        if status != 0 || buf.is_null() {
            return false;
        }
        let members = unsafe { std::slice::from_raw_parts(buf as *const LOCALGROUP_MEMBERS_INFO_0, read as usize) };
        let found = members.iter().any(|m| unsafe { EqualSid(m.lgrmi0_sid, user.0) } != 0);
        unsafe { NetApiBufferFree(buf as *const c_void) };
        found
    }

    /// 공급자 보안 설정을 SDDL로 (값이 없거나 못 읽으면 None)
    fn provider_sddl() -> Option<String> {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let key = winreg::RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(WMI_SECURITY_KEY).ok()?;
        let raw = key.get_raw_value(KERNEL_NETWORK).ok()?;
        let info = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | SACL_SECURITY_INFORMATION;
        let mut text = null_mut();
        let mut len = 0u32;
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                raw.bytes.as_ptr() as PSECURITY_DESCRIPTOR,
                SDDL_REVISION_1,
                info,
                &mut text,
                &mut len,
            )
        };
        if ok == 0 {
            return None;
        }
        let mut n = 0;
        while unsafe { *text.add(n) } != 0 {
            n += 1;
        }
        let sddl = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, n) });
        unsafe { LocalFree(text as _) };
        Some(sddl)
    }

    /// 공급자 보안 설정을 SDDL로 덮어쓴다 (관리자)
    fn write_provider_sddl(sddl: &str) -> Result<(), String> {
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_SET_VALUE, RegType};
        let w = wide(sddl);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let mut size = 0u32;
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), SDDL_REVISION_1, &mut sd, &mut size) } == 0 {
            return Err("bad security descriptor".into());
        }
        let bytes = unsafe { std::slice::from_raw_parts(sd as *const u8, size as usize) }.to_vec();
        unsafe { LocalFree(sd as _) };
        let key = winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(WMI_SECURITY_KEY, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        key.set_raw_value(KERNEL_NETWORK, &winreg::RegValue { bytes, vtype: RegType::REG_BINARY })
            .map_err(|e| e.to_string())
    }

    pub(super) fn access() -> Access {
        let admin = token_has(ADMINISTRATORS);
        let sid = current_user_sid();
        let group_in_token = token_has(PERF_LOG_USERS);
        let member = group_in_token || sid.as_deref().is_some_and(is_member);
        // 값이 없으면 기본 설정(그룹 허용)을 따른다
        let provider = match (provider_sddl(), sid.as_deref()) {
            (Some(sddl), Some(sid)) => Some(sddl_allows(&sddl, sid)),
            _ => None,
        };
        Access::from_parts(admin, group_in_token, member, provider)
    }

    pub(super) fn elevate(grant: bool) -> Result<(), String> {
        let sid = current_user_sid().ok_or("cannot read the current user")?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let verb = wide("runas");
        let file = wide(&exe.to_string_lossy());
        let params = wide(&format!("net-trace {} {}", if grant { "grant" } else { "revoke" }, sid));
        let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.nShow = 0; // SW_HIDE
        if unsafe { ShellExecuteExW(&mut sei) } == 0 {
            return Err("cancelled".into());
        }
        if sei.hProcess.is_null() {
            return Err("no process".into());
        }
        let mut code = 1u32;
        unsafe {
            WaitForSingleObject(sei.hProcess, 30_000);
            GetExitCodeProcess(sei.hProcess, &mut code);
            CloseHandle(sei.hProcess);
        }
        if code == 0 {
            Ok(())
        } else {
            Err(format!("exit code {}", code))
        }
    }

    pub(super) fn group_tool(args: &[String]) -> i32 {
        let (grant, sid) = match args {
            [op, sid] if op == "grant" => (true, sid),
            [op, sid] if op == "revoke" => (false, sid),
            _ => {
                eprintln!("usage: saba-core net-trace grant|revoke <SID>");
                return 2;
            }
        };
        let (Some(group), Some(user)) = (OwnedSid::parse(PERF_LOG_USERS), OwnedSid::parse(sid)) else {
            eprintln!("net-trace: bad SID");
            return 2;
        };
        let Some(name) = group_name(&group) else {
            eprintln!("net-trace: group not found");
            return 1;
        };
        let member = LOCALGROUP_MEMBERS_INFO_0 { lgrmi0_sid: user.0 };
        let buf = &member as *const LOCALGROUP_MEMBERS_INFO_0 as *const u8;
        let status = unsafe {
            if grant {
                NetLocalGroupAddMembers(null(), name.as_ptr(), 0, buf, 1)
            } else {
                NetLocalGroupDelMembers(null(), name.as_ptr(), 0, buf, 1)
            }
        };
        if !matches!(status, ERROR_SUCCESS | ERROR_MEMBER_IN_ALIAS | ERROR_MEMBER_NOT_IN_ALIAS) {
            eprintln!("net-trace: group change failed ({})", status);
            return 1;
        }
        // 그룹은 세션을 만들 권한만 준다. Kernel-Network 공급자는 따로 잠겨 있어(시스템·관리자만) 이 계정 허용을 더한다
        if grant {
            let code = unsafe {
                EventAccessControl(&KERNEL_NETWORK_GUID, EventSecurityAddDACL as u32, user.0, TRACELOG_GUID_ENABLE, 1)
            };
            if code != ERROR_SUCCESS {
                eprintln!("net-trace: provider permission failed ({})", code);
                return 1;
            }
        } else if let Some(sddl) = provider_sddl() {
            let stripped = strip_sid_aces(&sddl, sid);
            if stripped != sddl
                && let Err(e) = write_provider_sddl(&stripped)
            {
                eprintln!("net-trace: provider permission revoke failed ({})", e);
                return 1;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pp(field: &str, port: u16, protocol: Protocol) -> PublicPort {
        PublicPort {
            field: field.into(),
            port,
            protocol,
        }
    }

    #[test]
    fn only_data_events_are_counted() {
        assert_eq!(classify(10), Some((Protocol::Tcp, Dir::Tx)));
        assert_eq!(classify(11), Some((Protocol::Tcp, Dir::Rx)));
        assert_eq!(classify(42), Some((Protocol::Udp, Dir::Tx)));
        assert_eq!(classify(43), Some((Protocol::Udp, Dir::Rx)));
        assert_eq!(classify(58), Some((Protocol::Udp, Dir::Tx)));
        // 연결·끊김·재시도는 바이트가 아니다
        for id in [12, 13, 15, 16, 18, 29] {
            assert_eq!(classify(id), None, "event {id}");
        }
    }

    #[test]
    fn bytes_go_to_the_owning_instance_and_its_declared_ports() {
        let mut c = NetCounters::default();
        c.add(100, Protocol::Tcp, 25565, Dir::Tx, 6000);
        c.add(100, Protocol::Tcp, 25565, Dir::Rx, 900);
        c.add(101, Protocol::Tcp, 51000, Dir::Tx, 50); // 같은 서버의 자식 프로세스, 나가는 연결
        c.add(200, Protocol::Udp, 8211, Dir::Rx, 700);
        c.add(300, Protocol::Udp, 8211, Dir::Rx, 40); // 다른 프로그램이 같은 번호를 써도 서버 몫이 아니다
        c.add(999, Protocol::Tcp, 443, Dir::Rx, 1000);
        let owner: HashMap<u32, String> = [
            (100, "mc".to_string()),
            (101, "mc".to_string()),
            (200, "pal".to_string()),
        ]
        .into();
        let ports = vec![
            ("mc".to_string(), vec![pp("port", 25565, Protocol::Tcp)]),
            (
                "pal".to_string(),
                vec![pp("port", 8211, Protocol::Udp), pp("rest_port", 8212, Protocol::Tcp)],
            ),
        ];
        let s = split(&c.take(), &owner, &ports);
        assert_eq!(s.by_instance["mc"], (900, 6050));
        assert_eq!(s.by_instance["pal"], (700, 0));
        assert_eq!(s.others, (1040, 0));
        let rows: Vec<_> = s
            .ports
            .iter()
            .map(|p| (p.instance_id.as_str(), p.field.as_str(), p.port, p.protocol, p.rx, p.tx))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("mc", "port", 25565, Protocol::Tcp, 900, 6000),
                ("pal", "port", 8211, Protocol::Udp, 700, 0),
                ("pal", "rest_port", 8212, Protocol::Tcp, 0, 0),
            ]
        );
        // 한 번 가져가면 비워진다
        assert!(c.take().is_empty());
    }

    #[test]
    fn access_needs_both_the_group_and_the_provider_permission() {
        assert_eq!(Access::from_parts(true, false, false, Some(false)), Access::Ok, "admin needs nothing");
        assert_eq!(Access::from_parts(false, true, true, Some(true)), Access::Ok);
        assert_eq!(Access::from_parts(false, false, true, Some(true)), Access::NeedsRelogin);
        assert_eq!(Access::from_parts(false, false, false, Some(true)), Access::NeedsGrant);
        // 그룹만으로는 공급자를 켤 수 없다 (PC 실측): 공급자 허용이 없으면 다시 받아야 한다
        assert_eq!(Access::from_parts(false, true, true, Some(false)), Access::NeedsGrant);
        // 공급자 설정을 못 읽으면 그룹 상태로 판단하고, 세션을 열다 거부되면 그때 needs_grant
        assert_eq!(Access::from_parts(false, true, true, None), Access::Ok);
    }

    #[test]
    fn revoke_removes_only_the_users_ace() {
        // PC에서 읽은 실제 값 (주인님 계정 허용을 더한 뒤)
        let granted = "O:BAG:BAD:(A;;0x120fff;;;SY)(A;;LO;;;LS)(A;;0x1207ff;;;BA)(A;;LO;;;S-1-5-21-1-2-3-1001)";
        let original = "O:BAG:BAD:(A;;0x120fff;;;SY)(A;;LO;;;LS)(A;;0x1207ff;;;BA)";
        assert_eq!(strip_sid_aces(granted, "S-1-5-21-1-2-3-1001"), original);
        assert_eq!(strip_sid_aces(original, "S-1-5-21-1-2-3-1001"), original);
        // 다른 계정의 ACE와 SACL은 건드리지 않는다
        let other = "D:(A;;LO;;;S-1-5-21-9)(A;;LO;;;S-1-5-21-1-2-3-1001)S:(AU;SA;LO;;;S-1-5-21-1-2-3-1001)";
        assert_eq!(strip_sid_aces(other, "S-1-5-21-1-2-3-1001"), "D:(A;;LO;;;S-1-5-21-9)S:(AU;SA;LO;;;S-1-5-21-1-2-3-1001)");
        assert!(sddl_allows(granted, "s-1-5-21-1-2-3-1001"));
        assert!(!sddl_allows(original, "S-1-5-21-1-2-3-1001"));
        assert!(!sddl_allows("D:(D;;LO;;;S-1-5-21-9)", "S-1-5-21-9"), "deny is not allow");
    }

    #[test]
    fn session_stays_open_only_while_someone_looks() {
        let t = Instant::now();
        assert!(!keep_running(None, t));
        assert!(keep_running(Some(t), t + IDLE_STOP / 2));
        assert!(!keep_running(Some(t), t + IDLE_STOP));
    }

    #[test]
    fn collector_reports_why_it_is_not_counting() {
        let net = NetTrace::default();
        assert_eq!(net.take(false, true).unwrap_err(), "off");
        // 펼친 요청이 없으면 열지 않는다
        assert_eq!(net.take(true, false).unwrap_err(), "idle");
        #[cfg(not(windows))]
        assert_eq!(net.take(true, true).unwrap_err(), "unsupported");
    }
}
