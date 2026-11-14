//! 익스텐션 번역·글로벌 설정과 켜짐 상태 저장

use super::*;

impl ExtensionManager {
    /// i18n JSON 로드
    pub fn load_i18n(&self, ext_id: &str, locale: &str) -> Option<Value> {
        if !crate::utils::is_single_path_component(locale) {
            return None;
        }
        let ext = self.discovered.get(ext_id)?;
        let i18n_dir = ext.manifest.i18n_dir.as_deref()?;
        let relative = std::path::Path::new(i18n_dir);
        if i18n_dir.is_empty() || relative.is_absolute() || !relative.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
            tracing::warn!("Extension '{}' i18n_dir must stay inside its folder; locales ignored", ext_id);
            return None;
        }
        let path = ext.dir.join(i18n_dir).join(format!("{}.json", locale));
        if !crate::module_paths::is_inside(&path, std::slice::from_ref(&ext.dir)) {
            tracing::warn!("Extension '{}' i18n_dir resolves outside its folder or through a link; locales ignored", ext_id);
            return None;
        }
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 특정 익스텐션의 글로벌 설정을 반환합니다.
    pub fn get_extension_config(&self, ext_id: &str) -> HashMap<String, Value> {
        self.extension_config
            .get(ext_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 특정 익스텐션의 글로벌 설정을 업데이트합니다.
    /// 기존 키를 머지(덮어쓰기)하며, 디스크에 즉시 영속화합니다. 값이 null인 키는 지워 기본값으로 돌린다.
    /// 저장에 실패하면 메모리 값도 되돌리고 오류를 돌려준다.
    pub fn set_extension_config(&mut self, ext_id: &str, values: HashMap<String, Value>) -> Result<()> {
        let previous = self.extension_config.get(ext_id).cloned();
        let entry = self.extension_config
            .entry(ext_id.to_string())
            .or_default();
        for (k, v) in values {
            if v.is_null() {
                entry.remove(&k);
            } else {
                entry.insert(k, v);
            }
        }
        let saved = self.save_extension_config();
        if saved.is_err() {
            match previous {
                Some(prev) => self.extension_config.insert(ext_id.to_string(), prev),
                None => self.extension_config.remove(ext_id),
            };
        }
        saved
    }

    /// `extensions` 네임스페이스의 한 키를 저장
    fn persist(&self, key: &str, value: Value) -> Result<()> {
        self.config_file.update(|doc| {
            let section = doc
                .entry("extensions".to_string())
                .or_insert_with(|| Value::Object(Default::default()));
            if !section.is_object() {
                *section = Value::Object(Default::default());
            }
            section.as_object_mut().expect("object").insert(key.to_string(), value);
        })
        .map(|_| ())
        .with_context(|| format!("Failed to save extension {}", key))
    }

    /// enabled 목록 영속화
    pub(super) fn save_state(&self) -> Result<()> {
        let mut list: Vec<&str> = self.enabled.iter().map(String::as_str).collect();
        list.sort_unstable();
        self.persist("enabled", serde_json::json!(list))
    }

    /// 익스텐션 글로벌 설정 영속화
    pub(super) fn save_extension_config(&self) -> Result<()> {
        let v = serde_json::to_value(&self.extension_config).context("Failed to serialize extension config")?;
        self.persist("settings", v)
    }

    /// 저장된 enabled 목록과 글로벌 설정 로드
    pub(super) fn load_persisted(&mut self) {
        let section = self.config_file.section("extensions");
        if let Some(list) = section.get("enabled") {
            match serde_json::from_value::<Vec<String>>(list.clone()) {
                Ok(list) => {
                    self.enabled = list.into_iter().collect();
                    tracing::info!("Loaded extension state: {} enabled", self.enabled.len());
                }
                Err(e) => tracing::warn!("Invalid extensions.enabled in config: {}", e),
            }
        }
        if let Some(settings) = section.get("settings") {
            match serde_json::from_value(settings.clone()) {
                Ok(config) => self.extension_config = config,
                Err(e) => tracing::warn!("Invalid extensions.settings in config: {}", e),
            }
        }
    }
}
