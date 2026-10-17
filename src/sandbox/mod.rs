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
pub async fn guard_launch(entry: &str, program: &str, roots: LaunchRoots) -> Result<(), Blocked> {
    if !restricted(trust_of_entry(Path::new(entry))) {
        return Ok(());
    }
    if let Err(code) = check_launch(program, &roots) {
        return Err(Blocked { code, file: Some(program.to_string()), detail: None });
    }
    let path = PathBuf::from(program);
    if !path.is_absolute() {
        return Ok(()); // PATH의 java
    }
    tokio::task::spawn_blocking(move || {
        let mut cache = VerdictCache::load();
        let result = scan::scan_executable(&path, &scan::SystemScanner, &mut cache);
        cache.save();
        result
    })
    .await
    .unwrap_or(Ok(()))
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
    fn only_unofficial_components_are_restricted_and_only_on_windows() {
        assert!(!restricted(Trust::Verified));
        assert_eq!(restricted(Trust::Unverified), cfg!(windows));
        assert_eq!(restricted(Trust::Modified), cfg!(windows));
    }
}
