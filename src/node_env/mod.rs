//! Node.js 포터블 환경 매니저 — 엔드유저 무설치 Node.js 제공
//!
//! saba-chan의 Discord Bot은 Node.js 런타임이 필요합니다.
//! 엔드유저가 별도로 Node.js를 설치할 필요 없이, 포터블 Node.js를
//! 자동으로 다운로드하여 사용할 수 있도록 합니다.
//!
//! ## 부트스트랩 흐름
//! 1. `get_node_path()` 호출 → `OnceCell` 캐싱 (최초 1회만)
//! 2. 기존 포터블 Node.js 유효? → 바로 반환
//! 3. 시스템 Node.js ≥ 18.0 탐색 → 사용
//! 4. 둘 다 없음 → nodejs.org에서 포터블 Node.js 자동 다운로드 (~30 MB)
//!
//! ## 데이터 디렉토리 레이아웃
//! ```text
//! <data_dir>/
//!   node-portable/           ← 포터블 Node.js (자동 다운로드)
//!     node-v22.14.0-win-x64/
//!       node.exe
//!       npm.cmd
//!       ...
//! ```
//!
//! ## 새 Cargo 의존성: 없음
//! 다운로드는 OS 내장 도구(PowerShell / curl), 추출은 `tar` / PowerShell 사용.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::utils::apply_creation_flags;

// ── Node.js Build Info ──────────────────────────────────────────
// 공식 nodejs.org 배포: https://nodejs.org/dist/
// LTS 22.x 시리즈 사용 (2026-04 기준 Active LTS)

const NODE_VERSION: &str = "22.14.0";

/// 시스템 Node.js 사용 시 최소 요구 버전
const MIN_NODE_VERSION: (u32, u32) = (18, 0);

const PORTABLE_DIR_NAME: &str = "node-portable";

// ── Cached node path ────────────────────────────────────────────

#[allow(dead_code)]
static NODE_EXE: OnceCell<PathBuf> = OnceCell::const_new();

/// 관리 Node.js 실행 파일 경로를 반환합니다.
/// 최초 호출 시 전체 부트스트랩(다운로드 포함)을 자동 수행합니다.
#[allow(dead_code)]
pub async fn get_node_path() -> Result<PathBuf> {
    NODE_EXE
        .get_or_try_init(find_or_bootstrap)
        .await
        .cloned()
}

// ═══════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════

/// Node.js가 존재하고 유효한지 확인합니다. 없으면 부트스트랩합니다.
/// 반환값: node 실행 파일의 절대 경로
pub async fn find_or_bootstrap() -> Result<PathBuf> {
    let data_dir = resolve_data_dir()?;

    // ── Fast path: 기존 포터블 Node.js 유효 ──
    let portable_exe = find_portable_node_exe(&data_dir);
    if let Some(exe) = &portable_exe {
        if exe.exists() && verify_node(exe).await {
            tracing::debug!("포터블 Node.js 확인 완료: {}", exe.display());
            return Ok(exe.clone());
        }
    }

    // ── 시스템 Node.js ──
    if let Ok(cmd) = detect_system_node().await {
        let path = PathBuf::from(&cmd);
        tracing::info!("시스템 Node.js 사용: {}", cmd);
        return Ok(path);
    }

    // ── 자동 다운로드 ──
    tracing::info!(
        "Node.js를 찾을 수 없습니다. 포터블 Node.js v{}을(를) 다운로드합니다...",
        NODE_VERSION
    );
    let exe = download_portable_node(&data_dir).await?;
    Ok(exe)
}

/// 시스템에서 Node.js ≥ 18.0 을 탐지합니다.
pub async fn detect_system_node() -> Result<String> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["node", "node.exe"]
    } else {
        vec!["node", "nodejs"]
    };

    for cmd_name in candidates {
        let mut cmd = Command::new(cmd_name);
        cmd.arg("--version");
        apply_creation_flags(&mut cmd);

        if let Ok(output) = cmd.output().await {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout);
                if let Some((major, minor)) = parse_node_version(&ver) {
                    if (major, minor) >= MIN_NODE_VERSION {
                        tracing::info!("시스템 Node.js 발견: {} → {}", cmd_name, ver.trim());

                        // which/where 로 절대 경로 얻기
                        if let Ok(abs) = resolve_absolute_path(cmd_name).await {
                            return Ok(abs);
                        }
                        return Ok(cmd_name.to_string());
                    }
                    tracing::debug!(
                        "{} → {}.{} (최소 {}.{} 필요, 건너뜀)",
                        cmd_name, major, minor,
                        MIN_NODE_VERSION.0, MIN_NODE_VERSION.1
                    );
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "시스템에 Node.js >= {}.{} 없음",
        MIN_NODE_VERSION.0,
        MIN_NODE_VERSION.1
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
            });
        }
    };

    let portable_exe = find_portable_node_exe(&data_dir);
    let portable_ok = if let Some(ref exe) = portable_exe {
        exe.exists() && verify_node(exe).await
    } else {
        false
    };

    let mut info = serde_json::json!({
        "available": portable_ok || detect_system_node().await.is_ok(),
        "portable_installed": portable_ok,
        "portable_path": portable_exe.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "node_version_bundled": NODE_VERSION,
    });

    // 포터블 노드 버전
    if portable_ok {
        if let Some(ref exe) = portable_exe {
            if let Ok(ver) = get_version(exe).await {
                info["node_version"] = serde_json::json!(ver);
            }
        }
    }

    // 다운로드 URL
    if let Ok(url) = node_download_url() {
        info["portable_download_url"] = serde_json::json!(url);
    }

    // 시스템 Node.js 탐지
    match detect_system_node().await {
        Ok(cmd) => {
            info["system_node"] = serde_json::json!(cmd);
        }
        Err(_) => {
            info["system_node"] = serde_json::json!(null);
        }
    }

    info
}

// ═══════════════════════════════════════════════════════════════
//  Internal: Node.js 탐색 & 포터블 다운로드
// ═══════════════════════════════════════════════════════════════

/// nodejs.org 다운로드 URL 생성
fn node_download_url() -> Result<String> {
    // 환경변수 오버라이드 (미러, 사내망 등)
    if let Ok(custom_url) = std::env::var("SABA_NODE_URL") {
        return Ok(custom_url);
    }

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
        return Err(anyhow::anyhow!(
            "이 플랫폼에서는 포터블 Node.js 자동 설치를 지원하지 않습니다. \
             Node.js {}.{} 이상을 수동으로 설치해 주세요.",
            MIN_NODE_VERSION.0,
            MIN_NODE_VERSION.1
        ));
    };

    Ok(format!(
        "https://nodejs.org/dist/v{ver}/node-v{ver}-{os}-{arch}.{ext}",
        ver = NODE_VERSION,
        os = os,
        arch = arch,
        ext = ext,
    ))
}

/// 포터블 Node.js 다운로드 → 추출 → 검증
async fn download_portable_node(data_dir: &Path) -> Result<PathBuf> {
    let url = node_download_url()?;
    let portable_dir = data_dir.join(PORTABLE_DIR_NAME);

    let is_zip = url.ends_with(".zip");
    let tmp_ext = if is_zip { "zip" } else { "tar.gz" };
    let tmp_archive = data_dir.join(format!("_node_download.{}", tmp_ext));

    // 기존 실패분 정리
    if portable_dir.exists() {
        let _ = std::fs::remove_dir_all(&portable_dir);
    }

    // ── 다운로드 ──
    tracing::info!("포터블 Node.js 다운로드 중: {}", url);
    download_file(&url, &tmp_archive)
        .await
        .context(
            "Node.js 다운로드 실패. \
             인터넷 연결을 확인하거나, Node.js 18 이상을 수동 설치해 주세요.",
        )?;

    // ── 추출 ──
    tracing::info!("포터블 Node.js 추출 중...");
    std::fs::create_dir_all(&portable_dir)?;

    if is_zip {
        extract_zip(&tmp_archive, &portable_dir).await?;
    } else {
        extract_tar_gz(&tmp_archive, &portable_dir).await?;
    }

    // 임시 아카이브 삭제
    let _ = std::fs::remove_file(&tmp_archive);

    // ── 실행 파일 찾기 ──
    let exe = find_portable_node_exe(data_dir)
        .ok_or_else(|| anyhow::anyhow!(
            "포터블 Node.js 추출 후 실행 파일을 찾을 수 없습니다. \
             디렉토리: {}",
            portable_dir.display()
        ))?;

    // ── 실행 권한 (Unix) ──
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if exe.exists() {
            let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
        }
    }

    // ── 검증 ──
    if !verify_node(&exe).await {
        return Err(anyhow::anyhow!(
            "다운로드된 포터블 Node.js가 정상 동작하지 않습니다: {}",
            exe.display()
        ));
    }

    tracing::info!(
        "포터블 Node.js 설치 완료: {} ({:.1} MB)",
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

    // Node.js 배포판은 최소 수 MB
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

/// .tar.gz 추출 — tar 명령 사용 (Linux/macOS/Win10+)
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

/// .zip 추출 — Windows PowerShell Expand-Archive
async fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;

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

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("zip 추출 실패: {}", stderr));
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

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("zip 추출 실패: {}", stderr));
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════
//  Internal: 경로 & 유틸리티
// ═══════════════════════════════════════════════════════════════

/// 사바쨩 데이터 디렉토리 결정 (python_env와 동일한 로직)
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

/// 포터블 Node.js 실행 파일을 찾습니다.
///
/// nodejs.org 배포판 추출 시 `node-v22.14.0-win-x64/` 같은 하위 디렉토리가 생기므로,
/// `<portable_dir>/node-v*-*/` 를 glob하여 node.exe 또는 bin/node를 찾습니다.
fn find_portable_node_exe(data_dir: &Path) -> Option<PathBuf> {
    let portable_dir = data_dir.join(PORTABLE_DIR_NAME);
    if !portable_dir.exists() {
        return None;
    }

    // 1) 직접 존재 확인 (단일 파일 배포 등)
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

/// 명령어의 절대 경로를 얻습니다 (Windows: where, Unix: which)
async fn resolve_absolute_path(cmd_name: &str) -> Result<String> {
    #[cfg(target_os = "windows")]
    let (tool, args) = ("where", vec![cmd_name]);
    #[cfg(not(target_os = "windows"))]
    let (tool, args) = ("which", vec![cmd_name]);

    let mut cmd = Command::new(tool);
    for a in &args {
        cmd.arg(a);
    }
    apply_creation_flags(&mut cmd);

    let output = cmd.output().await?;
    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout);
        // where는 여러 줄 반환 가능 → 첫 줄
        if let Some(first) = out.lines().next() {
            let p = first.trim();
            if !p.is_empty() {
                return Ok(p.to_string());
            }
        }
    }

    Err(anyhow::anyhow!("'{}' 절대경로 해석 실패", cmd_name))
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

/// Node.js 실행 파일이 정상 동작하는지 확인
async fn verify_node(exe: &Path) -> bool {
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

/// node --version 문자열 반환
async fn get_version(exe: &Path) -> Result<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    apply_creation_flags(&mut cmd);
    let output = cmd.output().await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// "v22.14.0" → (22, 14)
fn parse_node_version(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let ver_part = s.strip_prefix('v').unwrap_or(s);
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
    fn test_parse_node_version() {
        assert_eq!(parse_node_version("v22.14.0"), Some((22, 14)));
        assert_eq!(parse_node_version("v18.0.0"), Some((18, 0)));
        assert_eq!(parse_node_version("v16.20.2"), Some((16, 20)));
        assert_eq!(parse_node_version("  v20.11.1  "), Some((20, 11)));
        assert_eq!(parse_node_version("garbage"), None);
        assert_eq!(parse_node_version(""), None);
    }

    #[test]
    fn test_node_download_url_format() {
        let url = node_download_url().unwrap();
        assert!(url.contains("nodejs.org"));
        assert!(url.contains(NODE_VERSION));
        #[cfg(target_os = "windows")]
        assert!(url.ends_with(".zip"));
        #[cfg(not(target_os = "windows"))]
        assert!(url.ends_with(".tar.gz"));
    }

    #[test]
    fn test_resolve_data_dir_no_panic() {
        let _ = resolve_data_dir();
    }

    #[tokio::test]
    async fn test_detect_system_node() {
        match detect_system_node().await {
            Ok(cmd) => println!("시스템 Node.js: {}", cmd),
            Err(e) => println!("시스템 Node.js 없음 (정상): {}", e),
        }
    }

    #[tokio::test]
    async fn test_status_returns_json() {
        let s = status().await;
        assert!(s.get("available").is_some());
        assert!(s.get("portable_installed").is_some());
        assert!(s.get("node_version_bundled").is_some());
    }
}
