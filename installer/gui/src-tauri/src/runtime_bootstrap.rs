//! 런타임 부트스트랩 — 포터블 Python 자동 설치
//!
//! 인스톨러가 설치 과정에서 **엔드유저가 Python을 별도로 설치할 필요 없이**
//! 포터블 런타임을 자동으로 다운로드하고 환경을 구성합니다.
//!
//! ## Python
//! - `python-build-standalone` 포터블 배포판 다운로드 (~20 MB)
//! - venv 생성 + pip 업그레이드

use saba_chan_updater_lib::constants;
use std::path::{Path, PathBuf};
use tokio::process::Command;

// ── Python 버전 (메인 앱과 동일) ────────────────────────────────
use saba_chan_updater_lib::python_pin::PYTHON_VERSION;
const PORTABLE_PYTHON_DIR: &str = "python-standalone";
const VENV_DIR_NAME: &str = "python-env";

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

    // ── 1) 포터블 Python 다운로드 (없을 때만) ──
    if !portable_exe.exists() || !verify_python(&portable_exe).await {
        tracing::info!("포터블 Python {} 다운로드 시작", PYTHON_VERSION);

        // 기존 실패분 정리
        if portable_dir.exists() {
            let _ = std::fs::remove_dir_all(&portable_dir);
        }

        let (url, sha256) = saba_chan_updater_lib::python_pin::download()
            .ok_or_else(|| "이 플랫폼에서는 포터블 Python 자동 설치를 지원하지 않습니다".to_string())?;

        let tmp_archive = data_dir.join("_python_download.tar.gz");

        download_file(&url, &tmp_archive)
            .await
            .map_err(|e| format!("Python 다운로드 실패: {}", e))?;
        // 고정 해시와 다르면 풀지 않는다
        if let Err(e) = saba_chan_updater_lib::python_pin::verify(&tmp_archive, sha256.as_deref()) {
            let _ = std::fs::remove_file(&tmp_archive);
            return Err(e);
        }

        std::fs::create_dir_all(&portable_dir)
            .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;

        extract_tar_gz(&tmp_archive, &portable_dir)
            .await
            .map_err(|e| format!("Python 추출 실패: {}", e))?;

        let _ = std::fs::remove_file(&tmp_archive);

        if !portable_exe.exists() {
            return Err(format!(
                "포터블 Python 추출 후 실행 파일을 찾을 수 없습니다: {}",
                portable_exe.display()
            ));
        }

        if !verify_python(&portable_exe).await {
            return Err("다운로드된 포터블 Python이 정상 동작하지 않습니다".into());
        }

        tracing::info!("포터블 Python 설치 완료: {}", portable_exe.display());
    } else {
        tracing::info!("포터블 Python 이미 존재: {}", portable_exe.display());
    }

    // ── 2) venv 생성 ──
    let venv_python = venv_python_exe(&venv_dir);

    // 기존 venv가 유효하면 건너뜀
    if venv_python.exists() && verify_python(&venv_python).await {
        tracing::info!("Python venv 이미 유효: {}", venv_python.display());
        return Ok(venv_python);
    }

    // 손상된 venv 제거
    if venv_dir.exists() {
        let _ = std::fs::remove_dir_all(&venv_dir);
    }

    tracing::info!("Python venv 생성 중: {}", venv_dir.display());

    let mut cmd = Command::new(&portable_exe);
    cmd.args(["-m", "venv", &venv_dir.to_string_lossy()]);
    apply_creation_flags(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("venv 생성 명령 실행 실패: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("venv 생성 실패: {}", stderr));
    }

    // pip 업그레이드 (실패해도 치명적이지 않음)
    let _ = run_pip(&venv_python, &["install", "--upgrade", "--quiet", "pip"]).await;

    // 검증
    if !verify_python(&venv_python).await {
        return Err("venv 생성 후 검증 실패".into());
    }

    tracing::info!("Python venv 준비 완료: {}", venv_python.display());
    Ok(venv_python)
}

/// Fix 5: 기존 런타임이 손상되었으면 강제 삭제 후 재설치하는 Python 설치
///
/// 기존 `setup_python`은 포터블 Python이 존재하면 스킵했지만,
/// 이 함수는 검증에 실패하면 기존 디렉토리를 삭제하고 재다운로드한다.
pub async fn setup_python_with_repair(data_dir: &Path) -> Result<PathBuf, String> {
    let portable_dir = data_dir.join(PORTABLE_PYTHON_DIR);
    let venv_dir = data_dir.join(VENV_DIR_NAME);
    let portable_exe = portable_python_exe(data_dir);

    // 포터블 Python이 존재하지만 검증 실패 → 삭제 후 재설치
    if portable_exe.exists() && !verify_python(&portable_exe).await {
        tracing::warn!(
            "기존 포터블 Python이 손상됨, 삭제 후 재설치: {}",
            portable_dir.display()
        );
        let _ = std::fs::remove_dir_all(&portable_dir);
    }

    // venv가 존재하지만 검증 실패 → 삭제
    let venv_python = venv_python_exe(&venv_dir);
    if venv_dir.exists() && (!venv_python.exists() || !verify_python(&venv_python).await) {
        tracing::warn!(
            "기존 Python venv가 손상됨, 삭제 후 재생성: {}",
            venv_dir.display()
        );
        let _ = std::fs::remove_dir_all(&venv_dir);
    }

    setup_python(data_dir).await
}

// ═══════════════════════════════════════════════════════════════
//  URL 생성
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
//  다운로드 & 추출 (OS 네이티브 도구)
// ═══════════════════════════════════════════════════════════════

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;
    }

    let output = {
        #[cfg(target_os = "windows")]
        {
            // URL·경로는 스크립트에 끼우지 않고 환경변수로 넘긴다 (따옴표가 든 경로도 그대로)
            let script = "$ProgressPreference = 'SilentlyContinue'; \
                 [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                 Invoke-WebRequest -Uri $env:SABA_DL_URL -OutFile $env:SABA_DL_DEST -UseBasicParsing";
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env("SABA_DL_URL", url)
                .env("SABA_DL_DEST", dest);
            apply_creation_flags(&mut cmd);
            cmd.output()
                .await
                .map_err(|e| format!("PowerShell 실행 실패: {}", e))?
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut cmd = Command::new("curl");
            cmd.args([
                "-fSL",
                "--retry",
                "3",
                "-o",
                &dest.to_string_lossy(),
                url,
            ]);
            apply_creation_flags(&mut cmd);
            cmd.output()
                .await
                .map_err(|e| format!("curl 실행 실패: {}", e))?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("다운로드 실패: {}", stderr));
    }

    // 최소 크기 검증
    let meta = std::fs::metadata(dest)
        .map_err(|_| "다운로드된 파일을 찾을 수 없습니다".to_string())?;
    if meta.len() < 1_000_000 {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "다운로드된 파일이 너무 작습니다 ({} bytes)",
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

async fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("디렉토리 생성 실패: {}", e))?;

    let mut cmd = Command::new("tar");
    cmd.args([
        "-xzf",
        &archive.to_string_lossy(),
        "-C",
        &dest.to_string_lossy(),
    ]);
    apply_creation_flags(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("tar 실행 실패: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tar 추출 실패: {}", stderr));
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

/// pip 명령 실행
async fn run_pip(python_exe: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(python_exe);
    cmd.arg("-m").arg("pip");
    for arg in args {
        cmd.arg(arg);
    }
    apply_creation_flags(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("pip 실행 실패: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pip 실행 실패: {}", stderr));
    }
    Ok(())
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
