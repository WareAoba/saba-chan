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

pub mod lock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 인스턴스 폴더 바로 아래에서 데몬이 쓰는 항목. 격리된 모듈에게 쓰기를 주지 않는다
/// (instance.json을 고칠 수 있으면 다음 로드 때 실행 파일·사용자 설정까지 바꾼다)
pub const DAEMON_ENTRIES: &[&str] = &["instance.json", "settings.json", "settings.json.tmp", "managed.pid", "run", "logs", "backups", "rcon.json", "rcon.json.tmp"];

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
    /// 예약 작업 (A3): 백업·재시작·공지·업데이트
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<crate::scheduler::ScheduledTask>,
    /// 자동 백업(예약·업데이트 전·복원 전)을 몇 개 남길지. 수동 백업은 세지 않는다. 기본 10
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_keep: Option<u32>,
    /// 설정 필드에 없는 추가 실행 인자. 모듈에 `extra_args`로 넘기고, 모듈이 넣지 않았으면
    /// (`extra_args_placed`가 없으면) 데몬이 실행 인자 끝에 붙인다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    /// 서버와 그 자식 프로세스의 메모리 상한(MiB). saba-runner가 Windows Job으로 막는다. 없으면 제한 없음
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u64>,
    /// 동의한 모듈 약관 (`[[agreements]]` id → 동의 시각 RFC3339). 선언된 약관을 모두 동의해야 켠다
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub accepted_agreements: std::collections::BTreeMap<String, String>,
    /// 사용자가 직접 정한 `module_settings` 키 (모듈 기본값·프리셋·게임 파일에서 읽은 값이 아닌 것).
    /// 실행 대상 검사·격리 실행이 사용자가 고른 런타임(`java_path`)만 인정하는 데 쓴다
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub user_settings: std::collections::BTreeSet<String>,
    /// 마이그레이션·시작 후 보충에서 import_settings가 판정한 경로형 설정 키. 값이 바뀌면 권한 근거를 지운다
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub imported_settings: std::collections::BTreeSet<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub data_roots_backfill_attempted: std::collections::BTreeSet<String>,
}

fn default_protocol_mode() -> String {
    "auto".to_string()
}

/// 데몬이 직접 처리하는 설치 방식. 그 밖의 방식은 같은 이름의 익스텐션이 맡는다
pub const BUILTIN_INSTALL_METHODS: &[&str] = &["manual", "download"];

/// 이 인스턴스가 `method` 익스텐션으로 설치됐다는 extension_data 키
pub fn install_method_key(method: &str) -> String {
    format!("install_method_{}", method)
}

impl ServerInstance {
    /// 설정 저장의 revision: 인스턴스 기록의 지문 (일정·백업 보관 수는 일정 API의 revision이 따로 본다)
    pub fn settings_revision(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        if let Some(map) = value.as_object_mut() {
            map.remove("schedule");
            map.remove("backup_keep");
        }
        crate::config_store::patch::revision(&value)
    }

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
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: HashMap::new(),
            server_version: None,
            extension_data: HashMap::new(),
            required_extensions: Vec::new(),
            schedule: Vec::new(),
            backup_keep: None,
            extra_args: Vec::new(),
            memory_limit_mb: None,
            accepted_agreements: Default::default(),
            user_settings: Default::default(),
            imported_settings: Default::default(),
            data_roots_backfill_attempted: Default::default(),
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
    #[cfg(test)]
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

    /// 모듈 `[install]` 설정을 설치 방식 익스텐션이 읽는 extension_data로 옮긴다.
    ///
    /// `manual`·`download`(데몬이 직접 처리) 밖의 방식은 같은 이름의 익스텐션이 맡는다.
    /// 키 규칙: `install_method_<방식>` = true, 설정 값마다 `<방식>_<이름>`
    /// (예: steamcmd → `install_method_steamcmd`, `steamcmd_app_id`). 익스텐션 manifest의
    /// `instance_fields`와 hook `condition`이 이 이름을 쓴다.
    /// 반환: 바뀐 것이 있으면 true
    pub fn apply_install_extension_data(&mut self, method: &str, fields: &[(&str, serde_json::Value)]) -> bool {
        if BUILTIN_INSTALL_METHODS.contains(&method) || method.is_empty() {
            return false;
        }
        let mut changed = false;
        let mut set = |key: String, value: serde_json::Value| {
            if self.extension_data.get(&key) != Some(&value) {
                self.extension_data.insert(key, value);
                changed = true;
            }
        };
        set(install_method_key(method), serde_json::json!(true));
        for (name, value) in fields {
            set(format!("{}_{}", method, name), value.clone());
        }
        changed
    }

    /// 게임 설정 파일에서 읽어 온 값(`import_settings` 결과)을 반영한다: `module_settings`에 넣고,
    /// 같은 값을 가리키는 인스턴스 필드(포트·REST 비밀번호)도 맞춘다. 안 그러면 인스턴스 필드가
    /// 옛 값으로 남아 다음 저장에서 파일을 되돌린다.
    /// `credential_map`: 데몬 키(`rest_password` 등) → 게임 키(`AdminPassword` 등)
    /// `types`: 설정 이름 → module.toml 필드 타입. 값은 그 타입으로 저장한다(문자열 필드는 문자열 그대로)
    pub fn apply_imported_settings(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        credential_map: &std::collections::HashMap<String, String>,
        types: &std::collections::HashMap<String, String>,
    ) {
        for (key, value) in settings {
            let typed = coerce_setting_value(value, types.get(key).map(String::as_str));
            if self.module_settings.get(key) != Some(&typed) {
                self.imported_settings.remove(key);
                self.user_settings.remove(key);
            }
            self.module_settings.insert(key.clone(), typed);
        }
        // 인스턴스 필드는 이번에 읽은 키로만 맞춘다 (캐시에 남은 옛 값으로 되돌리지 않는다)
        let port = |key: &str| {
            settings.get(key).and(self.module_settings.get(key)).and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_u64(),
                serde_json::Value::String(s) => s.trim().parse().ok(),
                _ => None,
            }).and_then(|p| u16::try_from(p).ok())
        };
        if let Some(p) = port("port") { self.port = Some(p); }
        if let Some(p) = port("rest_port") { self.rest_port = Some(p); }
        {
            let daemon_key = "rest_password";
            let game_key = credential_map.get(daemon_key).map(String::as_str).unwrap_or(daemon_key);
            let value = self.module_settings.iter()
                .find(|(k, _)| settings.contains_key(k.as_str()) && (k.eq_ignore_ascii_case(game_key) || k.as_str() == daemon_key))
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string);
            // 파일이 비어 있으면 비어 있는 것이 사실이다: 옛 캐시 값을 남기지 않는다
            if let Some(pwd) = value {
                self.rest_password = Some(pwd);
            }
        }
    }

    /// REST 비밀번호가 비어있으면 랜덤 비밀번호로 채웁니다.
    /// 변경이 발생하면 true를 반환합니다.
    /// `generate`: 빈 REST 비밀번호를 채우는가 (GUI "비밀번호 자동 생성")
    pub fn ensure_passwords(&mut self, generate: bool) -> bool {
        let mut changed = false;
        if generate && self.rest_password.as_deref().unwrap_or("").is_empty() {
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
                // 링크(정션 포함)는 따라가지 않는다: 저장·삭제가 다른 곳의 폴더를 건드린다
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let instance_json = path.join("instance.json");
                if !instance_json.exists() {
                    continue;
                }
                match self.load_instance_from_dir(&path) {
                    // 폴더 이름과 id가 다르면 저장·삭제가 다른 폴더를 건드린다: 건너뛴다
                    Ok(inst) if path.file_name().is_some_and(|n| n == inst.id.as_str()) => loaded.push(inst),
                    Ok(inst) => tracing::warn!(
                        "인스턴스 건너뜀: 폴더 이름 {} 과 id {} 가 다름",
                        path.display(),
                        inst.id
                    ),
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
        let dir = self.checked_dir(id)?;
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("인스턴스 디렉토리 삭제 실패: {}", dir.display()))?;
        }
        self.instances.retain(|i| i.id != id);
        self.order.retain(|oid| oid != id);
        self.save_order()?;
        Ok(())
    }

    /// 저장하지 않고 메모리의 인스턴스를 고친다 (읽을 때의 값 옮기기. 파일에는 다음 저장 때 쓴다)
    pub fn adjust_in_memory(&mut self, mut adjust: impl FnMut(&mut ServerInstance)) {
        self.instances.iter_mut().for_each(&mut adjust);
    }

    pub fn update(&mut self, id: &str, instance: ServerInstance) -> Result<()> {
        let pos = self
            .instances
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", id))?;
        if instance.id != id {
            anyhow::bail!("instance id cannot change ({} -> {})", id, instance.id);
        }
        self.save_instance(&instance)?;
        self.instances[pos] = instance;
        Ok(())
    }

    pub fn reorder(&mut self, ordered_ids: &[String]) -> Result<()> {
        let mut reordered: Vec<ServerInstance> = Vec::with_capacity(self.instances.len());
        for id in ordered_ids {
            // 같은 id가 두 번 와도 인스턴스는 한 번만
            if reordered.iter().any(|i| i.id == *id) {
                continue;
            }
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


    /// 인스턴스 디렉토리의 절대 경로를 반환합니다.
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.root_dir.join(id)
    }

    /// `dir`과 겹치는 폴더(서버 폴더나 인스턴스 폴더, 그 안이나 바깥)를 쓰는 다른 인스턴스.
    /// 한 폴더에 서버 둘을 두면 서로의 파일을 덮어쓰고, 한쪽의 복원·삭제가 다른 쪽 파일까지 치운다
    pub fn folder_user(&self, dir: &Path, except: &str) -> Option<&ServerInstance> {
        self.instances.iter().find(|i| {
            i.id != except
                && (i.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::paths_overlap(Path::new(d), dir))
                    || saba_chan_updater_lib::paths_overlap(&self.instance_dir(&i.id), dir))
        })
    }

    /// 쓰거나 지울 인스턴스 폴더: id가 경로 한 칸이고, 폴더가 심볼릭 링크가 아니어야 한다
    fn checked_dir(&self, id: &str) -> Result<PathBuf> {
        if !crate::utils::is_single_path_component(id) {
            anyhow::bail!("invalid instance id: {:?}", id);
        }
        let dir = self.root_dir.join(id);
        if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!("instance directory is a symlink: {}", dir.display());
        }
        Ok(dir)
    }

    // ── Internal: 디렉토리 I/O ──────────────────────────────

    /// 단일 인스턴스를 디렉토리에 저장
    fn save_instance(&self, instance: &ServerInstance) -> Result<()> {
        let dir = self.checked_dir(&instance.id)?;
        fs::create_dir_all(&dir)?;

        // instance.json — 메타데이터 (module_settings 제외)
        let mut meta = instance.clone();
        let settings = std::mem::take(&mut meta.module_settings);
        let instance_json = serde_json::to_string_pretty(&meta)?;
        let settings_json = serde_json::to_string_pretty(&settings)?;
        let instance_path = dir.join("instance.json");
        // 없을 때만 새 파일이다: 못 읽은 것을 없는 것으로 보면 되돌릴 때 기존 파일을 지운다
        let previous = match fs::read(&instance_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(anyhow::Error::from(e).context(format!("read {} before saving", instance_path.display()))),
        };
        write_atomic(&instance_path, &instance_json)?;

        // settings.json — 모듈별 동적 설정 (비어있어도 저장). 못 쓰면 instance.json도 되돌려 반쪽 저장을 남기지 않는다
        if let Err(e) = write_atomic(&dir.join("settings.json"), &settings_json) {
            let undone = match &previous {
                Some(old) => write_atomic_bytes(&instance_path, old),
                None => fs::remove_file(&instance_path).map_err(Into::into),
            };
            if let Err(u) = undone {
                return Err(e.context(format!("instance.json could not be put back either: {:#}", u)));
            }
            return Err(e);
        }

        Ok(())
    }

    /// 디렉토리에서 인스턴스 로드
    fn load_instance_from_dir(&self, dir: &Path) -> Result<ServerInstance> {
        let instance_path = dir.join("instance.json");
        let content = fs::read_to_string(&instance_path)
            .with_context(|| format!("instance.json 읽기 실패: {}", instance_path.display()))?;

        let mut instance: ServerInstance = serde_json::from_str(&content)?;
        let mut stale = serde_json::from_str::<serde_json::Value>(&content)
            .ok()
            .and_then(|v| v.as_object().map(|o| LEGACY_RCON_KEYS.iter().any(|k| o.contains_key(*k))))
            .unwrap_or(false);

        // settings.json이 있으면 module_settings 머지
        let settings_path = dir.join("settings.json");
        if settings_path.exists() {
            let settings_content = fs::read_to_string(&settings_path)?;
            let mut settings: HashMap<String, serde_json::Value> = match serde_json::from_str(&settings_content) {
                Ok(s) => s,
                Err(e) => {
                    // 다음 저장이 빈 설정으로 덮기 전에 원본을 남긴다. 남기지 못하면 이 인스턴스를 올리지 않는다
                    let Some(kept) = keep_corrupt_copy(&settings_path, &settings_content) else {
                        anyhow::bail!("settings.json is not valid JSON ({}) and could not be copied aside; fix or move it", e);
                    };
                    tracing::warn!(
                        "settings.json of instance '{}' is not valid JSON ({}); starting with empty settings, original kept as {}",
                        instance.name,
                        e,
                        kept.display()
                    );
                    HashMap::new()
                }
            };
            stale |= drop_legacy_rcon_keys(&mut settings);
            instance.module_settings = settings;
        }

        // RCON을 없애기 전에 만든 인스턴스: 남은 RCON 키를 한 번 지우고 다시 쓴다
        if stale {
            match self.save_instance(&instance) {
                Ok(()) => tracing::info!("Removed leftover RCON settings from instance '{}'", instance.name),
                Err(e) => tracing::warn!("Could not clean RCON settings of '{}': {}", instance.name, e),
            }
        }

        Ok(instance)
    }

    /// order.json 저장
    fn save_order(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.order)?;
        saba_atomic_file::write(&self.root_dir.join("order.json"), content.as_bytes(), saba_atomic_file::Mode::Plain)?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

/// RCON은 보안상 쓰지 않는다 (2026-10-17). 그 전에 만든 인스턴스 파일에 남은 키
const LEGACY_RCON_KEYS: &[&str] = &[
    "rcon_port",
    "rcon_password",
    "enable_rcon",
    "broadcast_rcon_to_ops",
    "rcon.port",
    "rcon.password",
    "enable-rcon",
    "broadcast-rcon-to-ops",
];

/// 깨진 설정 파일의 사본 `<이름>.corrupt`(이미 있으면 `<이름>.corrupt-<초>`). 같은 내용의 사본이 이미 있으면 그 경로
fn keep_corrupt_copy(path: &Path, content: &str) -> Option<PathBuf> {
    let base = path.file_name()?.to_string_lossy().to_string() + ".corrupt";
    let dir = path.parent()?;
    let same = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&base)) && fs::read_to_string(p).is_ok_and(|c| c == content)
    });
    if same.is_some() {
        return same;
    }
    let mut target = dir.join(&base);
    if target.exists() {
        target = dir.join(format!("{}-{}", base, chrono::Utc::now().timestamp()));
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 원본처럼 비밀번호가 들어 있을 수 있다: 처음부터 주인만 읽게 만든다
        options.mode(0o600);
    }
    let mut file = options.open(&target).ok()?;
    std::io::Write::write_all(&mut file, content.as_bytes()).ok()?;
    Some(target)
}

/// 남은 RCON 키를 지운다. 하나라도 지웠으면 true
fn drop_legacy_rcon_keys(settings: &mut HashMap<String, serde_json::Value>) -> bool {
    let before = settings.len();
    settings.retain(|k, _| !LEGACY_RCON_KEYS.contains(&k.as_str()));
    settings.len() != before
}

/// 임시 파일에 쓰고 rename: 쓰는 도중 꺼져도 예전 파일이나 새 파일 중 하나가 온전히 남는다.
/// instance.json·settings.json에 비밀번호가 들어 있을 수 있어 주인만 읽게 쓴다
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    write_atomic_bytes(path, content.as_bytes())
}

fn write_atomic_bytes(path: &Path, content: &[u8]) -> Result<()> {
    saba_atomic_file::write(path, content, saba_atomic_file::Mode::Secret)?;
    Ok(())
}

/// 설정 파일에서 읽은 값을 module.toml 필드 타입에 맞춘다.
/// number·boolean은 문자열이면 바꾸고, 그 밖의 알려진 타입(text, password, select…)은 문자열로 둔다
/// ("000123" 같은 비밀번호가 숫자로 바뀌지 않게). 스키마에 없는 키는 그대로 둔다
pub fn coerce_setting_value(value: &serde_json::Value, field_type: Option<&str>) -> serde_json::Value {
    use serde_json::Value;
    match (field_type, value) {
        (None, v) => v.clone(),
        (Some("number"), Value::String(s)) => {
            let t = s.trim();
            t.parse::<i64>().map(Value::from)
                .or_else(|_| t.parse::<f64>().map_err(|_| ()).and_then(|f| serde_json::Number::from_f64(f).map(Value::Number).ok_or(())))
                .unwrap_or_else(|_| value.clone())
        }
        (Some("boolean"), Value::String(s)) if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") => {
            Value::Bool(s.eq_ignore_ascii_case("true"))
        }
        (Some("number" | "boolean"), v) => v.clone(),
        (Some(_), Value::Number(n)) => Value::String(n.to_string()),
        (Some(_), Value::Bool(b)) => Value::String(b.to_string()),
        (Some(_), v) => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn contract_paths_reading_the_same_selected_path_keeps_its_authorization() {
        let mut instance = super::ServerInstance::new("g", "g");
        instance.module_settings.insert("data_home".into(), serde_json::json!("/selected"));
        instance.user_settings.insert("data_home".into());
        let types = std::collections::HashMap::from([("data_home".to_string(), "file".to_string())]);
        instance.apply_imported_settings(serde_json::json!({"data_home":"/selected"}).as_object().unwrap(), &Default::default(), &types);
        assert!(instance.user_settings.contains("data_home"));
        instance.imported_settings.insert("data_home".into());
        instance.apply_imported_settings(serde_json::json!({"data_home":"/changed"}).as_object().unwrap(), &Default::default(), &types);
        assert!(!instance.user_settings.contains("data_home"));
        assert!(!instance.imported_settings.contains("data_home"));
    }
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
    fn imported_settings_refresh_instance_fields() {
        let mut inst = ServerInstance::new("a", "palworld");
        inst.port = Some(27015);
        let settings = serde_json::json!({"port": "28015", "AdminPassword": "pw", "Other": 1});
        let map: std::collections::HashMap<String, String> = [("rest_password".to_string(), "adminpassword".to_string())].into();
        let types: std::collections::HashMap<String, String> =
            [("port", "number"), ("AdminPassword", "password")].map(|(k, t)| (k.to_string(), t.to_string())).into();
        inst.apply_imported_settings(settings.as_object().unwrap(), &map, &types);
        let raw = serde_json::json!({"AdminPassword": "000123"});
        inst.port = Some(30000); // 사용자가 포트만 바꾼 뒤
        inst.apply_imported_settings(raw.as_object().unwrap(), &map, &types);
        assert_eq!(inst.port, Some(30000), "a password-only import must not revert the port");
        inst.port = Some(28015);
        assert_eq!(inst.module_settings.get("AdminPassword"), Some(&serde_json::json!("000123")));
        assert_eq!(inst.rest_password.as_deref(), Some("000123"));
        inst.apply_imported_settings(serde_json::json!({"AdminPassword": "pw"}).as_object().unwrap(), &map, &types);
        assert_eq!(inst.port, Some(28015));
        assert_eq!(inst.rest_password.as_deref(), Some("pw"));
        assert_eq!(inst.module_settings.get("Other"), Some(&serde_json::json!(1)));
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

    // 백로그 L17d: 인스턴스 폴더 자리에 놓인 링크를 따라가 다른 곳의 폴더를 인스턴스로 불러왔다.
    // 저장·삭제가 그 폴더를 건드린다
    #[cfg(unix)]
    #[test]
    fn a_linked_instance_folder_is_not_loaded() {
        let tmp = TempDir::new().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        let mut other = InstanceStore::new(elsewhere.to_str().unwrap());
        other.load().unwrap();
        let inst = make_test_instance("linked", "minecraft");
        let id = inst.id.clone();
        other.add(inst).unwrap();

        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        std::os::unix::fs::symlink(elsewhere.join(&id), instances_dir.join(&id)).unwrap();
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        assert!(store.list().is_empty(), "the link was followed");
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
    fn a_failed_settings_write_leaves_the_instance_files_as_they_were() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let inst = make_test_instance("before", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();

        // settings.json을 쓸 수 없게 한다 (폴더 자리에는 파일로 바꿔 넣지 못한다)
        let settings = instances_dir.join(&id).join("settings.json");
        let original = fs::read(&settings).unwrap();
        fs::remove_file(&settings).unwrap();
        fs::create_dir_all(settings.join("x")).unwrap();
        let mut changed = store.get(&id).unwrap().clone();
        changed.name = "after".to_string();
        changed.module_settings.insert("difficulty".into(), serde_json::json!("easy"));
        assert!(store.update(&id, changed).is_err());
        fs::remove_dir_all(&settings).unwrap();
        fs::write(&settings, original).unwrap();

        let mut reloaded = InstanceStore::new(instances_dir.to_str().unwrap());
        reloaded.load().unwrap();
        let back = reloaded.get(&id).unwrap();
        assert_eq!(back.name, "before", "instance.json must not be half-updated");
        assert_eq!(back.module_settings.get("difficulty"), Some(&serde_json::json!("hard")));
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
    fn leftover_rcon_keys_are_removed_once_on_load() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let inst = make_test_instance("old", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();

        // RCON을 없애기 전의 파일처럼 만든다
        let dir = store.instance_dir(&id);
        let mut meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("instance.json")).unwrap()).unwrap();
        meta["rcon_port"] = serde_json::json!(25575);
        meta["rcon_password"] = serde_json::json!("pw");
        fs::write(dir.join("instance.json"), meta.to_string()).unwrap();
        fs::write(
            dir.join("settings.json"),
            r#"{"rcon_port":25575,"rcon.port":"25575","rcon.password":"pw","enable-rcon":"false","motd":"hi"}"#,
        )
        .unwrap();

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let loaded = store.get(&id).unwrap();
        assert_eq!(loaded.module_settings.keys().collect::<Vec<_>>(), vec!["motd"]);
        let settings: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings, serde_json::json!({ "motd": "hi" }));
        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("instance.json")).unwrap()).unwrap();
        assert!(meta.get("rcon_port").is_none() && meta.get("rcon_password").is_none());
        let temps: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(temps.is_empty(), "no temp file left: {temps:?}");
    }

    /// 손으로 고치다 깨진 settings.json은 다음 저장이 덮기 전에 사본으로 남긴다 (D-S5)
    #[test]
    fn a_broken_settings_file_is_kept_before_anything_overwrites_it() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let inst = make_test_instance("hand-edited", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();
        let dir = store.instance_dir(&id);
        let broken = r#"{"motd": "hi", "max_players": 20,}"#;
        fs::write(dir.join("settings.json"), broken).unwrap();

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let loaded = store.get(&id).cloned().unwrap();
        store.update(&id, loaded).unwrap();
        assert_eq!(fs::read_to_string(dir.join("settings.json.corrupt")).unwrap(), broken);

        // 같은 내용이면 사본을 더 만들지 않는다
        fs::write(dir.join("settings.json"), broken).unwrap();
        InstanceStore::new(instances_dir.to_str().unwrap()).load().unwrap();
        let copies = || fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.corrupt")).count();
        assert_eq!(copies(), 1);

        // 다른 내용도 이미 남긴 사본(`-<초>`)과 같으면 기동마다 늘지 않는다
        let other = r#"{"motd": "bye",,}"#;
        fs::write(dir.join("settings.json.corrupt-1"), other).unwrap();
        fs::write(dir.join("settings.json"), other).unwrap();
        InstanceStore::new(instances_dir.to_str().unwrap()).load().unwrap();
        assert_eq!(copies(), 2);
    }

    /// 사본을 남기지 못하면 빈 설정으로 올리지 않는다: 다음 저장이 원본을 덮는다
    #[test]
    fn a_broken_settings_file_that_cannot_be_kept_is_left_alone() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        let inst = make_test_instance("unkeepable", "minecraft");
        let id = inst.id.clone();
        store.add(inst).unwrap();
        let dir = store.instance_dir(&id);
        let broken = r#"{"motd": "hi",}"#;
        fs::write(dir.join("settings.json"), broken).unwrap();
        // 사본 자리를 모두 막는다 (`.corrupt`와 지금부터 몇 초 동안의 `.corrupt-<초>`)
        fs::create_dir_all(dir.join("settings.json.corrupt")).unwrap();
        let now = chrono::Utc::now().timestamp();
        for t in now..now + 10 {
            fs::create_dir_all(dir.join(format!("settings.json.corrupt-{t}"))).unwrap();
        }

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();
        assert!(store.get(&id).is_none(), "an instance whose settings could not be kept must not load with empty settings");
        assert_eq!(fs::read_to_string(dir.join("settings.json")).unwrap(), broken);
    }

    #[test]
    fn test_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let instances_dir = tmp.path().join("instances");

        let mut store = InstanceStore::new(instances_dir.to_str().unwrap());
        store.load().unwrap();

        let mut inst = make_test_instance("struct-test", "minecraft");
        inst.module_settings.insert("ram".to_string(), serde_json::json!(4));
        inst.rest_password = Some("secret123".to_string());
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

    // ── 설치 방식 extension_data 자동 주입 테스트 ──

    fn steam(app_id: Option<u32>, beta: Option<&str>, platform: Option<&str>) -> Vec<(&'static str, serde_json::Value)> {
        let mut f = vec![("anonymous", serde_json::json!(true))];
        if let Some(id) = app_id {
            f.push(("app_id", serde_json::json!(id)));
        }
        if let Some(b) = beta {
            f.push(("beta", serde_json::json!(b)));
        }
        if let Some(p) = platform {
            f.push(("platform", serde_json::json!(p)));
        }
        f
    }

    #[test]
    fn test_apply_install_extension_data_steamcmd() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        assert!(inst.extension_data.is_empty());

        let changed = inst.apply_install_extension_data("steamcmd", &steam(Some(2394010), None, Some("windows")));
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
        let changed = inst.apply_install_extension_data("steamcmd", &steam(Some(380870), Some("unstable"), None));
        assert!(changed);
        assert!(inst.ext_enabled("install_method_steamcmd"));
        assert_eq!(inst.ext_str("steamcmd_beta"), Some("unstable"));
    }

    #[test]
    fn test_apply_install_extension_data_non_steamcmd_noop() {
        let mut inst = ServerInstance::new("test-mc", "minecraft");
        let changed = inst.apply_install_extension_data("download", &steam(None, None, None));
        assert!(!changed, "Non-steamcmd method should not change anything");
        assert!(inst.extension_data.is_empty());
    }

    #[test]
    fn test_apply_install_extension_data_idempotent() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        inst.apply_install_extension_data("steamcmd", &steam(Some(2394010), None, Some("windows")));

        // 같은 값으로 다시 호출 → 변경 없음
        let changed = inst.apply_install_extension_data("steamcmd", &steam(Some(2394010), None, Some("windows")));
        assert!(!changed, "Calling with same values should be idempotent");
    }

    #[test]
    fn test_apply_install_extension_data_preserves_existing() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        // 다른 익스텐션의 값은 그대로 둔다
        inst.extension_data.insert("other_flag".to_string(), serde_json::json!(true));

        inst.apply_install_extension_data("steamcmd", &steam(Some(2394010), None, None));

        assert!(inst.ext_enabled("other_flag"));
        assert!(inst.ext_enabled("install_method_steamcmd"));
    }

    #[test]
    fn test_apply_install_extension_data_serialization_roundtrip() {
        let mut inst = ServerInstance::new("test-pw", "palworld");
        inst.apply_install_extension_data("steamcmd", &steam(Some(2394010), Some("beta-branch"), Some("windows")));

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
