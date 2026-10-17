//! # 릴리스 매니페스트 스키마 (Relay 전용)
//!
//! 릴레이 서버가 서명하여 배포하는 매니페스트 구조체 정의.
//! 이전에는 GitHub `manifest.json` 과 호환되도록 설계되었으며, 필드 레이아웃은
//! 기존 데이터 저장소와 테스트와의 호환을 위해 유지된다.
//!
//! ## 스키마 예시
//! ```json
//! {
//!   "version": "0.9.0-test1",
//!   "channel": "beta",
//!   "platform": "windows-x64",
//!   "components": {
//!     "saba-core": {
//!       "version": "0.9.0-test1",
//!       "asset": "saba-core-windows-x64.zip",
//!       "url": "https://pub-xxxx.r2.dev/...",
//!       "sha256": "abc123...",
//!       "size_bytes": 1234567,
//!       "install_dir": "."
//!     }
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 릴리스 매니페스트 — 릴레이가 서명하여 서빙하는 전체 매니페스트.
///
/// 역사적 이유로 타입 이름은 `ReleaseManifest` 를 유지한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    /// 릴리스 버전 (예: "0.9.0").
    ///
    /// 기존 GitHub 스키마는 `release_version` 필드를 사용했지만,
    /// 릴레이 스키마는 `version` 으로 통일되었다. serde alias 로 양쪽 호환.
    #[serde(alias = "release_version")]
    pub version: String,

    /// 업데이트 채널 (`stable` | `beta` | `nightly`).
    #[serde(default)]
    pub channel: Option<String>,

    /// 플랫폼 식별자 (`windows-x64` | `linux-x64` 등).
    #[serde(default)]
    pub platform: Option<String>,

    /// 컴포넌트 맵 (manifest_key → 정보).
    pub components: HashMap<String, ComponentInfo>,
}

/// 매니페스트 내 개별 컴포넌트 정보.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    /// 컴포넌트의 목표 버전 (SemVer).
    pub version: String,

    /// Windows 자산 파일명 (ZIP 등).
    #[serde(default)]
    pub asset: Option<String>,

    /// Linux 자산 파일명. 플랫폼 필드가 설정된 Relay 매니페스트에서는 선택적.
    #[serde(default)]
    pub asset_linux: Option<String>,

    /// **Relay 전용**: 자산 직접 다운로드 URL (R2 공개 URL 등).
    #[serde(default)]
    pub url: Option<String>,

    /// 자산의 SHA-256 (hex 소문자). 다운로드·적용에 필수 (없으면 거부).
    #[serde(default)]
    pub sha256: Option<String>,

    /// 자산 바이트 크기. Relay 매니페스트에서 필수, GitHub 경로에서는 옵션.
    #[serde(default)]
    pub size_bytes: Option<u64>,

    /// 설치 경로 (install_root 기준 상대 경로, 기본 ".").
    #[serde(default)]
    pub install_dir: Option<String>,

    /// 다른 컴포넌트에 대한 최소 버전 요구사항.
    #[serde(default)]
    pub requires: Option<HashMap<String, String>>,

    /// GUI 에 표시할 이름(선택). 익스텐션/모듈의 사용자 표기용.
    #[serde(default)]
    pub display_name: Option<String>,

    /// 설명(선택). GUI 카탈로그에서 사용.
    #[serde(default)]
    pub description: Option<String>,

    /// 아이콘 URL(선택). R2 등 릴레이가 지정한 CDN 을 가리킨다.
    #[serde(default)]
    pub icon_url: Option<String>,

    /// 익스텐션/모듈 간 의존성(선택). key=id, value=SemVer range.
    #[serde(default)]
    pub dependencies: Option<HashMap<String, String>>,

    /// 최소 앱 버전 요구(선택).
    #[serde(default)]
    pub min_app_version: Option<String>,
}

/// 해결된 다운로드 소스.
///
/// `check_for_updates` 가 매니페스트를 파싱한 뒤, 각 컴포넌트에 대해
/// `download_component` 가 사용할 수 있도록 생성한다.
#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    /// 매니페스트가 알리는 최신 버전.
    pub latest_version: String,
    /// 다운로드 소스를 가리키는 태그/라벨 (로그용).
    pub source_release_tag: String,
    /// 실제 다운로드 URL.
    pub download_url: String,
    /// 다운로드 저장 시 사용할 파일명.
    pub asset_name: String,
    /// 설치 경로 (install_root 기준).
    pub install_dir: Option<String>,
    /// 기대 SHA-256 (hex). 없으면 다운로드를 거부한다.
    pub sha256: Option<String>,
    /// 요구사항.
    pub requires: Option<HashMap<String, String>>,
}
