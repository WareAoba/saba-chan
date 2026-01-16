use serde::{Deserialize, Serialize};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

/// 서버 인스턴스 - 사용자가 추가한 관리 대상 서버
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInstance {
    pub id: String,                    // 고유 ID (uuid)
    pub name: String,                  // 사용자 지정 이름 (예: "메인 마크 서버")
    pub module_name: String,           // 사용할 모듈 이름 (예: "minecraft")
    pub executable_path: Option<String>, // 서버 실행 파일 경로
    pub working_dir: Option<String>,   // 작업 디렉토리
    pub auto_detect: bool,             // 프로세스 자동 감지 여부
    pub process_name: Option<String>,  // 감지할 프로세스 이름
    pub port: Option<u16>,             // 서버 포트
    pub rcon_port: Option<u16>,        // RCON 포트 (있는 경우)
    pub rcon_password: Option<String>, // RCON 비밀번호
    #[serde(default)]
    pub rest_host: Option<String>,     // REST API 호스트
    #[serde(default)]
    pub rest_port: Option<u16>,        // REST API 포트
    #[serde(default)]
    pub rest_username: Option<String>, // REST API 사용자명 (Basic Auth)
    #[serde(default)]
    pub rest_password: Option<String>, // REST API 비밀번호 (Basic Auth)
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
        }
    }
}

/// 인스턴스 저장소 - instances.json 관리
pub struct InstanceStore {
    file_path: PathBuf,
    instances: Vec<ServerInstance>,
}

impl InstanceStore {
    pub fn new(file_path: &str) -> Self {
        Self {
            file_path: PathBuf::from(file_path),
            instances: Vec::new(),
        }
    }

    /// 파일에서 인스턴스 로드
    pub fn load(&mut self) -> Result<()> {
        if !self.file_path.exists() {
            tracing::info!("Instance store file does not exist, creating new");
            self.instances = Vec::new();
            return Ok(());
        }

        let content = fs::read_to_string(&self.file_path)?;
        self.instances = serde_json::from_str(&content)?;
        tracing::info!("Loaded {} instances", self.instances.len());
        Ok(())
    }

    /// 파일에 인스턴스 저장
    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.instances)?;
        fs::write(&self.file_path, content)?;
        tracing::info!("Saved {} instances", self.instances.len());
        Ok(())
    }

    /// 인스턴스 추가
    pub fn add(&mut self, instance: ServerInstance) -> Result<()> {
        self.instances.push(instance);
        self.save()?;
        Ok(())
    }

    /// 인스턴스 제거
    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.instances.retain(|i| i.id != id);
        self.save()?;
        Ok(())
    }

    /// 인스턴스 조회
    pub fn get(&self, id: &str) -> Option<&ServerInstance> {
        self.instances.iter().find(|i| i.id == id)
    }

    /// 모든 인스턴스 조회
    pub fn list(&self) -> &[ServerInstance] {
        &self.instances
    }

    /// 인스턴스 업데이트
    pub fn update(&mut self, id: &str, instance: ServerInstance) -> Result<()> {
        if let Some(pos) = self.instances.iter().position(|i| i.id == id) {
            self.instances[pos] = instance;
            self.save()?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Instance not found: {}", id))
        }
    }
}
