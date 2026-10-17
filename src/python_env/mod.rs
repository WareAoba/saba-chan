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
//!    b. 없음 → 포터블 Python 자동 다운로드 (~20 MB)
//! 4. 찾은 Python으로 venv 생성 → pip 업그레이드 → 검증 → 캐시
//!
//! 포터블 바이너리만 사용한다. 사용자 PC의 시스템 Python은 환경 오염
//! 회피를 위해 절대 사용하지 않는다 (PATH/레지스트리 변경 금지 원칙).
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
//! `<data_dir>`는 데몬 공통 데이터 디렉토리(`SABA_DATA_DIR` 또는 `%APPDATA%/saba-chan`)다.
//! 인스톨러(`runtime_bootstrap.rs`)도 같은 위치에 설치한다.
//!
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

const VENV_DIR_NAME: &str = "python-env";
const PORTABLE_DIR_NAME: &str = "python-standalone";
const PYTHON_URL_ENV: &str = "SABA_PYTHON_URL";
const PYTHON_SHA256_ENV: &str = "SABA_PYTHON_SHA256";

// ── Cached venv python path ─────────────────────────────────────

static VENV_PYTHON: OnceCell<PathBuf> = OnceCell::const_new();

/// 관리 venv의 Python 인터프리터 경로를 반환합니다.
/// 최초 호출 시 전체 부트스트랩(다운로드 포함)을 자동 수행합니다.
///
/// 부트스트랩 진입점은 이 함수 하나다. `OnceCell`이 동시 호출을 직렬화하므로
/// preflight·플러그인·익스텐션 훅이 동시에 불러도 venv 삭제/다운로드가 겹치지 않는다.
/// 실패는 캐시하지 않아 다음 호출에서 다시 시도한다.
pub async fn get_python_path() -> Result<PathBuf> {
    VENV_PYTHON
        .get_or_try_init(bootstrap_venv)
        .await
        .cloned()
}

// ═══════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════

/// venv가 존재하고 유효한지 확인합니다. 없으면 전체 부트스트랩을 수행합니다.
/// 반환값: venv 내부 Python 실행 파일의 절대 경로
async fn bootstrap_venv() -> Result<PathBuf> {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
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

// ═══════════════════════════════════════════════════════════════
//  Internal: Python 탐색 & 포터블 다운로드
// ═══════════════════════════════════════════════════════════════

/// Python 인터프리터를 찾습니다: 포터블 전용 (시스템 Python은 사용하지 않음).
async fn find_base_python(data_dir: &Path) -> Result<String> {
    // 1) 이미 다운로드된 포터블 Python
    let portable_exe = portable_python_exe(data_dir);
    if portable_exe.exists() && verify_python(&portable_exe).await {
        tracing::info!("포터블 Python 사용: {}", portable_exe.display());
        return Ok(portable_exe.to_string_lossy().into_owned());
    }

    // 2) 자동 다운로드 (포터블 전용)
    tracing::info!(
        "포터블 Python을 찾을 수 없습니다. {} 다운로드를 시작합니다...",
        PYTHON_VERSION
    );
    let exe = download_portable_python(data_dir).await?;
    Ok(exe.to_string_lossy().into_owned())
}

/// python-build-standalone 다운로드 URL 생성
fn python_download_url() -> Result<String> {
    // 환경변수 오버라이드 (미러, 사내망 등)
    if let Ok(custom_url) = std::env::var(PYTHON_URL_ENV) {
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
            "이 플랫폼에서는 포터블 Python 자동 설치를 지원하지 않습니다."
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

    let expected_hash = resolve_expected_python_sha256(&url).await?;

    // ── 다운로드 ──
    tracing::info!("포터블 Python 다운로드 중: {}", url);
    // Python 배포판은 최소 수 MB — 그보다 작으면 오류 페이지 등으로 간주
    download_file(&url, &tmp_archive, 1_000_000)
        .await
        .context(
            "Python 다운로드 실패. \
             인터넷 연결을 확인하고 다시 시도해 주세요.",
        )?;

    // ── SHA256 검증 ──
    if let Some(expected) = expected_hash.as_ref() {
        let actual = sha256_file_hex(&tmp_archive)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&tmp_archive);
            return Err(anyhow::anyhow!(
                "Python 아카이브 SHA256 검증 실패: expected {}, got {}",
                expected, actual
            ));
        }
        tracing::info!("Python 아카이브 SHA256 검증 성공");
    }

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

/// python-build-standalone GitHub 릴리스의 `SHA256SUMS` 를 조회해
/// 다운로드 URL 에 해당하는 파일의 SHA-256 해시를 반환합니다.
/// 스키마: `https://github.com/indygreg/python-build-standalone/releases/download/{TAG}/SHA256SUMS`
async fn fetch_upstream_python_sha256(archive_url: &str) -> Result<String> {
    let sums_url = match archive_url.rsplit_once('/') {
        Some((base, _)) => format!("{}/SHA256SUMS", base),
        None => return Err(anyhow::anyhow!("archive_url 파싱 실패: {}", archive_url)),
    };
    let filename = archive_url
        .rsplit('/')
        .next()
        .ok_or_else(|| anyhow::anyhow!("archive_url에서 파일명 추출 실패"))?;

    let tmp = std::env::temp_dir().join(format!(
        "saba-chan-python-shasums-{}.txt",
        std::process::id()
    ));
    download_file(&sums_url, &tmp, 0)
        .await
        .with_context(|| format!("SHA256SUMS 다운로드 실패: {}", sums_url))?;
    let body = std::fs::read_to_string(&tmp)
        .with_context(|| format!("SHA256SUMS 읽기 실패: {}", tmp.display()))?;
    let _ = std::fs::remove_file(&tmp);

    find_hash_for_filename(&body, filename)
        .ok_or_else(|| anyhow::anyhow!("SHA256SUMS에 {} 해시가 없습니다", filename))
}

/// `SHA256SUMS` 본문(`<hex>  <파일명>` 줄 목록)에서 파일명에 해당하는 해시를 찾는다.
fn find_hash_for_filename(body: &str, filename: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == filename && is_sha256_hex(hash)).then(|| hash.to_ascii_lowercase())
    })
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// 파일의 SHA-256을 소문자 hex로 계산한다.
fn sha256_file_hex(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("해시 계산용 파일 열기 실패: {}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

async fn resolve_expected_python_sha256(archive_url: &str) -> Result<Option<String>> {
    if let Ok(hash_hex) = std::env::var(PYTHON_SHA256_ENV) {
        let hash_hex = hash_hex.trim();
        if !is_sha256_hex(hash_hex) {
            return Err(anyhow::anyhow!("{} 값이 올바른 SHA256 hex가 아닙니다", PYTHON_SHA256_ENV));
        }
        return Ok(Some(hash_hex.to_ascii_lowercase()));
    }

    match fetch_upstream_python_sha256(archive_url).await {
        Ok(hash) => Ok(Some(hash)),
        Err(e) if std::env::var(PYTHON_URL_ENV).is_ok() => {
            tracing::warn!(
                "커스텀 Python URL의 SHA256 확보 실패(검증 스킵). 운영 배포에서는 {} 설정 권장: {}",
                PYTHON_SHA256_ENV,
                e
            );
            Ok(None)
        }
        Err(e) => Err(e).context("공식 Python 다운로드의 SHA256 확보 실패"),
    }
}

/// 파일 다운로드 — Windows: PowerShell, Linux/macOS: curl.
/// `min_bytes`보다 작은 결과는 실패로 처리하고 삭제한다.
async fn download_file(url: &str, dest: &Path, min_bytes: u64) -> Result<()> {
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
                &dest.to_string_lossy(),
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

    let meta = std::fs::metadata(dest).context("다운로드된 파일을 찾을 수 없습니다")?;
    if meta.len() < min_bytes {
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
    fn test_python_download_url_format() {
        let url = python_download_url().unwrap();
        assert!(url.contains("python-build-standalone"));
        assert!(url.contains(PYTHON_VERSION));
        assert!(url.ends_with(".tar.gz"));
    }

    #[test]
    fn test_python_sha256_hex_validation() {
        assert!(!is_sha256_hex("not-a-sha256"));
        assert!(is_sha256_hex(&"a".repeat(64)));
    }

    #[test]
    fn find_hash_for_filename_matches_exact_name() {
        let h1 = "1".repeat(64);
        let h2 = "2".repeat(64);
        let body = format!("{}  other.tar.gz
{} *target.tar.gz
", h1, h2);
        assert_eq!(find_hash_for_filename(&body, "target.tar.gz"), Some(h2));
        assert_eq!(find_hash_for_filename(&body, "missing.tar.gz"), None);
    }

    #[test]
    fn sha256_file_hex_matches_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file_hex(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
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
}
