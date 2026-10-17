//! 런타임 부트스트랩 — 포터블 Python + Node.js 자동 설치
//!
//! 인스톨러가 설치 과정에서 **엔드유저가 Python/Node.js를 별도로 설치할 필요 없이**
//! 포터블 런타임을 자동으로 다운로드하고 환경을 구성합니다.
//! 기존 런타임이 검증에 실패하면 삭제 후 다시 받는다.
//!
//! ## Python
//! - `python-build-standalone` 포터블 배포판 다운로드 (~20 MB)
//! - venv 생성 + pip 업그레이드
//!
//! ## Node.js
//! - nodejs.org 포터블 배포판 다운로드 (~30 MB)

use crate::runtime_pins::{RuntimePin, NODE_VERSION, PYTHON_VERSION};
use saba_chan_updater_lib::constants;
use saba_chan_updater_lib::integrity::{download_verified, FileHash};
use std::path::{Path, PathBuf};
use tokio::process::Command;

const PORTABLE_PYTHON_DIR: &str = "python-standalone";
const VENV_DIR_NAME: &str = "python-env";
const PORTABLE_NODE_DIR: &str = "node-portable";

// ═══════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════

/// 포터블 Python 다운로드 + venv 생성
///
/// `data_dir`: 런타임을 설치할 루트 디렉토리 (`%APPDATA%/saba-chan`)
///
/// 반환: venv 내 Python 실행 파일 경로
pub async fn setup_python(data_dir: &Path) -> Result<PathBuf, String> {
    let portable_dir = data_dir.join(PORTABLE_PYTHON_DIR);
    let venv_dir = data_dir.join(VENV_DIR_NAME);
    let portable_exe = portable_python_exe(data_dir);

    // ── 1) 포터블 Python 다운로드 (없거나 손상됐을 때만) ──
    if verify_python(&portable_exe).await {
        tracing::info!("포터블 Python 이미 존재: {}", portable_exe.display());
    } else {
        tracing::info!("포터블 Python {} 다운로드 시작", PYTHON_VERSION);
        let pin = current_pin()?;
        download_and_extract(
            &pin.python_url(),
            pin.python_sha256,
            &data_dir.join("_python_download.tar.gz"),
            &portable_dir,
        )
        .await
        .map_err(|e| format!("Python 설치 실패: {}", e))?;

        if !verify_python(&portable_exe).await {
            return Err(format!(
                "다운로드된 포터블 Python이 정상 동작하지 않습니다: {}",
                portable_exe.display()
            ));
        }
        tracing::info!("포터블 Python 설치 완료: {}", portable_exe.display());
    }

    // ── 2) venv 생성 ──
    let venv_python = venv_python_exe(&venv_dir);
    if verify_python(&venv_python).await {
        tracing::info!("Python venv 이미 유효: {}", venv_python.display());
        return Ok(venv_python);
    }

    // 손상된 venv 제거
    if venv_dir.exists() {
        let _ = std::fs::remove_dir_all(&venv_dir);
    }

    tracing::info!("Python venv 생성 중: {}", venv_dir.display());
    let mut cmd = Command::new(&portable_exe);
    cmd.arg("-m").arg("venv").arg(&venv_dir);
    run_checked(cmd, "venv 생성").await?;

    // pip 업그레이드 (실패해도 치명적이지 않음)
    let mut pip = Command::new(&venv_python);
    pip.args(["-m", "pip", "install", "--upgrade", "--quiet", "pip"]);
    if let Err(e) = run_checked(pip, "pip 업그레이드").await {
        tracing::warn!("{}", e);
    }

    if !verify_python(&venv_python).await {
        return Err("venv 생성 후 검증 실패".into());
    }

    tracing::info!("Python venv 준비 완료: {}", venv_python.display());
    Ok(venv_python)
}

/// 포터블 Node.js 다운로드
///
/// `data_dir`: 런타임을 설치할 루트 디렉토리 (`%APPDATA%/saba-chan`)
///
/// 반환: node 실행 파일 경로
pub async fn setup_node(data_dir: &Path) -> Result<PathBuf, String> {
    if let Some(exe) = find_portable_node_exe(data_dir) {
        if verify_node(&exe).await {
            tracing::info!("포터블 Node.js 이미 존재: {}", exe.display());
            return Ok(exe);
        }
        tracing::warn!("기존 포터블 Node.js가 손상됨, 재설치: {}", exe.display());
    }

    tracing::info!("포터블 Node.js v{} 다운로드 시작", NODE_VERSION);
    let pin = current_pin()?;
    let portable_dir = data_dir.join(PORTABLE_NODE_DIR);
    download_and_extract(
        &pin.node_url(),
        pin.node_sha256,
        &data_dir.join(format!("_node_download.{}", pin.node_ext)),
        &portable_dir,
    )
    .await
    .map_err(|e| format!("Node.js 설치 실패: {}", e))?;

    let exe = find_portable_node_exe(data_dir).ok_or_else(|| {
        format!(
            "포터블 Node.js 추출 후 실행 파일을 찾을 수 없습니다. 디렉토리: {}",
            portable_dir.display()
        )
    })?;

    // Unix 실행 권한
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }

    if !verify_node(&exe).await {
        return Err("다운로드된 포터블 Node.js가 정상 동작하지 않습니다".into());
    }

    tracing::info!("포터블 Node.js 설치 완료: {}", exe.display());
    Ok(exe)
}

// ═══════════════════════════════════════════════════════════════
//  다운로드 & 추출
// ═══════════════════════════════════════════════════════════════

fn current_pin() -> Result<&'static RuntimePin, String> {
    RuntimePin::current().ok_or_else(|| {
        format!(
            "이 플랫폼({}-{})에서는 포터블 런타임 자동 설치를 지원하지 않습니다",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

/// 기존 `dest_dir` 를 비우고, 아카이브를 받아 SHA-256 검증 후 추출한다.
async fn download_and_extract(
    url: &str,
    sha256_hex: &str,
    archive: &Path,
    dest_dir: &Path,
) -> Result<(), String> {
    if dest_dir.exists() {
        let _ = std::fs::remove_dir_all(dest_dir);
    }
    download_file_verified(url, archive, sha256_hex).await?;
    let result = extract_archive(archive, dest_dir).await;
    let _ = std::fs::remove_file(archive);
    result
}

/// 파일을 다운로드하고 SHA-256 해시를 검증합니다.
///
/// 해시 불일치는 **치명적** — 파일 삭제, 재시도 없음.
/// 셸을 거치지 않으므로 경로에 따옴표·특수문자가 있어도 안전하다.
async fn download_file_verified(url: &str, dest: &Path, sha256_hex: &str) -> Result<(), String> {
    let expected = FileHash::parse_hex(sha256_hex)
        .map_err(|e| format!("고정 체크섬 형식이 잘못됨: {} ({})", sha256_hex, e))?;
    let client = reqwest::Client::new();
    download_verified(&client, url, dest, Some(&expected), None)
        .await
        .map_err(|e| format!("다운로드/무결성 검증 실패 ({}): {}", url, e))?;
    tracing::info!("포터블 런타임 다운로드 + 해시 검증 완료: {}", dest.display());
    Ok(())
}

/// `tar -xf` 로 tar.gz / zip 을 추출한다.
/// Windows 10+ 의 tar(bsdtar)는 zip 도 처리한다.
async fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("디렉토리 생성 실패: {}", e))?;
    let mut cmd = Command::new("tar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(dest);
    run_checked(cmd, "아카이브 추출").await
}

/// 명령을 실행하고 실패하면 stderr 를 담은 에러를 반환한다.
async fn run_checked(mut cmd: Command, what: &str) -> Result<(), String> {
    apply_creation_flags(&mut cmd);
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("{} 실행 실패: {}", what, e))?;
    if !output.status.success() {
        return Err(format!(
            "{} 실패: {}",
            what,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
//  경로 & 검증 유틸리티
// ═══════════════════════════════════════════════════════════════

/// 포터블 Python 실행 파일 경로
fn portable_python_exe(data_dir: &Path) -> PathBuf {
    let base = data_dir.join(PORTABLE_PYTHON_DIR).join("python");
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

/// 포터블 Node.js 실행 파일을 찾습니다.
fn find_portable_node_exe(data_dir: &Path) -> Option<PathBuf> {
    let portable_dir = data_dir.join(PORTABLE_NODE_DIR);
    if !portable_dir.exists() {
        return None;
    }

    // 1) 직접 존재 확인
    let direct = node_exe_in(&portable_dir);
    if direct.exists() {
        return Some(direct);
    }

    // 2) node-v*-*/ 서브디렉토리 탐색
    if let Ok(entries) = std::fs::read_dir(&portable_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("node-v") || name_str.starts_with("node-") {
                    let candidate = node_exe_in(&path);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    None
}

/// 주어진 디렉토리 기준 node 실행 파일 경로
fn node_exe_in(dir: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dir.join("node.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        dir.join("bin").join("node")
    }
}

/// Python 실행 파일이 정상 동작하는지 확인
async fn verify_python(exe: &Path) -> bool {
    if !exe.exists() {
        return false;
    }
    let mut cmd = Command::new(exe);
    cmd.args([
        "-c",
        "import sys; v=sys.version_info; print(f'{v.major}.{v.minor}.{v.micro}')",
    ]);
    apply_creation_flags(&mut cmd);
    matches!(cmd.output().await, Ok(o) if o.status.success())
}

/// Node.js 실행 파일이 정상 동작하는지 확인
async fn verify_node(exe: &Path) -> bool {
    if !exe.exists() {
        return false;
    }
    let mut cmd = Command::new(exe);
    cmd.args(["--version"]);
    apply_creation_flags(&mut cmd);
    match cmd.output().await {
        Ok(o) => {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .starts_with('v')
        }
        Err(_) => false,
    }
}

/// 런타임 데이터 디렉토리 결정 (메인 앱과 동일한 로직)
///
/// 설치 디렉토리가 쓰기 가능하면 그곳에, 아니면 `%APPDATA%/saba-chan`에 설치합니다.
pub fn resolve_runtime_data_dir(install_dir: &Path) -> PathBuf {
    // 1) install_dir에 쓰기 가능한가 확인
    let probe = install_dir.join(".saba-write-test");
    if std::fs::write(&probe, b"test").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return install_dir.to_path_buf();
    }

    // 2) %APPDATA%/saba-chan (Windows)
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join(constants::APP_NAME);
            let _ = std::fs::create_dir_all(&dir);
            return dir;
        }
    }

    // 3) ~/.local/share/saba-chan (Linux/macOS)
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let dir = PathBuf::from(home)
                .join(".local")
                .join("share")
                .join(constants::APP_NAME);
            let _ = std::fs::create_dir_all(&dir);
            return dir;
        }
    }

    // fallback
    install_dir.to_path_buf()
}

/// 콘솔 Windows 숨기기 (Windows에서 자식 프로세스 콘솔 창 방지)
fn apply_creation_flags(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd; // unused on non-Windows
}
