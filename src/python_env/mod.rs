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
//!    b. 시스템 Python ≥ 3.10 → 사용
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
//! ## 새 Cargo 의존성: 없음
//! 다운로드는 OS 내장 도구(PowerShell / curl), 추출은 OS 내장 tar 사용.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::utils::apply_creation_flags;

// ── Portable Python Build Info ──────────────────────────────────
// python-build-standalone: https://github.com/indygreg/python-build-standalone
// `install_only_stripped` 변형: pip/venv 포함, 디버그 심볼 제거 (~20 MB)

const PYTHON_VERSION: &str = "3.12.8";
const PYTHON_RELEASE_TAG: &str = "20250106";

/// 시스템 Python 사용 시 최소 요구 버전
const MIN_PYTHON_VERSION: (u32, u32) = (3, 10);

const VENV_DIR_NAME: &str = "python-env";
const PORTABLE_DIR_NAME: &str = "python-standalone";

// ── Cached venv python path ─────────────────────────────────────

static VENV_PYTHON: OnceCell<PathBuf> = OnceCell::const_new();

/// 관리 venv의 Python 인터프리터 경로를 반환합니다.
/// 최초 호출 시 전체 부트스트랩(다운로드 포함)을 자동 수행합니다.
pub async fn get_python_path() -> Result<PathBuf> {
    VENV_PYTHON
        .get_or_try_init(|| ensure_venv())
        .await
        .cloned()
}

// ═══════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════

/// venv가 존재하고 유효한지 확인합니다. 없으면 전체 부트스트랩을 수행합니다.
/// 반환값: venv 내부 Python 실행 파일의 절대 경로
pub async fn ensure_venv() -> Result<PathBuf> {
    let data_dir = resolve_data_dir()?;
    let venv_dir = data_dir.join(VENV_DIR_NAME);
    let python_exe = venv_python_exe(&venv_dir);

    // ── Fast path: 기존 venv 유효 ──
    if python_exe.exists() && verify_python(&python_exe).await {
        tracing::debug!("Python venv 확인 완료: {}", python_exe.display());
        return Ok(python_exe);
    }

    // 손상된 venv 제거
    if venv_dir.exists() {
        tracing::warn!("기존 venv 손상, 재생성합니다...");
        let _ = std::fs::remove_dir_all(&venv_dir);
    }

    // ── Python 인터프리터 찾기 ──
    let base_python = find_base_python(&data_dir).await?;

    tracing::info!(
        "Python venv 생성 중: {} (base: {})",
        venv_dir.display(),
        base_python
    );

    if let Some(parent) = venv_dir.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("디렉토리 생성 실패: {}", parent.display()))?;
    }

    // ── venv 생성 ──
    let mut cmd = Command::new(&base_python);
    cmd.args(["-m", "venv", &venv_dir.to_string_lossy()]);
    apply_creation_flags(&mut cmd);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("venv 생성 실패: {}", stderr));
    }

    // pip 업그레이드 (실패해도 치명적이지 않음)
    let _ = run_pip(&python_exe, &["install", "--upgrade", "--quiet", "pip"]).await;

    // ── 검증 ──
    if !verify_python(&python_exe).await {
        return Err(anyhow::anyhow!("venv 생성 후 검증 실패"));
    }

    tracing::info!("Python venv 준비 완료: {}", python_exe.display());
    Ok(python_exe)
}

/// pip 패키지를 관리 venv에 설치합니다.
pub async fn pip_install(packages: &[&str]) -> Result<()> {
    let python_exe = get_python_path().await?;
    tracing::info!("pip install: {:?}", packages);
    let mut args = vec!["install", "--upgrade"];
    args.extend(packages);
    run_pip(&python_exe, &args).await
}

/// requirements.txt로부터 의존성을 설치합니다.
pub async fn pip_install_requirements(requirements_path: &Path) -> Result<()> {
    let python_exe = get_python_path().await?;
    tracing::info!("requirements 설치: {}", requirements_path.display());
    run_pip(
        &python_exe,
        &["install", "-r", &requirements_path.to_string_lossy()],
    )
    .await
}

/// 시스템에서 Python ≥ 3.10 을 탐지합니다.
pub async fn detect_system_python() -> Result<String> {
    let candidates = ["python", "python3", "py"];
    for cmd_name in candidates {
        let mut cmd = Command::new(cmd_name);
        cmd.arg("--version");
        apply_creation_flags(&mut cmd);

        if let Ok(output) = cmd.output().await {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout);
                if let Some((major, minor)) = parse_python_version(&ver) {
                    if (major, minor) >= MIN_PYTHON_VERSION {
                        tracing::info!(
                            "시스템 Python 발견: {} → {}",
                            cmd_name,
                            ver.trim()
                        );
                        return Ok(cmd_name.to_string());
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

/// 진단 정보를 JSON으로 반환합니다. (IPC 엔드포인트용)
pub async fn status() -> serde_json::Value {
    let data_dir = match resolve_data_dir() {
        Ok(d) => d,
        Err(e) => {
            return serde_json::json!({
                "available": false,
                "error": e.to_string(),
            })
        }
    };

    let venv_dir = data_dir.join(VENV_DIR_NAME);
    let python_exe = venv_python_exe(&venv_dir);
    let portable_exe = portable_python_exe(&data_dir);
    let venv_ok = python_exe.exists() && verify_python(&python_exe).await;

    let mut info = serde_json::json!({
        "available": venv_ok,
        "venv_dir": venv_dir.to_string_lossy(),
        "venv_python": python_exe.to_string_lossy(),
        "portable_python_installed": portable_exe.exists(),
        "portable_python_path": portable_exe.to_string_lossy(),
        "python_version_bundled": PYTHON_VERSION,
    });

    if venv_ok {
        if let Ok(ver) = get_version(&python_exe).await {
            info["python_version"] = serde_json::json!(ver);
        }
    }

    if let Ok(url) = python_download_url() {
        info["portable_download_url"] = serde_json::json!(url);
    }

    match detect_system_python().await {
        Ok(cmd) => {
            info["system_python"] = serde_json::json!(cmd);
        }
        Err(_) => {
            info["system_python"] = serde_json::json!(null);
        }
    }

    info
}

// ═══════════════════════════════════════════════════════════════
//  Internal: Python 탐색 & 포터블 다운로드
// ═══════════════════════════════════════════════════════════════

/// Python 인터프리터를 찾습니다: 포터블 → 시스템 → 자동 다운로드
async fn find_base_python(data_dir: &Path) -> Result<String> {
    // 1) 이미 다운로드된 포터블 Python
    let portable_exe = portable_python_exe(data_dir);
    if portable_exe.exists() && verify_python(&portable_exe).await {
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

/// python-build-standalone 다운로드 URL 생성
fn python_download_url() -> Result<String> {
    // 환경변수 오버라이드 (미러, 사내망 등)
    if let Ok(custom_url) = std::env::var("SABA_PYTHON_URL") {
        return Ok(custom_url);
    }

    let triple = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else {
        return Err(anyhow::anyhow!(
            "이 플랫폼에서는 포터블 Python 자동 설치를 지원하지 않습니다. \
             Python {}.{} 이상을 수동으로 설치해 주세요.",
            MIN_PYTHON_VERSION.0,
            MIN_PYTHON_VERSION.1
        ));
    };

    Ok(format!(
        "https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz",
        tag = PYTHON_RELEASE_TAG,
        ver = PYTHON_VERSION,
        triple = triple,
    ))
}

/// 포터블 Python 다운로드 → 추출 → 검증
async fn download_portable_python(data_dir: &Path) -> Result<PathBuf> {
    let url = python_download_url()?;
    let portable_dir = data_dir.join(PORTABLE_DIR_NAME);
    let tmp_archive = data_dir.join("_python_download.tar.gz");

    // 기존 실패분 정리
    if portable_dir.exists() {
        let _ = std::fs::remove_dir_all(&portable_dir);
    }

    // ── 다운로드 ──
    tracing::info!("포터블 Python 다운로드 중: {}", url);
    download_file(&url, &tmp_archive)
        .await
        .context(
            "Python 다운로드 실패. \
             인터넷 연결을 확인하거나, Python 3.10 이상을 수동 설치해 주세요.",
        )?;

    // ── 추출 ──
    tracing::info!("포터블 Python 추출 중...");
    std::fs::create_dir_all(&portable_dir)?;
    extract_tar_gz(&tmp_archive, &portable_dir)
        .await
        .context("Python 아카이브 추출 실패")?;

    // 임시 아카이브 삭제
    let _ = std::fs::remove_file(&tmp_archive);

    // ── 실행 권한 (Unix) ──
    let exe = portable_python_exe(data_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if exe.exists() {
            let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
        }
    }

    // ── 검증 ──
    if !exe.exists() {
        return Err(anyhow::anyhow!(
            "포터블 Python 추출 후 실행 파일을 찾을 수 없습니다: {}",
            exe.display()
        ));
    }
    if !verify_python(&exe).await {
        return Err(anyhow::anyhow!(
            "다운로드된 포터블 Python이 정상 동작하지 않습니다: {}",
            exe.display()
        ));
    }

    tracing::info!(
        "포터블 Python 설치 완료: {} ({:.1} MB)",
        exe.display(),
        dir_size_mb(&portable_dir)
    );
    Ok(exe)
}

// ═══════════════════════════════════════════════════════════════
//  Internal: 다운로드 & 추출 (OS 네이티브 도구)
// ═══════════════════════════════════════════════════════════════

/// 파일 다운로드 — Windows: PowerShell, Linux/macOS: curl
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let output = {
        #[cfg(target_os = "windows")]
        {
            // PowerShell .NET WebClient — 진행률 비활성화로 최대 속도
            let script = format!(
                "$ProgressPreference = 'SilentlyContinue'; \
                 [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                 Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
                url,
                dest.to_string_lossy()
            );
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            apply_creation_flags(&mut cmd);
            cmd.output().await?
        }
        #[cfg(not(target_os = "windows"))]
        {
            // curl (macOS/Linux 기본 내장)
            let mut cmd = Command::new("curl");
            cmd.args([
                "-fSL",
                "--retry", "3",
                "-o",
                &dest.to_string_lossy().into_owned(),
                url,
            ]);
            apply_creation_flags(&mut cmd);
            cmd.output().await?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("다운로드 실패: {}", stderr));
    }

    // 크기 검증 (Python 배포판은 최소 수 MB)
    let meta = std::fs::metadata(dest).context("다운로드된 파일을 찾을 수 없습니다")?;
    if meta.len() < 1_000_000 {
        let _ = std::fs::remove_file(dest);
        return Err(anyhow::anyhow!(
            "다운로드된 파일이 너무 작습니다 ({} bytes). URL이 올바른지 확인하세요.",
            meta.len()
        ));
    }

    tracing::info!(
        "다운로드 완료: {} ({:.1} MB)",
        dest.display(),
        meta.len() as f64 / 1_048_576.0
    );
    Ok(())
}

/// .tar.gz 추출 — Windows 10+ / Linux / macOS 내장 `tar` 명령 사용
async fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;

    let mut cmd = Command::new("tar");
    cmd.args([
        "-xzf",
        &archive.to_string_lossy(),
        "-C",
        &dest.to_string_lossy(),
    ]);
    apply_creation_flags(&mut cmd);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("tar 추출 실패: {}", stderr));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
//  Internal: 경로 & 유틸리티
// ═══════════════════════════════════════════════════════════════

/// 사바쨩 데이터 디렉토리 결정
fn resolve_data_dir() -> Result<PathBuf> {
    // 1) 환경변수 오버라이드
    if let Ok(dir) = std::env::var("SABA_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }

    // 2) exe 옆 (포터블 배포 모드)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if is_dir_writable(exe_dir) {
                return Ok(exe_dir.to_path_buf());
            }
        }
    }

    // 3) 플랫폼별 앱 데이터
    platform_data_dir()
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA").context("APPDATA 환경변수 없음")?;
    Ok(PathBuf::from(appdata).join("saba-chan"))
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME 환경변수 없음")?;
    Ok(PathBuf::from(home).join(".local/share/saba-chan"))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME 환경변수 없음")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support/saba-chan"))
}

/// 포터블 Python 실행 파일 경로
/// 추출 레이아웃: `<portable_dir>/python/python.exe` (Win) 또는 `.../python/bin/python3` (Unix)
fn portable_python_exe(data_dir: &Path) -> PathBuf {
    let base = data_dir.join(PORTABLE_DIR_NAME).join("python");
    #[cfg(target_os = "windows")]
    {
        base.join("python.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        base.join("bin").join("python3")
    }
}

/// venv 내 Python 실행 파일 경로
fn venv_python_exe(venv_dir: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        venv_dir.join("Scripts").join("python.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        venv_dir.join("bin").join("python")
    }
}

/// 디렉토리에 쓰기 가능한지 확인
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

/// Python 실행 파일이 정상 동작하는지 확인
async fn verify_python(exe: &Path) -> bool {
    let mut cmd = Command::new(exe);
    cmd.args([
        "-c",
        "import sys; v=sys.version_info; print(f'{v.major}.{v.minor}.{v.micro}')",
    ]);
    apply_creation_flags(&mut cmd);
    matches!(cmd.output().await, Ok(o) if o.status.success())
}

/// Python --version 문자열 반환
async fn get_version(exe: &Path) -> Result<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    apply_creation_flags(&mut cmd);
    let output = cmd.output().await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// pip 명령 실행
async fn run_pip(python_exe: &Path, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new(python_exe);
    cmd.arg("-m").arg("pip");
    for arg in args {
        cmd.arg(arg);
    }
    apply_creation_flags(&mut cmd);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("pip 실행 실패: {}", stderr));
    }
    Ok(())
}

/// "Python 3.12.8" → (3, 12)
fn parse_python_version(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let ver_part = s
        .strip_prefix("Python ")
        .or_else(|| s.strip_prefix("python "))
        .unwrap_or(s);
    let parts: Vec<&str> = ver_part.split('.').collect();
    if parts.len() >= 2 {
        let major = parts[0].trim().parse().ok()?;
        let minor = parts[1].trim().parse().ok()?;
        Some((major, minor))
    } else {
        None
    }
}

/// 디렉토리 총 크기 (MB)
fn dir_size_mb(dir: &Path) -> f64 {
    fn walk(dir: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                } else if path.is_dir() {
                    total += walk(&path);
                }
            }
        }
        total
    }
    walk(dir) as f64 / 1_048_576.0
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python_version() {
        assert_eq!(parse_python_version("Python 3.12.8"), Some((3, 12)));
        assert_eq!(parse_python_version("Python 3.10.0"), Some((3, 10)));
        assert_eq!(parse_python_version("Python 2.7.18"), Some((2, 7)));
        assert_eq!(parse_python_version("  Python 3.11.5  "), Some((3, 11)));
        assert_eq!(parse_python_version("garbage"), None);
        assert_eq!(parse_python_version(""), None);
    }

    #[test]
    fn test_python_download_url_format() {
        let url = python_download_url().unwrap();
        assert!(url.contains("python-build-standalone"));
        assert!(url.contains(PYTHON_VERSION));
        assert!(url.ends_with(".tar.gz"));
    }

    #[test]
    fn test_venv_python_exe_path() {
        let dir = PathBuf::from(if cfg!(target_os = "windows") {
            "C:\\test-venv"
        } else {
            "/tmp/test-venv"
        });
        let exe = venv_python_exe(&dir);
        #[cfg(target_os = "windows")]
        assert!(exe.to_string_lossy().contains("Scripts\\python.exe"));
        #[cfg(not(target_os = "windows"))]
        assert!(exe.to_string_lossy().contains("bin/python"));
    }

    #[test]
    fn test_portable_python_exe_path() {
        let dir = PathBuf::from(if cfg!(target_os = "windows") {
            "C:\\test-data"
        } else {
            "/tmp/test-data"
        });
        let exe = portable_python_exe(&dir);
        let s = exe.to_string_lossy();
        assert!(s.contains(PORTABLE_DIR_NAME));
        assert!(s.contains("python"));
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

    #[tokio::test]
    async fn test_status_returns_json() {
        let s = status().await;
        assert!(s.get("available").is_some());
        assert!(s.get("venv_dir").is_some());
        assert!(s.get("portable_python_installed").is_some());
    }
}
