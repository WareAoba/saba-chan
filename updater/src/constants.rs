//! # saba-chan 전역 상수 및 경로 해석
//!
//! 모든 크레이트(saba-core, saba-chan-cli, installer, updater)가 공유하는
//! 단일 진실 원천(Single Source of Truth)입니다.
//!
//! ## 사용법
//! ```rust
//! use saba_chan_updater_lib::constants;
//!
//! let data_dir = constants::resolve_data_dir();
//! let token    = constants::token_file_path();
//! let port     = constants::DEFAULT_IPC_PORT;
//! ```

use std::path::PathBuf;

// ══════════════════════════════════════════════════════
//  앱 식별자
// ══════════════════════════════════════════════════════

/// 앱 이름 — 경로 구성, 레지스트리 등에서 사용
pub const APP_NAME: &str = "saba-chan";

/// 배포자 이름 (설치 레지스트리 Publisher)
pub const PUBLISHER: &str = "WareAoba";

// ══════════════════════════════════════════════════════
//  네트워크 상수
// ══════════════════════════════════════════════════════

/// 기본 IPC 서버 포트
pub const DEFAULT_IPC_PORT: u16 = 57474;

/// 기본 Daemon API URL
pub const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:57474";

// ══════════════════════════════════════════════════════
//  인스턴스 설정 칸
// ══════════════════════════════════════════════════════

/// 모듈이 선언하지 않아도 모든 서버 인스턴스에 있는 설정 칸 `(이름, 형식)`. 데몬 PATCH `/api/instance/:id`가
/// 인스턴스 필드로 받고, CLI `server set`·`server settings`가 같은 표를 쓴다
pub const INSTANCE_FIELDS: &[(&str, &str)] = &[
    ("port", "number"),
    ("rest_host", "text"),
    ("rest_port", "number"),
    ("rest_username", "text"),
    ("rest_password", "password"),
    ("executable_path", "text"),
    ("working_dir", "text"),
    ("protocol_mode", "text"),
    ("server_version", "text"),
    // 문자열이면 데몬이 따옴표를 지켜 나눈다
    ("extra_args", "text"),
    // 메모리 상한(MiB). 비우면 제한 없음 (Windows만 적용)
    ("memory_limit_mb", "number"),
];

/// `INSTANCE_FIELDS` 중 비울 수 없는 칸: 데몬 PATCH가 null을 받아도 지금 값을 둔다.
/// CLI `server set KEY=`(빈 값)는 이 칸을 성공으로 알리지 않고 거절한다
pub const UNCLEARABLE_INSTANCE_FIELDS: &[&str] = &["port", "working_dir", "protocol_mode", "server_version"];

/// 모듈이 선언하지 않아도 데몬이 `module_settings`에서 읽는 설정 `(이름, 형식)`
pub const DAEMON_SETTINGS: &[(&str, &str)] = &[
    ("auto_restart", "boolean"),
    ("restart_on_unknown_exit", "boolean"),
    ("managed_start", "boolean"),
    ("log_sessions_keep", "number"),
    ("stop_timeout", "number"),
];

// ══════════════════════════════════════════════════════
//  지원 언어
// ══════════════════════════════════════════════════════

/// 지원 언어 목록 — UI 언어 선택, 로케일 매칭 등에 사용
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "en", "ko", "ja", "zh-CN", "zh-TW", "es", "pt-BR", "ru", "de", "fr",
];

// ══════════════════════════════════════════════════════
//  경로 해석 — 단일 진실 원천
// ══════════════════════════════════════════════════════

/// 앱 데이터 디렉토리 해석
///
/// - Windows: `%APPDATA%/saba-chan`
/// - Unix:    `$HOME/.config/saba-chan`
/// - Fallback: `./saba-chan`
pub fn resolve_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SABA_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(dir) = test_isolated_root() {
        return dir;
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join(APP_NAME);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join(APP_NAME);
        }
    }
    PathBuf::from(".").join(APP_NAME)
}

/// IPC 인증 토큰 파일 경로
///
/// `SABA_TOKEN_PATH` 환경 변수로 오버라이드 가능.
pub fn token_file_path() -> PathBuf {
    if let Ok(p) = std::env::var("SABA_TOKEN_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    resolve_data_dir().join(".ipc_token")
}

/// 모듈 디렉토리 경로
///
/// `SABA_MODULES_PATH` 환경 변수로 오버라이드 가능.
pub fn resolve_modules_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SABA_MODULES_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    resolve_data_dir().join("modules")
}

/// 익스텐션 디렉토리 경로
///
/// `SABA_EXTENSIONS_DIR` 환경 변수로 오버라이드 가능.
pub fn resolve_extensions_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SABA_EXTENSIONS_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    resolve_data_dir().join("extensions")
}

/// 인스턴스 저장 디렉토리
///
/// `SABA_INSTANCES_PATH` 환경 변수로 오버라이드 가능.
pub fn resolve_instances_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SABA_INSTANCES_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    resolve_data_dir().join("instances")
}

/// 모듈·익스텐션 실행과 pip가 쓰는 임시 폴더 (`TEMP`·`TMP`·`TMPDIR`). 시스템 임시 폴더에 흔적을 남기지 않게
/// 데이터 폴더 안에 두고, 제거기가 늘 지운다 (`uninstall_entry::RUNTIME_ENTRIES`)
pub fn resolve_temp_dir() -> PathBuf {
    resolve_data_dir().join("temp")
}

/// pip 내려받기 캐시 (`PIP_CACHE_DIR`). 사용자 pip 캐시(`%LOCALAPPDATA%\pip`)에 섞지 않는다
pub fn resolve_pip_cache_dir() -> PathBuf {
    resolve_data_dir().join("pip-cache")
}

/// 테스트 빌드에서 사용자 데이터 폴더 대신 쓰는 프로세스별 임시 루트.
///
/// `test-isolation` 기능은 saba-core의 dev-dependency에서만 켜지므로
/// 일반 빌드에는 영향이 없다. 테스트가 실제 `%APPDATA%`/`~/.config`를
/// 건드리지 않게 한다.
#[cfg(any(test, feature = "test-isolation"))]
fn test_isolated_root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    Some(
        ROOT.get_or_init(|| {
            std::env::temp_dir().join(format!("{}-test-{}", APP_NAME, std::process::id()))
        })
        .clone(),
    )
}

#[cfg(not(any(test, feature = "test-isolation")))]
fn test_isolated_root() -> Option<PathBuf> {
    None
}

/// 스테이징 디렉터리 (업데이트 다운로드 임시 파일)
pub fn resolve_staging_dir() -> PathBuf {
    if let Some(dir) = test_isolated_root() {
        return dir.join("updates");
    }
    #[cfg(target_os = "windows")]
    {
        resolve_data_dir().join("updates")
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".cache")
                .join(APP_NAME)
                .join("updates");
        }
        PathBuf::from("./updates")
    }
}

/// 업데이터 상태 파일 경로 (`updater-state.json`)
pub fn resolve_updater_state_path() -> PathBuf {
    resolve_data_dir().join("updater-state.json")
}

/// 업데이트 완료 마커 경로 (`update-complete.json`)
pub fn resolve_update_complete_path() -> PathBuf {
    resolve_data_dir().join("update-complete.json")
}

/// 설치 매니페스트 경로 (`installed-manifest.json`)
pub fn resolve_installed_manifest_path() -> PathBuf {
    resolve_data_dir().join("installed-manifest.json")
}

/// 비밀번호 자동 생성 — 통일된 알고리즘
///
/// `secrets.choice(ascii_letters + digits)` 16자 (Python 모듈과 일치).
/// Rust의 `uuid::Uuid::new_v4()` 바이트를 사용하되 길이를 16자로 통일.
pub fn generate_random_password() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    // OsRng를 사용하면 좋지만, uuid 의존성만으로 구현
    let mut password = String::with_capacity(16);
    // uuid v4 = 122 bits random → 16바이트
    let bytes1 = *uuid::Uuid::new_v4().as_bytes();
    let bytes2 = *uuid::Uuid::new_v4().as_bytes();
    for i in 0..16 {
        let b = if i < 16 { bytes1[i] } else { bytes2[i - 16] };
        let idx = b as usize % CHARSET.len();
        password.push(CHARSET[idx] as char);
    }
    password
}

// ══════════════════════════════════════════════════════
//  로케일 해석
// ══════════════════════════════════════════════════════

/// 시스템 로케일 문자열을 지원 언어로 정규화
///
/// `"ko-KR"` → `"ko"`, `"zh-Hans"` → `"zh-CN"`, `"en-US"` → `"en"` 등
pub fn resolve_locale(locale: &str) -> String {
    let trimmed = locale.trim();

    // 정확한 매칭
    if SUPPORTED_LANGUAGES.contains(&trimmed) {
        return trimmed.to_string();
    }

    // 하이픈/언더스코어 정규화
    let normalized = trimmed.replace('_', "-");
    if SUPPORTED_LANGUAGES.contains(&normalized.as_str()) {
        return normalized;
    }

    // zh-Hans / zh-Hant 특수 처리
    let lower = normalized.to_lowercase();
    if lower.starts_with("zh-cn") || lower.starts_with("zh-hans") {
        return "zh-CN".to_string();
    }
    if lower.starts_with("zh-tw") || lower.starts_with("zh-hant") {
        return "zh-TW".to_string();
    }
    if lower.starts_with("pt-br") {
        return "pt-BR".to_string();
    }

    // 기본 언어 코드로 매칭
    let base = trimmed
        .split(&['-', '_', '.'][..])
        .next()
        .unwrap_or("en");
    SUPPORTED_LANGUAGES
        .iter()
        .find(|&&s| s == base || s.starts_with(&format!("{}-", base)))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_data_dir_returns_path() {
        let dir = resolve_data_dir();
        assert!(dir.to_string_lossy().contains(APP_NAME));
    }

    #[test]
    fn test_token_file_path_under_data_dir() {
        let token = token_file_path();
        assert!(token.to_string_lossy().contains(".ipc_token"));
    }

    #[test]
    fn test_resolve_locale_exact() {
        assert_eq!(resolve_locale("ko"), "ko");
        assert_eq!(resolve_locale("zh-CN"), "zh-CN");
        assert_eq!(resolve_locale("pt-BR"), "pt-BR");
    }

    #[test]
    fn test_resolve_locale_normalize() {
        assert_eq!(resolve_locale("ko-KR"), "ko");
        assert_eq!(resolve_locale("en-US"), "en");
        assert_eq!(resolve_locale("zh-Hans"), "zh-CN");
        assert_eq!(resolve_locale("zh-Hant"), "zh-TW");
        assert_eq!(resolve_locale("pt_BR"), "pt-BR");
    }

    #[test]
    fn test_resolve_locale_fallback() {
        assert_eq!(resolve_locale("xyz"), "en");
    }

    #[test]
    fn test_generate_random_password_length() {
        let pw = generate_random_password();
        assert_eq!(pw.len(), 16);
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_generate_random_password_uniqueness() {
        let pw1 = generate_random_password();
        let pw2 = generate_random_password();
        assert_ne!(pw1, pw2);
    }

    #[test]
    fn test_supported_languages_count() {
        assert_eq!(SUPPORTED_LANGUAGES.len(), 10);
    }
}
