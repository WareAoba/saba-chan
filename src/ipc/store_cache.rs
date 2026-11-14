//! 사바 스토리지 목록 캐시: 받을 수 있는 모듈·익스텐션 목록(릴레이 카탈로그)과 그 아이콘을 데몬이 받아 두고 바로 준다.
//!
//! GUI·CLI가 창을 열 때마다 릴레이와 GitHub에 다시 묻지 않는다. 목록이 오래되면 가진 것으로 먼저 답하고 뒤에서 새로 받는다.
//! 아이콘은 데이터 폴더(`cache/store-icons`)에 주소별로 저장하고, 목록을 새로 받을 때 미리 받아 둔다.
//! 설치·업데이트는 이 캐시를 쓰지 않고 그때 새로 받는다 (서명·버전 확인은 최신 목록으로).

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use saba_chan_updater_lib::relay::ReleaseManifest;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

/// 이만큼 지난 목록은 가진 것으로 답하고 뒤에서 새로 받는다
const FRESH_FOR: Duration = Duration::from_secs(30 * 60);
/// 이보다 큰 아이콘은 받지 않는다
const ICON_MAX_BYTES: usize = 1024 * 1024;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// 채널 → 그 채널의 카탈로그 (`relay::latest_catalog`)
type FetchCatalog = Arc<dyn Fn(String) -> BoxFut<Result<Option<ReleaseManifest>, String>> + Send + Sync>;
/// 주소 → 받은 바이트
type FetchBytes = Arc<dyn Fn(String) -> BoxFut<Result<Vec<u8>, String>> + Send + Sync>;

#[derive(Clone)]
struct Entry {
    channel: String,
    at: Instant,
    manifest: Option<Arc<ReleaseManifest>>,
}

pub struct StoreCache {
    fetch_catalog: FetchCatalog,
    fetch_bytes: FetchBytes,
    icons_dir: PathBuf,
    fresh_for: Duration,
    entry: Mutex<Option<Entry>>,
    refreshing: AtomicBool,
    /// 받는 중인 아이콘 (같은 아이콘을 두 번 받지 않는다)
    icon_lock: Mutex<()>,
}

/// 데몬이 쓰는 하나뿐인 캐시
pub fn store() -> &'static Arc<StoreCache> {
    static STORE: OnceLock<Arc<StoreCache>> = OnceLock::new();
    STORE.get_or_init(|| {
        let fetch_catalog: FetchCatalog = Arc::new(|channel| {
            Box::pin(async move {
                saba_chan_updater_lib::relay::latest_catalog(None, &channel).await.map_err(|e| e.to_string())
            })
        });
        let fetch_bytes: FetchBytes = Arc::new(|url| Box::pin(download(url)));
        let icons_dir = saba_chan_updater_lib::constants::resolve_data_dir().join("cache").join("store-icons");
        StoreCache::new(fetch_catalog, fetch_bytes, icons_dir, FRESH_FOR)
    })
}

async fn download(url: String) -> Result<Vec<u8>, String> {
    let client = saba_chan_updater_lib::tls::http_client_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    if resp.content_length().is_some_and(|n| n as usize > ICON_MAX_BYTES) {
        return Err("icon too large".into());
    }
    Ok(resp.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

impl StoreCache {
    fn new(fetch_catalog: FetchCatalog, fetch_bytes: FetchBytes, icons_dir: PathBuf, fresh_for: Duration) -> Arc<Self> {
        Arc::new(Self {
            fetch_catalog,
            fetch_bytes,
            icons_dir,
            fresh_for,
            entry: Mutex::new(None),
            refreshing: AtomicBool::new(false),
            icon_lock: Mutex::new(()),
        })
    }

    /// 이 채널의 카탈로그. `force`(새로고침 버튼)면 새로 받는다. 아니면 가진 것으로 답하고, 오래됐으면 뒤에서 새로 받는다.
    /// 처음이거나 채널이 바뀌었으면 받을 때까지 기다린다
    pub async fn catalog(self: &Arc<Self>, channel: &str, force: bool) -> Result<Option<Arc<ReleaseManifest>>, String> {
        let cached = self.entry.lock().await.clone().filter(|e| e.channel == channel);
        match cached {
            Some(entry) if !force => {
                if entry.at.elapsed() >= self.fresh_for {
                    self.refresh_in_background(channel);
                }
                Ok(entry.manifest)
            }
            _ => self.refresh(channel).await,
        }
    }

    /// 뒤에서 새로 받는다 (이미 받는 중이면 하지 않는다). 실패하면 가진 것을 그대로 둔다
    pub fn refresh_in_background(self: &Arc<Self>, channel: &str) {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        let channel = channel.to_string();
        tokio::spawn(async move {
            if let Err(e) = me.refresh(&channel).await {
                tracing::warn!("[Store] Catalog refresh failed: {}", e);
            }
            me.refreshing.store(false, Ordering::Release);
        });
    }

    async fn refresh(self: &Arc<Self>, channel: &str) -> Result<Option<Arc<ReleaseManifest>>, String> {
        let manifest = (self.fetch_catalog)(channel.to_string()).await?.map(Arc::new);
        *self.entry.lock().await = Some(Entry { channel: channel.to_string(), at: Instant::now(), manifest: manifest.clone() });
        // 창을 열기 전에 아이콘을 미리 받아 둔다
        if let Some(m) = manifest.clone() {
            let me = self.clone();
            tokio::spawn(async move {
                for (key, url) in icon_urls(&m) {
                    if let Err(e) = me.icon_from(&key, &url).await {
                        tracing::debug!("[Store] Icon prefetch failed for {}: {}", key, e);
                    }
                }
            });
        }
        Ok(manifest)
    }

    /// 카탈로그 항목(`module-<이름>`·`ext-<id>`)의 아이콘 PNG. 저장해 둔 것이 있으면 그것을, 없으면 받아 저장한다
    pub async fn icon(self: &Arc<Self>, key: &str, channel: &str) -> Result<Option<Vec<u8>>, String> {
        if !is_addon_key(key) {
            return Ok(None);
        }
        let Some(manifest) = self.catalog(channel, false).await? else { return Ok(None) };
        let Some(url) = manifest.components.get(key).and_then(|c| icon_url(c.meta.as_ref())) else {
            return Ok(None);
        };
        self.icon_from(key, &url).await.map(Some)
    }

    async fn icon_from(&self, key: &str, url: &str) -> Result<Vec<u8>, String> {
        let path = self.icons_dir.join(icon_file_name(key, url));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Ok(bytes);
        }
        let _guard = self.icon_lock.lock().await;
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Ok(bytes);
        }
        let bytes = (self.fetch_bytes)(url.to_string()).await?;
        if bytes.len() > ICON_MAX_BYTES || !bytes.starts_with(PNG_MAGIC) {
            return Err(format!("not a PNG icon: {}", url));
        }
        tokio::fs::create_dir_all(&self.icons_dir).await.map_err(|e| e.to_string())?;
        // 같은 항목의 지난 아이콘(다른 주소)은 지운다
        if let Ok(mut dir) = tokio::fs::read_dir(&self.icons_dir).await {
            let prefix = format!("{}-", key);
            while let Ok(Some(entry)) = dir.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) && name.len() == prefix.len() + 16 + 4 {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
        saba_atomic_file::write(&path, &bytes, saba_atomic_file::Mode::Plain).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

fn is_addon_key(key: &str) -> bool {
    crate::utils::is_single_path_component(key)
        && (key.starts_with(saba_chan_updater_lib::relay::MODULE_PREFIX)
            || key.starts_with(saba_chan_updater_lib::relay::EXTENSION_PREFIX))
}

/// 아이콘 파일 이름: 항목 키와 주소 해시 (주소가 바뀌면 새로 받는다)
fn icon_file_name(key: &str, url: &str) -> String {
    let hash = hex::encode(Sha256::digest(url.as_bytes()));
    format!("{}-{}.png", key, &hash[..16])
}

fn icon_url(meta: Option<&Value>) -> Option<String> {
    meta?.get("icon")?.as_str().filter(|u| u.starts_with("https://")).map(str::to_string)
}

fn icon_urls(manifest: &ReleaseManifest) -> Vec<(String, String)> {
    manifest
        .components
        .iter()
        .filter(|(key, _)| is_addon_key(key))
        .filter_map(|(key, c)| icon_url(c.meta.as_ref()).map(|u| (key.clone(), u)))
        .collect()
}

/// 목록 API의 `?refresh=true` (새로고침 버튼): 받아 둔 목록 대신 새로 받는다
#[derive(serde::Deserialize, Default)]
pub struct RefreshQuery {
    #[serde(default)]
    pub refresh: bool,
}

/// GET /api/store/icons/{key} — 받을 수 있는 모듈·익스텐션(`module-<이름>`·`ext-<id>`)의 아이콘 PNG.
/// 데몬이 저장해 둔 것을 준다. 아이콘이 없으면 404
pub async fn serve_icon(axum::extract::Path(key): axum::extract::Path<String>) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    let channel = crate::ipc::updates::load_updater_config().channel;
    match store().icon(&key, &channel).await {
        Ok(Some(bytes)) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::warn!("[Store] Icon for {} failed: {}", key, e);
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// 목록 응답용 meta: 아이콘 주소는 빼고 `has_icon`만 남긴다 (아이콘은 데몬의 `/api/store/icons/{key}`로 받는다)
pub fn public_meta(meta: Option<&Value>) -> serde_json::Map<String, Value> {
    let mut info = match meta {
        Some(Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    let has_icon = icon_url(meta).is_some();
    info.remove("icon");
    info.insert("has_icon".into(), Value::Bool(has_icon));
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nbody";

    fn manifest(version: &str) -> ReleaseManifest {
        let comp = serde_json::json!({
            "version": version, "asset": "module-game.zip", "sha256": "00", "url": "https://r2/x.zip",
            "meta": {"display_name": "Game", "icon": format!("https://raw.example/{version}/icon.png")}
        });
        ReleaseManifest {
            version: "0.1.15".into(),
            channel: "beta".into(),
            platform: "windows-x64".into(),
            released_at: None,
            is_critical: false,
            components: HashMap::from([("module-game".to_string(), serde_json::from_value(comp).unwrap())]),
            platforms: HashMap::new(),
        }
    }

    struct Fake {
        cache: Arc<StoreCache>,
        catalog_calls: Arc<AtomicUsize>,
        icon_calls: Arc<AtomicUsize>,
        version: Arc<std::sync::Mutex<String>>,
        fail: Arc<AtomicBool>,
        _dir: tempfile::TempDir,
    }

    fn fake(fresh_for: Duration) -> Fake {
        let catalog_calls = Arc::new(AtomicUsize::new(0));
        let icon_calls = Arc::new(AtomicUsize::new(0));
        let version = Arc::new(std::sync::Mutex::new("1.0.0".to_string()));
        let fail = Arc::new(AtomicBool::new(false));
        let (c, v, f) = (catalog_calls.clone(), version.clone(), fail.clone());
        let fetch_catalog: FetchCatalog = Arc::new(move |_channel| {
            c.fetch_add(1, Ordering::SeqCst);
            let result = if f.load(Ordering::SeqCst) { Err("offline".to_string()) } else { Ok(Some(manifest(&v.lock().unwrap()))) };
            Box::pin(async move { result })
        });
        let i = icon_calls.clone();
        let fetch_bytes: FetchBytes = Arc::new(move |_url| {
            i.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(PNG.to_vec()) })
        });
        let dir = tempfile::tempdir().unwrap();
        let cache = StoreCache::new(fetch_catalog, fetch_bytes, dir.path().join("icons"), fresh_for);
        Fake { cache, catalog_calls, icon_calls, version, fail, _dir: dir }
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn opening_the_store_again_reuses_the_fetched_list() {
        let f = fake(Duration::from_secs(600));
        f.cache.catalog("beta", false).await.unwrap();
        f.cache.catalog("beta", false).await.unwrap();
        assert_eq!(f.catalog_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_button_and_channel_change_fetch_again() {
        let f = fake(Duration::from_secs(600));
        f.cache.catalog("beta", false).await.unwrap();
        f.cache.catalog("beta", true).await.unwrap();
        f.cache.catalog("stable", false).await.unwrap();
        assert_eq!(f.catalog_calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn an_old_list_answers_at_once_and_is_refreshed_behind() {
        let f = fake(Duration::ZERO);
        f.cache.catalog("beta", false).await.unwrap();
        *f.version.lock().unwrap() = "2.0.0".into();
        let first = f.cache.catalog("beta", false).await.unwrap().unwrap();
        assert_eq!(first.components["module-game"].version, "1.0.0");
        settle().await;
        let next = f.cache.catalog("beta", false).await.unwrap().unwrap();
        assert_eq!(next.components["module-game"].version, "2.0.0");
    }

    #[tokio::test]
    async fn a_failed_background_refresh_keeps_the_list() {
        let f = fake(Duration::ZERO);
        f.cache.catalog("beta", false).await.unwrap();
        f.fail.store(true, Ordering::SeqCst);
        f.cache.catalog("beta", false).await.unwrap();
        settle().await;
        assert!(f.cache.catalog("beta", false).await.unwrap().is_some());
        // 새로고침 버튼은 실패를 알린다
        assert!(f.cache.catalog("beta", true).await.is_err());
    }

    #[tokio::test]
    async fn icons_are_saved_and_served_from_disk() {
        let f = fake(Duration::from_secs(600));
        f.cache.catalog("beta", false).await.unwrap();
        settle().await; // 목록을 받으면 아이콘을 미리 받는다
        assert_eq!(f.icon_calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.cache.icon("module-game", "beta").await.unwrap().unwrap(), PNG);
        assert_eq!(f.icon_calls.load(Ordering::SeqCst), 1);
        // 새 판의 아이콘(다른 주소)이 오면 새로 받고 지난 파일은 지운다
        *f.version.lock().unwrap() = "2.0.0".into();
        f.cache.catalog("beta", true).await.unwrap();
        settle().await;
        assert_eq!(f.icon_calls.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read_dir(&f.cache.icons_dir).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn unknown_or_unsafe_keys_have_no_icon() {
        let f = fake(Duration::from_secs(600));
        assert!(f.cache.icon("module-none", "beta").await.unwrap().is_none());
        assert!(f.cache.icon("../module-game", "beta").await.unwrap().is_none());
        assert!(f.cache.icon("saba-core", "beta").await.unwrap().is_none());
    }

    #[test]
    fn list_meta_hides_the_icon_address() {
        let meta = serde_json::json!({"name": "Pet", "icon": "https://raw.example/icon.png"});
        let out = public_meta(Some(&meta));
        assert_eq!(out.get("has_icon"), Some(&Value::Bool(true)));
        assert!(!out.contains_key("icon"));
        assert_eq!(public_meta(None).get("has_icon"), Some(&Value::Bool(false)));
    }
}
