//! 디렉토리 기반 인스턴스 저장소
//!
//! 각 인스턴스는 자체 디렉토리를 가집니다:
//! ```text
//! <instances_root>/
//!   order.json              ← 인스턴스 정렬 순서
//!   <uuid>/
//!     instance.json          ← 인스턴스 메타데이터 (id, name, module, ports …)
//!     settings.json          ← 모듈별 동적 게임 설정
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ═══════════════════════════════════════════════════════════════
//  ServerInstance — 인스턴스 메타데이터
// ═══════════════════════════════════════════════════════════════

/// 서버 인스턴스 정의 — `instance.json`에 저장되는 메타데이터
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInstance {
    pub id: String,
    pub name: String,
    pub module_name: String,
    pub executable_path: Option<String>,
    pub working_dir: Option<String>,
    pub auto_detect: bool,
    pub process_name: Option<String>,
    /// 마지막으로 추적에 성공한 실제 프로세스의 exe 경로.
    /// auto-detect가 같은 process_name의 다중 인스턴스를 정확히 구분하는 데 사용된다.
    /// (java.exe처럼 인스턴스 간 공유되는 바이너리는 매칭에서 제외됨 — supervisor 참조)
    #[serde(default)]
    pub tracked_process_path: Option<String>,
    pub port: Option<u16>,
    pub rcon_port: Option<u16>,
    pub rcon_password: Option<String>,
    #[serde(default)]
    pub rest_host: Option<String>,
    #[serde(default)]
    pub rest_port: Option<u16>,
    #[serde(default)]
    pub rest_username: Option<String>,
    #[serde(default)]
    pub rest_password: Option<String>,
    #[serde(default = "default_protocol_mode")]
    pub protocol_mode: String,
    /// 모듈별 동적 게임 설정 — 런타임에는 settings.json에서 머지됩니다.
    #[serde(default)]
    pub module_settings: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub server_version: Option<String>,
    /// 범용 익스텐션 확장 데이터 — 각 익스텐션이 필요한 플래그/설정을 저장합니다.
    /// 예: { "<ext>_enabled": true, "<ext>_cpu_limit": 2.0, "<ext>_memory_limit": "4g" }
    #[serde(default)]
    pub extension_data: HashMap<String, serde_json::Value>,
    /// 이 인스턴스가 실행을 위해 요구하는 익스텐션 ID 목록.
    /// 여기에 선언된 익스텐션이 활성화되어 있지 않으면 시작이 차단됩니다.
    #[serde(default)]
    pub required_extensions: Vec<String>,
    /// 마이그레이션 등록 시 실제 게임 설정 파일 임포트에 실패했음을 표시.
    /// true인 동안 module_settings 캐시는 스키마 기본값일 수 있어 신뢰할 수 없다.
    /// 이후 live 임포트(read_live_settings)가 성공하면 캐시 갱신과 함께 해제된다.
    #[serde(default)]
    pub settings_import_pending: bool,
}

fn default_protocol_mode() -> String {
    "auto".to_string()
}

impl ServerInstance {
    pub fn new(name: &str, module_name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            module_name: module_name.to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: true,
            process_name: None,
            tracked_process_path: None,
            port: None,
            rcon_port: None,
            rcon_password: None,
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: HashMap::new(),
            server_version: None,
            extension_data: HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
        }
    }

    /// extension_data에 특정 boolean 플래그가 true인지 확인합니다.
    /// 예: `instance.ext_enabled("<ext>_enabled")` → 해당 확장 활성 여부
    pub fn ext_enabled(&self, key: &str) -> bool {
        self.extension_data.get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// extension_data에서 문자열 값을 읽습니다.
    pub fn ext_str(&self, key: &str) -> Option<&str> {
        self.extension_data.get(key).and_then(|v| v.as_str())
    }

    /// 이 인스턴스가 요구하는 익스텐션 중 `enabled_extensions`에 포함되지 않은 것을 반환합니다.
    /// 빈 Vec이면 모든 의존성이 충족된 것입니다.
    pub fn missing_required_extensions(&self, enabled_extensions: &std::collections::HashSet<String>) -> Vec<String> {
        self.required_extensions.iter()
            .filter(|ext_id| !enabled_extensions.contains(ext_id.as_str()))
            .cloned()
            .collect()
    }

    /// 모듈의 install 설정(method=steamcmd 등)을 extension_data에 주입합니다.
    /// 마이그레이션 및 신규 생성 시 호출되어, 이후 SteamCMD 훅이 올바르게 트리거되도록 합니다.
    /// 반환: 변경이 발생했으면 true
    pub fn apply_install_extension_data(
        &mut self,
        method: &str,
        app_id: Option<u32>,
        anonymous: bool,
        beta: Option<&str>,
        platform: Option<&str>,
    ) -> bool {
        if method != "steamcmd" {
            return false;
        }
        let mut changed = false;
        // install_method_steamcmd 플래그
        if !self.ext_enabled("install_method_steamcmd") {
            self.extension_data.insert(
                "install_method_steamcmd".to_string(),
                serde_json::json!(true),
            );
            changed = true;
        }
        // steamcmd_app_id
        if let Some(id) = app_id {
            if self.extension_data.get("steamcmd_app_id")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32) != Some(id)
            {
                self.extension_data.insert(
                    "steamcmd_app_id".to_string(),
                    serde_json::json!(id),
                );
                changed = true;
            }
        }
        // steamcmd_anonymous
        if self.extension_data.get("steamcmd_anonymous")
            .and_then(|v| v.as_bool()) != Some(anonymous)
        {
            self.extension_data.insert(
                "steamcmd_anonymous".to_string(),
                serde_json::json!(anonymous),
            );
            changed = true;
        }
        // steamcmd_beta (optional)
        if let Some(b) = beta {
            if self.ext_str("steamcmd_beta") != Some(b) {
                self.extension_data.insert(
                    "steamcmd_beta".to_string(),
                    serde_json::json!(b),
                );
                changed = true;
            }
        }
        // steamcmd_platform (optional)
        if let Some(p) = platform {
            if self.ext_str("steamcmd_platform") != Some(p) {
                self.extension_data.insert(
                    "steamcmd_platform".to_string(),
                    serde_json::json!(p),
                );
                changed = true;
            }
        }
        changed
    }

    /// RCON/REST 비밀번호가 비어있으면 랜덤 비밀번호로 채웁니다.
    /// 변경이 발생하면 true를 반환합니다.
    pub fn ensure_passwords(&mut self) -> bool {
        let mut changed = false;
        if self.rcon_password.as_deref().unwrap_or("").is_empty() {
            let pw = generate_random_password();
            tracing::info!("Auto-generated RCON password for instance {}", self.id);
            self.rcon_password = Some(pw);
            changed = true;
        }
        if self.rest_password.as_deref().unwrap_or("").is_empty() {
            let pw = generate_random_password();
            tracing::info!("Auto-generated REST password for instance {}", self.id);
            self.rest_password = Some(pw);
            changed = true;
        }
        // REST username은 항상 "admin"으로 고정
        if self.rest_username.as_deref() != Some("admin") {
            tracing::info!("Forced rest_username to 'admin' for instance {}", self.id);
            self.rest_username = Some("admin".to_string());
            changed = true;
        }
        changed
    }
}

/// 영어 대/소문자, 숫자로 이뤄진 16자 랜덤 비밀번호를 생성합니다.
/// `constants::generate_random_password()`에 위임합니다.
pub fn generate_random_password() -> String {
    saba_chan_updater_lib::constants::generate_random_password()
}

// ═══════════════════════════════════════════════════════════════
//  InstanceStore — 디렉토리 기반 저장소
// ═══════════════════════════════════════════════════════════════

pub struct InstanceStore {
    /// 인스턴스 루트 디렉토리 (예: %APPDATA%/saba-chan/instances/)
    root_dir: PathBuf,
    /// 정렬된 인스턴스 목록 (메모리 캐시)
    instances: Vec<ServerInstance>,
    /// 정렬 순서
    order: Vec<String>,
}

impl InstanceStore {
    /// `root_dir`은 인스턴스를 저장할 디렉토리 경로입니다.
    pub fn new(root_dir: &str) -> Self {
        Self {
            root_dir: PathBuf::from(root_dir),
            instances: Vec::new(),
            order: Vec::new(),
        }
    }

    /// 인스턴스 로드 — 디렉토리 기반 로드
    pub fn load(&mut self) -> Result<()> {
        // 루트 디렉토리 보장
        fs::create_dir_all(&self.root_dir)
            .with_context(|| format!("인스턴스 루트 생성 실패: {}", self.root_dir.display()))?;

        // ── order.json 로드 ──
        let order_path = self.root_dir.join("order.json");
        self.order = if order_path.exists() {
            let content = fs::read_to_string(&order_path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        // ── 각 인스턴스 디렉토리에서 로드 ──
        let mut loaded = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.root_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                    continue;
                }
                let instance_json = path.join("instance.json");
                if !instance_json.exists() {
                    continue;
                }
                match self.load_instance_from_dir(&path) {
                    Ok(inst) => loaded.push(inst),
                    Err(e) => {
                        tracing::warn!(
                            "인스턴스 로드 실패 ({}): {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        // ── order.json 기준 정렬 ──
        let mut sorted = Vec::with_capacity(loaded.len());
        for id in &self.order {
            if let Some(pos) = loaded.iter().position(|i| &i.id == id) {
                sorted.push(loaded.remove(pos));
            }
        }
        // order에 없는 인스턴스는 뒤에 추가
        sorted.extend(loaded);

        // order 동기화 (새 인스턴스 포함)
        self.order = sorted.iter().map(|i| i.id.clone()).collect();
        self.instances = sorted;

        tracing::info!(
            "Loaded {} instances from {}",
            self.instances.len(),
            self.root_dir.display()
        );
        Ok(())
    }

    // ── CRUD ────────────────────────────────────────────────

    pub fn list(&self) -> &[ServerInstance] {
        &self.instances
    }

    pub fn get(&self, id: &str) -> Option<&ServerInstance> {
        self.instances.iter().find(|i| i.id == id)
    }

    pub fn add(&mut self, instance: ServerInstance) -> Result<()> {
        anyhow::ensure!(self.get(&instance.id).is_none(), "Instance already exists: {}", instance.id);
        self.save_instance(&instance)?;
        self.order.push(instance.id.clone());
        self.save_order()?;
        self.instances.push(instance);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        // 디렉토리 삭제
        let dir = self.checked_instance_dir(id)?;
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("인스턴스 디렉토리 삭제 실패: {}", dir.display()))?;
        }
        self.instances.retain(|i| i.id != id);
        self.order.retain(|oid| oid != id);
        self.save_order()?;
        Ok(())
    }

    pub fn update(&mut self, id: &str, instance: ServerInstance) -> Result<()> {
        anyhow::ensure!(instance.id == id, "Instance ID cannot change: {}", id);
        let pos = self
            .instances
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", id))?;
        self.save_instance(&instance)?;
        self.instances[pos] = instance;
        Ok(())
    }

    pub fn reorder(&mut self, ordered_ids: &[String]) -> Result<()> {
        let mut reordered = Vec::with_capacity(self.instances.len());
        let mut included = std::collections::HashSet::new();
        for id in ordered_ids {
            if !included.insert(id.as_str()) {
                continue;
            }
            if let Some(pos) = self.instances.iter().position(|i| i.id == *id) {
                reordered.push(self.instances[pos].clone());
            }
        }
        for inst in &self.instances {
            if !included.contains(inst.id.as_str()) {
                reordered.push(inst.clone());
            }
        }
        self.order = reordered.iter().map(|i| i.id.clone()).collect();
        self.instances = reordered;
        self.save_order()?;
        Ok(())
    }

    /// 인스턴스 디렉토리의 절대 경로를 반환합니다.
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.root_dir.join(id)
    }

    fn checked_instance_dir(&self, id: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            crate::utils::is_single_path_component(id),
            "Invalid instance ID"
        );
        let dir = self.instance_dir(id);
        if let Ok(metadata) = fs::symlink_metadata(&dir) {
            anyhow::ensure!(!metadata.file_type().is_symlink(), "Instance directory cannot be a symbolic link");
        }
        Ok(dir)
    }

    // ── Internal: 디렉토리 I/O ──────────────────────────────

    /// 단일 인스턴스를 디렉토리에 저장
    fn save_instance(&self, instance: &ServerInstance) -> Result<()> {
        let dir = self.checked_instance_dir(&instance.id)?;
        fs::create_dir_all(&dir)?;

        // instance.json — 메타데이터 (module_settings 제외)
        let mut meta = instance.clone();
        let settings = std::mem::take(&mut meta.module_settings);
        let instance_json = serde_json::to_string_pretty(&meta)?;
        fs::write(dir.join("instance.json"), &instance_json)?;

        // settings.json — 모듈별 동적 설정 (비어있어도 저장)
        let settings_json = serde_json::to_string_pretty(&settings)?;
        fs::write(dir.join("settings.json"), &settings_json)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // instance.json에 민감 데이터(RCON 비밀번호 등) 포함 가능
            let _ = fs::set_permissions(dir.join("instance.json"), fs::Permissions::from_mode(0o600));
            let _ = fs::set_permissions(dir.join("settings.json"), fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// 디렉토리에서 인스턴스 로드
    fn load_instance_from_dir(&self, dir: &Path) -> Result<ServerInstance> {
        let instance_path = dir.join("instance.json");
        let content = fs::read_to_string(&instance_path)
            .with_context(|| format!("instance.json 읽기 실패: {}", instance_path.display()))?;

        let mut instance: ServerInstance = serde_json::from_str(&content)?;
        self.checked_instance_dir(&instance.id)?;
        anyhow::ensure!(
            dir.file_name().and_then(|name| name.to_str()) == Some(instance.id.as_str()),
            "Instance ID does not match its directory"
        );

        // settings.json이 있으면 module_settings 머지
        let settings_path = dir.join("settings.json");
        if settings_path.exists() {
            let settings_content = fs::read_to_string(&settings_path)?;
            let settings: HashMap<String, serde_json::Value> =
                serde_json::from_str(&settings_content).unwrap_or_default();
            instance.module_settings = settings;
        }

        Ok(instance)
    }

    /// order.json 저장
    fn save_order(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.order)?;
        fs::write(self.root_dir.join("order.json"), &content)?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_instance(name: &str, module: &str) -> ServerInstance {
        let mut inst = ServerInstance::new(name, module);
        inst.port = Some(25565);
        inst.module_settings.insert(
            "difficulty".to_string(),
            serde_json::json!("hard"),
        );
        inst
    }

    #[test]
    fn recovery_remove_rejects_paths_outside_instance_root() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        assert!(store.remove("../outside").is_err());
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn recovery_add_rejects_duplicate_and_unsafe_ids() {
        let tmp = TempDir::new().unwrap();
        let mut store = InstanceStore::new(tmp.path().to_str().unwrap());
        store.load().unwrap();
        let instance = make_test_instance("original", "minecraft");
        store.add(instance.clone()).unwrap();
        assert!(store.add(instance).is_err());

        for id in ["", ".", "..", "../outside", "..\\outside", "C:\\outside", "name:stream"] {
            let mut unsafe_instance = make_test_instance("unsafe", "minecraft");
            unsafe_instance.id = id.to_string();
            assert!(store.add(unsafe_instance).is_err(), "accepted ID: {id:?}");
        }
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn recovery_update_cannot_change_instance_identity() {
        let tmp = TempDir::new().unwrap();
        let mut store = InstanceStore::new(tmp.path().to_str().unwrap());
        store.load().unwrap();
        let original = make_test_instance("original", "minecraft");
        let id = original.id.clone();
        store.add(original.clone()).unwrap();
        let replacement = make_test_instance("replacement", "minecraft");

        assert!(store.update(&id, replacement.clone()).is_err());
        assert_eq!(store.get(&id).unwrap().name, original.name);
        assert!(!store.instance_dir(&replacement.id).exists());
    }

    #[test]
    fn recovery_reorder_keeps_each_instance_once() {
        let tmp = TempDir::new().unwrap();
        let mut store = InstanceStore::new(tmp.path().to_str().unwrap());
        store.load().unwrap();
        let first = make_test_instance("first", "minecraft");
        let second = make_test_instance("second", "palworld");
        let first_id = first.id.clone();
        let second_id = second.id.clone();
        store.add(first).unwrap();
        store.add(second).unwrap();

        store.reorder(&[second_id.clone(), second_id.clone()]).unwrap();
        assert_eq!(store.list().iter().map(|i| &i.id).collect::<Vec<_>>(), vec![&second_id, &first_id]);
    }

    #[test]
    fn recovery_load_skips_mismatched_directory_identity() {
        let tmp = TempDir::new().unwrap();
        let mut store = InstanceStore::new(tmp.path().to_str().unwrap());
        store.load().unwrap();
        let original = make_test_instance("original", "minecraft");
        let id = original.id.clone();
        store.add(original.clone()).unwrap();
        let mut invalid = original;
        invalid.id = "../outside".to_string();
        fs::write(store.instance_dir(&id).join("instance.json"), serde_json::to_vec(&invalid).unwrap()).unwrap();

        store.load().unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_settings_import_pending_defaults_false_for_legacy_json() {
        // 구버전 instance.json(필드 부재)을 로드해도 역직렬화가 실패하지 않고
        // settings_import_pending이 false여야 한다 (하위 호환).
        let legacy = r#"{
            "id": "legacy-id",
            "name": "legacy",
            "module_name": "minecraft",
            "executable_path": null,
            "working_dir": null,
            "auto_detect": true,
            "process_name": null,
            "port": null,
            "rcon_port": null,
            "rcon_password": null,
            "protocol_mode": "auto"
        }"#;
        let inst: ServerInstance = serde_json::from_str(legacy).unwrap();
        assert!(!inst.settings_import_pending);
    }

    #[test]
    fn test_settings_import_pending_roundtrip() {
        // 플래그 true로 저장 → 리로드 후에도 유지되어야 한다.
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let mut inst = make_test_instance("import-pending", "palworld");
        inst.settings_import_pending = true;
        let id = inst.id.clone();
        store.add(inst).unwrap();

        let mut store2 = InstanceStore::new(instances_dir.to_str().unwrap());
        store2.load().unwrap();
        assert!(store2.get(&id).unwrap().settings_import_pending);

        // false로 갱신(해제) → 리로드 후에도 false
        let mut cleared = store2.get(&id).unwrap().clone();
        cleared.settings_import_pending = false;
        store2.update(&id, cleared).unwrap();

        let mut store3 = InstanceStore::new(instances_dir.to_str().unwrap());
        store3.load().unwrap();
        assert!(!store3.get(&id).unwrap().settings_import_pending);
    }

    #[test]
    fn test_add_and_load() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        assert_eq!(store.list().len(), 0);

        let inst = make_test_instance("test-server", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();
        assert_eq!(store.list().len(), 1);

        // 새 store로 다시 로드
        let mut store2 = InstanceStore::new(instances_dir.to_str().unwrap());
        store2.load().unwrap();
        assert_eq!(store2.list().len(), 1);
        assert_eq!(store2.list()[0].id, id);
        assert_eq!(store2.list()[0].name, "test-server");
        // settings.json에서 module_settings 복원 확인
        assert_eq!(
            store2.list()[0].module_settings.get("difficulty"),
            Some(&serde_json::json!("hard"))
        );
    }

    #[test]
    fn test_remove() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let inst = make_test_instance("to-delete", "palworld");
        let id = inst.id.clone();
        store.add(inst).unwrap();
        assert_eq!(store.list().len(), 1);

        store.remove(&id).unwrap();
        assert_eq!(store.list().len(), 0);
        // 디렉토리도 삭제됨
        assert!(!store.instance_dir(&id).exists());
    }

    #[test]
    fn test_update() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let inst = make_test_instance("updatable", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();

        let mut updated = store.get(&id).unwrap().clone();
        updated.name = "updated-name".to_string();
        updated.port = Some(9999);
        store.update(&id, updated).unwrap();

        // 리로드 확인
        let mut store2 = InstanceStore::new(instances_dir.to_str().unwrap());
        store2.load().unwrap();
        assert_eq!(store2.get(&id).unwrap().name, "updated-name");
        assert_eq!(store2.get(&id).unwrap().port, Some(9999));
    }

    #[test]
    fn test_reorder() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let a = make_test_instance("server-a", "minecraft");
        let b = make_test_instance("server-b", "palworld");
        let id_a = a.id.clone();
        let id_b = b.id.clone();
        store.add(a).unwrap();
        store.add(b).unwrap();

        assert_eq!(store.list()[0].id, id_a);
        assert_eq!(store.list()[1].id, id_b);

        store.reorder(&[id_b.clone(), id_a.clone()]).unwrap();
        assert_eq!(store.list()[0].id, id_b);
        assert_eq!(store.list()[1].id, id_a);

        // 리로드 후에도 순서 유지
        let mut store2 = InstanceStore::new(instances_dir.to_str().unwrap());
        store2.load().unwrap();
        assert_eq!(store2.list()[0].id, id_b);
        assert_eq!(store2.list()[1].id, id_a);
    }

    #[test]
    fn test_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let mut inst = make_test_instance("struct-test", "minecraft");
        inst.module_settings.insert("ram".to_string(), serde_json::json!(4));
        inst.rcon_password = Some("secret123".to_string());
        let id = inst.id.clone();
        store.add(inst).unwrap();

        let dir = store.instance_dir(&id);

        // instance.json에 module_settings가 비어있어야 함 (분리됨)
        let meta_content = fs::read_to_string(dir.join("instance.json")).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta_content).unwrap();
        let ms = meta.get("module_settings")
            .and_then(|v| v.as_object())
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(ms, 0, "instance.json should not contain module_settings");

        // settings.json에 설정이 있어야 함
        let settings_content = fs::read_to_string(dir.join("settings.json")).unwrap();
        let settings: HashMap<String, serde_json::Value> =
            serde_json::from_str(&settings_content).unwrap();
        assert_eq!(settings.get("difficulty"), Some(&serde_json::json!("hard")));
        assert_eq!(settings.get("ram"), Some(&serde_json::json!(4)));
    }

    // ── 인스턴스→익스텐션 명시적 의존성(required_extensions) 테스트 ──

    #[test]
    fn test_required_extensions_all_satisfied() {
        let mut inst = ServerInstance::new("test", "minecraft");
        inst.required_extensions = vec!["docker".to_string(), "steamcmd".to_string()];

        let mut enabled = std::collections::HashSet::new();
        enabled.insert("docker".to_string());
        enabled.insert("steamcmd".to_string());
        enabled.insert("extra".to_string());

        let missing = inst.missing_required_extensions(&enabled);
        assert!(missing.is_empty(), "All required extensions are enabled");
    }

    #[test]
    fn test_required_extensions_some_missing() {
        let mut inst = ServerInstance::new("test", "minecraft");
        inst.required_extensions = vec!["docker".to_string(), "steamcmd".to_string()];

        let mut enabled = std::collections::HashSet::new();
        enabled.insert("docker".to_string());
        // steamcmd not enabled

        let missing = inst.missing_required_extensions(&enabled);
        assert_eq!(missing, vec!["steamcmd"]);
    }

    #[test]
    fn test_required_extensions_none_enabled() {
        let mut inst = ServerInstance::new("test", "minecraft");
        inst.required_extensions = vec!["docker".to_string(), "steamcmd".to_string()];

        let enabled = std::collections::HashSet::new();
        let missing = inst.missing_required_extensions(&enabled);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&"docker".to_string()));
        assert!(missing.contains(&"steamcmd".to_string()));
    }

    #[test]
    fn test_required_extensions_empty() {
        let inst = ServerInstance::new("test", "minecraft");
        // No required_extensions → always satisfied

        let enabled = std::collections::HashSet::new();
        let missing = inst.missing_required_extensions(&enabled);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_required_extensions_serialization() {
        let mut inst = ServerInstance::new("test", "minecraft");
        inst.required_extensions = vec!["docker".to_string()];

        let json = serde_json::to_string(&inst).unwrap();
        assert!(json.contains("required_extensions"));
        assert!(json.contains("docker"));

        let deserialized: ServerInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.required_extensions, vec!["docker"]);
    }

    #[test]
    fn test_required_extensions_default_empty_on_deserialize() {
        // JSON without required_extensions field
        let json = r#"{
            "id": "test-id", "name": "test", "module_name": "minecraft",
            "auto_detect": true, "protocol_mode": "auto"
        }"#;
        let inst: ServerInstance = serde_json::from_str(json).unwrap();
        assert!(inst.required_extensions.is_empty(), "Should default to empty");
    }

    /// 구버전 instance.json(tracked_process_path 필드 없음)도 로드되어야 하고,
    /// 저장 시에는 기록된 경로가 왕복(직렬화→역직렬화) 보존되어야 한다.
    #[test]
    fn test_tracked_process_path_roundtrip_and_default() {
        // 구버전 JSON → 기본값 None
        let json = r#"{
            "id": "test-id", "name": "test", "module_name": "minecraft",
            "auto_detect": true, "protocol_mode": "auto"
        }"#;
        let inst: ServerInstance = serde_json::from_str(json).unwrap();
        assert!(inst.tracked_process_path.is_none(), "Should default to None");

        // 기록 후 왕복 보존
        let mut inst = ServerInstance::new("test-pw", "palworld");
        inst.tracked_process_path =
            Some(r"D:\Servers\B\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe".to_string());
        let serialized = serde_json::to_string(&inst).unwrap();
        let restored: ServerInstance = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.tracked_process_path, inst.tracked_process_path);
    }

    // ── SteamCMD extension_data 자동 주입 테스트 ──

    #[test]
    fn test_apply_install_extension_data_steamcmd() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        assert!(inst.extension_data.is_empty());

        let changed = inst.apply_install_extension_data(
            "steamcmd", Some(2394010), true, None, Some("windows"),
        );
        assert!(changed, "Should report changes");
        assert!(inst.ext_enabled("install_method_steamcmd"));
        assert_eq!(
            inst.extension_data.get("steamcmd_app_id").and_then(|v| v.as_u64()),
            Some(2394010)
        );
        assert_eq!(
            inst.extension_data.get("steamcmd_anonymous").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(inst.ext_str("steamcmd_platform"), Some("windows"));
        assert!(inst.ext_str("steamcmd_beta").is_none());
    }

    #[test]
    fn test_apply_install_extension_data_steamcmd_with_beta() {
        let mut inst = ServerInstance::new("test-zb", "zomboid");
        let changed = inst.apply_install_extension_data(
            "steamcmd", Some(380870), true, Some("unstable"), None,
        );
        assert!(changed);
        assert!(inst.ext_enabled("install_method_steamcmd"));
        assert_eq!(inst.ext_str("steamcmd_beta"), Some("unstable"));
    }

    #[test]
    fn test_apply_install_extension_data_non_steamcmd_noop() {
        let mut inst = ServerInstance::new("test-mc", "minecraft");
        let changed = inst.apply_install_extension_data(
            "download", None, true, None, None,
        );
        assert!(!changed, "Non-steamcmd method should not change anything");
        assert!(inst.extension_data.is_empty());
    }

    #[test]
    fn test_apply_install_extension_data_idempotent() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        inst.apply_install_extension_data(
            "steamcmd", Some(2394010), true, None, Some("windows"),
        );

        // 같은 값으로 다시 호출 → 변경 없음
        let changed = inst.apply_install_extension_data(
            "steamcmd", Some(2394010), true, None, Some("windows"),
        );
        assert!(!changed, "Calling with same values should be idempotent");
    }

    #[test]
    fn test_apply_install_extension_data_preserves_existing() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        // 기존 extension_data에 docker_enabled 같은 다른 플래그가 있을 수 있음
        inst.extension_data.insert("docker_enabled".to_string(), serde_json::json!(true));

        inst.apply_install_extension_data(
            "steamcmd", Some(2394010), true, None, None,
        );

        // docker_enabled가 보존되어야 함
        assert!(inst.ext_enabled("docker_enabled"));
        assert!(inst.ext_enabled("install_method_steamcmd"));
    }

    #[test]
    fn test_apply_install_extension_data_serialization_roundtrip() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        inst.apply_install_extension_data(
            "steamcmd", Some(2394010), true, Some("beta-branch"), Some("windows"),
        );

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: ServerInstance = serde_json::from_str(&json).unwrap();

        assert!(deserialized.ext_enabled("install_method_steamcmd"));
        assert_eq!(
            deserialized.extension_data.get("steamcmd_app_id").and_then(|v| v.as_u64()),
            Some(2394010)
        );
        assert_eq!(deserialized.ext_str("steamcmd_beta"), Some("beta-branch"));
        assert_eq!(deserialized.ext_str("steamcmd_platform"), Some("windows"));
    }
}
