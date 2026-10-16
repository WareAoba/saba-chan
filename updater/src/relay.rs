//! 릴레이 업데이트 API 클라이언트 (R2 배포 경로)
//!
//! 릴레이(오라클)는 버전·채널·롤아웃과 서명된 manifest를 갖고 있고, 바이너리는
//! Cloudflare R2에 있다. 업데이터는 릴레이에서 "무엇을 받을지"를 묻고, manifest
//! 서명을 검증한 뒤 R2에서 직접 받는다.
//!
//! - `GET  /v1/updates/check?channel&platform&current&client_id`
//! - `GET  /v1/updates/manifest/:version`
//! - `POST /v1/updates/event`
//!
//! manifest의 `components`는 업데이터 컴포넌트 키(`saba-core`, `cli`, `gui`,
//! `updater`, `locales`, `module-<name>`, `ext-<name>`)를 그대로 쓴다.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::signature::{self, SignedManifest};
use crate::version::SemVer;

/// 기본 릴레이 주소
pub const DEFAULT_RELAY_URL: &str = "https://saba-chan.online";
/// 기본 채널
pub const DEFAULT_CHANNEL: &str = "stable";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResponse {
    pub up_to_date: bool,
    #[serde(default)]
    pub latest: Option<LatestRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestRelease {
    pub version: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub is_critical: bool,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub notes: Option<HashMap<String, String>>,
}

/// 서명 검증을 통과한 manifest 본문
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub version: String,
    pub channel: String,
    pub platform: String,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub is_critical: bool,
    pub components: HashMap<String, ManifestComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestComponent {
    pub version: String,
    pub filename: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    pub sha256: String,
    /// R2 공개 URL
    pub url: String,
    /// install_root 기준 설치 폴더
    #[serde(default)]
    pub install_dir: Option<String>,
    /// 이 컴포넌트가 요구하는 다른 컴포넌트의 최소 버전 (예: `{"saba-core": ">=0.3.0"}`)
    #[serde(default)]
    pub requires: Option<HashMap<String, String>>,
}

/// 확인 결과
#[derive(Debug, Clone)]
pub struct VerifiedRelease {
    pub latest: LatestRelease,
    pub manifest: ReleaseManifest,
}

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("relay request failed: {0}")]
    Http(String),
    #[error("relay returned {0}")]
    Status(u16),
    #[error("malformed relay response: {0}")]
    Parse(String),
    #[error(transparent)]
    Signature(#[from] signature::SignatureError),
    #[error("manifest is for {got}, expected {expected}")]
    WrongTarget { expected: String, got: String },
    #[error("refusing downgrade from {current} to {offered}")]
    Downgrade { current: String, offered: String },
}

pub struct RelayClient {
    base_url: String,
    http: reqwest::Client,
}

impl RelayClient {
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("saba-chan-updater/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { base_url: base_url.trim_end_matches('/').to_string(), http }
    }

    pub async fn check(
        &self,
        channel: &str,
        platform: &str,
        current: &str,
        client_id: &str,
    ) -> Result<CheckResponse, RelayError> {
        let url = format!("{}/v1/updates/check", self.base_url);
        let resp = self
            .http
            .get(&url)
            .query(&[("channel", channel), ("platform", platform), ("current", current), ("client_id", client_id)])
            .send()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        Self::json(resp).await
    }

    pub async fn fetch_manifest(&self, latest: &LatestRelease) -> Result<SignedManifest, RelayError> {
        let url = latest
            .manifest_url
            .clone()
            .unwrap_or_else(|| format!("{}/v1/updates/manifest/{}", self.base_url, latest.version));
        let resp = self.http.get(&url).send().await.map_err(|e| RelayError::Http(e.to_string()))?;
        Self::json(resp).await
    }

    /// 익명 진행 이벤트. 실패해도 업데이트를 막지 않는다.
    pub async fn report_event(&self, event: &UpdateEvent) {
        let url = format!("{}/v1/updates/event", self.base_url);
        if let Err(e) = self.http.post(&url).json(event).send().await {
            tracing::debug!("[Relay] event report failed: {}", e);
        }
    }

    /// check → manifest → 서명·대상·다운그레이드 검증까지 한 번에.
    /// 최신이면 `Ok(None)`.
    pub async fn fetch_verified_release(
        &self,
        channel: &str,
        platform: &str,
        current: &str,
        client_id: &str,
    ) -> Result<Option<VerifiedRelease>, RelayError> {
        let check = self.check(channel, platform, current, client_id).await?;
        let latest = match (check.up_to_date, check.latest) {
            (false, Some(latest)) => latest,
            _ => return Ok(None),
        };
        let signed = self.fetch_manifest(&latest).await?;
        let manifest = verify_release(&signed, platform, current)?;
        Ok(Some(VerifiedRelease { latest, manifest }))
    }

    async fn json<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T, RelayError> {
        if !resp.status().is_success() {
            return Err(RelayError::Status(resp.status().as_u16()));
        }
        resp.json::<T>().await.map_err(|e| RelayError::Parse(e.to_string()))
    }
}

/// 서명을 검증하고, 플랫폼이 맞는지, 현재보다 낮은 버전이 아닌지 확인한다.
pub fn verify_release(
    signed: &SignedManifest,
    platform: &str,
    current: &str,
) -> Result<ReleaseManifest, RelayError> {
    signature::verify(signed)?;
    let manifest: ReleaseManifest =
        serde_json::from_value(signed.manifest.clone()).map_err(|e| RelayError::Parse(e.to_string()))?;
    if manifest.platform != platform {
        return Err(RelayError::WrongTarget { expected: platform.into(), got: manifest.platform });
    }
    if let (Some(offered), Some(cur)) = (SemVer::parse(&manifest.version), SemVer::parse(current)) {
        if offered < cur {
            return Err(RelayError::Downgrade { current: current.into(), offered: manifest.version });
        }
    }
    Ok(manifest)
}

/// 릴레이에 보내는 익명 이벤트
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEvent {
    pub anon_id: String,
    /// check | download | install | success | fail
    pub phase: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub error_code: Option<String>,
    pub platform: String,
}

/// 이 빌드의 플랫폼 이름 (`windows-x64`, `linux-x64`, ...)
pub fn current_platform() -> String {
    let os = std::env::consts::OS;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{}-{}", os, arch)
}

/// 롤아웃 계산용 익명 ID. `<data_dir>/.anon_id`에 한 번 만들어 두고 계속 쓴다.
pub fn anon_client_id() -> String {
    let path = crate::constants::resolve_data_dir().join(".anon_id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return id.to_string();
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(version: &str, platform: &str) -> serde_json::Value {
        json!({
            "version": version,
            "channel": "stable",
            "platform": platform,
            "components": {
                "saba-core": {
                    "version": version,
                    "filename": "saba-core.zip",
                    "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                    "url": "https://cdn.example/saba-core.zip"
                }
            }
        })
    }

    #[test]
    fn unsigned_or_foreign_manifest_is_rejected() {
        let (private, _) = signature::generate_key_pair().unwrap();
        // 테스트 키는 TRUSTED_KEYS에 없으므로 번들 키로는 검증되지 않는다
        let signed = signature::sign(manifest("1.0.0", "linux-x64"), "saba-updater-v1", &private).unwrap();
        assert!(matches!(
            verify_release(&signed, "linux-x64", "0.1.0"),
            Err(RelayError::Signature(signature::SignatureError::Invalid))
        ));
    }

    #[test]
    fn current_platform_has_os_and_arch() {
        let p = current_platform();
        assert!(p.contains('-'), "{}", p);
    }

    #[test]
    fn manifest_components_parse() {
        let m: ReleaseManifest = serde_json::from_value(manifest("1.0.0", "windows-x64")).unwrap();
        assert_eq!(m.components["saba-core"].filename, "saba-core.zip");
        assert!(m.components["saba-core"].install_dir.is_none());
    }
}
