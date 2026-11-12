//! Python 가상환경 매니저 — 엔드유저 무설치 Python 환경 제공
//!
//! saba-chan은 **엔드유저가 Python을 별도로 설치할 필요 없이** 동작합니다.
//! 필요 시 포터블 Python(`python-build-standalone`)을 자동으로 다운로드하고
//! 격리된 가상환경(venv)을 생성합니다.
//!
//! ## 부트스트랩 흐름
//! 1. `get_python_path()` 호출 → `OnceCell` 캐싱 (최초 1회만 실제 작업)
//! 2. 기존 venv 유효? → 바로 반환
//! 3. Python 인터프리터 탐색:
//!    a. 이미 다운로드된 포터블 Python → 사용
//!    b. 시스템 Python ≥ 3.11 → 사용 (모듈 공용 도우미가 표준 `tomllib`으로 module.toml을 읽는다)
//!    c. 둘 다 없음 → 포터블 Python 자동 다운로드 (~20 MB)
//! 4. 찾은 Python으로 venv 생성 → pip 업그레이드 → 검증 → 캐시
//!
//! ## 데이터 디렉토리 레이아웃
//! ```text
//! <data_dir>/
//!   python-standalone/    ← 포터블 Python (자동 다운로드)
//!     python/
//!       python.exe        (Windows)
//!       bin/python3       (Linux/macOS)
//!   python-env/           ← 격리 venv
//!     Scripts/python.exe  (Windows)
//!     bin/python           (Linux/macOS)
//! ```
//!
//! 자리·포터블 받기·검사·venv 만들기는 설치기와 같은 `saba_chan_updater_lib::python_runtime`을 쓴다.
//! 이 모듈은 고르는 순서(포터블 → 시스템 → 받기)와 캐시·사전 점검만 맡는다.

use anyhow::{Context, Result};
use saba_chan_updater_lib::python_runtime::{self as runtime, MIN_VERSION as MIN_PYTHON_VERSION};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::utils::apply_creation_flags;

use saba_chan_updater_lib::python_pin::PYTHON_VERSION;

/// 준비 명령은 준비가 취소되면 자식까지 끝낸다 (`plugin::preparation_output`)
struct Preparing;

impl runtime::Run for Preparing {
    fn output<'a>(&'a self, cmd: &'a mut Command) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<std::process::Output>> + Send + 'a>> {
        Box::pin(crate::plugin::preparation_output(cmd))
    }
}

// ── Cached venv python path ─────────────────────────────────────

static VENV_PYTHON: PythonCache = PythonCache::new();

/// 준비한 venv Python 경로. 실행 파일이 사라졌거나 실행되지 않으면 버리고 다음 호출이 다시 준비한다
/// (시스템 Python을 지우거나 올려 venv가 깨져도 데몬을 다시 켜지 않게)
struct PythonCache(std::sync::Mutex<Option<std::sync::Arc<OnceCell<PathBuf>>>>);

impl PythonCache {
    const fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    fn cell(&self) -> std::sync::Arc<OnceCell<PathBuf>> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().and_then(|cell| cell.get()).is_some_and(|path| !path.exists()) {
            *slot = None;
        }
        slot.get_or_insert_with(Default::default).clone()
    }

    async fn get_or_prepare<F: std::future::Future<Output = Result<PathBuf>>>(&self, prepare: impl FnOnce() -> F) -> Result<PathBuf> {
        self.cell().get_or_try_init(prepare).await.cloned()
    }

    fn forget(&self, path: &Path) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().and_then(|cell| cell.get()).is_some_and(|cached| cached == path) {
            *slot = None;
        }
    }
}

/// 관리 venv의 Python 인터프리터 경로를 반환합니다.
/// 최초 호출 시 전체 부트스트랩(다운로드 포함)을 자동 수행합니다. 실패는 캐시하지 않는다(다음 호출이 다시 시도)
pub async fn get_python_path() -> Result<PathBuf> {
    VENV_PYTHON.get_or_prepare(|| async {
        let mut preparing = Preparation::new(&PREFLIGHT);
        let result = ensure_venv().await;
        preparing.finish(&result);
        result
    }).await
}

/// 이 경로의 Python을 띄우지 못했다: 다음 호출이 venv를 다시 확인한다
pub fn forget_python(path: &Path) {
    VENV_PYTHON.forget(path);
}

/// 이 경로의 Python이 비정상 종료했다: 인터프리터 자체가 돌지 않으면 다음 호출이 venv를 다시 만든다
/// (Windows venv 실행기는 원본 Python이 없어져도 파일은 남아 있고, 띄우면 오류 코드로 끝난다)
pub async fn recheck_python(path: PathBuf) {
    if !verify_python(&path).await {
        VENV_PYTHON.forget(&path);
    }
}

// ── 부팅 사전 점검 (GET /api/runtime/preflight) ──────────────────

/// Python 준비 상태. 데몬이 부팅하자마자 준비를 시작하고, GUI는 끝날 때까지 이것을 본다
#[derive(Debug, Clone, PartialEq)]
pub enum Preflight {
    Idle,
    Running,
    Ready,
    Failed(String),
}

static PREFLIGHT: std::sync::Mutex<Preflight> = std::sync::Mutex::new(Preflight::Idle);

struct Preparation<'a> {
    state: &'a std::sync::Mutex<Preflight>,
    finished: bool,
}

impl<'a> Preparation<'a> {
    fn new(state: &'a std::sync::Mutex<Preflight>) -> Self {
        *state.lock().unwrap_or_else(|e| e.into_inner()) = Preflight::Running;
        Self { state, finished: false }
    }

    fn finish<T>(&mut self, result: &Result<T>) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = match result {
            Ok(_) => Preflight::Ready,
            Err(error) => Preflight::Failed(format!("{error:#}")),
        };
        self.finished = true;
    }
}

impl Drop for Preparation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Preflight::Idle;
        }
    }
}

/// `{phase, percent, complete, error, components: {python}}`. complete는 성공·실패 모두 끝난 상태
pub fn preflight_snapshot() -> serde_json::Value {
    let state = PREFLIGHT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (phase, percent, python) = match &state {
        Preflight::Idle => ("preflight-idle", 0, "pending"),
        Preflight::Running => ("preflight-python", 55, "running"),
        Preflight::Ready => ("preflight-done", 100, "ok"),
        Preflight::Failed(_) => ("preflight-failed", 0, "failed"),
    };
    let error = match state {
        Preflight::Failed(e) => Some(e),
        _ => None,
    };
    serde_json::json!({
        "phase": phase,
        "percent": percent,
        "complete": matches!(python, "ok" | "failed"),
        "error": error,
        "components": { "python": python },
    })
}

// ═══════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════

/// venv가 존재하고 유효한지 확인합니다. 없으면 전체 부트스트랩을 수행합니다.
/// 반환값: venv 내부 Python 실행 파일의 절대 경로
pub async fn ensure_venv() -> Result<PathBuf> {
    let data_dir = resolve_data_dir()?;
    let python_exe = runtime::venv_python(&data_dir);

    // ── Fast path: 기존 venv 유효 ──
    if verify_python(&python_exe).await {
        tracing::debug!("Python venv 확인 완료: {}", python_exe.display());
        return Ok(python_exe);
    }

    // ── Python 인터프리터 찾기 ── (venv 만들기가 손상된 venv를 지운다)
    let base_python = find_base_python(&data_dir).await?;

    let portable = async { download_portable_python(&data_dir).await.map(|exe| exe.to_string_lossy().into_owned()) };
    let portable = (Path::new(&base_python) != runtime::portable_python(&data_dir)).then_some(portable);
    create_venv_or_fall_back(&base_python, &data_dir, portable).await?;

    tracing::info!("Python venv 준비 완료: {}", python_exe.display());
    Ok(python_exe)
}

/// `base`로 `<root>/python-env`를 만든다. 만들지 못하면 `portable`(포터블 Python 받기)로 한 번 더 만든다:
/// Debian·Ubuntu는 python3가 있어도 python3-venv가 없으면 `python3 -m venv`가 실패한다
async fn create_venv_or_fall_back(base: &str, root: &Path, portable: Option<impl std::future::Future<Output = Result<String>>>) -> Result<()> {
    let Err(error) = runtime::create_venv(&Preparing, Path::new(base), root).await else { return Ok(()) };
    let Some(portable) = portable else { return Err(error) };
    tracing::warn!("{} 로 venv를 만들지 못해 포터블 Python으로 다시 만듭니다: {:#}", base, error);
    let base = portable.await.with_context(|| format!("{error:#}"))?;
    runtime::create_venv(&Preparing, Path::new(&base), root).await.map(|_| ())
}

/// 시스템에서 Python ≥ 3.11 을 탐지합니다.
pub async fn detect_system_python() -> Result<String> {
    let candidates = ["python", "python3", "py"];
    for cmd_name in candidates {
        let mut cmd = Command::new(cmd_name);
        cmd.args(["-I", "-S", "-c", "import json, sys; print(json.dumps(['Python %d.%d' % sys.version_info[:2], sys.executable]))"]);
        apply_creation_flags(&mut cmd);

        if let Ok(output) = crate::plugin::preparation_output(&mut cmd).await {
            if output.status.success() {
                let Ok((ver, executable)) = serde_json::from_slice::<(String, String)>(&output.stdout) else { continue; };
                if let Some((major, minor)) = runtime::parse_version(&ver) {
                    if (major, minor) >= MIN_PYTHON_VERSION {
                        if !Path::new(&executable).is_file() {
                            continue;
                        }
                        tracing::info!(
                            "시스템 Python 발견: {} → {}",
                            executable,
                            ver.trim()
                        );
                        return Ok(executable);
                    }
                    tracing::debug!(
                        "{} → {}.{} (최소 {}.{} 필요, 건너뜀)",
                        cmd_name, major, minor,
                        MIN_PYTHON_VERSION.0, MIN_PYTHON_VERSION.1
                    );
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "시스템에 Python >= {}.{} 없음",
        MIN_PYTHON_VERSION.0,
        MIN_PYTHON_VERSION.1
    ))
}

// ═══════════════════════════════════════════════════════════════
//  Internal: Python 탐색 & 포터블 다운로드
// ═══════════════════════════════════════════════════════════════

/// Python 인터프리터를 찾습니다: 포터블 → 시스템 → 자동 다운로드
async fn find_base_python(data_dir: &Path) -> Result<String> {
    // 1) 이미 다운로드된 포터블 Python
    let portable_exe = runtime::portable_python(data_dir);
    if verify_python(&portable_exe).await {
        tracing::info!("포터블 Python 사용: {}", portable_exe.display());
        return Ok(portable_exe.to_string_lossy().into_owned());
    }

    // 2) 시스템 Python
    if let Ok(cmd) = detect_system_python().await {
        tracing::info!("시스템 Python 사용: {}", cmd);
        return Ok(cmd);
    }

    // 3) 자동 다운로드
    tracing::info!(
        "Python을 찾을 수 없습니다. 포터블 Python {}을(를) 다운로드합니다...",
        PYTHON_VERSION
    );
    let exe = download_portable_python(data_dir).await?;
    Ok(exe.to_string_lossy().into_owned())
}

/// 포터블 Python 받기 → 추출 → 검증 (`python_runtime::install_portable`). 고정 해시와 다르면 풀지 않는다 (C6)
async fn download_portable_python(data_dir: &Path) -> Result<PathBuf> {
    runtime::install_portable(&Preparing, data_dir, &|_, _| {}).await.with_context(|| {
        format!(
            "포터블 Python을 준비하지 못했습니다. 인터넷 연결을 확인하거나, Python {}.{} 이상을 수동 설치해 주세요.",
            MIN_PYTHON_VERSION.0, MIN_PYTHON_VERSION.1
        )
    })
}

// ═══════════════════════════════════════════════════════════════
//  Internal: 경로 & 유틸리티
// ═══════════════════════════════════════════════════════════════

/// 런타임 폴더 (`python_runtime::runtime_dir`: `SABA_DATA_DIR` → 데몬 옆 → 플랫폼 앱 데이터)
fn resolve_data_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().ok();
    runtime::runtime_dir(exe.as_deref().and_then(Path::parent))
}

/// 데몬 venv의 Python 실행 파일 (있든 없든 그 자리)
pub fn venv_python() -> Option<PathBuf> {
    resolve_data_dir().ok().map(|d| runtime::venv_python(&d))
}

/// Python 실행 파일이 정상 동작하고 최소 버전 이상인지 확인
async fn verify_python(exe: &Path) -> bool {
    runtime::verify(&Preparing, exe).await
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_preparation_resets_state_and_allows_a_retry() {
        let state = std::sync::Mutex::new(Preflight::Idle);
        let cell = OnceCell::<PathBuf>::new();
        let result = tokio::time::timeout(std::time::Duration::from_millis(10), cell.get_or_try_init(|| async {
            let _preparing = Preparation::new(&state);
            std::future::pending::<Result<PathBuf>>().await
        })).await;
        assert!(result.is_err());
        assert_eq!(*state.lock().unwrap(), Preflight::Idle);
        assert!(cell.get().is_none());
        let path = cell.get_or_try_init(|| async {
            let mut preparing = Preparation::new(&state);
            let result = Ok(PathBuf::from("python"));
            preparing.finish(&result);
            result
        }).await.unwrap();
        assert_eq!(path, Path::new("python"));
        assert_eq!(*state.lock().unwrap(), Preflight::Ready);
    }

    #[tokio::test]
    async fn a_prepared_python_that_disappeared_or_was_forgotten_is_prepared_again() {
        let tmp = tempfile::tempdir().unwrap();
        let python = tmp.path().join("python");
        std::fs::write(&python, b"").unwrap();
        let cache = PythonCache::new();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let prepare = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::fs::write(&python, b"").unwrap();
            Ok(python.clone())
        };
        cache.get_or_prepare(prepare).await.unwrap();
        cache.get_or_prepare(prepare).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        std::fs::remove_file(&python).unwrap();
        assert_eq!(cache.get_or_prepare(prepare).await.unwrap(), python);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        cache.forget(Path::new("another-python"));
        cache.get_or_prepare(prepare).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        cache.forget(&python);
        cache.get_or_prepare(prepare).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// python3-venv가 없는 Debian·Ubuntu: 시스템 Python으로 venv를 못 만들면 포터블로 만든다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_base_python_that_cannot_make_a_venv_falls_back_to_the_portable_one() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = |name: &str, body: &str| {
            let path = tmp.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_string_lossy().into_owned()
        };
        let no_venv = script("no-venv", "mkdir -p \"$3/bin\"; echo 'ensurepip is not available' >&2; exit 1");
        let works = script("works", "[ \"$1\" = -m ] || exit 0\nmkdir -p \"$3/bin\"\nprintf '#!/bin/sh\\necho 3.12.0\\n' > \"$3/bin/python\"\nchmod +x \"$3/bin/python\"");
        let root = tmp.path().join("runtime");

        let error = create_venv_or_fall_back(&no_venv, &root, None::<std::future::Ready<Result<String>>>).await.unwrap_err();
        assert!(error.to_string().contains("ensurepip"), "{error:#}");

        create_venv_or_fall_back(&no_venv, &root, Some(async { Ok(works) })).await.unwrap();
        assert!(verify_python(&runtime::venv_python(&root)).await);
    }

    #[test]
    fn test_python_download_url_format() {
        let (url, sha) = saba_chan_updater_lib::python_pin::download().unwrap();
        assert!(sha.is_some_and(|s| s.len() == 64) || std::env::var("SABA_PYTHON_URL").is_ok());
        assert!(url.contains("python-build-standalone"));
        assert!(url.contains(PYTHON_VERSION));
        assert!(url.ends_with(".tar.gz"));
    }

    #[test]
    fn test_resolve_data_dir_no_panic() {
        let _ = resolve_data_dir();
    }

    #[tokio::test]
    async fn test_detect_system_python() {
        // CI/개발 환경에 따라 성공 또는 실패 가능
        match detect_system_python().await {
            Ok(cmd) => println!("시스템 Python: {}", cmd),
            Err(e) => println!("시스템 Python 없음 (정상): {}", e),
        }
    }
}
