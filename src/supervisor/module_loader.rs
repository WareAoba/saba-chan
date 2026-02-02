use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::io::Read;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub entry: String,  // lifecycle.py 경로
    pub process_name: Option<String>,  // config.process_name
    pub default_port: Option<u16>,  // config.default_port
    pub executable_path: Option<String>,  // config.executable_path
    #[serde(default)]
    pub settings: Option<ModuleSettings>,  // 설정 스키마
    #[serde(default)]
    pub commands: Option<ModuleCommands>,  // 명령어 스키마
}

impl ModuleMetadata {
    /// 설정 필드의 기본값을 가져옵니다
    pub fn get_setting_default<T: std::str::FromStr>(&self, field_name: &str) -> Option<T> {
        self.settings.as_ref().and_then(|s| {
            s.fields.iter().find(|f| f.name == field_name).and_then(|f| {
                f.default.as_ref().and_then(|d| {
                    match d {
                        toml::Value::Integer(i) => i.to_string().parse().ok(),
                        toml::Value::String(s) => s.parse().ok(),
                        toml::Value::Float(f) => f.to_string().parse().ok(),
                        toml::Value::Boolean(b) => b.to_string().parse().ok(),
                        _ => None,
                    }
                })
            })
        })
    }
    
    /// RCON 기본 포트를 가져옵니다 (모듈에 정의되지 않으면 25575)
    pub fn default_rcon_port(&self) -> u16 {
        self.get_setting_default("rcon_port").unwrap_or(25575)
    }
    
    /// REST API 기본 포트를 가져옵니다 (모듈에 정의되지 않으면 8212)
    pub fn default_rest_port(&self) -> u16 {
        self.get_setting_default("rest_port").unwrap_or(8212)
    }
    
    /// REST API 기본 호스트를 가져옵니다 (모듈에 정의되지 않으면 127.0.0.1)
    pub fn default_rest_host(&self) -> String {
        self.get_setting_default("rest_host").unwrap_or_else(|| "127.0.0.1".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSettings {
    pub fields: Vec<SettingField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCommands {
    pub fields: Vec<CommandField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandField {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub method: Option<String>,  // "rest", "rcon", "both"
    pub http_method: Option<String>,  // "GET", "POST" (REST only)
    pub endpoint_template: Option<String>,  // REST endpoint template
    pub rcon_template: Option<String>,  // RCON command template (e.g., "kick {userid}")
    #[serde(default)]
    pub inputs: Vec<CommandInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInput {
    pub name: String,
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub input_type: Option<String>,
    pub required: Option<bool>,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub default: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingField {
    pub name: String,
    pub field_type: String,  // "text", "number", "password", "file", "select", etc.
    pub label: String,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub default: Option<toml::Value>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub metadata: ModuleMetadata,
    pub path: String,  // 압축 해제된 디렉토리 또는 원본 디렉토리 절대 경로
    #[allow(dead_code)]
    pub is_zip: bool,  // ZIP에서 로드되었는지 여부
}

pub struct ModuleLoader {
    modules_dir: String,
    cached_modules: RwLock<Option<Vec<LoadedModule>>>,
}

impl ModuleLoader {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            modules_dir: modules_dir.to_string(),
            cached_modules: RwLock::new(None),
        }
    }

    /// 캐시를 무효화합니다 (새로운 모듈이 추가되었을 때 호출)
    pub fn invalidate_cache(&self) {
        *self.cached_modules.write().unwrap() = None;
        tracing::info!("Module cache invalidated");
    }

    /// 모듈 디렉터리에서 모든 사용 가능한 모듈 발견 (ZIP 및 폴더 모두 지원)
    pub fn discover_modules(&self) -> Result<Vec<LoadedModule>> {
        // 캐시 확인
        if let Some(modules) = self.cached_modules.read().unwrap().as_ref() {
            return Ok(modules.clone());
        }

        let mut modules = Vec::new();

        if !Path::new(&self.modules_dir).exists() {
            tracing::warn!("Modules directory does not exist: {}", self.modules_dir);
            *self.cached_modules.write().unwrap() = Some(modules.clone());
            return Ok(modules);
        }

        for entry in fs::read_dir(&self.modules_dir)? {
            let entry = entry?;
            let path = entry.path();

            // ZIP 파일 체크
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip") {
                match self.load_module_from_zip(&path) {
                    Ok(module) => {
                        tracing::info!(
                            "Discovered ZIP module: {} v{} from {}",
                            module.metadata.name, module.metadata.version, path.display()
                        );
                        modules.push(module);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to load ZIP module from {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
            // 폴더 체크 (기존 방식)
            else if path.is_dir() {
                let toml_path = path.join("module.toml");
                if toml_path.exists() {
                    match self.load_module_from_dir(&path) {
                        Ok(module) => {
                            tracing::info!(
                                "Discovered folder module: {} v{} at {}",
                                module.metadata.name, module.metadata.version, module.path
                            );
                            modules.push(module);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load folder module from {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        *self.cached_modules.write().unwrap() = Some(modules.clone());
        Ok(modules)
    }

    /// 폴더에서 개별 모듈 로드 (module.toml 파싱)
    fn load_module_from_dir(&self, module_path: &Path) -> Result<LoadedModule> {
        let toml_path = module_path.join("module.toml");
        let content = fs::read_to_string(&toml_path)?;
        let data: toml::Value = toml::from_str(&content)?;

        let module_section = data
            .get("module")
            .ok_or_else(|| anyhow::anyhow!("Missing [module] section"))?;

        let metadata = ModuleMetadata {
            name: module_section
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module name"))?
                .to_string(),
            version: module_section
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module version"))?
                .to_string(),
            description: module_section
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            entry: module_section
                .get("entry")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module entry"))?
                .to_string(),
            process_name: data
                .get("config")
                .and_then(|c| c.get("process_name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            default_port: data
                .get("config")
                .and_then(|c| c.get("default_port"))
                .and_then(|v| v.as_integer())
                .map(|i| i as u16),
            executable_path: data
                .get("config")
                .and_then(|c| c.get("executable_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            settings: parse_settings(&data),
            commands: parse_commands(&data),
        };

        Ok(LoadedModule {
            metadata,
            path: module_path.to_string_lossy().to_string(),
            is_zip: false,
        })
    }

    /// ZIP 파일에서 모듈 로드 (압축 해제 후 임시 디렉토리 사용)
    fn load_module_from_zip(&self, zip_path: &Path) -> Result<LoadedModule> {
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        // ZIP 내부에서 module.toml 찾기
        let mut toml_content = String::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            if name.ends_with("module.toml") {
                file.read_to_string(&mut toml_content)?;
                break;
            }
        }

        if toml_content.is_empty() {
            return Err(anyhow::anyhow!("No module.toml found in ZIP"));
        }

        // TOML 파싱
        let data: toml::Value = toml::from_str(&toml_content)?;
        let module_section = data
            .get("module")
            .ok_or_else(|| anyhow::anyhow!("Missing [module] section"))?;

        let metadata = ModuleMetadata {
            name: module_section
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module name"))?
                .to_string(),
            version: module_section
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module version"))?
                .to_string(),
            description: module_section
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            entry: module_section
                .get("entry")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing module entry"))?
                .to_string(),
            process_name: data
                .get("config")
                .and_then(|c| c.get("process_name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            default_port: data
                .get("config")
                .and_then(|c| c.get("default_port"))
                .and_then(|v| v.as_integer())
                .map(|i| i as u16),
            executable_path: data
                .get("config")
                .and_then(|c| c.get("executable_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            settings: parse_settings(&data),
            commands: parse_commands(&data),
        };

        // 임시 디렉토리에 압축 해제
        let temp_dir = tempfile::tempdir()?;
        let extract_path = temp_dir.path().to_path_buf();
        
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        archive.extract(&extract_path)?;

        // 임시 디렉토리 경로를 영구적으로 유지 (tempdir는 Drop시 삭제되므로)
        // 대신 modules/.extracted/ 디렉토리 사용
        let extracted_dir = PathBuf::from(&self.modules_dir)
            .join(".extracted")
            .join(&metadata.name);
        
        if extracted_dir.exists() {
            fs::remove_dir_all(&extracted_dir)?;
        }
        fs::create_dir_all(&extracted_dir)?;
        
        // temp_dir에서 extracted_dir로 복사
        copy_dir_all(&extract_path, &extracted_dir)?;

        Ok(LoadedModule {
            metadata,
            path: extracted_dir.to_string_lossy().to_string(),
            is_zip: true,
        })
    }

    /// 모듈 이름으로 모듈 조회
    #[allow(dead_code)]
    pub fn get_module(&self, name: &str) -> Result<LoadedModule> {
        let modules = self.discover_modules()?;
        modules
            .iter()
            .find(|m| m.metadata.name == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Module '{}' not found", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_loader_creation() {
        let loader = ModuleLoader::new("./modules");
        assert_eq!(loader.modules_dir, "./modules");
    }

    #[test]
    fn test_discover_modules_empty_dir() {
        let loader = ModuleLoader::new("./nonexistent_modules");
        let modules = loader.discover_modules().unwrap();
        assert!(modules.is_empty());
    }
}

/// \ub514\ub809\ud1a0\ub9ac \uc804\uccb4 \ubcf5\uc0ac \ud5ec\ud37c \ud568\uc218
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        
        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// TOML 데이터에서 settings 스키마 파싱
fn parse_settings(data: &toml::Value) -> Option<ModuleSettings> {
    let settings_table = data.get("settings")?;
    let fields_array = settings_table.get("fields")?;
    
    let mut fields = Vec::new();
    
    if let Some(array) = fields_array.as_array() {
        for field_value in array {
            if let Some(field_table) = field_value.as_table() {
                let name = field_table
                    .get("name")
                    .and_then(|v: &toml::Value| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let field_type = field_table
                    .get("type")
                    .and_then(|v: &toml::Value| v.as_str())
                    .unwrap_or("text")
                    .to_string();
                
                let label = field_table
                    .get("label")
                    .and_then(|v: &toml::Value| v.as_str())
                    .unwrap_or(&name)
                    .to_string();
                
                let description = field_table
                    .get("description")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                let required = field_table
                    .get("required")
                    .and_then(|v: &toml::Value| v.as_bool());
                
                let default = field_table.get("default").cloned();
                
                let min = field_table
                    .get("min")
                    .and_then(|v: &toml::Value| v.as_integer());
                
                let max = field_table
                    .get("max")
                    .and_then(|v: &toml::Value| v.as_integer());
                
                let options = field_table
                    .get("options")
                    .and_then(|v: &toml::Value| v.as_array())
                    .map(|arr: &Vec<toml::Value>| {
                        arr.iter()
                            .filter_map(|v: &toml::Value| v.as_str().map(|s: &str| s.to_string()))
                            .collect()
                    });
                
                fields.push(SettingField {
                    name,
                    field_type,
                    label,
                    description,
                    required,
                    default,
                    min,
                    max,
                    options,
                });
            }
        }
    }
    
    if fields.is_empty() {
        None
    } else {
        Some(ModuleSettings { fields })
    }
}

fn parse_commands(data: &toml::Value) -> Option<ModuleCommands> {
    let commands_table = data.get("commands")?;
    tracing::debug!("Found commands table: {:?}", commands_table);
    
    let fields_array = commands_table.get("fields")?;
    tracing::debug!("Found commands.fields array with {} items", fields_array.as_array()?.len());
    
    let mut fields = Vec::new();
    
    if let Some(array) = fields_array.as_array() {
        for field_value in array {
            if let Some(field_table) = field_value.as_table() {
                let name = field_table
                    .get("name")
                    .and_then(|v: &toml::Value| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let label = field_table
                    .get("label")
                    .and_then(|v: &toml::Value| v.as_str())
                    .unwrap_or(&name)
                    .to_string();
                
                let description = field_table
                    .get("description")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                let method = field_table
                    .get("method")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                let http_method = field_table
                    .get("http_method")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                let endpoint_template = field_table
                    .get("endpoint_template")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                let rcon_template = field_table
                    .get("rcon_template")
                    .and_then(|v: &toml::Value| v.as_str())
                    .map(|s: &str| s.to_string());
                
                // inputs 파싱
                let inputs = if let Some(inputs_value) = field_table.get("inputs") {
                    if let Some(inputs_array) = inputs_value.as_array() {
                        inputs_array
                            .iter()
                            .filter_map(|input_value| {
                                // 인라인 테이블도 as_table()로 파싱 가능
                                input_value.as_table().map(|input_table| {
                                    CommandInput {
                                        name: input_table
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown")
                                            .to_string(),
                                        label: input_table
                                            .get("label")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                        input_type: input_table
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                        required: input_table
                                            .get("required")
                                            .and_then(|v| v.as_bool()),
                                        placeholder: input_table
                                            .get("placeholder")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                        default: input_table.get("default").cloned(),
                                    }
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                
                fields.push(CommandField {
                    name,
                    label,
                    description,
                    method,
                    http_method,
                    endpoint_template,
                    rcon_template,
                    inputs,
                });
            }
        }
    }
    
    if fields.is_empty() {
        None
    } else {
        Some(ModuleCommands { fields })
    }
}
