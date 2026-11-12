//! 릴레이 업데이트 API 클라이언트 (R2 배포 경로)
//!
//! 릴레이(오라클)는 버전·채널·롤아웃과 서명된 manifest를 갖고 있고, 바이너리는
//! Cloudflare R2에 있다. 업데이터는 릴레이에서 "무엇을 받을지"를 묻고, manifest
//! 서명을 검증한 뒤 R2에서 직접 받는다.
//!
//! - `GET  /v1/updates/latest?channel&platform&current` — 캐시되는 확인. 롤아웃은 클라이언트가 판정한다 (`in_rollout`)
//! - `GET  /v1/updates/check?channel&platform&current&client_id` — 옛 릴레이(`latest`가 404·400)일 때만
//! - `GET  /v1/updates/manifest/:version?release=<id>` — 받은 서명 원문은 `<staging>/manifests/<id>.json`에 둔다
//!
//! manifest의 `components`는 업데이터 컴포넌트 키(`saba-core`, `cli`, `gui`,
//! `updater`, `locales`, `module-<name>`, `ext-<name>`)를 그대로 쓴다.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::profile::Profile;
use crate::signature::{self, SignedManifest};
use crate::version::SemVer;

/// 기본 릴레이 주소
pub const DEFAULT_RELAY_URL: &str = "https://saba-chan.online";
/// 기본 채널
pub const DEFAULT_CHANNEL: &str = "stable";
/// 채널은 안정한 순서로 쌓인다: 정식(stable) ⊂ 미리보기(rc) ⊂ 베타(beta) ⊂ nightly.
/// 위 채널을 고른 클라이언트는 아래 채널 릴리즈도 받는다 (릴레이 `routes/updates.ts` `CHANNEL_ORDER`와 같다)
pub const CHANNELS: [&str; 4] = ["stable", "rc", "beta", "nightly"];
/// 사용자가 고를 수 있는 채널 (앱 설정, 설치할 때의 채널 고르기)
pub const SELECTABLE_CHANNELS: [&str; 3] = ["stable", "rc", "beta"];

/// `selected` 채널을 고른 클라이언트가 `release` 채널의 릴리즈를 받는가
pub fn channel_includes(selected: &str, release: &str) -> bool {
    let rank = |c: &str| CHANNELS.iter().position(|&x| x == c);
    match (rank(selected), rank(release)) {
        (Some(s), Some(r)) => r <= s,
        _ => selected == release,
    }
}
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
    /// 릴레이 릴리즈 행 id (`/latest`만 싣는다). manifest 로컬 보관의 이름
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// 점진 배포 비율 (`/latest`만 싣는다. 없으면 이미 릴레이가 고른 것: 100으로 본다)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout_percent: Option<i64>,
}

/// `GET /v1/updates/latest` 응답: 지금보다 높은 이 플랫폼 후보(SemVer 내림차순, 롤아웃 0% 포함)와 지금 판의 행
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LatestResponse {
    #[serde(default)]
    pub releases: Vec<LatestRelease>,
    #[serde(default)]
    pub installed: Option<LatestRelease>,
}

/// 점진 배포 판정. 릴레이 `isInRollout`과 같은 식: `sha256("{client_id}:{version}")` 앞 4바이트(big-endian) `% 100 < percent`
pub fn in_rollout(client_id: &str, version: &str, percent: i64) -> bool {
    use sha2::{Digest, Sha256};
    if percent >= 100 {
        return true;
    }
    if percent <= 0 {
        return false;
    }
    let hash = Sha256::digest(format!("{client_id}:{version}").as_bytes());
    let bucket = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 100;
    i64::from(bucket) < percent
}

/// 같은 판 베타 규칙이 적용되는가: 정식 채널인데 지금 판이 프리릴리스(`X.Y.Z-…`)면 기다리는 정식 판 `X.Y.Z`
pub fn stable_wait_base(channel: &str, current: &str) -> Option<String> {
    let v = SemVer::parse(current)?;
    (channel == DEFAULT_CHANNEL && v.is_prerelease()).then(|| format!("{}.{}.{}", v.major, v.minor, v.patch))
}

/// 버전의 프리릴리스 앞부분(`0.1.14-beta.2` → `0.1.14`)이 `base`와 정확히 같은 프리릴리스인가.
/// 크기 비교로 대신하지 않는다: 그러면 `0.1.15-beta.1`도 받아 버린다
fn same_base_prerelease(version: &str, base: &str) -> bool {
    SemVer::parse(version).is_some_and(|v| v.is_prerelease() && format!("{}.{}.{}", v.major, v.minor, v.patch) == base)
}

/// 후보 중 받을 릴리즈: 고른 채널이 받는 채널이거나(같은 판 베타 규칙이면 그 판의 프리릴리스도), 지금보다 높고,
/// 이 클라이언트가 점진 배포에 드는 것 가운데 가장 높은 것
pub fn select_release(channel: &str, current: &str, client_id: &str, candidates: Vec<LatestRelease>) -> Option<LatestRelease> {
    let wait = stable_wait_base(channel, current);
    let now = SemVer::parse(current);
    let mut accepted: Vec<(SemVer, LatestRelease)> = candidates
        .into_iter()
        .filter_map(|r| {
            let v = SemVer::parse(&r.version)?;
            let takes = r.channel.as_deref().is_none_or(|c| channel_includes(channel, c))
                || wait.as_deref().is_some_and(|base| same_base_prerelease(&r.version, base));
            let newer = now.as_ref().is_none_or(|n| v > *n);
            (takes && newer && in_rollout(client_id, &r.version, r.rollout_percent.unwrap_or(100))).then_some((v, r))
        })
        .collect();
    accepted.sort_by(|a, b| b.0.cmp(&a.0));
    accepted.into_iter().next().map(|(_, r)| r)
}

/// 받은 서명 manifest 원문을 남겨 두는 수 (릴리즈 id가 큰 것부터)
pub const MANIFEST_CACHE_KEEP: usize = 5;

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
    /// `platform` 밖의 플랫폼 몫 (D3c). 키는 플랫폼 이름, 값은 그 플랫폼의 컴포넌트 목록 전체다.
    /// 0.1.10까지의 업데이터는 이 칸을 모르고 맨 위 `components`만 읽는다
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub platforms: HashMap<String, PlatformComponents>,
}

/// manifest `platforms.<플랫폼>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformComponents {
    pub components: HashMap<String, ManifestComponent>,
}

/// manifest 컴포넌트 항목. 필드 이름은 운영 중인 manifest(0.1.1~)를 따른다:
/// `asset`, `size_bytes`, `install_dir`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestComponent {
    pub version: String,
    #[serde(rename = "asset")]
    pub filename: String,
    #[serde(default, rename = "size_bytes", skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub sha256: String,
    /// R2 공개 URL
    pub url: String,
    /// install_root 기준 설치 폴더
    #[serde(default, rename = "install_dir", skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    /// 이 컴포넌트가 요구하는 다른 컴포넌트의 최소 버전 (예: `{"saba-core": ">=0.3.0"}`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<HashMap<String, String>>,
    /// 모듈·익스텐션 목록 화면용 정보 (U9e). 모듈: display_name, description, game_name,
    /// requires_extensions. 익스텐션: name, description, author, min_app_version, dependencies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    /// `saba-core`만: 이 코어가 다시 붙을 수 있는 러너 형식 범위 `[min, max]` (양 끝 포함). 없으면 `[0, 1]`로 본다 (`runner_formats`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads_runner_formats: Option<[u32; 2]>,
}

/// manifest에 `reads_runner_formats`가 없는 코어(형식 번호 전의 판)가 읽는다고 보는 러너 형식
pub const DEFAULT_READS_RUNNER_FORMATS: (u32, u32) = (0, 1);

impl ManifestComponent {
    /// 이 코어가 다시 붙을 수 있는 러너 형식 범위. 적지 않았으면 `DEFAULT_READS_RUNNER_FORMATS`
    pub fn runner_formats(&self) -> (u32, u32) {
        self.reads_runner_formats.map_or(DEFAULT_READS_RUNNER_FORMATS, |[min, max]| (min, max))
    }
}

/// 모듈 컴포넌트 키 접두사 (`module-<이름>`)
pub const MODULE_PREFIX: &str = "module-";
/// 익스텐션 컴포넌트 키 접두사 (`ext-<id>`)
pub const EXTENSION_PREFIX: &str = "ext-";

impl ReleaseManifest {
    /// Windows "설치된 앱" 목록에 보일 버전: 데몬(saba-core) 버전. 데몬이 없는 manifest면 릴리즈 버전
    pub fn display_version(&self) -> &str {
        self.components.get("saba-core").map_or(self.version.as_str(), |c| c.version.as_str())
    }

    /// 이 플랫폼의 manifest: 맨 위 `platform`이면 그대로, `platforms`에 있으면 그 목록으로 바꾼다
    pub fn for_platform(mut self, platform: &str) -> Result<Self, RelayError> {
        if self.platform != platform {
            let Some(own) = self.platforms.remove(platform) else {
                return Err(RelayError::WrongTarget { expected: platform.into(), got: self.platform });
            };
            self.platform = platform.into();
            self.components = own.components;
        }
        self.platforms.clear();
        Ok(self)
    }

    /// 이 설치 구성의 manifest: 구성마다 따로 빌드한 컴포넌트(`<키>.<구성>`, 예: GTK·WebKit 없는 리눅스 서버가 받는
    /// 창 없는 `installer.headless`)는 이 구성의 것이 `<키>`를 대신하고, 다른 구성의 것은 빠진다.
    /// 모듈·익스텐션 이름의 `.`은 구성 표시가 아니다. 0.1.11까지의 클라이언트는 이 키를 모르는 키로 건너뛴다
    pub fn for_profile(mut self, profile: Profile) -> Self {
        let variants: Vec<String> = self
            .components
            .keys()
            .filter(|k| !k.starts_with(MODULE_PREFIX) && !k.starts_with(EXTENSION_PREFIX) && k.contains('.'))
            .cloned()
            .collect();
        for key in variants {
            let Some(entry) = self.components.remove(&key) else { continue };
            if let Some((base, p)) = key.split_once('.') {
                if p == profile.as_str() {
                    self.components.insert(base.to_string(), entry);
                }
            }
        }
        self
    }

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

/// 모듈·익스텐션 목록의 출처: 이 플랫폼, 고른 업데이트 채널의 최신 릴리즈 서명 manifest (U9e).
/// 데몬이 쓴다. 게시된 릴리즈가 없으면 `Ok(None)`.
pub async fn latest_catalog(relay_url: Option<&str>, channel: &str) -> Result<Option<ReleaseManifest>, RelayError> {
    let client = RelayClient::new(relay_url.unwrap_or(DEFAULT_RELAY_URL));
    let release = client
        .fetch_verified_catalog(channel, &current_platform(), &crate::versions::bundled().release, &anon_client_id())
        .await?;
    // 모듈·익스텐션은 이 코어로 돌아가는 판 목록에서 (가장 새 릴리즈의 판은 더 새 코어를 요구할 수 있다).
    // 목록 API가 없는 릴레이면 릴리즈 manifest의 판을 그대로 두고, 목록을 읽지 못하면 내린 판일 수 있어 모듈·익스텐션을 내지 않는다
    let Some(release) = release else { return Ok(None) };
    let catalog = client.fetch_addon_catalog(&core_version(), channel).await;
    Ok(merge_addon_catalog(Some(release), catalog, &core_version(), channel, &current_platform()).map(|r| r.manifest))
}

/// 이 빌드의 코어(데몬) 버전 — 모듈·익스텐션 목록을 고르는 기준
pub fn core_version() -> String {
    crate::versions::component("saba-core").unwrap_or(&crate::versions::bundled().release).to_string()
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

/// 업데이트 확인의 앱 릴리즈와 모듈·익스텐션 판 목록을 합친다.
/// - 목록이 있으면 그 판으로 바꾼다(내린 판은 뺀다). 앱 릴리즈가 없어도 목록만으로 확인한다
/// - 목록 API가 없거나 올린 판이 없으면(`Ok(None)`) 릴리즈 manifest 그대로
/// - 목록을 읽지 못하면 이번 확인에서는 모듈·익스텐션을 내지 않는다: 릴리즈 manifest의 판은 그 뒤 내린 판일 수 있다
pub fn merge_addon_catalog(
    release: Option<VerifiedRelease>,
    catalog: Result<Option<AddonCatalog>, RelayError>,
    core: &str,
    channel: &str,
    platform: &str,
) -> Option<VerifiedRelease> {
    match (release, catalog) {
        (Some(mut release), Ok(Some(catalog))) => {
            catalog.overlay(&mut release.manifest);
            Some(release)
        }
        (None, Ok(Some(catalog))) => Some(VerifiedRelease::addons_only(core, channel, platform, &catalog)),
        (release, Ok(None)) => release,
        (release, Err(e)) => {
            tracing::warn!("[Updater] Addon catalog unavailable; not offering module or extension updates this time: {}", e);
            release.map(|mut r| {
                r.manifest.components.retain(|key, _| !key.starts_with(MODULE_PREFIX) && !key.starts_with(EXTENSION_PREFIX));
                r
            })
        }
    }
}

impl VerifiedRelease {
    /// 앱 릴리즈 없이 모듈·익스텐션 목록만 있을 때의 확인 결과 (앱 컴포넌트는 비교하지 않는다)
    pub fn addons_only(core: &str, channel: &str, platform: &str, catalog: &AddonCatalog) -> Self {
        Self {
            latest: LatestRelease { version: core.to_string(), ..Default::default() },
            manifest: ReleaseManifest {
                version: core.to_string(),
                channel: channel.to_string(),
                platform: platform.to_string(),
                released_at: None,
                is_critical: false,
                components: catalog.components.clone(),
                platforms: HashMap::new(),
            },
        }
    }
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
    /// 받은 서명 manifest 원문을 두는 폴더 (`<staging>/manifests`). 없으면 매번 받는다
    manifest_cache: Option<std::path::PathBuf>,
    /// heartbeat가 알린 릴리즈 id. `/latest` 주소에 `r`로 붙여 캐시 키를 바꾼다 (릴레이는 읽지 않는다)
    release_hint: Option<u64>,
    /// `/latest` 응답에 담긴 가장 큰 릴리즈 id (후보·지금 판). 0이면 없음
    latest_seen_id: std::sync::atomic::AtomicU64,
}

impl RelayClient {
    pub fn new(base_url: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(CLIENT_HEADER.0, reqwest::header::HeaderValue::from_static(CLIENT_HEADER.1));
        let http = crate::tls::http_client_builder()
            .user_agent(concat!("saba-chan-updater/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
            manifest_cache: None,
            release_hint: None,
            latest_seen_id: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// heartbeat가 알린 릴리즈 id를 `/latest` 캐시 키로 쓴다: 새 id면 캐시된 옛 응답이 아니라 새 응답을 받는다
    pub fn with_release_hint(mut self, id: Option<u64>) -> Self {
        self.release_hint = id;
        self
    }

    /// 지금까지 받은 `/latest` 응답에 담긴 가장 큰 릴리즈 id
    pub fn latest_seen_id(&self) -> Option<u64> {
        Some(self.latest_seen_id.load(std::sync::atomic::Ordering::SeqCst)).filter(|id| *id > 0)
    }

    /// 받은 서명 manifest 원문을 `dir/<릴리즈 id>.json`에 두고, 같은 id는 다시 받지 않는다 (서명 검증은 매번 한다)
    pub fn with_manifest_cache(mut self, dir: std::path::PathBuf) -> Self {
        self.manifest_cache = Some(dir);
        self
    }

    /// 캐시되는 확인 (`/v1/updates/latest`). 이 주소가 없는 옛 릴레이(404·400)면 `Ok(None)`: 옛 `check`로 물러선다
    pub async fn latest(&self, channel: &str, platform: &str, current: &str) -> Result<Option<LatestResponse>, RelayError> {
        let url = format!("{}/v1/updates/latest", self.base_url);
        let mut request = self.http.get(&url).query(&[("channel", channel), ("platform", platform), ("current", current_param(current))]);
        if let Some(id) = self.release_hint {
            request = request.query(&[("r", id)]);
        }
        let resp = request.send().await.map_err(|e| RelayError::Http(e.to_string()))?;
        if matches!(resp.status().as_u16(), 400 | 404) {
            return Ok(None);
        }
        let found: LatestResponse = Self::json(resp).await?;
        if let Some(max) = found.releases.iter().chain(found.installed.as_ref()).filter_map(|r| r.id).max() {
            self.latest_seen_id.fetch_max(max, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(Some(found))
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

    /// manifest를 받아 `check`(서명·대상·판 검증)를 통과한 것만 `<id>.json`으로 보관한다. 같은 id는 보관본을 쓰되 매번 검증하고,
    /// 보관본이 검증에 실패하면 지우고 한 번 다시 받는다 (릴레이를 바꿨거나 깨진 보관본이 계속 남지 않게)
    async fn fetch_checked<T>(&self, latest: &LatestRelease, check: impl Fn(&SignedManifest) -> Result<T, RelayError>) -> Result<T, RelayError> {
        let cached = self.manifest_cache.as_ref().zip(latest.id).map(|(dir, id)| dir.join(format!("{id}.json")));
        if let Some(path) = cached.as_ref() {
            if let Some(signed) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<SignedManifest>(&t).ok()) {
                match check(&signed) {
                    Ok(checked) => return Ok(checked),
                    Err(e) => tracing::warn!("[Updater] Kept manifest {} failed verification ({}); fetching it again", path.display(), e),
                }
            }
            let _ = std::fs::remove_file(path);
        }
        let url = self.manifest_url(latest);
        let resp = self.http.get(&url).send().await.map_err(|e| RelayError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RelayError::Status(resp.status().as_u16()));
        }
        let text = resp.text().await.map_err(|e| RelayError::Parse(e.to_string()))?;
        let signed: SignedManifest = serde_json::from_str(&text).map_err(|e| RelayError::Parse(e.to_string()))?;
        let checked = check(&signed)?;
        // 검증을 통과한 원문만 그대로 둔다 (쓰지 못해도 확인은 이어 간다: 다음에 다시 받을 뿐이다)
        if let Some(path) = cached {
            let kept = path.parent().map(|dir| std::fs::create_dir_all(dir).and_then(|_| crate::write_atomic(&path, text.as_bytes())));
            match kept {
                Some(Ok(())) => prune_manifest_cache(path.parent().expect("cache file has a folder"), MANIFEST_CACHE_KEEP),
                Some(Err(e)) => tracing::warn!("[Updater] Could not keep manifest {}: {}", path.display(), e),
                None => {}
            }
        }
        Ok(checked)
    }

    /// 받은 manifest를 검증해 이 릴리즈와 묶는다 (`verify_release` + `bind_release`)
    async fn fetch_verified(&self, latest: LatestRelease, platform: &str, channel: &str, current: &str) -> Result<VerifiedRelease, RelayError> {
        self.fetch_checked(&latest, |signed| verify_release(signed, platform, channel, current).and_then(|m| bind_release(latest.clone(), m)))
            .await
    }

    /// manifest 주소: 릴레이가 준 것. 경로만이면(`/latest`는 캐시되므로 요청 호스트를 싣지 않는다) 이 릴레이 주소에 붙인다.
    /// 옛 `/check`의 절대 주소는 그대로 쓰고, 없으면 판 번호로 찾는다
    fn manifest_url(&self, latest: &LatestRelease) -> String {
        match latest.manifest_url.as_deref() {
            Some(path) if path.starts_with('/') => format!("{}{}", self.base_url, path),
            Some(url) => url.to_string(),
            None => format!("{}/v1/updates/manifest/{}", self.base_url, latest.version),
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
        let Some(found) = self.latest(channel, platform, current).await? else {
            return self.fetch_verified_release_by_check(channel, platform, current, client_id).await;
        };
        let wait = stable_wait_base(channel, current);
        let mut candidates = found.releases;
        let mut installed = found.installed;
        // 같은 판 베타 규칙: 정식 채널이 아직 받지 않는 그 판의 베타도 본다 (`select_release`가 base로 거른다)
        if let Some(base) = wait.as_deref() {
            match self.latest("beta", platform, current).await {
                Ok(Some(beta)) => {
                    candidates.extend(beta.releases.into_iter().filter(|r| same_base_prerelease(&r.version, base)));
                    installed = installed.or(beta.installed);
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("[Updater] Could not look up beta builds of {}: {}", base, e),
            }
        }
        // 받을 것이 없으면 지금 판의 manifest로 구성요소(GUI·CLI…)만 비교한다
        let (latest, by_version) = match select_release(channel, current, client_id, candidates) {
            Some(latest) => (latest, false),
            None => match installed {
                Some(installed) if !current.is_empty() => (installed, true),
                _ => return Ok(None),
            },
        };
        // 같은 판 베타는 그 채널로 검증한다 (고른 정식 채널은 베타 manifest를 받지 않는다)
        let accept = match wait.as_deref() {
            Some(base) if same_base_prerelease(&latest.version, base) => "beta",
            _ => channel,
        };
        match self.fetch_verified(latest, platform, accept, current).await {
            Ok(release) => Ok(Some(release)),
            Err(RelayError::Status(404) | RelayError::WrongChannel { .. } | RelayError::WrongTarget { .. }) if by_version => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 옛 릴레이의 확인 (`/v1/updates/check`, 롤아웃은 릴레이가 판정한다)
    async fn fetch_verified_release_by_check(
        &self,
        channel: &str,
        platform: &str,
        current: &str,
        client_id: &str,
    ) -> Result<Option<VerifiedRelease>, RelayError> {
        let check = self.check(channel, platform, current, client_id).await?;
        // 코어가 최신이어도 같은 릴리즈의 다른 컴포넌트(GUI·CLI·모듈...)는 뒤처져 있을 수 있다:
        // 그 릴리즈 manifest를 받아 컴포넌트별로 비교하게 한다
        let by_version = check.up_to_date && check.latest.is_none();
        let latest = match (check.up_to_date, check.latest) {
            (_, Some(latest)) => latest,
            (true, None) if !current.is_empty() => LatestRelease { version: current.to_string(), ..Default::default() },
            _ => return Ok(None),
        };
        match self.fetch_verified(latest, platform, channel, current).await {
            Ok(release) => Ok(Some(release)),
            // 최신인데 그 버전 manifest가 릴레이에 없으면(옛 릴리즈) 받을 것이 없다
            Err(RelayError::Status(404)) if check.up_to_date => Ok(None),
            // 최신이고 릴리즈를 받지 않았으면 버전만으로 manifest를 찾는다. 같은 버전이 다른 채널에도 있으면 그쪽 것이 온다.
            // 다른 채널의 구성요소는 같은 코어 버전이어도 다르다: 비교하지 않고 받을 것이 없다고 본다.
            // 그 버전에 이 플랫폼 몫이 없어도(아직 그 플랫폼 빌드가 나오지 않은 버전) 받을 것이 없다
            Err(RelayError::WrongChannel { .. } | RelayError::WrongTarget { .. }) if by_version => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 아무것도 깔리지 않은 상태의 확인(모듈·익스텐션 카탈로그, 설치기): 릴레이에는 현재 버전 없이 묻되,
    /// `floor`(이 빌드의 릴리즈 버전)보다 옛 manifest는 다운그레이드로 거절한다. 릴레이가 옛 서명 manifest를 내주어도
    /// 새 설치가 옛 코어를, 처음 받는 모듈이 옛 판을 공식으로 받지 않게
    pub async fn fetch_verified_catalog(
        &self,
        channel: &str,
        platform: &str,
        floor: &str,
        client_id: &str,
    ) -> Result<Option<VerifiedRelease>, RelayError> {
        let latest = match self.latest(channel, platform, "").await? {
            Some(found) => select_release(channel, "", client_id, found.releases),
            None => self.check(channel, platform, "", client_id).await?.latest,
        };
        // 베타로 먼저 낸 빌드(정식 채널 빌드인데 판이 정식 최신보다 높다)면 정식 최신은 다운그레이드다:
        // 베타에서 이 빌드와 같은 판(그 판의 프리릴리스 포함)을 그 채널로 검증해 쓴다
        let behind = |r: &LatestRelease| SemVer::parse(&r.version).zip(SemVer::parse(floor)).is_some_and(|(v, f)| v < f);
        if channel != "beta" && latest.as_ref().is_none_or(behind) {
            if let Some(found) = self.latest("beta", platform, "").await.ok().flatten() {
                let own = found
                    .releases
                    .into_iter()
                    .filter(|r| r.version == floor || same_base_prerelease(&r.version, floor))
                    .filter(|r| in_rollout(client_id, &r.version, r.rollout_percent.unwrap_or(100)))
                    .filter_map(|r| SemVer::parse(&r.version).map(|v| (v, r)))
                    .max_by(|a, b| a.0.cmp(&b.0));
                if let Some((_, own)) = own {
                    return self.fetch_verified(own, platform, "beta", floor).await.map(Some);
                }
            }
        }
        let Some(latest) = latest else { return Ok(None) };
        self.fetch_verified(latest, platform, channel, floor).await.map(Some)
    }

    /// 이 코어 버전으로 돌아가는 모듈·익스텐션 판 목록 (`GET /v1/addons/catalog`). 판마다 번들 공개키로 서명을 확인하고,
    /// 서명·경로가 틀린 판은 빼고 남긴다. 이 API가 없는 릴레이(404)이거나 올린 판이 하나도 없으면 None:
    /// 그때는 릴리즈 manifest의 모듈·익스텐션을 그대로 쓴다
    pub async fn fetch_addon_catalog(&self, core: &str, channel: &str) -> Result<Option<AddonCatalog>, RelayError> {
        let url = format!("{}/v1/addons/catalog", self.base_url);
        let resp = self
            .http
            .get(&url)
            .query(&[("core", core), ("channel", channel)])
            .send()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: AddonCatalogResponse = Self::json(resp).await?;
        let catalog = AddonCatalog::from_signed(body.addons, body.yanked);
        // 판이 하나도 없어도 내린 판은 알아야 한다: 릴리즈 manifest의 그 판을 다시 받지 않게
        Ok((!catalog.components.is_empty() || !catalog.yanked.is_empty()).then_some(catalog))
    }

    async fn json<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T, RelayError> {
        if !resp.status().is_success() {
            return Err(RelayError::Status(resp.status().as_u16()));
        }
        resp.json::<T>().await.map_err(|e| RelayError::Parse(e.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct AddonCatalogResponse {
    #[serde(default)]
    addons: Vec<SignedManifest>,
    /// 서명된 내림 기록 `{manifest:{key, version, yanked:true}, signature, keyId}`
    #[serde(default)]
    yanked: Vec<SignedManifest>,
}

/// 코어 버전에 맞는 모듈·익스텐션 판 (앱 릴리즈와 따로 내는 판, 릴레이 `/v1/addons/catalog`)
#[derive(Debug, Clone, Default)]
pub struct AddonCatalog {
    /// 키(`module-<이름>`, `ext-<id>`) → 판. 릴리즈 manifest의 컴포넌트와 같은 모양
    pub components: HashMap<String, ManifestComponent>,
    /// 내린 판 (CI가 서명한 기록만): 이 판을 깐 데몬은 목록의 판으로 되돌린다
    pub yanked: HashMap<String, Vec<String>>,
}

impl AddonCatalog {
    /// 서명된 판 기록(`{manifest:{key, version, asset, sha256, url, requires?, meta?}, signature, keyId}`)을 확인해 모은다.
    /// 서명이 틀렸거나, 키가 모듈·익스텐션이 아니거나, 경로가 폴더 밖을 가리키는 판은 버린다
    pub fn from_signed(entries: Vec<SignedManifest>, yanked: Vec<SignedManifest>) -> Self {
        Self::from_signed_with(entries, yanked, signature::verify)
    }

    fn from_signed_with(
        entries: Vec<SignedManifest>,
        yanks: Vec<SignedManifest>,
        verify: impl Fn(&SignedManifest) -> Result<(), signature::SignatureError>,
    ) -> Self {
        // 내림 기록: 서명이 맞고 `yanked: true`인 모듈·익스텐션 판만. 릴레이가 붙인 서명 없는 표시는 믿지 않는다
        let mut yanked: HashMap<String, Vec<String>> = HashMap::new();
        for signed in yanks {
            let m = &signed.manifest;
            let (Some(key), Some(version)) = (m.get("key").and_then(|v| v.as_str()), m.get("version").and_then(|v| v.as_str())) else {
                continue;
            };
            let addon = key.starts_with(MODULE_PREFIX) || key.starts_with(EXTENSION_PREFIX);
            if m.get("yanked") != Some(&serde_json::Value::Bool(true)) || !addon || verify(&signed).is_err() {
                tracing::warn!("[Updater] Ignoring a yank record for '{}' {}: not a signed addon yank", key, version);
                continue;
            }
            yanked.entry(key.to_string()).or_default().push(version.to_string());
        }
        let mut components = HashMap::new();
        for signed in entries {
            let key = signed.manifest.get("key").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
            if let Err(e) = verify(&signed) {
                tracing::warn!("[Updater] Dropping addon '{}' from the catalog: {}", key, e);
                continue;
            }
            if !key.starts_with(MODULE_PREFIX) && !key.starts_with(EXTENSION_PREFIX) {
                tracing::warn!("[Updater] Dropping catalog entry '{}': not a module or extension", key);
                continue;
            }
            let component: ManifestComponent = match serde_json::from_value(signed.manifest.clone()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[Updater] Dropping addon '{}' from the catalog: {}", key, e);
                    continue;
                }
            };
            let single = ReleaseManifest {
                version: component.version.clone(),
                channel: String::new(),
                platform: String::new(),
                released_at: None,
                is_critical: false,
                components: HashMap::from([(key.clone(), component.clone())]),
                platforms: HashMap::new(),
            };
            if let Err(e) = check_manifest_paths(&single) {
                tracing::warn!("[Updater] Dropping addon '{}' from the catalog: {}", key, e);
                continue;
            }
            components.insert(key, component);
        }
        Self { components, yanked }
    }

    /// 이 판이 내린 판인가
    pub fn is_yanked(&self, key: &str, version: &str) -> bool {
        self.yanked.get(key).is_some_and(|v| v.iter().any(|y| y == version))
    }

    /// 릴리즈 manifest의 모듈·익스텐션 중 이 목록에 있는 것을 목록의 판으로 바꾼다.
    /// 목록에 없는 것(아직 따로 올리지 않은 판)은 릴리즈 manifest 그대로 둔다: 받을 때 requires 검사가 맞지 않는 판을 막는다.
    /// 다만 내린 판이면 빼서 다시 받지 않게 한다
    pub fn overlay(&self, manifest: &mut ReleaseManifest) {
        manifest.components.extend(self.components.clone());
        manifest.components.retain(|key, c| !self.is_yanked(key, &c.version));
    }
}

/// 서명을 검증하고, 플랫폼이 맞는지, 고른 채널이 받는 채널인지, 현재보다 낮은 버전이 아닌지 확인한다.
pub fn verify_release(
    signed: &SignedManifest,
    platform: &str,
    channel: &str,
    current: &str,
) -> Result<ReleaseManifest, RelayError> {
    signature::verify(signed)?;
    let manifest: ReleaseManifest =
        serde_json::from_value(signed.manifest.clone()).map_err(|e| RelayError::Parse(e.to_string()))?;
    let manifest = manifest.for_platform(platform)?;
    if !channel_includes(channel, &manifest.channel) {
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

/// 보관한 manifest 중 릴리즈 id가 큰 `keep`개만 남긴다
pub fn prune_manifest_cache(dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut ids: Vec<(u64, std::path::PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| Some((p.file_name()?.to_str()?.strip_suffix(".json")?.parse().ok()?, p.clone())))
        .collect();
    ids.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
    for (_, path) in ids.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
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
    let _ = crate::write_atomic(&path, id.as_bytes());
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

    /// "설치된 앱" 목록의 표시 버전은 데몬(saba-core) 버전이다. 데몬이 없는 manifest만 릴리즈 버전으로 둔다
    #[test]
    fn display_version_follows_the_daemon() {
        let comp = |v: &str| -> ManifestComponent {
            serde_json::from_value(json!({"version": v, "asset": "a.zip", "sha256": "00", "url": "https://x/a.zip"})).unwrap()
        };
        let mut m = ReleaseManifest {
            version: "0.1.13".into(), channel: "stable".into(), platform: "windows-x64".into(), released_at: None,
            is_critical: false, components: [("saba-core".to_string(), comp("0.1.12")), ("gui".to_string(), comp("0.1.14"))].into_iter().collect(),
            platforms: Default::default(),
        };
        assert_eq!(m.display_version(), "0.1.12");
        m.components.remove("saba-core");
        assert_eq!(m.display_version(), "0.1.13");
    }

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
    fn a_higher_channel_takes_lower_channel_releases_but_not_the_reverse() {
        assert!(channel_includes("stable", "stable"));
        assert!(channel_includes("rc", "stable"));
        assert!(channel_includes("beta", "rc"));
        assert!(channel_includes("beta", "stable"));
        assert!(!channel_includes("stable", "rc"));
        assert!(!channel_includes("stable", "beta"));
        assert!(!channel_includes("rc", "beta"));
        // 모르는 채널은 같은 이름끼리만
        assert!(channel_includes("custom", "custom"));
        assert!(!channel_includes("custom", "stable"));
        assert!(!channel_includes("stable", "custom"));
    }

    #[test]
    fn a_beta_client_accepts_a_stable_manifest() {
        let signed: SignedManifest = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        assert!(verify_release(&signed, "windows-x64", "stable", "0.1.0").is_ok());
        assert!(verify_release(&signed, "windows-x64", "beta", "0.1.0").is_ok());
        assert!(matches!(
            verify_release(&signed, "windows-x64", "custom", "0.1.0"),
            Err(RelayError::WrongChannel { .. })
        ));
    }

    /// 옛 릴레이(`/latest` 없음): check와 manifest에 답한다
    async fn fake_relay(check: String, manifest: String) -> String {
        routed_relay(vec![("/v1/updates/check", 200, check), ("/v1/updates/manifest/", 200, manifest)]).await.0
    }

    #[tokio::test]
    async fn an_up_to_date_client_does_not_compare_against_its_version_from_a_channel_it_does_not_take() {
        // 같은 버전이 여러 채널에 있으면 버전만 든 manifest 주소는 다른 채널 것을 줄 수 있다.
        // 고른 채널이 받지 않는 채널의 구성요소로 바꾸지 않는다: 받을 것이 없다 (외부 감사 10-23)
        let base = fake_relay(r#"{"upToDate":true}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("custom", "windows-x64", "0.1.1", "t").await;
        assert!(matches!(got, Ok(None)), "{:?}", got.map(|r| r.map(|v| v.manifest.version)));
    }

    #[tokio::test]
    async fn an_up_to_date_client_compares_against_its_version_from_its_own_channel() {
        let base = fake_relay(r#"{"upToDate":true}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("stable", "windows-x64", "0.1.1", "t").await;
        assert_eq!(got.unwrap().map(|v| v.manifest.version).as_deref(), Some("0.1.1"));
    }

    /// 지금 버전 릴리즈에 이 플랫폼 몫이 없으면(그 버전에 아직 linux-arm64가 나오지 않음) 받을 것이 없다. 확인 실패가 아니다
    #[tokio::test]
    async fn an_up_to_date_client_whose_version_has_no_build_for_its_platform_has_nothing_to_get() {
        let base = fake_relay(r#"{"upToDate":true}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("stable", "linux-arm64", "0.1.1", "t").await;
        assert!(matches!(got, Ok(None)), "{:?}", got.map(|r| r.map(|v| v.manifest.version)));
    }

    // U-11: 최신이라는 답에 다른 버전이 붙어 와도, 고른 채널이 받지 않는 채널의 그 버전 manifest는 받지 않는다
    #[tokio::test]
    async fn an_up_to_date_answer_does_not_bring_another_channels_newer_version() {
        let base = fake_relay(r#"{"upToDate":true,"latest":{"version":"0.1.1"}}"#.into(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_release("custom", "windows-x64", "0.1.0", "t").await;
        assert!(matches!(got, Err(RelayError::WrongChannel { .. })), "{:?}", got.map(|r| r.map(|v| v.manifest.version)));
    }

    // EE3: 아무것도 깔리지 않은 확인(카탈로그·설치기)도 이 빌드보다 옛 릴리즈는 받지 않는다
    #[tokio::test]
    async fn a_catalog_check_refuses_a_release_older_than_this_build() {
        let check = r#"{"upToDate":false,"latest":{"version":"0.1.1"}}"#.to_string();
        let base = fake_relay(check.clone(), PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_catalog("stable", "windows-x64", "0.2.0", "t").await;
        assert!(matches!(got, Err(RelayError::Downgrade { .. })), "{:?}", got.map(|r| r.map(|v| v.manifest.version)));
        let base = fake_relay(check, PRODUCTION_0_1_1.into()).await;
        let got = RelayClient::new(&base).fetch_verified_catalog("stable", "windows-x64", "0.1.0", "t").await;
        assert_eq!(got.unwrap().map(|v| v.manifest.version).as_deref(), Some("0.1.1"));
    }

    // 베타로 먼저 낸 빌드(정식 채널 빌드인데 판이 정식 최신보다 높다)의 설치기·카탈로그는 정식 최신을 다운그레이드로
    // 거절해 모듈 목록이 비었다: 베타에서 이 빌드와 같은 판을 받는다 (맥 설치기 11-12)
    #[tokio::test]
    async fn a_build_ahead_of_its_channel_takes_its_own_version_from_beta() {
        let (base, seen) = routed_relay(vec![
            ("/v1/updates/latest?channel=beta", 200, r#"{"releases":[{"version":"0.3.0","channel":"beta"},{"version":"0.2.0","channel":"beta"}]}"#.into()),
            ("/v1/updates/latest", 200, r#"{"releases":[{"version":"0.1.1","channel":"stable"}]}"#.into()),
            ("/v1/updates/manifest/", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let _ = RelayClient::new(&base).fetch_verified_catalog("stable", "windows-x64", "0.2.0", "t").await;
        let seen = seen.lock().unwrap().clone();
        assert!(seen.iter().any(|p| p.starts_with("/v1/updates/manifest/0.2.0")), "{seen:?}");
        assert!(!seen.iter().any(|p| p.starts_with("/v1/updates/manifest/0.1.1") || p.starts_with("/v1/updates/manifest/0.3.0")), "{seen:?}");
    }

    /// 경로마다 정한 답을 주고 요청을 센다 (몇 번이든 받는다). `(경로 앞부분, 상태, 본문)`
    async fn routed_relay(routes: Vec<(&'static str, u16, String)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or_default().to_string();
                log.lock().unwrap().push(path.clone());
                let (status, body) = routes
                    .iter()
                    .find(|(prefix, _, _)| path.starts_with(prefix))
                    .map_or((404, String::new()), |(_, st, b)| (*st, b.clone()));
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (format!("http://{}", addr), seen)
    }

    /// 릴레이 `isInRollout`과 같은 값: sha256("client-a:0.1.14") 앞 4바이트 % 100 = 22, client-b는 38
    #[test]
    fn rollout_is_decided_on_the_client_like_the_relay() {
        assert!(in_rollout("client-a", "0.1.14", 23));
        assert!(!in_rollout("client-a", "0.1.14", 22));
        assert!(in_rollout("client-b", "0.1.14", 39));
        assert!(!in_rollout("client-b", "0.1.14", 38));
        assert!(in_rollout("anyone", "1.0.0", 100) && in_rollout("anyone", "1.0.0", 250));
        assert!(!in_rollout("anyone", "1.0.0", 0) && !in_rollout("anyone", "1.0.0", -5));
    }

    fn release(version: &str, channel: &str, rollout: i64) -> LatestRelease {
        LatestRelease { version: version.into(), channel: Some(channel.into()), rollout_percent: Some(rollout), ..Default::default() }
    }

    /// 높은 판부터 보며 이 클라이언트가 배포 비율에 드는 첫 판을 고른다
    #[test]
    fn the_highest_release_this_client_is_rolled_out_to_wins() {
        let picked = |list: Vec<LatestRelease>| select_release("stable", "0.1.13", "client-a", list).map(|r| r.version);
        // client-a는 0.1.14에서 22번 칸: 20%에는 들지 않는다
        assert_eq!(picked(vec![release("0.1.14", "stable", 20), release("0.1.13", "stable", 100)]), None);
        assert_eq!(picked(vec![release("0.1.14", "stable", 20), release("0.1.13-rc.1", "stable", 100)]), None, "lower than current is ignored");
        assert_eq!(picked(vec![release("0.1.15", "stable", 0), release("0.1.14", "stable", 30)]).as_deref(), Some("0.1.14"));
        // 고른 채널이 받지 않는 채널의 판은 고르지 않는다
        assert_eq!(picked(vec![release("0.1.15-beta.1", "beta", 100)]), None);
    }

    /// 13절: 같은 판 베타는 base가 정확히 같은 것만. 크기 비교로 대신하면 0.1.15-beta.1을 받아 버린다
    #[test]
    fn a_stable_client_on_a_beta_takes_only_betas_of_the_same_release() {
        let picked = |current: &str, list: Vec<LatestRelease>| select_release("stable", current, "c", list).map(|r| r.version);
        assert_eq!(stable_wait_base("stable", "0.1.14-beta.2").as_deref(), Some("0.1.14"));
        assert_eq!(stable_wait_base("beta", "0.1.14-beta.2"), None);
        assert_eq!(stable_wait_base("stable", "0.1.14"), None);
        assert_eq!(
            picked("0.1.14-beta.2", vec![release("0.1.15-beta.1", "beta", 100), release("0.1.14-beta.3", "beta", 100)]).as_deref(),
            Some("0.1.14-beta.3")
        );
        assert_eq!(picked("0.1.14-beta.2", vec![release("0.1.15-beta.1", "beta", 100)]), None);
        assert_eq!(picked("0.1.14-beta.2", vec![release("0.1.14-beta.3", "beta", 100), release("0.1.14", "stable", 100)]).as_deref(), Some("0.1.14"));
        // 정식이 된 뒤에는 베타를 받지 않는다
        assert_eq!(picked("0.1.14", vec![release("0.1.15-beta.1", "beta", 100)]), None);
    }

    /// 러너 형식 범위: manifest에 없으면 [0, 1]로 본다
    #[test]
    fn a_core_without_runner_formats_reads_formats_zero_and_one() {
        let comp = |extra: serde_json::Value| -> ManifestComponent {
            let mut v = json!({"version": "0.1.14", "asset": "a.zip", "sha256": "00", "url": "https://x/a.zip"});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            serde_json::from_value(v).unwrap()
        };
        assert_eq!(comp(json!({})).runner_formats(), (0, 1));
        assert_eq!(comp(json!({"reads_runner_formats": [1, 2]})).runner_formats(), (1, 2));
        assert!(serde_json::to_value(comp(json!({}))).unwrap().get("reads_runner_formats").is_none());
    }

    /// 새 확인 주소: client_id를 싣지 않고, 받은 manifest는 릴리즈 id로 남겨 두어 같은 판을 다시 받지 않는다
    #[tokio::test]
    async fn the_cached_check_keeps_the_manifest_by_release_id() {
        let latest = r#"{"releases":[{"id":7,"version":"0.1.1","channel":"stable","rolloutPercent":100}],"installed":null}"#;
        let (base, seen_log) = routed_relay(vec![
            ("/v1/updates/latest", 200, latest.into()),
            ("/v1/updates/manifest/", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let cache = tempfile::tempdir().unwrap();
        let client = RelayClient::new(&base).with_manifest_cache(cache.path().to_path_buf());
        for _ in 0..2 {
            let got = client.fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap().unwrap();
            assert_eq!(got.manifest.version, "0.1.1");
            assert_eq!(got.latest.id, Some(7));
        }
        let seen = seen_log.lock().unwrap().clone();
        assert!(seen.iter().all(|p| !p.contains("client_id")), "{seen:?}");
        assert_eq!(seen.iter().filter(|p| p.starts_with("/v1/updates/manifest/")).count(), 1, "{seen:?}");
        assert!(cache.path().join("7.json").is_file());

        // 남겨 둔 원문도 매번 검증한다: 고친 파일은 쓰지 않고 지운 뒤 한 번 다시 받는다
        let mut tampered: serde_json::Value = serde_json::from_str(PRODUCTION_0_1_1).unwrap();
        tampered["manifest"]["components"]["gui"]["url"] = json!("https://evil.example/gui.zip");
        std::fs::write(cache.path().join("7.json"), tampered.to_string()).unwrap();
        let got = client.fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap().unwrap();
        assert_ne!(got.manifest.components["gui"].url, "https://evil.example/gui.zip");
        assert_eq!(seen_manifests(&seen_log), 2, "fetched once more");
        assert_eq!(std::fs::read_to_string(cache.path().join("7.json")).unwrap(), PRODUCTION_0_1_1, "the verified copy replaced it");
    }

    fn seen_manifests(seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> usize {
        seen.lock().unwrap().iter().filter(|p| p.starts_with("/v1/updates/manifest/")).count()
    }

    /// 검증에 실패한 manifest(광고한 판과 서명된 판이 다름)는 보관하지 않는다: 릴레이가 고쳐지면 다음 확인이 다시 받는다
    #[tokio::test]
    async fn a_manifest_that_fails_verification_is_not_kept() {
        let latest = r#"{"releases":[{"id":7,"version":"0.1.2","channel":"stable","rolloutPercent":100}],"installed":null}"#;
        let (base, seen) = routed_relay(vec![
            ("/v1/updates/latest", 200, latest.into()),
            ("/v1/updates/manifest/", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let cache = tempfile::tempdir().unwrap();
        let client = RelayClient::new(&base).with_manifest_cache(cache.path().to_path_buf());
        for _ in 0..2 {
            assert!(matches!(
                client.fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await,
                Err(RelayError::VersionMismatch { .. })
            ));
        }
        assert!(!cache.path().join("7.json").exists());
        assert_eq!(seen_manifests(&seen), 2, "asked the relay again instead of reusing a bad copy");
    }

    /// heartbeat가 알린 릴리즈 id를 `/latest` 캐시 키(`r`)로 붙이고, 응답에 담긴 가장 큰 id를 알려 준다
    #[tokio::test]
    async fn the_heartbeat_release_id_is_the_cache_key_of_the_cached_check() {
        let latest = r#"{"releases":[{"id":9,"version":"0.1.1","channel":"stable","rolloutPercent":100}],"installed":{"id":4,"version":"0.1.0","channel":"stable"}}"#;
        let (base, seen) = routed_relay(vec![
            ("/v1/updates/latest", 200, latest.into()),
            ("/v1/updates/manifest/", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let client = RelayClient::new(&base).with_release_hint(Some(12));
        assert_eq!(client.latest_seen_id(), None);
        client.fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap();
        assert_eq!(client.latest_seen_id(), Some(9));
        let first = seen.lock().unwrap()[0].clone();
        assert!(first.starts_with("/v1/updates/latest?") && first.ends_with("&r=12"), "{first}");
        // 알린 id가 없으면 붙이지 않는다
        RelayClient::new(&base).fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap();
        assert!(!seen.lock().unwrap().iter().filter(|p| p.starts_with("/v1/updates/latest")).nth(1).unwrap().contains("r="));
    }

    /// `/latest`는 경로만인 manifest 주소를 준다: 이 릴레이 주소에 붙여 받는다. 옛 `/check`의 절대 주소는 그대로 쓴다
    #[tokio::test]
    async fn a_path_only_manifest_url_is_fetched_from_this_relay() {
        let latest = r#"{"releases":[{"id":7,"version":"0.1.1","channel":"stable","rolloutPercent":100,"manifestUrl":"/v1/updates/manifest/0.1.1?release=7"}],"installed":null}"#;
        let (base, seen) = routed_relay(vec![
            ("/v1/updates/latest", 200, latest.into()),
            ("/v1/updates/manifest/0.1.1?release=7", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let got = RelayClient::new(&base).fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap().unwrap();
        assert_eq!(got.manifest.version, "0.1.1");
        assert!(seen.lock().unwrap().iter().any(|p| p == "/v1/updates/manifest/0.1.1?release=7"));

        let client = RelayClient::new("https://relay.example/");
        let with = |url: Option<&str>| LatestRelease { version: "0.1.1".into(), manifest_url: url.map(str::to_string), ..Default::default() };
        assert_eq!(client.manifest_url(&with(Some("/v1/updates/manifest/0.1.1?release=7"))), "https://relay.example/v1/updates/manifest/0.1.1?release=7");
        assert_eq!(client.manifest_url(&with(Some("https://other.example/v1/updates/manifest/0.1.1"))), "https://other.example/v1/updates/manifest/0.1.1");
        assert_eq!(client.manifest_url(&with(None)), "https://relay.example/v1/updates/manifest/0.1.1");
    }

    /// 옛 릴레이(새 주소가 404)면 옛 확인으로 한 번 물러선다
    #[tokio::test]
    async fn an_old_relay_without_the_cached_check_falls_back_to_check() {
        let (base, seen) = routed_relay(vec![
            ("/v1/updates/check", 200, r#"{"upToDate":false,"latest":{"version":"0.1.1"}}"#.into()),
            ("/v1/updates/manifest/", 200, PRODUCTION_0_1_1.into()),
        ])
        .await;
        let got = RelayClient::new(&base).fetch_verified_release("stable", "windows-x64", "0.1.0", "t").await.unwrap();
        assert_eq!(got.map(|r| r.manifest.version).as_deref(), Some("0.1.1"));
        let seen = seen.lock().unwrap().clone();
        assert!(seen[0].starts_with("/v1/updates/latest"), "{seen:?}");
        assert!(seen[1].starts_with("/v1/updates/check"), "{seen:?}");
    }

    #[test]
    fn only_the_newest_manifests_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for id in [3, 11, 7, 1, 9, 10, 2] {
            std::fs::write(dir.path().join(format!("{id}.json")), "{}").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        prune_manifest_cache(dir.path(), MANIFEST_CACHE_KEEP);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["10.json", "11.json", "3.json", "7.json", "9.json", "notes.txt"]);
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
                    "asset": "saba-core.zip",
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

    /// 판 목록을 읽지 못하면(네트워크·서버 오류) 이번 확인에서는 모듈·익스텐션을 내지 않는다: 릴리즈 manifest의 판은
    /// 그 뒤 내린 판일 수 있고, 내림 기록은 목록에만 있다. 목록 API가 없는 옛 릴레이(`Ok(None)`)는 릴리즈 manifest 그대로
    #[test]
    fn an_unreadable_addon_catalog_offers_no_addons() {
        let mut v = manifest("0.1.13", "windows-x64");
        v["components"]["module-palworld"] = json!({ "version": "0.1.6", "asset": "m.zip", "sha256": "a".repeat(64), "url": "https://cdn.example/m.zip" });
        let release = || Some(VerifiedRelease { latest: LatestRelease::default(), manifest: serde_json::from_value(v.clone()).unwrap() });
        let keys = |r: Option<VerifiedRelease>| {
            let mut k: Vec<String> = r.unwrap().manifest.components.into_keys().collect();
            k.sort();
            k
        };
        let failed = merge_addon_catalog(release(), Err(RelayError::Http("timeout".into())), "0.1.13", "stable", "windows-x64");
        assert_eq!(keys(failed), ["saba-core"]);
        let old_relay = merge_addon_catalog(release(), Ok(None), "0.1.13", "stable", "windows-x64");
        assert_eq!(keys(old_relay), ["module-palworld", "saba-core"]);
        assert!(merge_addon_catalog(None, Err(RelayError::Http("x".into())), "0.1.13", "stable", "windows-x64").is_none());
    }

    /// 모듈·익스텐션 판 목록: 서명이 맞고 경로가 안전한 모듈·익스텐션 판만 남기고, 릴리즈 manifest의 판을 바꾼다
    #[test]
    fn the_addon_catalog_keeps_only_signed_addon_entries_and_replaces_the_release_ones() {
        let (private, public) = signature::generate_key_pair().unwrap();
        let entry = |key: &str, version: &str, asset: &str| {
            json!({ "key": key, "version": version, "asset": asset, "sha256": "a".repeat(64),
                    "url": format!("https://cdn.example/{asset}"), "requires": { "saba-core": ">=0.1.12" } })
        };
        let signed = |m: serde_json::Value| signature::sign(m, "k", &private).unwrap();
        let mut tampered = signed(entry("module-zomboid", "0.1.6", "module-zomboid.zip"));
        tampered.manifest["url"] = json!("https://evil.example/x.zip");
        let entries = vec![
            signed(entry("module-palworld", "0.1.5", "module-palworld.zip")),
            signed(entry("ext-steamcmd", "0.1.4", "ext-steamcmd.zip")),
            tampered,
            signed(entry("saba-core", "9.9.9", "saba-core.zip")),
            signed(entry("module-minecraft", "0.1.7", "../evil.zip")),
        ];
        // 내림 기록도 CI가 서명한다: 릴레이가 마음대로 되돌리게 하지 못한다
        let yank = |key: &str, version: &str| signed(json!({ "key": key, "version": version, "yanked": true }));
        let mut forged = yank("module-palworld", "0.1.5");
        forged.manifest["version"] = json!("0.1.4");
        let yanked = vec![
            yank("module-palworld", "0.1.6"),
            yank("module-zomboid", "0.1.6"),
            forged,
            signed(json!({ "key": "module-palworld", "version": "0.1.3" })),
        ];
        let catalog = AddonCatalog::from_signed_with(entries, yanked, |s| signature::verify_with(s, &[("k", public.as_str())]));
        let mut keys: Vec<_> = catalog.components.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["ext-steamcmd", "module-palworld"]);
        assert_eq!(catalog.components["module-palworld"].requires.as_ref().unwrap()["saba-core"], ">=0.1.12");
        assert!(catalog.is_yanked("module-palworld", "0.1.6"));
        assert!(!catalog.is_yanked("module-palworld", "0.1.5"));
        assert!(!catalog.is_yanked("module-palworld", "0.1.4"), "a tampered yank record is ignored");
        assert!(!catalog.is_yanked("module-palworld", "0.1.3"), "a signed entry without yanked:true is not a yank");

        let mut v = manifest("0.1.13", "windows-x64");
        v["components"]["module-palworld"] = entry("module-palworld", "0.1.6", "module-palworld.zip");
        v["components"]["module-zomboid"] = entry("module-zomboid", "0.1.6", "module-zomboid.zip");
        v["components"]["module-terraria"] = entry("module-terraria", "0.1.5", "module-terraria.zip");
        let mut release: ReleaseManifest = serde_json::from_value(v).unwrap();
        catalog.overlay(&mut release);
        assert_eq!(release.components["module-palworld"].version, "0.1.5", "the catalog's version for this core wins");
        assert_eq!(release.components["module-terraria"].version, "0.1.5", "a module not in the catalog keeps the release's version");
        assert!(!release.components.contains_key("module-zomboid"), "a yanked release version is not offered again");
        assert!(release.components.contains_key("saba-core"), "app components stay");
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
        assert!(check_manifest_paths(&with("module-palworld", "asset", "module-palworld.zip")).is_ok());
        for (key, field, value) in [
            ("module-../evil", "asset", "a.zip"),
            ("module-..", "asset", "a.zip"),
            ("module-.", "asset", "a.zip"),
            ("ext-", "asset", "a.zip"),
            ("ext-a\\b", "asset", "a.zip"),
            ("saba-core", "asset", "../saba-core.zip"),
            ("saba-core", "asset", "C:saba-core.zip"),
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

    /// 한 서명 manifest에 다른 플랫폼 몫을 싣는다 (D3c): 맨 위는 windows-x64 그대로(0.1.10 업데이터가 읽는다),
    /// `platforms.<플랫폼>.components`가 그 플랫폼의 목록이다
    #[test]
    fn another_platform_takes_its_own_component_list() {
        let mut v = manifest("1.0.0", "windows-x64");
        v["platforms"] = json!({ "linux-x64": { "components": {
            "saba-core": {
                "version": "1.0.0",
                "asset": "saba-core-linux-x64.zip",
                "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "url": "https://cdn.example/saba-core-linux-x64.zip"
            }
        }}});
        let m: ReleaseManifest = serde_json::from_value(v).unwrap();

        let linux = m.clone().for_platform("linux-x64").unwrap();
        assert_eq!(linux.platform, "linux-x64");
        assert_eq!(linux.components["saba-core"].filename, "saba-core-linux-x64.zip");
        assert!(linux.platforms.is_empty());

        let windows = m.clone().for_platform("windows-x64").unwrap();
        assert_eq!(windows.components["saba-core"].filename, "saba-core.zip");
        assert!(matches!(m.for_platform("macos-arm64"), Err(RelayError::WrongTarget { .. })));
    }

    /// 구성마다 따로 빌드한 같은 컴포넌트(`<키>.<구성>`, 리눅스 서버용 창 없는 설치기)는 이 구성의 것만 `<키>`로 남는다.
    /// 다른 구성의 몫(구성이 아닌 `installer.dmg` 포함)은 빠지고, 모듈·익스텐션 이름의 `.`은 구성 표시가 아니다
    #[test]
    fn a_profile_build_stands_in_for_its_component() {
        let entry = |asset: &str| json!({
            "version": "1.0.0",
            "asset": asset,
            "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "url": format!("https://cdn.example/{asset}")
        });
        let mut v = manifest("1.0.0", "linux-x64");
        v["components"]["installer.desktop"] = entry("saba-chan-installer-desktop-linux-x64.zip");
        v["components"]["installer.headless"] = entry("saba-chan-installer-headless-linux-x64.zip");
        v["components"]["module-a.b"] = entry("module-a.b.zip");
        // 구성이 아닌 변형(맥 웹사이트 다운로드용 .dmg)은 어느 구성도 받지 않는다
        v["components"]["installer.dmg"] = entry("Saba-chan-macos-arm64.dmg");
        let m: ReleaseManifest = serde_json::from_value(v).unwrap();

        let headless = m.clone().for_profile(Profile::Headless);
        assert_eq!(headless.components["installer"].filename, "saba-chan-installer-headless-linux-x64.zip");
        assert!(!headless.components.keys().any(|k| k.starts_with("installer.")));
        assert!(headless.components.contains_key("module-a.b"));
        assert!(headless.components.contains_key("saba-core"));

        let desktop = m.for_profile(Profile::Desktop);
        assert_eq!(desktop.components["installer"].filename, "saba-chan-installer-desktop-linux-x64.zip");
        assert_eq!(desktop.components.len(), 3);
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
