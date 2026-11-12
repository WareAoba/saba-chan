//! 포터블 Python 런타임과 venv 준비 — 데몬(`python_env`)과 설치기(`runtime_bootstrap`)가 같은 자리·같은 절차를 쓴다
//!
//! ```text
//! <런타임 폴더>/
//!   python-standalone/python/   ← 포터블 Python (`python_pin`의 판, python.exe 또는 bin/python3)
//!   python-env/                 ← 격리 venv (Scripts/python.exe 또는 bin/python)
//! ```
//! 무엇으로 venv를 만들지(시스템 Python을 쓸지)는 부르는 쪽이 정한다

use anyhow::{Context, Result};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::process::Command;

pub const PORTABLE_DIR: &str = "python-standalone";
pub const VENV_DIR: &str = "python-env";
/// venv로 쓰는 최소 판. 모듈 공용 도우미가 표준 `tomllib`(3.11+)으로 module.toml을 읽는다
pub const MIN_VERSION: (u32, u32) = (3, 11);

/// 명령을 띄워 출력을 받는다. 데몬은 준비가 취소되면 자식까지 끝내는 실행(`plugin::preparation_output`)을 넘긴다
pub trait Run: Sync {
    fn output<'a>(&'a self, cmd: &'a mut Command) -> Pin<Box<dyn Future<Output = std::io::Result<std::process::Output>> + Send + 'a>>;
}

/// 그대로 띄운다 (설치기)
pub struct Plain;

impl Run for Plain {
    fn output<'a>(&'a self, cmd: &'a mut Command) -> Pin<Box<dyn Future<Output = std::io::Result<std::process::Output>> + Send + 'a>> {
        Box::pin(cmd.output())
    }
}

/// 런타임 폴더: `SABA_DATA_DIR` → (맥 앱 묶음이 아니고 쓸 수 있으면) 실행 파일 폴더 → 플랫폼 앱 데이터 폴더.
/// 데몬은 자기 폴더를, 설치기는 설치 폴더를 넘긴다 (데몬이 설치 폴더에 있으므로 같은 곳이 된다)
pub fn runtime_dir(exe_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("SABA_DATA_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(exe_dir) = exe_dir {
        // 맥 앱 묶음(`*.app/Contents/MacOS`) 안은 쓰지 않는다: 묶음에 파일을 더하면 서명이 깨진다.
        // 데이터 폴더에 두면 제거기가 런타임 항목으로 지운다
        if crate::app_bundle::bundle_of(exe_dir).is_some() {
            return Ok(crate::constants::resolve_data_dir());
        }
        if is_dir_writable(exe_dir) {
            return Ok(exe_dir.to_path_buf());
        }
    }
    platform_data_dir()
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA").context("APPDATA 환경변수 없음")?;
    Ok(PathBuf::from(appdata).join(crate::constants::APP_NAME))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME 환경변수 없음")?;
    Ok(PathBuf::from(home).join("Library/Application Support").join(crate::constants::APP_NAME))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME 환경변수 없음")?;
    Ok(PathBuf::from(home).join(".local/share").join(crate::constants::APP_NAME))
}

fn is_dir_writable(dir: &Path) -> bool {
    let probe = dir.join(".saba-write-test");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 포터블 Python 실행 파일 (`<root>/python-standalone/python/python.exe` 또는 `.../python/bin/python3`)
pub fn portable_python(root: &Path) -> PathBuf {
    let base = root.join(PORTABLE_DIR).join("python");
    if cfg!(windows) { base.join("python.exe") } else { base.join("bin").join("python3") }
}

/// venv의 Python 실행 파일 (있든 없든 그 자리)
pub fn venv_python(root: &Path) -> PathBuf {
    let venv = root.join(VENV_DIR);
    if cfg!(windows) { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") }
}

/// "Python 3.12.8"·"3.12.8" → (3, 12)
pub fn parse_version(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let ver = s.strip_prefix("Python ").or_else(|| s.strip_prefix("python ")).unwrap_or(s);
    let mut parts = ver.split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next()?.trim().parse().ok()?;
    Some((major, minor))
}

/// 콘솔 창 없이 띄운다 (Windows)
pub fn hide_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 실행되고 최소 판 이상인가
pub async fn verify(run: &dyn Run, exe: &Path) -> bool {
    if !exe.exists() {
        return false;
    }
    let mut cmd = Command::new(exe);
    cmd.args(["-c", "import sys; v=sys.version_info; print(f'{v.major}.{v.minor}.{v.micro}')"]);
    hide_window(&mut cmd);
    matches!(run.output(&mut cmd).await,
        Ok(o) if o.status.success() && parse_version(&String::from_utf8_lossy(&o.stdout)).is_some_and(|v| v >= MIN_VERSION))
}

/// 포터블 Python을 받아(`python_pin`, 고정 해시와 다르면 풀지 않는다) `<root>/python-standalone`에 풀고 확인한다.
/// 받는 동안 `on_progress(받은 크기, 전체)`를 부른다. 형태를 지운 future를 돌려준다: reqwest 타입의 Send 판정이
/// 이것을 기다리는 데몬의 시작 경로마다 깊어지지 않게
pub fn install_portable<'a>(
    run: &'a dyn Run,
    root: &'a Path,
    on_progress: &'a (dyn Fn(u64, Option<u64>) + Sync),
) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
    Box::pin(async move {
        let (url, sha256) = crate::python_pin::download().context("이 플랫폼에서는 포터블 Python 자동 설치를 지원하지 않습니다")?;
        let portable_dir = root.join(PORTABLE_DIR);
        let archive = root.join("_python_download.tar.gz");
        // 기존 실패분 정리
        if portable_dir.exists() {
            let _ = std::fs::remove_dir_all(&portable_dir);
        }
        std::fs::create_dir_all(&portable_dir).with_context(|| format!("디렉토리 생성 실패: {}", portable_dir.display()))?;

        tracing::info!("포터블 Python {} 다운로드 중: {}", crate::python_pin::PYTHON_VERSION, url);
        let client = crate::tls::http_client_builder().build()?;
        crate::download::download_reporting(&client, &url, sha256.as_deref(), &archive, None, on_progress)
            .await
            .map_err(|e| anyhow::anyhow!("다운로드 실패: {}", e))?;
        // Python 배포판은 최소 수 MB다
        let len = std::fs::metadata(&archive).context("다운로드된 파일을 찾을 수 없습니다")?.len();
        if len < 1_000_000 {
            let _ = std::fs::remove_file(&archive);
            anyhow::bail!("다운로드된 파일이 너무 작습니다 ({} bytes)", len);
        }

        let mut tar = Command::new(crate::process::system_tool("tar"));
        tar.arg("-xzf").arg(&archive).arg("-C").arg(&portable_dir);
        hide_window(&mut tar);
        let output = run.output(&mut tar).await.context("tar 실행 실패")?;
        let _ = std::fs::remove_file(&archive);
        if !output.status.success() {
            anyhow::bail!("Python 아카이브 추출 실패: {}", String::from_utf8_lossy(&output.stderr));
        }

        let exe = portable_python(root);
        #[cfg(unix)]
        if exe.exists() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
        }
        if !exe.exists() {
            anyhow::bail!("포터블 Python 추출 후 실행 파일을 찾을 수 없습니다: {}", exe.display());
        }
        if !verify(run, &exe).await {
            anyhow::bail!("다운로드된 포터블 Python이 정상 동작하지 않습니다: {}", exe.display());
        }
        tracing::info!("포터블 Python 설치 완료: {}", exe.display());
        Ok(exe)
    })
}

/// `base`로 `<root>/python-env`를 새로 만들고(만들다 만 것은 지운다) pip을 올린 뒤 확인한다. venv의 Python을 돌려준다
pub async fn create_venv(run: &dyn Run, base: &Path, root: &Path) -> Result<PathBuf> {
    let venv_dir = root.join(VENV_DIR);
    if venv_dir.exists() {
        let _ = std::fs::remove_dir_all(&venv_dir);
    }
    std::fs::create_dir_all(root).with_context(|| format!("디렉토리 생성 실패: {}", root.display()))?;
    tracing::info!("Python venv 생성 중: {} (base: {})", venv_dir.display(), base.display());

    let mut cmd = Command::new(base);
    cmd.arg("-m").arg("venv").arg(&venv_dir);
    hide_window(&mut cmd);
    let output = run.output(&mut cmd).await.context("venv 생성 명령 실행 실패")?;
    if !output.status.success() {
        anyhow::bail!("venv 생성 실패: {}", String::from_utf8_lossy(&output.stderr));
    }

    let python = venv_python(root);
    // pip 업그레이드 (실패해도 치명적이지 않음). 받은 패키지 캐시는 데이터 폴더에 둔다(제거기가 지운다)
    let mut pip = Command::new(&python);
    pip.env("PIP_CACHE_DIR", crate::constants::resolve_pip_cache_dir()).args(["-m", "pip", "install", "--upgrade", "--quiet", "pip"]);
    hide_window(&mut pip);
    match run.output(&mut pip).await {
        Ok(o) if !o.status.success() => tracing::warn!("pip 업그레이드 실패: {}", String::from_utf8_lossy(&o.stderr)),
        Err(e) => tracing::warn!("pip 실행 실패: {}", e),
        Ok(_) => {}
    }

    if !verify(run, &python).await {
        anyhow::bail!("venv 생성 후 검증 실패");
    }
    Ok(python)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_with_or_without_the_prefix() {
        assert_eq!(parse_version("Python 3.12.8"), Some((3, 12)));
        assert_eq!(parse_version("  3.10.12\n"), Some((3, 10)));
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version(""), None);
    }

    /// 맥 앱 묶음 안은 쓰기가 되어도 런타임을 두지 않는다 (묶음에 파일을 더하면 서명이 깨진다)
    #[test]
    fn the_runtime_is_not_placed_inside_an_app_bundle() {
        if std::env::var_os("SABA_DATA_DIR").is_some() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("Saba-chan.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        assert_eq!(runtime_dir(Some(&exe_dir)).unwrap(), crate::constants::resolve_data_dir());

        let plain = tmp.path().join("portable");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(runtime_dir(Some(&plain)).unwrap(), plain);
    }
}
