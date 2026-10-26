//! 확인되지 않은 모듈·익스텐션 실행 제한 (Windows: AppContainer, 리눅스: bubblewrap `bwrap.rs`)
//!
//! 신뢰 표시(`integrity::trust_of`)가 `verified`가 아닌 구성요소에만 적용한다. 공식 카탈로그에서 받아
//! 그대로인 것은 지금처럼 실행한다.
//! - 실행 대상 검사: 모듈이 돌려준 서버 실행 파일이 인스턴스 폴더나 데몬이 아는 런타임 안에 있어야 한다.
//!   셸·스크립트 호스트(`cmd`, `powershell` 등)는 어디 있든 거부한다 (`launch_outside_instance`).
//! - 백신 검사는 하지 않는다 (Defender·AMSI 검사는 오판정과 지연이 많아 뺐다). 격리 실행과 실행 대상 검사로 막는다.
//!
//! 격리할 수 없는 플랫폼은 파일 지문에 대한 사용자 동의가 있어야 미확인 코드를 실행한다.

use saba_chan_updater_lib::integrity::Trust;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub mod appcontainer;
pub mod bwrap;

/// 이 플랫폼에서 제한을 거는가 (Windows, bubblewrap을 쓸 수 있는 리눅스)
pub fn enforced() -> bool {
    cfg!(windows) || bwrap::program().is_some()
}

/// 모듈·익스텐션 진입 파일(`<modules>/<이름>/lifecycle.py`, `<modules>/<이름>/scripts/lifecycle.py` 등)의
/// 설치 기록 키와 폴더
pub fn component_of(entry: &Path) -> Option<(String, PathBuf)> {
    component_in(entry, &crate::plugin::resolve_modules_dir(), &crate::plugin::resolve_extensions_dir())
}

/// 진입 파일의 조상 중 모듈·익스텐션 폴더 바로 아래 것이 그 구성요소 폴더다 (진입 파일이 하위 폴더에 있어도)
fn component_in(entry: &Path, modules: &Path, extensions: &Path) -> Option<(String, PathBuf)> {
    // 조상을 글자로 거슬러 오르므로 `..`·`.`가 있으면 실제 위치와 다른 구성요소로 판정될 수 있다: 어느 것에도 속하지 않는다
    if entry.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return None;
    }
    let roots = [
        (std::fs::canonicalize(modules).ok(), saba_chan_updater_lib::relay::MODULE_PREFIX),
        (std::fs::canonicalize(extensions).ok(), saba_chan_updater_lib::relay::EXTENSION_PREFIX),
    ];
    for dir in entry.parent()?.ancestors() {
        let (Some(name), Some(parent)) = (dir.file_name(), dir.parent()) else { continue };
        let Ok(parent) = std::fs::canonicalize(parent) else { continue };
        for (root, prefix) in &roots {
            if root.as_ref() == Some(&parent) {
                let name = name.to_string_lossy();
                let id = if *prefix == saba_chan_updater_lib::relay::EXTENSION_PREFIX {
                    extension_id_in(dir, &name).unwrap_or_else(|| name.to_string())
                } else {
                    name.to_string()
                };
                return Some((format!("{}{}", prefix, id), dir.to_path_buf()));
            }
        }
    }
    None
}

/// 익스텐션 폴더의 manifest id. 폴더 이름은 id의 '-'를 '_'로 바꾼 것이라 신뢰 기록의 키(id)와 다를 수 있다.
/// 그 폴더 이름에 맞지 않는 id(다른 익스텐션을 사칭)는 받지 않는다
fn extension_id_in(dir: &Path, folder: &str) -> Option<String> {
    let content = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).ok()?;
    let id = manifest.get("id")?.as_str()?;
    (id.replace('-', "_") == folder).then(|| id.to_string())
}

/// 진입 파일의 신뢰 표시. 모듈·익스텐션 폴더 밖이면 확인되지 않은 것으로 본다
pub fn trust_of_entry(entry: &Path) -> Trust {
    match component_of(entry) {
        Some((key, dir)) => saba_chan_updater_lib::integrity::trust_of(&key, &dir),
        None => Trust::Unverified,
    }
}

/// 제한을 거는가 (격리하는 플랫폼이고 공식·그대로가 아닌 것)
pub fn restricted(trust: Trust) -> bool {
    trust != Trust::Verified && enforced()
}

pub fn python_isolation_required(trust: Trust) -> bool {
    trust != Trust::Verified
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
        let mut response = serde_json::json!({
            "success": false,
            "action_required": self.code,
            "message": message,
            "file": self.file,
            "detail": self.detail,
        });
        if self.code == "sandbox_unavailable" && !enforced() {
            response["error_code"] = serde_json::json!("isolation_unsupported");
        }
        response
    }
}

/// 막은 이유 문구 (영어 기본. GUI·CLI는 `action_required` 코드로 현지화한다)
pub fn message(blocked: &Blocked) -> String {
    let file = blocked.file.as_deref().unwrap_or("");
    match blocked.code {
        "launch_outside_instance" => format!("This unverified module tried to run a program outside the server folder: {}. If you set the Java path yourself, choose it again in the server settings.", file),
        "server_in_sync_folder" => format!("The server folder is inside a sync folder such as OneDrive, so saba-chan cannot check it: {}. Move the server folder outside the sync folder.", file),
        "server_folder_label" => format!("Could not remove the low integrity mark from the server folder: {}. Check that this account can change the permissions of that folder.", file),
        "sandbox_unavailable" if !enforced() => "This operating system cannot isolate this unverified component. Consent is required to run it without isolation.".to_string(),
        "sandbox_unavailable" => format!("Could not prepare the isolated environment for this unverified component: {}", blocked.detail.as_deref().unwrap_or("")),
        other => other.to_string(),
    }
}

/// `clear_low_labels` 오류 문구에 붙는 표시: 그 경로가 동기화 폴더(OneDrive 등)의 클라우드 파일이다
pub const SYNC_FOLDER_MARK: &str = "is a cloud file of a sync folder";

/// 표시를 걷지 못한 경로들로 막는 이유를 고른다. 동기화 폴더의 클라우드 파일이 섞여 있으면 그것이 원인이다
/// (재분석 지점이라 표시를 읽지 못한다. 사용자 결정으로 막는 것은 그대로 두고, 폴더를 옮기라고 알린다)
#[cfg_attr(not(windows), allow(dead_code))] // 표시 걷기는 Windows에서만 한다
fn folder_label_block(stuck: &[(PathBuf, String)]) -> Blocked {
    let detail = stuck.iter().map(|(p, e)| format!("{}: {}", p.display(), e)).collect::<Vec<_>>().join("; ");
    let (code, first) = match stuck.iter().find(|(_, e)| e.contains(SYNC_FOLDER_MARK)) {
        Some((path, _)) => ("server_in_sync_folder", Some(path)),
        None => ("server_folder_label", stuck.first().map(|(p, _)| p)),
    };
    Blocked { code, file: first.map(|p| p.display().to_string()), detail: Some(detail) }
}

/// 서버 폴더의 표시를 걷은 뒤 서버를 띄울 때까지 쥐는 잠금 (`guard_launch_for`가 돌려준다). 격리 실행은 권한을 주기
/// 전에 이 잠금을 잡으므로(`spawn_isolated`), 걷은 뒤 서버가 뜨기 전에 표시가 다시 붙지 않는다. 서버를 띄운 뒤 놓는다
pub struct LaunchGate(#[allow(dead_code)] tokio::sync::MutexGuard<'static, ()>);

static LABEL_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 서버 실행 전: 격리 실행이 서버 폴더에 남긴 낮은 수준 표시를 그 안 실행 파일에서 덮는다 (Windows). 그 표시가 걸린
/// 실행 파일로 띄운 서버는 낮은 무결성으로 돌아 뜨지 못한다. 신뢰 표시와 상관없이 한다: 0.1.6까지는 공식 모듈도 격리되어 표시가
/// 남아 있을 수 있다. 한 폴더에서 실패해도 나머지를 마저 보고, 낮은 수준이 남은 실행 파일이 있으면 `server_folder_label`로 막는다
/// `program`: 띄울 파일. 서버 폴더 안에 있으면 확장자와 상관없이 그 파일의 표시도 덮는다 (L38)
async fn restore_server_folders(dirs: Vec<PathBuf>, program: Option<PathBuf>) -> Result<(), Blocked> {
    #[cfg(windows)]
    {
        let stuck = tokio::task::spawn_blocking(move || {
            let mut stuck = Vec::new();
            let inside = |file: &Path| {
                let Ok(file) = std::fs::canonicalize(file) else { return false };
                dirs.iter().filter_map(|d| std::fs::canonicalize(d).ok()).any(|d| file.starts_with(d))
            };
            if let Some(program) = program.filter(|p| p.is_file() && inside(p)) {
                match appcontainer::clear_low_label_of(&program) {
                    Ok(true) => tracing::info!("[Sandbox] set {} to medium integrity", program.display()),
                    Ok(false) => {}
                    Err(failed) => stuck.extend(failed),
                }
            }
            for dir in dirs.iter().filter(|d| d.is_dir()) {
                let started = std::time::Instant::now();
                match appcontainer::clear_low_labels(dir) {
                    // 덮을 것이 없어도 폴더 전체를 훑으므로 큰 서버 폴더는 시간이 든다: 오래 걸리면 남긴다
                    Ok(0) if started.elapsed() >= std::time::Duration::from_millis(500) => tracing::info!(
                        "[Sandbox] checked {} for low integrity labels (none) in {} ms",
                        dir.display(),
                        started.elapsed().as_millis()
                    ),
                    Ok(0) => {}
                    Ok(n) => tracing::info!(
                        "[Sandbox] set {} executable(s) under {} to medium integrity in {} ms",
                        n,
                        dir.display(),
                        started.elapsed().as_millis()
                    ),
                    Err(failed) => stuck.extend(failed),
                }
            }
            stuck
        })
        .await
        .unwrap_or_else(|e| vec![(PathBuf::new(), e.to_string())]);
        if !stuck.is_empty() {
            let blocked = folder_label_block(&stuck);
            tracing::warn!("[Sandbox] could not remove the low integrity label: {}", blocked.detail.as_deref().unwrap_or(""));
            return Err(blocked);
        }
    }
    #[cfg(not(windows))]
    let _ = (dirs, program);
    Ok(())
}

/// 서버 실행 전: 확인되지 않은 모듈이면 실행 대상 위치를 보고 실행 파일을 검사한다. 띄울 프로그램 경로를 돌려준다
pub async fn guard_launch(entry: &str, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<(String, LaunchGate), Blocked> {
    guard_launch_for(|| trust_of_entry(Path::new(entry)), program, args, cwd, roots, env).await
}

/// 익스텐션 설치 기록의 신뢰 표시 (`extensions/<id의 - 를 _로 바꾼 이름>`)
pub fn extension_trust(ext_id: &str) -> Trust {
    let key = format!("{}{}", saba_chan_updater_lib::relay::EXTENSION_PREFIX, ext_id);
    saba_chan_updater_lib::integrity::trust_of(&key, &extension_dir_in(&crate::plugin::resolve_extensions_dir(), ext_id))
}

pub fn extension_unisolated_allowed(ext_id: &str) -> bool {
    let dir = extension_dir_in(&crate::plugin::resolve_extensions_dir(), ext_id);
    unisolated_allowed(&format!("{}{}", saba_chan_updater_lib::relay::EXTENSION_PREFIX, ext_id), &dir)
}

/// 익스텐션 폴더: 설치는 id의 '-'를 '_'로 바꾼 이름에 한다. 그 폴더가 없으면 id 그대로 (예전 설치)
fn extension_dir_in(extensions: &Path, ext_id: &str) -> PathBuf {
    let safe = extensions.join(ext_id.replace('-', "_"));
    if safe.is_dir() { safe } else { extensions.join(ext_id) }
}

/// 실행 대상을 찾을 때 쓰는 변수. 확인되지 않은 구성요소가 준 값은 버린다
/// (경로 없는 `java`가 그 폴더에서 찾아지지 않게)
const SEARCH_PATH_VARS: &[&str] = &["PATH", "PATHEXT"];

pub fn strip_search_path(env: &mut Vec<(String, String)>) {
    env.retain(|(k, _)| !SEARCH_PATH_VARS.iter().any(|v| k.eq_ignore_ascii_case(v)));
}

/// `guard_launch`의 신뢰 표시를 받는 형태 (익스텐션이 띄우는 프로그램에도 쓴다).
/// `trust`는 제한하는 플랫폼에서만 부른다. 제한하면 상대 경로를 작업 폴더 기준으로 푼 경로를 돌려준다.
/// 함께 돌려주는 `LaunchGate`는 서버 프로세스를 띄운 직후 놓는다
pub async fn guard_launch_for(trust: impl FnOnce() -> Trust, program: &str, args: &[String], cwd: &Path, roots: LaunchRoots, env: &mut Vec<(String, String)>) -> Result<(String, LaunchGate), Blocked> {
    // 모듈이 띄우든 익스텐션이 띄우든 서버를 실행하는 모든 길이 여기를 지난다
    let gate = LaunchGate(LABEL_GATE.lock().await);
    let target = Path::new(&resolve_program(program, cwd)).to_path_buf();
    restore_server_folders(roots.dirs.clone(), target.is_absolute().then_some(target)).await?;
    if !restricted(trust()) {
        return Ok((program.to_string(), gate));
    }
    strip_search_path(env);
    let program = resolve_program(program, cwd);
    if let Err(code) = check_launch(&program, &roots) {
        return Err(Blocked { code, file: Some(program), detail: None });
    }
    // 인자로 넘기는 파일(`-jar x.jar`, `@args.txt` 등)도 폴더 밖 경로·특수 경로를 쓰지 않아야 한다.
    // Java가 인자처럼 읽는 `@파일`의 내용과 JVM 옵션 환경 변수도 같이 본다
    if let Err(arg) = argument_files(args, cwd).and_then(|_| argument_files(&indirect_arguments(args, env, cwd)?, cwd)) {
        return Err(Blocked { code: "launch_outside_instance", file: Some(arg), detail: None });
    }
    Ok((program, gate))
}

/// Java가 실행 인자처럼 읽는 환경 변수 (`CLASSPATH`는 값 전체가 경로 목록 하나)
const JVM_OPTION_VARS: &[&str] = &["JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS"];

/// 이보다 큰 인자 파일은 읽지 않고 막는다 (인자 파일은 짧다)
const ARGUMENT_FILE_LIMIT: u64 = 1 << 20;

/// 인자 밖에서 Java가 인자로 읽는 것: JVM 옵션 환경 변수, `CLASSPATH`(경로 목록 하나), 그리고 인자·환경 변수가 가리키는
/// 인자 파일(`@파일`, `-XX:VMOptionsFile=파일`)의 내용. `argument_files`가 인자처럼 검사한다. 인자 파일은 한 단계만 읽는다
/// (Java도 인자 파일 안의 `@`를 다시 풀지 않는다). 로컬 경로가 아니거나 너무 크거나 읽지 못하는 인자 파일은 그 인자로 `Err`
fn indirect_arguments(args: &[String], env: &[(String, String)], cwd: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (key, value) in env {
        if JVM_OPTION_VARS.iter().any(|v| key.eq_ignore_ascii_case(v)) {
            out.extend(java_tokens(value));
        } else if key.eq_ignore_ascii_case("CLASSPATH") {
            out.push(value.clone());
        }
    }
    let named: Vec<String> = args.iter().chain(out.iter()).cloned().collect();
    for arg in &named {
        let Some(file) = arg.strip_prefix('@').or_else(|| arg.strip_prefix("-XX:VMOptionsFile=")) else { continue };
        let path = lexical(&cwd.join(file));
        if !plain_local_path(&path) {
            return Err(arg.clone());
        }
        // 없는 파일은 Java가 실행하지 못한다
        if !path.exists() {
            continue;
        }
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= ARGUMENT_FILE_LIMIT) {
            return Err(arg.clone());
        }
        let text = std::fs::read(&path).map_err(|_| arg.clone())?;
        out.extend(java_tokens(&String::from_utf8_lossy(&text)));
    }
    Ok(out)
}

/// Java가 인자 파일·옵션 변수를 나누는 대로 나눈 낱말. 공백으로 나누고, 낱말 어디서든 `"`·`'`로 감싼 부분은 따옴표를 벗겨
/// 이어 붙이며, 낱말 밖의 `#`부터 줄 끝은 주석이다. 따옴표 안의 `\`를 이스케이프로 보는 규칙(`@파일`·`JDK_JAVA_OPTIONS`)과
/// 글자 그대로 보는 규칙(`JAVA_TOOL_OPTIONS`·`VMOptionsFile`)이 달라, 두 가지로 나눈 낱말을 모두 돌려준다 (검사는 넓게 막는 쪽)
fn java_tokens(text: &str) -> Vec<String> {
    let mut out = split_java(text, true);
    out.extend(split_java(text, false).into_iter().filter(|t| !out.contains(t)).collect::<Vec<_>>());
    out
}

fn split_java(text: &str, escapes: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut in_word, mut quote) = (false, None::<char>);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) if escapes && c == '\\' => match chars.next() {
                // 줄 이음: 다음 줄 앞 공백까지 건너뛴다
                Some('\r' | '\n') => {
                    while chars.next_if(|n| n.is_whitespace()).is_some() {}
                }
                Some('n') => word.push('\n'),
                Some('t') => word.push('\t'),
                Some('r') => word.push('\r'),
                Some('f') => word.push('\u{c}'),
                Some(n) => word.push(n),
                None => {}
            },
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None if c == '#' && !in_word => while chars.next_if(|n| *n != '\n').is_some() {},
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
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
/// 클래스 경로처럼 경로 목록이면(Windows `;`) 항목마다 본다.
/// 로컬 드라이브가 아닌 경로(UNC 등)를 가리키는 인자는 묻지 않고 그 인자를 돌려준다 (`Err`)
fn argument_files(args: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for arg in args {
        let mut candidates = vec![arg.as_str(), arg.trim_start_matches('@')];
        if arg.starts_with('-') {
            candidates.extend(arg.split_once('=').map(|(_, v)| v));
            candidates.extend(arg.split_once(':').map(|(_, v)| v));
        }
        let listed: Vec<String> = candidates
            .iter()
            .filter(|c| cfg!(windows) && c.contains(';'))
            .flat_map(|c| c.split(';').map(str::to_string).collect::<Vec<_>>())
            .collect();
        for c in candidates.into_iter().chain(listed.iter().map(String::as_str)).filter(|c| !c.is_empty()) {
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
    /// 이 실행이 import한 익스텐션들이 쓸 폴더의 뿌리 (`extension_data_root`, 환경 변수 `SABA_EXTENSION_DATA_ROOT`)
    pub extension_root: PathBuf,
}

/// 데몬이 lifecycle 호출에 넣는 인스턴스 폴더 키 (설치·업데이트 호출은 `install_dir`만 넣는다)
const INSTANCE_DIR_FIELDS: [&str; 3] = ["instance_dir", "working_dir", "install_dir"];

/// 인스턴스 폴더 안의 모듈 데이터 폴더 목록 키 (`Supervisor::insert_instance_fields`가 만든다. 좀보이드 cache_dir 등)
pub const INSTANCE_DATA_DIRS: &str = "instance_data_dirs";

/// 격리 실행이 이 config 키의 폴더에 권한을 받는가. 이 키들은 데몬만 정하고 모듈 설정에서 싣지 않는다
pub fn grants_write(key: &str) -> bool {
    INSTANCE_DIR_FIELDS.contains(&key) || key == INSTANCE_DATA_DIRS || key == MIGRATION_IMPORT
}

/// 가져오기(마이그레이션)의 `import_settings` 호출에만 데몬이 싣는 표시 (`true`). 이 호출만 인스턴스 폴더 자체에
/// 쓰기를 받는다(가져온 서버의 포터블 런타임을 서버 폴더 대신 인스턴스 폴더에 받는다). 다른 호출은 읽기만 한다
pub const MIGRATION_IMPORT: &str = "migration_import";

/// config의 인스턴스 폴더 키(`field`)에 줄 권한: `instance_dir` 쓰기는 가져오기의 `import_settings`에만
fn field_access(field: &str, access: appcontainer::Access, config: &Value) -> appcontainer::Access {
    let import = config.get(MIGRATION_IMPORT).and_then(Value::as_bool) == Some(true);
    if field == "instance_dir" && access == appcontainer::Access::Write && !import {
        appcontainer::Access::Read
    } else {
        access
    }
}

/// 격리 실행이 config의 인스턴스 폴더(`INSTANCE_DIR_FIELDS`)에 받는 권한: 모듈은 쓰기, 그 밖(익스텐션 등)은 읽기
fn instance_access(key: &str) -> appcontainer::Access {
    if key.starts_with(saba_chan_updater_lib::relay::MODULE_PREFIX) {
        appcontainer::Access::Write
    } else {
        appcontainer::Access::Read
    }
}

/// 인스턴스 폴더 안의 데몬 항목 중 비밀이 든 것. 격리 실행에 인스턴스 폴더 권한을 줄 때 상속을 끊어 둔다
/// (비밀 파일은 저장할 때마다 `Mode::Secret`이 다시 끊는다. 여기서는 예전에 저장된 것과 백업 폴더,
/// 가져온 서버의 원래 설정 보관본을 막는다)
const PROTECTED_INSTANCE_ENTRIES: [&str; 5] = ["instance.json", "settings.json", "rcon.json", "backups", crate::instance::migration::DIR];

/// 하나라도 끊지 못하면 false: 그때는 인스턴스 폴더 권한을 주지 않는다 (상속으로 비밀이 열리지 않게)
fn protect_daemon_entries(instance_dir: &Path) -> bool {
    let mut protected = true;
    for name in PROTECTED_INSTANCE_ENTRIES {
        let path = instance_dir.join(name);
        if path.exists() {
            if let Err(e) = saba_atomic_file::restrict_to_user(&path) {
                tracing::warn!("[Sandbox] cannot restrict {}, not granting the instance folder: {}", path.display(), e);
                protected = false;
            }
        }
    }
    protected
}

/// 익스텐션 키면 그 익스텐션 전용 데이터 폴더
fn extension_data_dir(data_dir: &Path, key: &str) -> Option<PathBuf> {
    let name = key.strip_prefix(saba_chan_updater_lib::relay::EXTENSION_PREFIX)?;
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
        return None;
    }
    Some(data_dir.join("extension-data").join(name))
}

/// import된 익스텐션이 `<뿌리>/<id>`에 받은 파일을 두는 뿌리. 격리된 실행(`isolated_tag`)은 구성요소마다 따로 둔다:
/// 공용 폴더에 쓰게 하면 확인된 실행이 나중에 그 파일을 격리 없이 실행할 수 있다
pub fn extension_data_root(data_dir: &Path, isolated_tag: Option<&str>) -> PathBuf {
    match isolated_tag {
        Some(tag) => data_dir.join("sandbox-data").join(tag).join("extension-data"),
        None => data_dir.join("extension-data"),
    }
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
    if !cfg!(windows) || !restricted(trust_of_entry(entry)) {
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

/// 제거기가 격리 실행의 흔적을 거둘 폴더 (`saba-core sandbox reset`): 인스턴스 폴더·작업 폴더·실행 파일 폴더(`dirs`)와
/// 그 바로 위(목록 권한, `isolation_for`), 데몬 Python의 기반 설치. 인스턴스를 남겨도 그 폴더의 권한은 거둔다
pub fn reset_targets(dirs: &[PathBuf], python_base: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = dirs.iter().flat_map(|d| [Some(d.clone()), d.parent().map(Path::to_path_buf)]).flatten().collect();
    out.extend(python_base);
    out.retain(|p| !p.as_os_str().is_empty());
    out.sort();
    out.dedup();
    out
}

/// 인스턴스를 지우며 남는 폴더(가져온 서버 폴더·실행 파일 폴더, `dirs`)에서 거둘 곳: 그 폴더들과 바로 위(목록 권한).
/// 바로 위 폴더는 남는 다른 인스턴스의 폴더(`others`)도 그 아래에 있으면 그쪽이 아직 쓰므로 둔다
pub fn release_targets(dirs: &[PathBuf], others: &[PathBuf]) -> Vec<PathBuf> {
    let shared = |parent: &Path| others.iter().any(|o| o.parent() == Some(parent));
    let mut out: Vec<PathBuf> = dirs.iter().filter(|d| !d.as_os_str().is_empty()).cloned().collect();
    out.extend(dirs.iter().filter_map(|d| d.parent()).filter(|p| !p.as_os_str().is_empty() && !shared(p)).map(Path::to_path_buf));
    out.sort();
    out.dedup();
    out
}

/// 인스턴스를 지울 때 남는 폴더(`dirs`)에서 격리 실행의 흔적을 거둔다: 낮은 무결성 표시를 덮고,
/// 사바쨩 프로필들의 명시 ACE를 뺀다(프로필은 남는다: 같은 모듈의 다른 인스턴스가 쓴다). Windows만 할 일이 있다
pub fn release(dirs: &[PathBuf], others: &[PathBuf]) -> Result<(), Vec<String>> {
    let targets = release_targets(dirs, others);
    #[cfg(windows)]
    {
        appcontainer::release(&targets, dirs)
    }
    #[cfg(not(windows))]
    {
        let _ = targets;
        Ok(())
    }
}

/// 다른 위 폴더 아래 있다가 옮겨 온 폴더(가져오기를 거두며 원래 자리로 돌려놓은 서버 폴더)가 지금 위 폴더에서 다시
/// 물려받게 한다: 예전 자리에서 물려받은 권한·낮은 수준 표시를 걷고 원래 자리의 상속 권한을 되찾는다. Windows만 할 일이 있다
pub fn adopt_parent(dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        appcontainer::adopt_parent(dir)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(())
    }
}

/// `saba-core sandbox reset`: 인스턴스 기록에서 폴더를 모아 격리 실행의 프로필·권한·표시를 거둔다. Windows만 할 일이 있다.
/// 지운 프로필 수, 또는 거두지 못한 것
pub fn reset() -> Result<usize, Vec<String>> {
    let root = saba_chan_updater_lib::constants::resolve_instances_dir();
    let mut dirs: Vec<PathBuf> = Vec::new();
    // 기록 폴더가 없으면 만들지 않는다 (InstanceStore::load는 루트를 만든다)
    if root.is_dir() {
        let mut store = crate::instance::InstanceStore::new(&root.to_string_lossy());
        if let Err(e) = store.load() {
            return Err(vec![format!("read instances: {e:#}")]);
        }
        for instance in store.list() {
            dirs.push(store.instance_dir(&instance.id));
            dirs.extend(instance.working_dir.as_deref().filter(|d| !d.is_empty()).map(PathBuf::from));
            dirs.extend(instance.executable_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf));
        }
    }
    let base = crate::python_env::venv_python().and_then(|p| appcontainer::venv_home(&p));
    let targets = reset_targets(&dirs, base);
    #[cfg(windows)]
    {
        appcontainer::reset(&targets, &dirs)
    }
    #[cfg(not(windows))]
    {
        let _ = targets;
        Ok(0)
    }
}

/// 데몬이 시작할 때: 예전에 준 런타임 읽기 권한 중 지금 쓰지 않는 것을 거둔다. 지금 쓰는 것은 데몬이 아는 런타임과
/// 인스턴스마다 사용자가 정한 `java_path`(`(모듈 진입 파일, java_path)`)다. Windows에서만 한다
pub fn reconcile_runtimes(user_runtimes: &[(String, String)]) {
    if !cfg!(windows) {
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
    if !python_isolation_required(trust_of_entry(entry)) {
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
    // 모든 구성요소가 함께 읽는 폴더는 프로필마다 주지 않고 한 번만 준다 (ACE가 구성요소 수만큼 쌓이지 않게)
    // Python: venv와 그 기반 설치
    if let Some(venv) = python_exe.parent().and_then(Path::parent) {
        grants.push_shared(venv, Access::Read, &data_dir, home);
    }
    if let Some(base) = appcontainer::venv_home(python_exe) {
        // 유닉스의 `home`은 `<설치 루트>/bin`이다: 표준 라이브러리(`<루트>/lib/pythonX.Y`)도 보이게 루트를 준다
        let base = match base.parent() {
            Some(root) if cfg!(not(windows)) && base.file_name().is_some_and(|n| n == "bin") => root.to_path_buf(),
            _ => base,
        };
        grants.push_shared(&base, Access::Read, &data_dir, home);
    }
    let extensions = crate::plugin::resolve_extensions_dir();
    grants.push_shared(&raw_data_dir.join("python-shared"), Access::Read, &data_dir, home);
    grants.push_shared(&extensions, Access::Read, &data_dir, home);
    // PYTHONPATH의 익스텐션 상위 폴더: `import extensions.x`에 목록만 필요하다
    if let Some(parent) = extensions.parent() {
        grants.push_shared(parent, Access::List, &data_dir, home);
    }
    grants.push_shared(&data_dir.join("python-guard"), Access::Read, &data_dir, home);
    grants.push(&dir, Access::Read, &data_dir, home);
    grants.push(&pycache, Access::Write, &data_dir, home);
    grants.push(&temp, Access::Write, &data_dir, home);
    // 인스턴스 폴더 쓰기는 그 인스턴스의 모듈에게만. 익스텐션 hook은 확인된 모듈의 인스턴스에도 불리므로
    // 읽기만 주고, 쓸 곳은 익스텐션 전용 데이터 폴더다
    let access = instance_access(&key);
    let instance_protected = config
        .get("instance_dir")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .is_none_or(|dir| protect_daemon_entries(Path::new(dir)));
    // 리눅스는 같은 사용자라 상속을 끊어도 읽힌다: bwrap이 덮도록 적어 둔다
    if let Some(dir) = config.get("instance_dir").and_then(Value::as_str).filter(|d| !d.is_empty()) {
        grants.hidden = PROTECTED_INSTANCE_ENTRIES.iter().filter_map(|name| std::fs::canonicalize(Path::new(dir).join(name)).ok()).collect();
        let empty = raw_data_dir.join("sandbox-temp").join("empty");
        if !grants.hidden.is_empty() && (empty.is_file() || std::fs::write(&empty, "").is_ok()) {
            grants.empty_file = Some(empty);
        }
    }
    let data_dirs = config.get(INSTANCE_DATA_DIRS).and_then(Value::as_array).into_iter().flatten();
    // 데이터 폴더는 폴더일 때만 (데몬 파일 이름은 `insert_instance_fields`가 이미 뺐다)
    let data_dirs = data_dirs.filter_map(Value::as_str).filter(|p| !Path::new(p).is_file()).map(|p| (p, access));
    let fields = INSTANCE_DIR_FIELDS.iter().filter_map(|field| config.get(*field).and_then(Value::as_str).map(|p| (p, field_access(field, access, config))));
    for (path, access) in fields.chain(data_dirs).filter(|_| instance_protected) {
        if !path.is_empty() {
            let path = Path::new(path);
            // 새로 설치할 폴더는 아직 없다: 권한은 있는 폴더에만 줄 수 있으므로 먼저 만든다
            if access == Access::Write {
                let _ = std::fs::create_dir_all(path);
            }
            grants.push(path, access, &data_dir, home);
            // `os.makedirs(dir, exist_ok=True)`는 바로 위 폴더가 있는지부터 본다: 그 폴더의 목록만 준다
            if let Some(parent) = path.parent() {
                grants.push(parent, Access::List, &data_dir, home);
            }
        }
    }
    let data = extension_data_dir(&raw_data_dir, &key);
    if let Some(d) = &data {
        let _ = std::fs::create_dir_all(d);
        grants.push(d, Access::Write, &data_dir, home);
    }
    let extension_root = extension_data_root(&raw_data_dir, Some(&tag));
    let _ = std::fs::create_dir_all(&extension_root);
    grants.push(&extension_root, Access::Write, &data_dir, home);
    // 런타임은 없어도 되는 권한: 주지 못하면 그 폴더만 빼고 띄운다 (Java를 쓰지 않는 구성요소까지 막지 않게)
    for runtime in runtime_homes(None, std::env::var_os("JAVA_HOME").as_deref(), std::env::var_os("PATH").as_deref()) {
        let before = grants.entries.len();
        grants.push_optional(&runtime, Access::Read, &data_dir, home);
        if let Some(entry) = grants.entries.get(before).cloned() {
            grants.shared.push(entry);
        }
    }
    let profile = appcontainer::profile_for(&key);
    Some(Isolation { key, dir, profile, grants, pycache, temp, data, extension_root })
}

/// 격리 실행을 띄운 결과
pub enum Spawned {
    #[cfg(windows)]
    Contained(appcontainer::Contained),
    /// bwrap 안에서 띄울 명령 (리눅스). 입출력·프로세스 그룹은 부르는 쪽이 정하고 띄운다
    #[cfg(not(windows))]
    Wrapped(std::process::Command),
    /// 사용자가 이 구성요소를 격리 없이 돌리기로 했다
    Unisolated,
}

/// 격리해서 띄운다. 준비가 실패하면 사용자가 동의한 경우만 격리 없이 돌리고, 아니면 `sandbox_unavailable`
pub async fn spawn_isolated(cmd: &std::process::Command, iso: &Isolation) -> Result<Spawned, Blocked> {
    // 권한 주기(서버 폴더에 표시 붙이기)는 서버를 띄우는 중이 아닐 때만
    let gate = LABEL_GATE.lock().await;
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
        drop(gate);
        match started {
            Ok(child) => return Ok(Spawned::Contained(child)),
            Err(e) => e,
        }
    };
    #[cfg(not(windows))]
    let error = {
        drop(gate);
        match bwrap::program() {
            Some(program) => return Ok(Spawned::Wrapped(bwrap::wrap(program, cmd, &iso.grants))),
            None => "bubblewrap (bwrap) is not available".to_string(),
        }
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
pub(crate) fn consent_test_component(entry: &Path) {
    let (key, dir) = component_of(entry).unwrap_or_else(|| {
        let dir = entry.parent().unwrap().to_path_buf();
        (dir.to_string_lossy().into_owned(), dir)
    });
    consent_file().update(|cfg| {
        let section = cfg.entry(CONSENT_SECTION).or_insert_with(|| serde_json::json!({}));
        section["unisolated"][key.as_str()] = serde_json::json!(fingerprint(&dir));
    }).unwrap();
}

#[cfg(test)]
mod tests {

    /// 제거기는 인스턴스·작업·실행 파일 폴더와 그 바로 위, Python 기반 설치에서 권한을 거둔다
    #[test]
    fn the_uninstaller_revokes_grants_on_instance_folders_their_parents_and_the_python_base() {
        let dirs = [PathBuf::from("/data/instances/a"), PathBuf::from("/games/pal"), PathBuf::from("/games/pal")];
        let got = reset_targets(&dirs, Some(PathBuf::from("/py")));
        let want: Vec<PathBuf> = ["/data/instances", "/data/instances/a", "/games", "/games/pal", "/py"].iter().map(PathBuf::from).collect();
        assert_eq!(got, want);
        assert!(appcontainer::is_saba_profile("saba-chan.untrusted.0123456789abcdef"));
        assert!(appcontainer::is_saba_profile("saba-chan.untrusted"), "the old shared profile");
        assert!(!appcontainer::is_saba_profile("saba-chan.untrustedness") && !appcontainer::is_saba_profile("Microsoft.Photos"));
    }
    use super::*;

    /// 가져온 서버를 지우면 그 폴더와 바로 위에서 거두되, 다른 인스턴스가 같은 위 폴더를 쓰면 위 폴더는 둔다
    #[test]
    fn unlinking_releases_the_folder_and_its_parent_unless_another_server_still_sits_there() {
        let dirs = [PathBuf::from("/games/pal"), PathBuf::from("/games/pal/bin")];
        let got = release_targets(&dirs, &[PathBuf::from("/data/instances/b")]);
        let want: Vec<PathBuf> = ["/games", "/games/pal", "/games/pal/bin"].iter().map(PathBuf::from).collect();
        assert_eq!(got, want);
        let got = release_targets(&dirs[..1], &[PathBuf::from("/games/mc")]);
        assert_eq!(got, vec![PathBuf::from("/games/pal")]);
    }

    /// 인스턴스 폴더 자체에 쓰기는 가져오기의 `import_settings`만: 다른 호출은 읽기만 한다
    /// (인스턴스 폴더 바로 아래 데몬 항목을 격리 실행이 바꾸지 못하게). 모듈 설정 값으로는 이 표시를 싣지 못한다
    #[test]
    fn only_the_migration_import_writes_the_instance_folder_itself() {
        use appcontainer::Access;
        let start = serde_json::json!({ "instance_dir": "/i", "working_dir": "/i/server" });
        assert_eq!(field_access("instance_dir", Access::Write, &start), Access::Read);
        assert_eq!(field_access("working_dir", Access::Write, &start), Access::Write);
        let import = serde_json::json!({ "instance_dir": "/i", MIGRATION_IMPORT: true });
        assert_eq!(field_access("instance_dir", Access::Write, &import), Access::Write);
        assert_eq!(field_access("instance_dir", Access::Read, &import), Access::Read, "extensions only read");
        assert!(grants_write(MIGRATION_IMPORT));
    }

    /// 유닉스 venv의 `home`은 `<설치 루트>/bin`이다: 표준 라이브러리(`<루트>/lib/pythonX.Y`)까지 보이게 루트를 준다
    #[cfg(not(windows))]
    #[test]
    fn a_unix_base_python_is_granted_from_its_install_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (base, venv) = (root.join("py"), root.join("venv"));
        for d in [base.join("bin"), base.join("lib/python3.13"), venv.join("bin")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(venv.join("pyvenv.cfg"), format!("home = {}\n", base.join("bin").display())).unwrap();
        let entry = root.join("lifecycle.py");
        std::fs::write(&entry, "").unwrap();
        let iso = isolation_for(&entry, &serde_json::json!({}), &venv.join("bin").join("python")).unwrap();
        assert!(iso.grants.entries.contains(&(base, appcontainer::Access::Read)), "{:?}", iso.grants.entries);
    }

    #[test]
    fn python_isolation_policy_is_independent_of_launch_guard_platform() {
        assert!(!python_isolation_required(Trust::Verified));
        assert!(python_isolation_required(Trust::Unverified));
        let tmp = tempfile::tempdir().unwrap();
        let entry = tmp.path().join("lifecycle.py");
        std::fs::write(&entry, "").unwrap();
        assert!(isolation_for(&entry, &serde_json::json!({}), Path::new("python")).is_some());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn an_unsupported_isolation_run_is_refused_with_the_consent_action() {
        let tmp = tempfile::tempdir().unwrap();
        let iso = Isolation {
            key: "module-no-platform-consent".into(), dir: tmp.path().to_path_buf(),
            profile: String::new(), grants: appcontainer::Grants::default(),
            pycache: PathBuf::new(), temp: PathBuf::new(), data: None, extension_root: PathBuf::new(),
        };
        let blocked = spawn_isolated(&std::process::Command::new("python"), &iso).await.err().unwrap();
        let response = blocked.to_json(message(&blocked));
        assert_eq!(response["action_required"], "sandbox_unavailable");
        assert_eq!(response["error_code"], "isolation_unsupported");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn an_isolated_run_on_linux_is_wrapped_in_bubblewrap() {
        bwrap::use_in_this_test();
        let Some(program) = bwrap::program() else { return };
        assert!(restricted(Trust::Unverified) && !restricted(Trust::Verified));
        let tmp = tempfile::tempdir().unwrap();
        let iso = Isolation {
            key: "module-bwrap-test".into(), dir: tmp.path().to_path_buf(),
            profile: String::new(), grants: appcontainer::Grants::default(),
            pycache: PathBuf::new(), temp: PathBuf::new(), data: None, extension_root: PathBuf::new(),
        };
        let mut cmd = std::process::Command::new("python");
        cmd.arg("lifecycle.py").env("SABA_X", "1");
        let Ok(Spawned::Wrapped(wrapped)) = spawn_isolated(&cmd, &iso).await else { panic!("expected a bwrap command") };
        assert_eq!(wrapped.get_program(), program.as_os_str());
        let args: Vec<_> = wrapped.get_args().collect();
        let sep = args.iter().position(|a| *a == "--").unwrap();
        assert_eq!(&args[sep + 1..], ["python", "lifecycle.py"]);
        assert!(wrapped.get_envs().any(|(k, v)| k == "SABA_X" && v == Some("1".as_ref())));
    }

    // 백로그 M7 (사용자 결정: 동기화 폴더 아래 서버는 계속 막는다). 막되 원인과 할 일을 알린다
    #[test]
    fn a_server_folder_in_a_sync_folder_is_blocked_with_its_own_reason() {
        let synced = PathBuf::from(r"C:\Users\me\OneDrive\server\PalServer.exe");
        let blocked = folder_label_block(&[
            (PathBuf::from(r"C:\srv\a.txt"), "access denied".to_string()),
            (synced.clone(), format!("{} {}", synced.display(), SYNC_FOLDER_MARK)),
        ]);
        assert_eq!(blocked.code, "server_in_sync_folder");
        assert_eq!(blocked.file.as_deref(), Some(synced.to_str().unwrap()));
        assert!(message(&blocked).contains("sync folder"), "{}", message(&blocked));

        let plain = folder_label_block(&[(PathBuf::from(r"C:\srv\a.txt"), "access denied".to_string())]);
        assert_eq!(plain.code, "server_folder_label");
    }

    // 서버 폴더의 표시를 걷은 뒤 서버가 뜨기 전에 격리 실행이 끼어 표시를 다시 붙이면 서버가 낮은 무결성으로 뜬다
    #[tokio::test]
    async fn a_sandboxed_call_waits_until_the_server_has_launched() {
        let roots = LaunchRoots { dirs: Vec::new(), runtimes: Vec::new() };
        let (_, gate) = guard_launch_for(|| Trust::Verified, "java", &[], Path::new("."), roots, &mut Vec::new()).await.unwrap();
        let iso = Isolation {
            key: "module-gate-test".into(),
            dir: PathBuf::from("."),
            profile: String::new(),
            grants: appcontainer::Grants::default(),
            pycache: PathBuf::new(),
            temp: PathBuf::new(),
            data: None,
            extension_root: PathBuf::new(),
        };
        let call = tokio::spawn(async move { spawn_isolated(&std::process::Command::new("python"), &iso).await.is_ok() });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!call.is_finished(), "the sandboxed call must wait while a server launch is in progress");
        drop(gate);
        tokio::time::timeout(std::time::Duration::from_secs(5), call).await.expect("runs once the server has launched").unwrap();
    }

    #[test]
    fn an_entry_in_a_subfolder_belongs_to_its_module_or_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let (modules, extensions) = (tmp.path().join("modules"), tmp.path().join("extensions"));
        for dir in [modules.join("game/scripts"), extensions.join("tool/src/py"), tmp.path().join("other")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let of = |entry: PathBuf| component_in(&entry, &modules, &extensions);
        assert_eq!(of(modules.join("game/lifecycle.py")), Some(("module-game".into(), modules.join("game"))));
        assert_eq!(of(modules.join("game/scripts/lifecycle.py")), Some(("module-game".into(), modules.join("game"))));
        assert_eq!(of(extensions.join("tool/src/py/m.py")), Some(("ext-tool".into(), extensions.join("tool"))));
        assert_eq!(of(tmp.path().join("other/lifecycle.py")), None);
        assert_eq!(of(modules.join("lifecycle.py")), None);
        // `..`가 든 경로는 글자로 접어 다른 구성요소로 판정하지 않는다
        assert_eq!(of(extensions.join("tool/../../modules/game/lifecycle.py")), None);
        assert_eq!(of(modules.join("game/../game/lifecycle.py")), None);
    }

    // 익스텐션 id에 '-'가 있으면 폴더 이름은 '_'로 바뀐다 (`ue4-ini` → `ue4_ini`). 신뢰 기록의 키는 id로 적으므로
    // 폴더 이름이 아니라 manifest의 id로 찾아야 기록이 있는 공식 익스텐션이 격리되지 않는다
    #[test]
    fn an_extension_is_keyed_by_its_manifest_id_not_its_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (modules, extensions) = (tmp.path().join("modules"), tmp.path().join("extensions"));
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::create_dir_all(extensions.join("ue4_ini")).unwrap();
        std::fs::write(extensions.join("ue4_ini/manifest.json"), br#"{"id":"ue4-ini"}"#).unwrap();
        std::fs::create_dir_all(extensions.join("other_ext")).unwrap();
        std::fs::write(extensions.join("other_ext/manifest.json"), br#"{"id":"steamcmd"}"#).unwrap();
        let of = |entry: PathBuf| component_in(&entry, &modules, &extensions).map(|(key, _)| key);
        assert_eq!(of(extensions.join("ue4_ini/ue4_ini.py")).as_deref(), Some("ext-ue4-ini"));
        assert_eq!(of(extensions.join("other_ext/x.py")).as_deref(), Some("ext-other_ext"), "an id that is not this folder's is ignored");
    }

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

    /// 클래스 경로 목록(`a.jar;b.jar`) 안의 공유 폴더 경로도 막는다 (Java가 그 경로를 열며 자격 증명을 보낸다)
    #[cfg(windows)]
    #[test]
    fn a_share_inside_a_class_path_list_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let list = r"lib\a.jar;\\host\share\x.jar".to_string();
        assert_eq!(argument_files(&["-cp".to_string(), list.clone()], tmp.path()), Err(list));
        let prop = r"-Djava.class.path=lib\*;\\host\share\x.jar".to_string();
        assert_eq!(argument_files(&[prop.clone()], tmp.path()), Err(prop));
        assert!(argument_files(&["-cp".to_string(), r"lib\*;.".to_string()], tmp.path()).is_ok());
    }

    /// Java가 인자처럼 읽는 곳(`@파일`·`-XX:VMOptionsFile`의 내용, JVM 옵션·클래스 경로 환경 변수)도 인자와 같이 본다
    #[test]
    fn arguments_java_reads_from_files_and_variables_are_checked_too() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("args.txt"), "# 주석\n-javaagent:agent.jar \"-Dname=a b\" # 뒤 주석\n-Xmx4G\n").unwrap();
        std::fs::write(tmp.path().join("vm.txt"), "-agentlib:vm").unwrap();
        std::fs::write(tmp.path().join("env.txt"), "-Denv=1").unwrap();
        let env: Vec<(String, String)> = [
            ("JAVA_TOOL_OPTIONS", "-agentpath:x.dll"),
            ("JDK_JAVA_OPTIONS", "@env.txt"),
            ("CLASSPATH", "a.jar;b.jar"),
            ("SERVER_NAME", "-javaagent:ignored.jar"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let args = ["@args.txt", "@missing.txt", "-XX:VMOptionsFile=vm.txt", "-jar"].map(String::from);
        let found = indirect_arguments(&args, &env, tmp.path()).unwrap();
        assert_eq!(
            found,
            ["-agentpath:x.dll", "@env.txt", "a.jar;b.jar", "-javaagent:agent.jar", "-Dname=a b", "-Xmx4G", "-agentlib:vm", "-Denv=1"]
        );
        // 너무 큰 인자 파일은 건너뛰지 않고 막는다
        std::fs::write(tmp.path().join("big.txt"), vec![b' '; (ARGUMENT_FILE_LIMIT + 1) as usize]).unwrap();
        assert_eq!(indirect_arguments(&["@big.txt".to_string()], &[], tmp.path()), Err("@big.txt".to_string()));
    }

    /// 따옴표는 낱말 중간에서도 벗긴다. 따옴표 안의 `\`는 이스케이프로 보는 규칙과 글자 그대로 보는 규칙 둘 다로 나눈다
    #[test]
    fn java_splits_quotes_anywhere_in_a_word() {
        assert_eq!(split_java(r#"-javaagent:"a b.jar" -D'x'=y"#, true), ["-javaagent:a b.jar", "-Dx=y"]);
        assert_eq!(split_java("\"lo\\\n    ng\"", true), ["long"]);
        let both = java_tokens(r#"-javaagent:"\\h\s\x.jar""#);
        assert!(both.contains(&r"-javaagent:\hsx.jar".to_string()), "{both:?}");
        assert!(both.contains(&r"-javaagent:\\h\s\x.jar".to_string()), "{both:?}");
    }

    /// 공유 폴더 경로는 `@파일` 안에 있어도, JVM 옵션 환경 변수에 있어도, 값만 따옴표로 감싸도 막는다
    #[cfg(windows)]
    #[tokio::test]
    async fn a_share_inside_an_argument_file_or_a_jvm_variable_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = LaunchRoots::default();
        for (file, line) in [("args.txt", r"-javaagent:\\host\share\x.jar"), ("quoted.txt", r#"-javaagent:"\\host\share\x.jar""#)] {
            std::fs::write(tmp.path().join(file), line).unwrap();
            let arg = format!("@{file}");
            let refused = guard_launch_for(|| Trust::Unverified, "java", &[arg], tmp.path(), roots.clone(), &mut Vec::new()).await;
            assert!(refused.is_err(), "{line}");
        }
        let share = r"-javaagent:'\\host\share\x.jar'";
        let mut env = vec![("JAVA_TOOL_OPTIONS".to_string(), share.to_string())];
        assert!(guard_launch_for(|| Trust::Unverified, "java", &[], tmp.path(), roots.clone(), &mut env).await.is_err());
        let mut env = vec![("JDK_JAVA_OPTIONS".to_string(), r"@\\host\share\args.txt".to_string())];
        assert!(guard_launch_for(|| Trust::Unverified, "java", &[], tmp.path(), roots.clone(), &mut env).await.is_err());
        let mut env = vec![("JDK_JAVA_OPTIONS".to_string(), "-Xmx4G".to_string())];
        assert!(guard_launch_for(|| Trust::Unverified, "java", &[], tmp.path(), roots, &mut env).await.is_ok());
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
    fn daemon_secrets_in_an_instance_folder_are_restricted_before_a_grant() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["instance.json", "rcon.json"] {
            std::fs::write(tmp.path().join(name), b"{}").unwrap();
        }
        std::fs::create_dir_all(tmp.path().join("backups")).unwrap();
        assert!(protect_daemon_entries(tmp.path()));
        // 주인은 계속 읽고 쓴다
        assert_eq!(std::fs::read(tmp.path().join("rcon.json")).unwrap(), b"{}");
        std::fs::write(tmp.path().join("backups").join("a"), b"x").unwrap();
    }

    /// 가져오기 전 원래 설정 보관본은 격리된 모듈이 고치지 못해야 한다 (해제할 때 그대로 되돌린다)
    #[test]
    fn the_kept_originals_of_an_imported_server_are_protected_too() {
        assert!(PROTECTED_INSTANCE_ENTRIES.contains(&crate::instance::migration::DIR));
    }

    /// 익스텐션 폴더는 id의 '-'를 '_'로 바꾼 이름이다. 없는 폴더를 보면 기록에 없는 코드 검사가 빠진다
    #[test]
    fn an_extension_folder_is_found_by_its_python_safe_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("ue4_ini")).unwrap();
        std::fs::create_dir_all(tmp.path().join("old-style")).unwrap();
        assert_eq!(extension_dir_in(tmp.path(), "ue4-ini"), tmp.path().join("ue4_ini"));
        assert_eq!(extension_dir_in(tmp.path(), "old-style"), tmp.path().join("old-style"));
        assert_eq!(extension_dir_in(tmp.path(), "steamcmd"), tmp.path().join("steamcmd"));
    }

    /// 모듈이 import한 익스텐션(SteamCMD 등)이 받은 파일을 둘 곳. 격리된 실행은 구성요소마다 따로 써서
    /// 확인된 실행이 쓰는 파일을 바꾸지 못하게 한다 (실제 PC: 격리된 팰월드가 공용 SteamCMD 폴더를 만들지 못해 설치가 막혔다)
    #[test]
    fn isolated_runs_get_their_own_extension_data_root() {
        let data = Path::new("data");
        assert_eq!(extension_data_root(data, None), data.join("extension-data"));
        let tag = appcontainer::component_tag("module-palworld");
        assert_eq!(extension_data_root(data, Some(&tag)), data.join("sandbox-data").join(&tag).join("extension-data"));
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
