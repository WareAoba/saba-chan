//! 확인되지 않은 모듈·익스텐션 실행 제한 (Windows)
//!
//! 신뢰 표시(`integrity::trust_of`)가 `verified`가 아닌 구성요소에만 적용한다. 공식 카탈로그에서 받아
//! 그대로인 것은 지금처럼 실행한다.
//! - 실행 대상 검사: 모듈이 돌려준 서버 실행 파일이 인스턴스 폴더나 데몬이 아는 런타임 안에 있어야 한다.
//!   셸·스크립트 호스트(`cmd`, `powershell` 등)는 어디 있든 거부한다 (`launch_outside_instance`).
//! - 악성 코드 검사: Python·스크립트 파일은 AMSI(등록된 백신)로, 서버 실행 파일은 처음 실행 전에
//!   Defender `MpCmdRun`으로 본다. 찾으면 실행하지 않는다 (`malware_detected`). 결과는 sha256으로 저장해
//!   바뀌지 않은 파일은 다시 보지 않는다. 검사기가 없으면 건너뛰고 로그만 남긴다.
//!
//! Linux·macOS는 아직 지원 대상이 아니라 지금처럼 실행한다.

use saba_chan_updater_lib::integrity::Trust;
use serde_json::Value;
use std::collections::BTreeMap;
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
        "launch_outside_instance" => format!("This unverified module tried to run a program outside the server folder: {}", file),
        "unscannable_file" => format!("This unverified component contains compiled code without source that cannot be scanned: {}", file),
        "sandbox_unavailable" => format!("Could not prepare the isolated environment for this unverified component: {}", blocked.detail.as_deref().unwrap_or("")),
        other => other.to_string(),
    }
}

/// sha256 → 검사 결과 (`<data dir>/scan-verdicts.json`). 깨끗하다/찾았다만 저장한다
#[derive(Debug, Default)]
pub struct VerdictCache {
    path: Option<PathBuf>,
    entries: BTreeMap<String, bool>,
}

impl VerdictCache {
    pub fn load() -> Self {
        let path = saba_chan_updater_lib::constants::resolve_data_dir().join("scan-verdicts.json");
        let entries = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self { path: Some(path), entries }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Some(true) = 찾았다, Some(false) = 깨끗하다
    pub fn get(&self, sha: &str) -> Option<bool> {
        self.entries.get(sha).copied()
    }

    pub fn put(&mut self, sha: String, detected: bool) {
        self.entries.insert(sha, detected);
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

/// Python 실행 전: 확인되지 않은 구성요소면 스크립트를 검사한다
pub async fn guard_entry(entry: &str) -> Result<(), Blocked> {
    let entry = PathBuf::from(entry);
    let trust = trust_of_entry(&entry);
    if !restricted(trust) {
        return Ok(());
    }
    let Some(dir) = entry.parent().map(Path::to_path_buf) else { return Ok(()) };
    tokio::task::spawn_blocking(move || {
        let mut cache = VerdictCache::load();
        let result = scan::scan_component(&dir, &scan::SystemScanner, &mut cache);
        cache.save();
        result
    })
    .await
    .unwrap_or(Ok(()))
}

/// 서버 실행 전: 확인되지 않은 모듈이면 실행 대상 위치를 보고 실행 파일을 검사한다
pub async fn guard_launch(entry: &str, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<(), Blocked> {
    guard_launch_for(trust_of_entry(Path::new(entry)), program, args, cwd, roots, env).await
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

/// `guard_launch`의 신뢰 표시를 받는 형태 (익스텐션이 띄우는 프로그램에도 쓴다)
pub async fn guard_launch_for(trust: Trust, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<(), Blocked> {
    if !restricted(trust) {
        return Ok(());
    }
    strip_search_path(env);
    if let Err(code) = check_launch(program, &roots) {
        return Err(Blocked { code, file: Some(program.to_string()), detail: None });
    }
    // 실행 파일(경로 없는 java는 PATH의 런타임이라 빼고)과 인자로 넘기는 파일(`-jar x.jar`, `@args.txt` 등)
    let mut files: Vec<PathBuf> = Some(PathBuf::from(program)).filter(|p| p.is_absolute()).into_iter().collect();
    files.extend(argument_files(args, cwd));
    tokio::task::spawn_blocking(move || {
        let mut cache = VerdictCache::load();
        let result = files.iter().try_for_each(|f| scan::scan_executable(f, &scan::SystemScanner, &mut cache));
        cache.save();
        result
    })
    .await
    .unwrap_or(Ok(()))
}

/// 실행 인자 중 있는 파일을 가리키는 것: 인자 그대로, `@파일`, `-옵션=파일`·`-옵션:파일` (상대 경로는 `cwd` 기준)
fn argument_files(args: &[String], cwd: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for arg in args {
        let mut candidates = vec![arg.as_str(), arg.trim_start_matches('@')];
        if arg.starts_with('-') {
            candidates.extend(arg.split_once('=').map(|(_, v)| v));
            candidates.extend(arg.split_once(':').map(|(_, v)| v));
        }
        for c in candidates.into_iter().filter(|c| !c.is_empty()) {
            let path = cwd.join(c);
            if path.is_file() && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// 격리해서 돌릴 Python 실행 하나
#[derive(Debug, Clone)]
pub struct Isolation {
    /// 설치 기록 키 (`module-<이름>`·`ext-<이름>`). 모듈·익스텐션 폴더 밖이면 그 폴더 경로
    pub key: String,
    pub dir: PathBuf,
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

/// 확인되지 않은 구성요소의 Python 실행이면 격리 계획을 만든다. `config`의 `instance_dir`·`working_dir`에
/// 쓰기를 준다 (데몬이 넣는 값. 데이터 폴더 자체와 그 위는 `Grants::push`가 버린다)
pub fn isolation_for(entry: &Path, config: &Value, python_exe: &Path) -> Option<Isolation> {
    use appcontainer::Access;
    if !restricted(trust_of_entry(entry)) {
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
    let pycache = raw_data_dir.join("pycache-sandbox");
    let temp = raw_data_dir.join("sandbox-temp");
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
            add(Path::new(path), access);
        }
    }
    let data = extension_data_dir(&raw_data_dir, &key);
    if let Some(d) = &data {
        let _ = std::fs::create_dir_all(d);
        add(d, Access::Write);
    }
    Some(Isolation { key, dir, grants, pycache, temp, data })
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
        let grants = iso.grants.clone();
        let started = tokio::task::spawn_blocking(move || appcontainer::spawn(&program, &args, cwd.as_deref(), &env, &grants))
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

// ── 격리 없이 돌리기 동의 (`config.json` `sandbox.unisolated`: 키 → 파일 지문) ──
// 파일이 하나라도 바뀌면 지문이 달라져 다시 묻는다

const CONSENT_SECTION: &str = "sandbox";
const CONSENT_KEY: &str = "unisolated";

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
    let section = consent_file().section(CONSENT_SECTION);
    section.get(CONSENT_KEY).and_then(|m| m.get(key)).and_then(Value::as_str).is_some_and(|fp| fp == fingerprint(dir))
}

/// 동의 목록: `[{component, fingerprint, current}]` (`current`: 지금 파일과 지문이 같은가. 다르면 다시 묻는다)
pub fn unisolated_list() -> Vec<Value> {
    let section = consent_file().section(CONSENT_SECTION);
    let Some(entries) = section.get(CONSENT_KEY).and_then(Value::as_object) else { return Vec::new() };
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
    Some(root.join(name)).filter(|d| d.is_dir())
}

/// 동의를 기록하거나 지운다. 기록하면 지금 파일의 지문을 돌려준다
pub fn set_unisolated(key: &str, allow: bool) -> std::io::Result<Option<String>> {
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
        let entries = map.entry(CONSENT_KEY).or_insert_with(|| serde_json::json!({}));
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
        let found = argument_files(&args, tmp.path());
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["agent.jar", "args.txt", "extra.jar", "server.jar"]);
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
