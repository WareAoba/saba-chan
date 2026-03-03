//! 런타임 부트스트랩 — 포터블 Python + Node.js 자동 설치
//!
//! 인스톨러가 설치 과정에서 **엔드유저가 Python/Node.js를 별도로 설치할 필요 없이**
//! 포터블 런타임을 자동으로 다운로드하고 환경을 구성합니다.
//!
//! ## Python
//! - `python-build-standalone` 포터블 배포판 다운로드 (~20 MB)
//! - venv 생성 + pip 업그레이드
//!
//! ## Node.js
//! - nodejs.org 포터블 배포판 다운로드 (~30 MB)
//! - Discord Bot `npm install` 실행

use std::path::{Path, PathBuf};
use tokio::process::Command;

// ── Python 버전 (메인 앱과 동일) ────────────────────────────────
const PYTHON_VERSION: &str = "3.12.8";
const PYTHON_RELEASE_TAG: &str = "20250106";
const PORTABLE_PYTHON_DIR: &str = "python-standalone";
const VENV_DIR_NAME: &str = "python-env";

// ── Node.js 버전 (메인 앱과 동일) ───────────────────────────────
const NODE_VERSION: &str = "22.14.0";
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

    // ── 1) 포터블 Python 다운로드 (없을 때만) ──
    if !portable_exe.exists() || !verify_python(&portable_exe).await {
        tracing::info!("포터블 Python {} 다운로드 시작", PYTHON_VERSION);

        // 기존 실패분 정리
        if portable_dir.exists() {
            let _ = std::fs::remove_dir_all(&portable_dir);
        }

        let url = python_download_url()
            .map_err(|e| format!("Python 다운로드 URL 생성 실패: {}", e))?;

        let tmp_archive = data_dir.join("_python_download.tar.gz");

        download_file(&url, &tmp_archive)
            .await
            .map_err(|e| format!("Python 다운로드 실패: {}", e))?;

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

/// 포터블 Node.js 다운로드
///
/// `data_dir`: 런타임을 설치할 루트 디렉토리 (`%APPDATA%/saba-chan`)
///
/// 반환: node 실행 파일 경로
pub async fn setup_node(data_dir: &Path) -> Result<PathBuf, String> {
    // 기존 포터블 Node.js 확인
    if let Some(exe) = find_portable_node_exe(data_dir) {
        if verify_node(&exe).await {
            tracing::info!("포터블 Node.js 이미 존재: {}", exe.display());
            return Ok(exe);
        }
    }

    let portable_dir = data_dir.join(PORTABLE_NODE_DIR);

    tracing::info!("포터블 Node.js v{} 다운로드 시작", NODE_VERSION);

    // 기존 실패분 정리
    if portable_dir.exists() {
        let _ = std::fs::remove_dir_all(&portable_dir);
    }

    let url = node_download_url()
        .map_err(|e| format!("Node.js 다운로드 URL 생성 실패: {}", e))?;

    let is_zip = url.ends_with(".zip");
    let tmp_ext = if is_zip { "zip" } else { "tar.gz" };
    let tmp_archive = data_dir.join(format!("_node_download.{}", tmp_ext));

    download_file(&url, &tmp_archive)
        .await
        .map_err(|e| format!("Node.js 다운로드 실패: {}", e))?;

    std::fs::create_dir_all(&portable_dir)
        .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;

    if is_zip {
        extract_zip_archive(&tmp_archive, &portable_dir)
            .await
            .map_err(|e| format!("Node.js zip 추출 실패: {}", e))?;
    } else {
        extract_tar_gz(&tmp_archive, &portable_dir)
            .await
            .map_err(|e| format!("Node.js tar.gz 추출 실패: {}", e))?;
    }

    let _ = std::fs::remove_file(&tmp_archive);

    // 실행 파일 찾기
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
        if exe.exists() {
            let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
        }
    }

    if !verify_node(&exe).await {
        return Err("다운로드된 포터블 Node.js가 정상 동작하지 않습니다".into());
    }

    tracing::info!("포터블 Node.js 설치 완료: {}", exe.display());
    Ok(exe)
}

/// Discord Bot의 npm install 실행
///
/// `node_exe`: node 실행 파일 경로
/// `bot_dir`: discord_bot 디렉토리 (install_dir/discord_bot)
pub async fn npm_install(node_exe: &Path, bot_dir: &Path) -> Result<(), String> {
    if !bot_dir.join("package.json").exists() {
        tracing::info!("Discord Bot package.json 없음, npm install 건너뜀");
        return Ok(());
    }

    // npm 경로: node.exe 옆의 npm.cmd (Windows) 또는 bin/npm (Unix)
    let npm_path = find_npm_path(node_exe)?;

    tracing::info!(
        "npm install 실행 중: {} (npm: {})",
        bot_dir.display(),
        npm_path.display()
    );

    let mut cmd = Command::new(&npm_path);
    cmd.args(["install", "--production", "--no-optional"]);
    cmd.current_dir(bot_dir);

    // node.exe가 있는 디렉토리를 PATH에 추가
    if let Some(node_dir) = node_exe.parent() {
        let current_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{};{}", node_dir.to_string_lossy(), current_path);
        cmd.env("PATH", &new_path);
    }

    apply_creation_flags(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("npm install 실행 실패: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "npm install 실패:\nstdout: {}\nstderr: {}",
            stdout, stderr
        ));
    }

    tracing::info!("npm install 완료: {}", bot_dir.display());
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
//  URL 생성
// ═══════════════════════════════════════════════════════════════

fn python_download_url() -> Result<String, String> {
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
        return Err("이 플랫폼에서는 포터블 Python 자동 설치를 지원하지 않습니다".into());
    };

    Ok(format!(
        "https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz",
        tag = PYTHON_RELEASE_TAG,
        ver = PYTHON_VERSION,
        triple = triple,
    ))
}

fn node_download_url() -> Result<String, String> {
    let (os, arch, ext) = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        ("win", "x64", "zip")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        ("win", "arm64", "zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        ("linux", "x64", "tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        ("linux", "arm64", "tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        ("darwin", "x64", "tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        ("darwin", "arm64", "tar.gz")
    } else {
        return Err("이 플랫폼에서는 포터블 Node.js 자동 설치를 지원하지 않습니다".into());
    };

    Ok(format!(
        "https://nodejs.org/dist/v{ver}/node-v{ver}-{os}-{arch}.{ext}",
        ver = NODE_VERSION,
        os = os,
        arch = arch,
        ext = ext,
    ))
}

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
                &dest.to_string_lossy().into_owned(),
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

async fn extract_zip_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("디렉토리 생성 실패: {}", e))?;

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
            archive.to_string_lossy(),
            dest.to_string_lossy()
        );
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        apply_creation_flags(&mut cmd);

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("PowerShell 실행 실패: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("zip 추출 실패: {}", stderr));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("unzip");
        cmd.args([
            "-o",
            &archive.to_string_lossy(),
            "-d",
            &dest.to_string_lossy(),
        ]);
        apply_creation_flags(&mut cmd);

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("unzip 실행 실패: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("zip 추출 실패: {}", stderr));
        }
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

/// npm 경로 찾기: node.exe가 있는 디렉토리에서 npm.cmd (Win) 또는 bin/npm (Unix) 탐색
fn find_npm_path(node_exe: &Path) -> Result<PathBuf, String> {
    let node_dir = node_exe
        .parent()
        .ok_or("node 실행 파일의 부모 디렉토리를 찾을 수 없습니다")?;

    #[cfg(target_os = "windows")]
    {
        // npm.cmd 는 node.exe 와 같은 디렉토리에 있음
        let npm = node_dir.join("npm.cmd");
        if npm.exists() {
            return Ok(npm);
        }
        // 혹시 npm.exe 가 있을 수도 있음
        let npm_exe = node_dir.join("npm.exe");
        if npm_exe.exists() {
            return Ok(npm_exe);
        }
        Err(format!(
            "npm을 찾을 수 없습니다. 탐색 경로: {}",
            node_dir.display()
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Unix: node 가 bin/ 안에 있으면 npm 도 같은 bin/ 에 있음
        let npm = node_dir.join("npm");
        if npm.exists() {
            return Ok(npm);
        }
        Err(format!(
            "npm을 찾을 수 없습니다. 탐색 경로: {}",
            node_dir.display()
        ))
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
            let dir = PathBuf::from(appdata).join("saba-chan");
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
                .join("saba-chan");
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
