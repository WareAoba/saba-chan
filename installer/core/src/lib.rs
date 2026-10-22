//! Saba-chan 설치·제거 엔진 (창과 무관)
//!
//! 창 인스톨러(`installer/gui`)와 `saba-chan-cli install`이 같은 설치 순서를 쓴다.
//! 진행은 `InstallHooks::progress`로 알리고, 권한 상승은 창 인스톨러만 `InstallHooks::elevate_create_dir`로 한다.

#[cfg(target_os = "windows")]
pub mod registry;
pub mod release;
pub mod runtime_bootstrap;
#[cfg(target_os = "windows")]
pub mod shortcuts;
pub mod uninstall;

pub use release::VerifiedRelease;
use saba_chan_updater_lib::config_file::ConfigFile;
use saba_chan_updater_lib::constants;
pub use saba_chan_updater_lib::profile::Profile;
use saba_chan_updater_lib::relay::{self, ReleaseManifest};
use saba_chan_updater_lib::UpdateManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 설치기 버전 — 설치기 컴포넌트(`versions.toml` installer)와 같이 간다
const INSTALLER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

/// 설치 진행 상태
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallProgress {
    pub step: String,
    pub message: String,
    pub percent: i32,
    pub complete: bool,
    pub error: Option<String>,
    pub installed_components: Vec<String>,
    /// 현재 다운로드 중인 파일 이름
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_file: Option<String>,
    /// 현재 다운로드 수신 바이트
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_received: Option<u64>,
    /// 현재 다운로드 전체 바이트 (Content-Length)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_total: Option<u64>,
    /// 설치는 끝났지만 실패한 단계 (완료 때만)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// 설치 로그 경로 (경고가 있을 때만)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// 실패 종류 (화면이 번역 문구를 고른다. `message`는 원문 그대로 함께 보인다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// 사용 가능한 모듈 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    /// 이 모듈이 요구하는 익스텐션 ID 목록 (예: ["steamcmd", "ue4-ini"])
    #[serde(default)]
    pub required_extensions: Vec<String>,
}

/// 설치가 어느 경로로 끝나든 임시 폴더를 지운다 (실패하고 다시 시도하면 새 프로세스는 다른 폴더를 쓴다)
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 이 구성에 들지 않는 GUI 실행 파일·바로가기를 지우고, 그 구성요소의 설치 기록(installed-manifest, `records`의 파일 해시)을 뺀다.
/// GUI를 지우지 못하면 기록을 그대로 두고 실패한다: 남은 GUI를 업데이터가 더는 손대지 않게 되기 때문이다.
/// 파일 해시 기록을 고치지 못한 것은 경고로 돌려준다 (무결성 검사에 "파일 없음"으로 보일 뿐이다)
fn drop_parts_outside(
    profile: Profile,
    install_dir: &Path,
    records: &Path,
    installed: &mut HashMap<String, String>,
) -> Result<Option<String>, String> {
    if !profile.includes("gui") {
        #[cfg(target_os = "windows")]
        shortcuts::remove_shortcuts_into(install_dir, shortcuts::ALL_SHORTCUT_NAMES);
        for exe in saba_chan_updater_lib::GUI_EXE_NAMES
            .iter()
            .map(|name| install_dir.join(name))
            .filter(|exe| exe.is_file())
        {
            std::fs::remove_file(&exe).map_err(|e| format!("Failed to remove {}: {}", exe.display(), e))?;
        }
    }
    installed.retain(|key, _| profile.includes(key));
    Ok(
        saba_chan_updater_lib::integrity::retain_at(records, |key| profile.includes(key))
            .err()
            .map(|e| format!("Failed to update {}: {}", records.display(), e)),
    )
}

/// 설치 설정 (스냅샷)
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub install_path: PathBuf,
    pub language: String,
    /// 데스크톱이면 GUI와 바로가기까지, 헤드리스면 GUI 없이 설치한다. `install-profile.json`에 적는다
    pub profile: Profile,
    // 바로가기는 Windows에서만 만든다
    pub create_desktop_shortcut: bool,
    pub create_start_menu_shortcut: bool,
    pub selected_modules: Vec<String>,
    /// 미리 받아 둔 검증된 manifest. 없으면 설치가 받는다
    pub release: Option<VerifiedRelease>,
    /// 모듈 메타데이터 (required_extensions 해석용)
    pub module_metadata: Vec<ModuleInfo>,
    /// 설치 폴더에 `saba-chan-installer.exe`로 남길 제거기 (창 인스톨러는 자기 exe). 없으면 남기지 않는다
    pub uninstaller_exe: Option<PathBuf>,
}

/// 설치가 부르는 바깥 동작
pub trait InstallHooks: Send + Sync {
    /// 단계마다 (다운로드는 받는 동안 계속). 받기와 같은 작업에서 불리므로 막지 않는다 (무거운 일은 채널로 넘긴다)
    fn progress(&self, progress: &InstallProgress);

    /// 쓸 수 없는 설치 폴더를 권한 상승으로 만든다. 기본은 상승하지 않고 이유를 돌려준다
    fn elevate_create_dir(&self, dir: &Path) -> Result<(), String> {
        Err(format!(
            "{} is not writable; choose another folder or run with administrator rights",
            dir.display()
        ))
    }
}

// ═══════════════════════════════════════════════════════
// 설치
// ═══════════════════════════════════════════════════════

/// 설치 실행. 마지막 진행(완료 또는 실패)을 알리고 돌려준다
pub async fn install(config: InstallOptions, hooks: &dyn InstallHooks) -> InstallProgress {
    let emit = |step: &str, msg: &str, pct: i32| {
        hooks.progress(&InstallProgress {
            step: step.to_string(),
            message: msg.to_string(),
            percent: pct,
            ..Default::default()
        });
    };
    let fail = |code: &str, msg: &str| {
        // 안내하는 로그 파일에 실패 원인이 남도록 먼저 기록한다
        tracing::error!("[{}] {}", code, msg);
        let p = InstallProgress {
            step: "error".into(),
            message: msg.to_string(),
            error: Some(msg.to_string()),
            log_path: Some(log_file_path().display().to_string()),
            error_code: Some(code.to_string()),
            ..Default::default()
        };
        hooks.progress(&p);
        p
    };

    // 설치는 계속하되 사용자가 알아야 하는 실패 (로그 파일과 완료 화면에 남는다)
    let warnings = std::sync::Mutex::new(Vec::<String>::new());
    let warn = |msg: String| {
        tracing::warn!("{}", msg);
        if let Ok(mut w) = warnings.lock() {
            w.push(msg);
        }
    };

    // Step 1: 설치 디렉토리 생성 + 쓰기 권한 확인 (0-5%)
    emit("prepare", "Creating install directory...", 2);
    let install_dir = config.install_path.clone();
    // 한 PC에는 설치본 하나만 둔다: 설치본들이 사용자 데이터 폴더(설정·인스턴스·기록)를 함께 쓴다
    if let Some(other) = other_install(&install_dir) {
        return fail(
            "installed_elsewhere",
            &format!(
                "Saba-chan is already installed in {}. Update or remove that install first.",
                other.display()
            ),
        );
    }
    // 설치기가 새로 만든 폴더만 전용 폴더로 표시한다 (제거기가 폴더째 지워도 되는 곳).
    // 원래 있던 빈 폴더(드라이브 루트, 문서 폴더 등)는 나중에 다른 파일이 생길 수 있어 표시하지 않는다
    let dedicated = !install_dir.is_dir() || install_dir.join(uninstall::INSTALL_MARKER).is_file();
    let _ = std::fs::create_dir_all(&install_dir); // 이미 있어도 OK

    // 실제 쓰기 가능 여부 테스트
    let needs_elevation = {
        let probe = install_dir.join(".saba-write-test");
        match std::fs::write(&probe, b"test") {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => true,
            Err(_) => {
                // 디렉토리 자체가 없는 경우 (create_dir_all도 실패)
                !install_dir.exists()
            }
        }
    };

    if needs_elevation {
        emit("elevate", "Requesting administrator privileges...", 1);
        if let Err(elev_err) = hooks.elevate_create_dir(&install_dir) {
            return fail("elevation_failed", &format!("Failed to create directory: {}", elev_err));
        }
    }

    // 폴더를 만든 바로 뒤에 표시한다: 내려받다 실패한 뒤 다시 시도해도 전용 폴더로 남는다
    if dedicated {
        if let Err(e) = std::fs::write(install_dir.join(uninstall::INSTALL_MARKER), b"") {
            warn(format!("Failed to mark the install folder: {}", e));
        }
    }

    // Step 2: 서명된 릴리즈 manifest (5-10%)
    emit("fetch", "Fetching release information...", 7);
    let manifest = match config.release.clone() {
        Some(m) => m,
        None => match release::fetch_latest().await {
            Ok(m) => m,
            Err(e) => return fail("fetch_failed", &e),
        },
    };

    // 설치기 자체가 이 릴리즈를 설치할 수 있는 버전인지
    if let Some(required_version) = release::required_installer_version(&manifest) {
        if version_compare(INSTALLER_VERSION, required_version) == std::cmp::Ordering::Less {
            return fail(
                "installer_outdated",
                &format!(
                    "This installer (v{}) is outdated. Version v{} or later is required. \
                     Please download the latest installer.",
                    INSTALLER_VERSION, required_version
                ),
            );
        }
    }

    // Step 3: R2에서 받고 SHA-256 확인 후 압축 해제 (10-45%)
    // 하나라도 실패하면 설치를 멈춘다. 반쯤 설치된 앱은 업데이터도 고치기 어렵다.
    // 필수 컴포넌트가 빠진 릴리즈는 기존 설치를 건드리기 전에 멈춘다
    let components = match release::installable(&manifest, config.profile) {
        Ok(c) => c,
        Err(missing) => {
            let msg = format!("The release is missing required components: {}", missing.join(", "));
            return fail("release_incomplete", &msg);
        }
    };

    let total = components.len().max(1);
    let mut installed = Vec::new();
    let temp_dir = release::work_dir();
    let _cleanup = RemoveOnDrop(temp_dir.clone());
    let _ = std::fs::create_dir_all(&temp_dir);

    // 모두 받고 검증한 뒤에야 설치 폴더를 건드린다: 받다 실패해도 있던 설치는 그대로 돈다
    let fetched = fetch_all(&components, &temp_dir, |i, key, info, temp_file| {
        let pct = 10 + (i * 30 / total) as i32;
        emit("download", &format!("Downloading {}...", key), pct);
        let file_name = info.filename.clone();
        let key_name = key.to_string();
        let progress_cb = move |received: u64, total_bytes: Option<u64>| {
            let mb_received = received as f64 / 1_048_576.0;
            let msg = match total_bytes {
                Some(total) => format!(
                    "Downloading {} — {:.1}/{:.1} MB ({}%)",
                    key_name,
                    mb_received,
                    total as f64 / 1_048_576.0,
                    received * 100 / total.max(1)
                ),
                None => format!("Downloading {} — {:.1} MB", key_name, mb_received),
            };
            hooks.progress(&InstallProgress {
                step: "download".to_string(),
                message: msg,
                percent: pct,
                download_file: Some(file_name.clone()),
                download_received: Some(received),
                download_total: total_bytes,
                ..Default::default()
            });
        };
        async move { release::download(info, &temp_file, progress_cb).await }
    })
    .await;
    let fetched = match fetched {
        Ok(f) => f,
        Err((code, msg)) => return fail(code, &msg),
    };

    // 덮어쓰기 설치: 이 폴더의 데몬이 떠 있으면 먼저 끈다 (파일 잠금 방지)
    if let Err(e) = uninstall::stop_daemon_before_install(&install_dir).await {
        return fail("daemon_running", &e);
    }
    // 깨진 기록은 사본으로 옮겨 두고 새로 쓴다 (빈 표로 읽어 덮어쓰면 그 내용이 흔적 없이 사라진다)
    let mut installed_versions = UpdateManager::read_installed_manifest_for_update().unwrap_or_else(|e| {
        tracing::warn!("installed-manifest.json could not be read: {}", e);
        UpdateManager::load_installed_manifest()
    });
    for (i, ((key, info), zip)) in components.iter().zip(&fetched).enumerate() {
        emit(
            "extract",
            &format!("Extracting {}...", key),
            40 + (i * 5 / total) as i32,
        );
        if let Err(e) = extract_zip(zip, &release::target_dir(&install_dir, info)) {
            return fail("extract_failed", &format!("Failed to extract {}: {}", key, e));
        }
        let _ = std::fs::remove_file(zip);
        installed_versions.insert(key.to_string(), info.version.clone());
        installed.push(key.to_string());
    }

    // Step 3.5: 공유 모듈 유틸리티(i18n.py, daemon_rcon.py)를 APPDATA/modules/로 설치
    emit("modules-shared", "Installing shared module utilities...", 46);
    for failure in install_shared_modules(&install_dir) {
        warn(failure);
    }

    // Step 4: 모듈 다운로드 및 설치 (45-55%) — Fix 4: 스트리밍 진행률
    if !config.selected_modules.is_empty() {
        emit("modules-download", "Downloading game modules...", 47);

        let data_dir = resolve_data_dir();
        for module_id in &config.selected_modules {
            let key = format!("{}{}", relay::MODULE_PREFIX, module_id);
            let Some(info) = manifest.components.get(&key) else {
                warn(format!("Module '{}' is not in the release", module_id));
                continue;
            };
            emit("modules-download", &format!("Installing module: {}...", module_id), 49);
            match release::install_addon(&key, info, &data_dir.join("modules").join(module_id)).await {
                Ok(()) => installed.push(format!("module:{}", module_id)),
                Err(e) => warn(format!("Failed to install module '{}': {}", module_id, e)),
            }
        }

        // ── Fix 3: 모듈 의존 익스텐션 자동 설치 ──
        emit("extensions", "Checking required extensions...", 54);
        let required_extensions = collect_required_extensions(&config.selected_modules, &config.module_metadata);

        if !required_extensions.is_empty() {
            tracing::info!("선택된 모듈에 필요한 익스텐션: {:?}", required_extensions);

            let (ext_names, failed) = install_required_extensions(hooks, &manifest, &required_extensions).await;
            for name in ext_names {
                installed.push(format!("extension:{}", name));
            }
            for e in failed {
                warn(e);
            }
        }
    }

    // Step 5: 포터블 Python 다운로드 + venv 생성 (55-70%)
    // Fix 5: 기존 런타임이 손상되었으면 삭제 후 재설치
    emit("runtime-python", "Preparing Python runtime...", 57);
    let runtime_data_dir = runtime_bootstrap::resolve_runtime_data_dir(&install_dir);
    let _ = std::fs::create_dir_all(&runtime_data_dir);

    match runtime_bootstrap::setup_python_with_repair(&runtime_data_dir).await {
        Ok(python_path) => {
            tracing::info!("Python 환경 준비 완료: {}", python_path.display());
            installed.push("runtime:python".to_string());
        }
        Err(e) => {
            // Python 설정 실패는 치명적이지 않음 — 메인 앱이 첫 실행 시 재시도
            warn(format!(
                "Python runtime was not prepared ({}); Saba-chan will retry on first start",
                e
            ));
        }
    }

    // Step 7: 설정 파일 생성 (85-88%)
    emit("config", "Setting up configuration...", 86);
    setup_config(&install_dir, &config);

    // Step 8: 언어 설정 저장 (88-90%)
    emit("config-lang", "Saving language settings...", 89);
    save_language_setting(&config.language);

    // Step 9: 레지스트리 등록 (90-95%). 제거 항목이 없는 설치(CLI)도 폴더 목록에는 남겨, 다른 설치본을 지울 때 공용 데이터를 지키게 한다
    emit("registry", "Registering application...", 92);
    if let Err(e) = saba_chan_updater_lib::uninstall_entry::remember_install(&installs_file(), &install_dir) {
        warn(format!("Failed to record the install folder: {}", e));
    }
    #[cfg(target_os = "windows")]
    {
        if let Err(e) =
            registry::register_uninstall_entry(&install_dir, &manifest.version, config.uninstaller_exe.as_deref())
        {
            warn(format!("Failed to register in Installed apps: {}", e));
        }
    }

    // Step 10: 바로가기 (95-100%). 헤드리스 설치는 GUI가 없어 만들지 않는다
    #[cfg(target_os = "windows")]
    if config.profile == Profile::Desktop {
        emit("shortcuts", "Creating shortcuts...", 96);
        let gui_exe = saba_chan_updater_lib::find_gui_exe(&install_dir);
        let app_name = localized_app_name(&config.language);

        // 이 설치가 전에 만든 바로가기 제거 (언어 변경 시 이전 이름의 잔재 방지).
        // 다른 폴더에 설치된 같은 이름의 바로가기는 둔다
        shortcuts::remove_shortcuts_into(&install_dir, shortcuts::ALL_SHORTCUT_NAMES);

        match &gui_exe {
            Some(gui_exe) => {
                if config.create_desktop_shortcut {
                    if let Err(e) = shortcuts::create_desktop_shortcut(gui_exe, app_name) {
                        warn(format!("Desktop shortcut failed: {}", e));
                    }
                }
                if config.create_start_menu_shortcut {
                    if let Err(e) = shortcuts::create_start_menu_shortcut(gui_exe, app_name) {
                        warn(format!("Start menu shortcut failed: {}", e));
                    }
                }
            }
            None => warn(format!(
                "No GUI executable in {}; shortcuts not created",
                install_dir.display()
            )),
        }
    }

    // Step 10.5: 설치한 버전 기록 (업데이터가 같은 파일을 읽는다)
    emit("manifest", "Recording installed versions...", 98);
    // 설치 폴더에 남긴 제거기 사본도 기록한다. 업데이터가 이 버전을 보고 사본을 바꾼다
    // 복사가 실패해 옛 사본이 남았으면 기록하지 않는다 (새 버전으로 적으면 업데이터가 옛 사본을 안 바꾼다)
    let copy = install_dir.join(saba_chan_updater_lib::INSTALLER_EXE);
    if config
        .uninstaller_exe
        .as_deref()
        .is_some_and(|me| is_copy_of(&copy, me))
    {
        installed_versions.insert("installer".to_string(), INSTALLER_VERSION.to_string());
    }
    // 데스크톱 설치 위에 헤드리스로 덮으면 남은 GUI와 그 기록을 지운다 (업데이터가 다시 손대지 않아 낡은 채 남는다)
    match drop_parts_outside(
        config.profile,
        &install_dir,
        &saba_chan_updater_lib::integrity::records_path(),
        &mut installed_versions,
    ) {
        Ok(problem) => problem.into_iter().for_each(&warn),
        Err(e) => return fail("profile_failed", &e),
    }
    // 업데이터·데몬이 이 구성에 드는 것만 받는다 (헤드리스 설치에 GUI가 끌려 들어오지 않게)
    // 기록이 없으면 데스크톱으로 읽히므로, 헤드리스는 기록하지 못하면 설치를 실패로 끝낸다 (조용히 GUI를 받게 되지 않게)
    // 헤드리스 위에 데스크톱으로 덮다가 기록하지 못해도 실패다 (헤드리스 기록이 남아 새 GUI가 업데이트에서 빠진다)
    if let Err(e) = config.profile.save() {
        if config.profile != Profile::Desktop || Profile::load() != Profile::Desktop {
            return fail("profile_failed", &format!("Failed to write install profile: {}", e));
        }
        warn(format!("Failed to write install profile: {}", e));
    }
    if let Err(e) = UpdateManager::save_installed_manifest(&installed_versions) {
        warn(format!("Failed to write installed manifest: {}", e));
    }

    // 완료. 레지스트리·바로가기 등 실패한 단계는 숨기지 않고 완료 화면에 함께 보여 준다
    let warnings = warnings.into_inner().unwrap_or_default();
    let final_progress = InstallProgress {
        step: "complete".into(),
        message: if warnings.is_empty() {
            format!("{} components installed!", installed.len())
        } else {
            format!(
                "{} components installed, with problems:\n{}\nLog: {}",
                installed.len(),
                warnings.join("\n"),
                log_file_path().display()
            )
        },
        percent: 100,
        complete: true,
        installed_components: installed,
        log_path: (!warnings.is_empty()).then(|| log_file_path().display().to_string()),
        warnings,
        ..Default::default()
    };
    hooks.progress(&final_progress);

    // 임시 디렉토리 정리
    final_progress
}

// ═══════════════════════════════════════════════════════
// 설치 폴더·언어
// ═══════════════════════════════════════════════════════

/// 설치에 쓸 폴더: 고른 폴더가 비어 있지 않고 사바쨩 설치도 아니면 그 아래 `Saba-chan` 폴더.
/// 제거기가 설치 폴더를 지우므로 사용자 파일이 든 폴더에 바로 풀지 않는다
pub fn dedicated_install_dir(chosen: &Path) -> PathBuf {
    if chosen.as_os_str().is_empty()
        || !chosen.is_dir()
        || is_empty_dir(chosen)
        || uninstall::looks_like_install_dir(chosen)
    {
        chosen.to_path_buf()
    } else {
        chosen.join("Saba-chan")
    }
}

pub fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// 이 PC에 이미 있는 설치 폴더: 설치 목록과 "설치된 앱" 항목 중 데몬 실행 파일이 남은 곳
pub fn existing_install() -> Option<PathBuf> {
    found_installs(&installs_file(), &install_entries()).next()
}

/// `chosen`에 설치하면 막히는지: 다른 폴더에 남은 설치본 (`install`도 같은 이유로 멈춘다)
pub fn other_install(chosen: &Path) -> Option<PathBuf> {
    install_elsewhere(chosen, &installs_file(), &install_entries())
}

/// `chosen` 말고 다른 곳에 남은 설치본
fn install_elsewhere(chosen: &Path, installs: &Path, entries: &[(String, String)]) -> Option<PathBuf> {
    found_installs(installs, entries).find(|dir| !saba_chan_updater_lib::same_path(dir, chosen))
}

fn found_installs<'a>(installs: &Path, entries: &'a [(String, String)]) -> impl Iterator<Item = PathBuf> + 'a {
    let listed = saba_chan_updater_lib::uninstall_entry::listed_installs(installs);
    listed
        .into_iter()
        .chain(entries.iter().map(|(_, loc)| loc.clone()))
        .map(PathBuf::from)
        .filter(|dir| dir.join(saba_chan_updater_lib::uninstall_entry::CORE_EXE).is_file())
}

fn installs_file() -> PathBuf {
    saba_chan_updater_lib::uninstall_entry::installs_file()
}

/// "설치된 앱" 항목은 기본 데이터 폴더를 쓰는 설치본이다: 데이터 폴더를 따로 정한 설치(`SABA_DATA_DIR`)는 그 폴더의 설치 목록만 본다
fn install_entries() -> Vec<(String, String)> {
    if std::env::var_os("SABA_DATA_DIR").is_some_and(|d| !d.is_empty()) {
        return Vec::new();
    }
    saba_chan_updater_lib::uninstall_entry::entries()
}

/// 기본 설치 폴더: 이미 설치돼 있으면 그 자리 (그 자리를 업데이트한다)
pub fn default_install_path() -> String {
    if let Some(dir) = existing_install() {
        return dir.to_string_lossy().into_owned();
    }
    #[cfg(target_os = "windows")]
    {
        r"C:\Program Files\Saba-chan".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("saba-chan")
                .to_string_lossy()
                .to_string();
        }
        "saba-chan".to_string()
    }
}

/// 설치 화면의 처음 언어: 앱에서 고른 언어, 없으면 OS 언어, 그것도 없으면 영어
pub fn preferred_language() -> String {
    load_main_app_language()
        .and_then(|lang| normalize_language_tag(&lang))
        .or_else(|| sys_locale::get_locale().and_then(|locale| normalize_language_tag(&locale)))
        .unwrap_or_else(|| "en".to_string())
}

/// 로그 파일: 창 인스톨러는 stderr가 보이지 않는다. 실패한 단계를 나중에 확인할 수 있게 파일에도 남긴다
pub fn log_file_path() -> PathBuf {
    std::env::temp_dir().join("saba-chan-installer.log")
}

// ═══════════════════════════════════════════════════════
// 헬퍼 함수
// ═══════════════════════════════════════════════════════

/// 설치 언어에 맞는 앱 표시 이름을 반환한다 (바로가기 이름, Windows 전용).
#[cfg_attr(not(windows), allow(dead_code))]
fn localized_app_name(language: &str) -> &'static str {
    match language {
        "ko" => "사바쨩",
        "ja" => "サバちゃん",
        _ => "Saba-chan",
    }
}

/// saba-core와 동일한 데이터 디렉토리 경로 해석
/// Windows: `%APPDATA%/saba-chan`, Linux/macOS: `~/.config/saba-chan`
fn resolve_data_dir() -> PathBuf {
    constants::resolve_data_dir()
}

fn load_main_app_language() -> Option<String> {
    ConfigFile::default_location()
        .section("gui")
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn normalize_language_tag(input: &str) -> Option<String> {
    let result = constants::resolve_locale(input);
    if result == "en" && !input.trim().is_empty() {
        // resolve_locale always returns a value; check if input was explicitly "en"
        let trimmed = input.trim();
        let canonical = trimmed.replace('_', "-");
        if canonical.eq_ignore_ascii_case("en") || canonical.to_lowercase().starts_with("en") {
            return Some("en".to_string());
        }
        // If resolve_locale fell back to "en" for an unknown input, return None
        // to let the caller decide
        for supported in constants::SUPPORTED_LANGUAGES {
            if supported.eq_ignore_ascii_case(&canonical) {
                return Some(supported.to_string());
            }
        }
        let base = canonical.to_lowercase();
        let base = base.split('-').next().unwrap_or("");
        if ["ko", "ja", "zh", "es", "pt", "ru", "de", "fr"].contains(&base) {
            return Some(result);
        }
        return None;
    }
    Some(result)
}

/// 컴포넌트를 모두 `temp_dir`에 받고(`fetch`가 SHA-256까지 확인) 빈 폴더에 한 번 풀어 본다.
/// 하나라도 받지 못하거나 깨졌으면 무엇도 적용하기 전에 실패 종류와 이유를 돌려준다. 받은 zip 경로를 같은 순서로 돌려준다
async fn fetch_all<'a, F, Fut>(
    components: &[(&'a String, &'a relay::ManifestComponent)],
    temp_dir: &Path,
    mut fetch: F,
) -> Result<Vec<PathBuf>, (&'static str, String)>
where
    F: FnMut(usize, &'a String, &'a relay::ManifestComponent, PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut zips = Vec::new();
    for (i, (key, info)) in components.iter().enumerate() {
        let zip = temp_dir.join(&info.filename);
        fetch(i, key, info, zip.clone())
            .await
            .map_err(|e| ("download_failed", format!("Failed to download {}: {}", key, e)))?;
        let probe = temp_dir.join(format!(".check-{}", i));
        let checked = saba_chan_updater_lib::extract_zip(&zip, &probe, |_| Ok(()));
        let _ = std::fs::remove_dir_all(&probe);
        checked.map_err(|e| ("extract_failed", format!("Failed to extract {}: {}", key, e)))?;
        zips.push(zip);
    }
    Ok(zips)
}

fn extract_zip(zip_path: &Path, target_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(target_dir)?;
    // 덮어쓰기 설치 때 켜진 서버의 saba-runner처럼 잠긴 exe는 업데이터처럼 비켜 둔다
    saba_chan_updater_lib::extract_zip(zip_path, target_dir, saba_chan_updater_lib::move_aside_binary).map(|_| ())
}

/// core zip에 포함된 공유 Python 모듈(i18n.py, daemon_rcon.py)을
/// %APPDATA%/saba-chan/modules/로 복사한다.
/// 게임 모듈(minecraft, palworld 등)의 lifecycle.py가 이 유틸리티를 import 한다.
/// 실패한 것마다 안내 문장을 돌려준다 (완료 화면 경고)
fn install_shared_modules(install_dir: &Path) -> Vec<String> {
    const SHARED_MODULES: &[&str] = &["i18n.py", "daemon_rcon.py"];

    let data_dir = resolve_data_dir();
    let modules_dir = data_dir.join("modules");
    let _ = std::fs::create_dir_all(&modules_dir);
    let mut failures = Vec::new();

    for name in SHARED_MODULES {
        let src = install_dir.join(name);
        let dst = modules_dir.join(name);
        if src.exists() {
            match std::fs::copy(&src, &dst) {
                Ok(_) => {
                    tracing::info!("Shared module installed: {} → {}", src.display(), dst.display());
                    // install_dir에 남아 있는 복사본 제거 (Program Files에 불필요)
                    let _ = std::fs::remove_file(&src);
                }
                Err(e) => failures.push(format!("Failed to install shared module {}: {}", name, e)),
            }
        } else {
            tracing::debug!("Shared module not found in zip: {}", name);
        }
    }
    failures
}

fn setup_config(install_dir: &Path, config: &InstallOptions) {
    // 설정은 코드에 내장되므로 config 파일 생성 불필요
    // 필수 디렉터리만 생성
    let _ = std::fs::create_dir_all(install_dir.join("locales"));
    let data_dir = resolve_data_dir();
    let _ = std::fs::create_dir_all(data_dir.join("modules"));

    // 언어 설정만 별도 저장
    save_language_setting(&config.language);
}

/// 설치 시 고른 언어를 GUI 설정(`config.json`의 `gui.language`)에 넣는다. 다른 키는 보존.
fn save_language_setting(language: &str) {
    let result = ConfigFile::default_location().update(|doc| {
        let gui = doc.entry("gui".to_string()).or_insert_with(|| serde_json::json!({}));
        if !gui.is_object() {
            *gui = serde_json::json!({});
        }
        gui["language"] = serde_json::Value::String(language.to_string());
    });
    if let Err(e) = result {
        tracing::warn!("Failed to save language setting: {}", e);
    }
}

// ═══════════════════════════════════════════════════════
// Fix 1: 버전 비교 유틸리티
// ═══════════════════════════════════════════════════════

/// semver 문자열 비교 (major.minor.patch)
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    let max_len = va.len().max(vb.len());
    for i in 0..max_len {
        let a_part = va.get(i).copied().unwrap_or(0);
        let b_part = vb.get(i).copied().unwrap_or(0);
        match a_part.cmp(&b_part) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

// ═══════════════════════════════════════════════════════
// Fix 3: 모듈 의존 익스텐션 자동 설치
// ═══════════════════════════════════════════════════════

/// 선택된 모듈의 required_extensions를 수집 (중복 제거)
fn collect_required_extensions(selected_modules: &[String], module_metadata: &[ModuleInfo]) -> Vec<String> {
    let mut extensions: Vec<String> = Vec::new();
    for module_id in selected_modules {
        if let Some(meta) = module_metadata.iter().find(|m| m.id == *module_id) {
            for ext in &meta.required_extensions {
                if !extensions.contains(ext) {
                    extensions.push(ext.clone());
                }
            }
        }
    }
    extensions
}

/// 필요한 익스텐션을 릴리즈 manifest의 `ext-*` 컴포넌트에서 받아 설치 (SHA-256 검증)
async fn install_required_extensions(
    hooks: &dyn InstallHooks,
    manifest: &ReleaseManifest,
    required_extensions: &[String],
) -> (Vec<String>, Vec<String>) {
    let data_dir = resolve_data_dir();
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for ext_id in required_extensions {
        let key = format!("{}{}", relay::EXTENSION_PREFIX, ext_id);
        let Some(info) = manifest.components.get(&key) else {
            failed.push(format!("Extension {} is not in this release", ext_id));
            continue;
        };
        hooks.progress(&InstallProgress {
            step: "extensions".to_string(),
            message: format!("Installing extension: {}...", ext_id),
            percent: 54,
            download_file: Some(info.filename.clone()),
            ..Default::default()
        });

        let default_dir = format!("extensions/{}", ext_id.replace('-', "_"));
        let target_dir = data_dir.join(info.install_dir.as_deref().unwrap_or(&default_dir));
        match release::install_addon(&key, info, &target_dir).await {
            Ok(()) => {
                tracing::info!("익스텐션 '{}' 설치 완료: {}", ext_id, target_dir.display());
                installed.push(ext_id.clone());
            }
            Err(e) => failed.push(format!("Failed to install extension {}: {}", ext_id, e)),
        }
    }
    (installed, failed)
}

/// `copy`가 `original` 자신이거나 내용이 같은지
fn is_copy_of(copy: &Path, original: &Path) -> bool {
    if copy == original {
        return copy.is_file();
    }
    matches!((std::fs::read(copy), std::fs::read(original)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // M1: 설치 폴더 사본이 이 설치기와 다르면(복사 실패로 옛 사본이 남음) 새 버전으로 기록하지 않는다
    #[test]
    fn only_a_matching_installer_copy_is_recorded() {
        let root = std::env::temp_dir().join(format!("saba-installer-copy-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let me = root.join("me.exe");
        let copy = root.join("copy.exe");
        std::fs::write(&me, b"new").unwrap();
        std::fs::write(&copy, b"old").unwrap();
        assert!(!is_copy_of(&copy, &me));
        std::fs::write(&copy, b"new").unwrap();
        assert!(is_copy_of(&copy, &me));
        assert!(is_copy_of(&me, &me));
        assert!(!is_copy_of(&root.join("missing.exe"), &me));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_with_other_files_gets_its_own_saba_chan_subfolder() {
        let root = std::env::temp_dir().join(format!("saba-install-dir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(dedicated_install_dir(&root), root, "an empty folder is used as is");
        assert_eq!(
            dedicated_install_dir(&root.join("new")),
            root.join("new"),
            "a new folder is used as is"
        );

        std::fs::write(root.join("notes.txt"), b"").unwrap();
        assert_eq!(dedicated_install_dir(&root), root.join("Saba-chan"));

        let core = format!("saba-core{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(root.join(core), b"").unwrap();
        assert_eq!(
            dedicated_install_dir(&root),
            root,
            "an existing install is updated in place"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_headless_install_over_a_desktop_one_drops_the_gui() {
        let tmp = tempfile::tempdir().unwrap();
        let gui = tmp.path().join(saba_chan_updater_lib::GUI_EXE_NAMES[0]);
        std::fs::write(&gui, b"").unwrap();
        let mut installed: HashMap<String, String> = [("gui", "0.1.8"), ("saba-core", "0.1.9")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        // 업데이터가 적용한 GUI는 파일 해시 기록이 있다: 남기면 무결성 검사가 지운 GUI를 "파일 없음"으로 보인다
        let records = tmp.path().join("installed-files.json");
        std::fs::write(&records, r#"{"gui":{"a":"0"},"saba-core":{"b":"1"}}"#).unwrap();

        assert_eq!(
            drop_parts_outside(Profile::Desktop, tmp.path(), &records, &mut installed),
            Ok(None)
        );
        assert!(
            gui.is_file() && installed.contains_key("gui"),
            "a desktop install keeps the GUI"
        );

        assert_eq!(
            drop_parts_outside(Profile::Headless, tmp.path(), &records, &mut installed),
            Ok(None)
        );
        assert!(!gui.exists());
        assert_eq!(installed.keys().collect::<Vec<_>>(), ["saba-core"]);
        let kept: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&records).unwrap()).unwrap();
        assert_eq!(kept.as_object().unwrap().keys().collect::<Vec<_>>(), ["saba-core"]);
    }

    // 둘째 컴포넌트가 깨졌으면 첫째도 적용하지 않는다: 받기·검증이 모두 끝나야 설치 폴더를 건드린다
    #[tokio::test]
    async fn a_broken_second_archive_stops_before_anything_is_applied() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let comp = |file: &str| relay::ManifestComponent {
            version: "0.1.10".into(),
            filename: file.into(),
            size_bytes: None,
            sha256: "0".repeat(64),
            url: "https://example.invalid/x.zip".into(),
            install_dir: None,
            requires: None,
            meta: None,
        };
        let (core, gui) = (comp("core.zip"), comp("gui.zip"));
        let (k1, k2) = ("saba-core".to_string(), "gui".to_string());
        let components = vec![(&k1, &core), (&k2, &gui)];
        let fake = |_: usize, _: &String, info: &relay::ManifestComponent, dest: PathBuf| {
            let good = info.filename == "core.zip";
            async move {
                if good {
                    let mut zip = zip::ZipWriter::new(std::fs::File::create(&dest).unwrap());
                    zip.start_file("saba-core", zip::write::FileOptions::default()).unwrap();
                    zip.write_all(b"bin").unwrap();
                    zip.finish().unwrap();
                } else {
                    std::fs::write(&dest, b"not a zip").unwrap();
                }
                Ok(())
            }
        };

        let err = fetch_all(&components, tmp.path(), fake).await.unwrap_err();
        assert_eq!(err.0, "extract_failed", "{}", err.1);
        assert!(err.1.contains("gui"), "{}", err.1);

        let offline = |_: usize, key: &String, _: &relay::ManifestComponent, _: PathBuf| {
            let fail = key == "gui";
            async move {
                if fail {
                    Err("offline".to_string())
                } else {
                    Ok(())
                }
            }
        };
        let only_core = vec![(&k2, &gui)];
        assert_eq!(
            fetch_all(&only_core, tmp.path(), offline).await.unwrap_err().0,
            "download_failed"
        );

        let left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .collect();
        assert!(left.is_empty(), "no check folder is left behind");
    }

    // 한 PC 한 설치: 데몬 실행 파일이 남은 다른 폴더가 있으면 그곳을 알려 준다. 같은 폴더·지워진 폴더는 막지 않는다
    #[test]
    fn another_install_on_this_pc_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (here, there, gone) = (
            tmp.path().join("here"),
            tmp.path().join("there"),
            tmp.path().join("gone"),
        );
        for dir in [&here, &there, &gone] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(here.join(saba_chan_updater_lib::uninstall_entry::CORE_EXE), b"").unwrap();
        let installs = tmp.path().join("installs.json");
        std::fs::write(&installs, serde_json::to_vec(&[&here, &gone]).unwrap()).unwrap();

        assert_eq!(
            install_elsewhere(&here, &installs, &[]),
            None,
            "updating in place is fine"
        );
        assert_eq!(install_elsewhere(&there, &installs, &[]), Some(here.clone()));

        // 설치 목록에 없는 옛 설치도 "설치된 앱" 항목으로 찾는다
        std::fs::write(&installs, b"[]").unwrap();
        let entries = [("Saba-chan".to_string(), here.to_string_lossy().into_owned())];
        assert_eq!(install_elsewhere(&there, &installs, &entries), Some(here.clone()));
        assert_eq!(install_elsewhere(&there, &tmp.path().join("missing.json"), &[]), None);
    }

    #[cfg(unix)]
    struct Recorder(std::sync::Mutex<Vec<InstallProgress>>);

    #[cfg(unix)]
    impl InstallHooks for Recorder {
        fn progress(&self, progress: &InstallProgress) {
            self.0.lock().unwrap().push(progress.clone());
        }
    }

    // 창 없이 부르면 권한 상승을 하지 않는다: 쓸 수 없는 폴더에서 아무것도 받지 않고 이유와 함께 멈춘다
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unwritable_folder_stops_without_elevation() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("saba-install-ro-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
        // root는 권한과 무관하게 쓸 수 있어 이 확인이 뜻이 없다
        if std::fs::write(root.join("probe"), b"").is_ok() {
            eprintln!("skipped: running as root, permissions are not enforced");
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let hooks = Recorder(Default::default());
        let options = InstallOptions {
            install_path: root.join("Saba-chan"),
            language: "en".into(),
            profile: Profile::Headless,
            create_desktop_shortcut: false,
            create_start_menu_shortcut: false,
            selected_modules: Vec::new(),
            release: None,
            module_metadata: Vec::new(),
            uninstaller_exe: None,
        };

        let last = install(options, &hooks).await;

        assert_eq!(last.error_code.as_deref(), Some("elevation_failed"), "{:?}", last);
        assert!(last.message.contains("not writable"), "{}", last.message);
        assert_eq!(
            hooks.0.lock().unwrap().last().map(|p| p.step.clone()).as_deref(),
            Some("error")
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
}
