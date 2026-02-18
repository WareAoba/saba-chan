//! 범용 익스텐션 시스템
//!
//! `extensions/` 디렉토리의 서브디렉토리를 스캔하여 `manifest.json`을 파싱하고,
//! 데몬/서버 수명주기 Hook을 Python 모듈로 디스패치합니다.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ═══════════════════════════════════════════════════════════════
//  구조화된 에러 타입
// ═══════════════════════════════════════════════════════════════

/// 익스텐션 조작 시 발생할 수 있는 에러.
/// `error_code` 필드를 통해 GUI에서 종류별로 분기할 수 있음.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionError {
    /// 머신 판별용 코드 (예: "not_found", "dependency_missing", "dependency_not_enabled",
    /// "has_dependents", "in_use", "not_mounted", "id_mismatch", "manifest_not_found")
    pub error_code: String,
    /// 사람이 읽을 수 있는 메시지
    pub message: String,
    /// 관련 식별자 목록 (의존 익스텐션 ID, 인스턴스 이름 등)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExtensionError {}

impl ExtensionError {
    fn not_found(ext_id: &str) -> Self {
        Self {
            error_code: "not_found".to_string(),
            message: format!("Extension '{}' not found in discovered extensions", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    fn dependency_missing(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_missing".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not mounted", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    fn dependency_not_enabled(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_not_enabled".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not enabled", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    fn has_dependents(ext_id: &str, dependents: &[String]) -> Self {
        Self {
            error_code: "has_dependents".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': depended on by active extension(s): {}",
                ext_id,
                dependents.join(", ")
            ),
            related: dependents.to_vec(),
        }
    }
    fn in_use(ext_id: &str, instances: &[String]) -> Self {
        Self {
            error_code: "in_use".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': in use by instance(s): {}",
                ext_id,
                instances.join(", ")
            ),
            related: instances.to_vec(),
        }
    }
    fn not_mounted(ext_id: &str) -> Self {
        Self {
            error_code: "not_mounted".to_string(),
            message: format!("Extension '{}' is not mounted", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    fn manifest_not_found(path: &str) -> Self {
        Self {
            error_code: "manifest_not_found".to_string(),
            message: format!("Extension directory or manifest not found: {}", path),
            related: vec![],
        }
    }
    fn id_mismatch(manifest_id: &str, dir_name: &str) -> Self {
        Self {
            error_code: "id_mismatch".to_string(),
            message: format!(
                "Manifest id '{}' does not match directory name '{}'",
                manifest_id, dir_name
            ),
            related: vec![manifest_id.to_string(), dir_name.to_string()],
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Manifest 타입 정의
// ═══════════════════════════════════════════════════════════════

/// 익스텐션 매니페스트 — manifest.json을 역직렬화한 것
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub python_modules: HashMap<String, String>, // name → relative path
    #[serde(default)]
    pub hooks: HashMap<String, HookBinding>, // hook_name → binding
    #[serde(default)]
    pub gui: Option<GuiManifest>,
    /// 이 익스텐션이 관할하는 module.toml 섹션명 (예: "docker")
    #[serde(default)]
    pub module_config_section: Option<String>,
    #[serde(default)]
    pub instance_fields: HashMap<String, FieldDef>,
    #[serde(default)]
    pub i18n_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub module: String,   // python_modules의 키
    pub function: String, // Python 함수명
    #[serde(default)]
    pub condition: Option<String>, // "instance.ext_data.docker_enabled"
    #[serde(default, rename = "async")]
    pub is_async: Option<bool>, // true면 tokio::spawn으로 백그라운드 실행
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiManifest {
    pub bundle: String,
    #[serde(default)]
    pub styles: Option<String>,
    #[serde(default)]
    pub slots: HashMap<String, String>, // slot_id → component_name
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub optional: Option<bool>,
}

// ═══════════════════════════════════════════════════════════════
//  발견된 익스텐션
// ═══════════════════════════════════════════════════════════════

/// 발견된 익스텐션 정보 (manifest + 디렉토리 경로)
#[derive(Debug, Clone)]
pub struct DiscoveredExtension {
    pub manifest: ExtensionManifest,
    pub dir: PathBuf,
}

// ═══════════════════════════════════════════════════════════════
//  API 응답용 타입
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionListItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub hooks: Vec<String>,
    pub dependencies: Vec<String>,
    pub gui: Option<GuiManifest>,
    pub instance_fields: HashMap<String, FieldDef>,
}

// ═══════════════════════════════════════════════════════════════
//  ExtensionManager
// ═══════════════════════════════════════════════════════════════

pub struct ExtensionManager {
    extensions_dir: PathBuf,
    discovered: HashMap<String, DiscoveredExtension>,
    enabled: HashSet<String>,
    state_path: PathBuf,
}

#[allow(dead_code)]
impl ExtensionManager {
    /// 새 ExtensionManager 생성. `extensions_dir`은 `extensions/` 디렉토리 경로.
    pub fn new(extensions_dir: &str) -> Self {
        let extensions_dir = PathBuf::from(extensions_dir);

        // state_path: %APPDATA%/saba-chan/extensions_state.json
        let state_path = Self::resolve_state_path();

        let mut mgr = Self {
            extensions_dir,
            discovered: HashMap::new(),
            enabled: HashSet::new(),
            state_path,
        };
        mgr.load_state();
        mgr
    }

    /// %APPDATA%/saba-chan/extensions_state.json 경로 해석
    fn resolve_state_path() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata)
                .join("saba-chan")
                .join("extensions_state.json")
        } else {
            PathBuf::from("./extensions_state.json")
        }
    }

    /// extensions/ 서브디렉토리를 스캔하여 manifest.json이 있는 익스텐션 발견
    pub fn discover(&mut self) -> Result<Vec<String>> {
        let mut found = Vec::new();

        if !self.extensions_dir.is_dir() {
            tracing::warn!(
                "Extensions directory does not exist: {}",
                self.extensions_dir.display()
            );
            return Ok(found);
        }

        let entries = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| {
                format!(
                    "Failed to read extensions directory: {}",
                    self.extensions_dir.display()
                )
            })?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_manifest(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    tracing::info!(
                        "Discovered extension: {} v{} ({})",
                        manifest.name,
                        manifest.version,
                        id
                    );
                    self.discovered.insert(
                        id.clone(),
                        DiscoveredExtension {
                            manifest,
                            dir: path,
                        },
                    );
                    found.push(id);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load manifest from {}: {}",
                        manifest_path.display(),
                        e
                    );
                }
            }
        }

        tracing::info!("Extension discovery complete: {} found", found.len());
        Ok(found)
    }

    fn load_manifest(&self, path: &std::path::Path) -> Result<ExtensionManifest> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let manifest: ExtensionManifest = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    // ═══════════════════════════════════════════════════════════════
    //  동적 마운트/언마운트 (재시작 불필요)
    // ═══════════════════════════════════════════════════════════════

    /// 런타임 중 extensions/ 디렉토리를 재스캔하여 새로 추가된 익스텐션 발견.
    /// 기존에 이미 발견된 익스텐션은 매니페스트를 리로드(갱신),
    /// 디스크에서 제거된 (unmount 아닌) 익스텐션은 유지.
    pub fn rescan(&mut self) -> Result<Vec<String>> {
        let mut newly_found = Vec::new();

        if !self.extensions_dir.is_dir() {
            return Ok(newly_found);
        }

        let entries = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_manifest(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    let is_new = !self.discovered.contains_key(&id);
                    self.discovered.insert(
                        id.clone(),
                        DiscoveredExtension { manifest, dir: path },
                    );
                    if is_new {
                        tracing::info!("Rescan: newly discovered extension '{}'", id);
                        newly_found.push(id);
                    } else {
                        tracing::debug!("Rescan: reloaded manifest for '{}'", id);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load manifest {}: {}", manifest_path.display(), e);
                }
            }
        }

        tracing::info!("Rescan complete: {} new extension(s)", newly_found.len());
        Ok(newly_found)
    }

    /// 단일 익스텐션을 핫 마운트 (디스크에서 로드 → discovered에 추가).
    /// 이미 존재하면 매니페스트를 리로드.
    pub fn mount(&mut self, ext_id: &str) -> Result<()> {
        let ext_path = self.extensions_dir.join(ext_id);
        let manifest_path = ext_path.join("manifest.json");

        if !manifest_path.exists() {
            return Err(ExtensionError::manifest_not_found(
                &manifest_path.display().to_string(),
            ).into());
        }

        let manifest = self.load_manifest(&manifest_path)?;
        if manifest.id != ext_id {
            return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
        }

        tracing::info!("Mounted extension: {} v{}", manifest.name, manifest.version);
        self.discovered.insert(
            ext_id.to_string(),
            DiscoveredExtension { manifest, dir: ext_path },
        );
        Ok(())
    }

    /// 익스텐션 언마운트 (discovered + enabled에서 제거).
    /// 다른 활성 익스텐션이 이 익스텐션에 의존하면 실패.
    /// `active_ext_data`는 현재 존재하는 인스턴스들의 extension_data 목록 —
    /// 인스턴스가 이 익스텐션을 사용 중이면 실패.
    pub fn unmount(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_mounted(ext_id).into());
        }

        // 역의존성 검사: 다른 활성 익스텐션이 이 익스텐션을 dependency로 선언했는지
        let dependents = self.dependents_of(ext_id);
        if !dependents.is_empty() {
            return Err(ExtensionError::has_dependents(ext_id, &dependents).into());
        }

        // 인스턴스 사용 여부 검사
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }

        self.enabled.remove(ext_id);
        self.discovered.remove(ext_id);
        self.save_state();
        tracing::info!("Unmounted extension: {}", ext_id);
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    //  의존성 검증
    // ═══════════════════════════════════════════════════════════════

    /// ext_id를 dependency로 선언한 *활성* 익스텐션 목록
    fn dependents_of(&self, ext_id: &str) -> Vec<String> {
        self.discovered
            .values()
            .filter(|ext| {
                self.enabled.contains(&ext.manifest.id)
                    && ext.manifest.dependencies.iter().any(|d| d == ext_id)
            })
            .map(|ext| ext.manifest.id.clone())
            .collect()
    }

    /// 인스턴스의 extension_data에서 이 익스텐션이 선언한 instance_fields를
    /// 하나라도 truthy 값으로 사용하는 인스턴스 이름 목록.
    fn instances_using_ext(
        &self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Vec<String> {
        let ext = match self.discovered.get(ext_id) {
            Some(e) => e,
            None => return Vec::new(),
        };

        let field_keys: Vec<&String> = ext.manifest.instance_fields.keys().collect();
        if field_keys.is_empty() {
            return Vec::new();
        }

        active_ext_data
            .iter()
            .filter(|(_, data)| {
                field_keys.iter().any(|key| {
                    matches!(
                        data.get(*key),
                        Some(Value::Bool(true))
                            | Some(Value::String(_))
                            | Some(Value::Number(_))
                            | Some(Value::Object(_))
                            | Some(Value::Array(_))
                    )
                })
            })
            .map(|(name, _)| name.to_string())
            .collect()
    }

    /// 익스텐션 활성화 — 의존성 전부 discovered + enabled인지 검증
    pub fn enable(&mut self, ext_id: &str) -> Result<()> {
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_found(ext_id).into());
        }

        // 의존성 검증
        let deps = self.discovered[ext_id].manifest.dependencies.clone();
        for dep in &deps {
            if !self.discovered.contains_key(dep) {
                return Err(
                    ExtensionError::dependency_missing(ext_id, dep).into()
                );
            }
            if !self.enabled.contains(dep) {
                return Err(
                    ExtensionError::dependency_not_enabled(ext_id, dep).into()
                );
            }
        }

        self.enabled.insert(ext_id.to_string());
        self.save_state();
        tracing::info!("Extension enabled: {}", ext_id);
        Ok(())
    }

    /// 익스텐션 비활성화 — 역의존성 검사 + 인스턴스 사용 여부 검사
    pub fn disable(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        if !self.enabled.contains(ext_id) {
            // 이미 비활성 → no-op
            return Ok(());
        }

        // 역의존성 검사
        let dependents = self.dependents_of(ext_id);
        if !dependents.is_empty() {
            return Err(ExtensionError::has_dependents(ext_id, &dependents).into());
        }

        // 인스턴스 사용 여부 검사
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }

        self.enabled.remove(ext_id);
        self.save_state();
        tracing::info!("Extension disabled: {}", ext_id);
        Ok(())
    }

    /// 강제 비활성화 (인스턴스/의존성 무시) — 내부 마이그레이션/관리용
    pub fn force_disable(&mut self, ext_id: &str) {
        self.enabled.remove(ext_id);
        self.save_state();
        tracing::warn!("Extension force-disabled: {}", ext_id);
    }

    /// 활성 여부 확인
    pub fn is_enabled(&self, ext_id: &str) -> bool {
        self.enabled.contains(ext_id)
    }

    /// 발견된 전체 익스텐션 목록 (활성 상태 포함)
    pub fn list(&self) -> Vec<ExtensionListItem> {
        self.discovered
            .values()
            .map(|ext| {
                let m = &ext.manifest;
                ExtensionListItem {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.clone(),
                    description: m.description.clone(),
                    author: m.author.clone(),
                    enabled: self.enabled.contains(&m.id),
                    hooks: m.hooks.keys().cloned().collect(),
                    dependencies: m.dependencies.clone(),
                    gui: m.gui.clone(),
                    instance_fields: m.instance_fields.clone(),
                }
            })
            .collect()
    }

    /// 지정된 hook에 바인딩된 활성 익스텐션 목록
    pub fn hooks_for(&self, hook_name: &str) -> Vec<(&DiscoveredExtension, &HookBinding)> {
        let mut result = Vec::new();
        for ext in self.discovered.values() {
            if !self.enabled.contains(&ext.manifest.id) {
                continue;
            }
            if let Some(binding) = ext.manifest.hooks.get(hook_name) {
                result.push((ext, binding));
            }
        }
        result
    }

    /// 조건 문자열 평가: "instance.ext_data.<key>" → ext_data[key] == true
    pub fn evaluate_condition(
        condition: &str,
        ext_data: &HashMap<String, Value>,
    ) -> bool {
        // "instance.ext_data.<key>" 패턴
        if let Some(key) = condition.strip_prefix("instance.ext_data.") {
            match ext_data.get(key) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
                Some(Value::String(s)) => !s.is_empty(),
                _ => false,
            }
        } else {
            tracing::warn!("Unknown condition pattern: {}", condition);
            false
        }
    }

    /// Hook 디스패치: 조건 평가 → run_plugin 호출 → handled 체크
    ///
    /// 반환: Vec<(ext_id, Result<Value>)>
    /// handled=true가 나오면 이후 익스텐션은 스킵 (chain-of-responsibility)
    pub async fn dispatch_hook(
        &self,
        hook_name: &str,
        context: Value,
    ) -> Vec<(String, Result<Value>)> {
        let hooks = self.hooks_for(hook_name);
        if hooks.is_empty() {
            return Vec::new();
        }

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut results = Vec::new();

        for (ext, binding) in hooks {
            // 조건 평가
            if let Some(ref cond) = binding.condition {
                if !Self::evaluate_condition(cond, &ext_data) {
                    continue;
                }
            }

            // Python 모듈 절대 경로 해석
            let module_file = match ext.manifest.python_modules.get(&binding.module) {
                Some(rel_path) => ext.dir.join(rel_path),
                None => {
                    tracing::error!(
                        "Extension '{}' hook '{}' references unknown module '{}'",
                        ext.manifest.id,
                        hook_name,
                        binding.module
                    );
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!(
                            "Unknown python module: {}",
                            binding.module
                        )),
                    ));
                    continue;
                }
            };

            let module_path = module_file.to_string_lossy().to_string();
            tracing::debug!(
                "Dispatching hook '{}' → ext '{}' → {}::{}",
                hook_name,
                ext.manifest.id,
                binding.module,
                binding.function
            );

            let result = crate::plugin::run_plugin(
                &module_path,
                &binding.function,
                context.clone(),
            )
            .await;

            match &result {
                Ok(val) => {
                    tracing::debug!(
                        "Hook '{}' ext '{}' returned: {}",
                        hook_name,
                        ext.manifest.id,
                        serde_json::to_string(val).unwrap_or_default()
                    );
                    results.push((ext.manifest.id.clone(), Ok(val.clone())));

                    // handled=true → chain 종료
                    if val
                        .get("handled")
                        .and_then(|h| h.as_bool())
                        == Some(true)
                    {
                        tracing::debug!(
                            "Hook '{}' handled by extension '{}', skipping remaining",
                            hook_name,
                            ext.manifest.id
                        );
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Hook '{}' ext '{}' failed: {}",
                        hook_name,
                        ext.manifest.id,
                        e
                    );
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!("Hook dispatch failed: {}", e)),
                    ));
                    // 에러 시 graceful degradation — 기본 동작 진행을 위해 계속
                }
            }
        }

        results
    }

    /// Hook 디스패치 + 진행률 콜백 (server.post_create 등 장시간 hook용)
    pub async fn dispatch_hook_with_progress<F>(
        &self,
        hook_name: &str,
        context: Value,
        on_progress: F,
    ) -> Vec<(String, Result<Value>)>
    where
        F: Fn(crate::plugin::ExtensionProgress) + Send + 'static,
    {
        let hooks = self.hooks_for(hook_name);
        if hooks.is_empty() {
            return Vec::new();
        }

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut results = Vec::new();

        for (ext, binding) in hooks {
            if let Some(ref cond) = binding.condition {
                if !Self::evaluate_condition(cond, &ext_data) {
                    continue;
                }
            }

            let module_file = match ext.manifest.python_modules.get(&binding.module) {
                Some(rel_path) => ext.dir.join(rel_path),
                None => {
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!(
                            "Unknown python module: {}",
                            binding.module
                        )),
                    ));
                    continue;
                }
            };

            let module_path = module_file.to_string_lossy().to_string();

            let result = crate::plugin::run_plugin_with_progress(
                &module_path,
                &binding.function,
                context.clone(),
                on_progress,
            )
            .await;

            match &result {
                Ok(val) => {
                    results.push((ext.manifest.id.clone(), Ok(val.clone())));
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        break;
                    }
                }
                Err(e) => {
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!("Hook dispatch failed: {}", e)),
                    ));
                }
            }

            // progress 콜백은 한 번만 소비 가능하므로 첫 번째 익스텐션만 progress 지원
            break;
        }

        results
    }

    /// 해당 config 섹션명을 관할하는 활성 익스텐션이 있는지
    pub fn should_parse_config_section(&self, section: &str) -> bool {
        self.discovered.values().any(|ext| {
            self.enabled.contains(&ext.manifest.id)
                && ext.manifest.module_config_section.as_deref() == Some(section)
        })
    }

    /// 활성 익스텐션의 instance_fields를 합산
    pub fn all_instance_fields(&self) -> HashMap<String, FieldDef> {
        let mut fields = HashMap::new();
        for ext in self.discovered.values() {
            if self.enabled.contains(&ext.manifest.id) {
                for (k, v) in &ext.manifest.instance_fields {
                    fields.insert(k.clone(), v.clone());
                }
            }
        }
        fields
    }

    /// 활성 익스텐션의 GUI 매니페스트 목록
    pub fn gui_manifests(&self) -> Vec<(&str, &GuiManifest)> {
        self.discovered
            .values()
            .filter(|ext| self.enabled.contains(&ext.manifest.id))
            .filter_map(|ext| {
                ext.manifest
                    .gui
                    .as_ref()
                    .map(|gui| (ext.manifest.id.as_str(), gui))
            })
            .collect()
    }

    /// 익스텐션 파일 절대 경로
    pub fn extension_file_path(&self, ext_id: &str, relative: &str) -> Option<PathBuf> {
        self.discovered.get(ext_id).map(|ext| ext.dir.join(relative))
    }

    /// i18n JSON 로드
    pub fn load_i18n(&self, ext_id: &str, locale: &str) -> Option<Value> {
        let ext = self.discovered.get(ext_id)?;
        let i18n_dir = ext.manifest.i18n_dir.as_deref()?;
        let path = ext.dir.join(i18n_dir).join(format!("{}.json", locale));
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// enabled 목록 영속화
    fn save_state(&self) {
        let enabled_list: Vec<&str> = self.enabled.iter().map(|s| s.as_str()).collect();
        let json = match serde_json::to_string_pretty(&enabled_list) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize extension state: {}", e);
                return;
            }
        };

        if let Some(parent) = self.state_path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        if let Err(e) = std::fs::write(&self.state_path, json) {
            tracing::error!(
                "Failed to save extension state to {}: {}",
                self.state_path.display(),
                e
            );
        }
    }

    /// 저장된 enabled 목록 로드
    fn load_state(&mut self) {
        if !self.state_path.exists() {
            return;
        }
        match std::fs::read_to_string(&self.state_path) {
            Ok(content) => {
                match serde_json::from_str::<Vec<String>>(&content) {
                    Ok(list) => {
                        self.enabled = list.into_iter().collect();
                        tracing::info!(
                            "Loaded extension state: {} enabled",
                            self.enabled.len()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse extension state {}: {}",
                            self.state_path.display(),
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to read extension state {}: {}",
                    self.state_path.display(),
                    e
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_evaluate_condition_bool_true() {
        let mut ext_data = HashMap::new();
        ext_data.insert(
            "docker_enabled".to_string(),
            Value::Bool(true),
        );
        assert!(ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_bool_false() {
        let mut ext_data = HashMap::new();
        ext_data.insert(
            "docker_enabled".to_string(),
            Value::Bool(false),
        );
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_missing_key() {
        let ext_data = HashMap::new();
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_unknown_pattern() {
        let ext_data = HashMap::new();
        assert!(!ExtensionManager::evaluate_condition(
            "some.other.pattern",
            &ext_data
        ));
    }

    #[test]
    fn test_manifest_deserialization() {
        let json = json!({
            "id": "docker",
            "name": "Docker Isolation",
            "version": "1.0.0",
            "description": "Docker container isolation",
            "python_modules": {
                "compose_manager": "compose_manager.py"
            },
            "hooks": {
                "server.pre_start": {
                    "module": "compose_manager",
                    "function": "start",
                    "condition": "instance.ext_data.docker_enabled"
                }
            },
            "instance_fields": {
                "docker_enabled": { "type": "boolean", "default": false }
            }
        });

        let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "docker");
        assert_eq!(manifest.hooks.len(), 1);
        assert!(manifest.hooks.contains_key("server.pre_start"));
        assert_eq!(manifest.instance_fields.len(), 1);
    }

    #[test]
    fn test_extension_manager_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        let found = mgr.discover().unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn test_extension_manager_discover() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        let found = mgr.discover().unwrap();
        assert_eq!(found, vec!["test_ext"]);
        assert!(mgr.discovered.contains_key("test_ext"));
    }

    #[test]
    fn test_enable_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

        assert!(!mgr.is_enabled("test_ext"));
        mgr.enable("test_ext").unwrap();
        assert!(mgr.is_enabled("test_ext"));
        mgr.disable("test_ext", &no_instances).unwrap();
        assert!(!mgr.is_enabled("test_ext"));
    }

    #[test]
    fn test_enable_unknown_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        assert!(mgr.enable("nonexistent").is_err());
    }

    // ── 동적 마운트/언마운트 테스트 ──

    #[test]
    fn test_mount_unmount() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"my_ext","name":"My Extension","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        // 아직 discover 안 했으므로 비어 있음
        assert!(mgr.list().is_empty());

        // 핫 마운트
        mgr.mount("my_ext").unwrap();
        assert_eq!(mgr.list().len(), 1);

        // 핫 언마운트
        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        mgr.unmount("my_ext", &no_instances).unwrap();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn test_mount_bad_id_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        // 디렉토리 이름은 "wrong_dir"이지만 manifest id는 "correct_id"
        let ext_dir = tmp.path().join("wrong_dir");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"correct_id","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        let result = mgr.mount("wrong_dir");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }

    #[test]
    fn test_rescan_finds_new_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        assert!(mgr.list().is_empty());

        // 디스크에 새 익스텐션 추가
        let ext_dir = tmp.path().join("late_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"late_ext","name":"Late","version":"0.2.0"}"#,
        )
        .unwrap();

        let newly_found = mgr.rescan().unwrap();
        assert_eq!(newly_found, vec!["late_ext"]);
        assert_eq!(mgr.list().len(), 1);
    }

    // ── 의존성 검증 테스트 ──

    #[test]
    fn test_enable_with_missing_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let result = mgr.enable("child_ext");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parent_ext"));
    }

    #[test]
    fn test_enable_with_satisfied_dependency() {
        let tmp = tempfile::tempdir().unwrap();

        // parent_ext
        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        // child_ext depends on parent_ext
        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // parent 먼저 활성화
        mgr.enable("parent_ext").unwrap();
        // 이제 child 활성화 가능
        mgr.enable("child_ext").unwrap();
        assert!(mgr.is_enabled("child_ext"));
    }

    #[test]
    fn test_disable_blocked_by_dependent() {
        let tmp = tempfile::tempdir().unwrap();

        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("parent_ext").unwrap();
        mgr.enable("child_ext").unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

        // parent를 비활성화하려 하면 child가 의존하므로 실패
        let result = mgr.disable("parent_ext", &no_instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("child_ext"));
    }

    #[test]
    fn test_disable_blocked_by_instance_usage() {
        let tmp = tempfile::tempdir().unwrap();

        let ext_dir = tmp.path().join("docker");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"docker","name":"Docker","version":"1.0.0","instance_fields":{"docker_enabled":{"type":"boolean","default":false}}}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("docker").unwrap();

        // 인스턴스가 docker_enabled=true로 사용 중
        let mut ext_data = HashMap::new();
        ext_data.insert("docker_enabled".to_string(), Value::Bool(true));
        let instances: Vec<(&str, &HashMap<String, Value>)> =
            vec![("my_server", &ext_data)];

        let result = mgr.disable("docker", &instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("my_server"));
    }

    #[test]
    fn test_unmount_blocked_by_dependent() {
        let tmp = tempfile::tempdir().unwrap();

        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("parent_ext").unwrap();
        mgr.enable("child_ext").unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        let result = mgr.unmount("parent_ext", &no_instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("child_ext"));
    }

    #[test]
    fn test_force_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("test_ext").unwrap();
        assert!(mgr.is_enabled("test_ext"));

        mgr.force_disable("test_ext");
        assert!(!mgr.is_enabled("test_ext"));
    }
}
