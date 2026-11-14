//! 새 인스턴스의 서버 설치(프로비저닝)

use serde_json::{json, Value};


/// 새 서버 설치 (만들기 뒤 백그라운드): 모듈 `install_server`를 부르고 진행·결과를 프로비저닝 추적에 알린다
pub(crate) async fn install_new_server(job: InstallJob) {
    let InstallJob {
        mut install_guard,
        supervisor_arc,
        module_name_owned,
        tracker_done,
        name_done,
        id_done,
        text,
        chosen_version,
        install_method,
        leftovers_dir,
        made_folder,
        install_dir_str,
        accepted_agreements,
        target,
        provision_label,
    } = job;
    let leftovers: Option<InstallStaging>;
    // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
    let module_path = {
        let sup = supervisor_arc.read().await;
        match sup.module_loader.get_module(&module_name_owned) {
            Ok(m) => m.entry_path(),
            Err(e) => {
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(text.t("provision.module_missing", &[("error", &e.to_string())]))));
                return;
            }
        }
    };
    // supervisor read lock 해제됨 — 이후 플러그인 호출이
    // 모니터 태스크의 write lock을 차단하지 않음

    // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
    // lifecycle.py install_server 호출 (SteamCMD / download 모두 처리)
    // 설치한 버전 (고른 것, 고르지 않았으면 최신). 설치가 끝나면 인스턴스에 적는다
    let installed_version;
    let result = {
        // download 방식 모듈은 먼저 list_versions로 최신 버전 조회.
        // 인스턴스 설정(서버 종류 등)을 넘겨, 그 설정으로 받을 수 있는 최신을 고르게 한다
        let version = if let Some(version) = chosen_version {
            Some(version)
        } else if install_method == "download" {
            match crate::supervisor::Supervisor::latest_version(&module_path, target.settings()).await {
                Ok(version) => Some(version),
                Err(e) => {
                    drop(install_guard.take());
                    tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(text.t("provision.no_latest_version", &[]))).with_failure(crate::plugin::error_response(&e)));
                    return;
                }
            }
        } else {
            None // 익스텐션 방식은 version 불필요
        };
        installed_version = version.clone();

        // 만들 수 없으면 고른 폴더에 바로 설치한다 (실패해도 아무것도 지우지 않는다)
        leftovers = leftovers_dir.as_deref().and_then(|dir| {
            InstallStaging::new(dir, made_folder)
                .map_err(|e| tracing::warn!("Could not prepare the install folder in {}: {}", dir.display(), e))
                .ok()
        });
        let target_dir = leftovers.as_ref().map_or_else(|| install_dir_str.clone(), |l| l.staging.to_string_lossy().to_string());
        // 받은 경로는 고른 폴더로 옮긴 뒤 확인한다 (아래)
        let call = crate::supervisor::InstallCall::new(&module_name_owned, &module_path, &target_dir, &accepted_agreements)
            .version(version.as_deref())
            .instance(target)
            .paths_checked_by_caller();
        let progress = crate::supervisor::progress_to_tracker(tracker_done.clone(), id_done.clone(), name_done.clone(), provision_label);

        // 설치가 끝날 때까지 모듈·익스텐션 자동 교체를 막는다
        let gate = supervisor_arc.read().await.addon_gate();
        let _addons = gate.hold_addons().await;
        call.run(Some(progress)).await
    };

    let result = checked_install(&supervisor_arc, &id_done, &module_name_owned, leftovers.as_ref(), result).await;

    match result {
        Ok(val) => {
            let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
            if success {
                let saved = save_installed(&supervisor_arc, &id_done, &module_path, &install_method, installed_version.as_deref(), &val).await;

                // 받은 파일을 치우는 동안은 작업 잠금을 쥔다 (그 사이 시작·복원이 같은 폴더를 쓰지 않게)
                if let (Err(_), Some(left)) = (&saved, &leftovers) { left.clear() }
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                // 실행 경로를 저장하지 못했으면 설치했다고 알리지 않는다 (시작할 실행 파일을 모른다)
                if let Err(e) = saved {
                    tracing::warn!("Native provisioning for '{}' finished but the instance could not be saved: {}", name_done, e);
                    tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(text.t("provision.save_failed", &[]))));
                    return;
                }
                tracing::info!("Native provisioning completed for '{}'", name_done);
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.installed", &[]), None));

                // 5초 후 tracker 자동 정리
                let tracker_cleanup = tracker_done.clone();
                let id_cleanup = id_done.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    tracker_cleanup.remove(&id_cleanup);
                });
            } else {
                let err_msg = val.get("message")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| text.t("provision.install_failed", &[]));
                tracing::warn!("Native provisioning failed for '{}': {}", name_done, err_msg);
                if let Some(left) = &leftovers { left.clear() }
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(err_msg)).with_failure(val));
            }
        }
        Err(e) => {
            tracing::warn!("Native provisioning error for '{}': {}", name_done, e);
            if let Some(left) = &leftovers { left.clear() }
            drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
            tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(text.t("provision.script_failed", &[]))).with_failure(crate::plugin::error_response(&e)));
        }
    }
}

/// 모듈이 성공이라고 답한 설치: 받은 서버를 고른 폴더로 옮기고(`leftovers`), 돌려준 경로가 인스턴스 경계 안인지 본다
async fn checked_install(
    supervisor: &tokio::sync::RwLock<crate::supervisor::Supervisor>,
    id: &str,
    module: &str,
    leftovers: Option<&InstallStaging>,
    result: anyhow::Result<Value>,
) -> anyhow::Result<Value> {
    let mut value = match result {
        Ok(value) if value.get("success").and_then(Value::as_bool) == Some(true) => value,
        other => return other,
    };
    // 받은 서버를 고른 폴더로 옮긴다 (경로 확인은 옮긴 자리로)
    if let Some(left) = leftovers {
        if let Err(e) = left.promote(&mut value) {
            return Err(anyhow::anyhow!("Could not move the installed server into its folder: {}", e));
        }
    }
    let sup = supervisor.read().await;
    match sup.instance_store.get(id) {
        Some(inst) => {
            let metadata = sup.module_loader.get_module(&inst.module_name).ok();
            let boundary = crate::module_paths::Boundary::for_instance(inst, &sup.instance_store.instance_dir(id), metadata.as_ref().map(|m| &m.metadata));
            boundary.returned_paths(&mut value, module, "install_server").map(|()| value)
        }
        None => Err(anyhow::anyhow!("Instance disappeared during install")),
    }
}

/// 설치에 성공한 서버의 판·실행 파일·서버 폴더를 인스턴스에 적는다 (그 사이 지워졌으면 아무것도 하지 않는다)
async fn save_installed(
    supervisor: &tokio::sync::RwLock<crate::supervisor::Supervisor>,
    id: &str,
    module_path: &str,
    install_method: &str,
    installed_version: Option<&str>,
    val: &Value,
) -> anyhow::Result<()> {
    let exe_path = val.get("executable_path").and_then(|v| v.as_str());
    let install_path = val.get("install_path").and_then(|v| v.as_str());
    let mut sup = supervisor.write().await;
    let Some(mut updated) = sup.instance_store.get(id).cloned() else {
        return Ok(());
    };
    if let Some(version) = installed_version {
        updated.record_installed_version(install_method, version);
    }
    if let Some(exe) = exe_path.filter(|exe| !exe.is_empty()) {
        updated.executable_path = Some(exe.to_string());
    }
    if let Some(ip) = install_path.filter(|ip| !ip.trim().is_empty()) {
        // 확인되지 않은 모듈은 인스턴스 폴더 안만 (서버 폴더는 다음 격리 실행에 쓰기를 받는다)
        let root = sup.instance_store.instance_dir(id);
        if crate::sandbox::accepts_module_dir(std::path::Path::new(module_path), ip, &root) {
            updated.working_dir = Some(ip.to_string());
        } else {
            tracing::warn!("Ignoring install_path outside the instance folder from unverified module: {}", ip);
        }
    }
    sup.instance_store.update(id, updated)
}

/// `install_new_server`가 받는 값 (만들기 요청이 정한 것)
pub(crate) struct InstallJob {
    pub(crate) install_guard: Option<crate::instance::lock::InstanceLock>,
    pub(crate) supervisor_arc: std::sync::Arc<tokio::sync::RwLock<crate::supervisor::Supervisor>>,
    pub(crate) module_name_owned: String,
    pub(crate) tracker_done: crate::ipc::ProvisionTracker,
    pub(crate) name_done: String,
    pub(crate) id_done: String,
    pub(crate) text: crate::chat::i18n::Catalog,
    pub(crate) chosen_version: Option<String>,
    pub(crate) install_method: String,
    /// 고른 폴더에 설치할 때 그 폴더 (실패하면 설치가 만든 것만 치운다)
    pub(crate) leftovers_dir: Option<std::path::PathBuf>,
    pub(crate) made_folder: bool,
    pub(crate) install_dir_str: String,
    pub(crate) accepted_agreements: Vec<String>,
    /// 인스턴스 설정·데이터 폴더·HOST 문맥 (설치 중에 데이터 폴더에 첫 설정을 쓰는 모듈도 격리 실행에서 쓸 수 있게)
    pub(crate) target: crate::supervisor::InstanceInstall,
    pub(crate) provision_label: String,
}

/// 고른 폴더에 처음 설치할 때 모듈이 받는 설치 폴더 (그 폴더 안). 점으로 시작해 탐색이 건너뛴다
pub(crate) const INSTALL_STAGING: &str = ".saba-installing";

/// 고른 폴더(`working_dir`)에 처음 설치하기. 모듈은 그 안의 `INSTALL_STAGING`에 설치하고, 성공하면 데몬이 내용을 고른 폴더로 옮긴다.
/// 설치가 실패하면 그 설치가 만든 것(설치 폴더와 옮긴 항목)만 지운다: 설치하는 동안 사용자가 고른 폴더에 넣은 파일은 남는다.
/// 데몬이 만든 폴더였으면 비었을 때 그 폴더도 지운다
pub(crate) struct InstallStaging {
    dir: std::path::PathBuf,
    staging: std::path::PathBuf,
    made_folder: bool,
    /// 고른 폴더로 옮긴 항목 (옮긴 뒤 저장하지 못했을 때 지운다)
    moved: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl InstallStaging {
    fn new(dir: &std::path::Path, made_folder: bool) -> std::io::Result<Self> {
        let staging = dir.join(INSTALL_STAGING);
        // 중간에 끝난 예전 설치가 남긴 것 (이 이름은 데몬 것이다)
        if std::fs::symlink_metadata(&staging).is_ok() {
            remove_entry(&staging)?;
        }
        std::fs::create_dir(&staging)?;
        Ok(Self { dir: dir.to_path_buf(), staging, made_folder, moved: Default::default() })
    }

    /// 설치 폴더의 내용을 고른 폴더로 옮기고, 모듈이 알려 준 경로(`install_path`·`executable_path`)를 옮긴 자리로 바꾼다.
    /// 고른 폴더에 같은 이름이 생겼으면(사용자가 넣었다) 덮지 않고 실패한다
    fn promote(&self, result: &mut Value) -> std::io::Result<()> {
        let canonical = std::fs::canonicalize(&self.staging).ok();
        for entry in std::fs::read_dir(&self.staging)? {
            let entry = entry?;
            let target = self.dir.join(entry.file_name());
            if std::fs::symlink_metadata(&target).is_ok() {
                return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} already exists", target.display())));
            }
            // Windows는 방금 쓴 파일을 백신 등이 잠깐 쥐고 있어 옮기기가 실패하기도 한다 (폴더 옮기기와 같이 다시 한다)
            let mut tries = 0;
            while let Err(e) = std::fs::rename(entry.path(), &target) {
                tries += 1;
                if tries == 3 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            self.moved.lock().unwrap_or_else(|e| e.into_inner()).push(target);
        }
        std::fs::remove_dir(&self.staging)?;
        for field in ["install_path", "executable_path"] {
            let Some(path) = result.get(field).and_then(Value::as_str).map(std::path::PathBuf::from) else { continue };
            let rel = std::iter::once(&self.staging).chain(canonical.as_ref()).find_map(|base| path.strip_prefix(base).ok());
            if let Some(rel) = rel {
                let moved = if rel.as_os_str().is_empty() { self.dir.clone() } else { self.dir.join(rel) };
                result[field] = json!(moved.to_string_lossy());
            }
        }
        Ok(())
    }

    fn clear(&self) {
        let moved = std::mem::take(&mut *self.moved.lock().unwrap_or_else(|e| e.into_inner()));
        for path in moved.iter().chain(std::iter::once(&self.staging)) {
            match remove_entry(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("Could not remove {} left by a failed install: {}", path.display(), e),
                _ => {}
            }
        }
        if self.made_folder {
            let _ = std::fs::remove_dir(&self.dir); // 비었을 때만 지워진다
        }
    }
}

/// 항목 하나를 지운다. 링크(정션 포함)는 링크만 지운다 (가리키는 곳은 이 설치가 만든 것이 아니다)
fn remove_entry(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 설치하는 동안 사용자가 고른 폴더에 넣은 파일은 설치가 실패해도 남는다 (설치가 만든 것만 지운다)
    #[test]
    fn a_failed_install_keeps_files_the_user_added_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::create_dir_all(staging.staging.join("libraries")).unwrap();
        std::fs::write(staging.staging.join("partial.jar"), "x").unwrap();
        std::fs::write(dir.join("my-world.zip"), "mine").unwrap();
        staging.clear();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("my-world.zip")]);

        // 옮긴 뒤 저장하지 못해도 옮긴 것만 지운다
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::write(staging.staging.join("server.jar"), "x").unwrap();
        let mut result = json!({ "install_path": staging.staging.to_string_lossy(), "executable_path": staging.staging.join("server.jar").to_string_lossy() });
        staging.promote(&mut result).unwrap();
        assert_eq!(result["install_path"], json!(dir.to_string_lossy()));
        assert_eq!(result["executable_path"], json!(dir.join("server.jar").to_string_lossy()));
        assert!(dir.join("server.jar").is_file() && !staging.staging.exists());
        staging.clear();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("my-world.zip")]);

        // 사용자가 같은 이름을 넣었으면 덮지 않는다
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::write(staging.staging.join("my-world.zip"), "installer").unwrap();
        assert!(staging.promote(&mut json!({})).is_err());
        staging.clear();
        assert_eq!(std::fs::read_to_string(dir.join("my-world.zip")).unwrap(), "mine");
    }
}
