//! 모듈 폴더·ZIP을 읽어 들이는 로더

use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::io::Read;
use anyhow::Result;
use zip::ZipArchive;

const MODULE_ARCHIVE_DOWNLOAD_LIMIT: u64 = 128 * 1024 * 1024;
const MODULE_ARCHIVE_LIMITS: crate::archive::Limits = crate::archive::Limits {
    what: "Module archive",
    entries: 20_000,
    expanded: 1024 * 1024 * 1024,
};

/// 모듈 폴더 하나를 데몬과 같은 규칙으로 읽는다 (`saba-core module validate`)
pub fn load_module_dir(module_path: &Path) -> Result<LoadedModule> {
    let toml_path = module_path.join("module.toml");
    let content = fs::read_to_string(&toml_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", toml_path.display(), e))?;
    let module = LoadedModule {
        metadata: parse_module_toml(&content)?,
        path: module_path.to_string_lossy().to_string(),
        source: module_path.to_string_lossy().to_string(),
    };
    module.check_loadable()?;
    Ok(module)
}

/// module.toml 스키마 v1의 JSON Schema (편집기 자동완성·모듈 저장소 CI 검증용)
pub fn module_json_schema() -> serde_json::Value {
    // 모듈 저장소가 받는 draft-07(`definitions`)로 낸다. schemars 1.x의 기본은 2020-12(`$defs`)다
    let schema = schemars::generate::SchemaSettings::draft07().into_generator().into_root_schema_for::<ModuleToml>();
    serde_json::to_value(schema).unwrap_or_default()
}

pub struct ModuleLoader {
    pub(crate) modules_dir: String,
    /// 자주 부르는 곳(서버 목록 2초, 콘솔 0.5초, 감시 주기)이 목록을 복제하지 않고 함께 본다
    cached_modules: RwLock<Option<Arc<Vec<LoadedModule>>>>,
    /// 마지막 탐색에서 로드하지 못한 모듈
    load_errors: RwLock<Vec<ModuleLoadError>>,
}

impl ModuleLoader {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            modules_dir: modules_dir.to_string(),
            cached_modules: RwLock::new(None),
            load_errors: RwLock::new(Vec::new()),
        }
    }

    /// 마지막 탐색에서 로드하지 못한 모듈과 이유
    pub fn load_errors(&self) -> Vec<ModuleLoadError> {
        self.load_errors.read().unwrap().clone()
    }

    /// 모듈 디렉토리 경로 반환
    pub fn modules_dir(&self) -> &str {
        &self.modules_dir
    }

    /// 모듈 폴더를 옮긴 뒤 새 폴더를 보게 한다 (`/api/system/paths/move`). 캐시는 비운다
    pub fn set_modules_dir(&mut self, modules_dir: &str) {
        self.modules_dir = modules_dir.to_string();
        self.invalidate_cache();
    }

    /// 캐시를 무효화합니다 (새로운 모듈이 추가되었을 때 호출)
    pub fn invalidate_cache(&self) {
        *self.cached_modules.write().unwrap() = None;
        tracing::info!("Module cache invalidated");
    }

    /// 모듈 디렉터리에서 모든 사용 가능한 모듈 발견 (ZIP 및 폴더 모두 지원)
    pub fn discover_modules(&self) -> Result<Vec<LoadedModule>> {
        Ok(self.modules()?.as_ref().clone())
    }

    /// 캐시한 모듈 목록을 복제 없이 돌려준다. 캐시가 비었으면 모듈 폴더를 다시 읽는다
    pub fn modules(&self) -> Result<Arc<Vec<LoadedModule>>> {
        // 캐시 확인
        if let Some(modules) = self.cached_modules.read().unwrap().as_ref() {
            return Ok(modules.clone());
        }

        let mut modules = Vec::new();
        let mut errors = Vec::new();

        if !Path::new(&self.modules_dir).exists() {
            tracing::warn!("Modules directory does not exist: {}", self.modules_dir);
            let modules = Arc::new(modules);
            *self.cached_modules.write().unwrap() = Some(modules.clone());
            return Ok(modules);
        }

        for entry in fs::read_dir(&self.modules_dir)? {
            let entry = entry?;
            let path = entry.path();
            // 점으로 시작하는 것은 설치 중인 준비 폴더·옛 폴더·받는 중인 파일이다
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

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
                        errors.push(ModuleLoadError::new(&path, &e));
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
                            errors.push(ModuleLoadError::new(&path, &e));
                        }
                    }
                }
            }
        }

        let modules = Arc::new(modules);
        *self.cached_modules.write().unwrap() = Some(modules.clone());
        *self.load_errors.write().unwrap() = errors;
        Ok(modules)
    }

    /// 폴더에서 개별 모듈 로드 (module.toml 파싱)
    fn load_module_from_dir(&self, module_path: &Path) -> Result<LoadedModule> {
        load_module_dir(module_path)
    }

    /// ZIP 파일에서 모듈 로드 (압축 해제 후 임시 디렉토리 사용)
    fn load_module_from_zip(&self, zip_path: &Path) -> Result<LoadedModule> {
        if fs::metadata(zip_path)?.len() > MODULE_ARCHIVE_DOWNLOAD_LIMIT {
            anyhow::bail!("Module archive exceeds the download size limit");
        }
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        crate::archive::check(&mut archive, &MODULE_ARCHIVE_LIMITS)?;

        // ZIP 내부에서 가장 얕은 위치의 module.toml을 고른다
        // (`ends_with`로 고르면 `foo_module.toml`이나 하위 폴더 것이 먼저 잡힌다)
        let toml_entry = (0..archive.len())
            .filter_map(|i| {
                let name = archive.by_index(i).ok()?.name().ok()?.into_owned();
                let depth = name.trim_end_matches('/').matches('/').count();
                (name.rsplit('/').next() == Some("module.toml")).then_some((depth, i, name))
            })
            .min();
        let mut toml_content = String::new();
        // 고른 module.toml이 든 폴더가 모듈 폴더다 (`probe/module.toml`처럼 폴더로 감싼 ZIP)
        let mut root = PathBuf::new();
        if let Some((_, i, name)) = &toml_entry {
            archive.by_index(*i)?.read_to_string(&mut toml_content)?;
            let parent = Path::new(name).parent().unwrap_or(Path::new(""));
            if !parent.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
                anyhow::bail!("module.toml in {} is not inside a plain folder", zip_path.display());
            }
            root = parent.to_path_buf();
        }

        if toml_content.is_empty() {
            return Err(anyhow::anyhow!("No module.toml found in ZIP"));
        }

        // 강타입 TOML 파싱
        let metadata = parse_module_toml(&toml_content)?;

        // 임시 디렉토리에 압축 해제
        let temp_dir = tempfile::tempdir()?;
        let extract_path = temp_dir.path().to_path_buf();
        
        crate::archive::extract(archive, &extract_path, &MODULE_ARCHIVE_LIMITS)?;

        // 임시 디렉토리 경로를 영구적으로 유지 (tempdir는 Drop시 삭제되므로)
        // 대신 modules/.extracted/ 디렉토리 사용
        if !crate::utils::is_single_path_component(&metadata.name) {
            anyhow::bail!("module name {:?} in {} is not a plain folder name", metadata.name, zip_path.display());
        }
        let extracted_dir = PathBuf::from(&self.modules_dir)
            .join(".extracted")
            .join(&metadata.name);
        
        if extracted_dir.exists() {
            fs::remove_dir_all(&extracted_dir)?;
        }
        fs::create_dir_all(&extracted_dir)?;
        
        // temp_dir에서 extracted_dir로 복사
        copy_dir_all(&extract_path.join(&root), &extracted_dir)?;

        let module = LoadedModule {
            metadata,
            path: extracted_dir.to_string_lossy().to_string(),
            source: zip_path.to_string_lossy().to_string(),
        };
        module.check_loadable()?;
        Ok(module)
    }

    /// 모듈 이름으로 모듈 조회
    pub fn get_module(&self, name: &str) -> Result<LoadedModule> {
        let modules = self.modules()?;
        modules
            .iter()
            .find(|m| m.metadata.name == name)
            .cloned()
            .ok_or_else(|| match self.load_failure(name) {
                // 있지만 이 데몬으로는 못 쓰는 모듈: "없음" 대신 이유를 준다
                Some(failed) => anyhow::anyhow!(failed.error),
                None => anyhow::anyhow!("Module '{}' not found", name),
            })
    }

    /// 이름이 `name`인 모듈을 읽었지만 쓸 수 없었다면 그 기록
    pub fn load_failure(&self, name: &str) -> Option<ModuleLoadError> {
        // module.toml을 끝까지 읽지 못한 실패(더 새 스키마 등)는 이름이 없어 모듈 폴더 이름으로 찾는다
        let folder = |e: &ModuleLoadError| Path::new(&e.path).file_name().is_some_and(|f| f == name);
        self.load_errors
            .read()
            .unwrap()
            .iter()
            .find(|e| e.module.as_deref().map_or_else(|| folder(e), |m| m == name))
            .cloned()
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
