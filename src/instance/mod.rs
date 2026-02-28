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
//!
//! 기존 `instances.json` 단일 파일이 있으면 자동 마이그레이션합니다.

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
        }
    }

    /// extension_data에 특정 boolean 플래그가 true인지 확인합니다.
    /// 예: `instance.ext_enabled("<ext>_enabled")` → 해당 확장 활성 여부
    pub fn ext_enabled(&self, key: &str) -> bool {
        self.extension_data.get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// extension_data에서 f64 값을 읽습니다.
    #[allow(dead_code)]
    pub fn ext_f64(&self, key: &str) -> Option<f64> {
        self.extension_data.get(key).and_then(|v| v.as_f64())
    }

    /// extension_data에서 문자열 값을 읽습니다.
    #[allow(dead_code)]
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
        changed
    }
}

/// 영어 대/소문자, 숫자로 이뤄진 8자 랜덤 비밀번호를 생성합니다.
/// UUID v4의 랜덤 바이트를 시드로 활용하여 외부 크레이트 없이 생성합니다.
pub fn generate_random_password() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let uuid_bytes = *uuid::Uuid::new_v4().as_bytes(); // 16 random bytes
    let mut password = String::with_capacity(8);
    for i in 0..8 {
        // 2바이트씩 조합하여 62로 나눈 나머지를 인덱스로 사용
        let idx = ((uuid_bytes[i * 2] as u16) << 8 | uuid_bytes[i * 2 + 1] as u16) as usize % CHARSET.len();
        password.push(CHARSET[idx] as char);
    }
    password
}

// ═══════════════════════════════════════════════════════════════
//  InstanceStore — 디렉토리 기반 저장소
// ═══════════════════════════════════════════════════════════════

pub struct InstanceStore {
    /// 인스턴스 루트 디렉토리 (예: %APPDATA%/saba-chan/instances/)
    root_dir: PathBuf,
    /// 기존 instances.json 경로 (마이그레이션용)
    legacy_json_path: Option<PathBuf>,
    /// 정렬된 인스턴스 목록 (메모리 캐시)
    instances: Vec<ServerInstance>,
    /// 정렬 순서
    order: Vec<String>,
}

impl InstanceStore {
    /// `file_path`는 기존 호환을 위해 instances.json 경로를 받되,
    /// 실제로는 같은 디렉토리의 `instances/` 하위 디렉토리를 사용합니다.
    pub fn new(file_path: &str) -> Self {
        let legacy_path = PathBuf::from(file_path);
        // instances.json의 부모 디렉토리 아래에 instances/ 디렉토리 사용
        let root_dir = legacy_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("instances");

        Self {
            root_dir,
            legacy_json_path: Some(legacy_path),
            instances: Vec::new(),
            order: Vec::new(),
        }
    }

    /// 인스턴스 로드 — 디렉토리 기반 로드, 필요 시 레거시 JSON에서 마이그레이션
    pub fn load(&mut self) -> Result<()> {
        // 루트 디렉토리 보장
        fs::create_dir_all(&self.root_dir)
            .with_context(|| format!("인스턴스 루트 생성 실패: {}", self.root_dir.display()))?;

        // ── 마이그레이션: 레거시 instances.json → 디렉토리 구조 ──
        if let Some(ref legacy_path) = self.legacy_json_path {
            if legacy_path.exists() {
                self.migrate_from_legacy_json(legacy_path)?;
            }
        }

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
                if !path.is_dir() {
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
        self.save_instance(&instance)?;
        self.order.push(instance.id.clone());
        self.save_order()?;
        self.instances.push(instance);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        // 디렉토리 삭제
        let dir = self.root_dir.join(id);
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
        for id in ordered_ids {
            if let Some(pos) = self.instances.iter().position(|i| i.id == *id) {
                reordered.push(self.instances[pos].clone());
            }
        }
        for inst in &self.instances {
            if !ordered_ids.contains(&inst.id) {
                reordered.push(inst.clone());
            }
        }
        self.order = reordered.iter().map(|i| i.id.clone()).collect();
        self.instances = reordered;
        self.save_order()?;
        Ok(())
    }

    // ── 하위 호환: 전체 저장 ──
    // 기존 코드에서 save()를 호출하는 곳이 있을 수 있으므로 유지
    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        for inst in &self.instances {
            self.save_instance(inst)?;
        }
        self.save_order()?;
        Ok(())
    }

    /// 인스턴스 디렉토리의 절대 경로를 반환합니다.
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.root_dir.join(id)
    }

    // ── Internal: 디렉토리 I/O ──────────────────────────────

    /// 단일 인스턴스를 디렉토리에 저장
    fn save_instance(&self, instance: &ServerInstance) -> Result<()> {
        let dir = self.root_dir.join(&instance.id);
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

        // ── 레거시 필드 마이그레이션: use_docker/docker_* → extension_data ──
        // (구 버전 instance.json에 존재하던 필드를 extension_data로 통합)
        let mut raw: serde_json::Value = serde_json::from_str(&content)?;
        let mut needs_write = false;

        // 레거시 필드 값을 먼저 읽어둠 (borrow 충돌 회피)
        let legacy_use_docker = raw.get("use_docker").and_then(|v| v.as_bool()).unwrap_or(false);
        let legacy_cpu = raw.get("docker_cpu_limit").and_then(|v| v.as_f64());
        let legacy_mem = raw.get("docker_memory_limit").and_then(|v| v.as_str()).map(String::from);
        let has_legacy = raw.get("use_docker").is_some()
            || raw.get("docker_cpu_limit").is_some()
            || raw.get("docker_memory_limit").is_some();

        if has_legacy {
            if let Some(obj) = raw.as_object_mut() {
                if legacy_use_docker {
                    let ext = obj.entry("extension_data")
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(ext_obj) = ext.as_object_mut() {
                        ext_obj.entry("docker_enabled".to_string())
                            .or_insert(serde_json::json!(true));
                        if let Some(cpu) = legacy_cpu {
                            ext_obj.entry("docker_cpu_limit".to_string())
                                .or_insert(serde_json::json!(cpu));
                        }
                        if let Some(mem) = legacy_mem {
                            ext_obj.entry("docker_memory_limit".to_string())
                                .or_insert(serde_json::json!(mem));
                        }
                    }
                    tracing::info!(
                        "Migrated legacy docker fields → extension_data for instance in {}",
                        dir.display()
                    );
                }
                obj.remove("use_docker");
                obj.remove("docker_cpu_limit");
                obj.remove("docker_memory_limit");
                needs_write = true;
            }
        }

        let mut instance: ServerInstance = serde_json::from_value(raw)?;

        // settings.json이 있으면 module_settings 머지
        let settings_path = dir.join("settings.json");
        if settings_path.exists() {
            let settings_content = fs::read_to_string(&settings_path)?;
            let settings: HashMap<String, serde_json::Value> =
                serde_json::from_str(&settings_content).unwrap_or_default();
            instance.module_settings = settings;
        }

        // 마이그레이션된 instance.json 저장 (레거시 필드 제거)
        if needs_write {
            // module_settings 제외하고 저장
            let mut meta = instance.clone();
            std::mem::take(&mut meta.module_settings);
            let migrated = serde_json::to_string_pretty(&meta)?;
            fs::write(&instance_path, &migrated)?;
        }

        Ok(instance)
    }

    /// order.json 저장
    fn save_order(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.order)?;
        fs::write(self.root_dir.join("order.json"), &content)?;
        Ok(())
    }

    // ── 마이그레이션 ────────────────────────────────────────

    /// 레거시 `instances.json`에서 디렉토리 구조로 마이그레이션
    fn migrate_from_legacy_json(&self, legacy_path: &Path) -> Result<()> {
        tracing::info!(
            "레거시 instances.json 발견, 디렉토리 구조로 마이그레이션: {}",
            legacy_path.display()
        );

        let content = fs::read_to_string(legacy_path)
            .with_context(|| format!("레거시 파일 읽기 실패: {}", legacy_path.display()))?;
        let instances: Vec<ServerInstance> = serde_json::from_str(&content)
            .with_context(|| "레거시 instances.json 파싱 실패")?;

        let mut order = Vec::with_capacity(instances.len());

        for instance in &instances {
            let dir = self.root_dir.join(&instance.id);
            // 이미 마이그레이션된 경우 건너뛰기
            if dir.join("instance.json").exists() {
                tracing::debug!("이미 마이그레이션됨: {} ({})", instance.name, instance.id);
                order.push(instance.id.clone());
                continue;
            }
            self.save_instance(instance)?;
            order.push(instance.id.clone());
            tracing::info!(
                "마이그레이션 완료: {} ({}) → {}",
                instance.name,
                instance.id,
                dir.display()
            );
        }

        // order.json 저장 (기존 order.json이 없는 경우만)
        let order_path = self.root_dir.join("order.json");
        if !order_path.exists() {
            let order_json = serde_json::to_string_pretty(&order)?;
            fs::write(&order_path, &order_json)?;
        }

        // 레거시 파일을 .bak으로 이동 (덮어쓰지 않음)
        let bak_path = legacy_path.with_extension("json.migrated");
        if !bak_path.exists() {
            fs::rename(legacy_path, &bak_path)?;
            tracing::info!(
                "레거시 파일 백업: {} → {}",
                legacy_path.display(),
                bak_path.display()
            );
        } else {
            // 이미 .migrated가 있으면 원본 삭제
            fs::remove_file(legacy_path)?;
            tracing::info!("레거시 파일 제거: {}", legacy_path.display());
        }

        tracing::info!(
            "마이그레이션 완료: {} 인스턴스 → {}",
            order.len(),
            self.root_dir.display()
        );
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
    fn test_add_and_load() {
        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("instances.json");

        let mut store = InstanceStore::new(json_path.to_str().unwrap());
        store.load().unwrap();
        assert_eq!(store.list().len(), 0);

        let inst = make_test_instance("test-server", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();
        assert_eq!(store.list().len(), 1);

        // 새 store로 다시 로드
        let mut store2 = InstanceStore::new(json_path.to_str().unwrap());
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
        let json_path = tmp.path().join("instances.json");

        let mut store = InstanceStore::new(json_path.to_str().unwrap());
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
        let json_path = tmp.path().join("instances.json");

        let mut store = InstanceStore::new(json_path.to_str().unwrap());
        store.load().unwrap();

        let inst = make_test_instance("updatable", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();

        let mut updated = store.get(&id).unwrap().clone();
        updated.name = "updated-name".to_string();
        updated.port = Some(9999);
        store.update(&id, updated).unwrap();

        // 리로드 확인
        let mut store2 = InstanceStore::new(json_path.to_str().unwrap());
        store2.load().unwrap();
        assert_eq!(store2.get(&id).unwrap().name, "updated-name");
        assert_eq!(store2.get(&id).unwrap().port, Some(9999));
    }

    #[test]
    fn test_reorder() {
        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("instances.json");

        let mut store = InstanceStore::new(json_path.to_str().unwrap());
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
        let mut store2 = InstanceStore::new(json_path.to_str().unwrap());
        store2.load().unwrap();
        assert_eq!(store2.list()[0].id, id_b);
        assert_eq!(store2.list()[1].id, id_a);
    }

    #[test]
    fn test_migrate_from_legacy_json() {
        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("instances.json");

        // 레거시 instances.json 생성
        let legacy_instances = vec![
            make_test_instance("legacy-mc", "minecraft"),
            make_test_instance("legacy-pw", "palworld"),
        ];
        let legacy_json = serde_json::to_string_pretty(&legacy_instances).unwrap();
        fs::write(&json_path, &legacy_json).unwrap();

        // 로드 → 자동 마이그레이션
        let mut store = InstanceStore::new(json_path.to_str().unwrap());
        store.load().unwrap();

        assert_eq!(store.list().len(), 2);
        assert_eq!(store.list()[0].name, "legacy-mc");
        assert_eq!(store.list()[1].name, "legacy-pw");

        // 레거시 파일이 .migrated로 이동됨
        assert!(!json_path.exists());
        assert!(json_path.with_extension("json.migrated").exists());

        // 디렉토리 구조 확인
        for inst in &legacy_instances {
            let dir = store.instance_dir(&inst.id);
            assert!(dir.join("instance.json").exists());
            assert!(dir.join("settings.json").exists());
        }
    }

    #[test]
    fn test_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("instances.json");

        let mut store = InstanceStore::new(json_path.to_str().unwrap());
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
}
