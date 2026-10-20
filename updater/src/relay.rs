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
/// 릴레이가 외부 업데이트 요청에 요구하는 헤더
pub const CLIENT_HEADER: (&str, &str) = ("X-Saba-Client", "1");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResponse {
    pub up_to_date: bool,
    #[serde(default)]
    pub latest: Option<LatestRelease>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// manifest 컴포넌트 항목. 필드 이름은 운영 중인 manifest(0.1.1~)를 따른다:
/// `asset`, `size_bytes`, `install_dir`. 예전 초안 이름(`filename` 등)도 읽는다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestComponent {
    pub version: String,
    #[serde(rename = "asset", alias = "filename")]
    pub filename: String,
    #[serde(default, rename = "size_bytes", alias = "sizeBytes", skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub sha256: String,
    /// R2 공개 URL
    pub url: String,
    /// install_root 기준 설치 폴더
    #[serde(default, rename = "install_dir", alias = "installDir", skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    /// 이 컴포넌트가 요구하는 다른 컴포넌트의 최소 버전 (예: `{"saba-core": ">=0.3.0"}`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<HashMap<String, String>>,
    /// 모듈·익스텐션 목록 화면용 정보 (U9e). 모듈: display_name, description, game_name,
    /// requires_extensions. 익스텐션: name, description, author, min_app_version, dependencies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// 모듈 컴포넌트 키 접두사 (`module-<이름>`)
pub const MODULE_PREFIX: &str = "module-";
/// 익스텐션 컴포넌트 키 접두사 (`ext-<id>`)
pub const EXTENSION_PREFIX: &str = "ext-";

impl ReleaseManifest {
    /// 키가 `prefix`로 시작하는 컴포넌트를 `(접두사 뗀 이름, 항목)`으로, 이름 순으로
    pub fn addons(&self, prefix: &str) -> Vec<(&str, &ManifestComponent)> {
        let mut list: Vec<_> = self
            .components
            .iter()
            .filter_map(|(key, c)| key.strip_prefix(prefix).map(|name| (name, c)))
            .collect();
        list.sort_by(|a, b| a.0.cmp(b.0));
        list
    }
}

/// 모듈·익스텐션 목록의 출처: 이 플랫폼 최신 릴리즈의 서명된 manifest (U9e).
/// 데몬·설치기가 같은 함수를 쓴다. 게시된 릴리즈가 없으면 `Ok(None)`.
pub async fn latest_catalog(relay_url: Option<&str>) -> Result<Option<ReleaseManifest>, RelayError> {
    let client = RelayClient::new(relay_url.unwrap_or(DEFAULT_RELAY_URL));
    let release = client
        .fetch_verified_release(DEFAULT_CHANNEL, &current_platform(), "", &anon_client_id())
        .await?;
    Ok(release.map(|r| r.manifest))
}

/// 릴레이는 `current`가 semver가 아니면 400 INVALID_CURRENT_VERSION 이다. 아무것도 안 깔린 상태(설치기, 카탈로그)는 0.0.0
fn current_param(current: &str) -> &str {
    if current.trim().is_empty() { "0.0.0" } else { current }
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
    #[error("manifest is for channel {got}, expected {expected}")]
    WrongChannel { expected: String, got: String },
    #[error("relay offered {advertised} but the signed manifest is {signed}")]
    VersionMismatch { advertised: String, signed: String },
    #[error("manifest component {key} has an unsafe {field}: {value}")]
    UnsafePath { key: String, field: &'static str, value: String },
}

pub struct RelayClient {
    base_url: String,
    http: reqwest::Client,
}

impl RelayClient {
    pub fn new(base_url: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(CLIENT_HEADER.0, reqwest::header::HeaderValue::from_static(CLIENT_HEADER.1));
        let http = reqwest::Client::builder()
            .user_agent(concat!("saba-chan-updater/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
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
            .query(&[("channel", channel), ("platform", platform), ("current", current_param(current)), ("client_id", client_id)])
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
    /// 코어가 최신이면 현재 릴리즈 manifest를 돌려준다 (컴포넌트별 비교용). 받을 manifest가 없으면 `Ok(None)`.
    pub async fn fetch_verified_release(
        &self,
        channel: &str,
        platform: &str,
        current: &str,
        client_id: &str,
    ) -> Result<Option<VerifiedRelease>, RelayError> {
        let check = self.check(channel, platform, current, client_id).await?;
        // 코어가 최신이어도 같은 릴리즈의 다른 컴포넌트(GUI·CLI·모듈...)는 뒤처져 있을 수 있다:
        // 그 릴리즈 manifest를 받아 컴포넌트별로 비교하게 한다
        let latest = match (check.up_to_date, check.latest) {
            (_, Some(latest)) => latest,
            (true, None) if !current.is_empty() => LatestRelease { version: current.to_string(), ..Default::default() },
            _ => return Ok(None),
        };
        let signed = match self.fetch_manifest(&latest).await {
            Ok(signed) => signed,
            // 최신인데 그 버전 manifest가 릴레이에 없으면(옛 릴리즈) 받을 것이 없다
            Err(RelayError::Status(404)) if check.up_to_date => return Ok(None),
            Err(e) => return Err(e),
        };
        let manifest = match verify_release(&signed, platform, channel, current) {
            Ok(manifest) => manifest,
            // 최신일 때는 버전만으로 manifest를 찾아서, 같은 버전이 다른 채널에도 있으면 그쪽 것이 온다.
            // 지금 깔린 그 버전일 때만 컴포넌트 비교용으로 받는다. 다른 채널의 다른 버전은 받지 않는다
            Err(RelayError::WrongChannel { expected, got }) if check.up_to_date => {
                let other = verify_release(&signed, platform, &got, current)?;
                if other.version != current {
                    return Err(RelayError::WrongChannel { expected, got });
                }
                other
            }
            Err(e) => return Err(e),
        };
        bind_release(latest, manifest).map(Some)
    }

    async fn json<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T, RelayError> {
        if !resp.status().is_success() {
            return Err(RelayError::Status(resp.status().as_u16()));
        }
        resp.json::<T>().await.map_err(|e| RelayError::Parse(e.to_string()))
    }
}

/// 서명을 검증하고, 플랫폼·채널이 맞는지, 현재보다 낮은 버전이 아닌지 확인한다.
pub fn verify_release(
    signed: &SignedManifest,
    platform: &str,
    channel: &str,
    current: &str,
) -> Result<ReleaseManifest, RelayError> {
    signature::verify(signed)?;
    let manifest: ReleaseManifest =
        serde_json::from_value(signed.manifest.clone()).map_err(|e| RelayError::Parse(e.to_string()))?;
    if manifest.platform != platform {
        return Err(RelayError::WrongTarget { expected: platform.into(), got: manifest.platform });
    }
    if manifest.channel != channel {
        return Err(RelayError::WrongChannel { expected: channel.into(), got: manifest.channel });
    }
    if let (Some(offered), Some(cur)) = (SemVer::parse(&manifest.version), SemVer::parse(current)) {
        if offered < cur {
            return Err(RelayError::Downgrade { current: current.into(), offered: manifest.version });
        }
    }
    check_manifest_paths(&manifest)?;
    Ok(manifest)
}

/// 컴포넌트 키(모듈·익스텐션 폴더 이름이 된다), 받을 파일 이름, `install_dir`가 폴더 밖을 가리키지 않는지.
/// 서명이 맞아도 확인한다: 이 값들은 그대로 경로에 붙는다
pub fn check_manifest_paths(manifest: &ReleaseManifest) -> Result<(), RelayError> {
    // 경로 한 칸: 영문·숫자·`.`·`_`·`-`만, `.`으로 시작하지 않는다 (`..`, 숨김 파일, 구분자, 드라이브·ADS의 `:`를 막는다)
    let plain = |s: &str| {
        !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    for (key, c) in &manifest.components {
        let unsafe_path = |field, value: &str| RelayError::UnsafePath { key: key.clone(), field, value: value.to_string() };
        // 접두사 뒤 이름이 그대로 `modules/<이름>` 폴더가 되므로 이름만으로도 한 칸이어야 한다
        let name = key.strip_prefix("module-").or_else(|| key.strip_prefix("ext-")).unwrap_or(key);
        if !plain(key) || !plain(name) {
            return Err(unsafe_path("key", key));
        }
        if !plain(&c.filename) {
            return Err(unsafe_path("asset", &c.filename));
        }
        if let Some(dir) = c.install_dir.as_deref() {
            if dir != "." && !dir.split(['/', '\\']).all(plain) {
                return Err(unsafe_path("install_dir", dir));
            }
        }
    }
    Ok(())
}

/// check 응답(`latest`)은 서명되지 않는다. 서명된 manifest와 같은 버전일 때만 받으므로 릴리즈 요약의
/// 버전은 서명된 값이다. 패치노트는 릴레이에 등록하는 값이라 서명 밖에 있다
fn bind_release(latest: LatestRelease, manifest: ReleaseManifest) -> Result<VerifiedRelease, RelayError> {
    if latest.version != manifest.version {
        return Err(RelayError::VersionMismatch { advertised: latest.version, signed: manifest.version });
    }
    Ok(VerifiedRelease { latest, manifest })
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
    #[test]
    fn empty_current_is_sent_as_zero_version() {
        assert_eq!(super::current_param(""), "0.0.0");
        assert_eq!(super::current_param("  "), "0.0.0");
        assert_eq!(super::current_param("0.1.2"), "0.1.2");
    }

    use super::*;
    use serde_json::json;

    /// 운영 릴레이가 실제로 내려준 0.1.1 manifest (서명 봉투 그대로)
    const PRODUCTION_0_1_1: &str = include_str!("../tests/fixtures/relay-manifest-0.1.1.json");

    #[test]
    fn production_manifest_verifies_with_bundled_key() {
        let signed: SignedManifest = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        assert_eq!(signed.key_id, "saba-updater-v1");
        let m = verify_release(&signed, "windows-x64", "stable", "0.1.0").expect("production manifest must verify");
        assert_eq!(m.version, "0.1.1");
        let gui = &m.components["gui"];
        assert_eq!(gui.filename, "saba-chan-gui-windows-x64.zip");
        assert_eq!(gui.size_bytes, Some(128565223));
        assert_eq!(gui.install_dir.as_deref(), Some("."));
        assert_eq!(m.components["locales"].install_dir.as_deref(), Some("locales"));
        assert!(m.components["saba-core"].url.starts_with("https://pub-7193a1c92bf24fd992506560aa6d2a5d.r2.dev/releases/0.1.1/"));
        assert_eq!(m.components.len(), 7);
    }

    #[test]
    fn production_manifest_tampering_and_downgrade_are_rejected() {
        let mut signed: SignedManifest = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        assert!(matches!(verify_release(&signed, "windows-x64", "stable", "0.2.0"), Err(RelayError::Downgrade { .. })));
        assert!(matches!(verify_release(&signed, "linux-x64", "stable", "0.1.0"), Err(RelayError::WrongTarget { .. })));
        signed.manifest["components"]["gui"]["url"] = serde_json::json!("https://evil.example/gui.zip");
        assert!(matches!(
            verify_release(&signed, "windows-x64", "stable", "0.1.0"),
            Err(RelayError::Signature(signature::SignatureError::Invalid))
        ));
    }

    #[test]
    fn a_manifest_for_another_channel_is_rejected() {
        let signed: SignedManifest = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        assert!(verify_release(&signed, "windows-x64", "stable", "0.1.0").is_ok());
        assert!(matches!(
            verify_release(&signed, "windows-x64", "beta", "0.1.0"),
            Err(RelayError::WrongChannel { .. })
        ));
    }

    /// 경로로 응답을 고르는 가짜 릴레이: check와 manifest 두 요청에 답한다
    async fn fake_relay(check: String, manifest: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = if req.contains("/v1/updates/check") { &check } else { &manifest };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn an_up_to_date_client_compares_against_its_version_from_another_channel() {
        // 같은 버전이 stable·beta 둘 다에 있으면 버전만 든 manifest 주소는 다른 채널 것을 줄 수 있다.
        // 그래도 받아서 뒤처진 GUI·모듈을 비교할 수 있어야 한다
        let base = fake_relay(r#"{"upToDate":true}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("beta", "windows-x64", "0.1.1", "t").await;
        assert_eq!(got.unwrap().map(|v| v.manifest.version).as_deref(), Some("0.1.1"));
    }

    // U-11: 최신이라는 답에 다른 버전이 붙어 와도, 다른 채널의 그 버전 manifest는 받지 않는다
    #[tokio::test]
    async fn an_up_to_date_answer_does_not_bring_another_channels_newer_version() {
        let base = fake_relay(r#"{"upToDate":true,"latest":{"version":"0.1.1"}}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("beta", "windows-x64", "0.1.0", "t").await;
        assert!(matches!(got, Err(RelayError::WrongChannel { .. })), "{:?}", got.map(|r| r.map(|v| v.manifest.version)));
    }

    #[test]
    fn the_advertised_version_must_be_the_signed_one() {
        let manifest: ReleaseManifest = serde_json::from_value(manifest("1.0.0", "linux-x64")).unwrap();
        let latest = |version: &str| LatestRelease { version: version.into(), ..Default::default() };
        let bound = bind_release(latest("1.0.0"), manifest.clone()).unwrap();
        assert_eq!(crate::marker::ReleaseSummary::from(&bound.latest).version, "1.0.0");
        assert!(matches!(
            bind_release(latest("2.0.0"), manifest),
            Err(RelayError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn components_serialize_with_production_field_names() {
        let signed: SignedManifest = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        let m: ReleaseManifest = serde_json::from_value(signed.manifest.clone()).unwrap();
        let back = serde_json::to_value(&m.components["cli"]).unwrap();
        assert_eq!(back, signed.manifest["components"]["cli"]);
    }

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
            verify_release(&signed, "linux-x64", "stable", "0.1.0"),
            Err(RelayError::Signature(signature::SignatureError::Invalid))
        ));
    }

    // L16: 키·파일 이름·install_dir가 폴더 밖을 가리키면 서명이 맞아도 manifest를 받지 않는다
    #[test]
    fn manifest_paths_must_stay_inside_their_folders() {
        let with = |key: &str, field: &str, value: &str| {
            let mut v = manifest("1.0.0", "windows-x64");
            let comp = v["components"]["saba-core"].take();
            v["components"] = json!({ key: comp });
            v["components"][key][field] = json!(value);
            serde_json::from_value::<ReleaseManifest>(v).unwrap()
        };
        assert!(check_manifest_paths(&with("saba-core", "install_dir", ".")).is_ok());
        assert!(check_manifest_paths(&with("locales", "install_dir", "locales")).is_ok());
        assert!(check_manifest_paths(&with("module-palworld", "filename", "module-palworld.zip")).is_ok());
        for (key, field, value) in [
            ("module-../evil", "filename", "a.zip"),
            ("module-..", "filename", "a.zip"),
            ("module-.", "filename", "a.zip"),
            ("ext-", "filename", "a.zip"),
            ("ext-a\\b", "filename", "a.zip"),
            ("saba-core", "filename", "../saba-core.zip"),
            ("saba-core", "filename", "C:saba-core.zip"),
            ("saba-core", "install_dir", "../.."),
            ("saba-core", "install_dir", "C:\\Windows"),
            ("saba-core", "install_dir", "/etc"),
        ] {
            assert!(
                matches!(check_manifest_paths(&with(key, field, value)), Err(RelayError::UnsafePath { .. })),
                "{key} {field}={value}"
            );
        }
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
