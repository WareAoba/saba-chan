//! 게임 서버 설치와 버전: 버전 목록, 설치, 설치된 버전 감지·기록

use super::*;

/// 인스턴스에 설치할 때 함께 싣는 것: 그 인스턴스의 설정, 격리 실행이 쓸 데이터 폴더, 받은 경로로 인정할 인스턴스 폴더, HOST 문맥
pub(crate) struct InstanceInstall {
    settings: Value,
    data_dirs: Vec<String>,
    boundary: crate::module_paths::Boundary,
    host: crate::plugin::host::Context,
}

impl InstanceInstall {
    pub(crate) fn of(
        instance: &crate::instance::ServerInstance,
        instance_dir: &std::path::Path,
        metadata: Option<&module_loader::ModuleMetadata>,
        host: crate::plugin::host::Context,
    ) -> Self {
        Self {
            settings: json!(&instance.module_settings),
            data_dirs: Supervisor::instance_data_dirs(instance, instance_dir),
            boundary: crate::module_paths::Boundary::for_instance(instance, instance_dir, metadata),
            host,
        }
    }

    /// `install_server`에 넘기는 인스턴스 설정 (`list_versions`도 같은 값으로 최신을 고른다)
    pub(crate) fn settings(&self) -> &Value {
        &self.settings
    }
}

/// 모듈 `install_server` 호출 하나 (D5). 설치 API·새 인스턴스 설치·서버 업데이트가 같은 키를 싣고 같은 한도로 부른다
pub(crate) struct InstallCall {
    module_name: String,
    entry: String,
    install_dir: String,
    config: Value,
    instance_boundary: Option<crate::module_paths::Boundary>,
    host: Option<crate::plugin::host::Context>,
    check_paths: bool,
}

impl InstallCall {
    /// `install_dir`에 받는다. 받은 경로(`install_path` 등)는 그 폴더 안이어야 한다
    pub(crate) fn new(module_name: &str, entry: impl Into<String>, install_dir: &str, accepted_agreements: &[String]) -> Self {
        Self {
            module_name: module_name.to_string(),
            entry: entry.into(),
            install_dir: install_dir.to_string(),
            config: json!({ "install_dir": install_dir, "accepted_agreements": accepted_agreements }),
            instance_boundary: None,
            host: None,
            check_paths: true,
        }
    }

    pub(crate) fn version(mut self, version: Option<&str>) -> Self {
        if let Some(version) = version {
            self.config["version"] = json!(version);
        }
        self
    }

    /// 그 밖의 키 (`server_executable`, `initial_settings`)
    pub(crate) fn with(mut self, key: &str, value: Value) -> Self {
        self.config[key] = value;
        self
    }

    /// 그 인스턴스의 설치: 설정과 데이터 폴더를 싣고, 받은 경로는 인스턴스의 폴더도 인정한다
    pub(crate) fn instance(mut self, target: InstanceInstall) -> Self {
        self.config["settings"] = target.settings;
        self.config[crate::sandbox::INSTANCE_DATA_DIRS] = json!(target.data_dirs);
        self.instance_boundary = Some(target.boundary);
        self.host = Some(target.host);
        self
    }

    /// 받은 경로는 부르는 쪽이 옮긴 뒤 확인한다 (준비 폴더에 받아 고른 폴더로 옮기는 새 인스턴스 설치)
    pub(crate) fn paths_checked_by_caller(mut self) -> Self {
        self.check_paths = false;
        self
    }

    /// 부른다. `progress`가 있으면 모듈이 알린 진행을 넘긴다
    pub(crate) async fn run(self, progress: Option<Box<dyn Fn(crate::plugin::ExtensionProgress) + Send>>) -> Result<Value> {
        let Self { module_name, entry, install_dir, config, instance_boundary, host, check_paths } = self;
        // 서버 파일과 Java를 받는다: 기본 120초로는 느린 회선에서 중간에 끊긴다
        let call = crate::plugin::PluginCall::new(entry, "install_server", config).in_instance(host).timeout(crate::plugin::INSTALL_TIMEOUT_SECS);
        let mut value = match progress {
            Some(progress) => call.run_with_progress(progress).await?,
            None => call.run().await?,
        };
        if check_paths && value.get("success").and_then(Value::as_bool) == Some(true) {
            let boundary = crate::module_paths::Boundary::for_install(std::path::Path::new(&install_dir), instance_boundary);
            boundary.returned_paths(&mut value, &module_name, "install_server")?;
        }
        Ok(value)
    }
}

/// 모듈이 알린 설치 진행을 프로비저닝 추적에 옮긴다 (모듈이 단계 이름을 주지 않으면 `default_label`)
pub(crate) fn progress_to_tracker(
    tracker: crate::ipc::ProvisionTracker,
    id: String,
    name: String,
    default_label: String,
) -> Box<dyn Fn(crate::plugin::ExtensionProgress) + Send> {
    Box::new(move |prog: crate::plugin::ExtensionProgress| {
        tracker.update(
            &id,
            &name,
            crate::ipc::ProvisionProgress {
                step: prog.step.unwrap_or(0),
                total: prog.total.unwrap_or(1),
                label: prog.label.unwrap_or_else(|| default_label.clone()),
                message: prog.message.unwrap_or_default(),
                done: false,
                error: None,
                percent: Some(prog.percent.unwrap_or(0)),
                steps: None,
                details: Default::default(),
            },
        );
    })
}

/// `original` 옆의 비어 있는 이름(`이름.bak`, `이름.1.bak`, …)에 복사본을 만든다. 이미 있는 파일은 덮지 않는다
fn backup_beside(original: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let name = original.file_name().unwrap_or_default().to_string_lossy().into_owned();
    for n in 0..100 {
        let backup = original.with_file_name(if n == 0 { format!("{name}.bak") } else { format!("{name}.{n}.bak") });
        let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&backup) {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let copied = std::fs::File::open(original)
            .and_then(|mut input| std::io::copy(&mut input, &mut out))
            .and_then(|_| out.sync_all())
            .and_then(|_| std::fs::set_permissions(&backup, std::fs::metadata(original)?.permissions()));
        return match copied {
            Ok(()) => Ok(backup),
            Err(e) => {
                drop(out);
                let _ = std::fs::remove_file(&backup);
                Err(e)
            }
        };
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("no free backup name next to {name}")))
}

/// 같은 파일을 가리키는가 (있으면 실제 경로로, 없으면 쓴 그대로 비교한다)
fn same_file_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl Supervisor {
    /// List available server versions (delegates to module lifecycle)
    pub async fn list_versions(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        include_snapshots: bool,
        page: u32,
        per_page: u32,
        settings: &Value,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            let mut config = json!({
                "include_snapshots": include_snapshots,
                "page": page,
                "per_page": per_page,
            });
            // 고를 설정(서버 종류 등). 그 설정으로 받을 수 있는 버전만 (`latest_version`과 같은 키)
            if settings.is_object() {
                config["settings"] = settings.clone();
            }

            crate::plugin::PluginCall::new(module_path, "list_versions", config)
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// Get detailed info for a specific version
    pub async fn get_version_details(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        version: &str,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            let config = json!({ "version": version });
            crate::plugin::PluginCall::new(module_path, "get_version_details", config)
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// 모듈 `list_versions`로 그 설정(서버 종류 등)에서 받을 수 있는 최신 버전 (`latest.release`, 예전 모양 `latest_release`).
    /// 찾지 못하면 그 까닭(모듈 실행 오류, 모듈이 준 메시지)
    pub async fn latest_version(module_path: &str, settings: &Value) -> Result<String> {
        match crate::plugin::run_plugin(module_path, "list_versions", json!({ "settings": settings })).await {
            Ok(v) if v.get("success").and_then(Value::as_bool) == Some(false) => Err(module_failure(&v, v.get("module").and_then(Value::as_str).unwrap_or(""), "list_versions")),
            Ok(v) => v
                .pointer("/latest/release")
                .or_else(|| v.get("latest_release"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("{}", v.get("message").and_then(Value::as_str).unwrap_or("list_versions gave no latest release"))),
            Err(e) => {
                tracing::warn!("Failed to fetch versions from '{}': {}", module_path, e);
                Err(e)
            }
        }
    }

    /// Install a server: download binary, setup directory, optional initial settings.
    /// 기존 실행 파일이 존재하면 `.bak`으로 백업하고, 실패 시 복원합니다.
    /// `instance`: 그 인스턴스의 설정과 격리 실행이 쓸 데이터 폴더 (처음 설치와 같은 값을 받는다)
    /// `keep_executable`: 그 인스턴스가 실제로 띄우는 파일 (모듈 실행 파일과 다르면 함께 백업한다)
    /// `commit`: 설치가 성공하면 백업을 지우기 전에 부른다 (인스턴스 기록 저장). 실패하면 백업으로 되돌리고 그 오류를 돌려준다
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn install_server(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        version: &str,
        install_dir: &str,
        executable: Option<&str>,
        accepted_agreements: &[String],
        initial_settings: Option<Value>,
        instance: Option<InstanceInstall>,
        keep_executable: Option<&str>,
        commit: Option<&InstallCommit<'_>>,
    ) -> Result<Value> {
        // 설치가 끝날 때까지 모듈·익스텐션 자동 교체를 막는다
        let gate = sup.read().await.addon_gate();
        let _addons = gate.hold_addons().await;
        let module = sup.read().await.module_loader.get_module(module_name)?;
        let module_path = module.entry_path();

        // 서버 실행 파일 이름: 명시 > 모듈 metadata.server_executable > 기본값 없음
        let effective_jar = executable
            .map(|s| s.to_string())
            .or_else(|| {
                module.metadata.server_executable.clone()
            });

        // ── 기존 바이너리 백업: 모듈 실행 파일과 인스턴스가 실제로 띄우는 파일 (모듈이 어느 쪽을 바꾸다 실패해도 되돌린다) ──
        // 설치 폴더 밖의 파일(밖을 가리키는 링크 포함)은 이 설치가 바꾸는 대상이 아니므로 건드리지 않는다
        let inside = |path: &std::path::Path| match (std::fs::canonicalize(path), std::fs::canonicalize(install_dir)) {
            (Ok(path), Ok(dir)) => path.starts_with(dir),
            _ => false,
        };
        let mut originals: Vec<std::path::PathBuf> = effective_jar.iter().map(|jar| std::path::Path::new(install_dir).join(jar)).collect();
        if let Some(exe) = keep_executable {
            let exe = std::path::Path::new(install_dir).join(exe);
            if !originals.iter().any(|o| same_file_path(o, &exe)) {
                originals.push(exe);
            }
        }
        // (원래 경로, 백업, 심볼릭 링크면 그 대상): 백업은 비어 있는 새 이름에만 만든다 (사용자의 기존 .bak를 덮지 않는다)
        let mut backups: Vec<(std::path::PathBuf, std::path::PathBuf, Option<std::path::PathBuf>)> = Vec::new();
        for original in originals.into_iter().filter(|original| original.is_file() && inside(original)) {
            let link_target = match std::fs::symlink_metadata(&original) {
                Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(&original).ok(),
                _ => None,
            };
            match backup_beside(&original) {
                Ok(backup) => {
                    tracing::info!("Backed up existing binary: {} → {}", original.display(), backup.display());
                    backups.push((original, backup, link_target));
                }
                Err(e) => {
                    // 되돌릴 수 없는 채로 모듈을 부르지 않는다
                    tracing::warn!("Failed to backup '{}': {}", original.display(), e);
                    for (_, made, _) in &backups {
                        let _ = std::fs::remove_file(made);
                    }
                    return Ok(json!({
                        "success": false,
                        "error_code": "backup_failed",
                        "file": original.file_name().unwrap_or_default().to_string_lossy(),
                        "message": format!("Could not back up {} before installing: {}", original.file_name().unwrap_or_default().to_string_lossy(), e),
                    }));
                }
            }
        }
        // 되돌리지 못한 파일은 백업을 남기고 (파일 이름, 남긴 백업 이름)을 돌려준다 (rename은 같은 폴더라 공간이 필요 없다).
        // 심볼릭 링크가 그대로 있으면 링크를 두고 백업 내용을 링크 대상에 쓴다. 모듈이 링크를 바꿨으면 그 자리에 되돌린다
        let restore = |why: &str| -> Vec<(String, String)> {
            let mut failed = Vec::new();
            for (original, backup, link_target) in &backups {
                let still_linked = |target: &std::path::PathBuf| {
                    std::fs::symlink_metadata(original).is_ok_and(|m| m.file_type().is_symlink())
                        && std::fs::canonicalize(original).is_ok_and(|now| now == *target)
                };
                let put_back = match link_target.as_ref().filter(|target| still_linked(target)) {
                    Some(target) => std::fs::copy(backup, target).map(|_| ()),
                    None => std::fs::rename(backup, original).or_else(|_| std::fs::copy(backup, original).map(|_| ())),
                };
                match put_back {
                    Ok(()) => {
                        tracing::info!("Restored backup after {}: {}", why, original.display());
                        let _ = std::fs::remove_file(backup);
                    }
                    Err(e) => {
                        tracing::error!("Failed to restore backup '{}': {}", backup.display(), e);
                        let name = |p: &std::path::Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        failed.push((name(original), name(backup)));
                    }
                }
            }
            failed
        };

        let mut call = InstallCall::new(module_name, module_path, install_dir, accepted_agreements).version(Some(version));
        if let Some(ref name) = effective_jar {
            call = call.with("server_executable", json!(name));
        }
        if let Some(settings) = initial_settings {
            call = call.with("initial_settings", settings);
        }
        if let Some(instance) = instance {
            call = call.instance(instance);
        }
        let result = call.run(None).await;

        match result {
            Ok(ref val) => {
                if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                    // 기록을 저장하지 못하면 파일도 설치 전으로 되돌린다 (옛 기록과 새 실행 파일이 섞이지 않게)
                    if let Some(commit) = commit {
                        let saved = commit(&mut *sup.write().await, val);
                        if let Err(e) = saved {
                            let (files, kept) = restore("unsaved install").into_iter().unzip();
                            return Err(e.context(CommitFailed(PutBackFailed { files, kept })));
                        }
                    }
                    // 설치 성공 → 백업 파일 정리
                    for (_, backup, _) in &backups {
                        let _ = std::fs::remove_file(backup);
                        tracing::info!("Removed backup after successful install: {}", backup.display());
                    }
                    result
                } else {
                    // 모듈이 success=false 반환 → 복원
                    let failed = restore("failed install");
                    let mut val = val.clone();
                    if let (false, Some(obj)) = (failed.is_empty(), val.as_object_mut()) {
                        let (files, kept): (Vec<String>, Vec<String>) = failed.into_iter().unzip();
                        obj.insert("restore_failed".into(), json!(files));
                        obj.insert("kept_backups".into(), json!(kept));
                    }
                    Ok(val)
                }
            }
            Err(e) => {
                // 플러그인 에러 → 복원
                let failed = restore("plugin error");
                if failed.is_empty() {
                    Err(e)
                } else {
                    let (files, kept) = failed.into_iter().unzip();
                    Err(e.context(PutBackFailed { files, kept }))
                }
            }
        }
    }

    /// Detect the installed server version by inspecting the server binary (e.g. parsing server.jar).
    /// Delegates to the module's `get_installed_version` lifecycle function.
    pub async fn get_installed_version(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        instance_id: &str,
    ) -> Result<Value> {
        // 시작·점검과 같은 값(`server_executable`·`working_dir` 등)을 받는다
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            if instance.module_name != module_name {
                anyhow::bail!("Instance '{}' is not a '{}' server", instance_id, module_name);
            }
            s.instance_call(instance_id, "get_installed_version")?
        };
        call.run().await
    }

    /// 감지한 서버 버전을 저장한다. 감지를 시작할 때 본 값(`seen`)에서 그사이 바뀌었으면 더 새 값이므로 두고 false (D-S12)
    pub fn save_detected_version(&mut self, instance_id: &str, seen: Option<&str>, version: &str) -> Result<bool> {
        let Some(mut instance) = self.instance_store.get(instance_id).cloned() else { return Ok(false) };
        if instance.server_version.as_deref() != seen || seen == Some(version) {
            return Ok(false);
        }
        instance.server_version = Some(version.to_string());
        self.instance_store.update(instance_id, instance)?;
        Ok(true)
    }

    /// server_version이 비어있고 working_dir이 설정된 인스턴스에 대해
    /// 모듈의 get_installed_version을 호출하여 버전을 감지·저장합니다.
    /// 데몬 초기화 뒤 한 번 호출하여 기존 서버의 버전을 보충합니다.
    pub async fn backfill_server_versions(sup: std::sync::Arc<tokio::sync::RwLock<Self>>) {
        // 감지 대상 수집 (borrow 분리를 위해 먼저 복사)
        let targets: Vec<(String, String, String)> = sup.read().await.instance_store.list().iter()
            .filter(|i| i.server_version.is_none())
            .filter(|i| {
                // module_settings.working_dir 또는 instance.working_dir이 있어야 감지 가능
                let has_ms_wd = i.module_settings.get("working_dir")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_inst_wd = i.working_dir.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
                has_ms_wd || has_inst_wd
            })
            .map(|i| (i.id.clone(), i.module_name.clone(), i.name.clone()))
            .collect();

        if targets.is_empty() {
            return;
        }

        tracing::info!("Backfilling server versions for {} instance(s)...", targets.len());

        for (id, module_name, name) in targets {
            match Self::get_installed_version(&sup, &module_name, &id).await {
                Ok(result) => {
                    if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                        if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                            // 모으던 때 비어 있던 값만 채운다. 그사이 저장된 버전은 덮지 않는다
                            match sup.write().await.save_detected_version(&id, None, version) {
                                Ok(true) => tracing::info!("Backfilled server version '{}' for instance '{}'", version, name),
                                Ok(false) => {}
                                Err(e) => tracing::warn!("Failed to save backfilled version for '{}': {}", name, e),
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Version backfill skipped for '{}': {}", name, e);
                }
            }
        }
    }
}
