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
    /// Docker 모드 활성화 여부
    #[serde(default)]
    pub use_docker: bool,
    /// Docker CPU 제한 (코어 수, 예: 2.0)
    #[serde(default)]
    pub docker_cpu_limit: Option<f64>,
    /// Docker 메모리 제한 (예: "4g", "512m")
    #[serde(default)]
    pub docker_memory_limit: Option<String>,
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
            use_docker: false,
            docker_cpu_limit: None,
            docker_memory_limit: None,
        }
    }
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
        let mut instance: ServerInstance = serde_json::from_str(&content)?;

        // settings.json이 있으면 module_settings 머지
        let settings_path = dir.join("settings.json");
        if settings_path.exists() {
            let settings_content = fs::read_to_string(&settings_path)?;
            let settings: HashMap<String, serde_json::Value> =
                serde_json::from_str(&settings_content).unwrap_or_default();
            instance.module_settings = settings;
        }
        // instance.json 자체에 module_settings가 있을 수도 있음 (마이그레이션 직후)
        // settings.json이 우선

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
}
