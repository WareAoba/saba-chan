//! 익스텐션 폴더 탐색·ZIP 풀기·마운트와 해제, 쓰는 인스턴스 찾기

use super::*;

impl ExtensionManager {
    /// Python import에서 사용할 수 있도록 ext_id의 하이픈을 언더스코어로 변환합니다.
    /// 예: "ue4-ini" → "ue4_ini"
    pub(super) fn python_safe_dir_name(ext_id: &str) -> String {
        ext_id.replace('-', "_")
    }

    /// ext_id에 대응하는 디스크 상의 디렉토리를 반환합니다.
    ///
    /// 탐색 순서:
    /// 1. discovered에 등록되어 있으면 해당 `dir` 경로 반환
    /// 2. `extension_dir` (하이픈→언더스코어 이름, 없으면 원본 ID)
    /// 3. 없으면 Python-safe 디렉토리명 (신규 설치 시 사용)
    pub(super) fn ext_dir_on_disk(&self, ext_id: &str) -> PathBuf {
        if let Some(ext) = self.discovered.get(ext_id) {
            return ext.dir.clone();
        }
        extension_dir(&self.extensions_dir, ext_id)
            .unwrap_or_else(|| self.extensions_dir.join(Self::python_safe_dir_name(ext_id)))
    }

    /// 설치로 바꿀 폴더가 다른 익스텐션의 것이 아닌가. `a-b`와 `a_b`처럼 다른 id가 같은 폴더 이름이 될 수 있어,
    /// 폴더의 manifest id가 다르면 바꾸지 않는다 (manifest가 없거나 읽을 수 없는 폴더는 예전처럼 바꾼다)
    pub(super) fn ensure_folder_is_free_for(dest: &std::path::Path, ext_id: &str) -> Result<()> {
        let Ok(content) = std::fs::read_to_string(dest.join("manifest.json")) else { return Ok(()) };
        match parse_manifest(&content) {
            Ok((manifest, _)) if manifest.id != ext_id => Err(ExtensionError::folder_in_use(ext_id, &manifest.id).into()),
            _ => Ok(()),
        }
    }

    /// extensions/ 디렉토리를 스캔하여 익스텐션 발견.
    ///
    /// 지원 형식:
    /// - **폴더형**: `<id>/manifest.json` (현재 방식)
    /// - **단일 파일형**: `<id>.zip` → 자동 압축 해제 후 폴더형으로 등록
    pub fn discover(&mut self) -> Result<Vec<String>> {
        if !self.extensions_dir.is_dir() {
            tracing::warn!(
                "Extensions directory does not exist: {}",
                self.extensions_dir.display()
            );
            return Ok(Vec::new());
        }

        let zips: Vec<PathBuf> = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("zip"))
            .collect();
        for zip in zips {
            match self.extract_zip_extension(&zip) {
                Ok(Some(ext_id)) => tracing::info!("Auto-extracted zip extension: {}", ext_id),
                Ok(None) => {}
                Err(e) => tracing::warn!("Failed to extract zip extension {}: {}", zip.display(), e),
            }
        }

        let found = self.scan()?;
        tracing::info!("Extension discovery complete: {} found", found.len());
        Ok(found)
    }

    /// extensions/ 폴더에 놓인 `.zip`을 풀어 설치한다. 같은 익스텐션 폴더가 있으면 zip 내용으로 바꾼다
    /// (예전에는 zip을 지우고 새 내용을 버렸다). 다 풀고 나면 zip을 지운다.
    fn extract_zip_extension(&self, zip_path: &std::path::Path) -> Result<Option<String>> {
        if std::fs::metadata(zip_path)?.len() > PACKAGE_DOWNLOAD_LIMIT as u64 {
            anyhow::bail!("Extension package exceeds the download size limit");
        }
        let stem = zip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid zip filename: {}", zip_path.display()))?;
        let bytes = std::fs::read(zip_path)
            .with_context(|| format!("Failed to read zip: {}", zip_path.display()))?;
        let staging = self.extensions_dir.join(format!(".staging-{}", Self::python_safe_dir_name(stem)));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        let ext_id = unpack_package(&bytes, &staging, |_| Ok(()))?.id;
        let dest = self.extensions_dir.join(Self::python_safe_dir_name(&ext_id));
        if let Err(e) = Self::ensure_folder_is_free_for(&dest, &ext_id) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        crate::utils::swap_dir(&staging, &dest)?;
        Self::record_files(&ext_id, &dest, false);
        if let Err(e) = std::fs::remove_file(zip_path) {
            tracing::warn!("Failed to remove zip after extraction: {}", e);
        }
        tracing::info!("Extracted zip extension '{}' to {}", ext_id, dest.display());
        Ok(Some(ext_id))
    }

    /// 설치한 파일의 해시를 적어 둔다. 무결성 검사가 이 기록과 비교한다.
    /// `official`: 서명된 카탈로그에서 받은 것인지 (신뢰 표시)
    pub(super) fn record_files(ext_id: &str, dest: &std::path::Path, official: bool) {
        let key = format!("ext-{}", ext_id);
        let recorded = saba_chan_updater_lib::integrity::record_dir(&key, dest)
            .and_then(|_| saba_chan_updater_lib::integrity::set_official(&key, official));
        if let Err(e) = recorded {
            tracing::warn!("Could not record installed files for extension '{}': {}", ext_id, e);
        }
    }

    fn load_manifest(&self, path: &std::path::Path) -> Result<(ExtensionManifest, Vec<String>)> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        parse_manifest(&content).with_context(|| format!("Failed to parse {}", path.display()))
    }

    /// 마지막 스캔에서 로드하지 못한 익스텐션과 이유
    pub fn load_errors(&self) -> &[ExtensionLoadError] {
        &self.load_errors
    }

    /// 폴더마다 manifest.json을 (다시) 읽어 등록한다. 새로 찾은 id를 돌려준다.
    /// 폴더 이름은 manifest id(또는 하이픈을 밑줄로 바꾼 이름)와 같아야 한다.
    /// 디스크에서 사라진 (unmount 안 한) 익스텐션은 목록에 남는다.
    fn scan(&mut self) -> Result<Vec<String>> {
        let mut newly_found = Vec::new();
        let mut errors = Vec::new();
        if !self.extensions_dir.is_dir() {
            return Ok(newly_found);
        }
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.join("manifest.json").is_file())
            // `.staging-*`, `.old-*`는 설치 도중의 임시 폴더
            .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect();
        dirs.sort();

        for path in dirs {
            let manifest_path = path.join("manifest.json");
            let fail = |error: String| ExtensionLoadError { path: path.to_string_lossy().to_string(), error };
            let (manifest, warnings) = match self.load_manifest(&manifest_path) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("Failed to load manifest {}: {:#}", manifest_path.display(), e);
                    errors.push(fail(format!("{:#}", e)));
                    continue;
                }
            };
            let dir_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if dir_name != manifest.id && dir_name != Self::python_safe_dir_name(&manifest.id) {
                let e = ExtensionError::id_mismatch(&manifest.id, &dir_name);
                tracing::warn!("Skipping extension at {}: {}", path.display(), e);
                errors.push(fail(e.to_string()));
                continue;
            }
            for w in &warnings {
                tracing::warn!("extension '{}': {}", manifest.id, w);
            }
            let id = manifest.id.clone();
            if !self.discovered.contains_key(&id) {
                tracing::info!("Discovered extension: {} v{} ({})", manifest.name, manifest.version, id);
                newly_found.push(id.clone());
            }
            self.discovered.insert(id, DiscoveredExtension { manifest, dir: path, warnings });
        }

        self.load_errors = errors;
        Ok(newly_found)
    }

    /// 런타임 중 extensions/ 디렉토리를 재스캔하여 새로 추가된 익스텐션 발견.
    /// 기존에 이미 발견된 익스텐션은 매니페스트를 리로드(갱신),
    /// 디스크에서 제거된 (unmount 아닌) 익스텐션은 유지.
    pub fn rescan(&mut self) -> Result<Vec<String>> {
        let newly_found = self.scan()?;
        tracing::info!("Rescan complete: {} new extension(s)", newly_found.len());
        Ok(newly_found)
    }

    /// 단일 익스텐션을 핫 마운트 (디스크에서 로드 → discovered에 추가).
    /// 이미 존재하면 매니페스트를 리로드.
    pub fn mount(&mut self, ext_id: &str) -> Result<()> {
        let ext_path = self.ext_dir_on_disk(ext_id);
        let manifest_path = ext_path.join("manifest.json");

        if !manifest_path.exists() {
            return Err(ExtensionError::manifest_not_found(
                &manifest_path.display().to_string(),
            ).into());
        }

        let (manifest, warnings) = self.load_manifest(&manifest_path)?;
        if manifest.id != ext_id {
            return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
        }

        tracing::info!("Mounted extension: {} v{}", manifest.name, manifest.version);
        self.discovered.insert(
            ext_id.to_string(),
            DiscoveredExtension { manifest, dir: ext_path, warnings },
        );
        Ok(())
    }

    /// 끄거나 내리기 전 검사: 다른 활성 익스텐션이 이 익스텐션을 dependency로 선언했거나(역의존성),
    /// 인스턴스가 이 익스텐션을 쓰면 실패
    pub(super) fn ensure_unused(&self, ext_id: &str, active_ext_data: &[(&str, &HashMap<String, Value>)]) -> Result<()> {
        let dependents = self.dependents_of(ext_id);
        if !dependents.is_empty() {
            return Err(ExtensionError::has_dependents(ext_id, &dependents).into());
        }
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }
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

        self.ensure_unused(ext_id, active_ext_data)?;

        let was_enabled = self.enabled.remove(ext_id);
        let mounted = self.discovered.remove(ext_id);
        if let Err(e) = self.save_state() {
            if was_enabled {
                self.enabled.insert(ext_id.to_string());
            }
            if let Some(ext) = mounted {
                self.discovered.insert(ext_id.to_string(), ext);
            }
            return Err(e);
        }
        tracing::info!("Unmounted extension: {}", ext_id);
        Ok(())
    }

    /// ext_id를 dependency로 선언한 *활성* 익스텐션 목록
    fn dependents_of(&self, ext_id: &str) -> Vec<String> {
        self.discovered
            .values()
            .filter(|ext| {
                self.enabled.contains(&ext.manifest.id)
                    && ext.manifest.dependencies.contains_key(ext_id)
            })
            .map(|ext| ext.manifest.id.clone())
            .collect()
    }

    /// 인스턴스의 extension_data에서 이 익스텐션이 선언한 instance_fields를
    /// 하나라도 truthy 값으로 사용하는 인스턴스 이름 목록.
    /// 주어진 인스턴스 값(`extension_data`)이 쓰는 익스텐션 id: 필수 목록에 없어도 인스턴스 칸(`instance_fields`)에 값이 있으면 쓴다
    pub fn extensions_in_use(&self, ext_data: &[(&str, &HashMap<String, Value>)]) -> std::collections::HashSet<String> {
        self.discovered.keys().filter(|id| !self.instances_using_ext(id, ext_data).is_empty()).cloned().collect()
    }

    pub(super) fn instances_using_ext(
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
                field_keys.iter().any(|key| match data.get(*key) {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::String(v)) => !v.is_empty(),
                    Some(Value::Number(n)) => n.as_f64() != Some(0.0),
                    Some(Value::Array(v)) => !v.is_empty(),
                    Some(Value::Object(v)) => !v.is_empty(),
                    _ => false,
                })
            })
            .map(|(name, _)| name.to_string())
            .collect()
    }
}
