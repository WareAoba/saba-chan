//! 확인되지 않은 모듈·익스텐션 실행 제한 (Windows)
//!
//! 신뢰 표시(`integrity::trust_of`)가 `verified`가 아닌 구성요소에만 적용한다. 공식 카탈로그에서 받아
//! 그대로인 것은 지금처럼 실행한다.
//! - 실행 대상 검사: 모듈이 돌려준 서버 실행 파일이 인스턴스 폴더나 데몬이 아는 런타임 안에 있어야 한다.
//!   셸·스크립트 호스트(`cmd`, `powershell` 등)는 어디 있든 거부한다 (`launch_outside_instance`).
//! - 악성 코드 검사: Python·스크립트 파일은 AMSI(등록된 백신)로, 서버 실행 파일은 처음 실행 전에
//!   Defender `MpCmdRun`으로 본다. 찾으면 실행하지 않는다 (`malware_detected`). 결과는 sha256으로 저장해
//!   바뀌지 않은 파일은 다시 보지 않는다(깨끗하다는 결과는 7일 동안). 검사기가 없으면 건너뛰고 경고를 남긴다.
//!   파일을 읽지 못하거나 검사가 실패하면 실행하지 않는다 (`scan_failed`).
//!
//! Linux·macOS는 아직 지원 대상이 아니라 지금처럼 실행한다.

use saba_chan_updater_lib::integrity::Trust;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub mod appcontainer;
pub mod scan;

/// 이 플랫폼에서 제한을 거는가
pub const ENFORCED: bool = cfg!(windows);

/// 모듈·익스텐션 진입 파일(`<modules>/<이름>/lifecycle.py` 등)의 설치 기록 키와 폴더
pub fn component_of(entry: &Path) -> Option<(String, PathBuf)> {
    let dir = entry.parent()?;
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let parent = std::fs::canonicalize(dir.parent()?).ok()?;
    let same = |root: PathBuf| std::fs::canonicalize(root).is_ok_and(|r| r == parent);
    if same(crate::plugin::resolve_modules_dir()) {
        return Some((format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, name), dir.to_path_buf()));
    }
    if same(crate::plugin::resolve_extensions_dir()) {
        return Some((format!("{}{}", saba_chan_updater_lib::relay::EXTENSION_PREFIX, name), dir.to_path_buf()));
    }
    None
}

/// 진입 파일의 신뢰 표시. 모듈·익스텐션 폴더 밖이면 확인되지 않은 것으로 본다
pub fn trust_of_entry(entry: &Path) -> Trust {
    match component_of(entry) {
        Some((key, dir)) => saba_chan_updater_lib::integrity::trust_of(&key, &dir),
        None => Trust::Unverified,
    }
}

/// 제한을 거는가 (Windows이고 공식·그대로가 아닌 것)
pub fn restricted(trust: Trust) -> bool {
    ENFORCED && trust != Trust::Verified
}

/// 확인되지 않은 모듈이 돌려준 폴더(설치 결과 `install_path` 등)를 받아들이는가. 그 폴더는 인스턴스 폴더 키로
/// 저장되어 다음 격리 실행에 쓰기를 받으므로, 데몬이 정한 `root`(인스턴스 폴더) 안일 때만 받는다
pub fn accepts_module_dir(entry: &Path, dir: &str, root: &Path) -> bool {
    !restricted(trust_of_entry(entry)) || folder_inside(Path::new(dir), root)
}

/// `root` 안의 실제 폴더인가 (파일은 아니다: 폴더로 열면 그 파일을 실행하게 된다)
fn folder_inside(path: &Path, root: &Path) -> bool {
    inside(path, root) && path.is_dir()
}

/// 링크·`..`를 푼 실제 경로로 `path`가 `root` 안인가 (없는 경로는 아니다)
fn inside(path: &Path, root: &Path) -> bool {
    if !plain_local_path(path) {
        return false;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(p), Ok(r)) => p.starts_with(r),
        _ => false,
    }
}

/// 어디 있든 실행하지 않는 프로그램 (셸·스크립트 호스트). 파일 이름(확장자 뺀 것)을 소문자로 본다
const BLOCKED_PROGRAMS: &[&str] = &[
    "cmd", "powershell", "pwsh", "wscript", "cscript", "mshta", "rundll32", "regsvr32", "regsvcs", "regasm",
    "msbuild", "installutil", "bitsadmin", "certutil", "schtasks", "reg", "sh", "bash",
];

/// 그 자체로 셸이 실행하는 파일 (CreateProcess가 `.bat`·`.cmd`를 cmd로 연다)
const SCRIPT_EXTENSIONS: &[&str] = &["bat", "cmd", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "msc", "lnk", "scr"];

/// `java`처럼 PATH에서 찾는 런타임으로 인정하는 이름
const RUNTIME_NAMES: &[&str] = &["java", "javaw"];

/// 실행 대상 검사에 쓰는 허용 위치
#[derive(Debug, Default, Clone)]
pub struct LaunchRoots {
    /// 인스턴스 폴더, 서버 폴더(작업 폴더·마이그레이션 원본)
    pub dirs: Vec<PathBuf>,
    /// 사용자가 설정한 런타임 경로 (이름이 `RUNTIME_NAMES`일 때만 인정)
    pub runtimes: Vec<PathBuf>,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn extension(path: &Path) -> String {
    path.extension().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// 실행할 프로그램 경로. 경로가 있는 상대 경로는 데몬 폴더가 아니라 작업 폴더(`cwd`) 기준으로 풀어
/// 검사한 경로 그대로 띄운다. 경로 없는 이름(`java`)은 그대로 둔다
pub fn resolve_program(program: &str, cwd: &Path) -> String {
    let path = Path::new(program.trim());
    if path.is_absolute() || path.components().count() <= 1 {
        return program.trim().to_string();
    }
    let joined = cwd.join(path);
    std::path::absolute(&joined).unwrap_or(joined).to_string_lossy().into_owned()
}

/// 확인되지 않은 모듈이 돌려준 실행 대상 검사. 통과하지 못하면 `launch_outside_instance`
pub fn check_launch(program: &str, roots: &LaunchRoots) -> Result<(), &'static str> {
    const OUTSIDE: &str = "launch_outside_instance";
    let path = Path::new(program.trim());
    let name = stem(path);
    if name.is_empty() || BLOCKED_PROGRAMS.contains(&name.as_str()) || SCRIPT_EXTENSIONS.contains(&extension(path).as_str()) {
        return Err(OUTSIDE);
    }
    // 경로 없는 이름: PATH의 java 같은 런타임만
    if path.components().count() == 1 && !path.is_absolute() {
        return if RUNTIME_NAMES.contains(&name.as_str()) { Ok(()) } else { Err(OUTSIDE) };
    }
    // 모듈이 준 경로는 파일 시스템에 묻기 전에 거른다 (UNC 등)
    if !plain_local_path(path) {
        return Err(OUTSIDE);
    }
    // 링크·`..`로 빠져나가지 않게 실제 경로로 비교한다. 없는 파일은 거부
    let Ok(real) = std::fs::canonicalize(path) else { return Err(OUTSIDE) };
    let inside = roots.dirs.iter().filter_map(|d| std::fs::canonicalize(d).ok()).any(|d| real.starts_with(d));
    let runtime = RUNTIME_NAMES.contains(&name.as_str())
        && roots.runtimes.iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|r| r == real);
    if inside || runtime {
        Ok(())
    } else {
        Err(OUTSIDE)
    }
}

/// 실행을 막은 이유 (`action_required`로 돌려준다)
#[derive(Debug, Clone, PartialEq)]
pub struct Blocked {
    pub code: &'static str,
    pub file: Option<String>,
    pub detail: Option<String>,
}

impl Blocked {
    pub fn to_json(&self, message: String) -> Value {
        serde_json::json!({
            "success": false,
            "action_required": self.code,
            "message": message,
            "file": self.file,
            "detail": self.detail,
        })
    }
}

/// 막은 이유 문구 (영어 기본. GUI·CLI는 `action_required` 코드로 현지화한다)
pub fn message(blocked: &Blocked) -> String {
    let file = blocked.file.as_deref().unwrap_or("");
    match blocked.code {
        "malware_detected" => format!("Antivirus flagged a file of this unverified component: {}", file),
        "launch_outside_instance" => format!("This unverified module tried to run a program outside the server folder: {}. If you set the Java path yourself, choose it again in the server settings.", file),
        "unscannable_file" => format!("This unverified component contains compiled code without source that cannot be scanned: {}", file),
        "scan_failed" => format!("Could not scan a file of this unverified component, so it was not run: {}", file),
        "sandbox_unavailable" => format!("Could not prepare the isolated environment for this unverified component: {}", blocked.detail.as_deref().unwrap_or("")),
        other => other.to_string(),
    }
}

/// 깨끗하다는 결과를 다시 쓰는 기간 (백신 정의가 바뀌므로 지나면 다시 본다)
const CLEAN_VERDICT_TTL_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct StoredVerdict {
    detected: bool,
    /// 검사한 시각 (유닉스 초)
    #[serde(default)]
    at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 옛 형식({sha: bool})은 시각 0으로 옮긴다: 찾음은 그대로 남고 깨끗함은 다시 본다. 깨진 파일은 비운다
fn parse_entries(s: &str) -> BTreeMap<String, StoredVerdict> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Legacy(bool),
        Current(StoredVerdict),
    }
    let raw: BTreeMap<String, Entry> = serde_json::from_str(s).unwrap_or_default();
    raw.into_iter()
        .map(|(k, e)| match e {
            Entry::Legacy(detected) => (k, StoredVerdict { detected, at: 0 }),
            Entry::Current(v) => (k, v),
        })
        .collect()
}

/// sha256 → 검사 결과 (`<data dir>/scan-verdicts.json`). 깨끗하다/찾았다만 저장한다
#[derive(Debug, Default)]
pub struct VerdictCache {
    path: Option<PathBuf>,
    entries: BTreeMap<String, StoredVerdict>,
}

impl VerdictCache {
    pub fn load() -> Self {
        let path = saba_chan_updater_lib::constants::resolve_data_dir().join("scan-verdicts.json");
        let entries = std::fs::read_to_string(&path).ok().map(|s| parse_entries(&s)).unwrap_or_default();
        Self { path: Some(path), entries }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Some(true) = 찾았다, Some(false) = 깨끗하다 (7일이 지난 깨끗하다는 None: 다시 본다)
    pub fn get(&self, sha: &str) -> Option<bool> {
        self.get_at(sha, now_secs())
    }

    fn get_at(&self, sha: &str, now: u64) -> Option<bool> {
        let v = self.entries.get(sha)?;
        (v.detected || now.saturating_sub(v.at) < CLEAN_VERDICT_TTL_SECS).then_some(v.detected)
    }

    pub fn put(&mut self, sha: String, detected: bool) {
        self.entries.insert(sha, StoredVerdict { detected, at: now_secs() });
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Ok(json) = serde_json::to_string(&self.entries) {
            if let Err(e) = std::fs::write(path, json) {
                tracing::warn!("[Sandbox] could not save scan verdicts: {}", e);
            }
        }
    }
}

/// 검사를 끝내지 못했다 (검사 스레드가 죽었다 등). 제한하는 구성요소는 실행하지 않는다
fn scan_failed(detail: impl std::fmt::Display) -> Blocked {
    Blocked { code: "scan_failed", file: None, detail: Some(detail.to_string()) }
}

/// Python 실행 전: 확인되지 않은 구성요소면 스크립트를 검사한다
pub async fn guard_entry(entry: &str) -> Result<(), Blocked> {
    // 신뢰 표시는 설치한 파일을 해시한다: 제한하지 않는 플랫폼에서는 구하지 않는다
    if !ENFORCED {
        return Ok(());
    }
    let entry = PathBuf::from(entry);
    if !restricted(trust_of_entry(&entry)) {
        return Ok(());
    }
    let dir = entry.parent().map(Path::to_path_buf).ok_or_else(|| scan_failed("no component folder"))?;
    tokio::task::spawn_blocking(move || {
        let mut cache = VerdictCache::load();
        let result = scan::scan_component(&dir, &scan::SystemScanner, &mut cache);
        cache.save();
        result
    })
    .await
    .unwrap_or_else(|e| Err(scan_failed(e)))
}

/// 서버 실행 전: 확인되지 않은 모듈이면 실행 대상 위치를 보고 실행 파일을 검사한다. 띄울 프로그램 경로를 돌려준다
pub async fn guard_launch(entry: &str, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<String, Blocked> {
    guard_launch_for(|| trust_of_entry(Path::new(entry)), program, args, cwd, roots, env).await
}

/// 익스텐션 설치 기록의 신뢰 표시 (`extensions/<id>`)
pub fn extension_trust(ext_id: &str) -> Trust {
    let key = format!("{}{}", saba_chan_updater_lib::relay::EXTENSION_PREFIX, ext_id);
    saba_chan_updater_lib::integrity::trust_of(&key, &crate::plugin::resolve_extensions_dir().join(ext_id))
}

/// 실행 대상을 찾을 때 쓰는 변수. 확인되지 않은 구성요소가 준 값은 버린다
/// (경로 없는 `java`가 그 폴더에서 찾아지지 않게)
const SEARCH_PATH_VARS: &[&str] = &["PATH", "PATHEXT"];

pub fn strip_search_path(env: &mut Vec<(String, String)>) {
    env.retain(|(k, _)| !SEARCH_PATH_VARS.iter().any(|v| k.eq_ignore_ascii_case(v)));
}

/// `guard_launch`의 신뢰 표시를 받는 형태 (익스텐션이 띄우는 프로그램에도 쓴다).
/// `trust`는 제한하는 플랫폼에서만 부른다. 제한하면 상대 경로를 작업 폴더 기준으로 푼 경로를 돌려준다
pub async fn guard_launch_for(trust: impl FnOnce() -> Trust, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<String, Blocked> {
    if !ENFORCED || !restricted(trust()) {
        return Ok(program.to_string());
    }
    strip_search_path(env);
    let program = resolve_program(program, cwd);
    if let Err(code) = check_launch(&program, &roots) {
        return Err(Blocked { code, file: Some(program), detail: None });
    }
    tracing::info!("[Sandbox] checking launch of unverified module program '{}'", program);
    // 실행 파일(경로 없는 java는 PATH의 런타임이라 빼고)과 인자로 넘기는 파일(`-jar x.jar`, `@args.txt` 등)
    let mut files: Vec<PathBuf> = Some(PathBuf::from(&program)).filter(|p| p.is_absolute()).into_iter().collect();
    match argument_files(args, cwd) {
        Ok(found) => files.extend(found),
        Err(arg) => return Err(Blocked { code: "launch_outside_instance", file: Some(arg), detail: None }),
    }
    tokio::task::spawn_blocking(move || {
        let mut cache = VerdictCache::load();
        let result = files.iter().try_for_each(|f| scan::scan_executable(f, &scan::SystemScanner, &mut cache));
        cache.save();
        result
    })
    .await
    .unwrap_or_else(|e| Err(scan_failed(e)))?;
    Ok(program)
}

/// `.`·`..`를 글자로만 푼 경로 (파일 시스템에 묻지 않는다)
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// 실행 인자 중 있는 파일을 가리키는 것: 인자 그대로, `@파일`, `-옵션=파일`·`-옵션:파일` (상대 경로는 `cwd` 기준).
/// 로컬 드라이브가 아닌 경로(UNC 등)를 가리키는 인자는 묻지 않고 그 인자를 돌려준다 (`Err`)
fn argument_files(args: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for arg in args {
        let mut candidates = vec![arg.as_str(), arg.trim_start_matches('@')];
        if arg.starts_with('-') {
            candidates.extend(arg.split_once('=').map(|(_, v)| v));
            candidates.extend(arg.split_once(':').map(|(_, v)| v));
        }
        for c in candidates.into_iter().filter(|c| !c.is_empty()) {
            let path = lexical(&cwd.join(c));
            if !plain_local_path(&path) {
                return Err(arg.clone());
            }
            if path.is_file() && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// 격리해서 돌릴 Python 실행 하나
#[derive(Debug, Clone)]
pub struct Isolation {
    /// 설치 기록 키 (`module-<이름>`·`ext-<이름>`). 모듈·익스텐션 폴더 밖이면 그 폴더 경로
    pub key: String,
    pub dir: PathBuf,
    /// 이 구성요소만 쓰는 AppContainer 프로필 (`appcontainer::profile_for`)
    #[cfg_attr(not(windows), allow(dead_code))]
    pub profile: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub grants: appcontainer::Grants,
    pub pycache: PathBuf,
    pub temp: PathBuf,
    /// 익스텐션이 쓸 수 있는 전용 폴더 (`<data>/extension-data/<이름>`, 환경 변수 `SABA_EXTENSION_DATA_DIR`)
    pub data: Option<PathBuf>,
}

/// 데몬이 lifecycle 호출에 넣는 인스턴스 폴더 키 (설치·업데이트 호출은 `install_dir`만 넣는다)
const INSTANCE_DIR_FIELDS: [&str; 3] = ["instance_dir", "working_dir", "install_dir"];

/// 격리 실행이 config의 인스턴스 폴더(`INSTANCE_DIR_FIELDS`)에 받는 권한: 모듈은 쓰기, 그 밖(익스텐션 등)은 읽기
fn instance_access(key: &str) -> appcontainer::Access {
    if key.starts_with(saba_chan_updater_lib::relay::MODULE_PREFIX) {
        appcontainer::Access::Write
    } else {
        appcontainer::Access::Read
    }
}

/// 익스텐션 키면 그 익스텐션 전용 데이터 폴더
fn extension_data_dir(data_dir: &Path, key: &str) -> Option<PathBuf> {
    let name = key.strip_prefix(saba_chan_updater_lib::relay::EXTENSION_PREFIX)?;
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
        return None;
    }
    Some(data_dir.join("extension-data").join(name))
}

/// PATH에서 런타임 실행 파일을 찾는다 (앞에서부터 모두. 앞쪽 것이 Oracle `javapath` 같은 바로가기일 수 있다)
fn find_on_path(name: &str, path: Option<&OsStr>) -> Vec<PathBuf> {
    let Some(path) = path else { return Vec::new() };
    // 네트워크 공유(UNC)·상대 경로는 열어 보지 않는다 (느린 공유에서 멈추거나 자격 증명이 새지 않게)
    std::env::split_paths(path)
        .filter(|dir| plain_local_path(dir))
        .filter_map(|dir| [format!("{name}.exe"), name.to_string()].into_iter().map(|f| dir.join(f)).find(|p| p.is_file()))
        .collect()
}

/// JDK·JRE 배포판이 흔히 설치되는 폴더 아래 공급자 폴더 이름 (`<root>/<공급자>/<jdk>/release`)
const JAVA_VENDOR_DIRS: &[&str] = &[
    "Java",
    "Eclipse Adoptium",
    "Eclipse Foundation",
    "AdoptOpenJDK",
    "Microsoft",
    "Zulu",
    "BellSoft",
    "Amazon Corretto",
    "Semeru",
    "OpenJDK",
    "jvm",
];

/// 알려진 설치 위치 (Windows: Program Files와 `%LOCALAPPDATA%\Programs`, Unix: `/usr/lib`)
fn install_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var).map(PathBuf::from).filter(|d| plain_local_path(d)) {
                if !roots.contains(&dir) {
                    roots.push(dir);
                }
            }
        }
        // 사용자 단위 설치 (예: Temurin의 "이 사용자만" 설치)
        if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs")).filter(|d| plain_local_path(d)) {
            roots.push(dir);
        }
    } else {
        roots.push(PathBuf::from("/usr/lib"));
    }
    roots
}

/// 설치 위치 아래 JDK·JRE의 java 실행 파일. 공급자 폴더 바로 아래만 본다
fn installed_runtimes(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for vendor in roots.iter().flat_map(|r| JAVA_VENDOR_DIRS.iter().map(move |v| r.join(v))) {
        let Ok(entries) = std::fs::read_dir(&vendor) else { continue };
        let mut homes: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("release").is_file()).collect();
        homes.sort();
        found.extend(homes.into_iter().filter_map(|h| ["java.exe", "java"].into_iter().map(|f| h.join("bin").join(f)).find(|p| p.is_file())));
    }
    found
}

/// JDK·JRE의 `release` 파일에 적힌 값 (`JAVA_VERSION="21.0.5"`, `OS_ARCH="amd64"`)
fn release_value(home: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let prefix = format!("{key}=");
    text.lines().find_map(|l| l.strip_prefix(prefix.as_str())).map(|v| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty())
}

/// 주 버전: `1.8.0_381` → 8, `21.0.5` → 21
fn java_major(version: &str) -> Option<u32> {
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// 런타임 실행 파일의 설치 폴더 (`<home>/bin/java`). 링크는 따라간 실제 위치로 보고,
/// `release` 파일이 있는 JDK·JRE 폴더만 인정한다 (`bin`이 있는 아무 도구 폴더를 통째로 열지 않는다)
fn runtime_home(exe: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(exe).ok()?;
    if !real.is_file() || !RUNTIME_NAMES.contains(&stem(&real).as_str()) {
        return None;
    }
    let bin = real.parent()?;
    let home = bin.parent()?;
    (bin.file_name()?.eq_ignore_ascii_case("bin") && home.join("release").is_file()).then(|| home.to_path_buf())
}

/// 파일 시스템에 묻기 전에 걸러 낼 경로: 로컬 드라이브의 절대 경로이고 `..`가 없어야 한다
/// (UNC 경로는 묻는 것만으로 다른 PC에 사용자 자격을 보낼 수 있다)
pub fn plain_local_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    match path.components().next() {
        // `\\?\C:\`도 로컬 드라이브다 (Windows canonicalize가 돌려주는 모양)
        Some(Component::Prefix(p)) => matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)),
        _ => cfg!(not(windows)),
    }
}

/// 사용자가 설정에서 정한 런타임(`java_path`)을 그 구성요소의 격리 실행에 연다 (Windows, 확인되지 않은 것만).
/// 설정을 저장할 때 데몬이 부른다: 실행 때 받는 config의 값은 모듈 기본값과 구별할 수 없다
pub fn open_user_runtime(entry: &Path, java_path: &str) {
    if !ENFORCED || !restricted(trust_of_entry(entry)) {
        return;
    }
    let Some((key, _)) = component_of(entry) else { return };
    let raw_data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let data_dir = std::fs::canonicalize(&raw_data_dir).unwrap_or(raw_data_dir);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).and_then(|h| std::fs::canonicalize(h).ok());
    let mut grants = appcontainer::Grants::default();
    for runtime in runtime_homes(Some(java_path), None, std::env::var_os("PATH").as_deref()) {
        grants.push_optional(&runtime, appcontainer::Access::Read, &data_dir, home.as_deref());
    }
    if grants.entries.is_empty() {
        return;
    }
    #[cfg(windows)]
    if let Err(e) = appcontainer::grant(&appcontainer::profile_for(&key), &grants) {
        tracing::warn!("[Sandbox] could not open runtime {} to {}: {}", java_path, key, e);
    }
    #[cfg(not(windows))]
    let _ = key;
}

/// 데몬이 시작할 때: 예전에 준 런타임 읽기 권한 중 지금 쓰지 않는 것을 거둔다. 지금 쓰는 것은 데몬이 아는 런타임과
/// 인스턴스마다 사용자가 정한 `java_path`(`(모듈 진입 파일, java_path)`)다. Windows에서만 한다
pub fn reconcile_runtimes(user_runtimes: &[(String, String)]) {
    if !ENFORCED {
        return;
    }
    let path = std::env::var_os("PATH");
    let daemon: std::collections::BTreeSet<PathBuf> = runtime_homes(None, std::env::var_os("JAVA_HOME").as_deref(), path.as_deref()).into_iter().collect();
    let mut keep: BTreeMap<String, std::collections::BTreeSet<PathBuf>> = BTreeMap::new();
    for (entry, java) in user_runtimes {
        if let Some((key, _)) = component_of(Path::new(entry)) {
            keep.entry(appcontainer::profile_for(&key)).or_default().extend(runtime_homes(Some(java), None, path.as_deref()));
        }
    }
    #[cfg(windows)]
    appcontainer::revoke_stale_runtimes(&daemon, &keep);
    #[cfg(not(windows))]
    let _ = (daemon, keep);
}

/// 격리된 모듈이 확인하고 띄울 런타임의 설치 폴더: 사용자가 정한 `java_path`(경로 없는 이름이면 PATH에서),
/// `JAVA_HOME`, PATH의 모든 `java`, 알려진 설치 위치의 JDK·JRE. `release` 파일이 있는 실제 설치 폴더만 센다
/// (PATH 앞쪽의 바로가기는 빠진다). 모듈은 `java -version`으로 런타임을 확인하는데, AppContainer는 Program Files 밖 폴더를 읽지 못한다
pub fn runtime_homes(java_path: Option<&str>, java_home: Option<&OsStr>, path: Option<&OsStr>) -> Vec<PathBuf> {
    runtime_homes_in(java_path, java_home, path, &install_roots())
}

fn runtime_homes_in(java_path: Option<&str>, java_home: Option<&OsStr>, path: Option<&OsStr>, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut exes = Vec::new();
    if let Some(java) = java_path.map(str::trim).filter(|p| !p.is_empty()) {
        let java = Path::new(java);
        if java.is_absolute() {
            if !plain_local_path(java) {
                return Vec::new();
            }
            exes.push(java.to_path_buf());
        } else if java.components().count() == 1 && RUNTIME_NAMES.contains(&stem(java).as_str()) {
            exes.extend(find_on_path(&stem(java), path).into_iter().next());
        }
    }
    if let Some(home) = java_home.filter(|h| !h.is_empty()) {
        let bin = Path::new(home).join("bin");
        exes.extend(["java.exe", "java"].into_iter().map(|f| bin.join(f)).find(|p| p.is_file()));
    }
    exes.extend(find_on_path("java", path));
    exes.extend(installed_runtimes(roots));
    let mut homes: Vec<PathBuf> = Vec::new();
    for home in exes.iter().filter_map(|e| runtime_home(e)) {
        if !homes.contains(&home) {
            homes.push(home);
        }
    }
    homes
}

/// 데몬이 찾은 Java 런타임 하나 (모듈에 `SABA_JAVA_RUNTIMES`로 넘긴다)
#[derive(Debug, Clone, serde::Serialize)]
pub struct JavaRuntime {
    pub java: PathBuf,
    pub home: PathBuf,
    pub version: String,
    pub major: u32,
    /// `release`의 `OS_ARCH` (`amd64`, `x86`, `aarch64` …). 적혀 있지 않으면 빈 문자열
    pub arch: String,
}

/// 데몬 환경(`JAVA_HOME`, PATH)과 알려진 설치 위치의 Java 런타임. 순서는 `JAVA_HOME`, PATH, 설치 위치다
pub fn java_runtimes() -> Vec<JavaRuntime> {
    java_runtimes_in(std::env::var_os("JAVA_HOME").as_deref(), std::env::var_os("PATH").as_deref(), &install_roots())
}

fn java_runtimes_in(java_home: Option<&OsStr>, path: Option<&OsStr>, roots: &[PathBuf]) -> Vec<JavaRuntime> {
    runtime_homes_in(None, java_home, path, roots)
        .into_iter()
        .filter_map(|home| {
            let version = release_value(&home, "JAVA_VERSION")?;
            let major = java_major(&version)?;
            let arch = release_value(&home, "OS_ARCH").unwrap_or_default();
            let java = ["java.exe", "java"].into_iter().map(|f| home.join("bin").join(f)).find(|p| p.is_file())?;
            Some(JavaRuntime { java, home, version, major, arch })
        })
        .collect()
}

/// 데몬 환경(`JAVA_HOME`, PATH)이 가리키는 런타임 실행 파일. 격리 실행에 읽기를 준 그 런타임을 실행 대상 검사도 인정한다
pub fn daemon_runtimes() -> Vec<PathBuf> {
    runtime_executables(runtime_homes(None, std::env::var_os("JAVA_HOME").as_deref(), std::env::var_os("PATH").as_deref()))
}

fn runtime_executables(homes: Vec<PathBuf>) -> Vec<PathBuf> {
    homes
        .into_iter()
        .flat_map(|home| ["java.exe", "javaw.exe", "java", "javaw"].map(|f| home.join("bin").join(f)))
        .filter(|p| p.is_file())
        .collect()
}

/// 확인되지 않은 구성요소의 Python 실행이면 격리 계획을 만든다. `config`의 `instance_dir`·`working_dir`에
/// 쓰기를 준다 (데몬이 넣는 값. 데이터 폴더 자체와 그 위는 `Grants::push`가 버린다)
pub fn isolation_for(entry: &Path, config: &Value, python_exe: &Path) -> Option<Isolation> {
    use appcontainer::Access;
    if !ENFORCED || !restricted(trust_of_entry(entry)) {
        return None;
    }
    let (key, dir) = component_of(entry).unwrap_or_else(|| {
        let dir = entry.parent().map(Path::to_path_buf).unwrap_or_default();
        (dir.to_string_lossy().into_owned(), dir)
    });
    let raw_data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    // 비교는 실제 경로로, Python에 넘기는 경로는 원래 모양으로 (`\\?\` 접두어 없이)
    let data_dir = std::fs::canonicalize(&raw_data_dir).unwrap_or_else(|_| raw_data_dir.clone());
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).and_then(|h| std::fs::canonicalize(h).ok());
    let home = home.as_deref();
    // 바이트코드 캐시와 임시 폴더도 구성요소마다 따로 (다른 구성요소가 넣어 둔 .pyc를 읽지 않게)
    let tag = appcontainer::component_tag(&key);
    let pycache = raw_data_dir.join("pycache-sandbox").join(&tag);
    let temp = raw_data_dir.join("sandbox-temp").join(&tag);
    for d in [&pycache, &temp] {
        let _ = std::fs::create_dir_all(d);
    }
    let mut grants = appcontainer::Grants::default();
    let mut add = |path: &Path, access| grants.push(path, access, &data_dir, home);
    // Python: venv와 그 기반 설치
    if let Some(venv) = python_exe.parent().and_then(Path::parent) {
        add(venv, Access::Read);
    }
    if let Some(base) = appcontainer::venv_home(python_exe) {
        add(&base, Access::Read);
    }
    add(&dir, Access::Read);
    let modules = crate::plugin::resolve_modules_dir();
    let extensions = crate::plugin::resolve_extensions_dir();
    add(&modules, Access::Read);
    add(&extensions, Access::Read);
    // PYTHONPATH의 익스텐션 상위 폴더: `import extensions.x`에 목록만 필요하다
    if let Some(parent) = extensions.parent() {
        add(parent, Access::List);
    }
    add(&data_dir.join("python-guard"), Access::Read);
    add(&pycache, Access::Write);
    add(&temp, Access::Write);
    // 인스턴스 폴더 쓰기는 그 인스턴스의 모듈에게만. 익스텐션 hook은 확인된 모듈의 인스턴스에도 불리므로
    // 읽기만 주고, 쓸 곳은 익스텐션 전용 데이터 폴더다
    let access = instance_access(&key);
    for field in INSTANCE_DIR_FIELDS {
        if let Some(path) = config.get(field).and_then(Value::as_str).filter(|p| !p.is_empty()) {
            let path = Path::new(path);
            // 새로 설치할 폴더는 아직 없다: 권한은 있는 폴더에만 줄 수 있으므로 먼저 만든다
            if access == Access::Write {
                let _ = std::fs::create_dir_all(path);
            }
            add(path, access);
            // `os.makedirs(dir, exist_ok=True)`는 바로 위 폴더가 있는지부터 본다: 그 폴더의 목록만 준다
            if let Some(parent) = path.parent() {
                add(parent, Access::List);
            }
        }
    }
    let data = extension_data_dir(&raw_data_dir, &key);
    if let Some(d) = &data {
        let _ = std::fs::create_dir_all(d);
        add(d, Access::Write);
    }
    // 런타임은 없어도 되는 권한: 주지 못하면 그 폴더만 빼고 띄운다 (Java를 쓰지 않는 구성요소까지 막지 않게)
    for runtime in runtime_homes(None, std::env::var_os("JAVA_HOME").as_deref(), std::env::var_os("PATH").as_deref()) {
        grants.push_optional(&runtime, Access::Read, &data_dir, home);
    }
    let profile = appcontainer::profile_for(&key);
    Some(Isolation { key, dir, profile, grants, pycache, temp, data })
}

/// 격리 실행을 띄운 결과
pub enum Spawned {
    #[cfg(windows)]
    Contained(appcontainer::Contained),
    /// 사용자가 이 구성요소를 격리 없이 돌리기로 했다
    Unisolated,
}

/// 격리해서 띄운다. 준비가 실패하면 사용자가 동의한 경우만 격리 없이 돌리고, 아니면 `sandbox_unavailable`
pub async fn spawn_isolated(cmd: &std::process::Command, iso: &Isolation) -> Result<Spawned, Blocked> {
    #[cfg(windows)]
    let error = {
        let program = cmd.get_program().to_os_string();
        let args: Vec<std::ffi::OsString> = cmd.get_args().map(|a| a.to_os_string()).collect();
        let cwd = cmd.get_current_dir().map(Path::to_path_buf);
        let changes = cmd.get_envs().map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string()))).collect();
        let env = appcontainer::merge_env(std::env::vars_os().collect(), changes);
        let (profile, grants) = (iso.profile.clone(), iso.grants.clone());
        let started = tokio::task::spawn_blocking(move || appcontainer::spawn(&program, &args, cwd.as_deref(), &env, &profile, &grants))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match started {
            Ok(child) => return Ok(Spawned::Contained(child)),
            Err(e) => e,
        }
    };
    #[cfg(not(windows))]
    let error = {
        let _ = cmd;
        "AppContainer is only available on Windows".to_string()
    };
    if unisolated_allowed(&iso.key, &iso.dir) {
        tracing::warn!("[Sandbox] {} runs without isolation by the user's choice ({})", iso.key, error);
        return Ok(Spawned::Unisolated);
    }
    tracing::warn!("[Sandbox] cannot isolate {}: {}", iso.key, error);
    Err(Blocked { code: "sandbox_unavailable", file: None, detail: Some(error) })
}

// ── 사용자 동의 (`config.json` `sandbox.<종류>`: 키 → 파일 지문) ──
// 파일이 하나라도 바뀌면 지문이 달라져 다시 묻는다

const CONSENT_SECTION: &str = "sandbox";

/// 확인되지 않은 구성요소에 사용자가 이 PC에서 따로 허락하는 것
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// 격리 준비가 실패했을 때 격리 없이 돌리기 (`sandbox.unisolated`)
    Unisolated,
    /// 익스텐션 GUI 번들을 GUI에 싣기 (`sandbox.extension_gui`). 번들은 데몬 API 전체를 부를 수 있다
    ExtensionGui,
}

impl Consent {
    fn key(self) -> &'static str {
        match self {
            Consent::Unisolated => "unisolated",
            Consent::ExtensionGui => "extension_gui",
        }
    }
}

/// 구성요소 폴더의 지문: 상대 경로와 내용 sha256을 정렬해 묶은 sha256 (`__pycache__` 제외)
pub fn fingerprint(dir: &Path) -> String {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if entry.file_name() != "__pycache__" {
                    walk(base, &path, out);
                }
            } else if kind.is_file() {
                let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                let sha = std::fs::read(&path).map(|b| sha256_of(&b)).unwrap_or_default();
                out.push(format!("{}\t{}", rel, sha));
            }
        }
    }
    let mut lines = Vec::new();
    walk(dir, dir, &mut lines);
    lines.sort();
    sha256_of(lines.join("\n").as_bytes())
}

fn consent_file() -> saba_chan_updater_lib::config_file::ConfigFile {
    saba_chan_updater_lib::config_file::ConfigFile::default_location()
}

pub fn unisolated_allowed(key: &str, dir: &Path) -> bool {
    consented(Consent::Unisolated, key, dir)
}

/// `key`에 지금 파일 그대로 동의했는가
pub fn consented(kind: Consent, key: &str, dir: &Path) -> bool {
    let section = consent_file().section(CONSENT_SECTION);
    section.get(kind.key()).and_then(|m| m.get(key)).and_then(Value::as_str).is_some_and(|fp| fp == fingerprint(dir))
}

/// 동의 목록: `[{component, fingerprint, current}]` (`current`: 지금 파일과 지문이 같은가. 다르면 다시 묻는다)
pub fn consent_list(kind: Consent) -> Vec<Value> {
    let section = consent_file().section(CONSENT_SECTION);
    let Some(entries) = section.get(kind.key()).and_then(Value::as_object) else { return Vec::new() };
    entries
        .iter()
        .map(|(key, fp)| {
            let current = component_dir(key).is_some_and(|d| fp.as_str() == Some(fingerprint(&d).as_str()));
            serde_json::json!({ "component": key, "fingerprint": fp, "current": current })
        })
        .collect()
}

/// 설치 기록 키의 폴더 (`module-<이름>` → modules/<이름>). 이름에 경로 문자가 있으면 None
pub fn component_dir(key: &str) -> Option<PathBuf> {
    let (root, name) = if let Some(name) = key.strip_prefix(saba_chan_updater_lib::relay::MODULE_PREFIX) {
        (crate::plugin::resolve_modules_dir(), name)
    } else {
        (crate::plugin::resolve_extensions_dir(), key.strip_prefix(saba_chan_updater_lib::relay::EXTENSION_PREFIX)?)
    };
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
        return None;
    }
    // 익스텐션 폴더는 id의 `-`를 `_`로 바꾼 이름일 수 있다 (Python 패키지 이름)
    [root.join(name), root.join(name.replace('-', "_"))].into_iter().find(|d| d.is_dir())
}

/// 동의를 기록하거나 지운다. 기록하면 지금 파일의 지문을 돌려준다
pub fn set_consent(kind: Consent, key: &str, allow: bool) -> std::io::Result<Option<String>> {
    let fp = if allow {
        let dir = component_dir(key).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "unknown component"))?;
        Some(fingerprint(&dir))
    } else {
        None
    };
    consent_file().update(|cfg| {
        let section = cfg.entry(CONSENT_SECTION).or_insert_with(|| serde_json::json!({}));
        if !section.is_object() {
            *section = serde_json::json!({});
        }
        let map = section.as_object_mut().expect("object");
        let entries = map.entry(kind.key()).or_insert_with(|| serde_json::json!({}));
        if !entries.is_object() {
            *entries = serde_json::json!({});
        }
        let entries = entries.as_object_mut().expect("object");
        match &fp {
            Some(fp) => {
                entries.insert(key.to_string(), Value::String(fp.clone()));
            }
            None => {
                entries.remove(key);
            }
        }
    })?;
    Ok(fp)
}

pub fn sha256_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtimes_the_daemon_knows_are_opened_to_the_sandbox() {
        let tmp = tempfile::tempdir().unwrap();
        let jdk = |name: &str, release: bool| {
            let home = tmp.path().join(name);
            std::fs::create_dir_all(home.join("bin")).unwrap();
            std::fs::write(home.join("bin").join("java"), b"x").unwrap();
            if release {
                std::fs::write(home.join("release"), b"JAVA_VERSION=\"21\"").unwrap();
            }
            home
        };
        // 입력은 사용자가 쓰는 모양 그대로 (Windows의 canonicalize는 `\\?\` 경로를 돌려준다), 결과는 실제 경로
        let real = |p: &PathBuf| std::fs::canonicalize(p).unwrap();
        let (set, env_home, on_path, tools) = (jdk("set", true), jdk("env", true), jdk("path", true), jdk("tools", false));
        let path = std::env::join_paths([on_path.join("bin"), tools.join("bin")]).unwrap();
        let java = set.join("bin").join("java");

        let homes = runtime_homes_in(java.to_str(), Some(env_home.as_os_str()), Some(&path), &[]);
        assert_eq!(homes, vec![real(&set), real(&env_home), real(&on_path)]);
        // 경로 없는 이름은 PATH에서 찾고, 같은 폴더는 한 번만
        assert_eq!(runtime_homes_in(Some("java"), None, Some(&path), &[]), vec![real(&on_path)]);
        // `release` 없는 도구 폴더는 열지 않는다
        let tools_path = std::env::join_paths([tools.join("bin")]).unwrap();
        assert_eq!(runtime_homes_in(None, None, Some(&tools_path), &[]), Vec::<PathBuf>::new());
        // 이름이 java가 아닌 실행 파일, 없는 파일은 런타임이 아니다
        std::fs::write(set.join("bin").join("evil"), b"x").unwrap();
        for bad in [set.join("bin").join("evil"), set.join("bin").join("missing")] {
            assert_eq!(runtime_homes_in(bad.to_str(), None, None, &[]), Vec::<PathBuf>::new());
        }
        // 그 JDK의 java는 실행 대상 검사도 통과한다
        let roots = LaunchRoots { dirs: vec![], runtimes: runtime_executables(vec![env_home.clone()]) };
        assert_eq!(check_launch(&env_home.join("bin").join("java").to_string_lossy(), &roots), Ok(()));
        assert_eq!(check_launch(&tools.join("bin").join("java").to_string_lossy(), &roots), Err("launch_outside_instance"));
    }

    #[test]
    fn every_real_runtime_counts_not_just_the_first_java_on_path() {
        let tmp = tempfile::tempdir().unwrap();
        let jdk = |dir: &Path, version: &str| {
            std::fs::create_dir_all(dir.join("bin")).unwrap();
            std::fs::write(dir.join("bin").join("java"), b"x").unwrap();
            let arch = if version.starts_with("1.8") { "x86" } else { "amd64" };
            std::fs::write(dir.join("release"), format!("IMPLEMENTOR=\"x\"\nJAVA_VERSION=\"{version}\"\nOS_ARCH=\"{arch}\"\n")).unwrap();
            std::fs::canonicalize(dir).unwrap()
        };
        // PATH 앞쪽의 바로가기(shim): `bin`·`release`가 없는 폴더의 java
        let shim = tmp.path().join("javapath");
        std::fs::create_dir_all(&shim).unwrap();
        std::fs::write(shim.join("java"), b"x").unwrap();
        let on_path = jdk(&tmp.path().join("jdk17"), "17.0.9");
        let path = std::env::join_paths([shim.clone(), on_path.join("bin")]).unwrap();
        // 알려진 설치 폴더 아래의 JDK (`<root>/<vendor>/<jdk>`)
        let roots = vec![tmp.path().join("pf")];
        let installed = jdk(&roots[0].join("Eclipse Adoptium").join("jdk-21"), "21.0.5");
        let old = jdk(&roots[0].join("Java").join("jre1.8.0_381"), "1.8.0_381");

        let homes = runtime_homes_in(None, None, Some(&path), &roots);
        assert_eq!(homes.len(), 3, "{homes:?}");
        assert_eq!(homes[0], on_path, "PATH 순서가 먼저");
        assert!(homes.contains(&installed) && homes.contains(&old));
        let runtimes = java_runtimes_in(None, Some(&path), &roots);
        let majors: Vec<(u32, &str, &str)> = runtimes.iter().map(|r| (r.major, r.version.as_str(), r.arch.as_str())).collect();
        assert_eq!(majors[0], (17, "17.0.9", "amd64"));
        assert!(majors.contains(&(21, "21.0.5", "amd64")) && majors.contains(&(8, "1.8.0_381", "x86")));
        assert!(runtimes.iter().all(|r| r.java.starts_with(&r.home)));
        // 일반 로컬 경로가 아닌 PATH 항목은 열어 보지 않는다 (Windows에서는 UNC 공유)
        let indirect = std::env::join_paths([tmp.path().join("javapath").join("..").join("jdk17").join("bin")]).unwrap();
        assert_eq!(find_on_path("java", Some(&indirect)), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_folder_an_unverified_module_returns_must_stay_in_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("inst");
        std::fs::create_dir_all(inst.join("server")).unwrap();
        std::fs::create_dir_all(tmp.path().join("Games")).unwrap();
        assert!(inside(&inst.join("server"), &inst));
        assert!(!inside(&tmp.path().join("Games"), &inst));
        assert!(!inside(&inst.join("server").join("..").join("..").join("Games"), &inst));
        assert!(!inside(&inst.join("missing"), &inst));
        // 인스턴스 폴더 안이어도 파일은 폴더로 받지 않는다
        std::fs::write(inst.join("server").join("run.exe"), b"x").unwrap();
        assert!(folder_inside(&inst.join("server"), &inst));
        assert!(!folder_inside(&inst.join("server").join("run.exe"), &inst));
    }

    #[test]
    fn only_plain_local_paths_reach_the_file_system() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(plain_local_path(tmp.path()));
        assert!(!plain_local_path(&tmp.path().join("..").join("x")));
        assert!(!plain_local_path(Path::new("bin/java")));
        #[cfg(windows)]
        for bad in [r"\\host\share\bin\java.exe", r"\\?\UNC\host\share\java.exe", r"\\.\C:\jdk\bin\java.exe"] {
            assert!(!plain_local_path(Path::new(bad)), "{bad}");
        }
        #[cfg(windows)]
        assert!(plain_local_path(Path::new(r"\\?\C:\jdk\bin\java.exe")));
    }

    #[test]
    fn launch_targets_must_stay_inside_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("inst");
        let outside = tmp.path().join("other");
        std::fs::create_dir_all(inst.join("bin")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let server = inst.join("bin").join("server.exe");
        let evil = outside.join("evil.exe");
        let java = outside.join("java.exe");
        for f in [&server, &evil, &java] {
            std::fs::write(f, b"x").unwrap();
        }
        let roots = LaunchRoots { dirs: vec![inst.clone()], runtimes: vec![java.clone()] };
        let s = |p: &Path| p.to_string_lossy().into_owned();

        assert_eq!(check_launch(&s(&server), &roots), Ok(()));
        assert_eq!(check_launch(&s(&evil), &roots), Err("launch_outside_instance"));
        // `..`로 빠져나가기
        assert_eq!(check_launch(&s(&inst.join("bin").join("..").join("..").join("other").join("evil.exe")), &roots), Err("launch_outside_instance"));
        // 설정한 java는 런타임으로 인정, PATH의 java도
        assert_eq!(check_launch(&s(&java), &roots), Ok(()));
        assert_eq!(check_launch("java", &roots), Ok(()));
        // 셸·스크립트는 인스턴스 안에 있어도 거부
        let bat = inst.join("start.bat");
        std::fs::write(&bat, b"x").unwrap();
        let cmd = inst.join("cmd.exe");
        std::fs::write(&cmd, b"x").unwrap();
        for p in [s(&bat), s(&cmd), "cmd.exe".into(), "powershell".into(), "python".into(), String::new()] {
            assert_eq!(check_launch(&p, &roots), Err("launch_outside_instance"), "{}", p);
        }
        // 없는 파일
        assert_eq!(check_launch(&s(&inst.join("missing.exe")), &roots), Err("launch_outside_instance"));
    }

    /// 상대 경로는 데몬 폴더가 아니라 작업 폴더 기준으로 풀어, 검사한 그 경로를 띄운다
    #[test]
    fn relative_programs_resolve_against_the_working_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("inst");
        std::fs::create_dir_all(inst.join("bin")).unwrap();
        std::fs::write(inst.join("bin").join("server.exe"), b"x").unwrap();
        let resolved = resolve_program(&format!("bin{}server.exe", std::path::MAIN_SEPARATOR), &inst);
        assert_eq!(Path::new(&resolved), inst.join("bin").join("server.exe"));
        let roots = LaunchRoots { dirs: vec![inst.clone()], runtimes: vec![] };
        assert_eq!(check_launch(&resolved, &roots), Ok(()));
        // 경로 없는 이름과 절대 경로는 그대로
        assert_eq!(resolve_program("java", &inst), "java");
        let abs = inst.join("bin").join("server.exe").to_string_lossy().into_owned();
        assert_eq!(resolve_program(&abs, tmp.path()), abs);
    }

    #[test]
    fn clean_verdicts_expire_but_detections_do_not() {
        let mut cache = VerdictCache::in_memory();
        cache.put("clean".into(), false);
        cache.put("bad".into(), true);
        let now = now_secs();
        assert_eq!(cache.get_at("clean", now), Some(false));
        assert_eq!(cache.get_at("clean", now + CLEAN_VERDICT_TTL_SECS), None);
        assert_eq!(cache.get_at("bad", now + CLEAN_VERDICT_TTL_SECS), Some(true));
        assert_eq!(cache.get_at("none", now), None);
    }

    #[test]
    fn legacy_bool_verdicts_keep_detections() {
        let entries = parse_entries(r#"{"bad":true,"clean":false,"new":{"detected":false,"at":5}}"#);
        assert!(entries["bad"].detected);
        assert_eq!((entries["clean"].detected, entries["clean"].at), (false, 0));
        assert_eq!(entries["new"].at, 5);
        assert!(parse_entries("not json").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_instance_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("inst");
        std::fs::create_dir_all(&inst).unwrap();
        let evil = tmp.path().join("evil");
        std::fs::write(&evil, b"x").unwrap();
        std::os::unix::fs::symlink(&evil, inst.join("server")).unwrap();
        let roots = LaunchRoots { dirs: vec![inst.clone()], runtimes: vec![] };
        assert_eq!(check_launch(&inst.join("server").to_string_lossy(), &roots), Err("launch_outside_instance"));
    }

    #[test]
    fn fingerprint_changes_with_any_file_but_not_bytecode_cache() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("lifecycle.py"), b"a").unwrap();
        let first = fingerprint(tmp.path());
        std::fs::create_dir_all(tmp.path().join("__pycache__")).unwrap();
        std::fs::write(tmp.path().join("__pycache__").join("x.pyc"), b"c").unwrap();
        assert_eq!(fingerprint(tmp.path()), first);
        std::fs::write(tmp.path().join("data.json"), b"{}").unwrap();
        assert_ne!(fingerprint(tmp.path()), first);
    }

    #[test]
    fn files_named_by_launch_arguments_are_found_for_scanning() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["server.jar", "agent.jar", "args.txt", "extra.jar"] {
            std::fs::write(tmp.path().join(f), b"x").unwrap();
        }
        let args: Vec<String> = ["-Xmx4G", "-javaagent:agent.jar", "@args.txt", "-cp=extra.jar", "-jar", "server.jar", "nogui", "missing.jar"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let found = argument_files(&args, tmp.path()).unwrap();
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["agent.jar", "args.txt", "extra.jar", "server.jar"]);
        // `..`가 든 인자도 글자로 푼 경로로 검사 목록에 든다
        std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
        let up = argument_files(&["-jar".to_string(), "sub/../server.jar".to_string()], tmp.path()).unwrap();
        assert_eq!(up, vec![tmp.path().join("server.jar")]);
        #[cfg(windows)]
        assert_eq!(argument_files(&[r"\\host\share\x.jar".to_string()], tmp.path()), Err(r"\\host\share\x.jar".to_string()));
    }

    /// 익스텐션 hook은 확인된 모듈의 인스턴스에도 불린다: 인스턴스 폴더는 읽기만, 쓰기는 전용 폴더에만
    #[test]
    fn only_the_instance_module_may_write_its_folder() {
        use appcontainer::Access;
        assert_eq!(instance_access("module-minecraft"), Access::Write);
        assert_eq!(instance_access("ext-steamcmd"), Access::Read);
        assert_eq!(instance_access("/somewhere/else"), Access::Read);
        let data = Path::new("data");
        assert_eq!(extension_data_dir(data, "ext-steamcmd"), Some(data.join("extension-data").join("steamcmd")));
        for key in ["module-minecraft", "ext-", "ext-../x", "ext-.hidden", "ext-a/b", "/somewhere/else"] {
            assert_eq!(extension_data_dir(data, key), None, "{key}");
        }
    }

    /// 폴더 권한은 프로필 SID에 남는다: 구성요소마다 프로필·캐시·임시 폴더가 달라야 서로의 권한을 쓰지 못한다
    #[test]
    fn each_component_gets_its_own_profile() {
        let a = appcontainer::profile_for("module-minecraft");
        let b = appcontainer::profile_for("ext-steamcmd");
        assert_ne!(a, b);
        assert_eq!(a, appcontainer::profile_for("module-minecraft"));
        assert_ne!(a, "saba-chan.untrusted");
        assert!(a.len() <= 64 && b.len() <= 64);
        assert_ne!(appcontainer::component_tag("module-minecraft"), appcontainer::component_tag("ext-steamcmd"));
    }

    #[test]
    fn component_keys_resolve_only_to_plain_names() {
        for key in ["module-", "module-../x", "module-a/b", "ext-.hidden", "other-x", "module-C:x"] {
            assert_eq!(component_dir(key), None, "{}", key);
        }
    }

    #[test]
    fn search_path_from_the_component_is_dropped() {
        let mut env = vec![("Path".to_string(), "C:\\evil".to_string()), ("pathext".into(), ".X".into()), ("JAVA_OPTS".into(), "-Xmx1G".into())];
        strip_search_path(&mut env);
        assert_eq!(env, vec![("JAVA_OPTS".to_string(), "-Xmx1G".to_string())]);
    }

    #[test]
    fn only_unofficial_components_are_restricted_and_only_on_windows() {
        assert!(!restricted(Trust::Verified));
        assert_eq!(restricted(Trust::Unverified), cfg!(windows));
        assert_eq!(restricted(Trust::Modified), cfg!(windows));
    }
}
